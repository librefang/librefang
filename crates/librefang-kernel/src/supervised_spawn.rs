//! Supervised `tokio::spawn` wrapper that surfaces panics (#3740).
//!
//! Plain `tokio::spawn(async { ... })` for fire-and-forget tasks silently
//! drops the `JoinHandle`, so any panic inside the future vanishes — the
//! supervisor never learns the task died, and downstream subscribers
//! (channel listeners, cron tickers, inbox pumps, persist loops) just stop
//! producing without an error.
//!
//! `spawn_supervised(name, future)` wraps the future in `AssertUnwindSafe`
//! plus `catch_unwind` so a panic is logged at `error!` level with the task
//! name and (when available) panic payload, instead of being lost. The
//! returned `JoinHandle` is the same shape `tokio::spawn` returns, so call
//! sites can be migrated mechanically.

use futures::FutureExt as _;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use tokio::task::JoinHandle;
use tracing::error;

/// Spawn a fire-and-forget task with panic logging.
///
/// `name` is purely for log correlation — pass a static string identifying
/// the call site (e.g. `"channel_bridge_loop"`). The returned handle can
/// be discarded; the panic is caught and logged inside the wrapper.
///
/// On panic the future is dropped immediately — its internal state is
/// not preserved or retried. This wrapper is a diagnostic aid, not a
/// restart mechanism; callers that need retry logic must implement it
/// inside the future itself.
pub fn spawn_supervised<F>(name: &'static str, future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        // catch_unwind requires UnwindSafe; futures rarely advertise that
        // bound but in practice tokio tasks already isolate state at the
        // poll boundary. AssertUnwindSafe is the standard escape hatch
        // (mirrors what the tracing-subscriber and tower implementations
        // do for the same reason).
        let result = AssertUnwindSafe(future).catch_unwind().await;
        if let Err(payload) = result {
            // Best-effort: extract a string description from the payload.
            let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "<non-string panic payload>".to_string()
            };
            error!(task = name, panic = %msg, "supervised task panicked");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn ok_path_runs_to_completion() {
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        let h = spawn_supervised("test_ok", async move {
            f.store(true, Ordering::SeqCst);
        });
        h.await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn panic_is_caught_and_handle_resolves() {
        // Without spawn_supervised, awaiting this handle would yield a
        // JoinError. With supervision, the panic is swallowed inside and
        // the handle resolves cleanly to ().
        let h = spawn_supervised("test_panic", async {
            panic!("boom");
        });
        let result = h.await;
        assert!(result.is_ok(), "supervised handle must not propagate panic");
    }
}
