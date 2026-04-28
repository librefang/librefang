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
use std::sync::Mutex;
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
}

impl A2aTaskStore {
    /// Create a new task store with a capacity limit.
    pub fn new(max_tasks: usize) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_tasks,
            task_ttl: DEFAULT_TASK_TTL,
        }
    }

    /// Create a new task store with a custom TTL.
    pub fn with_ttl(max_tasks: usize, task_ttl: Duration) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_tasks,
            task_ttl,
        }
    }

    /// Remove all tasks whose `updated_at` is older than the TTL.
    fn evict_expired(tasks: &mut HashMap<String, TrackedTask>, ttl: Duration) {
        let now = Instant::now();
        tasks.retain(|_, tracked| now.duration_since(tracked.updated_at) < ttl);
    }

    /// Insert a task. Expired tasks are swept first, then capacity eviction
    /// is applied if needed.
    pub fn insert(&self, task: A2aTask) {
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
    pub fn get(&self, task_id: &str) -> Option<A2aTask> {
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(task_id)
            .map(|tracked| tracked.task.clone())
    }

    /// Update a task's status and optionally add messages/artifacts.
    pub fn update_status(&self, task_id: &str, status: A2aTaskStatus) -> bool {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tracked) = tasks.get_mut(task_id) {
            tracked.task.status = status.into();
            tracked.updated_at = Instant::now();
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
        }
    }

    /// Fail a task with an error message.
    pub fn fail(&self, task_id: &str, error_message: A2aMessage) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tracked) = tasks.get_mut(task_id) {
            tracked.task.messages.push(error_message);
            tracked.task.status = A2aTaskStatus::Failed.into();
            tracked.updated_at = Instant::now();
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
                discovered.push((agent.name.clone(), card));
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

/// Client for discovering and interacting with external A2A agents.
pub struct A2aClient {
    client: reqwest::Client,
}

impl A2aClient {
    /// Create a new A2A client.
    ///
    /// Redirects are disabled (`Policy::none`) to prevent SSRF bypass via
    /// server-side redirects. The SSRF check in the API layer validates the
    /// *original* URL before the request is made; if the remote server
    /// responds with a 3xx redirect to a private IP the client would
    /// silently follow it, bypassing the check. Refusing all redirects and
    /// returning an error prevents this class of attack. (#3563)
    pub fn new() -> Self {
        Self {
            client: crate::http_client::proxied_client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
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

        if !response.status().is_success() {
            return Err(format!("A2A discovery returned {}", response.status()));
        }

        let card: AgentCard = response
            .json()
            .await
            .map_err(|e| format!("Invalid Agent Card: {e}"))?;

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

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Invalid A2A response: {e}"))?;

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

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Invalid A2A response: {e}"))?;

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
}
