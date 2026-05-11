//! MemorySubstrate: unified implementation of the `Memory` trait.
//!
//! Composes the structured store, semantic store, knowledge store,
//! session store, and consolidation engine behind a single async API.

use crate::chunker;
use crate::consolidation::ConsolidationEngine;
use crate::knowledge::KnowledgeStore;
use crate::migration::run_migrations;
use crate::roster_store::RosterStore;
use crate::semantic::SemanticStore;
use crate::session::{Session, SessionStore};
use crate::structured::StructuredStore;
use crate::usage::UsageStore;
use crate::workflow_store::WorkflowStore;

use async_trait::async_trait;
use librefang_types::agent::{AgentEntry, AgentId, SessionId};
use librefang_types::config::ChunkConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    ConsolidationReport, Entity, ExportFormat, GraphMatch, GraphPattern, ImportReport, Memory,
    MemoryFilter, MemoryFragment, MemoryId, MemorySource, Relation,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The unified memory substrate. Implements the `Memory` trait by delegating
/// to specialized stores backed by a shared SQLite connection pool.
pub struct MemorySubstrate {
    pool: Pool<SqliteConnectionManager>,
    structured: StructuredStore,
    semantic: SemanticStore,
    knowledge: KnowledgeStore,
    sessions: SessionStore,
    consolidation: ConsolidationEngine,
    usage: UsageStore,
    roster: RosterStore,
    workflow_store: WorkflowStore,
    chunk_config: ChunkConfig,
}

/// Default pool size when callers do not pass an explicit value.
///
/// Mirrors `default_memory_pool_size` in `librefang-types::config` so that
/// callers constructing a substrate without a full `MemoryConfig` (tests,
/// the `open` shortcut, ad-hoc tools) still land on a value consistent with
/// what `config.toml: [memory] pool_size` defaults to.
pub const DEFAULT_POOL_SIZE: u32 = 8;

impl MemorySubstrate {
    /// Open or create a memory substrate at the given database path.
    pub fn open(db_path: &Path, decay_rate: f32) -> LibreFangResult<Self> {
        Self::open_with_chunking(db_path, decay_rate, ChunkConfig::default())
    }

    /// Open or create a memory substrate with explicit chunking configuration.
    ///
    /// Uses [`DEFAULT_POOL_SIZE`] for the underlying r2d2 pool; production
    /// callers that need to honour `config.toml: [memory] pool_size` should
    /// use [`Self::open_with_pool_size`] instead.
    pub fn open_with_chunking(
        db_path: &Path,
        decay_rate: f32,
        chunk_config: ChunkConfig,
    ) -> LibreFangResult<Self> {
        Self::open_with_pool_size(db_path, decay_rate, chunk_config, DEFAULT_POOL_SIZE)
    }

    /// Open or create a memory substrate with explicit chunking configuration
    /// **and** pool sizing.
    ///
    /// `pool_size` is the maximum number of pooled SQLite connections; values
    /// of 0 are clamped up to 1 (r2d2 panics on `max_size = 0`). The kernel
    /// boot path passes `config.memory.pool_size` so operators can tune for
    /// their concurrency profile (#3378 follow-up).
    pub fn open_with_pool_size(
        db_path: &Path,
        decay_rate: f32,
        chunk_config: ChunkConfig,
        pool_size: u32,
    ) -> LibreFangResult<Self> {
        // PRAGMAs run on every pooled connection's first checkout. The set
        // mirrors the pre-pool single-connection init: WAL journal for
        // multi-reader concurrency; 5 s busy_timeout so writers wait for the
        // reserved lock instead of failing fast; cache_size=-2000 caps the
        // per-connection page cache at 2 MiB (so total page cache ceiling is
        // `pool_size * 2 MiB`); mmap_size=0 disables mmap'd reads (kept for
        // parity with the pre-pool config — flipping this is a separate
        // decision); foreign_keys=ON enforces the schema FKs the migrations
        // rely on; synchronous=NORMAL is the WAL-default durability/perf
        // tradeoff.
        let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
            c.execute_batch(
                "PRAGMA journal_mode=WAL; \
                 PRAGMA busy_timeout=5000; \
                 PRAGMA cache_size=-2000; \
                 PRAGMA mmap_size=0; \
                 PRAGMA foreign_keys=ON; \
                 PRAGMA synchronous=NORMAL;",
            )
        });
        // Clamp to >= 1: r2d2 panics on `max_size = 0`, and a deserialised
        // `pool_size = 0` (operator typo) should fail soft, not crash boot.
        let max_size = pool_size.max(1);
        let pool = Pool::builder()
            .max_size(max_size)
            .idle_timeout(Some(Duration::from_secs(30)))
            .max_lifetime(Some(Duration::from_secs(3600)))
            .build(manager)
            .map_err(LibreFangError::memory)?;
        // Run migrations with a dedicated connection before any concurrent requests.
        {
            let migration_conn = pool.get().map_err(LibreFangError::memory)?;
            run_migrations(&migration_conn).map_err(LibreFangError::memory)?;
        }

        let sessions = SessionStore::new(pool.clone());
        // Repair any sessions/sessions_fts drift left over from #3451
        // before save_session became transactional.
        sessions.reconcile_fts_index();

        Ok(Self {
            pool: pool.clone(),
            structured: StructuredStore::new(pool.clone()),
            semantic: SemanticStore::new(pool.clone()),
            knowledge: KnowledgeStore::new(pool.clone()),
            sessions,
            usage: UsageStore::new(pool.clone()),
            roster: RosterStore::new(pool.clone()),
            workflow_store: WorkflowStore::new(pool.clone()),
            consolidation: ConsolidationEngine::new(pool, decay_rate),
            chunk_config,
        })
    }

    /// Create an in-memory substrate (for testing).
    pub fn open_in_memory(decay_rate: f32) -> LibreFangResult<Self> {
        Self::open_in_memory_with_chunking(decay_rate, ChunkConfig::default())
    }

    /// Create an in-memory substrate with explicit chunking configuration.
    pub fn open_in_memory_with_chunking(
        decay_rate: f32,
        chunk_config: ChunkConfig,
    ) -> LibreFangResult<Self> {
        let manager = SqliteConnectionManager::memory()
            .with_init(|c| c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;"));
        // in-memory DB: each connection is a separate database, so max_size must be 1.
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(LibreFangError::memory)?;
        {
            let migration_conn = pool.get().map_err(LibreFangError::memory)?;
            run_migrations(&migration_conn).map_err(LibreFangError::memory)?;
        }

        Ok(Self {
            pool: pool.clone(),
            structured: StructuredStore::new(pool.clone()),
            semantic: SemanticStore::new(pool.clone()),
            knowledge: KnowledgeStore::new(pool.clone()),
            sessions: SessionStore::new(pool.clone()),
            usage: UsageStore::new(pool.clone()),
            roster: RosterStore::new(pool.clone()),
            workflow_store: WorkflowStore::new(pool.clone()),
            consolidation: ConsolidationEngine::new(pool, decay_rate),
            chunk_config,
        })
    }

    /// Get a reference to the usage store.
    pub fn usage(&self) -> &UsageStore {
        &self.usage
    }

    /// Get a reference to the knowledge graph store.
    pub fn knowledge(&self) -> &KnowledgeStore {
        &self.knowledge
    }

    /// Get a reference to the group roster store.
    pub fn roster(&self) -> &RosterStore {
        &self.roster
    }

    /// Get a reference to the workflow run store.
    pub fn workflow_store(&self) -> &WorkflowStore {
        &self.workflow_store
    }

    /// Force a WAL checkpoint on the shared connection pool.
    ///
    /// Flushes any pending WAL frames to the main database file. Called
    /// during kernel shutdown to ensure all workflow state transitions
    /// (and other pending writes) are durable on disk.
    pub fn wal_checkpoint(&self) {
        if let Err(e) = self.workflow_store.wal_checkpoint() {
            tracing::warn!("WAL checkpoint failed: {e}");
        }
    }

    /// Attach an external vector store backend to the semantic store.
    ///
    /// When set, [`SemanticStore::recall_with_embedding`] will delegate vector
    /// similarity search to this backend instead of doing in-process cosine
    /// similarity over SQLite BLOBs.
    pub fn set_vector_store(&mut self, store: Arc<dyn librefang_types::memory::VectorStore>) {
        self.semantic.set_vector_store(store);
    }

    /// Clone the inner [`SemanticStore`] for use in blocking tasks.
    ///
    /// This is the bridge used by the [`crate::backend::SemanticBackend`]
    /// implementation to satisfy the `spawn_blocking` pattern without holding a
    /// reference into `self`.
    pub fn semantic_store_clone(&self) -> crate::semantic::SemanticStore {
        self.semantic.clone()
    }

    /// Get a clone of the connection pool (for constructing stores from outside).
    pub fn pool(&self) -> Pool<SqliteConnectionManager> {
        self.pool.clone()
    }

    /// Run time-based memory decay, deleting stale memories based on scope TTL.
    ///
    /// - USER scope: never decays
    /// - SESSION scope: decays after `session_ttl_days` of no access
    /// - AGENT scope: decays after `agent_ttl_days` of no access
    ///
    /// Returns the number of memories deleted.
    pub fn run_decay(
        &self,
        config: &librefang_types::config::MemoryDecayConfig,
    ) -> LibreFangResult<usize> {
        crate::decay::run_decay(&self.pool, config)
    }

    /// Hard-delete soft-deleted memories whose `deleted_at` is older than
    /// `older_than_days` days. Reclaims embedding BLOBs that would otherwise
    /// stay forever in soft-deleted rows (#3467).
    pub fn prune_soft_deleted_memories(&self, older_than_days: u64) -> LibreFangResult<usize> {
        crate::decay::prune_soft_deleted_memories(&self.pool, older_than_days)
    }

    /// Save an agent entry to persistent storage.
    pub fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        self.structured.save_agent(entry)
    }

    /// Load an agent entry from persistent storage.
    pub fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        self.structured.load_agent(agent_id)
    }

    /// Remove an agent and cascade-delete every agent-scoped row in a
    /// single transaction.
    ///
    /// Pre-fix (#3501) sessions and structured rows were deleted in
    /// independent locks/transactions: a failure between the two would
    /// orphan whichever side had not run yet. Now all DELETEs — including
    /// `sessions_fts` — share one `unchecked_transaction` so a partial
    /// cascade rolls back to the pre-call state.
    ///
    /// `sessions_fts` cannot be left outside the rollback path: it stores
    /// session content (`snippet(...)` returns it on any FTS hit) and
    /// `search_sessions` reads from it without joining the `sessions`
    /// table. A `sessions` row removed without its FTS twin would leave
    /// the deleted agent's content searchable, which is a privacy
    /// regression rather than a recoverable hygiene issue.
    pub fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        remove_agent_inner(&conn, agent_id)
    }

    /// Load all agent entries from persistent storage.
    pub fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>> {
        self.structured.load_all_agents()
    }

    /// List all saved agents.
    pub fn list_agents(&self) -> LibreFangResult<Vec<(String, String, String)>> {
        self.structured.list_agents()
    }

    /// Synchronous get from the structured store (for kernel handle use).
    pub fn structured_get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        self.structured.get(agent_id, key)
    }

    /// List all KV pairs for an agent.
    pub fn list_kv(&self, agent_id: AgentId) -> LibreFangResult<Vec<(String, serde_json::Value)>> {
        self.structured.list_kv(agent_id)
    }

    /// List only keys for an agent (without values).
    pub fn list_keys(&self, agent_id: AgentId) -> LibreFangResult<Vec<String>> {
        self.structured.list_keys(agent_id)
    }

    /// Delete a KV entry for an agent.
    pub fn structured_delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()> {
        self.structured.delete(agent_id, key)
    }

    /// Synchronous set in the structured store (for kernel handle use).
    pub fn structured_set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<()> {
        self.structured.set(agent_id, key, value)
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>> {
        self.sessions.get_session(session_id)
    }

    /// Get a session by ID along with its `created_at` timestamp.
    pub fn get_session_with_created_at(
        &self,
        session_id: SessionId,
    ) -> LibreFangResult<Option<(Session, String)>> {
        self.sessions.get_session_with_created_at(session_id)
    }

    /// Save a session.
    pub fn save_session(&self, session: &Session) -> LibreFangResult<()> {
        self.sessions.save_session(session)
    }

    /// Save a session asynchronously on a blocking worker thread.
    pub async fn save_session_async(&self, session: &Session) -> LibreFangResult<()> {
        let sessions = self.sessions.clone();
        let session = session.clone();
        tokio::task::spawn_blocking(move || sessions.save_session(&session))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Create a new empty session for an agent.
    pub fn create_session(&self, agent_id: AgentId) -> LibreFangResult<Session> {
        self.sessions.create_session(agent_id)
    }

    /// List all sessions with metadata.
    pub fn list_sessions(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        self.sessions.list_sessions()
    }

    /// Paginated session listing — pushes LIMIT/OFFSET into SQLite (#3485).
    pub fn list_sessions_paginated(
        &self,
        limit: Option<usize>,
        offset: usize,
    ) -> LibreFangResult<Vec<serde_json::Value>> {
        self.sessions.list_sessions_paginated(limit, offset)
    }

    /// Total number of sessions stored.
    pub fn count_sessions(&self) -> LibreFangResult<usize> {
        self.sessions.count_sessions()
    }

    /// 24-hour KPI rollup for an individual agent — see
    /// [`crate::session::SessionStore::agent_stats_24h`].
    pub fn agent_stats_24h(
        &self,
        agent_id: &str,
    ) -> LibreFangResult<crate::session::AgentStats24h> {
        self.sessions.agent_stats_24h(agent_id)
    }

    /// Bulk `(sessions_24h, cost_24h)` per agent. See
    /// [`crate::session::SessionStore::agents_stats_24h_bulk`].
    pub fn agents_stats_24h_bulk(
        &self,
    ) -> LibreFangResult<std::collections::HashMap<String, (u64, f64)>> {
        self.sessions.agents_stats_24h_bulk()
    }

    /// Delete a session by ID.
    pub fn delete_session(&self, session_id: SessionId) -> LibreFangResult<()> {
        self.sessions.delete_session(session_id)
    }

    /// Return all session IDs belonging to an agent.
    pub fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>> {
        self.sessions.get_agent_session_ids(agent_id)
    }

    /// Delete all sessions belonging to an agent.
    pub fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()> {
        self.sessions.delete_agent_sessions(agent_id)
    }

    /// Count an agent's sessions touched after the given Unix-millis timestamp.
    /// See [`SessionStore::count_agent_sessions_touched_since`] for semantics.
    pub fn count_agent_sessions_touched_since(
        &self,
        agent_id: AgentId,
        since_ms: u64,
        exclude_session: Option<SessionId>,
    ) -> LibreFangResult<u32> {
        self.sessions
            .count_agent_sessions_touched_since(agent_id, since_ms, exclude_session)
    }

    /// List an agent's session IDs touched after the given timestamp, newest
    /// first, capped at `limit`. See
    /// [`SessionStore::list_agent_sessions_touched_since`] for semantics.
    pub fn list_agent_sessions_touched_since(
        &self,
        agent_id: AgentId,
        since_ms: u64,
        limit: u32,
        exclude_session: Option<SessionId>,
    ) -> LibreFangResult<Vec<String>> {
        self.sessions
            .list_agent_sessions_touched_since(agent_id, since_ms, limit, exclude_session)
    }

    /// Delete the canonical (cross-channel) session for an agent.
    pub fn delete_canonical_session(&self, agent_id: AgentId) -> LibreFangResult<()> {
        self.sessions.delete_canonical_session(agent_id)
    }

    /// Set or clear a session label.
    pub fn set_session_label(
        &self,
        session_id: SessionId,
        label: Option<&str>,
    ) -> LibreFangResult<()> {
        self.sessions.set_session_label(session_id, label)
    }

    /// Find a session by label for a given agent.
    pub fn find_session_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> LibreFangResult<Option<Session>> {
        self.sessions.find_session_by_label(agent_id, label)
    }

    /// List all sessions for a specific agent.
    pub fn list_agent_sessions(
        &self,
        agent_id: AgentId,
    ) -> LibreFangResult<Vec<serde_json::Value>> {
        self.sessions.list_agent_sessions(agent_id)
    }

    /// Create a new session with an optional label.
    pub fn create_session_with_label(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> LibreFangResult<Session> {
        self.sessions.create_session_with_label(agent_id, label)
    }

    /// Delete sessions older than `retention_days`. Returns count deleted.
    pub fn cleanup_expired_sessions(&self, retention_days: u32) -> LibreFangResult<u64> {
        self.sessions.cleanup_expired_sessions(retention_days)
    }

    /// For each agent, keep only the newest `max_per_agent` sessions. Returns count deleted.
    pub fn cleanup_excess_sessions(&self, max_per_agent: u32) -> LibreFangResult<u64> {
        self.sessions.cleanup_excess_sessions(max_per_agent)
    }

    /// Delete sessions whose agent_id is not in the provided live set. Returns count deleted.
    pub fn cleanup_orphan_sessions(&self, live_agent_ids: &[AgentId]) -> LibreFangResult<u64> {
        self.sessions.cleanup_orphan_sessions(live_agent_ids)
    }

    /// Run WAL checkpoint then VACUUM if any rows were actually deleted.
    ///
    /// VACUUM rewrites the entire DB file and can take several seconds on
    /// large databases, so it is only worth running when something was
    /// genuinely removed. Callers should pass the total pruned row count
    /// returned by the cleanup_* methods; this function is a no-op when
    /// `pruned_count == 0`.
    ///
    /// VACUUM cannot run inside a transaction, so this method acquires the
    /// connection lock directly and calls `execute_batch`. Errors are logged
    /// as warnings rather than propagated — a failed VACUUM is not fatal.
    pub fn vacuum_if_shrank(&self, pruned_count: usize) -> LibreFangResult<()> {
        if pruned_count == 0 {
            return Ok(());
        }
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        vacuum_inner(&conn, pruned_count);
        Ok(())
    }

    /// Full-text search across session content using FTS5.
    pub fn search_sessions(
        &self,
        query: &str,
        agent_id: Option<&AgentId>,
    ) -> LibreFangResult<Vec<crate::session::SessionSearchResult>> {
        self.sessions.search_sessions(query, agent_id)
    }

    /// Full-text search with SQL-side pagination (#3691).
    ///
    /// Prefer this over `search_sessions` for any caller exposed to the
    /// network: untrusted clients must not be able to ask the substrate
    /// for an unbounded result set.
    pub fn search_sessions_paginated(
        &self,
        query: &str,
        agent_id: Option<&AgentId>,
        limit: Option<usize>,
        offset: usize,
    ) -> LibreFangResult<Vec<crate::session::SessionSearchResult>> {
        self.sessions
            .search_sessions_paginated(query, agent_id, limit, offset)
    }

    /// Load canonical session context for cross-channel memory.
    ///
    /// Returns the compacted summary (if any) and recent messages from the
    /// agent's persistent canonical session.
    pub fn canonical_context(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
        window_size: Option<usize>,
    ) -> LibreFangResult<(Option<String>, Vec<librefang_types::message::Message>)> {
        self.sessions
            .canonical_context(agent_id, session_id, window_size)
    }

    /// Store an LLM-generated summary, replacing older messages with the kept subset.
    ///
    /// Used by the compactor to replace text-truncation compaction with an
    /// LLM-generated summary of older conversation history.
    pub fn store_llm_summary(
        &self,
        agent_id: AgentId,
        summary: &str,
        kept_messages: Vec<librefang_types::message::Message>,
    ) -> LibreFangResult<()> {
        self.sessions
            .store_llm_summary(agent_id, summary, kept_messages)
    }

    /// Write a human-readable JSONL mirror of a session to disk.
    ///
    /// Best-effort — errors are returned but should be logged,
    /// never affecting the primary SQLite store.
    pub fn write_jsonl_mirror(
        &self,
        session: &Session,
        sessions_dir: &Path,
    ) -> Result<(), std::io::Error> {
        self.sessions.write_jsonl_mirror(session, sessions_dir)
    }

    /// Append messages to the agent's canonical session for cross-channel persistence.
    pub fn append_canonical(
        &self,
        agent_id: AgentId,
        messages: &[librefang_types::message::Message],
        compaction_threshold: Option<usize>,
        session_id: Option<SessionId>,
    ) -> LibreFangResult<()> {
        self.sessions
            .append_canonical(agent_id, messages, compaction_threshold, session_id)?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Paired devices persistence
    // -----------------------------------------------------------------

    /// Load all paired devices from the database.
    pub fn load_paired_devices(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT device_id, display_name, platform, paired_at, last_seen, push_token, api_key_hash FROM paired_devices"
        ).map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "device_id": row.get::<_, String>(0)?,
                    "display_name": row.get::<_, String>(1)?,
                    "platform": row.get::<_, String>(2)?,
                    "paired_at": row.get::<_, String>(3)?,
                    "last_seen": row.get::<_, String>(4)?,
                    "push_token": row.get::<_, Option<String>>(5)?,
                    "api_key_hash": row.get::<_, String>(6)?,
                }))
            })
            .map_err(LibreFangError::memory)?;
        let mut devices = Vec::new();
        for row in rows {
            devices.push(row.map_err(LibreFangError::memory)?);
        }
        Ok(devices)
    }

    /// Save a paired device to the database (insert or replace).
    #[allow(clippy::too_many_arguments)]
    pub fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
        api_key_hash: &str,
    ) -> LibreFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices (device_id, display_name, platform, paired_at, last_seen, push_token, api_key_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![device_id, display_name, platform, paired_at, last_seen, push_token, api_key_hash],
        ).map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Remove a paired device from the database.
    pub fn remove_paired_device(&self, device_id: &str) -> LibreFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibreFangError::memory_msg(e.to_string()))?;
        conn.execute(
            "DELETE FROM paired_devices WHERE device_id = ?1",
            rusqlite::params![device_id],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Embedding-aware memory operations
    // -----------------------------------------------------------------

    /// Store a memory with an embedding vector.
    ///
    /// When chunking is enabled and the content exceeds `max_chunk_size`,
    /// the text is split into overlapping chunks. Each chunk is stored as a
    /// separate memory entry with `parent_id` and `chunk_index` in its
    /// metadata. The returned `MemoryId` belongs to the first chunk (the
    /// logical parent).
    pub fn remember_with_embedding(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> LibreFangResult<MemoryId> {
        Self::store_with_chunking(
            &self.semantic,
            &self.chunk_config,
            agent_id,
            content,
            source,
            scope,
            metadata,
            embedding,
        )
    }

    /// Shared chunking + storing logic used by both sync and async paths.
    #[allow(clippy::too_many_arguments)]
    fn store_with_chunking(
        semantic: &SemanticStore,
        chunk_config: &ChunkConfig,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> LibreFangResult<MemoryId> {
        let should_chunk =
            chunk_config.enabled && content.chars().count() > chunk_config.max_chunk_size;

        if !should_chunk {
            return semantic.remember_with_embedding(
                agent_id,
                content,
                source,
                scope,
                metadata,
                embedding,
                None,
                None,
                Default::default(),
            );
        }

        let chunks =
            chunker::chunk_text(content, chunk_config.max_chunk_size, chunk_config.overlap);

        // chunk_text returns [] when max_chunk_size == 0 (or content is
        // empty, though the should_chunk guard above excludes that case).
        // Without this check the .expect() at the end of the loop panics.
        if chunks.is_empty() {
            return Err(LibreFangError::Internal(format!(
                "chunker produced no chunks (content_len={}, max_chunk_size={})",
                content.chars().count(),
                chunk_config.max_chunk_size,
            )));
        }

        // Store the first chunk and use its ID as the parent_id for siblings.
        let mut parent_id: Option<MemoryId> = None;
        let total_chunks = chunks.len();

        for (idx, chunk) in chunks.iter().enumerate() {
            let mut chunk_meta = metadata.clone();
            chunk_meta.insert(
                "chunk_index".to_string(),
                serde_json::Value::Number(serde_json::Number::from(idx)),
            );
            chunk_meta.insert(
                "total_chunks".to_string(),
                serde_json::Value::Number(serde_json::Number::from(total_chunks)),
            );

            if let Some(pid) = &parent_id {
                chunk_meta.insert(
                    "parent_id".to_string(),
                    serde_json::Value::String(pid.0.to_string()),
                );
            }

            // Pass None for chunk embeddings — the original embedding was
            // computed for the full text and is meaningless for individual
            // chunks.  Let the embedding pipeline compute per-chunk embeddings
            // later.
            let id = semantic.remember_with_embedding(
                agent_id,
                chunk,
                source.clone(),
                scope,
                chunk_meta,
                None,
                None,
                None,
                Default::default(),
            )?;

            if parent_id.is_none() {
                parent_id = Some(id);
            }
        }

        Ok(parent_id.expect("chunks is non-empty"))
    }

    /// Recall memories using vector similarity when a query embedding is provided.
    pub fn recall_with_embedding(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        self.semantic
            .recall_with_embedding(query, limit, filter, query_embedding)
    }

    /// Update the embedding for an existing memory.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> LibreFangResult<()> {
        self.semantic.update_embedding(id, embedding)
    }

    /// Async wrapper for `recall_with_embedding` — runs in a blocking thread.
    pub async fn recall_with_embedding_async(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        let store = self.semantic.clone();
        let query = query.to_string();
        let embedding_owned = query_embedding.map(|e| e.to_vec());
        tokio::task::spawn_blocking(move || {
            store.recall_with_embedding(&query, limit, filter, embedding_owned.as_deref())
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for `remember_with_embedding` — runs in a blocking thread.
    ///
    /// Applies chunking when enabled and the content exceeds `max_chunk_size`.
    pub async fn remember_with_embedding_async(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> LibreFangResult<MemoryId> {
        let store = self.semantic.clone();
        let content = content.to_string();
        let scope = scope.to_string();
        let embedding_owned = embedding.map(|e| e.to_vec());
        let chunk_config = self.chunk_config.clone();
        tokio::task::spawn_blocking(move || {
            Self::store_with_chunking(
                &store,
                &chunk_config,
                agent_id,
                &content,
                source,
                &scope,
                metadata,
                embedding_owned.as_deref(),
            )
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    // -----------------------------------------------------------------
    // Task queue operations
    // -----------------------------------------------------------------

    /// Post a new task to the shared queue. Returns the task ID.
    pub async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> LibreFangResult<String> {
        let conn = self.pool.clone();
        let title = title.to_string();
        let description = description.to_string();
        let assigned_to = assigned_to.unwrap_or("").to_string();
        let created_by = created_by.unwrap_or("").to_string();

        tokio::task::spawn_blocking(move || {
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let db = conn.get().map_err(LibreFangError::memory)?;
            db.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, priority, created_at, title, description, assigned_to, created_by)
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![id, &created_by, &title, b"", now, title, description, assigned_to, created_by],
            )
            .map_err(LibreFangError::memory)?;
            Ok(id)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Claim the next pending task (optionally for a specific assignee). Returns task JSON or None.
    ///
    /// `agent_id` must be the canonical UUID. `agent_name` is the human-readable
    /// name for the same agent; tasks posted with a name (rather than UUID) in
    /// `assigned_to` are also matched so that name-based assignments are never
    /// silently dropped (fixes issue #2841).
    pub async fn task_claim(
        &self,
        agent_id: &str,
        agent_name: Option<&str>,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        let conn = self.pool.clone();
        let agent_id = agent_id.to_string();
        let agent_name = agent_name.unwrap_or("").to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            // Match tasks assigned to this agent by UUID *or* by name (tasks posted
            // via the API or bridge tools may store the name rather than the UUID),
            // plus any unassigned (empty assigned_to) pending tasks.
            let mut stmt = db.prepare(
                "SELECT id, title, description, assigned_to, created_by, created_at
                 FROM task_queue
                 WHERE status = 'pending'
                   AND (assigned_to = ?1 OR assigned_to = ?2 OR assigned_to = '')
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1"
            ).map_err(LibreFangError::memory)?;

            let result = stmt.query_row(rusqlite::params![agent_id, agent_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            });

            match result {
                Ok((id, title, description, _assigned, created_by, created_at)) => {
                    // Update status to in_progress and stamp `claimed_at` so the
                    // stuck-task sweeper can TTL-reset workers that never complete.
                    let claimed_at = chrono::Utc::now().to_rfc3339();
                    db.execute(
                        "UPDATE task_queue SET status = 'in_progress', assigned_to = ?2, claimed_at = ?3 WHERE id = ?1",
                        rusqlite::params![id, agent_id, claimed_at],
                    ).map_err(LibreFangError::memory)?;

                    Ok(Some(serde_json::json!({
                        "id": id,
                        "title": title,
                        "description": description,
                        "status": "in_progress",
                        "assigned_to": agent_id,
                        "created_by": created_by,
                        "created_at": created_at,
                        "claimed_at": claimed_at,
                    })))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(LibreFangError::memory(e)),
            }
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Mark a task as completed with a result string.
    pub async fn task_complete(&self, task_id: &str, result: &str) -> LibreFangResult<()> {
        let conn = self.pool.clone();
        let task_id = task_id.to_string();
        let result = result.to_string();

        tokio::task::spawn_blocking(move || {
            let now_chrono = chrono::Utc::now();
            let now = now_chrono.to_rfc3339();
            let now_unix = now_chrono.timestamp();
            let db = conn.get().map_err(LibreFangError::memory)?;
            // `finished_at` is the unix-epoch column the retention sweep reads (#3466).
            let rows = db.execute(
                "UPDATE task_queue SET status = 'completed', result = ?2, completed_at = ?3, finished_at = ?4, claimed_at = NULL WHERE id = ?1",
                rusqlite::params![task_id, result, now, now_unix],
            ).map_err(LibreFangError::memory)?;
            if rows == 0 {
                return Err(LibreFangError::Internal(format!("Task not found: {task_id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Delete a task by ID. Returns true if a row was deleted.
    pub async fn task_delete(&self, task_id: &str) -> LibreFangResult<bool> {
        let conn = self.pool.clone();
        let task_id = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let rows = db
                .execute(
                    "DELETE FROM task_queue WHERE id = ?1",
                    rusqlite::params![task_id],
                )
                .map_err(LibreFangError::memory)?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Retry a failed or completed task by resetting it to pending.
    /// Only resets tasks with status 'completed' or 'failed' — in_progress
    /// tasks are excluded to prevent duplicate execution.
    pub async fn task_retry(&self, task_id: &str) -> LibreFangResult<bool> {
        let conn = self.pool.clone();
        let task_id = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let rows = db
                .execute(
                    "UPDATE task_queue \
                     SET status = 'pending', result = NULL, completed_at = NULL, \
                         finished_at = NULL, claimed_at = NULL \
                     WHERE id = ?1 AND status IN ('completed', 'failed')",
                    rusqlite::params![task_id],
                )
                .map_err(LibreFangError::memory)?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// List tasks, optionally filtered by status.
    pub async fn task_list(&self, status: Option<&str>) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = self.pool.clone();
        let status = status.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &status {
                Some(s) => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result, claimed_at FROM task_queue WHERE status = ?1 ORDER BY created_at DESC",
                    vec![Box::new(s.clone())],
                ),
                None => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result, claimed_at FROM task_queue ORDER BY created_at DESC",
                    vec![],
                ),
            };

            let mut stmt = db.prepare(sql).map_err(LibreFangError::memory)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1).unwrap_or_default(),
                    "description": row.get::<_, String>(2).unwrap_or_default(),
                    "status": row.get::<_, String>(3)?,
                    "assigned_to": row.get::<_, String>(4).unwrap_or_default(),
                    "created_by": row.get::<_, String>(5).unwrap_or_default(),
                    "created_at": row.get::<_, String>(6).unwrap_or_default(),
                    "completed_at": row.get::<_, Option<String>>(7).unwrap_or(None),
                    "result": row.get::<_, Option<String>>(8).unwrap_or(None),
                    "claimed_at": row.get::<_, Option<String>>(9).unwrap_or(None),
                }))
            }).map_err(LibreFangError::memory)?;

            let mut tasks = Vec::new();
            for row in rows {
                tasks.push(row.map_err(LibreFangError::memory)?);
            }
            Ok(tasks)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Reset `in_progress` tasks whose worker stalled without calling
    /// `task_complete` — fixes issue #2923. A task is considered stuck when
    /// `claimed_at` is older than `ttl_secs` seconds from now.
    ///
    /// When `max_retries > 0`: tasks that have already been reset that many
    /// times are marked `failed` instead of pending, preventing infinite retry
    /// loops. Pass `0` to disable the cap (current default behaviour).
    ///
    /// Returns the list of reset task IDs so the caller can log / emit events.
    pub async fn task_reset_stuck(
        &self,
        ttl_secs: u64,
        max_retries: u32,
    ) -> LibreFangResult<Vec<String>> {
        let conn = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;

            let cutoff = chrono::Utc::now()
                - chrono::Duration::from_std(std::time::Duration::from_secs(ttl_secs))
                    .unwrap_or_else(|_| chrono::Duration::seconds(0));
            let cutoff_str = cutoff.to_rfc3339();

            let mut stmt = db
                .prepare(
                    "SELECT id, COALESCE(retry_count, 0) FROM task_queue \
                     WHERE status = 'in_progress' \
                       AND claimed_at IS NOT NULL \
                       AND claimed_at < ?1",
                )
                .map_err(LibreFangError::memory)?;

            let stuck: Vec<(String, u32)> = stmt
                .query_map(rusqlite::params![cutoff_str], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
                })
                .map_err(LibreFangError::memory)?
                .filter_map(|r| r.ok())
                .collect();

            if stuck.is_empty() {
                return Ok(Vec::new());
            }

            let mut reset_ids = Vec::new();
            for (id, retries) in &stuck {
                let exhausted = max_retries > 0 && *retries >= max_retries;
                if exhausted {
                    let now_unix = chrono::Utc::now().timestamp();
                    db.execute(
                        "UPDATE task_queue \
                         SET status = 'failed', assigned_to = '', claimed_at = NULL, \
                             finished_at = ?2, \
                             retry_count = retry_count + 1 \
                         WHERE id = ?1 AND status = 'in_progress'",
                        rusqlite::params![id, now_unix],
                    )
                    .map_err(LibreFangError::memory)?;
                } else {
                    db.execute(
                        "UPDATE task_queue \
                         SET status = 'pending', assigned_to = '', claimed_at = NULL, \
                             retry_count = retry_count + 1 \
                         WHERE id = ?1 AND status = 'in_progress'",
                        rusqlite::params![id],
                    )
                    .map_err(LibreFangError::memory)?;
                }
                reset_ids.push(id.clone());
            }
            Ok(reset_ids)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Get a single task by ID.
    pub async fn task_get(&self, task_id: &str) -> LibreFangResult<Option<serde_json::Value>> {
        let conn = self.pool.clone();
        let task_id = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let mut stmt = db
                .prepare(
                    "SELECT id, title, description, status, assigned_to, created_by, \
                     created_at, completed_at, result, claimed_at, \
                     COALESCE(retry_count, 0) \
                     FROM task_queue WHERE id = ?1",
                )
                .map_err(LibreFangError::memory)?;
            let mut rows = stmt
                .query_map(rusqlite::params![task_id], |row| {
                    Ok(serde_json::json!({
                        "id":           row.get::<_, String>(0)?,
                        "title":        row.get::<_, String>(1).unwrap_or_default(),
                        "description":  row.get::<_, String>(2).unwrap_or_default(),
                        "status":       row.get::<_, String>(3)?,
                        "assigned_to":  row.get::<_, String>(4).unwrap_or_default(),
                        "created_by":   row.get::<_, String>(5).unwrap_or_default(),
                        "created_at":   row.get::<_, String>(6).unwrap_or_default(),
                        "completed_at": row.get::<_, Option<String>>(7).unwrap_or(None),
                        "result":       row.get::<_, Option<String>>(8).unwrap_or(None),
                        "claimed_at":   row.get::<_, Option<String>>(9).unwrap_or(None),
                        "retry_count":  row.get::<_, u32>(10).unwrap_or(0),
                    }))
                })
                .map_err(LibreFangError::memory)?;
            match rows.next() {
                Some(Ok(v)) => Ok(Some(v)),
                Some(Err(e)) => Err(LibreFangError::memory(e)),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Update a task's status to `pending` (reset) or `cancelled`.
    ///
    /// Only `in_progress` / `pending` tasks can be reset to `pending`.
    /// Any non-terminal task can be cancelled.
    /// Returns `false` when the task was not found or the transition is invalid.
    pub async fn task_update_status(
        &self,
        task_id: &str,
        new_status: &str,
    ) -> LibreFangResult<bool> {
        let conn = self.pool.clone();
        let task_id = task_id.to_string();
        let new_status = new_status.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let now_unix = chrono::Utc::now().timestamp();
            let rows = match new_status.as_str() {
                // Reset to pending: clear `finished_at` so a previous
                // `failed` stamp (line 985) doesn't make the row look
                // immediately prune-eligible if it later fails again
                // before the timestamp is refreshed (#3466).
                "pending" => db.execute(
                    "UPDATE task_queue \
                     SET status = 'pending', claimed_at = NULL, assigned_to = '', \
                         finished_at = NULL \
                     WHERE id = ?1 AND status IN ('in_progress', 'failed')",
                    rusqlite::params![task_id],
                ),
                // Cancellation is a terminal transition like complete/fail,
                // so it MUST stamp `finished_at` — otherwise the retention
                // sweep's `finished_at IS NOT NULL` filter excludes
                // cancelled rows forever and `task_queue` grows unbounded
                // for any agent that uses cancel (#3466).
                "cancelled" => db.execute(
                    "UPDATE task_queue \
                     SET status = 'cancelled', finished_at = ?2 \
                     WHERE id = ?1 AND status NOT IN ('completed', 'cancelled')",
                    rusqlite::params![task_id, now_unix],
                ),
                _ => {
                    return Err(LibreFangError::InvalidInput(format!(
                        "Invalid status '{new_status}': only 'pending' and 'cancelled' are allowed"
                    )))
                }
            }
            .map_err(LibreFangError::memory)?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Hard-delete `completed` / `failed` / `cancelled` rows whose
    /// `finished_at` is older than `older_than_days` days. Bounds the
    /// growth of `task_queue` so the queue table doesn't accumulate
    /// every job since the daemon was first installed (#3466).
    ///
    /// Rows with `finished_at IS NULL` (legacy completions written
    /// before migration v29) are ignored — they'll be picked up the
    /// next time their status changes, or operators can backfill with
    /// `UPDATE task_queue SET finished_at = strftime('%s','now') WHERE
    /// status IN ('completed','failed','cancelled') AND finished_at IS NULL`.
    pub async fn task_prune_finished(&self, older_than_days: u64) -> LibreFangResult<usize> {
        if older_than_days == 0 {
            return Ok(0);
        }
        let conn = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let db = conn.get().map_err(LibreFangError::memory)?;
            let cutoff = chrono::Utc::now().timestamp() - (older_than_days as i64) * 86_400;
            let rows = db
                .execute(
                    "DELETE FROM task_queue \
                     WHERE status IN ('completed', 'failed', 'cancelled') \
                       AND finished_at IS NOT NULL AND finished_at < ?1",
                    rusqlite::params![cutoff],
                )
                .map_err(LibreFangError::memory)?;
            Ok(rows)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    // -----------------------------------------------------------------
    // Async wrappers for sync substrate methods invoked from tokio tasks.
    //
    // Each wrapper here moves SQLite I/O onto
    // tokio's blocking thread pool (#3378). Without it, slow INSERTs
    // (FTS5 tokenization, transactional cascades, large UPDATE plans) would
    // park whichever tokio worker thread was running the future, stalling
    // every other future scheduled on that worker until the blocking I/O
    // completed. The underlying sync methods are kept verbatim — they are
    // still used by tests, migrations, and other non-async paths.
    // -----------------------------------------------------------------

    /// Async wrapper for [`Self::save_agent`].
    pub async fn save_agent_async(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        let store = self.structured.clone();
        let entry = entry.clone();
        tokio::task::spawn_blocking(move || store.save_agent(&entry))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::load_all_agents`].
    pub async fn load_all_agents_async(&self) -> LibreFangResult<Vec<AgentEntry>> {
        let store = self.structured.clone();
        tokio::task::spawn_blocking(move || store.load_all_agents())
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::remove_agent`]. Body is shared via
    /// [`remove_agent_inner`] so a future change to the agent-deletion
    /// strategy (e.g. adding a new per-agent table) only has to land in
    /// one place — the sync method and this wrapper both delegate.
    pub async fn remove_agent_async(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let conn = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.get().map_err(LibreFangError::memory)?;
            remove_agent_inner(&conn, agent_id)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::structured_get`].
    ///
    /// No in-tree caller yet — staged for #3378 part 2 (kernel-side
    /// migration of the remaining sync substrate calls). Keep alongside
    /// the other `_async` wrappers so a future dead-code sweep doesn't
    /// remove it before its caller lands.
    pub async fn structured_get_async(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.get(agent_id, &key))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::get_session`].
    pub async fn get_session_async(
        &self,
        session_id: SessionId,
    ) -> LibreFangResult<Option<Session>> {
        let store = self.sessions.clone();
        tokio::task::spawn_blocking(move || store.get_session(session_id))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::get_agent_session_ids`].
    ///
    /// No in-tree caller yet — staged for #3378 part 2 (kernel-side
    /// migration of the remaining sync substrate calls). Keep alongside
    /// the other `_async` wrappers so a future dead-code sweep doesn't
    /// remove it before its caller lands.
    pub async fn get_agent_session_ids_async(
        &self,
        agent_id: AgentId,
    ) -> LibreFangResult<Vec<SessionId>> {
        let store = self.sessions.clone();
        tokio::task::spawn_blocking(move || store.get_agent_session_ids(agent_id))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::delete_canonical_session`].
    ///
    /// No in-tree caller yet — staged for #3378 part 2 (kernel-side
    /// migration of the remaining sync substrate calls). Keep alongside
    /// the other `_async` wrappers so a future dead-code sweep doesn't
    /// remove it before its caller lands.
    pub async fn delete_canonical_session_async(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let store = self.sessions.clone();
        tokio::task::spawn_blocking(move || store.delete_canonical_session(agent_id))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::append_canonical`].
    pub async fn append_canonical_async(
        &self,
        agent_id: AgentId,
        messages: &[librefang_types::message::Message],
        compaction_threshold: Option<usize>,
        session_id: Option<SessionId>,
    ) -> LibreFangResult<()> {
        let store = self.sessions.clone();
        let messages = messages.to_vec();
        tokio::task::spawn_blocking(move || {
            store.append_canonical(agent_id, &messages, compaction_threshold, session_id)?;
            Ok(())
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Async wrapper for [`Self::vacuum_if_shrank`]. VACUUM rewrites the
    /// whole DB file and can take seconds — keeping it on the blocking
    /// pool is even more important than for the small CRUD wrappers above.
    /// Body is shared via [`vacuum_inner`] so the sync path and this
    /// wrapper stay in lockstep.
    pub async fn vacuum_if_shrank_async(&self, pruned_count: usize) -> LibreFangResult<()> {
        if pruned_count == 0 {
            return Ok(());
        }
        let conn = self.pool.clone();
        tokio::task::spawn_blocking(move || -> LibreFangResult<()> {
            let conn = conn.get().map_err(LibreFangError::memory)?;
            vacuum_inner(&conn, pruned_count);
            Ok(())
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }
}

/// Shared body for [`MemorySubstrate::remove_agent`] and its async sibling.
/// Both helpers share their canonical DELETE list with the standalone
/// `*_agent` methods on the individual stores so a new agent-scoped table
/// only has to be added in one place.
///
/// The caller passes in an already-acquired `PooledConnection`; this
/// function only owns the transaction lifecycle.
fn remove_agent_inner(conn: &Connection, agent_id: AgentId) -> LibreFangResult<()> {
    let id = agent_id.0.to_string();
    let tx = conn
        .unchecked_transaction()
        .map_err(LibreFangError::memory)?;
    crate::session::execute_session_agent_deletes(&tx, &id)?;
    crate::structured::execute_structured_agent_deletes(&tx, &id)?;
    tx.commit().map_err(LibreFangError::memory)?;
    Ok(())
}

/// Shared body for [`MemorySubstrate::vacuum_if_shrank`] and its async
/// sibling. Errors are logged as warnings rather than propagated — a
/// failed VACUUM is not fatal.
///
/// Caller is responsible for the `pruned_count == 0` short-circuit and
/// for passing an already-acquired `PooledConnection`.
fn vacuum_inner(conn: &Connection, pruned_count: usize) {
    // Flush WAL frames to the main DB file first so VACUUM has less work.
    if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
        tracing::warn!(error = %e, "WAL checkpoint before VACUUM failed; continuing");
    }
    tracing::info!(pruned_count, "Running VACUUM after session prune");
    if let Err(e) = conn.execute_batch("VACUUM;") {
        tracing::warn!(error = %e, "VACUUM after session prune failed");
    }
}

#[async_trait]
impl Memory for MemorySubstrate {
    async fn get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.get(agent_id, &key))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<()> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.set(agent_id, &key, value))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.delete(agent_id, &key))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> LibreFangResult<MemoryId> {
        // Delegate to remember_with_embedding (no embedding) which handles chunking.
        self.remember_with_embedding_async(agent_id, content, source, scope, metadata, None)
            .await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        let store = self.semantic.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || store.recall(&query, limit, filter))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn forget(&self, id: MemoryId) -> LibreFangResult<()> {
        let store = self.semantic.clone();
        tokio::task::spawn_blocking(move || store.forget(id))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn add_entity(&self, entity: Entity) -> LibreFangResult<String> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.add_entity(entity, ""))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn add_relation(&self, relation: Relation) -> LibreFangResult<String> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.add_relation(relation, ""))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.query_graph(pattern))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn consolidate(&self) -> LibreFangResult<ConsolidationReport> {
        let engine = self.consolidation.clone();
        tokio::task::spawn_blocking(move || engine.consolidate())
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn export(&self, format: ExportFormat) -> LibreFangResult<Vec<u8>> {
        let _ = format;
        Ok(Vec::new())
    }

    async fn import(&self, _data: &[u8], _format: ExportFormat) -> LibreFangResult<ImportReport> {
        Ok(ImportReport {
            entities_imported: 0,
            relations_imported: 0,
            memories_imported: 0,
            errors: vec!["Import not yet implemented in Phase 1".to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_substrate_kv() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();
        substrate
            .set(agent_id, "key", serde_json::json!("value"))
            .await
            .unwrap();
        let val = substrate.get(agent_id, "key").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("value")));
    }

    #[tokio::test]
    async fn test_substrate_remember_recall() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();
        substrate
            .remember(
                agent_id,
                "Rust is a great language",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();
        let results = substrate.recall("Rust", 10, None).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_task_post_and_list() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let id = substrate
            .task_post(
                "Review code",
                "Check the auth module for issues",
                Some("auditor"),
                Some("orchestrator"),
            )
            .await
            .unwrap();
        assert!(!id.is_empty());

        let tasks = substrate.task_list(Some("pending")).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "Review code");
        assert_eq!(tasks[0]["assigned_to"], "auditor");
        assert_eq!(tasks[0]["status"], "pending");
    }

    #[tokio::test]
    async fn test_task_claim_and_complete() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post(
                "Audit endpoint",
                "Security audit the /api/login endpoint",
                Some("auditor"),
                None,
            )
            .await
            .unwrap();

        // Claim the task (name stored in assigned_to; pass matching name param)
        let claimed = substrate
            .task_claim("auditor", Some("auditor"))
            .await
            .unwrap();
        assert!(claimed.is_some());
        let claimed = claimed.unwrap();
        assert_eq!(claimed["id"], task_id);
        assert_eq!(claimed["status"], "in_progress");

        // Complete the task
        substrate
            .task_complete(&task_id, "No vulnerabilities found")
            .await
            .unwrap();

        // Verify it shows as completed
        let tasks = substrate.task_list(Some("completed")).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["result"], "No vulnerabilities found");
    }

    #[tokio::test]
    async fn test_task_claim_empty() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let claimed = substrate.task_claim("nobody", None).await.unwrap();
        assert!(claimed.is_none());
    }

    /// Tasks posted with an agent *name* in assigned_to must be claimable when
    /// the claimer passes the corresponding UUID + name (issue #2841).
    #[tokio::test]
    async fn test_task_claim_by_name_when_posted_with_name() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        // External actor posts task using agent name (not UUID)
        let task_id = substrate
            .task_post(
                "Analyse logs",
                "Check for anomalies",
                Some("researcher"),
                None,
            )
            .await
            .unwrap();

        let fake_uuid = "4c549884-2aa1-4860-a5bb-f0aa35a1bf7e";

        // Claim with UUID only — should NOT match name-stored task
        let not_claimed = substrate.task_claim(fake_uuid, None).await.unwrap();
        assert!(
            not_claimed.is_none(),
            "UUID-only claim should not match name-assigned task"
        );

        // Claim with UUID + matching name — should match
        let claimed = substrate
            .task_claim(fake_uuid, Some("researcher"))
            .await
            .unwrap();
        assert!(
            claimed.is_some(),
            "UUID+name claim must match name-assigned task"
        );
        let claimed = claimed.unwrap();
        assert_eq!(claimed["id"], task_id);
        assert_eq!(claimed["status"], "in_progress");
        // assigned_to should be normalised to the claimer's UUID after claim
        assert_eq!(claimed["assigned_to"], fake_uuid);
    }

    /// Stuck `in_progress` tasks (worker LLM stalled, no `task_complete` call)
    /// must be automatically reset to `pending` once `claimed_at` is older than
    /// the configured TTL (issue #2923 / #2926).
    #[tokio::test]
    async fn test_task_reset_stuck_expires_in_progress() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post("Long task", "Takes forever", Some("worker"), None)
            .await
            .unwrap();

        // Worker claims the task.
        let claimed = substrate
            .task_claim("worker", Some("worker"))
            .await
            .unwrap();
        assert!(claimed.is_some());
        assert_eq!(claimed.as_ref().unwrap()["status"], "in_progress");

        // Simulate the worker stalling: back-date `claimed_at` to 5 minutes ago
        // so a TTL of 60 s trips and a TTL of 3600 s does not.
        {
            let conn = substrate.pool.get().unwrap();
            let five_min_ago = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
            conn.execute(
                "UPDATE task_queue SET claimed_at = ?1 WHERE id = ?2",
                rusqlite::params![five_min_ago, task_id],
            )
            .unwrap();
        }

        // With a 1 hour TTL, nothing should be reset (not stuck yet).
        let reset = substrate.task_reset_stuck(3600, 0).await.unwrap();
        assert!(
            reset.is_empty(),
            "TTL larger than stall should not reset any task"
        );
        let still_in_progress = substrate.task_list(Some("in_progress")).await.unwrap();
        assert_eq!(still_in_progress.len(), 1);

        // With a 60 s TTL, the stuck task should be flipped back to pending.
        let reset = substrate.task_reset_stuck(60, 0).await.unwrap();
        assert_eq!(reset, vec![task_id.clone()]);

        let pending = substrate.task_list(Some("pending")).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["id"], task_id);
        assert_eq!(pending[0]["assigned_to"], "");
        assert!(
            pending[0]["claimed_at"].is_null(),
            "claimed_at must be cleared on auto-reset"
        );
        let in_progress = substrate.task_list(Some("in_progress")).await.unwrap();
        assert!(in_progress.is_empty());

        // Second sweep is a no-op — stuck task is already pending.
        let reset_again = substrate.task_reset_stuck(60, 0).await.unwrap();
        assert!(reset_again.is_empty());
    }

    #[tokio::test]
    async fn test_chunking_short_text_passthrough() {
        let config = ChunkConfig {
            enabled: true,
            max_chunk_size: 1500,
            overlap: 200,
        };
        let substrate = MemorySubstrate::open_in_memory_with_chunking(0.1, config).unwrap();
        let agent_id = AgentId::new();
        // Short text should be stored as a single memory.
        substrate
            .remember(
                agent_id,
                "Short text",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();
        let results = substrate.recall("Short", 10, None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Short text"));
    }

    #[tokio::test]
    async fn test_chunking_long_text_splits() {
        let config = ChunkConfig {
            enabled: true,
            max_chunk_size: 100,
            overlap: 20,
        };
        let substrate = MemorySubstrate::open_in_memory_with_chunking(0.1, config).unwrap();
        let agent_id = AgentId::new();

        // Create text that exceeds max_chunk_size.
        let long_text = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        assert!(long_text.len() > 100);

        substrate
            .remember(
                agent_id,
                &long_text,
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();

        // Should have stored multiple chunks.
        let results = substrate.recall("fox", 20, None).await.unwrap();
        assert!(
            results.len() > 1,
            "expected multiple chunks, got {}",
            results.len()
        );

        // Each chunk should have chunk_index metadata.
        for result in &results {
            assert!(
                result.metadata.contains_key("chunk_index"),
                "chunk should have chunk_index metadata"
            );
            assert!(
                result.metadata.contains_key("total_chunks"),
                "chunk should have total_chunks metadata"
            );
        }
    }

    #[tokio::test]
    async fn test_chunking_does_not_share_embedding_across_chunks() {
        let config = ChunkConfig {
            enabled: true,
            max_chunk_size: 100,
            overlap: 20,
        };
        let substrate = MemorySubstrate::open_in_memory_with_chunking(0.1, config).unwrap();
        let agent_id = AgentId::new();
        let embedding = vec![0.1, 0.2, 0.3];
        let long_text = "The quick brown fox jumps over the lazy dog. ".repeat(10);

        substrate
            .remember_with_embedding_async(
                agent_id,
                &long_text,
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&embedding),
            )
            .await
            .unwrap();

        // Recall without embedding (FTS) so we can inspect all stored chunks.
        let results = substrate.recall("fox", 20, None).await.unwrap();

        assert!(results.len() > 1, "expected multiple chunks");
        // Chunks should NOT carry the original full-text embedding.
        assert!(
            results.iter().all(|result| result.embedding.is_none()),
            "chunks should not have the original full-text embedding"
        );
    }

    #[tokio::test]
    async fn test_chunking_disabled_stores_as_single() {
        let config = ChunkConfig {
            enabled: false,
            max_chunk_size: 100,
            overlap: 20,
        };
        let substrate = MemorySubstrate::open_in_memory_with_chunking(0.1, config).unwrap();
        let agent_id = AgentId::new();

        let long_text = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        substrate
            .remember(
                agent_id,
                &long_text,
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();

        // With chunking disabled, should store as one entry.
        let results = substrate.recall("fox", 20, None).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    /// `task_complete` must stamp `finished_at` so the retention sweep can
    /// hard-delete the row later (#3466).
    #[tokio::test]
    async fn test_task_complete_stamps_finished_at() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post("t", "d", Some("worker"), None)
            .await
            .unwrap();
        let _ = substrate
            .task_claim("worker", Some("worker"))
            .await
            .unwrap();
        substrate.task_complete(&task_id, "ok").await.unwrap();

        let conn = substrate.pool.get().unwrap();
        let finished_at: Option<i64> = conn
            .query_row(
                "SELECT finished_at FROM task_queue WHERE id = ?1",
                rusqlite::params![&task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            finished_at.is_some(),
            "task_complete must stamp finished_at"
        );
    }

    #[tokio::test]
    async fn test_task_prune_finished_respects_age_and_status() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let now_unix = chrono::Utc::now().timestamp();
        let old_unix = now_unix - 30 * 86_400; // 30 days ago
        let recent_unix = now_unix - 86_400; // 1 day ago

        // Insert directly to control finished_at precisely.
        {
            let conn = substrate.pool.get().unwrap();
            conn.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, created_at, completed_at, finished_at) \
                 VALUES ('old-done', 'a', 't', x'00', 'completed', ?1, ?1, ?2)",
                rusqlite::params![chrono::Utc::now().to_rfc3339(), old_unix],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, created_at, completed_at, finished_at) \
                 VALUES ('old-failed', 'a', 't', x'00', 'failed', ?1, NULL, ?2)",
                rusqlite::params![chrono::Utc::now().to_rfc3339(), old_unix],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, created_at, completed_at, finished_at) \
                 VALUES ('recent-done', 'a', 't', x'00', 'completed', ?1, ?1, ?2)",
                rusqlite::params![chrono::Utc::now().to_rfc3339(), recent_unix],
            )
            .unwrap();
            // Pending row must NEVER be pruned regardless of age.
            conn.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, created_at) \
                 VALUES ('pending-old', 'a', 't', x'00', 'pending', ?1)",
                rusqlite::params![chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        }

        let pruned = substrate.task_prune_finished(7).await.unwrap();
        assert_eq!(pruned, 2, "the two 30-day-old terminal rows should go");

        let conn = substrate.pool.get().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "recent-done + pending-old remain");
    }

    #[tokio::test]
    async fn test_task_prune_finished_zero_disabled() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let pruned = substrate.task_prune_finished(0).await.unwrap();
        assert_eq!(pruned, 0);
    }

    /// `task_update_status("cancelled")` must stamp `finished_at`,
    /// otherwise cancelled rows are excluded from the retention sweep
    /// forever (sweep filters by `finished_at IS NOT NULL`) and the
    /// queue table grows unbounded for any agent that uses cancel
    /// (#3466).
    #[tokio::test]
    async fn test_task_cancel_stamps_finished_at() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post("t", "d", Some("worker"), None)
            .await
            .unwrap();

        let changed = substrate
            .task_update_status(&task_id, "cancelled")
            .await
            .unwrap();
        assert!(changed, "cancellation of a pending task must update");

        let conn = substrate.pool.get().unwrap();
        let (status, finished_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, finished_at FROM task_queue WHERE id = ?1",
                rusqlite::params![&task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "cancelled");
        assert!(
            finished_at.is_some(),
            "task_update_status('cancelled') must stamp finished_at so the \
             retention sweep can hard-delete the row later"
        );
    }

    /// Reset-to-pending must clear `finished_at`. Otherwise a row that
    /// was failed once and reset would carry a stale `finished_at`, and
    /// the next failure could leave it eligible for prune sooner than
    /// the configured retention window if the new fail path's stamp
    /// got skipped for any reason.
    #[tokio::test]
    async fn test_task_reset_to_pending_clears_finished_at() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post("t", "d", Some("worker"), None)
            .await
            .unwrap();

        // Force a `failed` row with a stale `finished_at` directly.
        {
            let conn = substrate.pool.get().unwrap();
            conn.execute(
                "UPDATE task_queue SET status = 'failed', finished_at = ?2 WHERE id = ?1",
                rusqlite::params![&task_id, chrono::Utc::now().timestamp() - 86_400],
            )
            .unwrap();
        }

        let changed = substrate
            .task_update_status(&task_id, "pending")
            .await
            .unwrap();
        assert!(changed);

        let conn = substrate.pool.get().unwrap();
        let (status, finished_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, finished_at FROM task_queue WHERE id = ?1",
                rusqlite::params![&task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert!(
            finished_at.is_none(),
            "reset to pending must clear finished_at"
        );
    }

    /// #3501: `remove_agent` cascade-deletes sessions, memories, and the
    /// agent row in a single transaction. Pre-fix the cascade ran across
    /// multiple independent locks: a partial failure between the two
    /// could orphan sessions whose agent had already been removed.
    #[tokio::test]
    async fn test_remove_agent_cascades_sessions_and_memories() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();

        // Seed: a session and a memory under this agent.
        let session = substrate.create_session(agent_id).unwrap();
        substrate
            .remember(
                agent_id,
                "remember me",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();

        // Sanity: both rows exist.
        assert!(substrate.get_session(session.id).unwrap().is_some());
        let pre_count: i64 = substrate
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE agent_id = ?1 AND deleted = 0",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pre_count, 1);

        substrate.remove_agent(agent_id).unwrap();

        // Sessions, memories, and the agent row must all be gone.
        let conn = substrate.pool.get().unwrap();
        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE agent_id = ?1",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 0, "sessions must cascade-delete");

        let memory_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE agent_id = ?1",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(memory_count, 0, "memories must cascade-delete");
    }

    /// `remove_agent` must also wipe the `sessions_fts` index inside the
    /// cascade transaction. `search_sessions` reads FTS rows directly
    /// (no JOIN to `sessions`), so an orphan FTS row would let the
    /// removed agent's session content remain full-text searchable —
    /// a privacy regression, not a hygiene issue.
    #[tokio::test]
    async fn test_remove_agent_clears_sessions_fts() {
        use librefang_types::message::Message;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();

        // Seed a session whose content lands in the FTS index.
        let mut session = substrate.create_session(agent_id).unwrap();
        let needle = "removalprivacynonceabc123";
        session.messages.push(Message::user(needle));
        substrate.save_session(&session).unwrap();

        // Sanity: FTS sees it.
        let pre = substrate.search_sessions(needle, Some(&agent_id)).unwrap();
        assert!(!pre.is_empty(), "FTS must index the seeded content");

        substrate.remove_agent(agent_id).unwrap();

        // After remove_agent, the FTS row must be gone. If it survived
        // outside the tx, search_sessions would still surface a snippet
        // of the deleted agent's content.
        let post = substrate.search_sessions(needle, Some(&agent_id)).unwrap();
        assert!(
            post.is_empty(),
            "sessions_fts must be cleared inside remove_agent's tx"
        );

        // Also assert at the row level — search_sessions could in principle
        // filter by JOIN in the future; the underlying table must be empty.
        let conn = substrate.pool.get().unwrap();
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions_fts WHERE agent_id = ?1",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0, "sessions_fts must cascade-delete");
    }

    /// #3378: the r2d2 pool + WAL journal mode allow multiple readers to
    /// run concurrently without blocking each other. This test acquires 4
    /// pool connections simultaneously, each holding the connection for a
    /// fixed 50 ms window. If the pool serialised callers (old
    /// Mutex<Connection> behaviour), the batch would take ≥ 200 ms;
    /// with a pool size of 8 and WAL readers don't block each other, so
    /// multiple tasks can hold pooled connections simultaneously.
    #[tokio::test]
    async fn pool_enables_concurrent_reads() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        use std::sync::Arc as StdArc;
        use std::time::Duration;

        // File-backed DB so WAL journal_mode is active; in-memory pools
        // are max_size=1 and cannot exercise multi-reader concurrency.
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("concurrent_reads.db");
        let substrate = StdArc::new(MemorySubstrate::open(&db_path, 0.1).unwrap());

        // Seed one agent so reads hit real SQL rows.
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "reader-seed".to_string(),
            session_id: SessionId::new(),
            ..Default::default()
        };
        substrate.save_agent_async(&entry).await.unwrap();

        // Ordering-based proof: track how many tasks hold a pooled
        // connection simultaneously. If the pool serialises (old
        // Mutex<Connection> behaviour), max_concurrent stays at 1.
        // With a real pool and WAL readers, multiple tasks overlap and
        // max_concurrent reaches >= 2.
        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_concurrent = StdArc::new(AtomicUsize::new(0));

        let hold = Duration::from_millis(50);
        let pool = substrate.pool();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let p = pool.clone();
                let in_flight = StdArc::clone(&in_flight);
                let max_concurrent = StdArc::clone(&max_concurrent);
                tokio::task::spawn_blocking(move || {
                    let conn = p.get().expect("pool get");
                    // Record entry into the concurrent hold window.
                    let current = in_flight.fetch_add(1, SeqCst) + 1;
                    // Update the observed maximum atomically.
                    let mut prev = max_concurrent.load(SeqCst);
                    while current > prev {
                        match max_concurrent.compare_exchange_weak(prev, current, SeqCst, SeqCst) {
                            Ok(_) => break,
                            Err(actual) => prev = actual,
                        }
                    }
                    // Real read inside the held connection.
                    let _count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
                        .unwrap_or(0);
                    std::thread::sleep(hold);
                    in_flight.fetch_sub(1, SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.await.unwrap();
        }

        let observed = max_concurrent.load(SeqCst);
        assert!(
            observed >= 2,
            "expected >= 2 tasks to hold pooled connections concurrently, \
             but max_concurrent = {}. Pool concurrency or WAL may be broken (#3378)",
            observed,
        );
    }

    /// #3378: each `_async` substrate wrapper must offload its
    /// connection pool acquisition to tokio's blocking
    /// pool. This test holds the connection mutex from a non-tokio OS
    /// thread, then drives a wrapper from a `current_thread` runtime.
    /// If the wrapper took the lock on the runtime worker (the pre-fix
    /// kernel pattern), the spawned task would block for the full hold
    /// time AND a concurrently-scheduled tokio future on the same
    /// runtime would never make progress — the runtime has only one
    /// worker thread, the test thread itself. Putting the lock behind
    /// `spawn_blocking` lets the worker pump other futures while the
    /// DB I/O runs on a dedicated thread.
    ///
    /// The proof is **ordering**, not wall-clock: a tokio task captures
    /// `tick_at` after a short sleep, the blocker thread captures
    /// `released_at` immediately after dropping the lock guard, and we
    /// assert `tick_at < released_at`. In the correct (offloaded) case
    /// the tick fires *during* the lock hold, so it wins. In the broken
    /// case the runtime is parked until the lock is released, so the
    /// blocker captures `released_at` before the runtime can resume the
    /// tick task. No timing threshold — the test stays decisive even
    /// under heavy CI jitter (Windows / llvm-cov).
    #[test]
    fn async_wrappers_do_not_park_current_thread_runtime() {
        use std::sync::Mutex as StdMutex;
        use std::time::Instant;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
            let entry = AgentEntry {
                id: AgentId::new(),
                name: "starvation-test".to_string(),
                session_id: SessionId::new(),
                ..Default::default()
            };

            // Round-trip through save_agent_async + load_all_agents_async
            // first to confirm the wrappers persist correctly before we
            // assert on scheduling behaviour.
            substrate.save_agent_async(&entry).await.unwrap();
            let loaded = substrate.load_all_agents_async().await.unwrap();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].name, "starvation-test");

            // Both timestamps are captured under their own mutex so the
            // assertion can read them after both threads have settled.
            let tick_at: Arc<StdMutex<Option<Instant>>> = Arc::new(StdMutex::new(None));
            let released_at: Arc<StdMutex<Option<Instant>>> = Arc::new(StdMutex::new(None));

            // Saturate the connection pool from outside tokio. The
            // 100 ms hold is well above any plausible scheduler jitter
            // (Windows / coverage runners ~50 ms) so the in-hold tick
            // window is unambiguous.
            let pool = substrate.pool.clone();
            let blocker_holds = Arc::new(std::sync::Barrier::new(2));
            let blocker_holds_inner = Arc::clone(&blocker_holds);
            let released_at_for_blocker = Arc::clone(&released_at);
            let blocker = std::thread::spawn(move || {
                // Acquire the sole connection from the max_size=1 pool,
                // blocking any concurrent pool.get() for the hold duration.
                let g = pool.get().expect("pool get");
                blocker_holds_inner.wait();
                std::thread::sleep(std::time::Duration::from_millis(100));
                drop(g);
                // Capture the release timestamp the instant the guard
                // is gone, before any runtime worker can resume.
                *released_at_for_blocker.lock().unwrap() = Some(Instant::now());
            });
            blocker_holds.wait();

            // While the mutex is held, kick off a wrapper that wants
            // it. With spawn_blocking it parks on the blocking pool,
            // not the runtime. The runtime stays free to drive the
            // tick task below.
            let s = Arc::clone(&substrate);
            let mut entry2 = entry.clone();
            entry2.id = AgentId::new();
            entry2.name = "starvation-test-2".to_string();
            let pending = tokio::spawn(async move { s.save_agent_async(&entry2).await });

            // Tick task: sleeps for a fraction of the blocker hold,
            // then stamps `tick_at`. In the correct case this stamp
            // lands during the hold (tick_at < released_at). In the
            // broken case the runtime is parked, the sleep can't tick
            // until the worker is free, and `tick_at` lands after
            // `released_at`.
            let tick_at_for_task = Arc::clone(&tick_at);
            let ticker = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                *tick_at_for_task.lock().unwrap() = Some(Instant::now());
            });

            ticker.await.unwrap();
            pending.await.unwrap().unwrap();
            blocker.join().unwrap();

            let tick = tick_at.lock().unwrap().expect("tick task ran");
            let released = released_at.lock().unwrap().expect("blocker ran");
            assert!(
                tick < released,
                "runtime parked on the connection mutex (#3378): \
                 tick task ({tick:?}) fired after blocker released \
                 the lock ({released:?}), meaning the worker was \
                 stuck in std::sync::Mutex::lock instead of being \
                 driven by spawn_blocking"
            );
        });
    }
}
