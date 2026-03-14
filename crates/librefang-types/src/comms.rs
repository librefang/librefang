//! Shared wire types for the Agent Communication UI.
//!
//! These types are used by both the REST API and the TUI to represent
//! agent topology graphs, inter-agent communication events, and
//! request payloads for sending messages / posting tasks.

use serde::{Deserialize, Serialize};

/// A node in the agent topology graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoNode {
    /// Agent ID.
    pub id: String,
    /// Human-readable agent name.
    pub name: String,
    /// Current lifecycle state (e.g. "Running", "Suspended").
    pub state: String,
    /// Model name the agent is using.
    pub model: String,
}

/// An edge in the agent topology graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoEdge {
    /// Source agent ID.
    pub from: String,
    /// Target agent ID.
    pub to: String,
    /// Relationship kind.
    pub kind: EdgeKind,
}

/// The kind of relationship between two agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Parent spawned child.
    ParentChild,
    /// Peer-to-peer message exchange.
    Peer,
}

/// The full agent topology: nodes + edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topology {
    pub nodes: Vec<TopoNode>,
    pub edges: Vec<TopoEdge>,
}

/// A communication event between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommsEvent {
    /// Unique event ID.
    pub id: String,
    /// ISO-8601 timestamp.
    pub timestamp: String,
    /// Event kind.
    pub kind: CommsEventKind,
    /// Source agent ID.
    pub source_id: String,
    /// Source agent name.
    pub source_name: String,
    /// Target agent ID (empty for lifecycle events without a target).
    pub target_id: String,
    /// Target agent name.
    pub target_name: String,
    /// Human-readable detail text.
    pub detail: String,
}

/// The kind of inter-agent communication event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommsEventKind {
    /// Agent-to-agent message.
    AgentMessage,
    /// A new agent was spawned.
    AgentSpawned,
    /// An agent was terminated.
    AgentTerminated,
    /// A task was posted to the queue.
    TaskPosted,
    /// A task was claimed by an agent.
    TaskClaimed,
    /// A task was completed.
    TaskCompleted,
}

/// An attachment in a comms send request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// URL of the attachment (image, file, etc.).
    pub url: String,
    /// MIME type hint (e.g., "image/png", "application/pdf").
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Display filename for the attachment.
    #[serde(default)]
    pub filename: Option<String>,
}

/// Request body for POST /api/comms/send.
#[derive(Debug, Clone, Deserialize)]
pub struct CommsSendRequest {
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub message: String,
    /// Optional thread/topic ID for cross-channel threading.
    /// Platform-specific: Slack thread_ts, Telegram message_thread_id, Discord thread ID, etc.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Optional file/media attachments.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// Request body for POST /api/comms/task.
#[derive(Debug, Clone, Deserialize)]
pub struct CommsTaskRequest {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub assigned_to: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comms_event_kind_roundtrip() {
        let kind = CommsEventKind::AgentMessage;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"agent_message\"");
        let parsed: CommsEventKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, kind);
    }

    #[test]
    fn edge_kind_roundtrip() {
        let kind = EdgeKind::ParentChild;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"parent_child\"");
        let parsed: EdgeKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, kind);
    }

    #[test]
    fn topology_serialization() {
        let topo = Topology {
            nodes: vec![TopoNode {
                id: "a1".into(),
                name: "agent-1".into(),
                state: "Running".into(),
                model: "gpt-4".into(),
            }],
            edges: vec![TopoEdge {
                from: "a1".into(),
                to: "a2".into(),
                kind: EdgeKind::Peer,
            }],
        };
        let json = serde_json::to_string(&topo).unwrap();
        assert!(json.contains("\"agent-1\""));
        assert!(json.contains("\"peer\""));
    }

    #[test]
    fn comms_send_request_deser() {
        let json = r#"{"from_agent_id":"a","to_agent_id":"b","message":"hello"}"#;
        let req: CommsSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.from_agent_id, "a");
        assert_eq!(req.message, "hello");
        assert!(req.thread_id.is_none());
        assert!(req.attachments.is_empty());
    }

    #[test]
    fn comms_send_request_with_thread_id() {
        let json =
            r#"{"from_agent_id":"a","to_agent_id":"b","message":"reply","thread_id":"t123"}"#;
        let req: CommsSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.thread_id.as_deref(), Some("t123"));
    }

    #[test]
    fn comms_send_request_with_attachments() {
        let json = r#"{"from_agent_id":"a","to_agent_id":"b","message":"see attached","attachments":[{"url":"https://example.com/file.png","mime_type":"image/png","filename":"file.png"}]}"#;
        let req: CommsSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.attachments.len(), 1);
        assert_eq!(req.attachments[0].url, "https://example.com/file.png");
        assert_eq!(req.attachments[0].mime_type.as_deref(), Some("image/png"));
        assert_eq!(req.attachments[0].filename.as_deref(), Some("file.png"));
    }

    #[test]
    fn comms_task_request_deser() {
        let json = r#"{"title":"t","description":"d"}"#;
        let req: CommsTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, "t");
        assert!(req.assigned_to.is_none());
    }

    #[test]
    fn comms_task_request_with_assign() {
        let json = r#"{"title":"t","description":"d","assigned_to":"agent-x"}"#;
        let req: CommsTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.assigned_to.as_deref(), Some("agent-x"));
    }
}
