//! Task-local registry of `agent_msg_locks` entries the current task holds.
//!
//! `agent_msg_locks[agent_id]` (a non-reentrant `tokio::sync::Mutex`) is
//! acquired by `send_message_full` for the duration of an agent's turn. The
//! same async task can then re-enter the lock through two tool paths:
//!
//! - `agent_send` with a transitive `A -> B -> A` topology (issue #5125):
//!   the inner `send_message_full(A)` re-acquires `agent_msg_locks[A]` that
//!   the outer turn for A still holds — a silent self-deadlock.
//! - `channel_send` whose channel owner resolves back to the caller
//!   (issue #5126): `SessionWriter::append_to_session` does
//!   `block_in_place(|| lock.blocking_lock())` on the same
//!   `agent_msg_locks[A]` — `block_in_place` does **not** release the held
//!   async mutex, so the worker thread parks forever.
//!
//! Both are *same-task* re-entries. This module records which `AgentId`
//! locks the current task currently holds so the two consumer paths can
//! detect the re-entry and act (reject the cycle for #5125; perform the
//! mirror write without re-locking for #5126) **without** relaxing
//! cross-task mutual exclusion: a *different* task that wants the same
//! agent's lock still blocks on the real `tokio::sync::Mutex`.
//!
//! ## Why a `tokio::task_local!`, not a `std::thread_local!`
//!
//! The tokio multi-thread runtime can migrate a future across worker
//! threads at every `.await` point, so a `thread_local!` would observe the
//! wrong set after a yield. A `task_local!` is bound to the *task*, which
//! is exactly the unit across which the lock is held (the whole agent loop
//! runs inside one task on the non-streaming `send_message_full` path).
//!
//! `tokio::task::block_in_place` runs its closure on the **same task**
//! (it only signals the runtime to migrate *other* tasks off this worker),
//! so the task-local is fully visible inside `append_to_session`'s
//! `block_in_place` — which is the #5126 detection point.

use librefang_types::agent::AgentId;
use std::cell::RefCell;
use std::collections::HashSet;

tokio::task_local! {
    /// Set of `AgentId`s whose `agent_msg_locks` entry the current task
    /// currently holds. Established once at the outermost
    /// `send_message_full` entry via [`scope`]; inner re-entrant calls in
    /// the same task observe and mutate the same cell.
    ///
    /// `HashSet` (not `BTreeSet`): `AgentId` is `Hash + Eq` but not `Ord`,
    /// and this set is used for O(1) membership testing, not for any
    /// LLM-prompt ordering boundary (the #3298 determinism rule does not
    /// apply here). The diagnostic cycle-path snapshot is sorted by the
    /// inner `Uuid` in [`held_snapshot`] for a stable error message.
    static HELD_AGENT_LOCKS: RefCell<HashSet<AgentId>>;
}

/// Run `fut` with the held-locks registry available for the current task.
///
/// Idempotent: if a registry is already established for this task (an
/// outer `send_message_full` frame set it up), the future is awaited
/// directly so the *same* cell is shared with the outer frame — that
/// sharing is what makes same-task re-entry observable. Only the
/// outermost frame allocates the `BTreeSet`.
pub async fn scope<F>(fut: F) -> F::Output
where
    F: std::future::Future,
{
    if HELD_AGENT_LOCKS.try_with(|_| ()).is_ok() {
        // Registry already established by an outer frame on this task.
        fut.await
    } else {
        HELD_AGENT_LOCKS
            .scope(RefCell::new(HashSet::new()), fut)
            .await
    }
}

/// Is `agent_id`'s `agent_msg_locks` entry already held by the current
/// task? Returns `false` when called outside any [`scope`] (no agent turn
/// in flight on this task) — the safe default that preserves all existing
/// behaviour for non-agent-loop callers.
pub fn is_held(agent_id: AgentId) -> bool {
    HELD_AGENT_LOCKS
        .try_with(|set| set.borrow().contains(&agent_id))
        .unwrap_or(false)
}

/// Snapshot the currently-held set for diagnostics — used to render the
/// cycle path in the #5125 rejection message. Sorted by the inner `Uuid`
/// so the error string is stable across runs (the set itself is a
/// `HashSet`). Empty when called outside a [`scope`].
pub fn held_snapshot() -> Vec<AgentId> {
    let mut v: Vec<AgentId> = HELD_AGENT_LOCKS
        .try_with(|set| set.borrow().iter().copied().collect())
        .unwrap_or_default();
    v.sort_by_key(|a| a.0);
    v
}

/// RAII guard: records that the current task holds `agent_msg_locks[agent_id]`
/// for its lifetime, and removes the entry on drop.
///
/// Drop runs on normal return, on early `?`, **and** on panic (Rust unwinds
/// run destructors), so the registry can never leak a stale entry that would
/// spuriously reject a later, legitimate, non-re-entrant acquisition.
///
/// Constructed only *after* the real `tokio::sync::Mutex` guard is acquired,
/// and dropped *before* it (declare the registry guard after the lock guard
/// so drop order is registry-then-lock — reverse of declaration). The
/// registry therefore reflects "this task is inside the locked region",
/// never a window where the entry is registered but the mutex is free.
#[must_use = "dropping the guard immediately would clear the held-lock record before the locked region ends"]
pub struct HeldLockGuard {
    agent_id: AgentId,
    /// `true` when this guard inserted the entry (vs. it was already
    /// present because a *different* code path on the same task registered
    /// it). Only the inserter removes it on drop, so re-entrant frames that
    /// observe an already-held lock do not erase the outer frame's record.
    inserted: bool,
}

impl HeldLockGuard {
    /// Register `agent_id` as held by the current task. No-op-safe when
    /// called outside a [`scope`] (returns a guard whose drop is inert),
    /// so existing non-agent-loop lock acquirers keep working unchanged.
    pub fn register(agent_id: AgentId) -> Self {
        let inserted = HELD_AGENT_LOCKS
            .try_with(|set| set.borrow_mut().insert(agent_id))
            .unwrap_or(false);
        Self { agent_id, inserted }
    }
}

impl Drop for HeldLockGuard {
    fn drop(&mut self) {
        if self.inserted {
            // `try_with` can only fail if the scope was already torn down,
            // which cannot happen while this guard (a stack value inside
            // the scoped future) is alive. The `let _ =` is defensive.
            let _ = HELD_AGENT_LOCKS.try_with(|set| {
                set.borrow_mut().remove(&self.agent_id);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_held_is_false_outside_any_scope() {
        // Non-agent-loop callers (HTTP attachment path, etc.) must see the
        // safe default so their behaviour is unchanged.
        assert!(!is_held(AgentId::new()));
        assert!(held_snapshot().is_empty());
    }

    #[tokio::test]
    async fn register_then_drop_clears_the_entry() {
        let a = AgentId::new();
        scope(async move {
            assert!(!is_held(a));
            {
                let _g = HeldLockGuard::register(a);
                assert!(is_held(a), "registered while guard alive");
                assert_eq!(held_snapshot(), vec![a]);
            }
            // RAII drop on normal scope exit must clear the entry, otherwise a
            // later legitimate non-re-entrant acquisition would be wrongly
            // rejected.
            assert!(!is_held(a), "entry must be cleared after guard drop");
            assert!(held_snapshot().is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn drop_is_panic_safe() {
        let a = AgentId::new();
        let r = scope(async move {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _g = HeldLockGuard::register(a);
                assert!(is_held(a));
                panic!("boom");
            }));
            assert!(res.is_err(), "inner closure must have panicked");
            // Unwinding ran the guard's destructor — the entry must be gone so
            // a subsequent acquisition is not spuriously rejected.
            is_held(a)
        })
        .await;
        assert!(!r, "panic unwind must still clear the held entry (RAII)");
    }

    #[tokio::test]
    async fn scope_is_idempotent_inner_shares_outer_set() {
        // A transitively re-entered inner frame must observe the outer
        // frame's registrations, not a fresh empty set — that sharing is
        // exactly what makes same-task re-entry detectable.
        let a = AgentId::new();
        scope(async move {
            let _outer = HeldLockGuard::register(a);
            assert!(is_held(a));
            scope(async move {
                assert!(
                    is_held(a),
                    "inner scope must see the outer frame's held set"
                );
            })
            .await;
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn task_local_does_not_bleed_across_tasks() {
        // The whole correctness argument for #5125/#5126 rests on this: the
        // registry is per-TASK. A different task must NOT observe another
        // task's held set, so cross-task acquisitions always take the real
        // mutex.
        let a = AgentId::new();
        let handle = tokio::spawn(scope(async move {
            let _g = HeldLockGuard::register(a);
            assert!(is_held(a), "holder task sees its own registration");
            // A freshly spawned task is a new task → no scope, is_held false.
            tokio::spawn(async move { is_held(a) }).await.unwrap()
        }));
        let other_task_saw_held = handle.await.unwrap();
        assert!(
            !other_task_saw_held,
            "a different task must never observe the holder task's held set"
        );
    }
}
