//! A2A (Agent-to-Agent) Protocol — cross-framework agent interoperability.
//!
//! Google's A2A protocol enables cross-framework agent interoperability via
//! **Agent Cards** (JSON capability manifests) and **Task-based coordination**.
//!
//! This module provides:
//! - `AgentCard` — describes an agent's capabilities to external systems
//! - `A2aTask` — unit of work exchanged between agents
//! - `build_agent_card` — expose LibreFang agents via A2A
//! - `A2aClient` — discover and interact with external A2A agents

use librefang_types::agent::AgentManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// A2A Agent Card
// ---------------------------------------------------------------------------

/// A2A Agent Card — describes an agent's capabilities to external systems.
///
/// Served at `/.well-known/agent.json` per the A2A specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Agent display name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Agent endpoint URL.
    pub url: String,
    /// Protocol version.
    pub version: String,
    /// Agent capabilities.
    pub capabilities: AgentCapabilities,
    /// Skills this agent can perform (A2A skill descriptors, not LibreFang skills).
    pub skills: Vec<AgentSkill>,
    /// Supported input content types.
    #[serde(default)]
    pub default_input_modes: Vec<String>,
    /// Supported output content types.
    #[serde(default)]
    pub default_output_modes: Vec<String>,
}

/// A2A agent capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether this agent supports streaming responses.
    pub streaming: bool,
    /// Whether this agent supports push notifications.
    pub push_notifications: bool,
    /// Whether task status history is available.
    pub state_transition_history: bool,
}

/// A2A skill descriptor (not an LibreFang skill — describes a capability).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Unique skill identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description of what this skill does.
    pub description: String,
    /// Tags for discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Example prompts that trigger this skill.
    #[serde(default)]
    pub examples: Vec<String>,
}

// ---------------------------------------------------------------------------
// A2A Task
// ---------------------------------------------------------------------------

/// A2A Task — unit of work exchanged between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aTask {
    /// Unique task identifier.
    pub id: String,
    /// Optional session identifier for conversation continuity.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Current task status (accepts both string and object forms).
    pub status: A2aTaskStatusWrapper,
    /// Messages exchanged during the task.
    #[serde(default)]
    pub messages: Vec<A2aMessage>,
    /// Artifacts produced by the task.
    #[serde(default)]
    pub artifacts: Vec<A2aArtifact>,
    /// The local agent ID that this task was dispatched to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The external A2A caller's agent ID (from `X-A2A-Agent-ID` header or
    /// registered A2A agent entry). Stored for audit / ACL purposes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_a2a_agent_id: Option<String>,
}

/// A2A task status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum A2aTaskStatus {
    /// Task has been received but not started.
    Submitted,
    /// Task is being processed.
    Working,
    /// Agent needs more input from the caller.
    InputRequired,
    /// Task completed successfully.
    Completed,
    /// Task was cancelled.
    Cancelled,
    /// Task failed.
    Failed,
}

/// Wrapper that accepts either a bare status string (`"completed"`)
/// or the object form (`{"state": "completed", "message": null}`)
/// used by some A2A implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum A2aTaskStatusWrapper {
    /// Object form: `{"state": "completed", "message": ...}`.
    Object {
        state: A2aTaskStatus,
        #[serde(default)]
        message: Option<serde_json::Value>,
    },
    /// Bare enum form: `"completed"`.
    Enum(A2aTaskStatus),
}

impl A2aTaskStatusWrapper {
    /// Extract the underlying `A2aTaskStatus` regardless of encoding form.
    pub fn state(&self) -> &A2aTaskStatus {
        match self {
            Self::Object { state, .. } => state,
            Self::Enum(s) => s,
        }
    }
}

impl From<A2aTaskStatus> for A2aTaskStatusWrapper {
    fn from(status: A2aTaskStatus) -> Self {
        Self::Enum(status)
    }
}

impl PartialEq<A2aTaskStatus> for A2aTaskStatusWrapper {
    fn eq(&self, other: &A2aTaskStatus) -> bool {
        self.state() == other
    }
}

/// A2A message in a task conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aMessage {
    /// Message role ("user" or "agent").
    pub role: String,
    /// Message content parts.
    pub parts: Vec<A2aPart>,
}

/// A2A message content part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum A2aPart {
    /// Text content.
    Text { text: String },
    /// File content (base64-encoded).
    File {
        name: String,
        mime_type: String,
        data: String,
    },
    /// Structured data.
    Data {
        mime_type: String,
        data: serde_json::Value,
    },
}

/// A2A artifact produced by a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aArtifact {
    /// Artifact name (optional per spec).
    #[serde(default)]
    pub name: Option<String>,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Arbitrary metadata.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Artifact index in the sequence.
    #[serde(default)]
    pub index: Option<u32>,
    /// Whether this is the last chunk of a streamed artifact.
    #[serde(default)]
    pub last_chunk: Option<bool>,
    /// Artifact content parts.
    pub parts: Vec<A2aPart>,
}

// ---------------------------------------------------------------------------
// A2A Task Store — tracks task lifecycle
// ---------------------------------------------------------------------------

/// Entry in the task store that pairs a task with its last-updated timestamp.
#[derive(Debug, Clone)]
struct TrackedTask {
    task: A2aTask,
    updated_at: Instant,
}

/// Default TTL for tasks: 24 hours.
const DEFAULT_TASK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Tasks older than 7 days are pruned from the SQLite backing store on startup.
const DB_TASK_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;

/// In-memory store for tracking A2A task lifecycle.
///
/// Tasks are created by `tasks/send`, polled by `tasks/get`, and cancelled
/// by `tasks/cancel`. The store is bounded to prevent memory exhaustion.
///
/// Eviction policy (applied lazily on insert):
/// 1. **TTL**: any task whose `updated_at` exceeds `task_ttl` is removed,
///    regardless of state. This prevents Working/InputRequired tasks from
///    accumulating indefinitely.
/// 2. **Capacity**: if still at capacity after TTL sweep, evict the oldest
///    terminal-state task first, then fall back to the oldest task overall.
#[derive(Debug)]
pub struct A2aTaskStore {
    tasks: Mutex<HashMap<String, TrackedTask>>,
    /// Maximum number of tasks to retain.
    max_tasks: usize,
    /// Time-to-live for any task regardless of state.
    task_ttl: Duration,
    /// Optional SQLite connection for persistent storage.
    db: Option<Arc<Mutex<rusqlite::Connection>>>,
}

impl A2aTaskStore {
    /// Create a new task store with a capacity limit.
    pub fn new(max_tasks: usize) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_tasks,
            task_ttl: DEFAULT_TASK_TTL,
            db: None,
        }
    }

    /// Create a new task store with a custom TTL.
    pub fn with_ttl(max_tasks: usize, task_ttl: Duration) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_tasks,
            task_ttl,
            db: None,
        }
    }

    /// Open or create a SQLite-backed task store at `db_path`.
    ///
    /// The caller is responsible for providing a path in the daemon's data
    /// directory. The store creates the schema on first open, prunes rows
    /// older than 7 days, and loads surviving tasks into memory so pollers
    /// do not receive 404 after a restart.
    ///
    /// Persistence is **best-effort**: every mutation that returns to the
    /// caller has been written to memory but the SQLite write only logs a
    /// `warn!` on failure. A full disk or read-only volume therefore
    /// degrades silently to in-memory-only behaviour for the affected
    /// rows; tasks that have not yet been re-saved are lost on restart.
    pub fn with_persistence(max_tasks: usize, db_path: &Path) -> Self {
        match rusqlite::Connection::open(db_path) {
            Ok(conn) => {
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                    .unwrap_or_else(|e| warn!("a2a_tasks: failed to set PRAGMA: {e}"));

                // The first iteration of this schema split messages into
                // `input` / `output` columns, dropping artifacts and
                // session_id entirely and losing chronological ordering on
                // mixed user/agent conversations. v2 stores the full
                // `messages` / `artifacts` arrays as JSON and adds
                // `session_id`. Drop any v1 table — the schema only
                // shipped in unmerged PR revisions, so there is no
                // production data to migrate.
                if let Err(e) = conn.execute_batch(
                    "DROP TABLE IF EXISTS a2a_tasks;
                     CREATE TABLE IF NOT EXISTS a2a_tasks_v2 (
                        id                  TEXT PRIMARY KEY,
                        status              TEXT NOT NULL,
                        session_id          TEXT,
                        messages_json       TEXT NOT NULL,
                        artifacts_json      TEXT NOT NULL,
                        agent_id            TEXT,
                        caller_a2a_agent_id TEXT,
                        created_at          INTEGER NOT NULL,
                        updated_at          INTEGER NOT NULL
                     );",
                ) {
                    warn!("a2a_tasks: failed to create schema: {e}");
                }

                let db = Arc::new(Mutex::new(conn));
                let mut store = Self {
                    tasks: Mutex::new(HashMap::new()),
                    max_tasks,
                    task_ttl: DEFAULT_TASK_TTL,
                    db: Some(Arc::clone(&db)),
                };

                // Prune rows older than 7 days, then load survivors into memory.
                store.db_prune_old_tasks();
                store.db_load_into_memory();
                store
            }
            Err(e) => {
                warn!(
                    "a2a_tasks: failed to open persistence DB at {}: {e} — falling back to in-memory only",
                    db_path.display()
                );
                Self::new(max_tasks)
            }
        }
    }

    // ------------------------------------------------------------------
    // SQLite helpers
    // ------------------------------------------------------------------

    /// Delete tasks older than `DB_TASK_RETENTION_SECS` from the DB.
    fn db_prune_old_tasks(&self) {
        let db_arc = match &self.db {
            Some(d) => d,
            None => return,
        };
        let conn = db_arc.lock().unwrap_or_else(|e| e.into_inner());
        let cutoff = now_unix_secs() - DB_TASK_RETENTION_SECS;
        if let Err(e) = conn.execute(
            "DELETE FROM a2a_tasks_v2 WHERE created_at < ?1",
            rusqlite::params![cutoff],
        ) {
            warn!("a2a_tasks: failed to prune old tasks: {e}");
        } else {
            debug!("a2a_tasks: pruned rows created before unix={cutoff}");
        }
    }

    /// Load the most recent `max_tasks` rows from the DB into the in-memory
    /// map.
    ///
    /// Bound matters: a long-running daemon accumulates rows up to the
    /// 7-day retention window, which can be far more than `max_tasks`.
    /// Loading every row would (a) blow `max_tasks` on boot and force an
    /// immediate cascade of capacity evictions and (b) hold the full
    /// row set in memory during decode.  Older rows still live in the
    /// DB and stay reachable through `get()`'s SQLite fallback when a
    /// poller asks for them by ID.
    fn db_load_into_memory(&mut self) {
        let db_arc = match &self.db {
            Some(d) => d,
            None => return,
        };
        let conn = db_arc.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = match conn.prepare(
            "SELECT id, status, session_id, messages_json, artifacts_json, agent_id, caller_a2a_agent_id
             FROM a2a_tasks_v2
             ORDER BY updated_at DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("a2a_tasks: failed to prepare load query: {e}");
                return;
            }
        };

        let rows: Vec<_> = match stmt.query_map(rusqlite::params![self.max_tasks as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,         // id
                row.get::<_, String>(1)?,         // status (JSON)
                row.get::<_, Option<String>>(2)?, // session_id
                row.get::<_, String>(3)?,         // messages_json
                row.get::<_, String>(4)?,         // artifacts_json
                row.get::<_, Option<String>>(5)?, // agent_id
                row.get::<_, Option<String>>(6)?, // caller_a2a_agent_id
            ))
        }) {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                warn!("a2a_tasks: failed to load tasks from DB: {e}");
                return;
            }
        };
        drop(stmt);

        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let mut loaded = 0usize;
        for (
            id,
            status_json,
            session_id,
            messages_json,
            artifacts_json,
            agent_id,
            caller_a2a_agent_id,
        ) in rows
        {
            let Some(task) = decode_task_row(
                id.clone(),
                &status_json,
                session_id,
                &messages_json,
                &artifacts_json,
                agent_id,
                caller_a2a_agent_id,
            ) else {
                continue;
            };
            tasks.insert(
                id,
                TrackedTask {
                    task,
                    updated_at: Instant::now(),
                },
            );
            loaded += 1;
        }
        info!("a2a_tasks: loaded {loaded} task(s) from persistence DB");
    }

    /// Upsert a task into the SQLite backing store. Persists the full
    /// `messages` and `artifacts` arrays plus `session_id` so a round-trip
    /// through the DB returns an identical task.
    fn db_upsert(&self, task: &A2aTask) {
        let db_arc = match &self.db {
            Some(d) => d,
            None => return,
        };
        let conn = db_arc.lock().unwrap_or_else(|e| e.into_inner());
        let status_json = serde_json::to_string(&task.status).unwrap_or_default();
        let messages_json =
            serde_json::to_string(&task.messages).unwrap_or_else(|_| "[]".to_string());
        let artifacts_json =
            serde_json::to_string(&task.artifacts).unwrap_or_else(|_| "[]".to_string());
        let now = now_unix_secs();
        if let Err(e) = conn.execute(
            "INSERT INTO a2a_tasks_v2 (id, status, session_id, messages_json, artifacts_json, agent_id, caller_a2a_agent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
               status              = excluded.status,
               session_id          = excluded.session_id,
               messages_json       = excluded.messages_json,
               artifacts_json      = excluded.artifacts_json,
               agent_id            = excluded.agent_id,
               caller_a2a_agent_id = excluded.caller_a2a_agent_id,
               updated_at          = excluded.updated_at",
            rusqlite::params![
                task.id,
                status_json,
                task.session_id,
                messages_json,
                artifacts_json,
                task.agent_id,
                task.caller_a2a_agent_id,
                now,
            ],
        ) {
            warn!("a2a_tasks: failed to upsert task {}: {e}", task.id);
        }
    }

    // ------------------------------------------------------------------
    // In-memory helpers
    // ------------------------------------------------------------------

    /// Remove all tasks whose `updated_at` is older than the TTL.
    fn evict_expired(tasks: &mut HashMap<String, TrackedTask>, ttl: Duration) {
        let now = Instant::now();
        tasks.retain(|_, tracked| now.duration_since(tracked.updated_at) < ttl);
    }

    /// Insert a task. Expired tasks are swept first, then capacity eviction
    /// is applied if needed.
    pub fn insert(&self, task: A2aTask) {
        // Persist first so we never miss a task even if eviction removes it.
        self.db_upsert(&task);

        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());

        // Lazy TTL sweep — remove all expired tasks regardless of state.
        Self::evict_expired(&mut tasks, self.task_ttl);

        // Capacity eviction: prefer terminal-state tasks, fall back to oldest.
        if tasks.len() >= self.max_tasks {
            let is_terminal = |t: &TrackedTask| {
                matches!(
                    t.task.status.state(),
                    A2aTaskStatus::Completed | A2aTaskStatus::Failed | A2aTaskStatus::Cancelled
                )
            };

            // Try to evict the oldest terminal task first.
            let evict_key = tasks
                .iter()
                .filter(|(_, t)| is_terminal(t))
                .min_by_key(|(_, t)| t.updated_at)
                .map(|(k, _)| k.clone())
                .or_else(|| {
                    // No terminal tasks — evict the oldest task overall.
                    tasks
                        .iter()
                        .min_by_key(|(_, t)| t.updated_at)
                        .map(|(k, _)| k.clone())
                });

            if let Some(key) = evict_key {
                tasks.remove(&key);
            }
        }

        let now = Instant::now();
        tasks.insert(
            task.id.clone(),
            TrackedTask {
                task,
                updated_at: now,
            },
        );
    }

    /// Get a task by ID.
    ///
    /// Falls back to the SQLite backing store when the task has been evicted
    /// from the in-memory map (e.g. after a restart that loaded older tasks
    /// beyond the in-memory cap).
    pub fn get(&self, task_id: &str) -> Option<A2aTask> {
        // Fast path: in-memory hit.
        if let Some(tracked) = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(task_id)
        {
            return Some(tracked.task.clone());
        }

        // Slow path: query the DB for tasks that may have been evicted from memory.
        let db_arc = self.db.as_ref()?;
        let conn = db_arc.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn.query_row(
            "SELECT id, status, session_id, messages_json, artifacts_json, agent_id, caller_a2a_agent_id FROM a2a_tasks_v2 WHERE id = ?1",
            rusqlite::params![task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        );

        match result {
            Ok((
                id,
                status_json,
                session_id,
                messages_json,
                artifacts_json,
                agent_id,
                caller_a2a_agent_id,
            )) => decode_task_row(
                id,
                &status_json,
                session_id,
                &messages_json,
                &artifacts_json,
                agent_id,
                caller_a2a_agent_id,
            ),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => {
                warn!("a2a_tasks: DB lookup for {task_id} failed: {e}");
                None
            }
        }
    }

    /// Update a task's status and optionally add messages/artifacts.
    pub fn update_status(&self, task_id: &str, status: A2aTaskStatus) -> bool {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tracked) = tasks.get_mut(task_id) {
            tracked.task.status = status.into();
            tracked.updated_at = Instant::now();
            self.db_upsert(&tracked.task);
            true
        } else {
            false
        }
    }

    /// Complete a task with a response message and optional artifacts.
    pub fn complete(&self, task_id: &str, response: A2aMessage, artifacts: Vec<A2aArtifact>) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tracked) = tasks.get_mut(task_id) {
            tracked.task.messages.push(response);
            tracked.task.artifacts.extend(artifacts);
            tracked.task.status = A2aTaskStatus::Completed.into();
            tracked.updated_at = Instant::now();
            self.db_upsert(&tracked.task);
        }
    }

    /// Fail a task with an error message.
    pub fn fail(&self, task_id: &str, error_message: A2aMessage) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tracked) = tasks.get_mut(task_id) {
            tracked.task.messages.push(error_message);
            tracked.task.status = A2aTaskStatus::Failed.into();
            tracked.updated_at = Instant::now();
            self.db_upsert(&tracked.task);
        }
    }

    /// Cancel a task.
    pub fn cancel(&self, task_id: &str) -> bool {
        self.update_status(task_id, A2aTaskStatus::Cancelled)
    }

    /// Count of tracked tasks.
    pub fn len(&self) -> usize {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Return the current UNIX timestamp in seconds.
fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Decode one row from the `a2a_tasks_v2` schema into an `A2aTask`.
///
/// Returns `None` and logs a warning if the row's `status` column doesn't
/// deserialize to a recognised state (lets the load path skip a bad row
/// rather than aborting the whole load). `messages_json` and
/// `artifacts_json` failures fall back to empty arrays — they were
/// authored by us, so we tolerate `null` / older shapes by yielding `[]`
/// rather than dropping the task entirely.
#[allow(clippy::too_many_arguments)]
fn decode_task_row(
    id: String,
    status_json: &str,
    session_id: Option<String>,
    messages_json: &str,
    artifacts_json: &str,
    agent_id: Option<String>,
    caller_a2a_agent_id: Option<String>,
) -> Option<A2aTask> {
    let status: A2aTaskStatusWrapper = match serde_json::from_str(status_json) {
        Ok(s) => s,
        Err(_) => {
            match serde_json::from_value(serde_json::Value::String(status_json.to_string())) {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                    "a2a_tasks: skipping task {id} with unrecognised status {status_json:?}: {e}"
                );
                    return None;
                }
            }
        }
    };

    let messages: Vec<A2aMessage> = serde_json::from_str(messages_json).unwrap_or_default();
    let artifacts: Vec<A2aArtifact> = serde_json::from_str(artifacts_json).unwrap_or_default();

    Some(A2aTask {
        id,
        session_id,
        status,
        messages,
        artifacts,
        agent_id,
        caller_a2a_agent_id,
    })
}

impl Default for A2aTaskStore {
    fn default() -> Self {
        Self::new(1000)
    }
}

// ---------------------------------------------------------------------------
// A2A Discovery — auto-discover external agents at boot
// ---------------------------------------------------------------------------

/// Discover all configured external A2A agents and return their cards.
///
/// Called during kernel boot to populate the list of known external agents.
pub async fn discover_external_agents(
    agents: &[librefang_types::config::ExternalAgent],
) -> Vec<(String, AgentCard)> {
    let client = A2aClient::new();
    let mut discovered = Vec::new();

    for agent in agents {
        match client.discover(&agent.url).await {
            Ok(card) => {
                info!(
                    name = %agent.name,
                    url = %agent.url,
                    skills = card.skills.len(),
                    "Discovered external A2A agent"
                );
                // Bug #3786: store by URL so the trust gate in `/api/a2a/send`
                // can match on the same key callers pass. Statically-seeded
                // agents are operator-authored (config.toml) and therefore
                // legitimately trusted at boot.
                discovered.push((agent.url.clone(), card));
            }
            Err(e) => {
                warn!(
                    name = %agent.name,
                    url = %agent.url,
                    error = %e,
                    "Failed to discover external A2A agent"
                );
            }
        }
    }

    if !discovered.is_empty() {
        info!("A2A: discovered {} external agent(s)", discovered.len());
    }

    discovered
}

// ---------------------------------------------------------------------------
// A2A Server — expose LibreFang agents via A2A
// ---------------------------------------------------------------------------

/// Build an A2A Agent Card from an LibreFang agent manifest.
pub fn build_agent_card(manifest: &AgentManifest, base_url: &str) -> AgentCard {
    let tools: Vec<String> = manifest.capabilities.tools.clone();

    // Convert tool names to A2A skill descriptors
    let skills: Vec<AgentSkill> = tools
        .iter()
        .map(|tool| AgentSkill {
            id: tool.clone(),
            name: tool.replace('_', " "),
            description: format!("Can use the {tool} tool"),
            tags: vec!["tool".to_string()],
            examples: vec![],
        })
        .collect();

    AgentCard {
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        url: format!("{base_url}/a2a"),
        version: librefang_types::VERSION.to_string(),
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        skills,
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
    }
}

// ---------------------------------------------------------------------------
// A2A Client — discover and interact with external A2A agents
// ---------------------------------------------------------------------------

/// Hard cap on Agent Card responses (Bug #3785).
///
/// Agent Cards are tiny JSON manifests; 256 KiB is generous and still
/// bounds the daemon's memory exposure to a single hostile remote.
const MAX_AGENT_CARD_BYTES: usize = 256 * 1024;

/// Hard cap on A2A task RPC responses (Bug #3785).
///
/// Task payloads can carry larger transcripts/artifacts than agent cards,
/// but must still be bounded so a hostile remote cannot OOM the daemon
/// via `tasks/send` or `tasks/get`.
const MAX_A2A_TASK_BYTES: usize = 1024 * 1024;

/// Read at most `max_bytes` from a `reqwest::Response`, rejecting upfront
/// when `Content-Length` already exceeds the cap and aborting mid-stream
/// once the running total trips it (Bug #3785).
///
/// `reqwest::Response::json()` / `bytes()` read the entire body into memory
/// with no limit, so a hostile A2A peer can blow up the daemon by streaming
/// gigabytes within the 30 s timeout. We stream chunks instead and bail.
async fn read_capped_body(
    mut response: reqwest::Response,
    max_bytes: usize,
    what: &str,
) -> Result<Vec<u8>, String> {
    if let Some(len) = response.content_length() {
        if (len as usize) > max_bytes {
            return Err(format!(
                "{what} response Content-Length {len} exceeds cap of {max_bytes} bytes"
            ));
        }
    }

    let mut buf = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("{what} body read failed: {e}"))?
    {
        if buf.len() + chunk.len() > max_bytes {
            return Err(format!(
                "{what} response body exceeds cap of {max_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Client for discovering and interacting with external A2A agents.
pub struct A2aClient {
    client: reqwest::Client,
}

impl A2aClient {
    /// Create a new A2A client with an empty SSRF allowlist.
    pub fn new() -> Self {
        Self::new_with_allowlist(Vec::new())
    }

    /// Create a new A2A client; every redirect hop is re-validated via `check_ssrf` (#3782).
    pub fn new_with_allowlist(allowed_hosts: Vec<String>) -> Self {
        let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            // Re-validate every hop; a 302 response can otherwise bypass the entry-point SSRF check.
            let target = attempt.url().as_str().to_owned();
            match crate::web_fetch::check_ssrf(&target, &allowed_hosts) {
                Ok(_) => attempt.follow(),
                Err(reason) => {
                    attempt.error(format!("A2A SSRF blocked redirect to {target}: {reason}"))
                }
            }
        });

        Self {
            client: crate::http_client::proxied_client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(redirect_policy)
                .build()
                .expect("HTTP client build"),
        }
    }

    /// Discover an external agent by fetching its Agent Card.
    pub async fn discover(&self, url: &str) -> Result<AgentCard, String> {
        let agent_json_url = format!("{}/.well-known/agent.json", url.trim_end_matches('/'));

        debug!(url = %agent_json_url, "Discovering A2A agent");

        let response = self
            .client
            .get(&agent_json_url)
            .header(
                "User-Agent",
                format!("LibreFang/{} A2A", librefang_types::VERSION),
            )
            .send()
            .await
            .map_err(|e| format!("A2A discovery failed: {e}"))?;

        if response.status().is_redirection() {
            return Err("A2A request redirect not followed (SSRF prevention)".to_string());
        }
        if !response.status().is_success() {
            return Err(format!("A2A discovery returned {}", response.status()));
        }

        // Bug #3785: cap remote body size — reqwest::Response::json() is unbounded.
        let bytes = read_capped_body(response, MAX_AGENT_CARD_BYTES, "A2A discovery").await?;
        let card: AgentCard =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid Agent Card: {e}"))?;

        info!(agent = %card.name, skills = card.skills.len(), "Discovered A2A agent");
        Ok(card)
    }

    /// Send a task to an external A2A agent.
    pub async fn send_task(
        &self,
        url: &str,
        message: &str,
        session_id: Option<&str>,
    ) -> Result<A2aTask, String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/send",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [{"type": "text", "text": message}]
                },
                "sessionId": session_id,
            }
        });

        let response = self
            .client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("A2A send_task failed: {e}"))?;

        if response.status().is_redirection() {
            return Err("A2A request redirect not followed (SSRF prevention)".to_string());
        }

        // Bug #3785: cap remote body size — reqwest::Response::json() is unbounded.
        let bytes = read_capped_body(response, MAX_A2A_TASK_BYTES, "A2A send_task").await?;
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid A2A response: {e}"))?;

        if let Some(result) = body.get("result") {
            serde_json::from_value(result.clone())
                .map_err(|e| format!("Invalid A2A task response: {e}"))
        } else if let Some(error) = body.get("error") {
            Err(format!("A2A error: {}", error))
        } else {
            Err("Empty A2A response".to_string())
        }
    }

    /// Get the status of a task from an external A2A agent.
    pub async fn get_task(&self, url: &str, task_id: &str) -> Result<A2aTask, String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/get",
            "params": {
                "id": task_id,
            }
        });

        let response = self
            .client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("A2A get_task failed: {e}"))?;

        if response.status().is_redirection() {
            return Err("A2A request redirect not followed (SSRF prevention)".to_string());
        }

        // Bug #3785: cap remote body size — reqwest::Response::json() is unbounded.
        let bytes = read_capped_body(response, MAX_A2A_TASK_BYTES, "A2A get_task").await?;
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid A2A response: {e}"))?;

        if let Some(result) = body.get("result") {
            serde_json::from_value(result.clone()).map_err(|e| format!("Invalid A2A task: {e}"))
        } else {
            Err("Empty A2A response".to_string())
        }
    }
}

impl Default for A2aClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_agent_card_from_manifest() {
        let manifest = AgentManifest {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            ..Default::default()
        };

        let card = build_agent_card(&manifest, "https://example.com");
        assert_eq!(card.name, "test-agent");
        assert_eq!(card.description, "A test agent");
        assert!(card.url.contains("/a2a"));
        assert!(card.capabilities.streaming);
        assert_eq!(card.default_input_modes, vec!["text"]);
    }

    #[test]
    fn test_a2a_task_status_transitions() {
        let task = A2aTask {
            id: "task-1".to_string(),
            session_id: None,
            status: A2aTaskStatus::Submitted.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        assert_eq!(task.status, A2aTaskStatus::Submitted);

        // Simulate progression
        let working = A2aTask {
            status: A2aTaskStatus::Working.into(),
            ..task.clone()
        };
        assert_eq!(working.status, A2aTaskStatus::Working);

        let completed = A2aTask {
            status: A2aTaskStatus::Completed.into(),
            ..task.clone()
        };
        assert_eq!(completed.status, A2aTaskStatus::Completed);

        let cancelled = A2aTask {
            status: A2aTaskStatus::Cancelled.into(),
            ..task.clone()
        };
        assert_eq!(cancelled.status, A2aTaskStatus::Cancelled);

        let failed = A2aTask {
            status: A2aTaskStatus::Failed.into(),
            ..task
        };
        assert_eq!(failed.status, A2aTaskStatus::Failed);
    }

    #[test]
    fn test_a2a_task_status_wrapper_object_form() {
        // Test deserialization of the object form: {"state": "completed", "message": null}
        let json = r#"{"state":"completed","message":null}"#;
        let wrapper: A2aTaskStatusWrapper = serde_json::from_str(json).unwrap();
        assert_eq!(wrapper, A2aTaskStatus::Completed);
        assert_eq!(wrapper.state(), &A2aTaskStatus::Completed);

        // Test with a message payload
        let json_with_msg = r#"{"state":"working","message":{"text":"Processing..."}}"#;
        let wrapper2: A2aTaskStatusWrapper = serde_json::from_str(json_with_msg).unwrap();
        assert_eq!(wrapper2, A2aTaskStatus::Working);

        // Test bare string form
        let json_bare = r#""completed""#;
        let wrapper3: A2aTaskStatusWrapper = serde_json::from_str(json_bare).unwrap();
        assert_eq!(wrapper3, A2aTaskStatus::Completed);
    }

    #[test]
    fn test_a2a_artifact_optional_fields() {
        // name is now optional — artifact with no name should deserialize
        let json = r#"{"parts":[{"type":"text","text":"hello"}]}"#;
        let artifact: A2aArtifact = serde_json::from_str(json).unwrap();
        assert!(artifact.name.is_none());
        assert!(artifact.description.is_none());
        assert!(artifact.metadata.is_none());
        assert!(artifact.index.is_none());
        assert!(artifact.last_chunk.is_none());
        assert_eq!(artifact.parts.len(), 1);

        // Full artifact with all optional fields
        let json_full = r#"{"name":"output.txt","description":"The result","metadata":{"key":"val"},"index":0,"lastChunk":true,"parts":[]}"#;
        let full: A2aArtifact = serde_json::from_str(json_full).unwrap();
        assert_eq!(full.name.as_deref(), Some("output.txt"));
        assert_eq!(full.description.as_deref(), Some("The result"));
        assert_eq!(full.index, Some(0));
        assert_eq!(full.last_chunk, Some(true));
    }

    #[test]
    fn test_a2a_message_serde() {
        let msg = A2aMessage {
            role: "user".to_string(),
            parts: vec![
                A2aPart::Text {
                    text: "Hello".to_string(),
                },
                A2aPart::Data {
                    mime_type: "application/json".to_string(),
                    data: serde_json::json!({"key": "value"}),
                },
            ],
        };

        let json = serde_json::to_string(&msg).unwrap();
        let back: A2aMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.parts.len(), 2);

        match &back.parts[0] {
            A2aPart::Text { text } => assert_eq!(text, "Hello"),
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_task_store_insert_and_get() {
        let store = A2aTaskStore::new(10);
        let task = A2aTask {
            id: "t-1".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);
        assert_eq!(store.len(), 1);

        let got = store.get("t-1").unwrap();
        assert_eq!(got.status, A2aTaskStatus::Working);
    }

    #[test]
    fn test_task_store_complete_and_fail() {
        let store = A2aTaskStore::new(10);
        let task = A2aTask {
            id: "t-2".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);

        store.complete(
            "t-2",
            A2aMessage {
                role: "agent".to_string(),
                parts: vec![A2aPart::Text {
                    text: "Done".to_string(),
                }],
            },
            vec![],
        );

        let completed = store.get("t-2").unwrap();
        assert_eq!(completed.status, A2aTaskStatus::Completed);
        assert_eq!(completed.messages.len(), 1);
    }

    #[test]
    fn test_task_store_cancel() {
        let store = A2aTaskStore::new(10);
        let task = A2aTask {
            id: "t-3".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);
        assert!(store.cancel("t-3"));
        assert_eq!(store.get("t-3").unwrap().status, A2aTaskStatus::Cancelled);
        // Cancel a nonexistent task returns false
        assert!(!store.cancel("t-999"));
    }

    #[test]
    fn test_task_store_eviction() {
        let store = A2aTaskStore::new(2);
        // Insert 2 tasks
        for i in 0..2 {
            let task = A2aTask {
                id: format!("t-{i}"),
                session_id: None,
                status: A2aTaskStatus::Completed.into(),
                messages: vec![],
                artifacts: vec![],
                agent_id: None,
                caller_a2a_agent_id: None,
            };
            store.insert(task);
        }
        assert_eq!(store.len(), 2);

        // Insert a 3rd — one completed task should be evicted
        let task = A2aTask {
            id: "t-2".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);
        // One was evicted, plus the new one
        assert!(store.len() <= 2);
    }

    #[test]
    fn test_task_store_ttl_eviction() {
        // Use a very short TTL so we can test expiration without sleeping.
        let store = A2aTaskStore::with_ttl(100, Duration::from_secs(0));

        // Insert a Working task (previously un-evictable).
        let task = A2aTask {
            id: "stuck-working".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);
        assert_eq!(store.len(), 1);

        // Insert another task — the TTL sweep on insert should evict the
        // expired Working task.
        let task2 = A2aTask {
            id: "new-task".to_string(),
            session_id: None,
            status: A2aTaskStatus::Submitted.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task2);

        // The stuck Working task should have been evicted by TTL.
        assert!(store.get("stuck-working").is_none());
        // Only the newly inserted task should remain (it was inserted after
        // the sweep, so its updated_at is fresh).
        assert!(store.get("new-task").is_some());
    }

    #[test]
    fn test_task_store_capacity_evicts_oldest_when_no_terminal() {
        // All tasks are Working — capacity eviction should still work by
        // evicting the oldest task.
        let store = A2aTaskStore::new(2);
        for i in 0..2 {
            let task = A2aTask {
                id: format!("w-{i}"),
                session_id: None,
                status: A2aTaskStatus::Working.into(),
                messages: vec![],
                artifacts: vec![],
                agent_id: None,
                caller_a2a_agent_id: None,
            };
            store.insert(task);
        }
        assert_eq!(store.len(), 2);

        // Insert a 3rd Working task — should evict the oldest Working task.
        let task = A2aTask {
            id: "w-2".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(task);
        assert!(store.len() <= 2);
        // The newest task should always be present.
        assert!(store.get("w-2").is_some());
    }

    #[test]
    fn test_a2a_config_serde() {
        use librefang_types::config::{A2aConfig, ExternalAgent};

        let config = A2aConfig {
            enabled: true,
            name: "LibreFang Agent OS".to_string(),
            description: "Test description".to_string(),
            listen_path: "/a2a".to_string(),
            external_agents: vec![ExternalAgent {
                name: "other-agent".to_string(),
                url: "https://other.example.com".to_string(),
            }],
        };

        let json = serde_json::to_string(&config).unwrap();
        let back: A2aConfig = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.listen_path, "/a2a");
        assert_eq!(back.external_agents.len(), 1);
        assert_eq!(back.external_agents[0].name, "other-agent");
    }

    /// Round-trip a fully-populated task through the SQLite backing store:
    /// insert, drop the store, reopen on the same DB path, and verify
    /// `get` returns every field we wrote — including `session_id`,
    /// interleaved user/agent messages (in order), and artifacts.
    ///
    /// This is the regression test the original PR was missing — the
    /// schema split messages into `input` / `output` columns and silently
    /// dropped artifacts and `session_id`, all of which would have been
    /// caught here.
    #[test]
    fn test_persistence_round_trip_preserves_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("a2a.db");

        let original = A2aTask {
            id: "round-trip-1".to_string(),
            session_id: Some("session-abc".to_string()),
            status: A2aTaskStatus::Working.into(),
            // Deliberately interleave user / agent / user so the old schema
            // (which split by role) would scramble the order on reload.
            messages: vec![
                A2aMessage {
                    role: "user".to_string(),
                    parts: vec![A2aPart::Text {
                        text: "first user msg".to_string(),
                    }],
                },
                A2aMessage {
                    role: "agent".to_string(),
                    parts: vec![A2aPart::Text {
                        text: "agent response".to_string(),
                    }],
                },
                A2aMessage {
                    role: "user".to_string(),
                    parts: vec![A2aPart::Text {
                        text: "follow-up".to_string(),
                    }],
                },
            ],
            artifacts: vec![A2aArtifact {
                name: Some("result.txt".to_string()),
                description: None,
                metadata: None,
                index: Some(0),
                last_chunk: Some(true),
                parts: vec![A2aPart::Text {
                    text: "final artifact".to_string(),
                }],
            }],
            agent_id: Some("11111111-1111-1111-1111-111111111111".to_string()),
            caller_a2a_agent_id: Some("caller-bot".to_string()),
        };

        // Phase 1 — write through the persistent store, then drop it.
        {
            let store = A2aTaskStore::with_persistence(100, &db_path);
            store.insert(original.clone());
        }

        // Phase 2 — reopen on the same path; load_into_memory should
        // restore the task exactly.
        let store = A2aTaskStore::with_persistence(100, &db_path);
        let reloaded = store
            .get("round-trip-1")
            .expect("task should survive a store restart");

        assert_eq!(reloaded.id, original.id);
        assert_eq!(reloaded.session_id, original.session_id);
        assert_eq!(reloaded.status.state(), original.status.state());
        assert_eq!(
            reloaded.messages.len(),
            original.messages.len(),
            "all messages should round-trip"
        );
        for (loaded, expected) in reloaded.messages.iter().zip(original.messages.iter()) {
            assert_eq!(
                loaded.role, expected.role,
                "message roles should match in order"
            );
        }
        assert_eq!(
            reloaded.artifacts.len(),
            original.artifacts.len(),
            "artifacts should round-trip"
        );
        assert_eq!(reloaded.agent_id, original.agent_id);
        assert_eq!(reloaded.caller_a2a_agent_id, original.caller_a2a_agent_id);
    }

    /// The slow path of `get` (querying the DB directly when the
    /// in-memory map has evicted the task) must produce the same task
    /// shape as the load path. Construct two stores with `max_tasks=1`
    /// to force eviction, then verify the evicted task is still
    /// returned by `get` via the SQLite fallback.
    #[test]
    fn test_persistence_get_falls_back_to_db_after_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("a2a.db");

        let store = A2aTaskStore::with_persistence(1, &db_path);
        let evicted = A2aTask {
            id: "evicted-1".to_string(),
            session_id: Some("s1".to_string()),
            status: A2aTaskStatus::Completed.into(),
            messages: vec![A2aMessage {
                role: "user".to_string(),
                parts: vec![A2aPart::Text {
                    text: "hi".to_string(),
                }],
            }],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        let kept = A2aTask {
            id: "kept-1".to_string(),
            session_id: None,
            status: A2aTaskStatus::Working.into(),
            messages: vec![],
            artifacts: vec![],
            agent_id: None,
            caller_a2a_agent_id: None,
        };
        store.insert(evicted);
        store.insert(kept);

        // The first task was evicted from memory by capacity pressure but
        // the DB still has it — `get` should find it via the slow path.
        let got = store
            .get("evicted-1")
            .expect("evicted task must still be retrievable from the DB");
        assert_eq!(got.id, "evicted-1");
        assert_eq!(got.session_id.as_deref(), Some("s1"));
    }

    /// `with_persistence` must not load more than `max_tasks` rows on boot,
    /// even when the DB has accumulated many more (long-running daemon
    /// inside the 7-day retention window).  The `LIMIT` clause picks the
    /// most recently updated rows; older rows stay reachable via the
    /// `get()` SQLite fallback path.
    #[test]
    fn test_persistence_load_respects_max_tasks_cap() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("a2a.db");

        // First daemon lifetime: insert 10 tasks under a generous cap.
        {
            let store = A2aTaskStore::with_persistence(20, &db_path);
            for i in 0..10 {
                store.insert(A2aTask {
                    id: format!("t-{i:02}"),
                    session_id: None,
                    status: A2aTaskStatus::Completed.into(),
                    messages: vec![],
                    artifacts: vec![],
                    agent_id: None,
                    caller_a2a_agent_id: None,
                });
            }
        }

        // Second lifetime: tighter cap.  The DB still has 10 rows, but the
        // in-memory map must hold at most 3 to honour the new cap.
        let restarted = A2aTaskStore::with_persistence(3, &db_path);
        let in_memory_len = {
            let tasks = restarted.tasks.lock().unwrap();
            tasks.len()
        };
        assert_eq!(
            in_memory_len, 3,
            "boot load must respect max_tasks=3, got {in_memory_len}"
        );

        // Older rows still reachable through the DB fallback path.
        for i in 0..10 {
            assert!(
                restarted.get(&format!("t-{i:02}")).is_some(),
                "task t-{i:02} must remain queryable after restart (DB fallback)"
            );
        }
    }
    /// Regression: 302 redirect to cloud-metadata IP must be blocked even when the originating host is allowlisted (#3782).
    #[tokio::test]
    async fn redirect_to_cloud_metadata_is_blocked_by_ssrf_revalidation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // TCP listener that replies 302 → cloud-metadata IP; allowlisted so the initial connect succeeds.
        let attacker = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let attacker_addr = attacker.local_addr().unwrap();

        let attacker_task = tokio::spawn(async move {
            let (mut stream, _) = attacker.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let response = concat!(
                "HTTP/1.1 302 Found\r\n",
                "Location: http://169.254.169.254/latest/meta-data/iam/security-credentials/\r\n",
                "Content-Length: 0\r\n",
                "Connection: close\r\n",
                "\r\n",
            )
            .as_bytes();
            let _ = stream.write_all(response).await;
            let _ = stream.shutdown().await;
        });

        // 127.0.0.1 allowlisted for test reachability; 169.254.0.0/16 is unconditionally blocked.
        let client = A2aClient::new_with_allowlist(vec!["127.0.0.1".to_string()]);
        let url = format!("http://{}", attacker_addr);
        let result = client.discover(&url).await;

        let _ = attacker_task.await;

        let err = result
            .expect_err("discover() must fail when the peer 302-redirects to a cloud metadata IP");
        assert!(
            err.starts_with("A2A discovery failed:") || err.contains("redirect"),
            "expected an A2A request failure, got: {err}"
        );
    }

    /// Bug #3785: a hostile peer that advertises an oversized Content-Length on
    /// `/.well-known/agent.json` must be rejected upfront, before the daemon
    /// allocates the body. Without the cap, `reqwest::Response::json()` would
    /// happily read multi-GB into memory.
    #[tokio::test]
    async fn discover_rejects_oversized_content_length() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                // Advertise a huge body; we never actually send it. The cap is
                // enforced from Content-Length alone, so the daemon must bail
                // before allocating.
                let oversize = MAX_AGENT_CARD_BYTES + 1;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {oversize}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });

        let client = A2aClient::new_with_allowlist(vec!["127.0.0.1".to_string()]);
        let url = format!("http://{addr}");
        let result = client.discover(&url).await;
        let _ = server.await;

        let err = result.expect_err("discover() must reject oversized Content-Length");
        assert!(
            err.contains("exceeds cap"),
            "expected cap rejection, got: {err}"
        );
    }

    /// Bug #3785: a peer that under-reports Content-Length (or omits it) and
    /// then streams more bytes than the cap allows must still be cut off
    /// mid-stream. Guards against the chunked-transfer evasion of the
    /// upfront Content-Length check.
    #[tokio::test]
    async fn discover_rejects_oversized_streamed_body() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let header = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(header.as_bytes()).await;
                // 64 KiB of garbage per chunk; loop until we definitely
                // exceed MAX_AGENT_CARD_BYTES.
                let payload = vec![b'x'; 65_536];
                let chunk_header = format!("{:x}\r\n", payload.len());
                for _ in 0..((MAX_AGENT_CARD_BYTES / payload.len()) + 4) {
                    if stream.write_all(chunk_header.as_bytes()).await.is_err() {
                        break;
                    }
                    if stream.write_all(&payload).await.is_err() {
                        break;
                    }
                    if stream.write_all(b"\r\n").await.is_err() {
                        break;
                    }
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
                let _ = stream.shutdown().await;
            }
        });

        let client = A2aClient::new_with_allowlist(vec!["127.0.0.1".to_string()]);
        let url = format!("http://{addr}");
        let result = tokio::time::timeout(Duration::from_secs(10), client.discover(&url))
            .await
            .expect("client must terminate, not hang");
        let _ = server.await;

        let err = result.expect_err("discover() must abort once streamed body exceeds cap");
        assert!(
            err.contains("exceeds cap"),
            "expected streaming cap rejection, got: {err}"
        );
    }
}
