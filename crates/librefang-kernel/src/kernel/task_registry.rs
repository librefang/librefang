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
    /// - `Ok(false)` if the task was found and removed but no live
    ///   receiver was attached — the kernel still consumed the entry
    ///   (delete-on-delivery) because step 3 will surface the result via
    ///   the next-turn wake path; for step 2 the event is effectively
    ///   dropped if the session is idle.
    /// - `Ok(false)` if the task id was not in the registry. Idempotent:
    ///   a second call for the same id (e.g. retry-after-error in the
    ///   workflow supervisor) hits this branch and no-ops.
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
        // there is no receiver attached and the inject returns Ok(false)
        // / no live target — step 3 adds the next-turn wake path.
        let injected = self
            .inject_task_completion_signal(entry.agent_id, entry.session_id, event)
            .await?;
        info!(
            task_id = %task_id,
            agent_id = %entry.agent_id,
            session_id = %entry.session_id,
            delivered = injected,
            "Async task completion injected"
        );
        Ok(injected)
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
