//! Semantic memory store with vector embedding support.
//!
//! Phase 1: SQLite LIKE matching (fallback when no embeddings).
//! Phase 2: Vector cosine similarity search using stored embeddings.
//!
//! Embeddings are stored as BLOBs in the `embedding` column of the memories table.
//! When a query embedding is provided, recall uses cosine similarity ranking.
//! When no embeddings are available, falls back to LIKE matching.

use chrono::Utc;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    MemoryFilter, MemoryFragment, MemoryId, MemoryModality, MemorySource, VectorStore,
};
// Single canonical impl lives in librefang-types; re-exported here so
// existing `librefang_memory::semantic::cosine_similarity` callers keep
// working without three independently-edited copies drifting (see PR #4125).
pub use librefang_types::memory::cosine_similarity;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{debug, warn};

/// Semantic store backed by SQLite with optional vector search.
///
/// When a [`VectorStore`] backend is provided, vector similarity search in
/// [`recall_with_embedding`](Self::recall_with_embedding) is delegated to that
/// backend instead of doing in-process cosine similarity over SQLite BLOBs.
/// When no backend is set (the default), the original SQLite path is used.
#[derive(Clone)]
pub struct SemanticStore {
    conn: Arc<Mutex<Connection>>,
    vector_store: Option<Arc<dyn VectorStore>>,
}

impl SemanticStore {
    /// Create a new semantic store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            conn,
            vector_store: None,
        }
    }

    /// Create a new semantic store with an external vector backend.
    pub fn new_with_vector_store(
        conn: Arc<Mutex<Connection>>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            conn,
            vector_store: Some(vector_store),
        }
    }

    /// Set or replace the vector store backend at runtime.
    pub fn set_vector_store(&mut self, store: Arc<dyn VectorStore>) {
        self.vector_store = Some(store);
    }

    /// Get a reference to the underlying connection for advanced operations.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Store a new memory fragment (without embedding).
    pub fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<MemoryId> {
        self.remember_with_embedding(
            agent_id,
            content,
            source,
            scope,
            metadata,
            None,
            None,
            None,
            MemoryModality::Text,
        )
    }

    /// Store a new memory fragment with an optional embedding vector and multimodal fields.
    #[allow(clippy::too_many_arguments)]
    pub fn remember_with_embedding(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
        image_url: Option<&str>,
        image_embedding: Option<&[f32]>,
        modality: MemoryModality,
    ) -> LibreFangResult<MemoryId> {
        self.remember_with_embedding_and_peer(
            agent_id,
            content,
            source,
            scope,
            metadata,
            embedding,
            image_url,
            image_embedding,
            modality,
            None,
        )
    }

    /// Store a new memory fragment with optional embedding, multimodal fields, and peer scoping.
    #[allow(clippy::too_many_arguments)]
    pub fn remember_with_embedding_and_peer(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
        image_url: Option<&str>,
        image_embedding: Option<&[f32]>,
        modality: MemoryModality,
        peer_id: Option<&str>,
    ) -> LibreFangResult<MemoryId> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let id = MemoryId::new();
        let now = Utc::now().to_rfc3339();
        let source_str = serde_json::to_string(&source)
            .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
        let meta_str = serde_json::to_string(&metadata)
            .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
        let embedding_bytes: Option<Vec<u8>> = embedding.map(embedding_to_bytes);
        let image_embedding_bytes: Option<Vec<u8>> = image_embedding.map(embedding_to_bytes);
        let modality_str = serde_json::to_string(&modality)
            .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
        // Strip the surrounding quotes from the JSON string (e.g. "\"text\"" -> "text")
        let modality_str = modality_str.trim_matches('"');

        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, embedding, image_url, image_embedding, modality, peer_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6, ?7, ?7, 0, 0, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                id.0.to_string(),
                agent_id.0.to_string(),
                content,
                source_str,
                scope,
                meta_str,
                now,
                embedding_bytes,
                image_url,
                image_embedding_bytes,
                modality_str,
                peer_id,
            ],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(id)
    }

    /// Search for memories using text matching (fallback, no embeddings).
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        self.recall_with_embedding(query, limit, filter, None)
    }

    /// Search for memories using vector similarity when a query embedding is provided,
    /// falling back to LIKE matching otherwise.
    ///
    /// When an external [`VectorStore`] is configured **and** a `query_embedding`
    /// is supplied, the search is delegated to that backend.  The returned IDs
    /// are then hydrated into full [`MemoryFragment`]s from SQLite so the caller
    /// always gets the same rich result type.
    pub fn recall_with_embedding(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        // ── Delegate to external vector store when available ──────────
        if let (Some(vs), Some(qe)) = (&self.vector_store, query_embedding) {
            return self.recall_via_vector_store(vs, qe, limit, filter.clone());
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // Build SQL: fetch candidates (broader than limit for vector re-ranking)
        let fetch_limit = if query_embedding.is_some() {
            // Fetch more candidates for vector search re-ranking
            (limit * 10).max(100)
        } else {
            limit
        };

        let mut sql = String::from(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality
             FROM memories WHERE deleted = 0",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        // Text search filter (only when no embeddings — vector search handles relevance)
        if query_embedding.is_none() && !query.is_empty() {
            sql.push_str(&format!(" AND content LIKE ?{param_idx} ESCAPE '\\'"));
            params.push(Box::new(format!("%{}%", escape_like(query))));
            param_idx += 1;
        }

        // Apply filters
        if let Some(ref f) = filter {
            if let Some(agent_id) = f.agent_id {
                sql.push_str(&format!(" AND agent_id = ?{param_idx}"));
                params.push(Box::new(agent_id.0.to_string()));
                param_idx += 1;
            }
            if let Some(ref scope) = f.scope {
                sql.push_str(&format!(" AND scope = ?{param_idx}"));
                params.push(Box::new(scope.clone()));
                param_idx += 1;
            }
            if let Some(min_conf) = f.min_confidence {
                sql.push_str(&format!(" AND confidence >= ?{param_idx}"));
                params.push(Box::new(min_conf as f64));
                param_idx += 1;
            }
            if let Some(ref source) = f.source {
                let source_str = serde_json::to_string(source)
                    .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
                sql.push_str(&format!(" AND source = ?{param_idx}"));
                params.push(Box::new(source_str));
                param_idx += 1;
            }
            if let Some(ref after) = f.after {
                sql.push_str(&format!(" AND created_at > ?{param_idx}"));
                params.push(Box::new(after.to_rfc3339()));
                param_idx += 1;
            }
            if let Some(ref before) = f.before {
                sql.push_str(&format!(" AND created_at < ?{param_idx}"));
                params.push(Box::new(before.to_rfc3339()));
                param_idx += 1;
            }
            // Metadata filtering via json_extract (keys must be alphanumeric/underscore only)
            for (key, value) in &f.metadata {
                if let Some(s) = value.as_str() {
                    // Reject keys with non-alphanumeric characters to prevent injection
                    if key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        sql.push_str(&format!(
                            " AND json_extract(metadata, '$.{}') = ?{param_idx}",
                            key
                        ));
                        params.push(Box::new(s.to_string()));
                        param_idx += 1;
                    }
                }
            }
            if let Some(ref pid) = f.peer_id {
                sql.push_str(&format!(" AND peer_id = ?{param_idx}"));
                params.push(Box::new(pid.clone()));
                param_idx += 1;
            }
            let _ = param_idx;
        }

        sql.push_str(" ORDER BY confidence DESC, accessed_at DESC, access_count DESC");
        sql.push_str(&format!(" LIMIT {fetch_limit}"));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id_str: String = row.get(0)?;
                let agent_str: String = row.get(1)?;
                let content: String = row.get(2)?;
                let source_str: String = row.get(3)?;
                let scope: String = row.get(4)?;
                let confidence: f64 = row.get(5)?;
                let meta_str: String = row.get(6)?;
                let created_str: String = row.get(7)?;
                let accessed_str: String = row.get(8)?;
                let access_count: i64 = row.get(9)?;
                let embedding_bytes: Option<Vec<u8>> = row.get(10)?;
                let image_url: Option<String> = row.get(11)?;
                let image_embedding_bytes: Option<Vec<u8>> = row.get(12)?;
                let modality_str: Option<String> = row.get(13)?;
                Ok((
                    id_str,
                    agent_str,
                    content,
                    source_str,
                    scope,
                    confidence,
                    meta_str,
                    created_str,
                    accessed_str,
                    access_count,
                    embedding_bytes,
                    image_url,
                    image_embedding_bytes,
                    modality_str,
                ))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut fragments = Vec::new();
        for row_result in rows {
            let (
                id_str,
                agent_str,
                content,
                source_str,
                scope,
                confidence,
                meta_str,
                created_str,
                accessed_str,
                access_count,
                embedding_bytes,
                image_url,
                image_embedding_bytes,
                modality_str,
            ) = row_result.map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let id = uuid::Uuid::parse_str(&id_str)
                .map(MemoryId)
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let agent_id = uuid::Uuid::parse_str(&agent_str)
                .map(librefang_types::agent::AgentId)
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let source: MemorySource =
                serde_json::from_str(&source_str).unwrap_or(MemorySource::System);
            let metadata: HashMap<String, serde_json::Value> =
                serde_json::from_str(&meta_str).unwrap_or_default();
            let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let accessed_at = chrono::DateTime::parse_from_rfc3339(&accessed_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let embedding = embedding_bytes.as_deref().map(embedding_from_bytes);
            let image_embedding = image_embedding_bytes.as_deref().map(embedding_from_bytes);
            let modality: MemoryModality = modality_str
                .as_deref()
                .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
                .unwrap_or_default();

            fragments.push(MemoryFragment {
                id,
                agent_id,
                content,
                embedding,
                metadata,
                source,
                confidence: confidence as f32,
                created_at,
                accessed_at,
                access_count: access_count as u64,
                scope,
                image_url,
                image_embedding,
                modality,
            });
        }

        // If we have a query embedding, re-rank by cosine similarity.
        // Non-comparable vectors (dim mismatch, zero magnitude) sort to
        // the bottom (NEG_INFINITY sentinel) instead of being treated as
        // 0.0, which would have ranked them above genuinely orthogonal
        // hits. We deliberately do NOT use -1.0: that is a valid cosine
        // result for anti-similar vectors and would tie with the
        // "non-comparable" bucket.
        if let Some(qe) = query_embedding {
            fragments.sort_by(|a, b| {
                let sim_a = a
                    .embedding
                    .as_deref()
                    .and_then(|e| cosine_similarity(qe, e))
                    .unwrap_or(f32::NEG_INFINITY);
                let sim_b = b
                    .embedding
                    .as_deref()
                    .and_then(|e| cosine_similarity(qe, e))
                    .unwrap_or(f32::NEG_INFINITY);
                sim_b
                    .partial_cmp(&sim_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            fragments.truncate(limit);
            debug!(
                "Vector recall: {} results from {} candidates",
                fragments.len(),
                fetch_limit
            );
        }

        // Update access counts for returned memories. Logged on failure
        // because the decay/consolidation engine keys TTL decisions off
        // accessed_at — silently losing updates means "active" memories
        // can be garbage-collected when they shouldn't be.
        for frag in &fragments {
            if let Err(e) = conn.execute(
                "UPDATE memories SET access_count = access_count + 1, accessed_at = ?1 WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), frag.id.0.to_string()],
            ) {
                warn!(memory_id = %frag.id.0, error = %e, "Failed to update access tracking");
            }
        }

        Ok(fragments)
    }

    /// Delegate vector search to an external [`VectorStore`] backend, then
    /// hydrate the returned IDs into full [`MemoryFragment`]s from SQLite.
    fn recall_via_vector_store(
        &self,
        vs: &Arc<dyn VectorStore>,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        // VectorStore is async — run inside a small blocking-compatible context.
        let vs = Arc::clone(vs);
        let qe = query_embedding.to_vec();
        let filter_clone = filter.clone();
        let results: Vec<librefang_types::memory::VectorSearchResult> =
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(vs.search(&qe, limit, filter_clone))
            })?;

        debug!(
            "VectorStore ({}) recall: {} results",
            vs.backend_name(),
            results.len()
        );

        // Hydrate full MemoryFragments from SQLite by ID
        let mut fragments = Vec::with_capacity(results.len());
        for r in &results {
            let mem_id = uuid::Uuid::parse_str(&r.id)
                .map(MemoryId)
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            if let Some(frag) = self.get_by_id(mem_id, false)? {
                fragments.push(frag);
            }
        }

        // Update access counts — see note on the SQLite-path update above
        // for why silent drops would corrupt decay logic.
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        for frag in &fragments {
            if let Err(e) = conn.execute(
                "UPDATE memories SET access_count = access_count + 1, accessed_at = ?1 WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), frag.id.0.to_string()],
            ) {
                warn!(memory_id = %frag.id.0, error = %e, "Failed to update access tracking");
            }
        }

        Ok(fragments)
    }

    /// Get a single memory fragment by ID (including soft-deleted ones for history).
    pub fn get_by_id(
        &self,
        id: MemoryId,
        include_deleted: bool,
    ) -> LibreFangResult<Option<MemoryFragment>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let deleted_clause = if include_deleted {
            ""
        } else {
            " AND deleted = 0"
        };
        let sql = format!(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality
             FROM memories WHERE id = ?1{deleted_clause}",
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let result = stmt.query_row(rusqlite::params![id.0.to_string()], |row| {
            let id_str: String = row.get(0)?;
            let agent_str: String = row.get(1)?;
            let content: String = row.get(2)?;
            let source_str: String = row.get(3)?;
            let scope: String = row.get(4)?;
            let confidence: f64 = row.get(5)?;
            let meta_str: String = row.get(6)?;
            let created_str: String = row.get(7)?;
            let accessed_str: String = row.get(8)?;
            let access_count: i64 = row.get(9)?;
            let embedding_bytes: Option<Vec<u8>> = row.get(10)?;
            let image_url: Option<String> = row.get(11)?;
            let image_embedding_bytes: Option<Vec<u8>> = row.get(12)?;
            let modality_str: Option<String> = row.get(13)?;
            Ok((
                id_str,
                agent_str,
                content,
                source_str,
                scope,
                confidence,
                meta_str,
                created_str,
                accessed_str,
                access_count,
                embedding_bytes,
                image_url,
                image_embedding_bytes,
                modality_str,
            ))
        });

        match result {
            Ok((
                id_str,
                agent_str,
                content,
                source_str,
                scope,
                confidence,
                meta_str,
                created_str,
                accessed_str,
                access_count,
                embedding_bytes,
                image_url,
                image_embedding_bytes,
                modality_str,
            )) => {
                let id = uuid::Uuid::parse_str(&id_str)
                    .map(MemoryId)
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let agent_id = uuid::Uuid::parse_str(&agent_str)
                    .map(librefang_types::agent::AgentId)
                    .map_err(|e| LibreFangError::Memory(e.to_string()))?;
                let source: MemorySource =
                    serde_json::from_str(&source_str).unwrap_or(MemorySource::System);
                let metadata: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&meta_str).unwrap_or_default();
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let accessed_at = chrono::DateTime::parse_from_rfc3339(&accessed_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let embedding = embedding_bytes.as_deref().map(embedding_from_bytes);
                let image_embedding = image_embedding_bytes.as_deref().map(embedding_from_bytes);
                let modality: MemoryModality = modality_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
                    .unwrap_or_default();

                Ok(Some(MemoryFragment {
                    id,
                    agent_id,
                    content,
                    embedding,
                    metadata,
                    source,
                    confidence: confidence as f32,
                    created_at,
                    accessed_at,
                    access_count: access_count as u64,
                    scope,
                    image_url,
                    image_embedding,
                    modality,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::Memory(e.to_string())),
        }
    }

    /// Soft-delete a memory fragment.
    pub fn forget(&self, id: MemoryId) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        conn.execute(
            "UPDATE memories SET deleted = 1 WHERE id = ?1",
            rusqlite::params![id.0.to_string()],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Update the content (and optionally metadata) of an existing memory in-place.
    ///
    /// Preserves the original ID, agent_id, scope, source, and access stats.
    /// Updates `accessed_at` to now.
    pub fn update_content(
        &self,
        id: MemoryId,
        new_content: &str,
        new_metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let now = Utc::now().to_rfc3339();
        if let Some(meta) = new_metadata {
            let meta_str = serde_json::to_string(&meta)
                .map_err(|e| LibreFangError::Serialization(e.to_string()))?;
            conn.execute(
                "UPDATE memories SET content = ?1, metadata = ?2, accessed_at = ?3 WHERE id = ?4 AND deleted = 0",
                rusqlite::params![new_content, meta_str, now, id.0.to_string()],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        } else {
            conn.execute(
                "UPDATE memories SET content = ?1, accessed_at = ?2 WHERE id = ?3 AND deleted = 0",
                rusqlite::params![new_content, now, id.0.to_string()],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        }
        Ok(())
    }

    /// Update the embedding for an existing memory.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id.0.to_string()],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Load stored embeddings for a batch of memory IDs.
    ///
    /// Returns a map of `id_string -> embedding_vec`. IDs without stored
    /// embeddings are simply omitted from the result.
    pub fn get_embeddings_batch(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        // SQLite doesn't support IN with parameterized lists easily for large N,
        // so we query one at a time for safety (N ≤ 100 in find_duplicates).
        let mut map = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT embedding FROM memories WHERE id = ?1 AND deleted = 0")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        for id in ids {
            if let Ok(Some(b)) = stmt.query_row(rusqlite::params![*id], |row| {
                let b: Option<Vec<u8>> = row.get(0)?;
                Ok(b)
            }) {
                if !b.is_empty() {
                    map.insert(id.to_string(), embedding_from_bytes(&b));
                }
            }
        }
        Ok(map)
    }

    /// Soft-delete all memories for a specific agent.
    pub fn forget_by_agent(&self, agent_id: AgentId) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string()],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Soft-delete all memories for a specific agent and scope.
    pub fn forget_by_scope(&self, agent_id: AgentId, scope: &str) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND scope = ?2 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string(), scope],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Soft-delete memories older than a given timestamp for a specific agent and scope.
    pub fn forget_older_than(
        &self,
        agent_id: AgentId,
        scope: &str,
        before: chrono::DateTime<Utc>,
    ) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND scope = ?2 AND created_at < ?3 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string(), scope, before.to_rfc3339()],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Soft-delete session memories older than a given timestamp across ALL agents.
    ///
    /// Unlike `forget_older_than`, this is not scoped to a single agent — it cleans up
    /// expired session memories globally, which is useful for periodic TTL enforcement.
    pub fn forget_session_older_than_global(
        &self,
        scope: &str,
        before: chrono::DateTime<Utc>,
    ) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE scope = ?1 AND created_at < ?2 AND deleted = 0",
                rusqlite::params![scope, before.to_rfc3339()],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Count non-deleted memories for a specific agent, optionally filtered by scope.
    pub fn count(&self, agent_id: AgentId, scope: Option<&str>) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count: i64 = if let Some(s) = scope {
            conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE agent_id = ?1 AND scope = ?2 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string(), s],
                |row| row.get(0),
            )
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE agent_id = ?1 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
        }
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Return the IDs of the lowest-confidence memories for a given agent,
    /// ordered by confidence ASC then created_at ASC (oldest first as tiebreaker).
    /// Used by the per-agent memory cap to evict the weakest memories.
    pub fn lowest_confidence(
        &self,
        agent_id: AgentId,
        limit: usize,
    ) -> LibreFangResult<Vec<MemoryId>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id FROM memories WHERE agent_id = ?1 AND deleted = 0 \
                 ORDER BY confidence ASC, created_at ASC LIMIT ?2",
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(
                rusqlite::params![agent_id.0.to_string(), limit as i64],
                |row| {
                    let id_str: String = row.get(0)?;
                    Ok(id_str)
                },
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut ids = Vec::new();
        for row in rows {
            let id_str = row.map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let uuid = uuid::Uuid::parse_str(&id_str)
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            ids.push(MemoryId(uuid));
        }
        Ok(ids)
    }

    /// Count memories across ALL agents, optionally filtered by scope.
    pub fn count_all(&self, scope: Option<&str>) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let count: i64 = if let Some(s) = scope {
            conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE scope = ?1 AND deleted = 0",
                rusqlite::params![s],
                |row| row.get(0),
            )
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE deleted = 0",
                [],
                |row| row.get(0),
            )
        }
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(count as u64)
    }

    /// Count non-deleted memories grouped by category (from JSON metadata).
    ///
    /// For a specific agent, pass `Some(agent_id)`. For global stats, pass `None`.
    /// Uses `json_extract` to avoid loading all rows into memory.
    pub fn count_by_category(
        &self,
        agent_id: Option<AgentId>,
    ) -> LibreFangResult<HashMap<String, usize>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
            if let Some(aid) = agent_id {
                (
                    "SELECT json_extract(metadata, '$.category') AS cat, COUNT(*) \
                     FROM memories WHERE agent_id = ?1 AND deleted = 0 \
                     AND json_extract(metadata, '$.category') IS NOT NULL \
                     GROUP BY cat"
                        .to_string(),
                    vec![Box::new(aid.0.to_string())],
                )
            } else {
                (
                    "SELECT json_extract(metadata, '$.category') AS cat, COUNT(*) \
                     FROM memories WHERE deleted = 0 \
                     AND json_extract(metadata, '$.category') IS NOT NULL \
                     GROUP BY cat"
                        .to_string(),
                    vec![],
                )
            };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let cat: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((cat, count as usize))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut map = HashMap::new();
        for row in rows {
            let (cat, count) = row.map_err(|e| LibreFangError::Memory(e.to_string()))?;
            map.insert(cat, count);
        }
        Ok(map)
    }
}

/// Escape LIKE special characters (`%`, `_`, `\`) in user-supplied search strings.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Serialize embedding to bytes for SQLite BLOB storage.
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize embedding from bytes.
fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// SqliteVectorStore — VectorStore trait implementation for SQLite backend
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use librefang_types::memory::VectorSearchResult;

/// SQLite-backed vector store (the default backend).
///
/// Uses BLOB-serialized embeddings and in-process cosine similarity
/// re-ranking. Suitable for single-node deployments with moderate
/// memory counts (< 100k vectors).
///
/// For larger-scale or production deployments, implement the `VectorStore`
/// trait for a dedicated vector database (Qdrant, Pinecone, Chroma, etc.).
#[derive(Clone)]
pub struct SqliteVectorStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteVectorStore {
    /// Create a new SQLite vector store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl VectorStore for SqliteVectorStore {
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        _payload: &str,
        _metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<librefang_types::memory::MemoryFilter>,
    ) -> LibreFangResult<Vec<VectorSearchResult>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let fetch_limit = (limit * 10).max(100);
        let mut sql = String::from(
            "SELECT id, content, metadata, embedding FROM memories WHERE deleted = 0 AND embedding IS NOT NULL",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(ref f) = filter {
            if let Some(agent_id) = f.agent_id {
                sql.push_str(&format!(" AND agent_id = ?{param_idx}"));
                params.push(Box::new(agent_id.0.to_string()));
                param_idx += 1;
            }
            if let Some(ref scope) = f.scope {
                sql.push_str(&format!(" AND scope = ?{param_idx}"));
                params.push(Box::new(scope.clone()));
                param_idx += 1;
            }
            let _ = param_idx;
        }

        sql.push_str(&format!(" LIMIT {fetch_limit}"));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id: String = row.get(0)?;
                let content: String = row.get(1)?;
                let meta_str: String = row.get(2)?;
                let emb_bytes: Vec<u8> = row.get(3)?;
                Ok((id, content, meta_str, emb_bytes))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        let mut skipped_non_comparable: u64 = 0;
        for row_result in rows {
            let (id, content, meta_str, emb_bytes) =
                row_result.map_err(|e| LibreFangError::Memory(e.to_string()))?;
            let emb = embedding_from_bytes(&emb_bytes);
            // Skip non-comparable rows (dim mismatch from re-embedding,
            // zero vector). Including them with score=0.0 would let them
            // outrank genuinely orthogonal hits and pollute the result set.
            let Some(score) = cosine_similarity(query_embedding, &emb) else {
                // Per-row stays at debug to avoid flooding logs during a
                // re-embedding migration; the loop emits one aggregated
                // warn at the end if any were skipped.
                tracing::debug!(
                    memory_id = %id,
                    "skipping vector candidate: dim mismatch or zero magnitude"
                );
                skipped_non_comparable += 1;
                continue;
            };
            let metadata: HashMap<String, serde_json::Value> =
                serde_json::from_str(&meta_str).unwrap_or_default();
            results.push(VectorSearchResult {
                id,
                payload: content,
                score,
                metadata,
            });
        }
        if skipped_non_comparable > 0 {
            tracing::warn!(
                count = skipped_non_comparable,
                "vector search skipped non-comparable candidates (dim mismatch or zero magnitude); \
                 likely a re-embedding migration is in progress"
            );
        }

        // Sort by score descending, truncate to limit
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    async fn delete(&self, id: &str) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        conn.execute(
            "UPDATE memories SET embedding = NULL WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    async fn get_embeddings(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let mut map = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT embedding FROM memories WHERE id = ?1 AND deleted = 0")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        for id in ids {
            if let Ok(Some(b)) = stmt.query_row(rusqlite::params![*id], |row| {
                let b: Option<Vec<u8>> = row.get(0)?;
                Ok(b)
            }) {
                if !b.is_empty() {
                    map.insert(id.to_string(), embedding_from_bytes(&b));
                }
            }
        }
        Ok(map)
    }

    fn backend_name(&self) -> &str {
        "sqlite"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> SemanticStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        SemanticStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_remember_and_recall() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "The user likes Rust programming",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let results = store.recall("Rust", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn test_recall_with_filter() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "Memory A",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                AgentId::new(),
                "Memory B",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let filter = MemoryFilter::agent(agent_id);
        let results = store.recall("Memory", 10, Some(filter)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Memory A");
    }

    #[test]
    fn test_recall_with_peer_filter_isolates_users() {
        // Regression for per-peer memory isolation (#2058 follow-up).
        // Two users A and B share an agent; recalling with peer_id=Some("A")
        // must not return B's memories.
        let store = setup();
        let agent_id = AgentId::new();
        let _ = store
            .remember_with_embedding_and_peer(
                agent_id,
                "Alice likes dark roast coffee",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                None,
                None,
                None,
                Default::default(),
                Some("user-A"),
            )
            .unwrap();
        let _ = store
            .remember_with_embedding_and_peer(
                agent_id,
                "Bob likes dark roast coffee",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                None,
                None,
                None,
                Default::default(),
                Some("user-B"),
            )
            .unwrap();

        // Query as user A — should only see Alice's memory.
        let mut f = MemoryFilter::agent(agent_id);
        f.peer_id = Some("user-A".into());
        let results = store.recall("coffee", 10, Some(f)).unwrap();
        assert_eq!(
            results.len(),
            1,
            "user-A must not see user-B's memory, got: {:?}",
            results.iter().map(|r| &r.content).collect::<Vec<_>>()
        );
        assert!(results[0].content.starts_with("Alice"));

        // Query as user B — should only see Bob's memory.
        let mut f = MemoryFilter::agent(agent_id);
        f.peer_id = Some("user-B".into());
        let results = store.recall("coffee", 10, Some(f)).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.starts_with("Bob"));
    }

    #[test]
    fn test_forget() {
        let store = setup();
        let agent_id = AgentId::new();
        let id = store
            .remember(
                agent_id,
                "To forget",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store.forget(id).unwrap();
        let results = store.recall("To forget", 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_remember_with_embedding() {
        let store = setup();
        let agent_id = AgentId::new();
        let embedding = vec![0.1, 0.2, 0.3, 0.4];
        let id = store
            .remember_with_embedding(
                agent_id,
                "Rust is great",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&embedding),
                None,
                None,
                Default::default(),
            )
            .unwrap();
        assert_ne!(id.0.to_string(), "");
    }

    #[test]
    fn test_vector_recall_ranking() {
        let store = setup();
        let agent_id = AgentId::new();

        // Store 3 memories with embeddings pointing in different directions
        let emb_rust = vec![0.9, 0.1, 0.0, 0.0]; // "Rust" direction
        let emb_python = vec![0.0, 0.0, 0.9, 0.1]; // "Python" direction
        let emb_mixed = vec![0.5, 0.5, 0.0, 0.0]; // mixed

        store
            .remember_with_embedding(
                agent_id,
                "Rust is a systems language",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_rust),
                None,
                None,
                Default::default(),
            )
            .unwrap();
        store
            .remember_with_embedding(
                agent_id,
                "Python is interpreted",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_python),
                None,
                None,
                Default::default(),
            )
            .unwrap();
        store
            .remember_with_embedding(
                agent_id,
                "Both are popular",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&emb_mixed),
                None,
                None,
                Default::default(),
            )
            .unwrap();

        // Query with a "Rust"-like embedding
        let query_emb = vec![0.85, 0.15, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 3, None, Some(&query_emb))
            .unwrap();

        assert_eq!(results.len(), 3);
        // Rust memory should be first (highest cosine similarity)
        assert!(results[0].content.contains("Rust"));
        // Python memory should be last (lowest similarity)
        assert!(results[2].content.contains("Python"));
    }

    #[test]
    fn test_update_embedding() {
        let store = setup();
        let agent_id = AgentId::new();
        let id = store
            .remember(
                agent_id,
                "No embedding yet",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // Update with embedding
        let emb = vec![1.0, 0.0, 0.0];
        store.update_embedding(id, &emb).unwrap();

        // Verify the embedding is stored by doing vector recall
        let query_emb = vec![1.0, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 10, None, Some(&query_emb))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].embedding.is_some());
        assert_eq!(results[0].embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_mixed_embedded_and_non_embedded() {
        let store = setup();
        let agent_id = AgentId::new();

        // One memory with embedding, one without
        store
            .remember_with_embedding(
                agent_id,
                "Has embedding",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&[1.0, 0.0]),
                None,
                None,
                Default::default(),
            )
            .unwrap();
        store
            .remember(
                agent_id,
                "No embedding",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // Vector recall should rank embedded memory higher
        let results = store
            .recall_with_embedding("", 10, None, Some(&[1.0, 0.0]))
            .unwrap();
        assert_eq!(results.len(), 2);
        // Embedded memory should rank first
        assert_eq!(results[0].content, "Has embedding");
    }

    #[test]
    fn test_forget_by_agent() {
        let store = setup();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        store
            .remember(
                agent_a,
                "Agent A memory 1",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent_a,
                "Agent A memory 2",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent_b,
                "Agent B memory",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();

        let deleted = store.forget_by_agent(agent_a).unwrap();
        assert_eq!(deleted, 2);

        // Agent A memories should be gone
        let results = store
            .recall("Agent A", 10, Some(MemoryFilter::agent(agent_a)))
            .unwrap();
        assert!(results.is_empty());

        // Agent B memory should remain
        let results = store
            .recall("Agent B", 10, Some(MemoryFilter::agent(agent_b)))
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_forget_by_scope() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .remember(
                agent_id,
                "Session mem",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent_id,
                "User mem",
                MemorySource::Conversation,
                "user_memory",
                HashMap::new(),
            )
            .unwrap();

        let deleted = store.forget_by_scope(agent_id, "session_memory").unwrap();
        assert_eq!(deleted, 1);

        // User memory should remain
        let results = store
            .recall("User mem", 10, Some(MemoryFilter::agent(agent_id)))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].scope, "user_memory");
    }

    #[test]
    fn test_count() {
        let store = setup();
        let agent_id = AgentId::new();

        assert_eq!(store.count(agent_id, None).unwrap(), 0);

        store
            .remember(
                agent_id,
                "Mem 1",
                MemorySource::Conversation,
                "session_memory",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent_id,
                "Mem 2",
                MemorySource::Conversation,
                "user_memory",
                HashMap::new(),
            )
            .unwrap();

        assert_eq!(store.count(agent_id, None).unwrap(), 2);
        assert_eq!(store.count(agent_id, Some("session_memory")).unwrap(), 1);
        assert_eq!(store.count(agent_id, Some("user_memory")).unwrap(), 1);
        assert_eq!(store.count(agent_id, Some("agent_memory")).unwrap(), 0);
    }
}
