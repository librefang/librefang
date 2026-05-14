//! Async task tracker registry (#4983 step 2).
//!
//! The kernel maintains a `HashMap<TaskId, PendingTask>` so async
//! operations spawned by an agent (workflow runs today, agent delegations
//! later) can deliver their terminal result back into the originating
//! agent session as a synthetic system message — without the agent having
//! to poll, and without bricking the conversation loop while the
//! operation runs. See `librefang_types::task` for the data shapes.
//!
//! ## Design (defaulted by step 1, honoured here)
//!
//! - **Delete on delivery** — the registry entry is removed as soon as
//!   the `TaskCompletionEvent` is built and the injection attempt has
//!   completed. There is no retention window and no replay; the session
//!   history is the durable record.
//! - **Timeout ownership is agent-side** — the kernel does not impose a
//!   global default and does not garbage-collect on its own. The agent
//!   that registered the task is responsible for cancelling it (step 3
//!   adds the `[async_tasks]` config knob).
//! - **Error shape is `TaskStatus::Failed(String)`** — conservative free
//!   form. A richer typed-error variant will land as an additive enum
//!   arm.
//!
//! ## Lifecycle
//!
//! ```text
//!   register_async_task(agent, session, kind) -> TaskHandle
//!         |
//!         | (workflow / delegation runs in background)
//!         v
//!   complete_async_task(task_id, terminal_status)
//!         |
//!         | (looks up agent_id + session_id from the registry,
//!         |  builds TaskCompletionEvent, removes the entry,
//!         |  injects AgentLoopSignal::TaskCompleted into the
//!         |  per-(agent, session) channel via the existing #956 path)
//!         v
//!   Agent loop wakes up (or mid-turn injects) and surfaces the result.
//! ```
//!
//! `complete_async_task` is idempotent: calling it twice for the same
//! `TaskId` is a no-op on the second call (the entry was already
//! removed). This guards against retry races between the workflow engine
//! and any future supervisor that watches for terminal states.

use chrono::Utc;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::task::{TaskCompletionEvent, TaskHandle, TaskId, TaskKind, TaskStatus};
use librefang_types::tool::AgentLoopSignal;
use tracing::{debug, info, warn};

use super::subsystems::events::PendingTask;
use super::LibreFangKernel;

/// Render a `TaskCompletionEvent` as the human-readable system text
/// that the wake-idle path passes to `send_message_full` as the new
/// turn's body. Mirrors `librefang_runtime::agent_loop::format_task_completion_text`
/// so the session history reads consistently regardless of which path
/// surfaced the result.
///
/// Duplicated rather than shared because the runtime crate cannot
/// re-export back into the kernel (the runtime depends on
/// `librefang-kernel-handle`, not on the kernel directly). The format
/// is stable across the two sites by convention — covered by the
/// `wake_idle_text_matches_runtime_format` integration test.
fn format_task_completion_text(event: &TaskCompletionEvent) -> String {
    let kind_str = match &event.handle.kind {
        TaskKind::Workflow { run_id } => format!("workflow (run {run_id})"),
        TaskKind::Delegation { agent_id, .. } => format!("delegation to agent {agent_id}"),
    };
    let status_str = match &event.status {
        TaskStatus::Completed(value) => {
            let rendered = value.to_string();
            let preview = librefang_types::truncate_str(&rendered, 300);
            format!("completed. Output: {preview}")
        }
        TaskStatus::Failed(msg) => {
            let preview = librefang_types::truncate_str(msg, 300);
            format!("failed: {preview}")
        }
        TaskStatus::Cancelled => "cancelled".to_string(),
        TaskStatus::Pending => "pending (unexpected in completion event)".to_string(),
        TaskStatus::Running => "running (unexpected in completion event)".to_string(),
    };
    format!(
        "[System] [ASYNC_RESULT] task {id} ({kind}) {status}",
        id = event.handle.id,
        kind = kind_str,
        status = status_str,
    )
}

impl LibreFangKernel {
    /// Register a new async task in the kernel registry and return the
    /// typed `TaskHandle` the spawning agent should stash to correlate the
    /// eventual `TaskCompletionEvent`.
    ///
    /// The caller is expected to call [`Self::complete_async_task`] when
    /// the underlying operation terminates (Ok, Err, or Cancelled).
    ///
    /// Wrapped under a `parking_lot::Mutex` rather than `DashMap` so the
    /// "look up, remove, then send" sequence in `complete_async_task` can
    /// be expressed atomically without holding a shard lock across the
    /// `try_send` boundary.
    pub fn register_async_task(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        kind: TaskKind,
    ) -> TaskHandle {
        let handle = TaskHandle {
            id: TaskId::new(),
            kind,
            started_at: Utc::now(),
        };
        let entry = PendingTask {
            handle: handle.clone(),
            agent_id,
            session_id,
        };
        self.events
            .async_tasks
            .lock()
            .insert(handle.id, entry.clone());
        debug!(
            task_id = %handle.id,
            agent_id = %agent_id,
            session_id = %session_id,
            "Async task registered"
        );
        handle
    }

    /// Mark an in-flight async task as terminated with `status` and
    /// inject a `TaskCompletionEvent` into the originating session.
    ///
    /// Returns:
    /// - `Ok(true)` if the task was found and the completion event was
    ///   delivered to at least one live agent-loop receiver (mid-turn
    ///   injection succeeded).
    /// - `Ok(false)` if the task was found and removed but neither the
    ///   mid-turn injection channel had a live receiver nor the
    ///   wake-idle path could spawn a new turn. The kernel still
    ///   consumed the entry (delete-on-delivery contract from step 1).
    /// - `Ok(false)` if the task id was not in the registry. Idempotent:
    ///   a second call for the same id (e.g. retry-after-error in the
    ///   workflow supervisor) hits this branch and no-ops.
    ///
    /// **Delivery paths (step 3, #4983).**
    /// 1. **Mid-turn injection** — if the originating session has an
    ///    active agent loop with an injection channel attached, the
    ///    `TaskCompletionEvent` is sent as an `AgentLoopSignal::TaskCompleted`
    ///    through the existing #956 channel. The loop renders it as a
    ///    `[System] [ASYNC_RESULT]` text and processes it on the next
    ///    iteration of the current turn.
    /// 2. **Wake-idle** — if no receiver is attached, the kernel
    ///    spawns a fresh turn via `send_message_full` with the same
    ///    `[System] [ASYNC_RESULT]` text pinned to the originating
    ///    session, so the agent wakes up and acts on the result.
    ///    Spawned in a detached `tokio::task` so completion delivery
    ///    is fire-and-forget — the spawning workflow does not block
    ///    on the agent's response.
    ///
    /// `status` should be one of the terminal variants
    /// (`Completed` / `Failed` / `Cancelled`); the kernel will surface
    /// `Pending` and `Running` defensively but they are not semantically
    /// terminal and indicate caller bugs.
    pub async fn complete_async_task(
        &self,
        task_id: TaskId,
        status: TaskStatus,
    ) -> Result<bool, crate::error::KernelError> {
        // Atomically remove the entry. `parking_lot::Mutex` guards keep
        // the section non-async-friendly; drop the guard before we touch
        // the async injection channel.
        let entry = {
            let mut guard = self.events.async_tasks.lock();
            guard.remove(&task_id)
        };
        let Some(entry) = entry else {
            debug!(
                task_id = %task_id,
                "complete_async_task: id not found (already completed or never registered)"
            );
            return Ok(false);
        };

        match &status {
            TaskStatus::Completed(_) | TaskStatus::Failed(_) | TaskStatus::Cancelled => {}
            TaskStatus::Pending | TaskStatus::Running => {
                warn!(
                    task_id = %task_id,
                    "complete_async_task called with non-terminal status; surfacing anyway"
                );
            }
        }

        let event = TaskCompletionEvent {
            handle: entry.handle.clone(),
            status,
            completed_at: Utc::now(),
        };

        // Inject through the same `(agent, session)` channel that
        // mid-turn message injection (#956) uses. When the loop is idle,
        // there is no receiver attached — fall through to the wake-idle
        // path below.
        let injected = self
            .inject_task_completion_signal(entry.agent_id, entry.session_id, event.clone())
            .await?;

        if injected {
            info!(
                task_id = %task_id,
                agent_id = %entry.agent_id,
                session_id = %entry.session_id,
                "Async task completion injected (mid-turn path)"
            );
            return Ok(true);
        }

        // Wake-idle path (#4983 step 3). Spawn a fresh turn so the
        // agent processes the result without the operator having to
        // poke it manually. Detached so the workflow that called
        // `complete_async_task` returns immediately.
        let woken = self.spawn_wake_idle_turn(entry.agent_id, entry.session_id, &event);
        info!(
            task_id = %task_id,
            agent_id = %entry.agent_id,
            session_id = %entry.session_id,
            mid_turn = false,
            wake_idle_spawned = woken,
            "Async task completion delivered (idle path)"
        );
        Ok(woken)
    }

    /// Step-3 wake-idle path: when no live agent-loop receiver is
    /// attached to the originating session, spawn a fresh turn whose
    /// content is the rendered task-completion text. The agent loop
    /// then processes the result on its next iteration as if a
    /// `[System]` message had arrived through any other channel.
    ///
    /// Returns `true` if a wake-up turn was spawned (no guarantee on
    /// the resulting turn's outcome — that's the agent's
    /// responsibility), `false` if the kernel self-handle has not
    /// been initialised yet (boot-time race; the entry has already
    /// been consumed from the registry per the delete-on-delivery
    /// contract, so the event is dropped).
    fn spawn_wake_idle_turn(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        event: &TaskCompletionEvent,
    ) -> bool {
        let kernel_arc = match self.self_handle.get().and_then(|w| w.upgrade()) {
            Some(arc) => arc,
            None => {
                tracing::warn!(
                    agent_id = %agent_id,
                    session_id = %session_id,
                    "Async task wake-idle: kernel self-handle not yet initialised; dropping completion"
                );
                return false;
            }
        };

        // Render the same text shape the runtime's
        // `format_task_completion_text` produces for the mid-turn path,
        // so the session history reads consistently regardless of how
        // the agent surfaced the result.
        let body = format_task_completion_text(event);

        tokio::spawn(async move {
            let handle = kernel_arc.kernel_handle();
            match kernel_arc
                .send_message_full(
                    agent_id,
                    &body,
                    handle,
                    None,
                    None,
                    None,
                    None,
                    Some(session_id),
                )
                .await
            {
                Ok(_) => {
                    tracing::debug!(
                        agent_id = %agent_id,
                        session_id = %session_id,
                        "Async task wake-idle turn completed"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        session_id = %session_id,
                        "Async task wake-idle turn failed: {e}"
                    );
                }
            }
        });
        true
    }

    /// Wrap a `TaskCompletionEvent` in `AgentLoopSignal::TaskCompleted`
    /// and send it through the per-(agent, session) injection channel.
    ///
    /// Returns the same delivered/no-receiver semantics as
    /// `inject_message_for_session`: `Ok(true)` if at least one live
    /// receiver accepted the signal, `Ok(false)` if no receiver was
    /// attached for this session, `Err(Backpressure)` if every live
    /// receiver was full.
    async fn inject_task_completion_signal(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        event: TaskCompletionEvent,
    ) -> Result<bool, crate::error::KernelError> {
        let signal = AgentLoopSignal::TaskCompleted { event };

        // Grab the matching sender (if any). Step 2 keeps a tight scope —
        // we deliberately do NOT fan out across sibling sessions of the
        // same agent the way `inject_message_for_session` does for
        // `session_id = None`. The completion is addressed at exactly
        // the originating session and nowhere else.
        let target = self
            .events
            .injection_senders
            .get(&(agent_id, session_id))
            .map(|entry| (*entry.key(), entry.value().clone()));

        let Some((key, tx)) = target else {
            debug!(
                agent_id = %agent_id,
                session_id = %session_id,
                "TaskCompleted: no live injection receiver — signal dropped (step 3 adds wake-idle)"
            );
            return Ok(false);
        };

        match tx.try_send(signal) {
            Ok(()) => Ok(true),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    agent_id = %agent_id,
                    session_id = %session_id,
                    "TaskCompleted injection channel full — applying backpressure"
                );
                Err(crate::error::KernelError::Backpressure(format!(
                    "TaskCompleted injection channel for {key:?} is full"
                )))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                // Receiver dropped between lookup and send — clean up
                // the stale sender entry, same as
                // `inject_message_for_session` does.
                self.events.injection_senders.remove(&key);
                debug!(
                    agent_id = %agent_id,
                    session_id = %session_id,
                    "TaskCompleted injection channel closed — sender removed"
                );
                Ok(false)
            }
        }
    }

    /// Test helper — number of currently-registered async tasks.
    #[doc(hidden)]
    pub fn pending_async_task_count(&self) -> usize {
        self.events.async_tasks.lock().len()
    }

    /// Test helper — peek at a pending task by id without removing it.
    /// Returns `None` if the id is not registered.
    #[doc(hidden)]
    pub fn lookup_async_task(&self, task_id: TaskId) -> Option<TaskHandle> {
        self.events
            .async_tasks
            .lock()
            .get(&task_id)
            .map(|entry| entry.handle.clone())
    }
}
