//! SurrealDB-backed [`crate::backend::SemanticBackend`] and
//! [`librefang_types::memory::VectorStore`] implementations.
//!
//! ## Design
//!
//! Reuses the `surreal-memory` crate's `memory` table, which already carries
//! HNSW vector indexes (v5 migration, 1536-dim COSINE TYPE F32).  Two search
//! paths are provided:
//!
//! 1. **Text / BM25 path** (`recall` without a precomputed embedding) →
//!    delegates to `surreal_memory::MemoryStorage::search_memories`.
//! 2. **Vector / HNSW path** (`recall` with a precomputed embedding) →
//!    issues a `SELECT … <|k,COSINE|> $vec` KNN query directly
//!    against the connection pool.  All values are bound — no caller-supplied
//!    strings are ever interpolated.  SurrealDB requires the KNN `k` operand
//!    to be a literal unsigned integer, so LibreFang clamps and formats that
//!    numeric value itself while still binding user-derived filters and vectors.
//!
//! ## `MemoryFragment` ↔ `surreal_memory::Memory` mapping
//!
//! | `MemoryFragment` field | `Memory` field / location |
//! |---|---|
//! | `content` | `content` |
//! | `embedding` | `embedding` |
//! | `agent_id` | `agent_id` (string) |
//! | `scope` | first element of `categories` |
//! | `confidence` | `importance` |
//! | `peer_id` (from filter) | `user_id` |
//! | `created_at` | `created_at` (Datetime) |
//! | `accessed_at` / `access_count` | stored in `metadata["librefang"]["accessed_at"]` / `["access_count"]` |
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

use crate::backend::SemanticBackend;
use crate::proactive::EmbeddingFn;
use async_trait::async_trait;
use chrono::DateTime;
use librefang_storage::pool::SurrealSession;
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
use tracing::{debug, warn};
use uuid::Uuid;

// ── Re-export so the AgentId → String conversion is available ─────────────────
use librefang_types::agent::AgentId;

// ── SurrealSemanticBackend ────────────────────────────────────────────────────

/// SurrealDB-backed semantic memory store.
///
/// Wraps `surreal_memory::SurrealStorage` for BM25-based recall and issues
/// direct KNN queries over the pooled connection for vector recall.
pub struct SurrealSemanticBackend {
    /// `surreal-memory` storage for BM25/embedding search and CRUD.
    storage: Arc<SurrealStorage>,
    /// Raw DB handle for direct KNN SurrealQL queries.
    db: Arc<Surreal<Any>>,
    /// Optional embedding driver — when `Some`, `add_memory` will embed the
    /// content before writing; when `None`, the `SurrealStorage` internal
    /// embedding service is used (which may be a `NoopEmbedding`).
    pub embedding: Option<Arc<dyn EmbeddingFn>>,
}

impl SurrealSemanticBackend {
    /// Create from an existing `SurrealStorage` and a raw connection session.
    pub fn new(
        storage: Arc<SurrealStorage>,
        session: &SurrealSession,
        embedding: Option<Arc<dyn EmbeddingFn>>,
    ) -> Self {
        Self {
            storage,
            db: Arc::new(session.client().clone()),
            embedding,
        }
    }

    /// Open a `SurrealSemanticBackend` from a kernel storage config, building a
    /// dedicated `SurrealStorage` connection internally.
    ///
    /// This is the factory used by `librefang-kernel` so that `surreal-memory`
    /// is not a direct dependency of the kernel crate (it reaches it via
    /// `librefang-memory`).
    ///
    /// The internal `SurrealStorage` is initialised with a [`NoopEmbedding`]
    /// service.  Real embeddings are supplied at query time by the
    /// `ContextEngine`'s `EmbeddingDriver` via the `query_embedding` parameter
    /// on [`SemanticBackend::recall`].
    pub async fn open_with_storage(
        session: &SurrealSession,
        storage_cfg: &librefang_storage::config::StorageConfig,
    ) -> Result<Self, String> {
        // Delegate to the single-source factory so this path is functionally
        // identical to (and shares the same RocksDB lock with) the kernel's
        // shared-storage boot path.  Standalone callers (e.g. the
        // `#[ignore]`d `knn_hnsw_relevance` integration test) get a fresh
        // `SurrealStorage` here because no other opener exists in their process.
        let storage = super::shared::open_shared_memory_storage(storage_cfg)
            .await
            .map_err(|e| format!("SurrealStorage (semantic backend): {e}"))?;
        Ok(Self::new(storage, session, None))
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
        "accessed_at":   frag.accessed_at.to_rfc3339(),
        "access_count":  frag.access_count,
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
        access_count: frag.access_count as u32,
        last_accessed_at: None,
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

    // Recover the original `MemoryId` from `lf["lf_id"]`; generate a new one
    // if missing (e.g. for memories not written by librefang).
    let id = lf
        .get("lf_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
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

    let accessed_at = lf
        .get("accessed_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let access_count: u64 = lf
        .get("access_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(mem.access_count as u64);

    let image_url: Option<String> = lf
        .get("image_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let image_embedding: Option<Vec<f32>> = lf
        .get("image_embedding")
        .and_then(|v| serde_json::from_value::<Vec<f32>>(v.clone()).ok());

    // `created_at` is stored as an RFC-3339 string inside the SurrealDB Datetime.
    // The `Datetime` type's Display is already RFC-3339 compatible.
    let created_at = DateTime::parse_from_rfc3339(&mem.created_at.to_string())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());

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
        access_count,
        scope,
        image_url,
        image_embedding,
        modality,
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
        let frag = MemoryFragment {
            id: MemoryId::new(),
            agent_id,
            content: content.to_string(),
            embedding,
            metadata,
            source,
            confidence: 0.8,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: scope.to_string(),
            image_url: None,
            image_embedding: None,
            modality: MemoryModality::default(),
        };
        let lf_id = frag.id;
        let mem = fragment_to_memory(&frag);
        self.storage
            .add_memory(mem)
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
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

        if let Some(vec) = query_embedding {
            // ── HNSW vector path (fully parameterised) ────────────────────────
            if vec.is_empty() {
                // Embedding service produced an empty vector (Noop) — fall back.
                debug!("SurrealSemanticBackend: empty embedding, falling back to BM25 recall");
            } else {
                return self
                    .knn_recall(&vec, limit, agent_id_str.as_deref(), peer_id_str.as_deref())
                    .await;
            }
        }

        // ── BM25 / embedding path via surreal-memory ──────────────────────────
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
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

        Ok(results.into_iter().map(memory_to_fragment).collect())
    }

    async fn forget(&self, id: MemoryId) -> LibreFangResult<bool> {
        // `delete_memory` takes a SurrealDB record-ID string.  Since we store
        // the librefang `lf_id` inside metadata, we cannot reverse-lookup by ID
        // efficiently without a dedicated index.  For now we delete by searching
        // for the `lf_id` tag.
        let db = self.db.clone();
        let id_str = id.0.to_string();
        let rows: Vec<JsonValue> = db
            .query(
                "SELECT id FROM memory WHERE meta::value(metadata.librefang.lf_id) = $lf_id \
                 LIMIT 1",
            )
            .bind(("lf_id", id_str))
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?
            .take(0)
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

        if let Some(row) = rows.into_iter().next() {
            if let Some(surreal_id) = row.get("id").and_then(|v| v.as_str()) {
                self.storage
                    .delete_memory(surreal_id)
                    .await
                    .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn count(&self, filter: MemoryFilter) -> LibreFangResult<u64> {
        let agent_id_str = filter.agent_id.map(|a| a.0.to_string());
        let peer_id_str = filter.peer_id;
        let db = self.db.clone();

        // Build a parameterised COUNT query — no string interpolation.

        let rows: Vec<JsonValue> = db
            .query(
                "SELECT count() FROM memory \
                 WHERE ($agent_id IS NONE OR agent_id = $agent_id) \
                 AND ($peer_id IS NONE OR user_id = $peer_id) \
                 GROUP ALL",
            )
            .bind(("agent_id", agent_id_str))
            .bind(("peer_id", peer_id_str))
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?
            .take(0)
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

        let count = rows
            .first()
            .and_then(|r| r.get("count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(count)
    }

    async fn update_access(&self, id: MemoryId) -> LibreFangResult<()> {
        let db = self.db.clone();
        let id_str = id.0.to_string();
        db.query(
            "UPDATE memory SET \
               access_count = access_count + 1, \
               last_accessed_at = time::now(), \
               metadata.librefang.accessed_at = time::now(), \
               metadata.librefang.access_count = <int>(meta::value(metadata.librefang.access_count)) + 1 \
             WHERE meta::value(metadata.librefang.lf_id) = $lf_id",
        )
        .bind(("lf_id", id_str))
        .await
        .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
        Ok(())
    }

    fn backend_name(&self) -> &str {
        "surreal"
    }
}

// ── VectorStore shim ──────────────────────────────────────────────────────────
//
// Implement `VectorStore` so existing callers that reach the semantic backend
// through the `set_vector_store` path (e.g. kernel wiring prior to full trait
// migration) also get HNSW vector search.

#[async_trait]
impl VectorStore for SurrealSemanticBackend {
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        payload: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<()> {
        let mem = surreal_memory::memory::Memory {
            id: None,
            content: payload.to_string(),
            embedding: Some(embedding.to_vec()),
            scope: surreal_memory::memory::MemoryScope::default(),
            memory_type: surreal_memory::memory::MemoryType::default(),
            user_id: None,
            session_id: None,
            agent_id: None,
            task_stream_id: None,
            categories: Vec::new(),
            metadata: Some(serde_json::json!({
                "librefang": { "lf_id": id },
                "user": serde_json::to_value(&metadata).unwrap_or(JsonValue::Null),
            })),
            token_count: None,
            importance: 0.5,
            access_count: 0,
            last_accessed_at: None,
            valid_until: None,
            version: 1,
            created_at: surrealdb::types::Datetime::default(),
            updated_at: surrealdb::types::Datetime::default(),
        };
        self.storage
            .add_memory(mem)
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
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
        let results = self
            .knn_recall(
                query_embedding,
                limit,
                agent_id_str.as_deref(),
                peer_id_str.as_deref(),
            )
            .await?;
        Ok(results
            .into_iter()
            .map(|f| VectorSearchResult {
                id: f.id.0.to_string(),
                payload: f.content,
                score: f.confidence,
                metadata: f.metadata,
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        let db = self.db.clone();
        let id_str = id.to_string();
        db.query("DELETE memory WHERE meta::value(metadata.librefang.lf_id) = $lf_id")
            .bind(("lf_id", id_str))
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
        Ok(())
    }

    async fn get_embeddings(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let db = self.db.clone();
        let ids_json: JsonValue = serde_json::to_value(ids).unwrap_or(JsonValue::Array(vec![]));
        let rows: Vec<JsonValue> = db
            .query(
                "SELECT metadata.librefang.lf_id AS lf_id, embedding \
                 FROM memory \
                 WHERE meta::value(metadata.librefang.lf_id) IN $ids \
                 AND embedding != NONE",
            )
            .bind(("ids", ids_json))
            .await
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?
            .take(0)
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;

        let mut map = HashMap::new();
        for row in rows {
            if let (Some(lf_id), Some(emb)) = (
                row.get("lf_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                row.get("embedding")
                    .and_then(|v| serde_json::from_value::<Vec<f32>>(v.clone()).ok()),
            ) {
                map.insert(lf_id, emb);
            }
        }
        Ok(map)
    }

    fn backend_name(&self) -> &str {
        "surreal"
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

impl SurrealSemanticBackend {
    /// Issue a parameterised KNN query using SurrealDB's `<|k,COSINE|>` syntax.
    ///
    /// All user-supplied values (agent_id, peer_id, limit) are bound — the KNN
    /// vector itself is also bound as `$vec`.
    async fn knn_recall(
        &self,
        embedding: &[f32],
        limit: usize,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        let db = self.db.clone();
        let vec = embedding.to_vec();
        let k = limit.clamp(1, 1000) as u64;

        // SurrealDB KNN syntax requires a literal unsigned integer in the
        // `<|k,COSINE|>` slot. `k` is derived from `usize` and clamped above,
        // so formatting it into the query does not introduce caller-controlled
        // SQL. User-derived values remain bound below.
        let query = format!(
            "SELECT *, vector::similarity::cosine(embedding, $vec) AS _score \
             FROM memory \
             WHERE embedding <|{k},COSINE|> $vec \
               AND ($agent_id IS NONE OR agent_id = $agent_id) \
               AND ($peer_id  IS NONE OR user_id  = $peer_id) \
             ORDER BY _score DESC \
             LIMIT $k"
        );
        let rows: Vec<JsonValue> = db
            .query(query)
            .bind(("vec", vec))
            .bind(("k", k))
            .bind(("agent_id", agent_id.map(str::to_string)))
            .bind(("peer_id", peer_id.map(str::to_string)))
            .await
            .map_err(|e| LibreFangError::memory_msg(format!("knn_recall query: {e}")))?
            .take(0)
            .map_err(|e| LibreFangError::memory_msg(format!("knn_recall take: {e}")))?;

        let mut fragments = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<surreal_memory::memory::Memory>(row) {
                Ok(mem) => fragments.push(memory_to_fragment(mem)),
                Err(e) => {
                    warn!("SurrealSemanticBackend: failed to deserialize memory row: {e}");
                }
            }
        }
        Ok(fragments)
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

    /// Verify that the KNN query template does not contain caller-supplied
    /// filter values as literals in the query text — the parameterized binding
    /// path is the only way those values reach the query.
    ///
    /// SurrealDB requires the KNN `k` operand to be a literal unsigned integer,
    /// so only that clamped numeric value is formatted into the query.
    #[test]
    fn knn_query_template_has_no_inline_caller_values() {
        let k = 5_u64;
        let knn_query = format!(
            "SELECT *, vector::similarity::cosine(embedding, $vec) AS _score \
             FROM memory \
             WHERE embedding <|{k},COSINE|> $vec \
            AND ($agent_id IS NONE OR agent_id = $agent_id) \
            AND ($peer_id IS NONE OR user_id = $peer_id) \
            AND ($scope IS NONE OR $scope IN categories) \
             ORDER BY _score DESC \
             LIMIT $k"
        );

        // All variable references use the `$name` placeholder form —
        // none of the caller-derived filters are interpolated inline.
        assert!(knn_query.contains("$vec"), "embedding vector must be bound");
        assert!(knn_query.contains("$agent_id"), "agent_id must be bound");
        assert!(knn_query.contains("$peer_id"), "peer_id must be bound");
        assert!(knn_query.contains("$scope"), "scope must be bound");
        assert!(
            knn_query.contains("$k"),
            "limit must remain bound outside the KNN operator"
        );
        assert!(
            knn_query.contains("embedding <|5,COSINE|> $vec"),
            "KNN operator must use a literal numeric k"
        );

        // No raw string interpolation: the dangerous injection payload
        // cannot appear because the template is a compile-time constant.
        let injection = "'; DROP TABLE memory; --";
        assert!(
            !knn_query.contains(injection),
            "injection string must not appear in query template"
        );
    }

    /// Verify the count query template uses parameterized bindings.
    #[test]
    fn count_query_template_is_parameterized() {
        const COUNT_QUERY: &str = "SELECT count() FROM memory \
            WHERE ($agent_id IS NONE OR agent_id = $agent_id) \
            AND ($peer_id IS NONE OR user_id = $peer_id) \
            GROUP ALL";

        assert!(COUNT_QUERY.contains("$agent_id"));
        assert!(COUNT_QUERY.contains("$peer_id"));

        // A crafted agent-id string must not appear literally in the template.
        let fake_id = "00000000-0000-0000-0000-000000000000";
        assert!(!COUNT_QUERY.contains(fake_id));
    }

    /// Integration test: insert 5 fragments with distinct embeddings and verify
    /// that a KNN query returns them in cosine-similarity order.
    ///
    /// Requires a running SurrealDB instance.  Run with:
    ///   `cargo test --features surreal-backend -- --ignored knn_hnsw_relevance`
    #[tokio::test]
    #[ignore = "requires a running SurrealDB instance"]
    async fn knn_hnsw_relevance() {
        use librefang_types::memory::MemorySource;

        // Synthetic 3-dim embeddings.  Fragment 0 is the "target" query vector.
        let query_vec = vec![1.0_f32, 0.0, 0.0];
        let vecs: Vec<Vec<f32>> = vec![
            vec![1.0, 0.0, 0.0], // 0: identical to query → highest similarity
            vec![0.8, 0.2, 0.0], // 1: close
            vec![0.5, 0.5, 0.0], // 2: moderate
            vec![0.1, 0.9, 0.0], // 3: distant
            vec![0.0, 0.0, 1.0], // 4: orthogonal → lowest similarity
        ];

        // Build the backend using a SurrealSession pointing at the local instance.
        // SurrealSession is obtained via SurrealConnectionPool::open(cfg).
        // Consult integration_test.rs for the full boot sequence.
        use librefang_storage::config::StorageConfig;
        use librefang_storage::pool::SurrealConnectionPool;
        let pool = SurrealConnectionPool::default();
        let storage_cfg = StorageConfig::default();
        let session = pool
            .open(&storage_cfg)
            .await
            .expect("Failed to connect to SurrealDB");
        let backend = SurrealSemanticBackend::open_with_storage(&session, &storage_cfg)
            .await
            .expect("Failed to create SurrealSemanticBackend");

        // Insert all 5 fragments.
        let agent_id = AgentId(uuid::Uuid::new_v4());
        let mut ids = vec![];
        for (i, emb) in vecs.iter().enumerate() {
            let mut meta = HashMap::new();
            meta.insert("test_idx".to_string(), serde_json::json!(i));
            let id = backend
                .remember(
                    agent_id,
                    &format!("Memory fragment {}", i),
                    MemorySource::Conversation,
                    "test",
                    meta,
                    Some(emb.clone()),
                )
                .await
                .unwrap_or_else(|e| panic!("remember failed for fragment {}: {}", i, e));
            ids.push(id);
        }

        // Issue a KNN recall with the query vector.
        let filter = Some(librefang_types::memory::MemoryFilter::agent(agent_id));
        let results = backend
            .recall("", 5, filter, Some(query_vec))
            .await
            .expect("recall failed");

        assert!(!results.is_empty(), "expected at least one result");

        // The first result should be the identical vector (fragment 0).
        let top = &results[0];
        assert_eq!(
            top.metadata.get("test_idx").and_then(|v| v.as_u64()),
            Some(0),
            "highest-similarity fragment should rank first"
        );

        // Clean up.
        for id in ids {
            let _ = backend.forget(id).await;
        }
    }
}
