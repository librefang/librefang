//! Session-scoped interrupt signaling.
//!
//! Provides per-session interrupt tracking so that stopping one agent session
//! does not kill tools running in other concurrent sessions.  This mirrors the
//! design of `hermes-agent/tools/interrupt.py` but uses Rust idioms:
//!
//! * Each `SessionInterrupt` wraps an `Arc<AtomicBool>`, so cloning it is
//!   cheap and the same flag is shared between the agent loop and any tool
//!   futures that hold a clone.
//! * There is **no** global mutable state — isolation is structural, not
//!   based on thread-locals.
//! * `child_token()` returns a derived handle that shares the parent's flag,
//!   enabling sub-agents (forks, `agent_spawn`) to inherit the interrupt
//!   without an extra allocation when a parent is cancelled.
//!
//! # Usage
//!
//! ```rust,ignore
//! use librefang_runtime::interrupt::SessionInterrupt;
//!
//! // Created once per session / turn.
//! let interrupt = SessionInterrupt::new();
//!
//! // Pass a clone into tool execution context; call cancel() from outside
//! // (e.g. when the user sends /stop).
//! let tool_interrupt = interrupt.clone();
//! tokio::spawn(async move {
//!     for chunk in long_running_work() {
//!         if tool_interrupt.is_cancelled() {
//!             return "[interrupted]".to_string();
//!         }
//!         process(chunk);
//!     }
//! });
//!
//! // Somewhere else — user clicked "Stop":
//! interrupt.cancel();
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Per-session interrupt handle.
///
/// Cheaply cloneable — all clones share the same underlying flag.
/// Thread-safe and async-safe; `cancel()` / `is_cancelled()` can be called
/// from any thread or async task.
#[derive(Clone, Debug, Default)]
pub struct SessionInterrupt {
    flag: Arc<AtomicBool>,
    /// Optional upstream flag observed by `is_cancelled()` but NOT affected
    /// by `cancel()` on this handle. Used to implement one-way cascade from
    /// a parent session to a subagent: parent cancel kills the child, but
    /// cancelling the child has no effect on the parent. See
    /// `new_with_upstream`.
    upstream: Option<Arc<AtomicBool>>,
}

impl SessionInterrupt {
    /// Create a new, un-cancelled interrupt handle.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            upstream: None,
        }
    }

    /// Create a new interrupt whose `is_cancelled()` ALSO returns `true`
    /// whenever `upstream` has been cancelled. `cancel()` on this handle
    /// does not affect `upstream`.
    ///
    /// Use this when a subagent is invoked on behalf of a parent session
    /// (e.g. `agent_send`, hand dispatch) and the subagent's loop should
    /// abort as soon as the parent's `/stop` fires, without cancelling the
    /// child's flag leaking back to the parent. See issue #3044.
    pub fn new_with_upstream(upstream: &SessionInterrupt) -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            upstream: Some(Arc::clone(&upstream.flag)),
        }
    }

    /// Signal that the session should stop.
    ///
    /// Idempotent — safe to call multiple times. Only affects this handle's
    /// own flag; any `upstream` reference is read-only.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Returns `true` if [`cancel`](Self::cancel) has been called on this
    /// handle, OR on the upstream handle (if any). Intended for polling
    /// inside tool execution hot-paths where the tool wants to bail out
    /// early without blocking.
    ///
    /// ```rust,ignore
    /// if interrupt.is_cancelled() {
    ///     return Err("[interrupted]".to_string());
    /// }
    /// ```
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
            || self
                .upstream
                .as_ref()
                .is_some_and(|u| u.load(Ordering::Acquire))
    }

    /// Reset the interrupt flag so the handle can be reused for a new turn.
    ///
    /// Only call this when you are certain no outstanding tool futures are
    /// still polling `is_cancelled()` on this handle. Does not touch the
    /// upstream flag.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Release);
    }

    /// Return a child handle that shares this interrupt's flag.
    ///
    /// Useful for sub-agents created by `agent_spawn`/`agent_send`: the child
    /// inherits the parent's cancellation without needing a separate channel.
    /// Cancelling the parent (or the child) raises the shared flag, so both
    /// will observe `is_cancelled() == true`.
    ///
    /// NOTE: unlike `new_with_upstream`, `child_token` is fully symmetric —
    /// cancelling either side cancels both. Prefer `new_with_upstream` when
    /// you need one-way cascade.
    pub fn child_token(&self) -> Self {
        Self {
            flag: Arc::clone(&self.flag),
            upstream: self.upstream.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_not_cancelled() {
        let interrupt = SessionInterrupt::new();
        assert!(!interrupt.is_cancelled());
    }

    #[test]
    fn cancel_sets_flag() {
        let interrupt = SessionInterrupt::new();
        interrupt.cancel();
        assert!(interrupt.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let interrupt = SessionInterrupt::new();
        interrupt.cancel();
        interrupt.cancel();
        assert!(interrupt.is_cancelled());
    }

    #[test]
    fn reset_clears_flag() {
        let interrupt = SessionInterrupt::new();
        interrupt.cancel();
        interrupt.reset();
        assert!(!interrupt.is_cancelled());
    }

    #[test]
    fn clone_shares_flag() {
        let a = SessionInterrupt::new();
        let b = a.clone();
        a.cancel();
        assert!(b.is_cancelled(), "clone must see parent cancel");
    }

    #[test]
    fn child_token_shares_flag() {
        let parent = SessionInterrupt::new();
        let child = parent.child_token();
        parent.cancel();
        assert!(child.is_cancelled(), "child must see parent cancel");
    }

    #[test]
    fn two_independent_sessions_are_isolated() {
        let s1 = SessionInterrupt::new();
        let s2 = SessionInterrupt::new();
        s1.cancel();
        assert!(s1.is_cancelled());
        assert!(!s2.is_cancelled(), "cancelling s1 must not affect s2");
    }

    // ── Upstream cascade (issue #3044 follow-up) ───────────────────────────

    #[test]
    fn upstream_cancel_cascades_to_child() {
        let parent = SessionInterrupt::new();
        let child = SessionInterrupt::new_with_upstream(&parent);
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled(), "child must observe parent cancel");
    }

    #[test]
    fn child_cancel_does_not_leak_to_upstream() {
        let parent = SessionInterrupt::new();
        let child = SessionInterrupt::new_with_upstream(&parent);
        child.cancel();
        assert!(child.is_cancelled());
        assert!(
            !parent.is_cancelled(),
            "cancelling child must NOT cancel the parent"
        );
    }

    #[test]
    fn child_reset_does_not_affect_upstream() {
        let parent = SessionInterrupt::new();
        let child = SessionInterrupt::new_with_upstream(&parent);
        parent.cancel();
        assert!(child.is_cancelled());
        child.reset(); // no-op on upstream
        assert!(
            child.is_cancelled(),
            "child.reset() must not hide upstream cancel"
        );
    }

    #[test]
    fn multiple_siblings_share_upstream() {
        let parent = SessionInterrupt::new();
        let a = SessionInterrupt::new_with_upstream(&parent);
        let b = SessionInterrupt::new_with_upstream(&parent);
        parent.cancel();
        assert!(a.is_cancelled());
        assert!(b.is_cancelled());
    }
}
