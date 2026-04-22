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
}

impl SessionInterrupt {
    /// Create a new, un-cancelled interrupt handle.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal that the session should stop.
    ///
    /// Idempotent — safe to call multiple times.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Returns `true` if [`cancel`](Self::cancel) has been called.
    ///
    /// Intended for polling inside tool execution hot-paths where the tool
    /// wants to bail out early without blocking.
    ///
    /// ```rust,ignore
    /// if interrupt.is_cancelled() {
    ///     return Err("[interrupted]".to_string());
    /// }
    /// ```
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Reset the interrupt flag so the handle can be reused for a new turn.
    ///
    /// Only call this when you are certain no outstanding tool futures are
    /// still polling `is_cancelled()` on this handle.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Release);
    }

    /// Return a child handle that shares this interrupt's flag.
    ///
    /// Useful for sub-agents created by `agent_spawn`/`agent_send`: the child
    /// inherits the parent's cancellation without needing a separate channel.
    /// Cancelling the parent (or the child) raises the shared flag, so both
    /// will observe `is_cancelled() == true`.
    pub fn child_token(&self) -> Self {
        Self {
            flag: Arc::clone(&self.flag),
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
}
