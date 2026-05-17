//! Regression (#5144): the SSE log / comms-event stream handlers spawn a
//! detached polling task that holds an `Arc<AppState>` (and therefore the
//! whole kernel graph via `state.kernel`). Before the fix the only loop
//! exit was `tx.send` returning `Err` (client disconnect), so on daemon
//! shutdown the task kept the entire `AppState` pinned for as long as any
//! dashboard tab kept an SSE channel open.
//!
//! These tests keep the SSE response (and thus the channel receiver)
//! alive so a client-disconnect exit is impossible, then fire the kernel
//! shutdown signal and assert the detached task drops its `Arc<AppState>`
//! clone — i.e. it exited on the shutdown signal, not on disconnect.

use axum::extract::{Query, State};
use librefang_api::routes::{logs, network, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Poll `Arc::strong_count(state)` down to `target` within a bounded
/// budget. Returns the final observed count.
async fn wait_for_strong_count(state: &Arc<AppState>, target: usize) -> usize {
    for _ in 0..200 {
        let c = Arc::strong_count(state);
        if c <= target {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Arc::strong_count(state)
}

// Multi-thread flavor: `MockKernelBuilder::build` calls `block_in_place`
// during kernel boot, which panics on the single-thread runtime.
#[tokio::test(flavor = "multi_thread")]
async fn logs_stream_task_exits_on_kernel_shutdown() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();

    // Baseline: `state` (our handle) + `test.state` (the harness's).
    let baseline = Arc::strong_count(&state);

    // Invoke the handler directly. It spawns a detached task that moves
    // a clone of `state` in, so the strong count must rise.
    let resp = logs::logs_stream(State(state.clone()), Query(HashMap::new())).await;

    // The detached poll task now holds an extra Arc clone.
    let with_task = wait_for_task_to_hold_ref(&state, baseline).await;
    assert!(
        with_task > baseline,
        "detached SSE task must hold an extra Arc<AppState> (baseline {baseline}, observed {with_task})"
    );

    // Keep the response (and its channel receiver) ALIVE so the only
    // possible loop exit is the shutdown signal, never client
    // disconnect.
    let _resp_kept_alive = resp;

    // Fire the kernel shutdown signal.
    state.kernel.supervisor_ref().shutdown();

    // The task must observe `shutdown_rx.changed()` and return, dropping
    // its Arc clone — strong count returns to baseline even though the
    // SSE receiver is still held.
    let after = wait_for_strong_count(&state, baseline).await;
    assert_eq!(
        after, baseline,
        "logs_stream task did not exit on shutdown while the SSE receiver was still alive \
         (baseline {baseline}, after {after}) — it would have pinned the kernel"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn comms_events_stream_task_exits_on_kernel_shutdown() {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let baseline = Arc::strong_count(&state);

    let resp = network::comms_events_stream(State(state.clone())).await;

    let with_task = wait_for_task_to_hold_ref(&state, baseline).await;
    assert!(
        with_task > baseline,
        "detached comms SSE task must hold an extra Arc<AppState> (baseline {baseline}, observed {with_task})"
    );

    let _resp_kept_alive = resp;

    state.kernel.supervisor_ref().shutdown();

    let after = wait_for_strong_count(&state, baseline).await;
    assert_eq!(
        after, baseline,
        "comms_events_stream task did not exit on shutdown while the SSE receiver was still alive \
         (baseline {baseline}, after {after}) — it would have pinned the kernel"
    );
}

/// Spin until the freshly-spawned detached task has actually started and
/// captured its `Arc<AppState>` clone (it is spawned, not run inline).
async fn wait_for_task_to_hold_ref(state: &Arc<AppState>, baseline: usize) -> usize {
    for _ in 0..200 {
        let c = Arc::strong_count(state);
        if c > baseline {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Arc::strong_count(state)
}
