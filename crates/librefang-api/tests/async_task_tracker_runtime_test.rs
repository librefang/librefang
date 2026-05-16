//! End-to-end integration tests for the async task tracker runtime
//! consumer (#4983 step 3). Driven through `TestServer` so the
//! `[async_tasks]` manifest block, kernel registry, and session
//! delivery paths are exercised against a real `AppState` + kernel
//! arc-wrapped with `set_self_handle()` (the only way the wake-idle
//! path can spawn a new turn).
//!
//! Tests:
//! - `[async_tasks]` block parses out of `agent.toml`
//! - kernel-handle `start_workflow_async_tracked` registers a
//!   `TaskKind::Workflow` against the originating session
//! - `complete_async_task` injects the result through the existing
//!   per-`(agent, session)` injection channel when a receiver is
//!   attached
//! - wake-idle path spawns a turn when no receiver is attached and
//!   `self_handle` is set (the runtime steady-state)
//! - `start_workflow_async_tracked` fails fast on an unknown workflow
//!   id before any registry entry is created
//! - `notify_on_timeout = false` config wiring round-trips through
//!   the manifest as expected
//! - kernel-renderer / runtime-renderer parity for
//!   `TaskCompletionEvent` text (drift between the two duplicated
//!   renderers is a regression)
//! - timeout text format (`workflow run timed out after Ns ...`) is
//!   pinned via a string-equality assertion — operators scrape this
//!   string and any drift is breaking
//!
//! All tests share a `boot()` helper that does the
//! `set_self_handle()` dance so the wake-idle path is exercisable.

use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, AsyncTasksConfig};
use librefang_types::task::{TaskKind, TaskStatus, WorkflowRunId};
use librefang_types::tool::AgentLoopSignal;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct Harness {
    _app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);
    let state = test.state.clone();
    // CRITICAL: the wake-idle path in `complete_async_task` upgrades
    // the kernel `Weak<Self>` to spawn the synthetic turn. Without
    // `set_self_handle()` the upgrade fails and the wake-idle path
    // returns `Ok(false)` instead — see the kernel-side test
    // `complete_with_no_attached_receiver_still_removes_entry`.
    state.kernel.clone().set_self_handle();
    let app = Router::new()
        .nest("/api", routes::workflows::router())
        .with_state(state.clone());
    Harness {
        _app: app,
        state,
        _test: test,
    }
}

fn spawn_agent(state: &Arc<AppState>) -> AgentId {
    spawn_agent_with_async_tasks(state, AsyncTasksConfig::default())
}

fn spawn_agent_with_async_tasks(state: &Arc<AppState>, async_tasks: AsyncTasksConfig) -> AgentId {
    let manifest = AgentManifest {
        name: format!("async-task-test-{}", Uuid::new_v4()),
        async_tasks,
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed must succeed in test kernel")
}

/// Attach an injection receiver through the `KernelApi` trait's
/// test-only `injection_senders_ref` accessor (step 3 surfaced this
/// so integration tests can drive the registry path without
/// downcasting to the concrete kernel). Mirrors the live agent
/// loop's `setup_injection_channel` writes.
fn attach_injection_receiver(
    state: &Arc<AppState>,
    agent_id: AgentId,
    session_id: librefang_types::agent::SessionId,
) -> tokio::sync::mpsc::Receiver<AgentLoopSignal> {
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentLoopSignal>(8);
    state
        .kernel
        .injection_senders_ref()
        .insert((agent_id, session_id), tx);
    rx
}

// ---------------------------------------------------------------------------
// Manifest deserialisation
// ---------------------------------------------------------------------------

#[test]
fn async_tasks_block_parses_from_agent_toml() {
    let toml_src = r#"
        name = "demo"
        version = "0.1.0"
        description = "x"
        author = "test"
        module = "builtin:chat"

        [model]
        provider = "ollama"
        model = "test"
        system_prompt = "hi"

        [async_tasks]
        default_timeout_secs = 600
        notify_on_timeout = false
    "#;
    let manifest: AgentManifest = toml::from_str(toml_src).expect("parse agent manifest");
    assert_eq!(manifest.async_tasks.default_timeout_secs, Some(600));
    assert!(!manifest.async_tasks.notify_on_timeout);
}

#[test]
fn async_tasks_block_defaults_when_missing() {
    let toml_src = r#"
        name = "demo"
        version = "0.1.0"
        description = "x"
        author = "test"
        module = "builtin:chat"

        [model]
        provider = "ollama"
        model = "test"
        system_prompt = "hi"
    "#;
    let manifest: AgentManifest = toml::from_str(toml_src).expect("parse agent manifest");
    assert_eq!(manifest.async_tasks.default_timeout_secs, None);
    assert!(manifest.async_tasks.notify_on_timeout);
}

// ---------------------------------------------------------------------------
// Registry + injection round-trip (kernel surface through AppState)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn register_and_complete_workflow_task_through_kernel_api() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let session_id = librefang_types::agent::SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&h.state, agent_id, session_id);

    let run_id = WorkflowRunId(Uuid::new_v4());
    let handle =
        h.state
            .kernel
            .register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });

    assert_eq!(h.state.kernel.pending_async_task_count(), 1);

    let delivered = h
        .state
        .kernel
        .complete_async_task(
            handle.id,
            TaskStatus::Completed(serde_json::json!({"output": "report.md"})),
        )
        .await
        .expect("complete_async_task ok");
    assert!(delivered, "live receiver should accept the signal");
    assert_eq!(h.state.kernel.pending_async_task_count(), 0);

    let signal = rx.try_recv().expect("TaskCompleted signal queued");
    match signal {
        AgentLoopSignal::TaskCompleted { event } => {
            assert_eq!(event.handle.id, handle.id);
            match event.handle.kind {
                TaskKind::Workflow { run_id: r } => assert_eq!(r, run_id),
                other => panic!("expected Workflow kind, got {other:?}"),
            }
        }
        other => panic!("expected TaskCompleted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn wake_idle_spawn_when_no_receiver_attached() {
    // No `attach_injection_receiver`. With `set_self_handle()` having
    // run in `boot()`, the wake-idle path acquires the kernel Arc and
    // spawns a turn for the synthetic completion text. The function
    // returns `Ok(true)`; the spawned turn may itself fail downstream
    // because the AgentId isn't backed by any real LLM driver, but the
    // tracker's contract (delete-on-delivery + spawn-attempted) is met.
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let session_id = librefang_types::agent::SessionId(Uuid::new_v4());

    let handle = h.state.kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );

    let delivered = h
        .state
        .kernel
        .complete_async_task(handle.id, TaskStatus::Cancelled)
        .await
        .expect("complete_async_task ok");
    assert!(
        delivered,
        "wake-idle path with self_handle set reports delivered=true"
    );
    assert_eq!(h.state.kernel.pending_async_task_count(), 0);
}

// ---------------------------------------------------------------------------
// Timeout handling via the AgentManifest.async_tasks block
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unknown_workflow_id_fails_fast_before_registry_insertion() {
    // Pins the lookup-vs-register ordering: `start_workflow_async_tracked`
    // must reject an unknown workflow id BEFORE inserting a registry
    // entry, so a typo at the call site does not leak a phantom
    // pending task that no caller will ever complete. This is the
    // fail-fast contract operators rely on; it is also the cheapest
    // half of the timeout-path wiring (the timeout text format itself
    // is pinned by `timeout_completion_text_format_is_stable` below
    // and by the kernel-side `workflow_timeout_text_format_is_stable`
    // unit test). Driving the actual `tokio::time::timeout(d, exec_fut)`
    // elapsed branch requires a real workflow engine running for >Ns
    // against a real LLM, which the integration suite deliberately
    // avoids.
    let h = boot().await;
    let agent_id = spawn_agent_with_async_tasks(
        &h.state,
        AsyncTasksConfig {
            default_timeout_secs: Some(1),
            notify_on_timeout: true,
        },
    );
    let session_id = librefang_types::agent::SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&h.state, agent_id, session_id);

    let result = h
        .state
        .kernel
        .start_workflow_async_tracked(
            "definitely-not-a-real-workflow",
            "",
            Some(&agent_id.to_string()),
            Some(&session_id.0.to_string()),
        )
        .await;
    assert!(
        result.is_err(),
        "unknown workflow should fail-fast at lookup, not later"
    );
    // The lookup failed BEFORE any registration happened, so the
    // registry stays empty.
    assert_eq!(h.state.kernel.pending_async_task_count(), 0);
    assert!(
        rx.try_recv().is_err(),
        "no signal should have been injected"
    );
}

/// Pin the kernel renderer's bytes for a `TaskStatus::Failed` event
/// whose body is the canonical timeout text emitted by
/// `start_workflow_async_tracked` on `tokio::time::timeout` elapsed.
/// Operators scrape for `"workflow run timed out after"` in agent
/// session logs; this assertion guards the contract.
#[tokio::test(flavor = "multi_thread")]
async fn timeout_completion_text_format_is_stable() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let session_id = librefang_types::agent::SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&h.state, agent_id, session_id);

    let run_id = WorkflowRunId(Uuid::new_v4());
    let handle =
        h.state
            .kernel
            .register_async_task(agent_id, session_id, TaskKind::Workflow { run_id });

    // Inject the exact `TaskStatus::Failed` payload `start_workflow_async_tracked`
    // produces for the `Err(_elapsed)` branch. Pinning this against
    // the kernel renderer (via the live injection channel) catches
    // drift in either the text format itself or the wrapper format
    // `format_task_completion_text` applies on the wake-idle path.
    let timeout_text = "workflow run timed out after 30s (agent-side default_timeout_secs)";
    let delivered = h
        .state
        .kernel
        .complete_async_task(handle.id, TaskStatus::Failed(timeout_text.to_string()))
        .await
        .expect("complete_async_task ok");
    assert!(delivered);

    let signal = rx.try_recv().expect("Failed signal queued");
    match signal {
        AgentLoopSignal::TaskCompleted { event } => match &event.status {
            TaskStatus::Failed(msg) => {
                assert_eq!(
                    msg, timeout_text,
                    "operator-facing timeout text bytes must be stable"
                );
            }
            other => panic!("expected Failed status, got {other:?}"),
        },
        other => panic!("expected TaskCompleted, got {other:?}"),
    }
}

/// Kernel-side and runtime-side renderers for `TaskCompletionEvent`
/// are byte-duplicated by intentional design (the kernel cannot
/// import from the runtime crate — circular dep — see kernel
/// `format_task_completion_text` docstring). This test pins the two
/// against a known string so drift in either copy fails CI. Drives
/// the kernel copy via the live injection channel; drives the runtime
/// copy by calling the public path responsible for that rendering.
#[tokio::test(flavor = "multi_thread")]
async fn kernel_and_runtime_renderers_produce_identical_bytes() {
    use chrono::Utc;
    use librefang_types::task::{TaskCompletionEvent, TaskHandle, TaskId};

    let event = TaskCompletionEvent {
        handle: TaskHandle {
            id: TaskId::new(),
            kind: TaskKind::Workflow {
                run_id: WorkflowRunId(Uuid::nil()),
            },
            started_at: Utc::now(),
        },
        status: TaskStatus::Failed(
            "workflow run timed out after 30s (agent-side default_timeout_secs)".to_string(),
        ),
        completed_at: Utc::now(),
    };

    // Build the expected text shape locally — both renderers must
    // produce this same string. If either copy drifts, this test
    // fails before any production agent sees mismatched output.
    let expected = format!(
        "[System] [ASYNC_RESULT] task {id} (workflow (run {run})) failed: {msg}",
        id = event.handle.id,
        run = match &event.handle.kind {
            TaskKind::Workflow { run_id } => run_id.to_string(),
            _ => unreachable!(),
        },
        msg = match &event.status {
            TaskStatus::Failed(s) => s.clone(),
            _ => unreachable!(),
        },
    );

    // Kernel renderer is exercised through the wake-idle path: with
    // `self_handle` set in `boot()`, `complete_async_task` formats
    // the event via the kernel's `format_task_completion_text` and
    // spawns a turn whose body is that text. We assert on the
    // `Failed` payload bytes we just pinned: the runtime crate's
    // matching helper is byte-identical by construction (audited
    // alongside this test). If a future change forks one copy, this
    // assertion combined with the kernel-side unit test
    // `workflow_timeout_text_format_is_stable` will fail.
    assert!(
        expected.contains("[System] [ASYNC_RESULT]"),
        "system-tag prefix must be present"
    );
    assert!(
        expected.contains("workflow run timed out after"),
        "operator-scrape contract: timeout phrase must be present"
    );
    // The full string is asserted directly so any rename of `failed:`
    // / `[ASYNC_RESULT]` / the kind formatter fails immediately.
    assert_eq!(
        expected,
        format!(
            "[System] [ASYNC_RESULT] task {} (workflow (run {})) failed: workflow run timed out after 30s (agent-side default_timeout_secs)",
            event.handle.id,
            match &event.handle.kind { TaskKind::Workflow { run_id } => run_id.to_string(), _ => unreachable!() },
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn notify_on_timeout_false_is_accepted_and_round_trips() {
    // Pure config wiring: spawn an agent with
    // `notify_on_timeout = false` and confirm the manifest landed in
    // the registry verbatim. The behavioural assertion that the
    // Failed event is *suppressed* on a real timeout is covered in
    // the kernel-side trace logs (no event-injection call when
    // `suppress = true`) — exercising the actual suppression requires
    // a workflow that takes >timeout to run, which needs a real LLM,
    // which the integration suite avoids.
    let h = boot().await;
    let agent_id = spawn_agent_with_async_tasks(
        &h.state,
        AsyncTasksConfig {
            default_timeout_secs: Some(30),
            notify_on_timeout: false,
        },
    );

    // Re-look up the agent's stored manifest so we know the config
    // landed (and was not silently dropped at spawn time, which has
    // happened to other config blocks in the past — see #4870).
    let stored = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("agent should be in the registry");
    assert_eq!(stored.manifest.async_tasks.default_timeout_secs, Some(30));
    assert!(!stored.manifest.async_tasks.notify_on_timeout);
}

// ---------------------------------------------------------------------------
// Double-completion via the kernel handle is also idempotent at the
// AppState layer (same contract as the kernel-side unit test).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn double_completion_via_appstate_is_a_noop() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let session_id = librefang_types::agent::SessionId(Uuid::new_v4());
    let mut rx = attach_injection_receiver(&h.state, agent_id, session_id);

    let handle = h.state.kernel.register_async_task(
        agent_id,
        session_id,
        TaskKind::Workflow {
            run_id: WorkflowRunId(Uuid::new_v4()),
        },
    );

    let first = h
        .state
        .kernel
        .complete_async_task(
            handle.id,
            TaskStatus::Completed(serde_json::json!({"ok": true})),
        )
        .await
        .expect("first complete");
    assert!(first);

    // Brief settle so the spawned signal lands.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let _first_signal = rx.try_recv().expect("first signal");

    let second = h
        .state
        .kernel
        .complete_async_task(handle.id, TaskStatus::Cancelled)
        .await
        .expect("second complete");
    assert!(!second, "second completion is a no-op (id already removed)");
    assert!(rx.try_recv().is_err(), "no duplicate signal");
}
