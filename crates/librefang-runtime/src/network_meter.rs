//! Per-tool-call accounting of bytes an agent pulls in over the network.
//!
//! `agent.toml: [resources] max_network_bytes_per_hour` is enforced by the kernel scheduler, but the only place that knows how many bytes actually came off the wire is the runtime primitive doing the reading — the `web_fetch` streaming loop, the `web_fetch_to_file` download loop, the legacy plain-HTTP fallback, the WASM `net_fetch` host call, an MCP tool's response payload.
//! Those primitives sit several layers below the dispatcher that holds the kernel handle and the calling agent's id, and threading a counter through every one of their signatures would mean changing the public shape of `WebFetchEngine::fetch_with_options` and friends for every caller, most of which have no agent to attribute to.
//!
//! So the counter travels on the task instead, exactly as `tool_runner`'s `AGENT_CALL_DEPTH` task-local does for nesting depth: `tool_runner::dispatch` wraps each tool call in [`measure`], the primitives call [`record`] as they read, and the dispatcher hands the total to the kernel afterwards.
//! A primitive invoked outside a measured tool call — the agent loop's own link prefetch, a channel bridge, a test — finds no counter installed and [`record`] is a no-op, which is why the field's documentation names the tools it covers rather than claiming to meter all egress.
//!
//! One reporter cannot reach the task-local at all: a WASM guest's `net_fetch` runs on a `spawn_blocking` thread, and a tokio task-local is set on the polling thread only for the duration of `TaskLocalFuture::poll`.
//! [`current`] and [`scoped`] exist for that hop — `WasmSandbox::execute` takes a handle to the counter before it leaves the task and `host_net_fetch` re-installs it around its `block_on`, so the guest's reads land on the same meter as `web_fetch`'s.
//!
//! The counter is an `Arc<AtomicU64>` rather than a `Cell` so [`measure`] keeps a handle to read the total after the measured future has finished with it; the task-local itself is never handed out.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

tokio::task_local! {
    /// Bytes reported so far against the tool call currently running on this task.
    static NETWORK_BYTES: Arc<AtomicU64>;
}

/// Run `fut` with a fresh byte counter installed, returning its output and the bytes [`record`] reported while it ran.
///
/// Nesting is not special-cased: an inner `measure` shadows the outer counter for the duration of the inner future, so bytes read there are attributed to the inner call only.
/// Tool dispatch is the single caller and does not nest, so in practice there is one counter per tool call.
pub(crate) async fn measure<F>(fut: F) -> (F::Output, u64)
where
    F: Future,
{
    let counter = Arc::new(AtomicU64::new(0));
    let output = NETWORK_BYTES.scope(Arc::clone(&counter), fut).await;
    (output, counter.load(Ordering::Relaxed))
}

/// A handle to the counter installed on the current task, if there is one.
///
/// The escape hatch for a reporter that runs somewhere the task-local cannot follow — today only the WASM sandbox, whose guest executes on a blocking-pool thread.
/// Taking the handle on the task and re-installing it there with [`scoped`] keeps the reporting API a plain [`record`] call everywhere else.
pub(crate) fn current() -> Option<Arc<AtomicU64>> {
    NETWORK_BYTES.try_with(Arc::clone).ok()
}

/// Run `fut` with `counter` installed as this task's byte counter.
///
/// The counterpart to [`current`]: a counter carried across a thread boundary by hand goes back into the task-local here, so the primitives underneath keep reporting through [`record`] instead of needing a counter argument threaded through every signature.
pub(crate) async fn scoped<F>(counter: Arc<AtomicU64>, fut: F) -> F::Output
where
    F: Future,
{
    NETWORK_BYTES.scope(counter, fut).await
}

/// Report `bytes` read off the network against the tool call currently running on this task.
///
/// A no-op when no [`measure`] scope is installed. Saturating, because the total is an operator-facing byte count and wrapping it would silently hand an agent an unlimited budget.
pub(crate) fn record(bytes: u64) {
    if bytes == 0 {
        return;
    }
    let _ = NETWORK_BYTES.try_with(|counter| {
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(bytes))
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The meter sums every report made inside one scope. Without the module this cannot compile; with a counter that resets per report it would return 5 instead of 15.
    #[tokio::test]
    async fn measure_sums_reports_made_inside_the_scope() {
        let (output, bytes) = measure(async {
            record(10);
            record(5);
            "done"
        })
        .await;
        assert_eq!(output, "done");
        assert_eq!(bytes, 15);
    }

    /// A report made outside any scope must be dropped rather than panic: the same HTTP primitives run on the agent loop's prefetch path and in tests, where no tool call is being measured.
    #[tokio::test]
    async fn record_outside_a_scope_is_a_noop() {
        record(4096);
        let (_, bytes) = measure(async { record(1) }).await;
        assert_eq!(
            bytes, 1,
            "the unscoped report must not leak into the next measured call"
        );
    }

    /// A report made on a `spawn_blocking` thread never reaches the task that installed the counter: a tokio task-local lives in a thread-local slot only while `TaskLocalFuture::poll` is running on the polling thread.
    /// This is the whole reason [`current`] and [`scoped`] exist — a WASM guest's `net_fetch` runs behind exactly this hop, and a bare [`record`] there is silently dropped.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_report_from_a_blocking_thread_does_not_reach_the_installing_task() {
        let (_, bytes) = measure(async {
            tokio::task::spawn_blocking(|| record(4096))
                .await
                .expect("blocking task joins");
        })
        .await;
        assert_eq!(
            bytes, 0,
            "a task-local does not cross spawn_blocking; if this ever starts passing, the sandbox's hand-carried counter is redundant rather than load-bearing"
        );
    }

    /// Re-installing the handle [`current`] took puts the blocking thread's reads back on the tool call's counter — the shape `WasmSandbox::execute` and `host_net_fetch` use between them.
    #[tokio::test(flavor = "multi_thread")]
    async fn scoped_returns_a_blocking_thread_s_reports_to_the_task_s_counter() {
        let handle = tokio::runtime::Handle::current();
        let (_, bytes) = measure(async move {
            let counter = current().expect("measure installs a counter on this task");
            tokio::task::spawn_blocking(move || {
                handle.block_on(scoped(counter, async {
                    record(1024);
                    record(3072);
                }))
            })
            .await
            .expect("blocking task joins");
        })
        .await;
        assert_eq!(
            bytes, 4096,
            "bytes read on the blocking thread must be charged to the tool call that started it"
        );
    }

    /// Outside a measured tool call there is no counter to carry anywhere, and the sandbox must get `None` rather than a counter charged to nobody.
    #[tokio::test]
    async fn current_is_none_outside_a_measured_call() {
        assert!(current().is_none());
        let (inner, _) = measure(async { current().is_some() }).await;
        assert!(inner, "inside a scope the handle must be available");
    }

    /// Reports made across an await point still land on the same counter — the case that matters, since every real reporter is a chunked read loop.
    #[tokio::test]
    async fn measure_survives_await_points() {
        let (_, bytes) = measure(async {
            record(1);
            tokio::task::yield_now().await;
            record(2);
        })
        .await;
        assert_eq!(bytes, 3);
    }
}
