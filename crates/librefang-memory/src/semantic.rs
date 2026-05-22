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
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
// Single canonical impl lives in librefang-types; re-exported here so
// existing `librefang_memory::semantic::cosine_similarity` callers keep
// working without three independently-edited copies drifting (see PR #4125).
pub use librefang_types::memory::cosine_similarity;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, warn};

/// Semantic store backed by SQLite with optional vector search.
///
/// When a [`VectorStore`] backend is provided, vector similarity search in
/// [`recall_with_embedding`](Self::recall_with_embedding) is delegated to that
/// backend instead of doing in-process cosine similarity over SQLite BLOBs.
/// When no backend is set (the default), the original SQLite path is used.
#[derive(Clone)]
pub struct SemanticStore {
    pool: Pool<SqliteConnectionManager>,
    vector_store: Option<Arc<dyn VectorStore>>,
}

impl SemanticStore {
    /// Create a new semantic store wrapping the given connection pool.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            vector_store: None,
        }
    }

    /// Create a new semantic store with an external vector backend.
    pub fn new_with_vector_store(
        pool: Pool<SqliteConnectionManager>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            pool,
            vector_store: Some(vector_store),
        }
    }

    /// Set or replace the vector store backend at runtime.
    pub fn set_vector_store(&mut self, store: Arc<dyn VectorStore>) {
        self.vector_store = Some(store);
    }

    /// Get a reference to the underlying connection for advanced operations.
    pub fn pool(&self) -> &Pool<SqliteConnectionManager> {
        &self.pool
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let id = MemoryId::new();
        let now = Utc::now().to_rfc3339();
        let source_str = serde_json::to_string(&source).map_err(LibreFangError::serialization)?;
        let meta_str = serde_json::to_string(&metadata).map_err(LibreFangError::serialization)?;
        let embedding_bytes: Option<Vec<u8>> = embedding.map(embedding_to_bytes);
        let image_embedding_bytes: Option<Vec<u8>> = image_embedding.map(embedding_to_bytes);
        let modality_str =
            serde_json::to_string(&modality).map_err(LibreFangError::serialization)?;
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
        .map_err(LibreFangError::memory)?;
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

        // mut: needed for the `transaction()` call inside
        // `bump_recall_access_counts` after the read is done. The
        // read-side `stmt` borrow is explicitly dropped below
        // before that borrow occurs.
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

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
                let source_str =
                    serde_json::to_string(source).map_err(LibreFangError::serialization)?;
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

        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;

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
            .map_err(LibreFangError::memory)?;

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
            ) = row_result.map_err(LibreFangError::memory)?;

            let id = uuid::Uuid::parse_str(&id_str)
                .map(MemoryId)
                .map_err(LibreFangError::memory)?;
            let agent_id = uuid::Uuid::parse_str(&agent_str)
                .map(librefang_types::agent::AgentId)
                .map_err(LibreFangError::memory)?;
            let source: MemorySource =
                serde_json::from_str(&source_str).unwrap_or(MemorySource::System);
            // Refuse to silently substitute `HashMap::default()` for a TEXT
            // blob we cannot parse — that disguises corruption (manual SQL
            // edit, pre-#3451 FTS bug, serde drift) as "no metadata". Skip
            // the row with a loud log so the operator can audit / repair it
            // (audit: json-text-silent-parse-fallback).
            let metadata: HashMap<String, serde_json::Value> = match serde_json::from_str(&meta_str)
            {
                Ok(m) => m,
                Err(e) => {
                    error!(
                        row_id = %id_str,
                        table = "memories",
                        column = "metadata",
                        error = %e,
                        "corrupt JSON in TEXT column; skipping row in recall"
                    );
                    continue;
                }
            };
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

        // Drop the prepared SELECT explicitly so `conn` is no
        // longer borrowed below — we need a mutable borrow to open
        // the access-count transaction. (NLL would keep `stmt`
        // alive to end-of-scope otherwise; the explicit drop is
        // cheaper than restructuring the entire read into a sub-
        // block.)
        drop(stmt);

        // Bump access_count + accessed_at on recalled fragments
        // (audit: memory-recall-n+1-update). Pre-fix this was a
        // per-row `conn.execute` with no transaction wrapper, which
        // forced WAL fsync once per recalled fragment — at 100
        // recalls per tool-augmented turn the latency dominated the
        // recall path. Now wrapped in a single transaction +
        // prepared statement so all UPDATEs amortise to one WAL
        // fsync. The decay/consolidation engine keys TTL decisions
        // off `accessed_at`, so this MUST persist; the helper keeps
        // the per-row warn-on-failure log so silent loss of a
        // single row's bump (e.g. transient SQLite lock) still
        // surfaces.
        bump_recall_access_counts(&mut conn, &fragments);

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

        // Hydrate full MemoryFragments from SQLite by ID. Pre-fix
        // this was K calls to `get_by_id`, each opening a
        // pool connection + preparing a statement (audit:
        // memory-recall-n+1-update — second sub-finding). At K=50
        // that was 50 round-trips for what is a single SELECT
        // WHERE id IN (?,?,...). Parse all ANN-returned ids first
        // (so a single malformed UUID fails the whole hydrate
        // rather than silently skipping), then issue one batched
        // SELECT. The batch preserves the ANN ranking order by
        // re-ordering against the input vec after fetch.
        let mut ordered_ids: Vec<MemoryId> = Vec::with_capacity(results.len());
        for r in &results {
            let mem_id = uuid::Uuid::parse_str(&r.id)
                .map(MemoryId)
                .map_err(LibreFangError::memory)?;
            ordered_ids.push(mem_id);
        }
        let mut by_id = self.get_by_ids_batch(&ordered_ids, false)?;
        let mut fragments: Vec<MemoryFragment> = Vec::with_capacity(ordered_ids.len());
        for mem_id in &ordered_ids {
            if let Some(frag) = by_id.remove(mem_id) {
                fragments.push(frag);
            }
        }

        // Update access counts — see note on the SQLite-path
        // update above for why silent drops would corrupt decay
        // logic. Same tx-wrapped helper. The vector-store branch
        // has no other live conn handle at this point, so we
        // acquire one for the write.
        if let Ok(mut write_conn) = self.pool.get() {
            bump_recall_access_counts(&mut write_conn, &fragments);
        } else {
            warn!("memory recall (vector store): pool.get() for access-count bump failed");
        }

        Ok(fragments)
    }

    /// Batch counterpart to [`Self::get_by_id`] used by
    /// `recall_via_vector_store` (audit: memory-recall-n+1-update).
    /// Issues a single `SELECT … WHERE id IN (?,?,…)` query and
    /// returns a map keyed by `MemoryId` so the caller can re-order
    /// against its ANN-ranked input vector. Empty input returns
    /// an empty map without touching the pool.
    fn get_by_ids_batch(
        &self,
        ids: &[MemoryId],
        include_deleted: bool,
    ) -> LibreFangResult<HashMap<MemoryId, MemoryFragment>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let deleted_clause = if include_deleted {
            ""
        } else {
            " AND deleted = 0"
        };
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality
             FROM memories WHERE id IN ({placeholders}){deleted_clause}",
        );
        let id_strs: Vec<String> = ids.iter().map(|m| m.0.to_string()).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = id_strs
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), decode_memory_row)
            .map_err(LibreFangError::memory)?;

        let mut out = HashMap::with_capacity(ids.len());
        for row in rows {
            let frag = row.map_err(LibreFangError::memory)?;
            out.insert(frag.id, frag);
        }
        Ok(out)
    }

    /// Get a single memory fragment by ID (including soft-deleted ones for history).
    pub fn get_by_id(
        &self,
        id: MemoryId,
        include_deleted: bool,
    ) -> LibreFangResult<Option<MemoryFragment>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        let deleted_clause = if include_deleted {
            ""
        } else {
            " AND deleted = 0"
        };
        let sql = format!(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality
             FROM memories WHERE id = ?1{deleted_clause}",
        );

        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;

        // Row decoder lives at module scope (`decode_memory_row`) so
        // `get_by_ids_batch` can share it without copy-pasting the
        // ~60-line column mapping (audit:
        // memory-recall-n+1-update).
        match stmt.query_row(rusqlite::params![id.0.to_string()], decode_memory_row) {
            Ok(frag) => Ok(Some(frag)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(LibreFangError::memory(e)),
        }
    }

    /// Soft-delete a memory fragment.
    pub fn forget(&self, id: MemoryId) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "UPDATE memories SET deleted = 1 WHERE id = ?1",
            rusqlite::params![id.0.to_string()],
        )
        .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let now = Utc::now().to_rfc3339();
        if let Some(meta) = new_metadata {
            let meta_str = serde_json::to_string(&meta).map_err(LibreFangError::serialization)?;
            conn.execute(
                "UPDATE memories SET content = ?1, metadata = ?2, accessed_at = ?3 WHERE id = ?4 AND deleted = 0",
                rusqlite::params![new_content, meta_str, now, id.0.to_string()],
            )
            .map_err(LibreFangError::memory)?;
        } else {
            conn.execute(
                "UPDATE memories SET content = ?1, accessed_at = ?2 WHERE id = ?3 AND deleted = 0",
                rusqlite::params![new_content, now, id.0.to_string()],
            )
            .map_err(LibreFangError::memory)?;
        }
        Ok(())
    }

    /// Update the embedding for an existing memory.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id.0.to_string()],
        )
        .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        // SQLite doesn't support IN with parameterized lists easily for large N,
        // so we query one at a time for safety (N ≤ 100 in find_duplicates).
        let mut map = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT embedding FROM memories WHERE id = ?1 AND deleted = 0")
            .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string()],
            )
            .map_err(LibreFangError::memory)?;
        Ok(count as u64)
    }

    /// Soft-delete all memories for a specific agent and scope.
    pub fn forget_by_scope(&self, agent_id: AgentId, scope: &str) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND scope = ?2 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string(), scope],
            )
            .map_err(LibreFangError::memory)?;
        Ok(count as u64)
    }

    /// Soft-delete memories older than a given timestamp for a specific agent and scope.
    pub fn forget_older_than(
        &self,
        agent_id: AgentId,
        scope: &str,
        before: chrono::DateTime<Utc>,
    ) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE agent_id = ?1 AND scope = ?2 AND created_at < ?3 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string(), scope, before.to_rfc3339()],
            )
            .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1 WHERE scope = ?1 AND created_at < ?2 AND deleted = 0",
                rusqlite::params![scope, before.to_rfc3339()],
            )
            .map_err(LibreFangError::memory)?;
        Ok(count as u64)
    }

    /// Count non-deleted memories for a specific agent, optionally filtered by scope.
    pub fn count(&self, agent_id: AgentId, scope: Option<&str>) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
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
        .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT id FROM memories WHERE agent_id = ?1 AND deleted = 0 \
                 ORDER BY confidence ASC, created_at ASC LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(
                rusqlite::params![agent_id.0.to_string(), limit as i64],
                |row| {
                    let id_str: String = row.get(0)?;
                    Ok(id_str)
                },
            )
            .map_err(LibreFangError::memory)?;
        let mut ids = Vec::new();
        for row in rows {
            let id_str = row.map_err(LibreFangError::memory)?;
            let uuid = uuid::Uuid::parse_str(&id_str).map_err(LibreFangError::memory)?;
            ids.push(MemoryId(uuid));
        }
        Ok(ids)
    }

    /// Count memories across ALL agents, optionally filtered by scope.
    pub fn count_all(&self, scope: Option<&str>) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
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
        .map_err(LibreFangError::memory)?;
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

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
        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let cat: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((cat, count as usize))
            })
            .map_err(LibreFangError::memory)?;

        let mut map = HashMap::new();
        for row in rows {
            let (cat, count) = row.map_err(LibreFangError::memory)?;
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

/// Row decoder shared by `MemoryStore::get_by_id` and
/// `MemoryStore::get_by_ids_batch` (audit:
/// memory-recall-n+1-update — second sub-finding). The closure
/// must satisfy `FnMut(&Row) -> rusqlite::Result<MemoryFragment>`
/// so it can be passed to both `query_row` and `query_map` —
/// rusqlite errors propagate to the caller, which is responsible
/// for converting them into `LibreFangError`.
///
/// UUID / JSON parse failures inside the row map to
/// `rusqlite::Error::FromSqlConversionFailure` so they surface in
/// the same channel as primitive-column errors. Most rows in
/// practice parse cleanly; this only matters when a row is
/// hand-mutated outside the kernel write paths (operator running
/// SQL by hand).
fn decode_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryFragment> {
    fn fsql<E: std::error::Error + Send + Sync + 'static>(
        idx: usize,
        ty: rusqlite::types::Type,
        e: E,
    ) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(idx, ty, Box::new(e))
    }
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

    let id = uuid::Uuid::parse_str(&id_str)
        .map(MemoryId)
        .map_err(|e| fsql(0, rusqlite::types::Type::Text, e))?;
    let agent_id = uuid::Uuid::parse_str(&agent_str)
        .map(librefang_types::agent::AgentId)
        .map_err(|e| fsql(1, rusqlite::types::Type::Text, e))?;
    let source: MemorySource = serde_json::from_str(&source_str).unwrap_or(MemorySource::System);
    // Surface corruption rather than disguising it as "no metadata" — the
    // caller (`get_by_id` / `get_by_ids_batch`) receives a `Result`, so a
    // bad row should be loud, not a silent `HashMap::default()` (audit:
    // json-text-silent-parse-fallback).
    let metadata: HashMap<String, serde_json::Value> = match serde_json::from_str(&meta_str) {
        Ok(m) => m,
        Err(e) => {
            error!(
                row_id = %id_str,
                table = "memories",
                column = "metadata",
                error = %e,
                "corrupt JSON in TEXT column"
            );
            return Err(fsql(6, rusqlite::types::Type::Text, e));
        }
    };
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
    Ok(MemoryFragment {
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
    })
}

/// Bump access_count + accessed_at on every recalled fragment in
/// a single transaction (audit: memory-recall-n+1-update — first
/// sub-finding). Pre-fix this was a per-row `conn.execute` with
/// no transaction wrapper, forcing one WAL fsync per row; at 100
/// fragments per tool-augmented turn the latency dominated the
/// recall path.
///
/// Failures on individual rows are logged but don't abort the
/// remaining UPDATEs — the decay/consolidation engine keys TTL
/// decisions off `accessed_at`, so we'd rather persist what we
/// can than lose the whole batch on one bad row. A failure to
/// acquire the connection or open the transaction is also
/// logged + ignored (recall already returned the fragments to
/// the caller; we don't want to surface a write-side error on a
/// successful read).
fn bump_recall_access_counts(conn: &mut rusqlite::Connection, fragments: &[MemoryFragment]) {
    if fragments.is_empty() {
        return;
    }
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "memory recall: transaction() failed for access-count bump");
            return;
        }
    };
    let now = Utc::now().to_rfc3339();
    {
        let mut stmt = match tx.prepare(
            "UPDATE memories SET access_count = access_count + 1, accessed_at = ?1 WHERE id = ?2",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "memory recall: stmt.prepare() failed");
                return;
            }
        };
        for frag in fragments {
            if let Err(e) = stmt.execute(rusqlite::params![now, frag.id.0.to_string()]) {
                warn!(memory_id = %frag.id.0, error = %e, "Failed to update access tracking");
            }
        }
    }
    if let Err(e) = tx.commit() {
        warn!(error = %e, "memory recall: tx.commit() failed for access-count bump");
    }
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
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteVectorStore {
    /// Create a new SQLite vector store wrapping the given connection.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<librefang_types::memory::MemoryFilter>,
    ) -> LibreFangResult<Vec<VectorSearchResult>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

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

        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;
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
            .map_err(LibreFangError::memory)?;

        let mut results = Vec::new();
        let mut skipped_non_comparable: u64 = 0;
        for row_result in rows {
            let (id, content, meta_str, emb_bytes) = row_result.map_err(LibreFangError::memory)?;
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
            // Skip rather than silently substitute `HashMap::default()` for
            // a corrupt `metadata` TEXT blob — that disguises corruption as
            // a row with no metadata, which the operator cannot tell apart
            // from a legitimately empty row (audit:
            // json-text-silent-parse-fallback).
            let metadata: HashMap<String, serde_json::Value> = match serde_json::from_str(&meta_str)
            {
                Ok(m) => m,
                Err(e) => {
                    error!(
                        row_id = %id,
                        table = "memories",
                        column = "metadata",
                        error = %e,
                        "corrupt JSON in TEXT column; skipping vector search candidate"
                    );
                    continue;
                }
            };
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
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "UPDATE memories SET embedding = NULL WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    async fn get_embeddings(&self, ids: &[&str]) -> LibreFangResult<HashMap<String, Vec<f32>>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut map = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT embedding FROM memories WHERE id = ?1 AND deleted = 0")
            .map_err(LibreFangError::memory)?;
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
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        SemanticStore::new(pool)
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

    /// Regression for the audit item `json-text-silent-parse-fallback`.
    ///
    /// Pre-fix, `recall` decoded a row whose `metadata` TEXT column was
    /// corrupt by silently substituting `HashMap::default()` — so the
    /// caller could not distinguish "this memory has no metadata" from
    /// "this memory's metadata is destroyed". After the fix, the loop
    /// drops the corrupt row with a loud `error!` log and the healthy
    /// row beside it still surfaces.
    #[test]
    fn recall_skips_corrupt_metadata_row_instead_of_returning_default() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "healthy memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let corrupt_id = store
            .remember(
                agent_id,
                "corrupt memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE memories SET metadata = ?1 WHERE id = ?2",
                rusqlite::params!["not-json", corrupt_id.0.to_string()],
            )
            .unwrap();
        }

        let results = store.recall("memory", 10, None).unwrap();
        assert_eq!(
            results.len(),
            1,
            "corrupt row must be skipped (not silently coerced to default metadata)"
        );
        assert_eq!(results[0].content, "healthy memory");
    }

    /// Same audit item, on the `decode_memory_row` path — used by
    /// `get_by_id` / `get_by_ids_batch`. Pre-fix, a corrupt `metadata`
    /// blob would silently produce a `MemoryFragment` with empty
    /// metadata; after the fix, the row decoder returns an error so
    /// callers see the failure instead of working with poisoned data.
    #[test]
    fn get_by_id_surfaces_corrupt_metadata_instead_of_defaulting() {
        let store = setup();
        let agent_id = AgentId::new();
        let id = store
            .remember(
                agent_id,
                "fragment",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE memories SET metadata = ?1 WHERE id = ?2",
                rusqlite::params!["not-json", id.0.to_string()],
            )
            .unwrap();
        }

        let res = store.get_by_id(id, false);
        assert!(
            res.is_err(),
            "corrupt metadata must surface as Err from get_by_id, not be silently defaulted; \
             got: {res:?}"
        );
    }
}
