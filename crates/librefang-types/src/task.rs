//! Async task tracker types for non-blocking workflow / delegation results.
//!
//! Refs librefang/librefang#4983.
//!
//! ## Why this module exists
//!
//! Today, an agent that calls `workflow_run` blocks its conversation loop
//! for the full duration of the workflow. If the workflow takes minutes,
//! the agent is unresponsive to any other inbound message and a timeout
//! at the tool layer surfaces as a dead-end with no `run_id`. The
//! `workflow_start` (async) variant returns a `run_id` immediately but
//! has no mechanism to deliver the eventual result back into the agent's
//! session — by the next user turn the agent has moved on.
//!
//! `TaskHandle` is the typed handle an agent receives synchronously when
//! it spawns a long-running operation. The kernel registers the handle
//! and, when the operation completes, injects a `TaskCompletionEvent`
//! into the agent's session as a synthetic message. The agent processes
//! the result on its next turn — no polling, no orchestrator.
//!
//! ## Scope of this module
//!
//! Types only. Cross-crate data shapes for the async task tracker:
//! `TaskId`, `TaskKind`, `TaskHandle`, `TaskStatus`,
//! `TaskCompletionEvent`. Behaviour lives in the kernel (pending-task
//! registry, completion injection) and the runtime (agent-loop consumer).
//!
//! ## Design decisions
//!
//! - **Cleanup semantics** — a registered task is removed from the
//!   kernel registry the moment its `TaskCompletionEvent` is delivered
//!   into the originating session. There is no retention window and no
//!   replay; the session history is the durable record. Step 2 wires
//!   this up.
//!
//! - **Timeout ownership** — timeouts are agent-side. The spawning
//!   agent passes a deadline when it registers the task; the kernel
//!   does not impose a global default. This keeps the policy decision
//!   ("how long is too long for THIS operation?") with the caller that
//!   actually knows the answer.
//!
//! - **Error shape** — `TaskStatus::Failed(String)` is conservative on
//!   purpose. A richer typed error variant can land later as an
//!   additive enum variant without breaking on-disk or wire formats
//!   (serde will continue to deserialise the `String` form, and new
//!   variants will deserialise into their own arm).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::AgentId;

// ---------------------------------------------------------------------------
// TaskId
// ---------------------------------------------------------------------------

/// Unique identifier for a registered async task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub Uuid);

impl TaskId {
    /// Generate a new random TaskId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

// ---------------------------------------------------------------------------
// WorkflowRunId
// ---------------------------------------------------------------------------

/// Unique identifier for a running workflow instance.
///
/// Mirrors the shape of `librefang_kernel::workflow::WorkflowRunId`. Lives
/// here because `librefang-types` sits at the bottom of the crate DAG and
/// cannot import the kernel; the kernel re-exports this definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowRunId(pub Uuid);

impl WorkflowRunId {
    /// Generate a new random `WorkflowRunId`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for WorkflowRunId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

// ---------------------------------------------------------------------------
// TaskKind
// ---------------------------------------------------------------------------

/// Discriminator + payload describing what an async task is tracking.
///
/// Serialised with an explicit `kind` tag so additive variants in steps
/// 2 / 3 (`ExternalWebhook`, `LongRunningTool`, …) do not break wire
/// compatibility with already-registered handles.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskKind {
    /// A workflow invocation. Carries the `run_id` so the kernel can
    /// correlate completion events from the workflow engine back to
    /// the originating handle.
    Workflow {
        /// The workflow engine's run identifier.
        run_id: WorkflowRunId,
    },
    /// An agent-to-agent delegation (`agent_send`). Carries the target
    /// agent and a stable hash of the prompt so duplicate delegations
    /// can be deduplicated by callers without storing the full prompt.
    Delegation {
        /// The agent the work was delegated to.
        agent_id: AgentId,
        /// Deterministic hash of the prompt that was delegated. The
        /// hashing algorithm is the caller's choice; this field is
        /// opaque to the kernel.
        prompt_hash: String,
    },
}

// ---------------------------------------------------------------------------
// TaskHandle
// ---------------------------------------------------------------------------

/// Typed handle returned synchronously to an agent that spawns an async
/// task. The agent can hold this across turns; the kernel will deliver a
/// matching `TaskCompletionEvent` when the underlying operation finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHandle {
    /// Kernel-assigned identifier for this registration.
    pub id: TaskId,
    /// What the task is tracking (workflow run, delegation, …).
    pub kind: TaskKind,
    /// When the task was registered with the kernel.
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

/// Terminal or in-flight state of a registered task.
///
/// `Failed(String)` is intentionally a free-form message in this first
/// cut; a richer typed-error variant will be added later as an additive
/// enum arm so existing serialized handles keep round-tripping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Registered, not yet picked up by an executor.
    Pending,
    /// Executor has started work.
    Running,
    /// Finished successfully. Payload is the executor's result (workflow
    /// output, delegation reply body, …) as a `serde_json::Value` so we
    /// don't lock the type of result into this crate.
    Completed(serde_json::Value),
    /// Finished with a failure. Message is human-readable; structured
    /// error data will land in a later additive variant.
    Failed(String),
    /// Cancelled before completion (operator, agent, or supervisor).
    Cancelled,
}

// ---------------------------------------------------------------------------
// TaskCompletionEvent
// ---------------------------------------------------------------------------

/// Wire payload the kernel injects into an agent's session when a
/// registered task reaches a terminal state.
///
/// `handle` lets the agent correlate the event with whichever handle it
/// stashed away when the task was spawned; `status` carries the terminal
/// result; `completed_at` records when the kernel observed completion
/// (not when the underlying operation actually finished — the executor's
/// own timestamp lives inside `status` for `Completed` payloads if the
/// executor chose to include one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCompletionEvent {
    /// The handle that was returned when the task was spawned.
    pub handle: TaskHandle,
    /// Terminal status. Only `Completed`, `Failed`, or `Cancelled`
    /// variants are emitted in completion events; `Pending` and
    /// `Running` are intermediate states observable via separate query
    /// APIs on the kernel registry.
    pub status: TaskStatus,
    /// When the kernel observed completion.
    pub completed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixed_started_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-14T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixed_completed_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-14T12:05:30Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_workflow_handle() -> TaskHandle {
        TaskHandle {
            id: TaskId(Uuid::nil()),
            kind: TaskKind::Workflow {
                run_id: WorkflowRunId(Uuid::nil()),
            },
            started_at: fixed_started_at(),
        }
    }

    fn sample_delegation_handle() -> TaskHandle {
        TaskHandle {
            id: TaskId(Uuid::nil()),
            kind: TaskKind::Delegation {
                agent_id: AgentId(Uuid::nil()),
                prompt_hash: "sha256:abcd".to_string(),
            },
            started_at: fixed_started_at(),
        }
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let cases = vec![
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Completed(json!({"output": "ok", "items": [1, 2, 3]})),
            TaskStatus::Failed("upstream timeout after 30s".to_string()),
            TaskStatus::Cancelled,
        ];
        for status in cases {
            let json = serde_json::to_string(&status).expect("serialize");
            let back: TaskStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, back, "status did not round-trip: {json}");
        }
    }

    #[test]
    fn task_kind_serde_roundtrip() {
        let workflow = TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::nil()),
        };
        let workflow_back: TaskKind =
            serde_json::from_str(&serde_json::to_string(&workflow).unwrap()).unwrap();
        assert_eq!(workflow, workflow_back);

        let delegation = TaskKind::Delegation {
            agent_id: AgentId(Uuid::nil()),
            prompt_hash: "sha256:deadbeef".to_string(),
        };
        let delegation_back: TaskKind =
            serde_json::from_str(&serde_json::to_string(&delegation).unwrap()).unwrap();
        assert_eq!(delegation, delegation_back);
    }

    #[test]
    fn task_completion_event_full_roundtrip() {
        let event = TaskCompletionEvent {
            handle: sample_workflow_handle(),
            status: TaskStatus::Completed(json!({"artifact": "report.md"})),
            completed_at: fixed_completed_at(),
        };
        let wire = serde_json::to_string(&event).expect("serialize event");
        let back: TaskCompletionEvent = serde_json::from_str(&wire).expect("deserialize event");
        assert_eq!(event, back);
    }

    #[test]
    fn task_status_failed_preserves_message() {
        let status = TaskStatus::Failed("workflow run aborted: provider 429".to_string());
        let wire = serde_json::to_string(&status).unwrap();
        let back: TaskStatus = serde_json::from_str(&wire).unwrap();
        match back {
            TaskStatus::Failed(msg) => {
                assert_eq!(msg, "workflow run aborted: provider 429");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn task_completion_event_delegation_roundtrip() {
        let event = TaskCompletionEvent {
            handle: sample_delegation_handle(),
            status: TaskStatus::Cancelled,
            completed_at: fixed_completed_at(),
        };
        let wire = serde_json::to_string(&event).expect("serialize delegation event");
        let back: TaskCompletionEvent =
            serde_json::from_str(&wire).expect("deserialize delegation event");
        assert_eq!(event, back);
        assert!(matches!(back.status, TaskStatus::Cancelled));
        match back.handle.kind {
            TaskKind::Delegation { prompt_hash, .. } => {
                assert_eq!(prompt_hash, "sha256:abcd");
            }
            other => panic!("expected Delegation kind, got {other:?}"),
        }
    }

    #[test]
    fn task_id_display_and_parse_roundtrip() {
        let id = TaskId::new();
        let rendered = id.to_string();
        let parsed: TaskId = rendered.parse().expect("parse back");
        assert_eq!(id, parsed);
    }
}
