//! MemorySubstrate: unified implementation of the `Memory` trait.
//!
//! Composes the structured store, semantic store, knowledge store,
//! session store, and consolidation engine behind a single async API.

use crate::chunker;
use crate::consolidation::ConsolidationEngine;
use crate::knowledge::KnowledgeStore;
use crate::migration::run_migrations;
use crate::semantic::SemanticStore;
use crate::session::{Session, SessionStore};
use crate::structured::StructuredStore;
use crate::usage::UsageStore;

use async_trait::async_trait;
use librefang_types::agent::{AgentEntry, AgentId, SessionId};
use librefang_types::config::ChunkConfig;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    ConsolidationReport, Entity, ExportFormat, GraphMatch, GraphPattern, ImportReport, Memory,
    MemoryFilter, MemoryFragment, MemoryId, MemorySource, Relation,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The unified memory substrate. Implements the `Memory` trait by delegating
/// to specialized stores backed by a shared SQLite connection.
pub struct MemorySubstrate {
    conn: Arc<Mutex<Connection>>,
    structured: StructuredStore,
    semantic: SemanticStore,
    knowledge: KnowledgeStore,
    sessions: SessionStore,
    consolidation: ConsolidationEngine,
    usage: UsageStore,
    chunk_config: ChunkConfig,
}

impl MemorySubstrate {
    /// Open or create a memory substrate at the given database path.
    pub fn open(db_path: &Path, decay_rate: f32) -> LibreFangResult<Self> {
        Self::open_with_chunking(db_path, decay_rate, ChunkConfig::default())
    }

    /// Open or create a memory substrate with explicit chunking configuration.
    pub fn open_with_chunking(
        db_path: &Path,
        decay_rate: f32,
        chunk_config: ChunkConfig,
    ) -> LibreFangResult<Self> {
        let conn = Connection::open(db_path).map_err(|e| LibreFangError::Memory(e.to_string()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let shared = Arc::new(Mutex::new(conn));

        Ok(Self {
            conn: Arc::clone(&shared),
            structured: StructuredStore::new(Arc::clone(&shared)),
            semantic: SemanticStore::new(Arc::clone(&shared)),
            knowledge: KnowledgeStore::new(Arc::clone(&shared)),
            sessions: SessionStore::new(Arc::clone(&shared)),
            usage: UsageStore::new(Arc::clone(&shared)),
            consolidation: ConsolidationEngine::new(shared, decay_rate),
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
        let conn =
            Connection::open_in_memory().map_err(|e| LibreFangError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let shared = Arc::new(Mutex::new(conn));

        Ok(Self {
            conn: Arc::clone(&shared),
            structured: StructuredStore::new(Arc::clone(&shared)),
            semantic: SemanticStore::new(Arc::clone(&shared)),
            knowledge: KnowledgeStore::new(Arc::clone(&shared)),
            sessions: SessionStore::new(Arc::clone(&shared)),
            usage: UsageStore::new(Arc::clone(&shared)),
            consolidation: ConsolidationEngine::new(shared, decay_rate),
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

    /// Attach an external vector store backend to the semantic store.
    ///
    /// When set, [`SemanticStore::recall_with_embedding`] will delegate vector
    /// similarity search to this backend instead of doing in-process cosine
    /// similarity over SQLite BLOBs.
    pub fn set_vector_store(&mut self, store: Arc<dyn librefang_types::memory::VectorStore>) {
        self.semantic.set_vector_store(store);
    }

    /// Get the shared database connection (for constructing stores from outside).
    pub fn usage_conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
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
        crate::decay::run_decay(&self.conn, config)
    }

    /// Save an agent entry to persistent storage.
    pub fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        self.structured.save_agent(entry)
    }

    /// Load an agent entry from persistent storage.
    pub fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        self.structured.load_agent(agent_id)
    }

    /// Remove an agent from persistent storage and cascade-delete sessions.
    pub fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        // Delete associated sessions first
        let _ = self.sessions.delete_agent_sessions(agent_id);
        self.structured.remove_agent(agent_id)
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

    /// Delete a session by ID.
    pub fn delete_session(&self, session_id: SessionId) -> LibreFangResult<()> {
        self.sessions.delete_session(session_id)
    }

    /// Delete all sessions belonging to an agent.
    pub fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()> {
        self.sessions.delete_agent_sessions(agent_id)
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

    /// Full-text search across session content using FTS5.
    pub fn search_sessions(
        &self,
        query: &str,
        agent_id: Option<&AgentId>,
    ) -> LibreFangResult<Vec<crate::session::SessionSearchResult>> {
        self.sessions.search_sessions(query, agent_id)
    }

    /// Load canonical session context for cross-channel memory.
    ///
    /// Returns the compacted summary (if any) and recent messages from the
    /// agent's persistent canonical session.
    pub fn canonical_context(
        &self,
        agent_id: AgentId,
        window_size: Option<usize>,
    ) -> LibreFangResult<(Option<String>, Vec<librefang_types::message::Message>)> {
        self.sessions.canonical_context(agent_id, window_size)
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
    ) -> LibreFangResult<()> {
        self.sessions
            .append_canonical(agent_id, messages, compaction_threshold)?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Paired devices persistence
    // -----------------------------------------------------------------

    /// Load all paired devices from the database.
    pub fn load_paired_devices(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT device_id, display_name, platform, paired_at, last_seen, push_token FROM paired_devices"
        ).map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "device_id": row.get::<_, String>(0)?,
                    "display_name": row.get::<_, String>(1)?,
                    "platform": row.get::<_, String>(2)?,
                    "paired_at": row.get::<_, String>(3)?,
                    "last_seen": row.get::<_, String>(4)?,
                    "push_token": row.get::<_, Option<String>>(5)?,
                }))
            })
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        let mut devices = Vec::new();
        for row in rows {
            devices.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
        }
        Ok(devices)
    }

    /// Save a paired device to the database (insert or replace).
    pub fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
    ) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices (device_id, display_name, platform, paired_at, last_seen, push_token) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![device_id, display_name, platform, paired_at, last_seen, push_token],
        ).map_err(|e| LibreFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Remove a paired device from the database.
    pub fn remove_paired_device(&self, device_id: &str) -> LibreFangResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
        conn.execute(
            "DELETE FROM paired_devices WHERE device_id = ?1",
            rusqlite::params![device_id],
        )
        .map_err(|e| LibreFangError::Memory(e.to_string()))?;
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
        let conn = Arc::clone(&self.conn);
        let title = title.to_string();
        let description = description.to_string();
        let assigned_to = assigned_to.unwrap_or("").to_string();
        let created_by = created_by.unwrap_or("").to_string();

        tokio::task::spawn_blocking(move || {
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let db = conn.lock().map_err(|e| LibreFangError::Internal(e.to_string()))?;
            db.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, priority, created_at, title, description, assigned_to, created_by)
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![id, &created_by, &title, b"", now, title, description, assigned_to, created_by],
            )
            .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            Ok(id)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Claim the next pending task (optionally for a specific assignee). Returns task JSON or None.
    pub async fn task_claim(&self, agent_id: &str) -> LibreFangResult<Option<serde_json::Value>> {
        let conn = Arc::clone(&self.conn);
        let agent_id = agent_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.lock().map_err(|e| LibreFangError::Internal(e.to_string()))?;
            // Find first pending task assigned to this agent, or any unassigned pending task
            let mut stmt = db.prepare(
                "SELECT id, title, description, assigned_to, created_by, created_at
                 FROM task_queue
                 WHERE status = 'pending' AND (assigned_to = ?1 OR assigned_to = '')
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1"
            ).map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let result = stmt.query_row(rusqlite::params![agent_id], |row| {
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
                Ok((id, title, description, assigned, created_by, created_at)) => {
                    // Update status to in_progress
                    db.execute(
                        "UPDATE task_queue SET status = 'in_progress', assigned_to = ?2 WHERE id = ?1",
                        rusqlite::params![id, agent_id],
                    ).map_err(|e| LibreFangError::Memory(e.to_string()))?;

                    Ok(Some(serde_json::json!({
                        "id": id,
                        "title": title,
                        "description": description,
                        "status": "in_progress",
                        "assigned_to": if assigned.is_empty() { &agent_id } else { &assigned },
                        "created_by": created_by,
                        "created_at": created_at,
                    })))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(LibreFangError::Memory(e.to_string())),
            }
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Mark a task as completed with a result string.
    pub async fn task_complete(&self, task_id: &str, result: &str) -> LibreFangResult<()> {
        let conn = Arc::clone(&self.conn);
        let task_id = task_id.to_string();
        let result = result.to_string();

        tokio::task::spawn_blocking(move || {
            let now = chrono::Utc::now().to_rfc3339();
            let db = conn.lock().map_err(|e| LibreFangError::Internal(e.to_string()))?;
            let rows = db.execute(
                "UPDATE task_queue SET status = 'completed', result = ?2, completed_at = ?3 WHERE id = ?1",
                rusqlite::params![task_id, result, now],
            ).map_err(|e| LibreFangError::Memory(e.to_string()))?;
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
        let conn = Arc::clone(&self.conn);
        let task_id = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn
                .lock()
                .map_err(|e| LibreFangError::Internal(e.to_string()))?;
            let rows = db
                .execute(
                    "DELETE FROM task_queue WHERE id = ?1",
                    rusqlite::params![task_id],
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// Retry a failed or completed task by resetting it to pending.
    /// Only resets tasks with status 'completed' or 'failed' — in_progress
    /// tasks are excluded to prevent duplicate execution.
    pub async fn task_retry(&self, task_id: &str) -> LibreFangResult<bool> {
        let conn = Arc::clone(&self.conn);
        let task_id = task_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn
                .lock()
                .map_err(|e| LibreFangError::Internal(e.to_string()))?;
            let rows = db
                .execute(
                    "UPDATE task_queue \
                     SET status = 'pending', result = NULL, completed_at = NULL \
                     WHERE id = ?1 AND status IN ('completed', 'failed')",
                    rusqlite::params![task_id],
                )
                .map_err(|e| LibreFangError::Memory(e.to_string()))?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    /// List tasks, optionally filtered by status.
    pub async fn task_list(&self, status: Option<&str>) -> LibreFangResult<Vec<serde_json::Value>> {
        let conn = Arc::clone(&self.conn);
        let status = status.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let db = conn.lock().map_err(|e| LibreFangError::Internal(e.to_string()))?;
            let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &status {
                Some(s) => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result FROM task_queue WHERE status = ?1 ORDER BY created_at DESC",
                    vec![Box::new(s.clone())],
                ),
                None => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result FROM task_queue ORDER BY created_at DESC",
                    vec![],
                ),
            };

            let mut stmt = db.prepare(sql).map_err(|e| LibreFangError::Memory(e.to_string()))?;
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
                }))
            }).map_err(|e| LibreFangError::Memory(e.to_string()))?;

            let mut tasks = Vec::new();
            for row in rows {
                tasks.push(row.map_err(|e| LibreFangError::Memory(e.to_string()))?);
            }
            Ok(tasks)
        })
        .await
        .map_err(|e| LibreFangError::Internal(e.to_string()))?
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

        // Claim the task
        let claimed = substrate.task_claim("auditor").await.unwrap();
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
        let claimed = substrate.task_claim("nobody").await.unwrap();
        assert!(claimed.is_none());
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
}
