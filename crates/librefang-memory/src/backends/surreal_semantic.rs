//! SurrealDB-backed [`crate::backend::SemanticBackend`] and
//! [`librefang_types::memory::VectorStore`] implementations.
//!
//! ## Design
//!
//! Both faces live in the `surreal-memory` crate's `memory` table, which carries an HNSW vector index sized from the embedding driver's dimension.
//! Every query in this file runs on the memory store's own connection ([`SurrealStorage::db`]); the operational `librefang.surreal` database never holds a `memory` row, so querying it would always find nothing.
//!
//! Each row is keyed by the librefang id (`memory:⟨<MemoryId>⟩`), so `forget`, `update_access`, `delete` and `get_embeddings` address a row directly instead of searching metadata for it.
//!
//! Two search paths are provided:
//!
//! 1. **Text path** (`recall` without a query embedding) → `surreal_memory::MemoryStorage::search_memories`.
//! 2. **Vector path** (`recall` with a query embedding, and `VectorStore::search`) → a `SELECT … <|k,COSINE|> $vec` KNN query.
//!    SurrealDB requires the KNN `k` operand to be a literal unsigned integer, so LibreFang clamps and formats that number itself; every caller-derived value is bound.
//!
//! ## The two faces write different rows
//!
//! `SemanticBackend::remember` writes a full fragment: `agent_id`, the scope as the first category, and the librefang-only fields nested under `metadata.librefang` / `metadata.user`.
//! `VectorStore::insert` is the mirror the kernel attaches to the SQLite `SemanticStore` (`[memory] vector_backend = "surreal"`): SQLite stays the system of record, and the mirror row holds only the id, the content and the vector.
//! The `VectorStore` contract gives an insert no agent, so mirror rows carry no `agent_id`; `VectorStore::search` therefore lets unowned rows through its agent filter, and the `SemanticStore` re-applies the caller's filter to the fragments it hydrates from SQLite.
//! `SemanticBackend::recall` and `count` filter strictly, so a mirror row can never surface as another agent's memory there.
//!
//! ## `MemoryFragment` ↔ `surreal_memory::Memory` mapping
//!
//! | `MemoryFragment` field | `Memory` field / location |
//! |---|---|
//! | `id` | record key, and `metadata["librefang"]["lf_id"]` |
//! | `content` | `content` |
//! | `embedding` | `embedding` |
//! | `agent_id` | `agent_id` (string) |
//! | `scope` | first element of `categories` |
//! | `confidence` | `importance` |
//! | `peer_id` (from filter) | `user_id` |
//! | `created_at` | `created_at` (Datetime) |
//! | `accessed_at` / `access_count` | `last_accessed_at` / `access_count` |
//! | `source`, `modality`, `image_url`, `image_embedding`, caller `metadata` | nested under `metadata["librefang"]` and `metadata["user"]` |
//!
//! The round-trip is lossless: `fragment_to_memory` → store → `memory_to_fragment`
//! produces an identical [`librefang_types::memory::MemoryFragment`] (verified
//! by the unit tests at the bottom of this file).
//!
//! ## SQL injection safety
//!
//! All SurrealQL queries in this file use parameterised bindings (`.bind()`).
//! No caller-supplied strings are ever interpolated into query text.

use super::shared::memory_error;
use crate::backend::SemanticBackend;
use crate::proactive::EmbeddingFn;
use async_trait::async_trait;
use chrono::DateTime;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    MemoryFilter, MemoryFragment, MemoryId, MemoryModality, MemorySource, VectorSearchResult,
    VectorStore,
};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use surreal_memory::{MemoryStorage, SurrealStorage};
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

// ── Re-export so the AgentId → String conversion is available ─────────────────
use librefang_types::agent::AgentId;

/// Upper bound on the KNN `k` operand, which SurrealDB needs as a literal.
const MAX_KNN_K: usize = 1000;

// ── SurrealSemanticBackend ────────────────────────────────────────────────────

/// SurrealDB-backed semantic memory store.
///
/// Wraps `surreal_memory::SurrealStorage` for text recall and CRUD, and issues KNN queries on the same store's connection for vector recall.
pub struct SurrealSemanticBackend {
    /// `surreal-memory` storage for text search, writes and deletes.
    storage: Arc<SurrealStorage>,
    /// Connection to the memory store, for the queries `MemoryStorage` has no method for (KNN, count, access bump).
    db: Surreal<Any>,
    /// Embeds the content of a `remember` call that arrives without a vector.
    embedding: Arc<dyn EmbeddingFn>,
}

impl SurrealSemanticBackend {
    /// Wrap an open `SurrealStorage`.
    ///
    /// `embedding` must be the driver the storage was opened with (see [`super::shared::open_shared_memory_storage`]), so a vector computed here lands in the same space as the storage's own.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage's connection is not live.
    pub fn new(
        storage: Arc<SurrealStorage>,
        embedding: Arc<dyn EmbeddingFn>,
    ) -> LibreFangResult<Self> {
        let db = storage
            .db()
            .map_err(|e| memory_error("SurrealSemanticBackend: memory store connection", e))?;
        Ok(Self {
            storage,
            db,
            embedding,
        })
    }

    /// Open a `SurrealSemanticBackend` from a kernel storage config, building a dedicated `SurrealStorage` connection internally.
    ///
    /// This is the factory `librefang-kernel` uses, so that `surreal-memory` is not a direct dependency of the kernel crate.
    ///
    /// `embedding` and `dimensions` are passed to [`super::shared::open_shared_memory_storage`]: `SurrealStorage` embeds every memory it stores without a vector, and every recall made without `query_embedding`, with that driver, so it must be the driver the `ContextEngine` uses for `query_embedding`.
    pub async fn open_with_storage(
        storage_cfg: &librefang_storage::config::StorageConfig,
        embedding: Arc<dyn EmbeddingFn>,
        dimensions: usize,
    ) -> Result<Self, String> {
        // Delegate to the single-source factory, which owns the memory store's SurrealKV lock; standalone callers get a fresh `SurrealStorage` here because no other opener exists in their process.
        let storage =
            super::shared::open_shared_memory_storage(storage_cfg, embedding.clone(), dimensions)
                .await
                .map_err(|e| format!("SurrealStorage (semantic backend): {e}"))?;
        Self::new(storage, embedding).map_err(|e| e.to_string())
    }

    /// The underlying `SurrealStorage`, for callers that need a second backend on the same store (the proactive backend).
    #[must_use]
    pub fn storage(&self) -> Arc<SurrealStorage> {
        Arc::clone(&self.storage)
    }
}

// ── MemoryFragment ↔ surreal_memory::Memory round-trip helpers ────────────────

/// Convert a [`MemoryFragment`] into a `surreal_memory::Memory` for storage.
pub fn fragment_to_memory(frag: &MemoryFragment) -> surreal_memory::memory::Memory {
    // Encode all librefang-specific fields that have no direct counterpart
    // in `surreal_memory::Memory` into `metadata["librefang"]`.
    let source_str = serde_json::to_string(&frag.source)
        .unwrap_or_else(|_| "\"conversation\"".to_string())
        .trim_matches('"')
        .to_string();
    let modality_str = serde_json::to_string(&frag.modality)
        .unwrap_or_else(|_| "\"text\"".to_string())
        .trim_matches('"')
        .to_string();

    let lf_meta = serde_json::json!({
        "lf_id":         frag.id.0.to_string(),
        "source":        source_str,
        "modality":      modality_str,
        "image_url":     frag.image_url,
        "image_embedding": frag.image_embedding,
    });
    let user_meta: JsonValue = if frag.metadata.is_empty() {
        JsonValue::Null
    } else {
        serde_json::to_value(&frag.metadata).unwrap_or(JsonValue::Null)
    };

    let metadata = serde_json::json!({
        "librefang": lf_meta,
        "user":      user_meta,
    });

    surreal_memory::memory::Memory {
        id: None,
        content: frag.content.clone(),
        embedding: frag.embedding.clone(),
        scope: surreal_memory::memory::MemoryScope::default(),
        memory_type: surreal_memory::memory::MemoryType::default(),
        user_id: None, // peer_id is not available on a bare fragment
        session_id: None,
        agent_id: Some(frag.agent_id.0.to_string()),
        task_stream_id: None,
        categories: if frag.scope.is_empty() {
            Vec::new()
        } else {
            vec![frag.scope.clone()]
        },
        metadata: Some(metadata),
        token_count: None,
        importance: frag.confidence,
        access_count: u32::try_from(frag.access_count).unwrap_or(u32::MAX),
        last_accessed_at: Some(frag.accessed_at.into()),
        valid_until: None,
        version: 1,
        created_at: surrealdb::types::Datetime::default(),
        updated_at: surrealdb::types::Datetime::default(),
    }
}

/// Convert a `surreal_memory::Memory` back into a [`MemoryFragment`].
///
/// All librefang-specific fields are recovered from `metadata["librefang"]`.
/// Fields absent from the metadata fall back to safe defaults.
pub fn memory_to_fragment(mem: surreal_memory::memory::Memory) -> MemoryFragment {
    let lf: HashMap<String, JsonValue> = mem
        .metadata
        .as_ref()
        .and_then(|m| m.get("librefang"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let user_meta: HashMap<String, JsonValue> = mem
        .metadata
        .as_ref()
        .and_then(|m| m.get("user"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Recover the original `MemoryId` from `lf["lf_id"]`, then from the record key; generate a new one only for rows librefang did not write.
    let record_key = mem.id.as_ref().map(record_key_string);
    let id = lf
        .get("lf_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or(record_key)
        .and_then(|s| Uuid::parse_str(&s).ok())
        .map(MemoryId)
        .unwrap_or_default();

    let agent_id = mem
        .agent_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(AgentId)
        .unwrap_or_default();

    let scope = mem.categories.into_iter().next().unwrap_or_default();

    let source: MemorySource = lf
        .get("source")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
        .unwrap_or(MemorySource::Conversation);

    let modality: MemoryModality = lf
        .get("modality")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
        .unwrap_or_default();

    let created_at: DateTime<chrono::Utc> = mem.created_at.into();
    // `last_accessed_at` is what `update_access` bumps; a row that was never accessed has none and was last touched at creation.
    let accessed_at: DateTime<chrono::Utc> =
        mem.last_accessed_at.map(Into::into).unwrap_or(created_at);

    let image_url: Option<String> = lf
        .get("image_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let image_embedding: Option<Vec<f32>> = lf
        .get("image_embedding")
        .and_then(|v| serde_json::from_value::<Vec<f32>>(v.clone()).ok());

    MemoryFragment {
        id,
        agent_id,
        content: mem.content,
        embedding: mem.embedding,
        metadata: user_meta,
        source,
        confidence: mem.importance,
        created_at,
        accessed_at,
        access_count: u64::from(mem.access_count),
        scope,
        image_url,
        image_embedding,
        modality,
        // Upstream added `similarity` as the cosine score against a *query*
        // embedding. This mapper materializes a stored row with no query in
        // scope, so there is nothing to score against — matching upstream's own
        // construction sites (semantic.rs:726, proactive.rs:4277).
        similarity: None,
    }
}

/// The key of a `memory` record as a plain string (`memory:⟨k⟩` → `k`).
fn record_key_string(id: &surrealdb::types::RecordId) -> String {
    match &id.key {
        surrealdb::types::RecordIdKey::String(s) => s.clone(),
        surrealdb::types::RecordIdKey::Uuid(u) => u.to_string(),
        other => format!("{other:?}"),
    }
}

// ── SemanticBackend impl ──────────────────────────────────────────────────────

#[async_trait]
impl SemanticBackend for SurrealSemanticBackend {
    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<MemoryId> {
        let now = chrono::Utc::now();
        let frag = MemoryFragment {
            id: MemoryId::new(),
            agent_id,
            content: content.to_string(),
            embedding: None,
            metadata,
            source,
            confidence: 0.8,
            created_at: now,
            accessed_at: now,
            access_count: 0,
            scope: scope.to_string(),
            image_url: None,
            image_embedding: None,
            modality: MemoryModality::default(),
            // A fragment being stored for the first time has no query to be
            // scored against; `similarity` is populated only on recall.
            similarity: None,
        };
        // Store the caller's vector as given instead of embedding the content a second time; only a call without one pays for an embedding.
        let vector = match embedding.filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => self.embedding.embed_one(content).await?,
        };
        let lf_id = frag.id;
        self.storage
            .store_indexed_memory(
                &lf_id.0.to_string(),
                fragment_to_memory(&frag),
                vector,
                None,
            )
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::remember", e))?;
        Ok(lf_id)
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        let filter = filter.unwrap_or_default();
        let agent_id_str = filter.agent_id.map(|a| a.0.to_string());
        let peer_id_str = filter.peer_id.clone();

        if let Some(vec) = query_embedding.filter(|v| !v.is_empty()) {
            let hits = self
                .knn(
                    &vec,
                    limit,
                    agent_id_str.as_deref(),
                    peer_id_str.as_deref(),
                    AgentFilter::Strict,
                )
                .await?;
            let mut fragments = Vec::with_capacity(hits.len());
            for hit in hits {
                let Some(mem) = self
                    .storage
                    .get_memory(&hit.key)
                    .await
                    .map_err(|e| memory_error("SurrealSemanticBackend::recall (hydrate)", e))?
                else {
                    // Deleted between the KNN query and this read.
                    continue;
                };
                let mut frag = memory_to_fragment(mem);
                frag.similarity = Some(hit.score);
                fragments.push(frag);
            }
            return Ok(fragments);
        }

        // ── Text path via surreal-memory ──────────────────────────────────────
        let results = self
            .storage
            .search_memories(
                query,
                peer_id_str.as_deref(),
                agent_id_str.as_deref(),
                None,
                None,
                limit,
            )
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::recall (search_memories)", e))?;

        Ok(results.into_iter().map(memory_to_fragment).collect())
    }

    async fn forget(&self, id: MemoryId) -> LibreFangResult<bool> {
        let key = id.0.to_string();
        let exists = self
            .storage
            .get_memory(&key)
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::forget (lookup)", e))?
            .is_some();
        if !exists {
            return Ok(false);
        }
        self.storage
            .delete_memory(&key)
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::forget", e))?;
        Ok(true)
    }

    async fn count(&self, filter: MemoryFilter) -> LibreFangResult<u64> {
        let agent_id_str = filter.agent_id.map(|a| a.0.to_string());
        let peer_id_str = filter.peer_id;

        let rows: Vec<JsonValue> = self
            .db
            .query(COUNT_QUERY)
            .bind(("agent_id", agent_id_str))
            .bind(("peer_id", peer_id_str))
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::count", e))?
            .take(0)
            .map_err(|e| memory_error("SurrealSemanticBackend::count (decode)", e))?;

        Ok(rows
            .first()
            .and_then(|r| r.get("count"))
            .and_then(JsonValue::as_u64)
            .unwrap_or(0))
    }

    async fn update_access(&self, id: MemoryId) -> LibreFangResult<()> {
        self.db
            .query(
                "UPDATE type::record('memory', $key) SET \
                   access_count += 1, \
                   last_accessed_at = time::now()",
            )
            .bind(("key", id.0.to_string()))
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::update_access", e))?
            .check()
            .map_err(|e| memory_error("SurrealSemanticBackend::update_access", e))?;
        Ok(())
    }

    fn backend_name(&self) -> &str {
        "surreal"
    }
}

/// Count query shared by [`SemanticBackend::count`] and its template test.
const COUNT_QUERY: &str = "SELECT count() FROM memory \
     WHERE ($agent_id IS NONE OR agent_id = $agent_id) \
     AND ($peer_id IS NONE OR user_id = $peer_id) \
     GROUP ALL";

// ── VectorStore (mirror of the SQLite SemanticStore) ─────────────────────────

#[async_trait]
impl VectorStore for SurrealSemanticBackend {
    /// Upsert the vector for `id`.
    ///
    /// The row carries no metadata: SQLite holds the fragment, and the `SemanticStore` hydrates every hit from there by id.
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        payload: &str,
        _metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<()> {
        let existing = self
            .storage
            .get_memory(id)
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::insert (lookup)", e))?;
        if existing.is_some() {
            // `update_embedding` re-inserts an id with its new vector; the contract is an upsert.
            self.db
                .query(
                    "UPDATE type::record('memory', $key) SET \
                       content = $content, embedding = $embedding, updated_at = time::now()",
                )
                .bind(("key", id.to_string()))
                .bind(("content", payload.to_string()))
                .bind(("embedding", embedding.to_vec()))
                .await
                .map_err(|e| memory_error("SurrealSemanticBackend::insert (update)", e))?
                .check()
                .map_err(|e| memory_error("SurrealSemanticBackend::insert (update)", e))?;
            return Ok(());
        }
        let now = surrealdb::types::Datetime::default();
        let mem = surreal_memory::memory::Memory {
            id: None,
            content: payload.to_string(),
            embedding: None,
            scope: surreal_memory::memory::MemoryScope::default(),
            memory_type: surreal_memory::memory::MemoryType::default(),
            user_id: None,
            session_id: None,
            agent_id: None,
            task_stream_id: None,
            categories: Vec::new(),
            metadata: None,
            token_count: None,
            importance: 0.5,
            access_count: 0,
            last_accessed_at: None,
            valid_until: None,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        self.storage
            .store_indexed_memory(id, mem, embedding.to_vec(), None)
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::insert", e))?;
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<VectorSearchResult>> {
        let filter = filter.unwrap_or_default();
        let agent_id_str = filter.agent_id.map(|a| a.0.to_string());
        let peer_id_str = filter.peer_id;
        let hits = self
            .knn(
                query_embedding,
                limit,
                agent_id_str.as_deref(),
                peer_id_str.as_deref(),
                AgentFilter::AllowUnowned,
            )
            .await?;
        Ok(hits
            .into_iter()
            .map(|hit| VectorSearchResult {
                id: hit.key,
                payload: hit.content,
                score: hit.score,
                metadata: HashMap::new(),
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        self.db
            .query("DELETE type::record('memory', $key)")
            .bind(("key", id.to_string()))
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::delete", e))?
            .check()
            .map_err(|e| memory_error("SurrealSemanticBackend::delete", e))?;
        Ok(())
    }

    async fn get_embeddings(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let keys: Vec<String> = ids.iter().map(|s| (*s).to_string()).collect();
        let rows: Vec<JsonValue> = self
            .db
            .query(
                "SELECT record::id(id) AS key, embedding \
                 FROM array::map($keys, |$k| type::record('memory', $k)) \
                 WHERE embedding != NONE",
            )
            .bind(("keys", keys))
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend::get_embeddings", e))?
            .take(0)
            .map_err(|e| memory_error("SurrealSemanticBackend::get_embeddings (decode)", e))?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let key = row.get("key").and_then(JsonValue::as_str);
            let emb = row
                .get("embedding")
                .and_then(|v| serde_json::from_value::<Vec<f32>>(v.clone()).ok());
            if let (Some(key), Some(emb)) = (key, emb) {
                map.insert(key.to_string(), emb);
            }
        }
        Ok(map)
    }

    fn backend_name(&self) -> &str {
        "surreal"
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// How the KNN query treats rows that carry no `agent_id`.
#[derive(Clone, Copy)]
enum AgentFilter {
    /// Only rows owned by the requested agent (the `SemanticBackend` face, which returns the rows themselves).
    Strict,
    /// Also rows with no owner: the `VectorStore` mirror writes none, and its caller re-filters what it hydrates.
    AllowUnowned,
}

/// One KNN hit: record key, stored content and cosine similarity to the query.
struct KnnHit {
    key: String,
    content: String,
    score: f32,
}

/// Build the KNN query for a clamped `k`.
fn knn_query(k: usize, agent_filter: AgentFilter) -> String {
    let agent_clause = match agent_filter {
        AgentFilter::Strict => "($agent_id IS NONE OR agent_id = $agent_id)",
        AgentFilter::AllowUnowned => {
            "($agent_id IS NONE OR agent_id IS NONE OR agent_id = $agent_id)"
        }
    };
    // SurrealDB KNN syntax requires a literal unsigned integer in the `<|k,COSINE|>` slot.
    // `k` is a clamped `usize`, so formatting it into the query does not introduce caller-controlled SQL; every user-derived value stays bound.
    format!(
        "SELECT record::id(id) AS key, content, \
                vector::similarity::cosine(embedding, $vec) AS score \
         FROM memory \
         WHERE embedding <|{k},COSINE|> $vec \
           AND {agent_clause} \
           AND ($peer_id IS NONE OR user_id = $peer_id) \
         ORDER BY score DESC \
         LIMIT $k"
    )
}

impl SurrealSemanticBackend {
    /// Nearest neighbours of `embedding` by cosine similarity, best first.
    async fn knn(
        &self,
        embedding: &[f32],
        limit: usize,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
        agent_filter: AgentFilter,
    ) -> LibreFangResult<Vec<KnnHit>> {
        let k = limit.clamp(1, MAX_KNN_K);
        let rows: Vec<JsonValue> = self
            .db
            .query(knn_query(k, agent_filter))
            .bind(("vec", embedding.to_vec()))
            .bind(("k", k as u64))
            .bind(("agent_id", agent_id.map(str::to_string)))
            .bind(("peer_id", peer_id.map(str::to_string)))
            .await
            .map_err(|e| memory_error("SurrealSemanticBackend knn query", e))?
            .take(0)
            .map_err(|e| memory_error("SurrealSemanticBackend knn query (decode)", e))?;

        rows.into_iter()
            .map(|row| {
                let key = row
                    .get("key")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| {
                        LibreFangError::memory_msg(format!(
                            "SurrealSemanticBackend knn query: row without a string key: {row}"
                        ))
                    })?
                    .to_string();
                let content = row
                    .get("content")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string();
                let score = row.get("score").and_then(JsonValue::as_f64).unwrap_or(0.0) as f32;
                Ok(KnnHit {
                    key,
                    content,
                    score,
                })
            })
            .collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::memory::{MemoryId, MemoryModality, MemorySource};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_fragment(scope: &str) -> MemoryFragment {
        let mut meta = HashMap::new();
        meta.insert("key".to_string(), serde_json::json!("value"));
        MemoryFragment {
            id: MemoryId(Uuid::new_v4()),
            agent_id: AgentId(Uuid::new_v4()),
            content: "Test memory content".to_string(),
            embedding: Some(vec![0.1_f32, 0.2, 0.3]),
            metadata: meta,
            source: MemorySource::Conversation,
            confidence: 0.75,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 5,
            scope: scope.to_string(),
            image_url: Some("https://example.com/img.png".to_string()),
            image_embedding: Some(vec![0.9_f32, 0.8]),
            modality: MemoryModality::MultiModal,
            similarity: None,
        }
    }

    /// Verify that `fragment_to_memory` → `memory_to_fragment` is lossless for
    /// all fields that have a defined round-trip path.
    #[test]
    fn round_trip_all_fields() {
        let original = make_fragment("episodic");
        let mem = fragment_to_memory(&original);
        let recovered = memory_to_fragment(mem);

        assert_eq!(recovered.id, original.id, "id mismatch");
        assert_eq!(
            recovered.agent_id.0, original.agent_id.0,
            "agent_id mismatch"
        );
        assert_eq!(recovered.content, original.content, "content mismatch");
        assert_eq!(
            recovered.embedding, original.embedding,
            "embedding mismatch"
        );
        assert_eq!(recovered.scope, original.scope, "scope mismatch");
        assert_eq!(
            recovered.confidence, original.confidence,
            "confidence mismatch"
        );
        assert_eq!(
            recovered.access_count, original.access_count,
            "access_count mismatch"
        );
        assert_eq!(
            recovered.image_url, original.image_url,
            "image_url mismatch"
        );
        assert_eq!(
            recovered.image_embedding, original.image_embedding,
            "image_embedding mismatch"
        );
        assert_eq!(recovered.modality, original.modality, "modality mismatch");
        assert_eq!(
            recovered.metadata.get("key"),
            original.metadata.get("key"),
            "user metadata mismatch"
        );
    }

    /// Verify that an empty scope is preserved.
    #[test]
    fn round_trip_empty_scope() {
        let frag = make_fragment("");
        let mem = fragment_to_memory(&frag);
        let recovered = memory_to_fragment(mem);
        assert_eq!(recovered.scope, "");
    }

    /// Verify that `fragment_to_memory` stores `scope` as the first element
    /// of `categories`.
    #[test]
    fn scope_maps_to_first_category() {
        let frag = make_fragment("semantic");
        let mem = fragment_to_memory(&frag);
        assert_eq!(mem.categories.first().map(String::as_str), Some("semantic"));
    }

    /// Verify that injection-style strings in `scope` are stored verbatim and
    /// recovered correctly (the raw content is never interpolated into SQL).
    #[test]
    fn injection_string_is_data_not_sql() {
        let injection = "'; DROP TABLE memory; --";
        let frag = make_fragment(injection);
        let mem = fragment_to_memory(&frag);
        let recovered = memory_to_fragment(mem);
        assert_eq!(recovered.scope, injection);
    }

    /// Verify `confidence` maps to `importance` and vice-versa.
    #[test]
    fn confidence_maps_to_importance() {
        let frag = make_fragment("agent");
        let mem = fragment_to_memory(&frag);
        assert!((mem.importance - frag.confidence).abs() < 1e-6);
        let recovered = memory_to_fragment(mem);
        assert!((recovered.confidence - frag.confidence).abs() < 1e-6);
    }

    /// The KNN query binds every caller-derived value; only the clamped numeric `k` is formatted in, because SurrealDB needs it as a literal.
    #[test]
    fn knn_query_template_has_no_inline_caller_values() {
        for agent_filter in [AgentFilter::Strict, AgentFilter::AllowUnowned] {
            let knn = knn_query(5, agent_filter);
            assert!(knn.contains("$vec"), "embedding vector must be bound");
            assert!(knn.contains("$agent_id"), "agent_id must be bound");
            assert!(knn.contains("$peer_id"), "peer_id must be bound");
            assert!(
                knn.contains("LIMIT $k"),
                "limit must stay bound outside the KNN operator"
            );
            assert!(
                knn.contains("embedding <|5,COSINE|> $vec"),
                "KNN operator must use a literal numeric k"
            );
        }
        assert!(!knn_query(5, AgentFilter::Strict).contains("OR agent_id IS NONE OR"));
        assert!(knn_query(5, AgentFilter::AllowUnowned).contains("OR agent_id IS NONE OR"));
    }

    /// The count query binds its filters.
    #[test]
    fn count_query_template_is_parameterized() {
        assert!(COUNT_QUERY.contains("$agent_id"));
        assert!(COUNT_QUERY.contains("$peer_id"));
    }
}
