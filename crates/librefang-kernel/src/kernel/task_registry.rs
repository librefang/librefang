//! Async task tracker registry (#4983).
//!
//! The kernel maintains a `HashMap<TaskId, PendingTask>` so async
//! operations spawned by an agent (workflow runs today, agent delegations
//! later) can deliver their terminal result back into the originating
//! agent session as a synthetic system message — without the agent having
//! to poll, and without bricking the conversation loop while the
//! operation runs. See `librefang_types::task` for the data shapes.
//!
//! ## Design
//!
//! - **Delete on delivery** — the registry entry is removed as soon as
//!   the `TaskCompletionEvent` is built and the injection attempt has
//!   completed. There is no retention window and no replay; the session
//!   history is the durable record.
//! - **Timeout ownership is agent-side** — the kernel does not impose a
//!   global default and does not garbage-collect on its own. The agent
//!   that registered the task is responsible for cancelling it; the
//!   `[async_tasks]` config block (`default_timeout_secs`,
//!   `notify_on_timeout`) configures the spawn-side behaviour.
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
/// is stable across the two sites by convention — covered by
/// `kernel_and_runtime_renderers_produce_identical_bytes` in
/// `crates/librefang-api/tests/async_task_tracker_runtime_test.rs`
/// and `workflow_timeout_text_format_is_stable` in
/// `crates/librefang-kernel/src/kernel/handles/workflow_runner.rs`.
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
    ///
    /// **Idempotency-of-registration (#5033 review).** A `kind` that is
    /// `TaskKind::Workflow { run_id }` or `TaskKind::Delegation { agent_id,
    /// prompt_hash }` is deduped against the existing registry contents:
    /// if a matching live entry is present (same `run_id` for workflows,
    /// same `(agent_id, prompt_hash)` pair for delegations) we return the
    /// existing handle instead of minting a fresh `TaskId`. This complements
    /// the existing idempotency-of-completion contract — without it,
    /// `workflow_start` returning the same `run_id` (engine-dependent
    /// behaviour) would silently orphan one of the two registry entries
    /// at completion time. The scan is O(n) over a small in-memory map,
    /// kept cheap by the lock scope already required for insertion.
    ///
    /// **Cross-caller dedupe semantics (#5033 re-review).** The match key
    /// is purely `kind`-structural and intentionally ignores the caller
    /// `(agent_id, session_id)`. The two registration paths reach this
    /// design point from different directions:
    ///
    /// - **Workflow**: `run_id` is minted 1:1 by `WorkflowEngine::create_run`
    ///   so two distinct sessions cannot legitimately race on the same
    ///   id. The single production caller (`start_workflow_async_tracked`
    ///   in `kernel/handles/workflow_runner.rs`) calls `create_run`
    ///   immediately before this registration, so the only way a second
    ///   caller could match an existing workflow entry is if it
    ///   re-registers against the same engine run — in which case sharing
    ///   the handle is the correct behaviour, not a bug.
    /// - **Delegation**: `prompt_hash` is opaque to the kernel (see the
    ///   docstring at `librefang_types::task::TaskKind::Delegation`) and
    ///   chosen by the caller. Two callers sending the same
    ///   `(target_agent, prompt_hash)` will share a single registry
    ///   handle and the completion event will be delivered to **only the
    ///   first caller's `(agent_id, session_id)`** (whichever attached
    ///   the registry entry). This is the documented contract: dedupe is
    ///   cross-session-idempotent across the `prompt_hash` namespace.
    ///   Callers that need per-session isolation must salt their
    ///   `prompt_hash` (e.g. include the calling session id in the hash
    ///   input) — the kernel does not silently distinguish them.
    ///
    /// Picking `kind`-only over `(kind, caller)` was deliberate: salting
    /// is cheap at the caller side, and the alternative (silently minting
    /// two entries for one underlying delegation) leaks a phantom pending
    /// task in the registry when the upstream operation is idempotent on
    /// its side too. The cross-session share is pinned by the test
    /// `register_dedupe_is_cross_session_for_delegation_kind` in
    /// `crates/librefang-kernel/tests/async_task_tracker_test.rs`.
    pub fn register_async_task(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        kind: TaskKind,
    ) -> TaskHandle {
        let mut guard = self.events.async_tasks.lock();

        // Dedupe by `kind`. The match is structural — same run_id for a
        // workflow, same (agent, prompt_hash) for a delegation — and
        // deliberately ignores the caller `(agent_id, session_id)`. See
        // the docstring on `register_async_task` for the cross-session
        // share contract. Callers that legitimately want two distinct
        // registrations against the same underlying operation should
        // pass a distinguishing field (different prompt_hash for
        // delegations); workflow registrations are 1:1 with engine run
        // ids by construction.
        let existing = guard.values().find_map(|pending| {
            let matches = match (&pending.handle.kind, &kind) {
                (TaskKind::Workflow { run_id: a }, TaskKind::Workflow { run_id: b }) => a == b,
                (
                    TaskKind::Delegation {
                        agent_id: a_agent,
                        prompt_hash: a_hash,
                    },
                    TaskKind::Delegation {
                        agent_id: b_agent,
                        prompt_hash: b_hash,
                    },
                ) => a_agent == b_agent && a_hash == b_hash,
                _ => false,
            };
            if matches {
                Some(pending.handle.clone())
            } else {
                None
            }
        });

        if let Some(handle) = existing {
            debug!(
                task_id = %handle.id,
                agent_id = %agent_id,
                session_id = %session_id,
                "Async task already registered for this kind — returning existing handle"
            );
            return handle;
        }

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
        guard.insert(handle.id, entry);
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
    ///   consumed the entry (delete-on-delivery contract).
    /// - `Ok(false)` if the task id was not in the registry. Idempotent:
    ///   a second call for the same id (e.g. retry-after-error in the
    ///   workflow supervisor) hits this branch and no-ops.
    ///
    /// **Delivery paths (#4983).**
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
        // path below. Channel-full (Backpressure) also falls through
        // rather than bubbling: the completion event still needs to
        // surface, and the wake-idle path spawns a fresh turn that
        // does not depend on the live injection channel having room.
        let injected = match self
            .inject_task_completion_signal(entry.agent_id, entry.session_id, event.clone())
            .await
        {
            Ok(b) => b,
            Err(crate::error::KernelError::Backpressure(reason)) => {
                warn!(
                    task_id = %task_id,
                    agent_id = %entry.agent_id,
                    session_id = %entry.session_id,
                    reason = %reason,
                    "TaskCompleted mid-turn channel full — falling back to wake-idle"
                );
                false
            }
            Err(e) => return Err(e),
        };

        if injected {
            info!(
                task_id = %task_id,
                agent_id = %entry.agent_id,
                session_id = %entry.session_id,
                "Async task completion injected (mid-turn path)"
            );
            return Ok(true);
        }

        // Wake-idle path (#4983). Spawn a fresh turn so the
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
    ///
    /// **Concurrency (#5033 review).** The spawned turn passes through
    /// the same two semaphores that the trigger dispatcher acquires
    /// before `send_message_full`:
    ///   1. Global `Lane::Trigger` semaphore (`queue.concurrency.trigger_lane`).
    ///   2. Per-agent semaphore (`max_concurrent_invocations` →
    ///      `queue.concurrency.default_per_agent`).
    ///
    /// The third dimension (`session_msg_locks` inside
    /// `send_message_full`) is acquired by the inner call as it is for
    /// every other code path. Without these two acquisitions a batch
    /// agent that fires N `workflow_start` calls would have N wake-idle
    /// turns spawn in parallel on completion, silently bypassing
    /// operator-set fan-out caps.
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

        // Resolve both semaphores synchronously on the caller's task so
        // they survive the spawn. `agent_concurrency_for` clones the
        // shared `Arc<Semaphore>` (created lazily once per agent), and
        // `semaphore_for_lane` returns the kernel-wide trigger lane's
        // owned handle.
        let trigger_sem = kernel_arc
            .workflows
            .command_queue
            .semaphore_for_lane(librefang_runtime::command_lane::Lane::Trigger);
        let agent_sem = kernel_arc.agent_concurrency_for(agent_id);

        tokio::spawn(async move {
            // (1) Global trigger-lane permit. Held across the entire
            //     spawned turn so a wake-idle storm cannot starve
            //     trigger-dispatch and agent_send traffic.
            let _lane_permit = match trigger_sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    tracing::debug!(
                        agent_id = %agent_id,
                        session_id = %session_id,
                        "Async task wake-idle: Lane::Trigger semaphore closed (shutdown)"
                    );
                    return;
                }
            };
            // (2) Per-agent permit. Same resolver the trigger dispatcher
            //     uses, so `max_concurrent_invocations` applies to the
            //     wake-idle dimension uniformly with the other dispatchers.
            let _agent_permit = match agent_sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    tracing::debug!(
                        agent_id = %agent_id,
                        session_id = %session_id,
                        "Async task wake-idle: per-agent semaphore closed (shutdown)"
                    );
                    return;
                }
            };

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
                "TaskCompleted: no live injection receiver — caller will fall back to wake-idle"
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

    /// Boot-recovery hook (#5033 review). For each run id in `recovered`
    /// (returned by `WorkflowEngine::recover_stale_running_runs`), drain
    /// any matching `TaskKind::Workflow { run_id }` entry from the
    /// async-task registry and synthesize a `TaskStatus::Failed(
    /// "interrupted by daemon restart")` completion event into the
    /// originating session's injection channel.
    ///
    /// Synchronous: only touches `events.async_tasks` (a `parking_lot::Mutex`)
    /// and `events.injection_senders` (a `DashMap`). Does **not** call
    /// the async wake-idle path: at boot the kernel's `self_handle` is
    /// not yet wrapped in an `Arc`, so a wake-idle spawn would no-op
    /// regardless. Channel-full / closed-receiver / no-receiver paths
    /// log and drop — the agent that registered the task will not see
    /// an event in those cases, but the registry entry is still
    /// consumed (delete-on-delivery contract).
    ///
    /// Empty `recovered` is a zero-cost no-op. Empty registry (the
    /// common cold-boot case where async_tasks is in-memory and was
    /// just constructed) walks no entries.
    ///
    /// `#[doc(hidden)] pub` rather than `pub(crate)` so the
    /// out-of-crate integration test
    /// `recovery_synthesizes_failed_event_for_matching_pending_workflow`
    /// in `crates/librefang-kernel/tests/async_task_tracker_test.rs`
    /// can pin the contract end-to-end. Production callers should
    /// reach this through the boot hook only.
    #[doc(hidden)]
    pub fn synthesize_task_failures_for_recovered_runs(
        &self,
        recovered: &[librefang_types::task::WorkflowRunId],
    ) {
        use librefang_types::task::{TaskStatus, WorkflowRunId};

        if recovered.is_empty() {
            return;
        }

        // Pull the matching `(TaskId, PendingTask)` pairs out under the
        // registry lock and drop the guard before touching the
        // injection senders. The lock scope is the same delete-on-delivery
        // shape as `complete_async_task` — atomically remove, then send.
        let recovered_set: std::collections::HashSet<WorkflowRunId> =
            recovered.iter().copied().collect();
        let drained: Vec<PendingTask> = {
            let mut guard = self.events.async_tasks.lock();
            let matching_ids: Vec<TaskId> = guard
                .iter()
                .filter_map(|(id, pending)| match &pending.handle.kind {
                    librefang_types::task::TaskKind::Workflow { run_id }
                        if recovered_set.contains(run_id) =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
                .collect();
            matching_ids
                .into_iter()
                .filter_map(|id| guard.remove(&id))
                .collect()
        };

        if drained.is_empty() {
            return;
        }

        let now = Utc::now();
        for entry in drained {
            let event = TaskCompletionEvent {
                handle: entry.handle.clone(),
                status: TaskStatus::Failed(
                    "workflow run interrupted by daemon restart".to_string(),
                ),
                completed_at: now,
            };
            let signal = AgentLoopSignal::TaskCompleted { event };
            let target = self
                .events
                .injection_senders
                .get(&(entry.agent_id, entry.session_id))
                .map(|e| (*e.key(), e.value().clone()));

            match target {
                Some((_, tx)) => match tx.try_send(signal) {
                    Ok(()) => {
                        info!(
                            task_id = %entry.handle.id,
                            agent_id = %entry.agent_id,
                            session_id = %entry.session_id,
                            "Recovery: synthesized Failed completion for restart-interrupted workflow"
                        );
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            task_id = %entry.handle.id,
                            agent_id = %entry.agent_id,
                            session_id = %entry.session_id,
                            "Recovery: injection channel full — restart-Failed event dropped (registry entry already consumed)"
                        );
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        warn!(
                            task_id = %entry.handle.id,
                            agent_id = %entry.agent_id,
                            session_id = %entry.session_id,
                            "Recovery: injection channel closed — restart-Failed event dropped (registry entry already consumed)"
                        );
                    }
                },
                None => {
                    warn!(
                        task_id = %entry.handle.id,
                        agent_id = %entry.agent_id,
                        session_id = %entry.session_id,
                        "Recovery: no live injection receiver — restart-Failed event dropped (registry entry already consumed). Agent must reconcile from session history."
                    );
                }
            }
        }
    }

    /// Test helper — number of currently-registered async tasks.
    ///
    /// Exposed as `pub` (not `pub(crate)`) only so integration tests in
    /// `crates/librefang-kernel/tests/` (which are compiled as an
    /// out-of-crate binary) can read the registry size without going
    /// through the `KernelApi` trait. Marked `#[doc(hidden)]` so it
    /// stays off the public docs surface; new production callers should
    /// go through `KernelApi::pending_async_task_count`.
    #[doc(hidden)]
    pub fn pending_async_task_count(&self) -> usize {
        self.events.async_tasks.lock().len()
    }

    /// Test helper — peek at a pending task by id without removing it.
    /// Returns `None` if the id is not registered.
    ///
    /// Same visibility rationale as
    /// [`Self::pending_async_task_count`]: `#[doc(hidden)] pub` for
    /// out-of-crate integration tests, hidden from the rustdoc surface.
    #[doc(hidden)]
    pub fn lookup_async_task(&self, task_id: TaskId) -> Option<TaskHandle> {
        self.events
            .async_tasks
            .lock()
            .get(&task_id)
            .map(|entry| entry.handle.clone())
    }
}
