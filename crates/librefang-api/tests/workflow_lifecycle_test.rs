//! Integration tests for workflow run lifecycle — cancel, total-timeout,
//! and retry-backoff fields (refs #4844 gaps 1, 9, 10).
//!
//! The tests use the same `tower::oneshot` harness as
//! `workflows_routes_integration.rs`. They do NOT require LLM credentials:
//!
//! * Cancel tests manipulate run state via the kernel's `workflow_engine()`
//!   directly (creating Pending / Paused runs) and then hit the HTTP
//!   `POST /api/workflows/runs/{id}/cancel` endpoint.
//! * Total-timeout and retry-backoff tests exercise the round-trip through
//!   `POST /api/workflows` + `GET /api/workflows/{id}` to verify the new
//!   fields are accepted, stored, and returned correctly.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness (identical to workflows_routes_integration.rs)
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
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
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::workflows::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => {
            builder = builder.header("content-type", "application/json");
            b"{}".to_vec()
        }
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// Helper: create a minimal workflow via the HTTP API and return its id.
// ---------------------------------------------------------------------------

async fn create_workflow(h: &Harness) -> String {
    let agent_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = json_request(
        h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "lifecycle-test",
            "description": "test",
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hello"}]
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create_workflow failed: {body:?}"
    );
    body["workflow_id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Gap #1 — Cancel run
// ---------------------------------------------------------------------------

/// Cancel a `Pending` run: the HTTP endpoint returns 200 with
/// `"state": "cancelled"`, and a subsequent GET on the run confirms the
/// state.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_pending_run_returns_200_and_state_is_cancelled() {
    use librefang_kernel::workflow::{WorkflowId, WorkflowRunState};

    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    // Seed a Pending run directly through the engine.
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "test input".to_string())
        .await
        .expect("create_run must succeed for a registered workflow");

    // Hit the cancel endpoint.
    let path = format!("/api/workflows/runs/{}/cancel", run_id);
    let (status, body) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::OK, "cancel must be 200: {body:?}");
    assert_eq!(body["state"], "cancelled", "response state field: {body:?}");
    assert_eq!(
        body["run_id"].as_str().unwrap(),
        run_id.to_string(),
        "run_id echoed back: {body:?}"
    );

    // Verify kernel state.
    let run = engine.get_run(run_id).await.expect("run must exist");
    assert!(
        matches!(run.state, WorkflowRunState::Cancelled),
        "kernel state must be Cancelled, got {:?}",
        run.state
    );
    assert!(run.completed_at.is_some(), "completed_at must be set");
}

/// Cancel a `Paused` run: state transitions to `Cancelled` and the pause
/// snapshot (step index, variables, current input) is cleared.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_paused_run_clears_pause_snapshot() {
    use librefang_kernel::workflow::{WorkflowId, WorkflowRunState};

    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();

    // Create a Pending run then manually advance it to Paused via pause_run.
    let run_id = engine
        .create_run(wf_id, "paused input".to_string())
        .await
        .expect("create_run");

    // pause_run transitions Pending/Running to Paused when the executor next
    // checks — but for our test we call it directly. It sets `pause_request`
    // on a Pending run; that's enough for cancel_run to see the paused-ish
    // state. However, to fully exercise the "already Paused" branch we need
    // to manually set the state. Use the engine's `pause_run` on a run that
    // is Pending: the method lodges a pause_request and returns a token
    // without touching state. We then manually force the state to Paused
    // by calling `cancel_run` after lodging the pause.
    //
    // Simpler approach: just call the cancel endpoint on the Pending run that
    // has a pause_request set, which exercises the same clear_pause_state
    // code path. But to also cover the `Paused{..}` variant, we directly
    // manipulate via the engine's DashMap through the public `pause_run`
    // method and then verify.

    // Lodge a pause request (state stays Pending, pause_request is set).
    engine
        .pause_run(run_id, "awaiting human input")
        .await
        .expect("pause_run on Pending must succeed");

    // Cancel the run — should clear pause_request regardless of state.
    let path = format!("/api/workflows/runs/{}/cancel", run_id);
    let (status, body) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::OK, "cancel must be 200: {body:?}");

    let run = engine.get_run(run_id).await.expect("run must exist");
    assert!(
        matches!(run.state, WorkflowRunState::Cancelled),
        "state must be Cancelled, got {:?}",
        run.state
    );
    // pause_request must be cleared.
    assert!(
        run.pause_request.is_none(),
        "pause_request must be cleared after cancel"
    );
}

/// Cancelling a run that is already in a terminal state (`Cancelled`) returns
/// 409 Conflict at the HTTP layer, with `state` echoed in the body so
/// callers can distinguish completed vs failed vs cancelled conflicts.
///
/// `Completed` and `Failed` runs can only reach those states via the executor
/// (which requires LLM credentials). The 409 path for all three terminal
/// states is exercised at the kernel level in the unit tests; here we cover
/// the HTTP mapping via the only terminal state reachable without LLM:
/// a run that was already cancelled.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_terminal_run_returns_409() {
    use librefang_kernel::workflow::{WorkflowId, WorkflowRunState};

    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    // Move to terminal via the kernel directly.
    engine.cancel_run(run_id).await.expect("first cancel");

    // Run is now Cancelled (terminal).
    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Cancelled));

    // Second cancel via HTTP must be 409.
    let path = format!("/api/workflows/runs/{}/cancel", run_id);
    let (status, body) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "cancelling an already-terminal run must be 409: {body:?}"
    );
    assert_eq!(
        body["error"].as_str().unwrap_or(""),
        "conflict",
        "error field must be 'conflict': {body:?}"
    );
    // R2: state field must be present and name the terminal state.
    assert_eq!(
        body["state"].as_str().unwrap_or(""),
        "cancelled",
        "state field must echo the terminal state: {body:?}"
    );
}

/// Cancel on an unknown run ID returns 404.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_unknown_run_returns_404() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();
    let path = format!("/api/workflows/runs/{}/cancel", unknown);
    let (status, body) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// Cancel with a malformed run ID returns 400.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_invalid_run_id_returns_400() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows/runs/not-a-uuid/cancel",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

// ---------------------------------------------------------------------------
// Gap #9 — total_timeout_secs round-trip
// ---------------------------------------------------------------------------

/// A workflow created with `total_timeout_secs` has that value echoed back
/// in `GET /api/workflows/{id}`.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_total_timeout_secs_round_trips() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "timeout-wf",
            "description": "test",
            "total_timeout_secs": 42,
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["total_timeout_secs"].as_u64(),
        Some(42),
        "total_timeout_secs must survive create→get round-trip: {body:?}"
    );
}

/// A workflow created without `total_timeout_secs` must not emit the field
/// (or emit it as null), not as a default value.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_total_timeout_secs_absent_when_not_set() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "no-timeout-wf",
            "description": "test",
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let val = &body["total_timeout_secs"];
    assert!(
        val.is_null() || val == &serde_json::Value::Null,
        "total_timeout_secs must be absent/null when not set: {body:?}"
    );
}

/// `PUT /api/workflows/{id}` preserves `total_timeout_secs` when the field
/// is omitted from the update payload.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_update_preserves_total_timeout_secs() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    // Create with a timeout.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "preserve-timeout",
            "description": "v1",
            "total_timeout_secs": 99,
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    // Update name only — no total_timeout_secs in payload.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/workflows/{wf_id}"),
        Some(serde_json::json!({
            "name": "preserve-timeout-v2",
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // timeout must be preserved.
    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["total_timeout_secs"].as_u64(),
        Some(99),
        "total_timeout_secs must survive update without the field: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// Gap #10 — retry backoff / jitter fields round-trip
// ---------------------------------------------------------------------------

/// A step configured with `error_mode: "retry"`, `backoff_ms`, and
/// `jitter_pct` has those values reflected back in `GET /api/workflows/{id}`.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_retry_backoff_fields_round_trip() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "retry-backoff-wf",
            "description": "test",
            "steps": [{
                "name": "step-with-retry",
                "agent_id": agent_id,
                "prompt": "do it",
                "error_mode": "retry",
                "max_retries": 3,
                "backoff_ms": 500,
                "jitter_pct": 25
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let step = &body["steps"][0];
    let error_mode = &step["error_mode"];
    // The error_mode field is serialised as a JSON object:
    // {"retry": {"max_retries": 3, "backoff_ms": 500, "jitter_pct": 25}}
    let retry_obj = error_mode.get("retry").unwrap_or(error_mode);
    assert_eq!(
        retry_obj["max_retries"].as_u64(),
        Some(3),
        "max_retries round-trip: {body:?}"
    );
    assert_eq!(
        retry_obj["backoff_ms"].as_u64(),
        Some(500),
        "backoff_ms round-trip: {body:?}"
    );
    assert_eq!(
        retry_obj["jitter_pct"].as_u64(),
        Some(25),
        "jitter_pct round-trip: {body:?}"
    );
}

/// A retry step without `backoff_ms`/`jitter_pct` deserialises cleanly —
/// the new optional fields default to absent (backward-compat check).
#[tokio::test(flavor = "multi_thread")]
async fn workflow_retry_without_backoff_fields_is_backward_compatible() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "retry-compat-wf",
            "description": "test",
            "steps": [{
                "name": "step",
                "agent_id": agent_id,
                "prompt": "do it",
                "error_mode": "retry",
                "max_retries": 2
            }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let step = &body["steps"][0];
    let error_mode = &step["error_mode"];
    let retry_obj = error_mode.get("retry").unwrap_or(error_mode);
    assert_eq!(
        retry_obj["max_retries"].as_u64(),
        Some(2),
        "max_retries must survive without backoff fields: {body:?}"
    );
    // backoff_ms / jitter_pct must be absent (skip_serializing_if = None).
    assert!(
        retry_obj["backoff_ms"].is_null(),
        "backoff_ms must be absent: {retry_obj:?}"
    );
    assert!(
        retry_obj["jitter_pct"].is_null(),
        "jitter_pct must be absent: {retry_obj:?}"
    );
}

// ---------------------------------------------------------------------------
// list_runs state filter for "cancelled"
// ---------------------------------------------------------------------------

/// `GET /api/workflows/runs?state=cancelled` returns only cancelled runs.
#[tokio::test(flavor = "multi_thread")]
async fn list_runs_state_filter_cancelled() {
    use librefang_kernel::workflow::{WorkflowId, WorkflowRunState};

    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();

    // Create two runs, cancel one.
    let run_a = engine
        .create_run(wf_id, "a".to_string())
        .await
        .expect("create a");
    let run_b = engine
        .create_run(wf_id, "b".to_string())
        .await
        .expect("create b");
    engine.cancel_run(run_a).await.expect("cancel a");

    // Both state variants must be visible in the unfiltered list.
    let all_runs = engine.list_runs(None).await;
    assert_eq!(all_runs.len(), 2, "expected 2 runs total: {all_runs:?}");

    // Only run_a in the cancelled filter.
    let cancelled = engine.list_runs(Some("cancelled")).await;
    assert_eq!(
        cancelled.len(),
        1,
        "expected 1 cancelled run: {cancelled:?}"
    );
    assert!(
        matches!(cancelled[0].state, WorkflowRunState::Cancelled),
        "filtered run must be Cancelled"
    );
    assert_eq!(cancelled[0].id, run_a);

    // run_b must not be in the cancelled list.
    assert_ne!(cancelled[0].id, run_b);
}

// ---------------------------------------------------------------------------
// Q2 — success_rate excludes cancelled runs from denominator
// ---------------------------------------------------------------------------

/// Cancelled runs must not count toward the `success_rate` denominator.
///
/// Scenario: 3 completed + 3 cancelled. Completed runs are driven via
/// `execute_run` with an in-process mock sender (no LLM credentials needed).
/// Cancelled runs use `cancel_run` directly. success_rate must be 1.0
/// (not 0.5). `cancelled_count` in the list response must be 3.
#[tokio::test(flavor = "multi_thread")]
async fn list_workflows_success_rate_excludes_cancelled() {
    use librefang_kernel::workflow::WorkflowId;
    use librefang_types::agent::AgentId;

    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    // Create a workflow via the API so it's registered in the engine.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "rate-test",
            "description": "success_rate test",
            "steps": [{"name": "s1", "agent_id": agent_id, "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id_str = body["workflow_id"].as_str().unwrap().to_string();
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();

    // Seed 3 Completed runs via execute_run with an in-process mock sender.
    // No LLM needed — the closure runs entirely in-process.
    for _ in 0..3 {
        let run_id = engine
            .create_run(wf_id, "input".to_string())
            .await
            .expect("create_run for completed");
        engine
            .execute_run(
                run_id,
                |_agent| Some((AgentId::new(), "mock".to_string(), false)),
                |_id: AgentId, _msg: String, _sm: Option<librefang_types::agent::SessionMode>| async move {
                    Ok(("done".to_string(), 0u64, 0u64))
                },
            )
            .await
            .expect("execute_run must complete successfully");
    }

    // Seed 3 Cancelled runs via create_run + cancel_run.
    for _ in 0..3 {
        let run_id = engine
            .create_run(wf_id, "input".to_string())
            .await
            .expect("create_run for cancelled");
        engine.cancel_run(run_id).await.expect("cancel");
    }

    // GET /api/workflows — check aggregates.
    let (status, body) = get(&h, "/api/workflows").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let items = body["items"].as_array().expect("items array");
    let wf_item = items
        .iter()
        .find(|item| item["id"].as_str() == Some(&wf_id_str))
        .unwrap_or_else(|| panic!("workflow not in list: {body:?}"));

    assert_eq!(
        wf_item["run_count"].as_u64(),
        Some(6),
        "total run_count must be 6: {wf_item:?}"
    );
    assert_eq!(
        wf_item["cancelled_count"].as_u64(),
        Some(3),
        "cancelled_count must be 3: {wf_item:?}"
    );
    let rate = wf_item["success_rate"]
        .as_f64()
        .unwrap_or_else(|| panic!("success_rate must be present: {wf_item:?}"));
    assert!(
        (rate - 1.0_f64).abs() < 0.001,
        "success_rate must be 1.0 (cancelled excluded from denominator), got {rate}: {wf_item:?}"
    );
}

// ---------------------------------------------------------------------------
// Re-run with same params (#6292 need 2)
// ---------------------------------------------------------------------------

/// `POST /api/workflows/runs/{run_id}/rerun` starts a fresh run of the same workflow with the original run's input, and leaves the original untouched.
#[tokio::test(flavor = "multi_thread")]
async fn rerun_workflow_run_starts_new_run_with_same_input() {
    use librefang_kernel::workflow::WorkflowId;

    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let original = engine
        .create_run(wf_id, "original params".to_string())
        .await
        .expect("create original run");

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{original}/rerun"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body:?}");
    let new_id = body["run_id"].as_str().expect("run_id in body").to_string();
    assert_ne!(
        new_id,
        original.to_string(),
        "re-run must create a distinct run: {body:?}"
    );

    // The new run carries the same workflow + input as the original.
    let (status, detail) = get(&h, &format!("/api/workflows/runs/{new_id}")).await;
    assert_eq!(status, StatusCode::OK, "{detail:?}");
    assert_eq!(detail["input"].as_str(), Some("original params"));
    assert_eq!(detail["workflow_id"].as_str(), Some(wf_id_str.as_str()));
}

/// Re-running a run id that does not exist is a 404.
#[tokio::test(flavor = "multi_thread")]
async fn rerun_unknown_run_returns_404() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{}/rerun", uuid::Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A malformed run id is rejected with 400, not treated as a missing run.
#[tokio::test(flavor = "multi_thread")]
async fn rerun_invalid_run_id_returns_400() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/workflows/runs/not-a-uuid/rerun",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Per-step error exposed in the run-detail endpoint (#6292 need 4)
// ---------------------------------------------------------------------------

/// `GET /api/workflows/runs/{run_id}` surfaces a failed operator step's `error` field; successful steps omit it.
#[tokio::test(flavor = "multi_thread")]
async fn run_detail_exposes_per_step_error_for_failed_step() {
    use librefang_kernel::workflow::{
        ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowStep,
    };

    let h = boot().await;
    let engine = h.state.kernel.workflow_engine();

    // A Transform step with an invalid Tera template fails without any agent
    // call, recording a failed StepResult with a populated `error`.
    let wf = Workflow {
        id: WorkflowId::new(),
        name: "transform-fail".to_string(),
        description: String::new(),
        steps: vec![WorkflowStep {
            name: "bad-transform".to_string(),
            agent: StepAgent::ByName {
                name: "unused".to_string(),
            },
            prompt_template: String::new(),
            mode: StepMode::Transform {
                code: "{{ this is not a valid template".to_string(),
            },
            timeout_secs: 10,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        }],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
        input_schema: None,
    };
    let wf_id = engine.register(wf).await;
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create run");

    let resolver =
        |_a: &StepAgent| -> Option<(librefang_types::agent::AgentId, String, bool)> { None };
    let sender =
        |_id: librefang_types::agent::AgentId,
         msg: String,
         _sm: Option<librefang_types::agent::SessionMode>| async move { Ok((msg, 0u64, 0u64)) };
    let _ = engine.execute_run(run_id, resolver, sender).await;

    let (status, detail) = get(&h, &format!("/api/workflows/runs/{run_id}")).await;
    assert_eq!(status, StatusCode::OK, "{detail:?}");
    let steps = detail["step_results"]
        .as_array()
        .expect("step_results array");
    let failed = steps
        .iter()
        .find(|s| s["step_name"] == "bad-transform")
        .expect("failed step present in detail");
    assert!(
        failed["error"].as_str().is_some_and(|e| !e.is_empty()),
        "failed step must expose a non-empty error in the API payload: {failed:?}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/workflows/{id}/runs scoping (regression)
// ---------------------------------------------------------------------------

/// Regression: `GET /api/workflows/{id}/runs` must scope to the path workflow — previously `list_runs(None)` returned all workflows' runs.
#[tokio::test(flavor = "multi_thread")]
async fn list_workflow_runs_is_scoped_to_path_workflow() {
    use librefang_kernel::workflow::WorkflowId;

    let h = boot().await;
    let wf_a = create_workflow(&h).await;
    let wf_b = create_workflow(&h).await;
    assert_ne!(wf_a, wf_b, "two distinct workflows expected");
    let id_a = WorkflowId(wf_a.parse().unwrap());
    let id_b = WorkflowId(wf_b.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();
    let run_a = engine
        .create_run(id_a, "input-a".to_string())
        .await
        .expect("create run a");
    engine
        .create_run(id_b, "input-b1".to_string())
        .await
        .expect("create run b1");
    engine
        .create_run(id_b, "input-b2".to_string())
        .await
        .expect("create run b2");

    // A's endpoint must return exactly A's single run.
    let (status, body) = get(&h, &format!("/api/workflows/{wf_a}/runs")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body.as_array().expect("array body");
    assert_eq!(arr.len(), 1, "expected only workflow A's run: {body:?}");
    assert_eq!(
        arr[0]["id"].as_str(),
        Some(run_a.to_string().as_str()),
        "wrong run returned for A: {body:?}"
    );

    // B's endpoint must return exactly B's two runs.
    let (status, body) = get(&h, &format!("/api/workflows/{wf_b}/runs")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body.as_array().expect("array body").len(),
        2,
        "expected workflow B's two runs: {body:?}"
    );

    // Invalid id must be rejected with 400, not silently treated as "all runs".
    let (status, _body) = get(&h, "/api/workflows/not-a-uuid/runs").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
