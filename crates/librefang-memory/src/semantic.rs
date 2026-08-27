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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tracing::{debug, error, warn};

/// Upper bound on how many candidate rows the in-process (SQLite) semantic
/// recall scans and cosine re-ranks when a query embedding is supplied.
///
/// The candidate SELECT for the embedding path is ordered by a
/// similarity-neutral key (`created_at`), NOT by recency, so the true nearest
/// neighbor is never excluded just because it was last accessed long ago (see
/// the `fetch_limit` / `ORDER BY` logic in `recall_impl`). This cap only bounds
/// the brute-force scan for very large stores; deployments that exceed it
/// should attach an external `VectorStore` backend.
const MAX_BRUTEFORCE_CANDIDATES: usize = 5000;

/// Upper bound on how many bm25-ranked ids the `memories_fts` lookup hands to
/// the hydrating SELECT (#7808).
///
/// Deliberately far below [`MAX_BRUTEFORCE_CANDIDATES`]: these ids become bound
/// parameters in an `id IN (?,?,…)` clause, and a 5000-parameter statement is
/// both close to SQLite's variable ceiling and slower to prepare than the
/// relevance it buys — bm25 has already ordered the matches, so a caller asking
/// for at most 50 fragments gains nothing from a 5000th-ranked candidate.
/// The remaining SQL predicates (scope, confidence, peer_id, …) run after this
/// cut, which is why it over-fetches at all rather than stopping at `limit`.
const MAX_FTS_CANDIDATES: usize = 500;

/// Upper bound on how many terms a natural-language query contributes to the
/// FTS5 `MATCH` expression, so a pathological paste cannot build a query string
/// SQLite has to parse into thousands of OR-ed phrases.
const MAX_FTS_TERMS: usize = 32;

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
    /// Identity of the embedding model the daemon is currently configured with,
    /// in `provider/model` form (#7912).
    ///
    /// Shared through an `Arc` so a clone of the store — `MemorySubstrate` holds
    /// one by value, and callers clone it freely — observes a value pushed in
    /// after construction. The kernel resolves the *effective* model late in
    /// boot (auto-detection can substitute a provider default for the configured
    /// string), long after the substrate itself exists, and by then it holds only
    /// `Arc<MemorySubstrate>`, so the setter cannot take `&mut self`.
    ///
    /// `None` means "the daemon does not know what it is embedding with" — every
    /// test store, and any deployment with no embedding driver. In that state
    /// nothing is stamped and nothing is treated as stale.
    active_embedding_model: Arc<RwLock<Option<Arc<str>>>>,
}

/// Census key used for rows whose `embedding` predates the `embedding_model`
/// column (v51) and therefore carries no provenance.
///
/// Deliberately not a valid `provider/model` string so it can never collide
/// with a real model identity in the census map.
pub const UNSTAMPED_EMBEDDING_MODEL: &str = "(unstamped, pre-v51)";

impl SemanticStore {
    /// Create a new semantic store wrapping the given connection pool.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            vector_store: None,
            active_embedding_model: Arc::new(RwLock::new(None)),
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
            active_embedding_model: Arc::new(RwLock::new(None)),
        }
    }

    /// Set or replace the vector store backend at runtime.
    pub fn set_vector_store(&mut self, store: Arc<dyn VectorStore>) {
        self.vector_store = Some(store);
    }

    /// Record the embedding model the daemon is currently configured with, in
    /// `provider/model` form (#7912).
    ///
    /// Takes `&self` because the kernel resolves the effective model after the
    /// substrate is already behind an `Arc`. Passing an empty string clears the
    /// identity and restores the unstamped, unguarded behaviour.
    pub fn set_active_embedding_model(&self, model: &str) {
        let value: Option<Arc<str>> = if model.is_empty() {
            None
        } else {
            Some(Arc::from(model))
        };
        match self.active_embedding_model.write() {
            Ok(mut guard) => *guard = value,
            // A poisoned lock here would mean a panic while swapping a string.
            // Recover rather than propagate: the alternative is that one
            // unrelated panic permanently disables embedding provenance.
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }

    /// The embedding model identity stamped onto new vectors, if the daemon
    /// has one. `None` disables both stamping and the staleness guard.
    pub fn active_embedding_model(&self) -> Option<Arc<str>> {
        match self.active_embedding_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Count the stored vectors by the model that produced them (#7912).
    ///
    /// Only live rows (`deleted = 0`) that actually carry an `embedding` are
    /// counted — a row with no vector has nothing to be stale. Rows written
    /// before the v51 stamp are reported under
    /// [`UNSTAMPED_EMBEDDING_MODEL`].
    ///
    /// Returns a `BTreeMap` so the census renders in a stable order in logs and
    /// in any future operator-facing surface, regardless of the order SQLite
    /// hands back the groups.
    pub fn embedding_model_census(&self) -> LibreFangResult<BTreeMap<String, i64>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT embedding_model, COUNT(*) FROM memories \
                 WHERE deleted = 0 AND embedding IS NOT NULL \
                 GROUP BY embedding_model",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(LibreFangError::memory)?;
        let mut census: BTreeMap<String, i64> = BTreeMap::new();
        for row in rows {
            let (model, count) = row.map_err(LibreFangError::memory)?;
            let key = model
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| UNSTAMPED_EMBEDDING_MODEL.to_string());
            *census.entry(key).or_insert(0) += count;
        }
        Ok(census)
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

        // Honor an explicit confidence the caller stashed in metadata (the LLM
        // extractor does this when it has a probability for an extracted fact).
        // Defaulting to 1.0 keeps backwards compatibility for rule-based and
        // manual writes that don't set the key.
        let confidence = metadata
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        // Stamp the vector with the model that produced it (#7912). Only when
        // there is a vector to attribute: a text-only row has no embedding
        // space to belong to, and stamping it would inflate the census with
        // rows a model change cannot affect.
        let embedding_model: Option<Arc<str>> = if embedding_bytes.is_some() {
            self.active_embedding_model()
        } else {
            None
        };

        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, deleted, embedding, image_url, image_embedding, modality, peer_id, embedding_model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, 0, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                id.0.to_string(),
                agent_id.0.to_string(),
                content,
                source_str,
                scope,
                confidence,
                meta_str,
                now,
                embedding_bytes,
                image_url,
                image_embedding_bytes,
                modality_str,
                peer_id,
                embedding_model.as_deref(),
            ],
        )
        .map_err(LibreFangError::memory)?;

        // Release the pooled connection before the (potentially blocking)
        // external vector-store write so a single-connection pool is never
        // held across it.
        drop(conn);

        // Mirror the write into an external vector backend when one is attached
        // (config.toml: vector_backend = "http"). The default SQLite path
        // leaves vector_store = None, so this is a no-op there. Without it the
        // external store is write-blind: every embedding recall against it
        // hydrates zero ids and silently returns empty. Uses the same
        // async->sync bridge as recall_via_vector_store.
        if let (Some(vs), Some(emb)) = (&self.vector_store, embedding) {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(vs.insert(
                    &id.0.to_string(),
                    emb,
                    content,
                    metadata.clone(),
                ))
            })?;
        }

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
        self.recall_impl(query, limit, filter, query_embedding, true)
    }

    /// Read-only listing recall: same query/filter semantics as [`Self::recall`]
    /// but does **not** bump `access_count` / `accessed_at`.
    ///
    /// Use this for the dashboard list/get and any polled listing path. A
    /// genuine semantic recall (a real query feeding a prompt) should track
    /// access so the decay/consolidation engine sees the memory as "used"; but
    /// a listing read must not — a dashboard auto-refreshing the memory list
    /// would otherwise reset `accessed_at = now` on every poll and effectively
    /// freeze those memories from ever decaying (#5839). Always uses the SQLite
    /// path (no vector store), which is what listing wants.
    pub fn recall_readonly(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        self.recall_impl(query, limit, filter, None, false)
    }

    fn recall_impl(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
        track_access: bool,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        // ── Delegate to external vector store when available ──────────
        if let (Some(vs), Some(qe)) = (&self.vector_store, query_embedding) {
            return self.recall_via_vector_store(vs, qe, limit, filter.clone(), track_access);
        }

        // mut: needed for the `transaction()` call inside
        // `bump_recall_access_counts` after the read is done. The
        // read-side `stmt` borrow is explicitly dropped below
        // before that borrow occurs.
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;

        // ── Full-text pre-selection when there is no query embedding ──
        //
        // Without this the no-embedding path was `content LIKE '%{query}%'`
        // (#7808): a substring scan that only matches when the caller's entire
        // phrasing appears verbatim inside a memory, which a natural-language
        // question essentially never does. `memories_fts` (v49) turns the same
        // path into a real inverted-index lookup ranked by bm25.
        //
        // Strictly additive: a query that yields no FTS hits falls through to
        // the LIKE predicate below, so substring matches that FTS's word
        // boundaries would miss ("fang" inside "librefang") still work, and a
        // deployment whose index has not been built yet degrades to exactly
        // the old behaviour instead of going silent.
        let fts_ranked: Option<Vec<String>> = if query_embedding.is_none() && !query.is_empty() {
            fts_candidate_ids(
                &conn,
                query,
                filter.as_ref().and_then(|f| f.agent_id),
                MAX_FTS_CANDIDATES,
            )
        } else {
            None
        };

        // Build SQL: fetch candidates (broader than limit for re-ranking).
        // Both re-ranked paths — cosine and bm25 — order in Rust after the
        // rows are materialized, so both must over-fetch for the same reason:
        // a `LIMIT limit` here would let the SQL ordering, not relevance,
        // decide which rows the ranker ever sees.
        let fetch_limit = if query_embedding.is_some() {
            // Cosine re-ranking (below) decides final relevance, so the
            // candidate set must NOT be pre-filtered by recency — an old,
            // rarely-accessed memory can still be the nearest neighbor. Scan a
            // large, similarity-neutral window bounded by
            // MAX_BRUTEFORCE_CANDIDATES rather than the 100 most-recently
            // accessed rows, which silently dropped relevant older memories.
            (limit * 10).max(MAX_BRUTEFORCE_CANDIDATES)
        } else if fts_ranked.is_some() {
            // The `id IN (…)` list already caps this path at
            // MAX_FTS_CANDIDATES rows; the LIMIT is belt-and-braces.
            MAX_FTS_CANDIDATES
        } else {
            limit
        };

        let mut sql = String::from(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality, embedding_model
             FROM memories WHERE deleted = 0",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        // Text search filter (only when no embeddings — vector search handles
        // relevance). The FTS index answers first when it matched anything;
        // LIKE remains the fallback for the queries it cannot serve.
        if let Some(ref ids) = fts_ranked {
            let placeholders = ids
                .iter()
                .enumerate()
                .map(|(offset, _)| format!("?{}", param_idx + offset))
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(" AND id IN ({placeholders})"));
            for id in ids {
                params.push(Box::new(id.clone()));
                param_idx += 1;
            }
        } else if query_embedding.is_none() && !query.is_empty() {
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
            // Metadata filtering via json_extract. Keys must be
            // alphanumeric/underscore only (interpolated into the JSON path,
            // so a non-identifier key would be an injection vector); a
            // rejected key is logged, never silently dropped. Every scalar
            // value type yields a predicate so the filter is actually applied
            // rather than silently widening the result set.
            for (key, value) in &f.metadata {
                if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    tracing::warn!(
                        metadata_key = %key,
                        "recall: ignoring metadata filter with non-identifier key (allowed: [A-Za-z0-9_]); filter not applied"
                    );
                    continue;
                }
                match value {
                    serde_json::Value::String(s) => {
                        sql.push_str(&format!(
                            " AND json_extract(metadata, '$.{}') = ?{param_idx}",
                            key
                        ));
                        params.push(Box::new(s.to_string()));
                        param_idx += 1;
                    }
                    serde_json::Value::Bool(b) => {
                        // SQLite's json_extract yields integer 1/0 for JSON
                        // booleans; bind the matching integer so the equality
                        // holds (a text "true"/"false" would never match).
                        sql.push_str(&format!(
                            " AND json_extract(metadata, '$.{}') = ?{param_idx}",
                            key
                        ));
                        params.push(Box::new(if *b { 1_i64 } else { 0_i64 }));
                        param_idx += 1;
                    }
                    serde_json::Value::Number(n) => {
                        // Bind numbers with their native SQLite type so the
                        // comparison against json_extract's numeric result
                        // holds under SQLite type affinity rules.
                        sql.push_str(&format!(
                            " AND json_extract(metadata, '$.{}') = ?{param_idx}",
                            key
                        ));
                        if let Some(i) = n.as_i64() {
                            params.push(Box::new(i));
                        } else if let Some(f) = n.as_f64() {
                            params.push(Box::new(f));
                        } else {
                            // u64 beyond i64 range — fall back to text form.
                            params.push(Box::new(n.to_string()));
                        }
                        param_idx += 1;
                    }
                    // Null / array / object filters have no equality predicate;
                    // warn rather than silently drop them.
                    other => {
                        tracing::warn!(
                            metadata_key = %key,
                            value_kind = ?other,
                            "recall: ignoring metadata filter with unsupported value type (only string/bool/number); filter not applied"
                        );
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

        if query_embedding.is_some() || fts_ranked.is_some() {
            // Relevance-neutral candidate ordering: recency (accessed_at) must
            // not decide which rows reach cosine or bm25 re-ranking, or an
            // old-but-relevant memory outside the recency window is silently
            // dropped. `created_at DESC` only breaks ties when the store is
            // larger than the cap.
            sql.push_str(" ORDER BY created_at DESC");
        } else {
            sql.push_str(" ORDER BY confidence DESC, accessed_at DESC, access_count DESC");
        }
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
                let embedding_model: Option<String> = row.get(14)?;
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
                    embedding_model,
                ))
            })
            .map_err(LibreFangError::memory)?;

        let mut fragments = Vec::new();
        let mut candidate_count = 0usize;
        // Ids whose stored vector was produced by a model other than the one
        // the daemon is configured with (#7912). Keyed by id rather than held
        // as a parallel `Vec` because the bm25 re-ordering below permutes
        // `fragments` and the corrupt-metadata `continue` skips rows, so
        // positional alignment does not survive.
        let mut stale_vector_ids: HashSet<String> = HashSet::new();
        let active_embedding_model = self.active_embedding_model();
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
                row_embedding_model,
            ) = row_result.map_err(LibreFangError::memory)?;
            candidate_count += 1;

            // #7912: a vector produced by a different embedding model does not
            // live in the active model's space, so cosine against it is a
            // meaningless number rather than a weak match. Record the row here
            // and skip scoring it below. A `NULL` stamp is a row written before
            // v51 and is treated as comparable — see `migrate_v51`.
            if let (Some(active), Some(stored)) = (
                active_embedding_model.as_deref(),
                row_embedding_model.as_deref(),
            ) {
                if !stored.is_empty() && stored != active {
                    stale_vector_ids.insert(id_str.clone());
                }
            }

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
                // Filled in by the cosine pass below when there is a query
                // embedding; stays `None` on the text-match path, where no
                // similarity was measured (#7808).
                similarity: None,
            });
        }

        // Restore the bm25 ordering the FTS lookup produced. The SELECT above
        // reads `id IN (...)`, which SQLite is free to return in any order, so
        // relevance has to be re-imposed here — otherwise the index would only
        // decide *which* rows match and `created_at DESC` would decide which
        // of them survive the truncation, which is the recency bias the
        // over-fetch exists to avoid. Rows the FTS lookup did not rank cannot
        // occur (they came from that id list) but sort last if they ever do.
        if let Some(ref ids) = fts_ranked {
            let rank: HashMap<&str, usize> = ids
                .iter()
                .enumerate()
                .map(|(position, id)| (id.as_str(), position))
                .collect();
            fragments.sort_by_key(|frag| {
                rank.get(frag.id.0.to_string().as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            fragments.truncate(limit);
            debug!(
                "FTS recall: {} results from {candidate_count} candidates",
                fragments.len(),
            );
        }

        // If we have a query embedding, re-rank by cosine similarity.
        // Non-comparable vectors (dim mismatch, zero magnitude) score `None`
        // and sort to the bottom on a NEG_INFINITY sentinel instead of being
        // treated as 0.0, which would have ranked them above genuinely
        // orthogonal hits. We deliberately do NOT use -1.0: that is a valid
        // cosine result for anti-similar vectors and would tie with the
        // "non-comparable" bucket.
        //
        // The score is now stamped onto the fragment before the sort rather
        // than computed inside the comparator and discarded (#7808). Two
        // reasons: callers need the number (a similarity floor is impossible
        // without it, and the search tool reports it so a model can judge how
        // much a fragment is worth trusting), and the comparator was
        // recomputing cosine O(n log n) times over vectors of ~1500 floats
        // where O(n) suffices.
        if let Some(qe) = query_embedding {
            for frag in &mut fragments {
                if stale_vector_ids.contains(&frag.id.0.to_string()) {
                    frag.similarity = None;
                    continue;
                }
                frag.similarity = frag
                    .embedding
                    .as_deref()
                    .and_then(|e| cosine_similarity(qe, e));
            }
            // #7912: one line per recall, not per row, so an operator who
            // changed `[memory] embedding_model` without re-embedding sees the
            // consequence in the logs rather than only in worse answers.
            if !stale_vector_ids.is_empty() {
                warn!(
                    stale = stale_vector_ids.len(),
                    candidates = candidate_count,
                    active_model = %active_embedding_model.as_deref().unwrap_or("unknown"),
                    "Vector recall: candidates were embedded by a different model and were not scored; re-embed them or restore the previous embedding_model"
                );
            }
            fragments.sort_by(|a, b| {
                let sim_a = a.similarity.unwrap_or(f32::NEG_INFINITY);
                let sim_b = b.similarity.unwrap_or(f32::NEG_INFINITY);
                sim_b
                    .partial_cmp(&sim_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            // Similarity floor (#7808): "nothing rather than noise".
            // Applied after ranking and before truncation, so the floor
            // decides membership rather than merely reordering the same top-k.
            // A fragment with no measurable similarity is dropped too — the
            // caller asked for a measured minimum, and an unmeasured row
            // cannot clear it.
            if let Some(floor) = filter.as_ref().and_then(|f| f.min_similarity) {
                let before = fragments.len();
                fragments.retain(|frag| frag.similarity.is_some_and(|s| s >= floor));
                if fragments.len() < before {
                    debug!(
                        "Vector recall: similarity floor {floor} dropped {} of {before} ranked candidates",
                        before - fragments.len(),
                    );
                }
            }
            fragments.truncate(limit);
            debug!(
                "Vector recall: {} results from {candidate_count} candidates",
                fragments.len(),
            );
            if candidate_count >= fetch_limit {
                debug!(
                    "Vector recall candidate scan hit the cap ({fetch_limit}); the true nearest neighbor may lie beyond it — attach an external VectorStore backend for large stores"
                );
            }
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
        if track_access {
            bump_recall_access_counts(&mut conn, &fragments);
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
        track_access: bool,
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
        // WHERE id IN (?,?,...). Parse all ANN-returned ids first,
        // then issue one batched SELECT. The batch preserves the ANN
        // ranking order by re-ordering against the input vec after fetch.
        //
        // An id that is not a UUID is dropped, not propagated as an error.
        // The id space belongs to an untrusted external backend, and a single
        // unparseable row used to abort the whole recall — one malformed
        // entry in a result set of fifty denied the agent every memory it
        // would otherwise have recalled, which is a denial of service handed
        // to whoever controls the backend's id column.
        // Dropping it also makes this loop consistent with the hydrate loop
        // below, where `by_id.remove` already skips an id SQLite does not
        // know about: both are "the backend named something we cannot use",
        // and they now degrade the same way.
        let mut ordered_ids: Vec<MemoryId> = Vec::with_capacity(results.len());
        for r in &results {
            match uuid::Uuid::parse_str(&r.id) {
                Ok(uuid) => ordered_ids.push(MemoryId(uuid)),
                Err(e) => {
                    // The id is backend-controlled, so cap it before it
                    // reaches the log line.
                    let shown: String = r.id.chars().take(64).collect();
                    warn!(
                        backend = vs.backend_name(),
                        vector_id = %shown,
                        error = %e,
                        "vector store returned a non-UUID id; dropping that result and continuing with the rest of the recall"
                    );
                }
            }
        }
        let mut by_id = self.get_by_ids_batch(
            &ordered_ids,
            false,
            filter.as_ref().and_then(|f| f.peer_id.as_deref()),
        )?;
        let mut fragments: Vec<MemoryFragment> = Vec::with_capacity(ordered_ids.len());
        for mem_id in &ordered_ids {
            if let Some(frag) = by_id.remove(mem_id) {
                fragments.push(frag);
            }
        }

        // Defense-in-depth (audit: vector-store-hydrate-tenant-filter). The
        // external VectorStore backend is untrusted — a misbehaving or
        // compromised backend can return ids belonging to another agent /
        // scope / source than the caller's filter requested, and
        // `get_by_ids_batch` only enforces `deleted = 0` (plus peer_id, passed
        // above) at the SQL layer. Re-apply the caller's MemoryFilter to the
        // hydrated fragments so tenant isolation never depends on the backend
        // honouring the filter. This mirrors the WHERE clauses `recall_impl`
        // pushes into SQLite for the fields carried by MemoryFragment.
        if let Some(ref f) = filter {
            fragments.retain(|frag| fragment_matches_filter(frag, f));
        }

        // Carry the backend's own similarity onto the fragments, then apply the
        // caller's floor (#7808). The external path never reaches the cosine
        // block in `recall_impl`, so without this a `min_similarity` request
        // would be silently ignored the moment a VectorStore is attached — the
        // floor would hold on the SQLite path and evaporate on the one built
        // for large stores.
        let scores: HashMap<&str, f32> = results.iter().map(|r| (r.id.as_str(), r.score)).collect();
        for frag in &mut fragments {
            frag.similarity = scores.get(frag.id.0.to_string().as_str()).copied();
        }
        if let Some(floor) = filter.as_ref().and_then(|f| f.min_similarity) {
            fragments.retain(|frag| frag.similarity.is_some_and(|s| s >= floor));
        }

        // Update access counts — see note on the SQLite-path
        // update above for why silent drops would corrupt decay
        // logic. Same tx-wrapped helper. The vector-store branch
        // has no other live conn handle at this point, so we
        // acquire one for the write.
        if track_access {
            if let Ok(mut write_conn) = self.pool.get() {
                bump_recall_access_counts(&mut write_conn, &fragments);
            } else {
                warn!("memory recall (vector store): pool.get() for access-count bump failed");
            }
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
        peer_id: Option<&str>,
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
        // Enforce peer isolation in SQL when the caller filters by peer, so an
        // untrusted vector backend returning another peer's id never hydrates
        // (MemoryFragment does not carry peer_id, so this cannot be re-checked
        // after hydration — it must be a query-time predicate). Mirrors the
        // `peer_id = ?` clause in `recall_impl`.
        let peer_clause = if peer_id.is_some() {
            " AND peer_id = ?"
        } else {
            ""
        };
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, agent_id, content, source, scope, confidence, metadata, created_at, accessed_at, access_count, embedding, image_url, image_embedding, modality
             FROM memories WHERE id IN ({placeholders}){deleted_clause}{peer_clause}",
        );
        let id_strs: Vec<String> = ids.iter().map(|m| m.0.to_string()).collect();
        let mut param_refs: Vec<&dyn rusqlite::types::ToSql> = id_strs
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        if let Some(ref p) = peer_id {
            param_refs.push(p as &dyn rusqlite::types::ToSql);
        }

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
    ///
    /// Stamps `deleted_at` (unix seconds) alongside `deleted = 1` so the
    /// `prune_soft_deleted_memories` retention sweep can hard-delete the row
    /// later. Without the timestamp the prune filter (`deleted_at IS NOT NULL`)
    /// would skip it forever, leaking the embedding BLOB indefinitely.
    pub fn forget(&self, id: MemoryId) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        conn.execute(
            "UPDATE memories SET deleted = 1, deleted_at = ?1 \
             WHERE id = ?2 AND deleted = 0",
            rusqlite::params![Utc::now().timestamp(), id.0.to_string()],
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
    ///
    /// Re-stamps `embedding_model` with the active model (#7912). This is the
    /// re-embedding path: leaving the previous stamp in place would leave a row
    /// that has just been rebuilt in the current space permanently marked as
    /// belonging to the old one, and therefore permanently excluded from
    /// scoring.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let bytes = embedding_to_bytes(embedding);
        let model = self.active_embedding_model();
        conn.execute(
            "UPDATE memories SET embedding = ?1, embedding_model = ?2 WHERE id = ?3",
            rusqlite::params![bytes, model.as_deref(), id.0.to_string()],
        )
        .map_err(LibreFangError::memory)?;

        // Mirror the re-embedding into an external vector backend when one is
        // attached (config.toml: vector_backend = "http"). `VectorStore::insert`
        // is an upsert, so re-inserting with the same id and the NEW embedding
        // replaces the stale vector. Without this the external store keeps the
        // OLD embedding after a content re-embed and `recall_via_vector_store`
        // ranks against it — the same write-blindness the `remember` path fixes,
        // on the update side. The default SQLite path leaves vector_store = None,
        // so this is a no-op there.
        if let Some(vs) = &self.vector_store {
            let row = conn.query_row(
                "SELECT content, metadata FROM memories WHERE id = ?1 AND deleted = 0",
                rusqlite::params![id.0.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            );
            // Release the pooled connection before the (potentially blocking)
            // external vector-store write so a single-connection pool is never
            // held across it (mirrors the `remember` path).
            drop(conn);
            let (content, meta_str) = match row {
                Ok(v) => v,
                // Row gone (deleted between UPDATE and this read) — nothing to
                // mirror; the embedding write above is harmless.
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
                Err(e) => return Err(LibreFangError::memory(e)),
            };
            // Match the recall path: refuse to disguise a corrupt metadata blob
            // as empty. Skip the mirror with a loud log rather than upserting
            // wrong metadata; the SQLite embedding update still stands.
            match serde_json::from_str::<HashMap<String, serde_json::Value>>(&meta_str) {
                Ok(metadata) => {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(vs.insert(
                            &id.0.to_string(),
                            embedding,
                            &content,
                            metadata,
                        ))
                    })?;
                }
                Err(e) => {
                    error!(
                        memory_id = %id.0,
                        error = %e,
                        "update_embedding: metadata column unparseable, skipping external \
                         vector-store mirror (embedding recall may stay stale for this id)"
                    );
                }
            }
        }
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

        // #7912: a vector produced by a different embedding model is withheld
        // rather than handed out. The only caller is `find_duplicates`, which
        // compares two *stored* vectors and merges the rows when they score
        // above the threshold — a merge is destructive, and a cosine across two
        // embedding spaces is a number rather than a similarity. Withholding
        // the vector drops that pair to the Jaccard word-overlap fallback,
        // which is weaker but cannot merge two unrelated memories on the
        // strength of a meaningless score. A `NULL` stamp is a pre-v51 row and
        // stays available; see `migrate_v51`.
        let active = self.active_embedding_model();

        // SQLite doesn't support IN with parameterized lists easily for large N,
        // so we query one at a time for safety (N ≤ 100 in find_duplicates).
        let mut map = HashMap::new();
        let mut withheld = 0usize;
        let mut stmt = conn
            .prepare(
                "SELECT embedding, embedding_model FROM memories WHERE id = ?1 AND deleted = 0",
            )
            .map_err(LibreFangError::memory)?;
        for id in ids {
            if let Ok((Some(b), stored_model)) = stmt.query_row(rusqlite::params![*id], |row| {
                let b: Option<Vec<u8>> = row.get(0)?;
                let m: Option<String> = row.get(1)?;
                Ok((b, m))
            }) {
                if b.is_empty() {
                    continue;
                }
                let comparable = match (active.as_deref(), stored_model.as_deref()) {
                    (Some(a), Some(stored)) if !stored.is_empty() => stored == a,
                    _ => true,
                };
                if comparable {
                    map.insert(id.to_string(), embedding_from_bytes(&b));
                } else {
                    withheld += 1;
                }
            }
        }
        if withheld > 0 {
            warn!(
                withheld,
                requested = ids.len(),
                active_model = %active.as_deref().unwrap_or("unknown"),
                "Withheld stored vectors embedded by a different model; dedup falls back to text similarity for them"
            );
        }
        Ok(map)
    }

    /// Soft-delete all memories for a specific agent.
    ///
    /// See [`Self::forget`] for why `deleted_at` is stamped.
    pub fn forget_by_agent(&self, agent_id: AgentId) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1, deleted_at = ?1 \
                 WHERE agent_id = ?2 AND deleted = 0",
                rusqlite::params![Utc::now().timestamp(), agent_id.0.to_string()],
            )
            .map_err(LibreFangError::memory)?;
        Ok(count as u64)
    }

    /// Soft-delete all memories for a specific agent and scope.
    pub fn forget_by_scope(&self, agent_id: AgentId, scope: &str) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let count = conn
            .execute(
                "UPDATE memories SET deleted = 1, deleted_at = ?1 \
                 WHERE agent_id = ?2 AND scope = ?3 AND deleted = 0",
                rusqlite::params![Utc::now().timestamp(), agent_id.0.to_string(), scope],
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
                "UPDATE memories SET deleted = 1, deleted_at = ?1 \
                 WHERE agent_id = ?2 AND scope = ?3 AND created_at < ?4 AND deleted = 0",
                rusqlite::params![
                    Utc::now().timestamp(),
                    agent_id.0.to_string(),
                    scope,
                    before.to_rfc3339()
                ],
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
                "UPDATE memories SET deleted = 1, deleted_at = ?1 \
                 WHERE scope = ?2 AND created_at < ?3 AND deleted = 0",
                rusqlite::params![Utc::now().timestamp(), scope, before.to_rfc3339()],
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

    /// Return the IDs the per-agent memory cap should evict first, worst first.
    ///
    /// Ordering is **class before confidence** (#7756 §1.2). Raw dialogue is evicted
    /// ahead of extracted facts even when the fact scores lower, and only within a
    /// class does the old `confidence ASC, created_at ASC` ordering apply.
    ///
    /// The rationale is that the two classes have different exit paths. An extracted
    /// fact is the distilled, categorised artefact of many turns and is produced at a
    /// few rows a day; raw dialogue is written unconditionally, one row per turn, is
    /// never distilled into anything, and has no TTL — so the cap is the only exit it
    /// has. Ordering by confidence alone made the cap evict whichever class happened
    /// to score lower, which is not a decision anybody made.
    ///
    /// The raw-dialogue predicate is the exact write signature of
    /// `remember_interaction_best_effort` (`librefang-runtime`, `agent_loop::prompt`):
    /// `MemorySource::Conversation`, scope `episodic`, empty metadata. Extracted facts
    /// always carry a `category` (see `ProactiveMemoryStore::add_with_decision`) and
    /// always land in a `*_memory` scope, so they can never match; imported and
    /// system-sourced rows differ in `source`. A row that matches all three is one this
    /// writer produced.
    pub fn eviction_candidates(
        &self,
        agent_id: AgentId,
        limit: usize,
    ) -> LibreFangResult<Vec<MemoryId>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        // Derived rather than hard-coded so the predicate follows the enum's serde
        // representation if it is ever renamed.
        let conversation_source = serde_json::to_string(&MemorySource::Conversation)
            .map_err(LibreFangError::serialization)?;
        let mut stmt = conn
            .prepare(
                "SELECT id FROM memories WHERE agent_id = ?1 AND deleted = 0 \
                 ORDER BY \
                   CASE WHEN scope = 'episodic' AND source = ?3 \
                             AND COALESCE(json_extract(metadata, '$.category'), '') = '' \
                        THEN 0 ELSE 1 END, \
                   confidence ASC, created_at ASC \
                 LIMIT ?2",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(
                rusqlite::params![agent_id.0.to_string(), limit as i64, conversation_source],
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

    /// List one stable page of non-deleted memories and return the full filtered count.
    ///
    /// Count and page reads share one SQLite snapshot so callers never pair a
    /// page from one database state with a total from another. Filters are
    /// applied in SQL before `LIMIT` and `OFFSET`; this is the authoritative
    /// dashboard listing path and intentionally has no hidden candidate cap.
    pub fn list_page(
        &self,
        agent_id: Option<AgentId>,
        category: Option<&str>,
        scope: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> LibreFangResult<(Vec<MemoryFragment>, usize)> {
        let mut conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = conn.transaction().map_err(LibreFangError::memory)?;
        let agent_id = agent_id.map(|id| id.to_string());
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);

        let total: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM memories
                 WHERE deleted = 0
                   AND CASE
                         WHEN json_valid(metadata) THEN json_type(metadata) = 'object'
                         ELSE 0
                       END
                   AND (?1 IS NULL OR agent_id = ?1)
                   AND (?2 IS NULL OR json_extract(
                         CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                         '$.category'
                       ) = ?2)
                   AND (?3 IS NULL OR scope = ?3)",
                rusqlite::params![agent_id.as_deref(), category, scope],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;

        let fragments = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, agent_id, content, source, scope, confidence, metadata,
                            created_at, accessed_at, access_count, embedding, image_url,
                            image_embedding, modality
                     FROM memories
                     WHERE deleted = 0
                       AND CASE
                             WHEN json_valid(metadata) THEN json_type(metadata) = 'object'
                             ELSE 0
                           END
                       AND (?1 IS NULL OR agent_id = ?1)
                       AND (?2 IS NULL OR json_extract(
                             CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                             '$.category'
                           ) = ?2)
                       AND (?3 IS NULL OR scope = ?3)
                     ORDER BY created_at DESC, id DESC
                     LIMIT ?4 OFFSET ?5",
                )
                .map_err(LibreFangError::memory)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![agent_id.as_deref(), category, scope, limit, offset],
                    decode_memory_row,
                )
                .map_err(LibreFangError::memory)?;
            let mut fragments = Vec::new();
            for row in rows {
                fragments.push(row.map_err(LibreFangError::memory)?);
            }
            fragments
        };
        tx.commit().map_err(LibreFangError::memory)?;

        let total = usize::try_from(total).map_err(LibreFangError::memory)?;
        Ok((fragments, total))
    }

    /// Count non-deleted memories grouped by agent ID.
    pub fn count_by_agent(&self) -> LibreFangResult<HashMap<String, usize>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, COUNT(*) FROM memories \
                 WHERE deleted = 0 GROUP BY agent_id",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| {
                let agent_id: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((agent_id, count as usize))
            })
            .map_err(LibreFangError::memory)?;

        let mut counts = HashMap::new();
        for row in rows {
            let (agent_id, count) = row.map_err(LibreFangError::memory)?;
            counts.insert(agent_id, count);
        }
        Ok(counts)
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

/// Re-apply a [`MemoryFilter`]'s tenant / scope predicates to an already
/// hydrated fragment.
///
/// Used by `recall_via_vector_store` as defense in depth: the external vector
/// backend is untrusted, so tenant isolation must not rely on it honouring the
/// filter it was handed. Mirrors the SQL WHERE clauses `recall_impl` pushes
/// into SQLite for the fields carried by [`MemoryFragment`] (`agent_id`,
/// `scope`, `min_confidence`, `source`, `after`, `before`). `peer_id` is
/// enforced in the hydration query (`get_by_ids_batch`) rather than here,
/// because `MemoryFragment` does not carry `peer_id`.
fn fragment_matches_filter(frag: &MemoryFragment, f: &MemoryFilter) -> bool {
    if let Some(agent_id) = f.agent_id {
        if frag.agent_id != agent_id {
            return false;
        }
    }
    if let Some(ref scope) = f.scope {
        if &frag.scope != scope {
            return false;
        }
    }
    if let Some(min_conf) = f.min_confidence {
        if frag.confidence < min_conf {
            return false;
        }
    }
    if let Some(ref source) = f.source {
        if &frag.source != source {
            return false;
        }
    }
    // SQLite path uses strict `>`/`<` bounds; keep the same semantics.
    if let Some(ref after) = f.after {
        if frag.created_at <= *after {
            return false;
        }
    }
    if let Some(ref before) = f.before {
        if frag.created_at >= *before {
            return false;
        }
    }
    // Metadata equality, mirroring the `json_extract(metadata, '$.key') = ?`
    // predicates `recall_impl` pushes into SQLite. Without this the
    // vector-store hydrate path re-checked every filter field except this
    // one, so a caller that scoped a recall by metadata got that scope
    // enforced on the SQLite path and silently dropped on the external-backend
    // path — the divergence the defense-in-depth re-check exists to prevent.
    for (key, want) in &f.metadata {
        // Same key and value-kind admissibility as the SQL builder: a
        // non-identifier key or a null/array/object value yields no predicate
        // there, so it must yield no predicate here either, or the two paths
        // would disagree in the opposite direction. `recall_impl` already
        // warned about both; warning again would double-log the same filter.
        if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        match want {
            serde_json::Value::String(_) | serde_json::Value::Bool(_) => {
                if frag.metadata.get(key) != Some(want) {
                    return false;
                }
            }
            serde_json::Value::Number(n) => {
                // SQLite compares json_extract's numeric result under type
                // affinity, where 1 and 1.0 are equal; serde_json's `Value`
                // equality treats them as distinct. Compare as f64 so the two
                // paths agree.
                let matches = frag
                    .metadata
                    .get(key)
                    .and_then(serde_json::Value::as_f64)
                    .zip(n.as_f64())
                    .is_some_and(|(have, want)| have == want);
                if !matches {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Build an FTS5 `MATCH` expression from a natural-language query (#7808).
///
/// The query is split on every non-alphanumeric character and each surviving
/// token is emitted as a quoted phrase, OR-ed together.
/// Splitting that way is also what makes the result injection-proof: a token
/// can only contain alphanumerics, so no token can carry a quote, a `*`, a
/// `NEAR`, or any other character FTS5's query grammar treats as syntax.
/// Passing a raw user question straight to `MATCH` would instead be a parse
/// error on the first apostrophe or question mark, and would surface as a
/// failed recall rather than a bad one.
///
/// `OR` rather than `AND` because bm25 already rewards documents matching more
/// of the terms: `AND` would return nothing for a question whose every word
/// must appear in one memory, which is the LIKE failure mode in a new costume.
///
/// Returns `None` when the query contributes no usable term (punctuation only),
/// which the caller reads as "no FTS pre-selection" rather than "no results".
fn fts_match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .take(MAX_FTS_TERMS)
        .map(|token| format!("\"{token}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Look up bm25-ranked memory ids for `query` in the `memories_fts` index.
///
/// `None` means "no usable FTS pre-selection" — no parseable term, no matching
/// row, or an index that could not be queried — and the caller must fall back
/// to the `content LIKE` predicate rather than treating it as an empty result
/// set. That distinction is the whole safety property of this function: FTS can
/// only ever add matches to the no-embedding path, never remove them.
///
/// Index errors are logged and swallowed for the same reason. The index is a
/// derivative of `memories` maintained by triggers, so a missing or corrupt one
/// is an operational fault to repair (`rebuild_memories_fts`), not a reason to
/// fail a recall that the base table can still answer.
fn fts_candidate_ids(
    conn: &rusqlite::Connection,
    query: &str,
    agent_id: Option<AgentId>,
    limit: usize,
) -> Option<Vec<String>> {
    let expression = fts_match_expression(query)?;

    // `agent_id` is UNINDEXED in the FTS table, so this is a filter over the
    // matched rows rather than an index probe — still far cheaper than
    // hydrating another agent's matches only to discard them in SQL.
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match agent_id {
        Some(agent) => (
            "SELECT memory_id FROM memories_fts \
             WHERE memories_fts MATCH ?1 AND agent_id = ?2 \
             ORDER BY rank LIMIT ?3",
            vec![
                Box::new(expression),
                Box::new(agent.0.to_string()),
                Box::new(limit as i64),
            ],
        ),
        None => (
            "SELECT memory_id FROM memories_fts \
             WHERE memories_fts MATCH ?1 \
             ORDER BY rank LIMIT ?2",
            vec![Box::new(expression), Box::new(limit as i64)],
        ),
    };

    let read = || -> Result<Vec<String>, rusqlite::Error> {
        let mut stmt = conn.prepare(sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;
        rows.collect()
    };

    match read() {
        Ok(ids) if ids.is_empty() => None,
        Ok(ids) => Some(ids),
        Err(e) => {
            debug!(
                error = %e,
                "memories_fts lookup failed; falling back to LIKE matching"
            );
            None
        }
    }
}

/// Deserialize embedding from bytes.
fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
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
        // Row decoding carries no query, so no similarity was measured (#7808).
        similarity: None,
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

    /// Overwrite a row's confidence so eviction ordering can be exercised without
    /// waiting on decay.
    fn set_confidence(store: &SemanticStore, id: MemoryId, confidence: f32) {
        store
            .pool
            .get()
            .unwrap()
            .execute(
                "UPDATE memories SET confidence = ?1 WHERE id = ?2",
                rusqlite::params![confidence, id.0.to_string()],
            )
            .unwrap();
    }

    /// The exact write signature of `remember_interaction_best_effort`.
    fn remember_raw_dialogue(store: &SemanticStore, agent_id: AgentId, body: &str) -> MemoryId {
        store
            .remember(
                agent_id,
                &format!("[Past exchange]\nThem: {body}\nYou: sure"),
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap()
    }

    /// The write signature of `ProactiveMemoryStore::add_with_decision`.
    fn remember_extracted_fact(store: &SemanticStore, agent_id: AgentId, body: &str) -> MemoryId {
        let mut metadata = HashMap::new();
        metadata.insert("category".to_string(), serde_json::json!("preference"));
        store
            .remember(
                agent_id,
                body,
                MemorySource::Conversation,
                "user_memory",
                metadata,
            )
            .unwrap()
    }

    /// Read a row's `embedding_model` stamp straight out of SQLite.
    fn stamped_model(store: &SemanticStore, id: MemoryId) -> Option<String> {
        store
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT embedding_model FROM memories WHERE id = ?1",
                rusqlite::params![id.0.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
    }

    /// Write a row carrying a vector, under whatever model the store is
    /// currently configured with.
    fn remember_with_vector(
        store: &SemanticStore,
        agent_id: AgentId,
        body: &str,
        embedding: &[f32],
    ) -> MemoryId {
        store
            .remember_with_embedding(
                agent_id,
                body,
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(embedding),
                None,
                None,
                MemoryModality::default(),
            )
            .unwrap()
    }

    /// #7912: `memories.embedding` was a bare BLOB, so nothing recorded which
    /// model produced a vector.
    #[test]
    fn a_stored_vector_records_the_model_that_produced_it() {
        let store = setup();
        store.set_active_embedding_model("ollama/bge-m3");
        let id = remember_with_vector(&store, AgentId::new(), "hello", &[1.0, 0.0, 0.0]);
        assert_eq!(stamped_model(&store, id).as_deref(), Some("ollama/bge-m3"));
    }

    /// A row with no vector has no embedding space to belong to, so stamping it
    /// would inflate the census with rows a model change cannot affect.
    #[test]
    fn a_text_only_row_is_not_stamped() {
        let store = setup();
        store.set_active_embedding_model("ollama/bge-m3");
        let id = remember_raw_dialogue(&store, AgentId::new(), "no vector here");
        assert_eq!(stamped_model(&store, id), None);
    }

    /// With no configured model — every test store, and any deployment without
    /// an embedding driver — nothing is stamped and nothing is guarded.
    #[test]
    fn no_configured_model_means_no_stamp() {
        let store = setup();
        let id = remember_with_vector(&store, AgentId::new(), "hello", &[1.0, 0.0, 0.0]);
        assert_eq!(stamped_model(&store, id), None);
    }

    /// The failure the issue describes: two 1024-d models, so the length check
    /// inside `cosine_similarity` never fires and the swap is silent.
    /// A vector from the old model must not be scored as if it lived in the new
    /// model's space — it gets no similarity at all, which is the same
    /// treatment a dimension-mismatched vector already got.
    #[test]
    fn recall_does_not_score_a_vector_from_a_different_model() {
        let store = setup();
        let agent_id = AgentId::new();

        store.set_active_embedding_model("ollama/bge-m3");
        let stale = remember_with_vector(&store, agent_id, "stale space", &[1.0, 0.0, 0.0]);

        store.set_active_embedding_model("ollama/multilingual-e5-large");
        let fresh = remember_with_vector(&store, agent_id, "current space", &[1.0, 0.0, 0.0]);

        // Identical vectors, so without the guard both would score 1.0 and the
        // stale row would be indistinguishable from a perfect match.
        let hits = store
            .recall_with_embedding("", 10, None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();

        let stale_hit = hits.iter().find(|f| f.id == stale).expect("stale present");
        let fresh_hit = hits.iter().find(|f| f.id == fresh).expect("fresh present");
        assert_eq!(
            stale_hit.similarity, None,
            "a vector from another embedding space must not carry a similarity"
        );
        assert_eq!(fresh_hit.similarity, Some(1.0));
        assert_eq!(
            hits.first().map(|f| f.id),
            Some(fresh),
            "the row in the active space must rank above the unscored one"
        );
    }

    /// A similarity floor is the caller asking for a measured minimum, and an
    /// unscored row cannot clear it — so a stale-model row is dropped outright
    /// rather than surfacing as a weak match.
    #[test]
    fn a_similarity_floor_drops_vectors_from_a_different_model() {
        let store = setup();
        let agent_id = AgentId::new();

        store.set_active_embedding_model("ollama/bge-m3");
        remember_with_vector(&store, agent_id, "stale space", &[1.0, 0.0, 0.0]);
        store.set_active_embedding_model("ollama/multilingual-e5-large");
        let fresh = remember_with_vector(&store, agent_id, "current space", &[1.0, 0.0, 0.0]);

        let filter = MemoryFilter {
            min_similarity: Some(0.5),
            ..Default::default()
        };
        let hits = store
            .recall_with_embedding("", 10, Some(filter), Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, fresh);
    }

    /// Rows written before the v51 stamp carry `NULL` and stay comparable.
    /// The overwhelmingly common case is an operator who never changed the
    /// model, and dropping every historical vector out of retrieval would be a
    /// far worse default than the risk it removes.
    #[test]
    fn a_pre_stamp_vector_is_still_scored() {
        let store = setup();
        let agent_id = AgentId::new();

        // Written with no active model, exactly as a pre-v51 binary would.
        let legacy = remember_with_vector(&store, agent_id, "legacy row", &[1.0, 0.0, 0.0]);
        assert_eq!(stamped_model(&store, legacy), None);

        store.set_active_embedding_model("ollama/multilingual-e5-large");
        let hits = store
            .recall_with_embedding("", 10, None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].similarity, Some(1.0));
    }

    /// The census is what turns a silent corruption into something an operator
    /// can be told about at boot.
    #[test]
    fn the_census_counts_live_vectors_by_producing_model() {
        let store = setup();
        let agent_id = AgentId::new();

        // One unstamped, two from an old model, one from the current one.
        remember_with_vector(&store, agent_id, "legacy", &[1.0, 0.0]);
        store.set_active_embedding_model("ollama/bge-m3");
        remember_with_vector(&store, agent_id, "old a", &[1.0, 0.0]);
        remember_with_vector(&store, agent_id, "old b", &[0.0, 1.0]);
        store.set_active_embedding_model("openai/text-embedding-3-small");
        remember_with_vector(&store, agent_id, "new", &[0.0, 1.0]);
        // A text-only row must not appear at all — it has no vector to attribute.
        remember_raw_dialogue(&store, agent_id, "no vector");

        let census = store.embedding_model_census().unwrap();
        assert_eq!(census.get("ollama/bge-m3"), Some(&2));
        assert_eq!(census.get("openai/text-embedding-3-small"), Some(&1));
        assert_eq!(census.get(UNSTAMPED_EMBEDDING_MODEL), Some(&1));
        assert_eq!(census.len(), 3);
    }

    /// Re-embedding is the repair for a stale vector, so it has to clear the
    /// stale stamp too — otherwise a row rebuilt in the current space stays
    /// permanently excluded from scoring.
    #[test]
    fn re_embedding_a_row_restamps_it_with_the_active_model() {
        let store = setup();
        let agent_id = AgentId::new();

        store.set_active_embedding_model("ollama/bge-m3");
        let id = remember_with_vector(&store, agent_id, "row", &[1.0, 0.0, 0.0]);

        store.set_active_embedding_model("ollama/multilingual-e5-large");
        store.update_embedding(id, &[0.0, 1.0, 0.0]).unwrap();
        assert_eq!(
            stamped_model(&store, id).as_deref(),
            Some("ollama/multilingual-e5-large")
        );

        let hits = store
            .recall_with_embedding("", 10, None, Some(&[0.0, 1.0, 0.0]))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].similarity,
            Some(1.0),
            "the rebuilt vector must be scored again"
        );
    }

    /// `find_duplicates` compares two *stored* vectors and merges the rows that
    /// score above the threshold. A merge is destructive, so a vector from
    /// another embedding space is withheld and that pair drops to the Jaccard
    /// fallback instead of being merged on a meaningless score.
    #[test]
    fn get_embeddings_batch_withholds_vectors_from_a_different_model() {
        let store = setup();
        let agent_id = AgentId::new();

        store.set_active_embedding_model("ollama/bge-m3");
        let stale = remember_with_vector(&store, agent_id, "stale", &[1.0, 0.0]);
        let legacy = remember_with_vector(&store, agent_id, "legacy", &[0.0, 1.0]);
        // Clear the legacy row's stamp so it looks like a pre-v51 write.
        store
            .pool
            .get()
            .unwrap()
            .execute(
                "UPDATE memories SET embedding_model = NULL WHERE id = ?1",
                rusqlite::params![legacy.0.to_string()],
            )
            .unwrap();

        store.set_active_embedding_model("ollama/multilingual-e5-large");
        let fresh = remember_with_vector(&store, agent_id, "fresh", &[1.0, 1.0]);

        let stale_s = stale.0.to_string();
        let legacy_s = legacy.0.to_string();
        let fresh_s = fresh.0.to_string();
        let got = store
            .get_embeddings_batch(&[stale_s.as_str(), legacy_s.as_str(), fresh_s.as_str()])
            .unwrap();

        assert!(
            got.contains_key(&fresh_s),
            "the active model's vector is returned"
        );
        assert!(
            got.contains_key(&legacy_s),
            "an unstamped pre-v51 vector stays available"
        );
        assert!(
            !got.contains_key(&stale_s),
            "a vector stamped with a different model is withheld"
        );
    }

    /// A soft-deleted row is on its way out and must not make an operator think
    /// they have a re-embedding problem they do not have.
    #[test]
    fn the_census_ignores_soft_deleted_rows() {
        let store = setup();
        let agent_id = AgentId::new();
        store.set_active_embedding_model("ollama/bge-m3");
        let id = remember_with_vector(&store, agent_id, "going away", &[1.0, 0.0]);
        store.forget(id).unwrap();
        assert!(store.embedding_model_census().unwrap().is_empty());
    }

    #[test]
    fn eviction_evicts_raw_dialogue_before_extracted_facts() {
        // #7756 §1.2: the per-agent cap is the only exit raw dialogue has, so it must
        // not spend that exit on a fact — not even a fact that scores far lower.
        let store = setup();
        let agent_id = AgentId::new();

        let fact = remember_extracted_fact(&store, agent_id, "Prefers concise answers");
        let raw = remember_raw_dialogue(&store, agent_id, "what is the weather");
        set_confidence(&store, fact, 0.01);
        set_confidence(&store, raw, 1.0);

        assert_eq!(
            store.eviction_candidates(agent_id, 1).unwrap(),
            vec![raw],
            "confidence outranked class"
        );
        // Once raw dialogue is exhausted the fact is next — the class rank changes the
        // order, it does not make facts unevictable.
        assert_eq!(
            store.eviction_candidates(agent_id, 2).unwrap(),
            vec![raw, fact]
        );
    }

    #[test]
    fn eviction_still_orders_by_confidence_within_the_raw_dialogue_class() {
        let store = setup();
        let agent_id = AgentId::new();

        let strong = remember_raw_dialogue(&store, agent_id, "one");
        let weak = remember_raw_dialogue(&store, agent_id, "two");
        set_confidence(&store, strong, 0.9);
        set_confidence(&store, weak, 0.1);

        assert_eq!(
            store.eviction_candidates(agent_id, 2).unwrap(),
            vec![weak, strong]
        );
    }

    #[test]
    fn eviction_does_not_class_imported_episodic_rows_as_raw_dialogue() {
        // Imported rows land in the default `episodic` scope with empty metadata but a
        // different `source`, so only `source` keeps them out of the evict-first class.
        let store = setup();
        let agent_id = AgentId::new();

        let imported = store
            .remember(
                agent_id,
                "Chapter 1 of the handbook",
                MemorySource::Document,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let raw = remember_raw_dialogue(&store, agent_id, "hello");
        set_confidence(&store, imported, 0.01);
        set_confidence(&store, raw, 1.0);

        assert_eq!(store.eviction_candidates(agent_id, 1).unwrap(), vec![raw]);
    }

    #[test]
    fn eviction_bound_holds_under_the_per_agent_cap() {
        // The bound the cap promises: after eviction the agent is at the cap, and the
        // rows spent on getting there are raw dialogue, not facts.
        let store = setup();
        let agent_id = AgentId::new();

        let facts: Vec<MemoryId> = (0..3)
            .map(|i| remember_extracted_fact(&store, agent_id, &format!("fact {i}")))
            .collect();
        for id in &facts {
            // Facts deliberately score lowest, as they do today on any instance that
            // ran under the pre-#7864 decay.
            set_confidence(&store, *id, 0.001);
        }
        for i in 0..7 {
            remember_raw_dialogue(&store, agent_id, &format!("turn {i}"));
        }

        // Cap of 4 over a corpus of 10 means six evictions.
        let doomed = store.eviction_candidates(agent_id, 6).unwrap();
        assert_eq!(doomed.len(), 6);
        for id in &facts {
            assert!(!doomed.contains(id), "an extracted fact was evicted");
        }
        for id in &doomed {
            store.forget(*id).unwrap();
        }
        assert_eq!(store.count(agent_id, None).unwrap(), 4);
    }

    /// Store one memory carrying `embedding`, returning its id.
    fn remember_vec(store: &SemanticStore, agent: AgentId, content: &str, embedding: &[f32]) {
        store
            .remember_with_embedding(
                agent,
                content,
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(embedding),
                None,
                None,
                MemoryModality::Text,
            )
            .unwrap();
    }

    // -----------------------------------------------------------------
    // #7808 — the similarity floor.
    // -----------------------------------------------------------------

    /// The score the ranker computes must survive onto the fragment.
    ///
    /// Before #7808 `cosine_similarity` was called inside the `sort_by`
    /// closure and its result dropped on the floor, so rank order was the only
    /// thing a caller ever learned. Everything below depends on the number
    /// being carried out, so pin that first.
    #[test]
    fn recall_carries_the_cosine_score_onto_each_fragment() {
        let store = setup();
        let agent = AgentId::new();
        remember_vec(&store, agent, "exactly on axis", &[1.0, 0.0]);
        remember_vec(&store, agent, "orthogonal", &[0.0, 1.0]);

        let results = store
            .recall_with_embedding("anything", 10, None, Some(&[1.0, 0.0]))
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "exactly on axis");
        let top = results[0]
            .similarity
            .expect("ranked fragment must carry a score");
        let bottom = results[1]
            .similarity
            .expect("ranked fragment must carry a score");
        assert!(
            (top - 1.0).abs() < 1e-5,
            "identical vectors score 1.0, got {top}"
        );
        assert!(
            bottom.abs() < 1e-5,
            "orthogonal vectors score 0.0, got {bottom}"
        );
    }

    /// A text-match recall measures nothing, so it must report nothing rather
    /// than a plausible-looking zero.
    #[test]
    fn recall_without_an_embedding_reports_no_similarity() {
        let store = setup();
        let agent = AgentId::new();
        remember_vec(
            &store,
            agent,
            "the deploy pipeline runs on Fridays",
            &[1.0, 0.0],
        );

        let results = store.recall("deploy", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].similarity, None,
            "an unranked fragment must carry None, never 0.0 — 0.0 is a measured miss"
        );
    }

    /// The floor decides membership, not merely order: a query whose only
    /// candidates are irrelevant must come back empty.
    ///
    /// This is the failure the floor exists for. With none, recall over-fetches,
    /// ranks, and truncates to top-k, so a sparse store hands the prompt its
    /// least-bad rows and the model reads them as answers.
    #[test]
    fn recall_similarity_floor_excludes_low_scoring_rows() {
        let store = setup();
        let agent = AgentId::new();
        remember_vec(&store, agent, "near match", &[1.0, 0.1]);
        remember_vec(&store, agent, "orthogonal noise", &[0.0, 1.0]);
        remember_vec(&store, agent, "opposite", &[-1.0, 0.0]);

        let query = [1.0_f32, 0.0];

        // No floor: every candidate survives, including the two that answer
        // nothing — the pre-#7808 behaviour, pinned so a regression is visible.
        let unfiltered = store
            .recall_with_embedding("q", 10, None, Some(&query))
            .unwrap();
        assert_eq!(
            unfiltered.len(),
            3,
            "without a floor every candidate is returned"
        );

        let filter = MemoryFilter {
            min_similarity: Some(0.5),
            ..Default::default()
        };
        let filtered = store
            .recall_with_embedding("q", 10, Some(filter), Some(&query))
            .unwrap();
        assert_eq!(
            filtered.len(),
            1,
            "only the near match clears a 0.5 floor; got {:?}",
            filtered
                .iter()
                .map(|f| (&f.content, f.similarity))
                .collect::<Vec<_>>()
        );
        assert_eq!(filtered[0].content, "near match");

        // A floor nothing reaches yields nothing, which is the whole point:
        // "ask for nothing rather than noise" has to be able to return nothing.
        let filter = MemoryFilter {
            min_similarity: Some(0.999),
            ..Default::default()
        };
        let none = store
            .recall_with_embedding("q", 10, Some(filter), Some(&query))
            .unwrap();
        assert!(
            none.is_empty(),
            "an unreachable floor must return an empty set"
        );
    }

    /// A fragment with no stored embedding cannot be measured, so it cannot
    /// clear a floor — promoting it would hand back the one row guaranteed to
    /// violate the guarantee the caller asked for.
    #[test]
    fn recall_similarity_floor_drops_unmeasurable_fragments() {
        let store = setup();
        let agent = AgentId::new();
        remember_vec(&store, agent, "measurable", &[1.0, 0.0]);
        store
            .remember(
                agent,
                "no embedding at all",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        let filter = MemoryFilter {
            min_similarity: Some(0.5),
            ..Default::default()
        };
        let results = store
            .recall_with_embedding("q", 10, Some(filter), Some(&[1.0, 0.0]))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "measurable");
    }

    /// The floor is a post-ranking cut, and there is no ranking to cut on the
    /// text-match path. Applying it there would silently empty the fallback the
    /// no-embedding deployments live on.
    #[test]
    fn recall_similarity_floor_is_inert_without_a_query_embedding() {
        let store = setup();
        let agent = AgentId::new();
        remember_vec(
            &store,
            agent,
            "the deploy pipeline runs on Fridays",
            &[1.0, 0.0],
        );

        let filter = MemoryFilter {
            min_similarity: Some(0.99),
            ..Default::default()
        };
        let results = store.recall("deploy", 10, Some(filter)).unwrap();
        assert_eq!(
            results.len(),
            1,
            "a floor must not empty a recall that measured no similarity"
        );
    }

    // -----------------------------------------------------------------
    // #7808 — memories_fts.
    // -----------------------------------------------------------------

    /// Search with no embedding provider must go through the FTS index, and
    /// that has to be observable rather than inferred: a query whose words
    /// appear in the memory but whose phrasing does not is exactly the query
    /// `content LIKE '%…%'` cannot answer and an inverted index can.
    #[test]
    fn recall_without_embeddings_matches_via_fts_not_substring() {
        let store = setup();
        let agent = AgentId::new();
        store
            .remember(
                agent,
                "The staging deploy pipeline runs every Friday afternoon",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // The old LIKE path looked for this whole string inside the content and
        // found nothing. FTS matches on the terms.
        let results = store
            .recall("when does the deploy pipeline run?", 10, None)
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "a natural-language question must reach the memory whose words it shares"
        );
        assert!(results[0].content.contains("deploy pipeline"));
    }

    /// FTS ranking must survive the hydrating SELECT. `id IN (...)` returns
    /// rows in whatever order SQLite likes, so bm25 order has to be re-imposed
    /// in Rust — otherwise the index picks the candidates and `created_at`
    /// picks the winners, which is the recency bias the over-fetch exists to
    /// avoid.
    #[test]
    fn fts_recall_orders_by_relevance_not_recency() {
        let store = setup();
        let agent = AgentId::new();
        // Written first, so `created_at DESC` would rank it LAST.
        store
            .remember(
                agent,
                "kubernetes ingress certificate rotation runbook",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent,
                "lunch preferences: no coriander",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        store
            .remember(
                agent,
                "the office kubernetes cluster is in Frankfurt",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        let results = store
            .recall("kubernetes ingress certificate rotation", 10, None)
            .unwrap();
        assert!(!results.is_empty());
        assert!(
            results[0].content.contains("runbook"),
            "the memory sharing four query terms must outrank the one sharing one, \
             regardless of write order; got {:?}",
            results.iter().map(|f| &f.content).collect::<Vec<_>>()
        );
        assert!(
            !results.iter().any(|f| f.content.contains("coriander")),
            "a memory sharing no term with the query must not be returned at all"
        );
    }

    /// FTS may only ever add matches. A substring hit that lands inside a word
    /// has no FTS term to match, so the LIKE path must still answer it.
    #[test]
    fn fts_miss_falls_back_to_substring_matching() {
        let store = setup();
        let agent = AgentId::new();
        store
            .remember(
                agent,
                "we deployed librefang to the edge nodes",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // "fang" is not a token in "librefang", so FTS returns nothing and the
        // LIKE fallback has to carry the query.
        let results = store.recall("fang", 10, None).unwrap();
        assert_eq!(
            results.len(),
            1,
            "an FTS miss must fall through to substring matching, not report an empty store"
        );
    }

    /// A query made entirely of punctuation contributes no FTS term. That must
    /// read as "no pre-selection" and fall through, not as "no results".
    #[test]
    fn fts_ignores_a_query_with_no_usable_terms() {
        let store = setup();
        let agent = AgentId::new();
        store
            .remember(
                agent,
                "??? mystery memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        assert_eq!(fts_match_expression("???"), None);
        let results = store.recall("???", 10, None).unwrap();
        assert_eq!(
            results.len(),
            1,
            "the LIKE path still answers a term-free query"
        );
    }

    /// The MATCH expression is built from alphanumeric runs only, so nothing a
    /// caller types can reach FTS5's query grammar as syntax.
    #[test]
    fn fts_match_expression_neutralises_query_syntax() {
        let expression = fts_match_expression("what's \"broken\" NEAR* deploy?").unwrap();
        assert_eq!(
            expression,
            "\"what\" OR \"s\" OR \"broken\" OR \"NEAR\" OR \"deploy\""
        );
        assert!(
            !expression.contains('*') && !expression.contains('\''),
            "no query character may survive as FTS5 syntax: {expression}"
        );
        assert_eq!(
            fts_match_expression("   ").as_deref(),
            None,
            "whitespace contributes no term"
        );
        // The term cap holds, so a pasted document cannot build an unbounded
        // OR-chain for SQLite to parse.
        let long: String = (0..MAX_FTS_TERMS * 3)
            .map(|i| format!("term{i} "))
            .collect();
        let capped = fts_match_expression(&long).unwrap();
        assert_eq!(capped.matches(" OR ").count(), MAX_FTS_TERMS - 1);
    }

    /// The triggers, not the write paths, keep the index in step — so an
    /// edit or a hard delete made anywhere in the crate stays indexed.
    #[test]
    fn memories_fts_tracks_updates_and_deletes() {
        let store = setup();
        let agent = AgentId::new();
        store
            .remember(
                agent,
                "the incident postmortem is scheduled",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        assert_eq!(store.recall("postmortem", 10, None).unwrap().len(), 1);

        // The pool is single-connection in these tests, so every direct SQL
        // step has to release its handle before the store needs one back.
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE memories SET content = 'the incident retrospective is scheduled'",
                [],
            )
            .unwrap();
        }
        assert!(
            store.recall("postmortem", 10, None).unwrap().is_empty(),
            "the old term must leave the index when the row is edited"
        );
        assert_eq!(store.recall("retrospective", 10, None).unwrap().len(), 1);

        let conn = store.pool.get().unwrap();
        conn.execute("DELETE FROM memories", []).unwrap();
        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(orphans, 0, "a hard delete must take its FTS row with it");
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
    fn recall_readonly_does_not_bump_access_count() {
        // #5839 MEDIUM: listing reads must not reset the decay/idle clock.
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

        // Read-only recalls leave access_count untouched no matter how often
        // they run (e.g. a dashboard polling the list every few seconds).
        let c0 = store.recall_readonly("Rust", 10, None).unwrap()[0].access_count;
        store.recall_readonly("Rust", 10, None).unwrap();
        let c1 = store.recall_readonly("Rust", 10, None).unwrap()[0].access_count;
        assert_eq!(c0, c1, "recall_readonly must not bump access_count");

        // A genuine tracking recall still bumps, so decay sees real usage.
        store.recall("Rust", 10, None).unwrap();
        let c2 = store.recall_readonly("Rust", 10, None).unwrap()[0].access_count;
        assert_eq!(c2, c1 + 1, "tracking recall must bump access_count by 1");
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
    fn recall_honors_non_string_metadata_filters() {
        // Regression: a bool / number metadata filter must emit a predicate
        // and actually narrow the result set, not be silently dropped
        // (which returned a superset — the filter appeared to have no effect).
        let store = setup();
        let agent_id = AgentId::new();

        let mut meta_pinned = HashMap::new();
        meta_pinned.insert("pinned".to_string(), serde_json::Value::Bool(true));
        meta_pinned.insert("priority".to_string(), serde_json::Value::Number(5.into()));
        store
            .remember(
                agent_id,
                "Pinned high-priority memory",
                MemorySource::Conversation,
                "episodic",
                meta_pinned,
            )
            .unwrap();

        let mut meta_other = HashMap::new();
        meta_other.insert("pinned".to_string(), serde_json::Value::Bool(false));
        meta_other.insert("priority".to_string(), serde_json::Value::Number(1.into()));
        store
            .remember(
                agent_id,
                "Unpinned low-priority memory",
                MemorySource::Conversation,
                "episodic",
                meta_other,
            )
            .unwrap();

        // Bool filter must select exactly the pinned row.
        let mut f_bool = MemoryFilter::agent(agent_id);
        f_bool
            .metadata
            .insert("pinned".to_string(), serde_json::Value::Bool(true));
        let by_bool = store.recall("memory", 10, Some(f_bool)).unwrap();
        assert_eq!(by_bool.len(), 1, "bool metadata filter must be applied");
        assert_eq!(by_bool[0].content, "Pinned high-priority memory");

        // Number filter must select exactly the priority-5 row.
        let mut f_num = MemoryFilter::agent(agent_id);
        f_num
            .metadata
            .insert("priority".to_string(), serde_json::Value::Number(5.into()));
        let by_num = store.recall("memory", 10, Some(f_num)).unwrap();
        assert_eq!(by_num.len(), 1, "number metadata filter must be applied");
        assert_eq!(by_num[0].content, "Pinned high-priority memory");
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

    /// A VectorStore backend that ignores the MemoryFilter it is handed and
    /// returns every id it was seeded with — modelling a misbehaving or
    /// compromised external backend. Used to prove tenant isolation does not
    /// depend on the backend honouring the filter.
    struct LeakyVectorStore {
        ids: Vec<String>,
    }

    #[async_trait]
    impl VectorStore for LeakyVectorStore {
        async fn insert(
            &self,
            _id: &str,
            _embedding: &[f32],
            _payload: &str,
            _metadata: HashMap<String, serde_json::Value>,
        ) -> LibreFangResult<()> {
            Ok(())
        }

        async fn search(
            &self,
            _query_embedding: &[f32],
            _limit: usize,
            _filter: Option<MemoryFilter>,
        ) -> LibreFangResult<Vec<VectorSearchResult>> {
            // Deliberately ignore `_filter` and leak every seeded id.
            Ok(self
                .ids
                .iter()
                .map(|id| VectorSearchResult {
                    id: id.clone(),
                    payload: String::new(),
                    score: 1.0,
                    metadata: HashMap::new(),
                })
                .collect())
        }

        async fn delete(&self, _id: &str) -> LibreFangResult<()> {
            Ok(())
        }

        async fn get_embeddings(
            &self,
            _ids: &[&str],
        ) -> LibreFangResult<HashMap<String, Vec<f32>>> {
            Ok(HashMap::new())
        }

        fn backend_name(&self) -> &str {
            "leaky-test"
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recall_via_vector_store_reapplies_filter_against_leaky_backend() {
        // Defense-in-depth regression: even when the external VectorStore
        // returns ids for a different agent / peer than the filter requested,
        // the hydration path must re-enforce the MemoryFilter so no
        // cross-tenant content leaks.
        let mut store = setup();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        let id_a = store
            .remember_with_embedding_and_peer(
                agent_a,
                "Alpha secret for agent A",
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
        let id_b = store
            .remember(
                agent_b,
                "Beta secret for agent B",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let id_a_peer_b = store
            .remember_with_embedding_and_peer(
                agent_a,
                "Alpha content but a different peer",
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

        // Backend leaks all three ids regardless of the filter.
        store.set_vector_store(Arc::new(LeakyVectorStore {
            ids: vec![
                id_a.0.to_string(),
                id_b.0.to_string(),
                id_a_peer_b.0.to_string(),
            ],
        }));

        // Recall as agent A / peer user-A. Only the matching fragment must
        // survive hydration; agent B's row and agent A's other-peer row are
        // filtered out despite the backend returning them.
        let mut filter = MemoryFilter::agent(agent_a);
        filter.peer_id = Some("user-A".into());
        let query = [0.1_f32, 0.2, 0.3];
        let results = store
            .recall_with_embedding("secret", 10, Some(filter), Some(&query))
            .unwrap();

        assert_eq!(
            results.len(),
            1,
            "leaky backend must not bypass tenant isolation, got: {:?}",
            results.iter().map(|r| &r.content).collect::<Vec<_>>()
        );
        assert_eq!(results[0].agent_id, agent_a);
        assert_eq!(results[0].content, "Alpha secret for agent A");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recall_via_vector_store_drops_non_uuid_ids_instead_of_failing_the_recall() {
        // An external backend controls the id column, so one row whose id is
        // not a UUID must cost the caller that one row — not the entire
        // recall. Before this fix the parse error propagated out of
        // `recall_via_vector_store` and every hydratable memory in the same
        // result set was denied along with it.
        let mut store = setup();
        let agent = AgentId::new();

        let good = store
            .remember(
                agent,
                "A memory the backend indexed correctly",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();
        let also_good = store
            .remember(
                agent,
                "A second correctly indexed memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .unwrap();

        // The malformed id is first in ANN order, so a fix that only tolerates
        // trailing garbage would not pass this.
        store.set_vector_store(Arc::new(LeakyVectorStore {
            ids: vec![
                "not-a-uuid".to_string(),
                good.0.to_string(),
                String::new(),
                also_good.0.to_string(),
            ],
        }));

        let query = [0.1_f32, 0.2, 0.3];
        let results = store
            .recall_with_embedding("memory", 10, Some(MemoryFilter::agent(agent)), Some(&query))
            .expect("a non-UUID id must not fail the whole recall");

        let contents: Vec<&str> = results.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(
            contents,
            vec![
                "A memory the backend indexed correctly",
                "A second correctly indexed memory"
            ],
            "both hydratable memories must survive, in ANN order"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recall_via_vector_store_reapplies_metadata_filter_against_leaky_backend() {
        // `MemoryFilter.metadata` is a tenancy dimension on the SQLite path
        // (`json_extract(metadata, '$.key') = ?`). The hydrate path's
        // defense-in-depth re-check must enforce it too, or a backend that
        // ignores the filter widens the caller's scope on the external path
        // only.
        let mut store = setup();
        let agent = AgentId::new();

        let mut tenant_a = HashMap::new();
        tenant_a.insert("tenant".to_string(), serde_json::json!("acme"));
        let mut tenant_b = HashMap::new();
        tenant_b.insert("tenant".to_string(), serde_json::json!("globex"));

        let id_a = store
            .remember(
                agent,
                "Acme quarterly numbers",
                MemorySource::Conversation,
                "episodic",
                tenant_a,
            )
            .unwrap();
        let id_b = store
            .remember(
                agent,
                "Globex quarterly numbers",
                MemorySource::Conversation,
                "episodic",
                tenant_b,
            )
            .unwrap();

        store.set_vector_store(Arc::new(LeakyVectorStore {
            ids: vec![id_a.0.to_string(), id_b.0.to_string()],
        }));

        let mut filter = MemoryFilter::agent(agent);
        filter
            .metadata
            .insert("tenant".to_string(), serde_json::json!("acme"));
        let query = [0.1_f32, 0.2, 0.3];
        let results = store
            .recall_with_embedding("quarterly", 10, Some(filter), Some(&query))
            .unwrap();

        assert_eq!(
            results.len(),
            1,
            "the metadata filter must survive the external-backend path, got: {:?}",
            results.iter().map(|r| &r.content).collect::<Vec<_>>()
        );
        assert_eq!(results[0].content, "Acme quarterly numbers");
    }

    /// A VectorStore backend that records every `insert` it receives and can
    /// answer `search` over what it was given — models a real external store
    /// (e.g. Qdrant over HTTP) closely enough to prove the write path reaches
    /// it.
    struct RecordingVectorStore {
        inserted: std::sync::Mutex<Vec<(String, Vec<f32>)>>,
    }

    #[async_trait]
    impl VectorStore for RecordingVectorStore {
        async fn insert(
            &self,
            id: &str,
            embedding: &[f32],
            _payload: &str,
            _metadata: HashMap<String, serde_json::Value>,
        ) -> LibreFangResult<()> {
            // Upsert by id, matching the trait contract ("Insert or update")
            // and a real backend (e.g. Qdrant): a re-insert for the same id
            // replaces the stored vector rather than accumulating duplicates.
            let mut guard = self.inserted.lock().unwrap();
            if let Some(existing) = guard.iter_mut().find(|(rid, _)| rid == id) {
                existing.1 = embedding.to_vec();
            } else {
                guard.push((id.to_string(), embedding.to_vec()));
            }
            Ok(())
        }

        async fn search(
            &self,
            query_embedding: &[f32],
            limit: usize,
            _filter: Option<MemoryFilter>,
        ) -> LibreFangResult<Vec<VectorSearchResult>> {
            let mut scored: Vec<VectorSearchResult> = self
                .inserted
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(id, emb)| {
                    cosine_similarity(query_embedding, emb).map(|score| VectorSearchResult {
                        id: id.clone(),
                        payload: String::new(),
                        score,
                        metadata: HashMap::new(),
                    })
                })
                .collect();
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(limit);
            Ok(scored)
        }

        async fn delete(&self, _id: &str) -> LibreFangResult<()> {
            Ok(())
        }

        async fn get_embeddings(
            &self,
            _ids: &[&str],
        ) -> LibreFangResult<HashMap<String, Vec<f32>>> {
            Ok(HashMap::new())
        }

        fn backend_name(&self) -> &str {
            "recording-test"
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remember_writes_through_to_external_vector_store() {
        // Regression: with an external vector backend attached, `remember`
        // must push the embedding to that backend. Pre-fix the write path only
        // touched SQLite, so the external store stayed empty and every
        // embedding recall against it silently returned nothing.
        let mut store = setup();
        let vs = Arc::new(RecordingVectorStore {
            inserted: std::sync::Mutex::new(Vec::new()),
        });
        store.set_vector_store(vs.clone());

        let agent_id = AgentId::new();
        let embedding = vec![1.0_f32, 0.0, 0.0];
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

        // The external backend must have received the insert.
        {
            let inserted = vs.inserted.lock().unwrap();
            assert_eq!(
                inserted.len(),
                1,
                "remember must write through to the external vector store"
            );
            assert_eq!(inserted[0].0, id.0.to_string());
        }

        // And the memory must be recallable through the vector-store path.
        let query = vec![1.0_f32, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 5, None, Some(&query))
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "memory written through must be recallable via the vector backend"
        );
        assert_eq!(results[0].content, "Rust is great");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn update_embedding_writes_through_to_external_vector_store() {
        // Regression: a content re-embed (`update_embedding`) must mirror the
        // NEW vector to the external backend too. Pre-fix only `remember` wrote
        // through, so after an update the external store kept the OLD embedding
        // and `recall_via_vector_store` ranked against a stale vector — a query
        // matching the new content failed to surface the memory.
        let mut store = setup();
        let vs = Arc::new(RecordingVectorStore {
            inserted: std::sync::Mutex::new(Vec::new()),
        });
        store.set_vector_store(vs.clone());

        let agent_id = AgentId::new();
        let old = vec![1.0_f32, 0.0, 0.0];
        let id = store
            .remember_with_embedding(
                agent_id,
                "cats are great",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&old),
                None,
                None,
                Default::default(),
            )
            .unwrap();

        // Re-embed with an orthogonal vector, as a content update would.
        let new = vec![0.0_f32, 1.0, 0.0];
        store.update_embedding(id, &new).unwrap();

        // The external backend must have received the NEW embedding for this id
        // (a second upsert), not just the original from `remember`.
        {
            let inserted = vs.inserted.lock().unwrap();
            let last_for_id = inserted
                .iter()
                .rev()
                .find(|(rid, _)| *rid == id.0.to_string())
                .expect("update must write through to the external vector store");
            assert_eq!(
                last_for_id.1, new,
                "external store must hold the re-embedded vector, not the stale one"
            );
        }

        // And a query matching the NEW vector must recall the memory via the
        // vector-store path (proves the stale vector was actually replaced).
        let results = store
            .recall_with_embedding("", 5, None, Some(&new))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "cats are great");
    }

    #[test]
    fn vector_recall_considers_old_unaccessed_memories_not_just_recent_window() {
        // Regression: cosine re-ranking must scan a similarity-neutral
        // candidate window, not the N most-recently-accessed rows. An old,
        // rarely-accessed memory that is the true nearest neighbor must still
        // be recalled. Pre-fix the candidate SELECT used
        // `ORDER BY confidence DESC, accessed_at DESC LIMIT 100`, so the target
        // below (oldest accessed_at) fell outside the window and was never
        // cosine-ranked.
        let store = setup();
        let agent_id = AgentId::new();

        // The single best semantic match, stored first and then marked as
        // accessed long ago so it falls outside any recency-ordered window.
        let target_emb = vec![1.0_f32, 0.0, 0.0];
        let target_id = store
            .remember_with_embedding(
                agent_id,
                "old but highly relevant memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&target_emb),
                None,
                None,
                Default::default(),
            )
            .unwrap();
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE memories SET accessed_at = ?1 WHERE id = ?2",
                rusqlite::params!["2000-01-01T00:00:00+00:00", target_id.0.to_string()],
            )
            .unwrap();
        }

        // Flood the store with more recently-accessed, poorly-matching
        // memories so the target is pushed outside the old 100-row recency
        // window.
        let filler_emb = vec![0.0_f32, 1.0, 0.0];
        for i in 0..120 {
            store
                .remember_with_embedding(
                    agent_id,
                    &format!("unrelated filler memory {i}"),
                    MemorySource::Conversation,
                    "episodic",
                    HashMap::new(),
                    Some(&filler_emb),
                    None,
                    None,
                    Default::default(),
                )
                .unwrap();
        }

        let query = vec![1.0_f32, 0.0, 0.0];
        let results = store
            .recall_with_embedding("", 5, None, Some(&query))
            .unwrap();

        assert!(
            results.iter().any(|r| r.id == target_id),
            "the old-but-most-relevant memory must be recalled, got: {:?}",
            results.iter().map(|r| &r.content).collect::<Vec<_>>()
        );
        // It is the exact nearest neighbor, so it must rank first.
        assert_eq!(results[0].id, target_id);
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

    #[test]
    fn test_count_by_agent_uses_one_grouped_snapshot() {
        let store = setup();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        for content in ["A1", "A2"] {
            store
                .remember(
                    agent_a,
                    content,
                    MemorySource::Conversation,
                    "session_memory",
                    HashMap::new(),
                )
                .unwrap();
        }
        store
            .remember(
                agent_b,
                "B1",
                MemorySource::Conversation,
                "user_memory",
                HashMap::new(),
            )
            .unwrap();

        let counts = store.count_by_agent().unwrap();
        assert_eq!(counts.get(&agent_a.to_string()), Some(&2));
        assert_eq!(counts.get(&agent_b.to_string()), Some(&1));

        store.forget_by_agent(agent_a).unwrap();
        let counts = store.count_by_agent().unwrap();
        assert!(!counts.contains_key(&agent_a.to_string()));
        assert_eq!(counts.get(&agent_b.to_string()), Some(&1));
    }

    #[test]
    fn test_list_page_filters_before_count_and_pagination() {
        let store = setup();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        for (agent, scope, category, content) in [
            (agent_a, "user_memory", "keep", "A1"),
            (agent_a, "user_memory", "keep", "A2"),
            (agent_a, "user_memory", "other", "A3"),
            (agent_a, "session_memory", "keep", "A4"),
            (agent_b, "user_memory", "keep", "B1"),
        ] {
            store
                .remember(
                    agent,
                    content,
                    MemorySource::Conversation,
                    scope,
                    HashMap::from([(
                        "category".to_string(),
                        serde_json::Value::String(category.to_string()),
                    )]),
                )
                .unwrap();
        }

        let (first, total) = store
            .list_page(Some(agent_a), Some("keep"), Some("user_memory"), 0, 1)
            .unwrap();
        let (second, second_total) = store
            .list_page(Some(agent_a), Some("keep"), Some("user_memory"), 1, 1)
            .unwrap();

        assert_eq!(total, 2);
        assert_eq!(second_total, 2);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].id, second[0].id);
        for fragment in first.iter().chain(&second) {
            assert_eq!(fragment.agent_id, agent_a);
            assert_eq!(fragment.scope, "user_memory");
            assert_eq!(
                fragment
                    .metadata
                    .get("category")
                    .and_then(|value| value.as_str()),
                Some("keep")
            );
        }
    }

    #[test]
    fn test_list_page_skips_corrupt_metadata_without_inflating_total() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .remember(
                agent_id,
                "healthy memory",
                MemorySource::Conversation,
                "user_memory",
                HashMap::from([(
                    "category".to_string(),
                    serde_json::Value::String("keep".to_string()),
                )]),
            )
            .unwrap();
        let corrupt_id = store
            .remember(
                agent_id,
                "corrupt memory",
                MemorySource::Conversation,
                "user_memory",
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

        let (unfiltered, total) = store
            .list_page(Some(agent_id), None, Some("user_memory"), 0, 10)
            .unwrap();
        let (filtered, filtered_total) = store
            .list_page(Some(agent_id), Some("keep"), Some("user_memory"), 0, 10)
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(filtered_total, 1);
        assert_eq!(unfiltered.len(), 1);
        assert_eq!(filtered.len(), 1);
        assert_eq!(unfiltered[0].content, "healthy memory");
        assert_eq!(filtered[0].content, "healthy memory");
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
