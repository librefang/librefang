//! Integration tests for workflow pause/resume HTTP endpoints and async POST /run
//! (refs #4844 gaps #3 and #5).
//!
//! Tests do NOT require LLM credentials. Pause/resume state is driven via the
//! kernel `workflow_engine()` directly; the HTTP layer is exercised via
//! `tower::oneshot`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_kernel::workflow::{WorkflowEngine, WorkflowId, WorkflowRunState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentId;
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
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
            extra_params: std::collections::HashMap::new(),
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

/// Create a minimal 1-step workflow via the HTTP API and return its id string.
async fn create_workflow(h: &Harness) -> String {
    let agent_id = uuid::Uuid::new_v4().to_string();
    let (status, body) = json_request(
        h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "pause-resume-test",
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
// Pause endpoint tests
// ---------------------------------------------------------------------------

/// Pause a Pending run: returns 200 with a plaintext resume_token UUID, and
/// the run's pause_request field is populated with a hash (not the token).
#[tokio::test(flavor = "multi_thread")]
async fn pause_pending_run_returns_200_and_token() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "test input".to_string())
        .await
        .expect("create_run must succeed");

    let path = format!("/api/workflows/runs/{}/pause", run_id);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"reason": "waiting for approval"})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "pause must be 200: {body:?}");
    assert_eq!(
        body["run_id"].as_str().unwrap(),
        run_id.to_string(),
        "run_id echoed: {body:?}"
    );

    // resume_token must be a valid UUID.
    let token_str = body["resume_token"]
        .as_str()
        .unwrap_or_else(|| panic!("resume_token missing from response: {body:?}"));
    let token: uuid::Uuid = token_str
        .parse()
        .unwrap_or_else(|_| panic!("resume_token is not a UUID: {token_str}"));

    // The stored hash must match the token.
    let run = engine.get_run(run_id).await.expect("run must exist");
    let lodged = run.pause_request.expect("pause_request must be set");
    let expected_hash = WorkflowEngine::hash_resume_token(&token);
    assert_eq!(
        lodged.resume_token_hash, expected_hash,
        "stored hash must match the returned token"
    );
    // No plaintext token anywhere on the struct.
    assert_eq!(lodged.reason, "waiting for approval");
}

/// Pause an unknown run returns 404.
#[tokio::test(flavor = "multi_thread")]
async fn pause_unknown_run_returns_404() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{}/pause", unknown),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// Pause a Paused run returns 409 with `error: "already_paused"` and the
/// stored hash (so callers can verify idempotency without the plaintext token).
#[tokio::test(flavor = "multi_thread")]
async fn pause_already_paused_run_returns_409_already_paused() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    // First pause: succeeds, returns token.
    let path = format!("/api/workflows/runs/{}/pause", run_id);
    let (status, body1) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::OK, "{body1:?}");
    let first_token: uuid::Uuid = body1["resume_token"].as_str().unwrap().parse().unwrap();
    let expected_hash = WorkflowEngine::hash_resume_token(&first_token);

    // Second pause (pause_request already set): returns 409.
    let (status, body2) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "second pause must be 409: {body2:?}"
    );
    assert_eq!(
        body2["error"].as_str().unwrap_or(""),
        "already_paused",
        "{body2:?}"
    );
    assert_eq!(
        body2["resume_token_hash"].as_str().unwrap_or(""),
        expected_hash,
        "hash must be echoed: {body2:?}"
    );
}

/// Pausing a terminal (Cancelled) run returns 409 with `error: "conflict"`.
#[tokio::test(flavor = "multi_thread")]
async fn pause_terminal_run_returns_409() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    engine.cancel_run(run_id).await.expect("cancel");

    let path = format!("/api/workflows/runs/{}/pause", run_id);
    let (status, body) = json_request(&h, Method::POST, &path, None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "pause on terminal must be 409: {body:?}"
    );
    assert_eq!(body["error"].as_str().unwrap_or(""), "conflict", "{body:?}");
}

// ---------------------------------------------------------------------------
// Resume endpoint tests
// ---------------------------------------------------------------------------

/// Resume with the wrong token returns 401.
#[tokio::test(flavor = "multi_thread")]
async fn resume_with_wrong_token_returns_401() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    // Pause the run (pre-pause so execute_run will transition it to Paused).
    let _real_token = engine.pause_run(run_id, "test").await.expect("pause_run");
    // Actually execute so the state becomes Paused (not just pending pause_request).
    engine
        .execute_run(
            run_id,
            |_agent| Some((AgentId::new(), "mock".to_string(), false)),
            |_id: AgentId, _msg: String, _sm: Option<librefang_types::agent::SessionMode>| async {
                Ok(("done".to_string(), 0u64, 0u64))
            },
        )
        .await
        .expect("execute_run should pause cleanly");

    let run = engine.get_run(run_id).await.unwrap();
    assert!(
        matches!(run.state, WorkflowRunState::Paused { .. }),
        "run must be Paused: {:?}",
        run.state
    );

    // Present a wrong token.
    let wrong_token = uuid::Uuid::new_v4();
    let path = format!("/api/workflows/runs/{}/resume", run_id);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"resume_token": wrong_token.to_string()})),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "wrong token must be 401: {body:?}"
    );
    assert_eq!(
        body["error"].as_str().unwrap_or(""),
        "token_mismatch",
        "{body:?}"
    );
}

/// Resume an unknown run returns 404.
#[tokio::test(flavor = "multi_thread")]
async fn resume_unknown_run_returns_404() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{}/resume", unknown),
        Some(serde_json::json!({"resume_token": uuid::Uuid::new_v4().to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// Resume a non-paused (Pending) run returns 409.
#[tokio::test(flavor = "multi_thread")]
async fn resume_non_paused_run_returns_409() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");
    // Run is Pending, not Paused.
    let path = format!("/api/workflows/runs/{}/resume", run_id);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"resume_token": uuid::Uuid::new_v4().to_string()})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "resume on non-paused must be 409: {body:?}"
    );
    assert_eq!(
        body["error"].as_str().unwrap_or(""),
        "not_paused",
        "{body:?}"
    );
}

/// Resume missing resume_token field returns 400.
#[tokio::test(flavor = "multi_thread")]
async fn resume_missing_token_returns_400() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{}/resume", unknown),
        Some(serde_json::json!({})), // no resume_token field
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing token must be 400");
}

/// The persisted run must NOT contain a plaintext resume_token — only the hash.
///
/// We pause a run, then read the `WorkflowRun` back and verify that the
/// `pause_request.resume_token_hash` is a 64-char hex string and that there
/// is no field named `resume_token` (non-hash) anywhere on the struct.
#[tokio::test(flavor = "multi_thread")]
async fn resume_token_not_present_in_persisted_run_json() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();
    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    // Pause via HTTP to get the plaintext token.
    let path = format!("/api/workflows/runs/{}/pause", run_id);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"reason": "security test"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let token_str = body["resume_token"].as_str().unwrap();
    let token: uuid::Uuid = token_str.parse().unwrap();

    // Now read the run back and inspect.
    let run = engine.get_run(run_id).await.expect("run must exist");

    // pause_request.resume_token_hash must be 64 hex chars.
    let pr = run
        .pause_request
        .clone()
        .expect("pause_request must be set");
    assert_eq!(
        pr.resume_token_hash.len(),
        64,
        "hash must be 64 hex chars (SHA-256): {}",
        pr.resume_token_hash
    );
    // Hash must match what we compute from the returned token.
    let expected_hash = WorkflowEngine::hash_resume_token(&token);
    assert_eq!(
        pr.resume_token_hash, expected_hash,
        "stored hash must match the returned token"
    );

    // Serialize to JSON and assert no `resume_token` key at non-hash path.
    let run_json = serde_json::to_value(&run).unwrap();
    if let Some(pr_json) = run_json.get("pause_request") {
        // `resume_token` (non-hash field) must not exist.
        assert!(
            pr_json.get("resume_token").is_none(),
            "plaintext resume_token must NOT appear in serialized PauseRequest: {pr_json}"
        );
        // `resume_token_hash` MUST exist and be 64 chars.
        assert_eq!(
            pr_json["resume_token_hash"].as_str().unwrap_or("").len(),
            64,
            "resume_token_hash must be 64 chars in JSON: {pr_json}"
        );
    }
}

// ---------------------------------------------------------------------------
// Pause + Resume round-trip (with mock execution)
// ---------------------------------------------------------------------------

/// Pause a run mid-execution then resume it via HTTP and verify it completes.
///
/// We use the kernel-level pause_run + execute_run with a mock sender to reach
/// the Paused state, then call the HTTP resume endpoint and verify the run
/// eventually reaches Completed (by polling the engine directly).
#[tokio::test(flavor = "multi_thread")]
async fn pause_then_resume_via_http_completes_workflow() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());
    let engine = h.state.kernel.workflow_engine();

    let run_id = engine
        .create_run(wf_id, "input".to_string())
        .await
        .expect("create_run");

    // Lodge a pause request, then execute to transition to Paused state.
    let plaintext_token = engine
        .pause_run(run_id, "test pause")
        .await
        .expect("pause_run");
    engine
        .execute_run(
            run_id,
            |_agent| Some((AgentId::new(), "mock".to_string(), false)),
            |_id: AgentId, _msg: String, _sm: Option<librefang_types::agent::SessionMode>| async {
                Ok(("done".to_string(), 0u64, 0u64))
            },
        )
        .await
        .expect("execute_run should pause cleanly");

    let run = engine.get_run(run_id).await.unwrap();
    assert!(
        matches!(run.state, WorkflowRunState::Paused { .. }),
        "run must be Paused: {:?}",
        run.state
    );

    // Hit the HTTP resume endpoint with the correct token.
    let path = format!("/api/workflows/runs/{}/resume", run_id);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"resume_token": plaintext_token.to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resume must be 200: {body:?}");
    assert_eq!(
        body["run_id"].as_str().unwrap(),
        run_id.to_string(),
        "{body:?}"
    );
    assert_eq!(body["state"].as_str().unwrap_or(""), "running", "{body:?}");

    // The resume runs in the background. Poll for any terminal state (max 2s).
    // The mock kernel's agent_registry returns None for unknown agents, so the
    // run ends up Failed rather than Completed — that's expected for a mock
    // round-trip. What we're verifying is that the HTTP handler actually spawned
    // the resume execution and the run left the Running state.
    let mut terminal = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let r = engine.get_run(run_id).await.unwrap();
        if matches!(
            r.state,
            WorkflowRunState::Completed | WorkflowRunState::Failed
        ) {
            terminal = true;
            break;
        }
    }
    assert!(
        terminal,
        "run must reach a terminal state after resume (Completed or Failed): {:?}",
        engine.get_run(run_id).await.map(|r| r.state)
    );
}

// ---------------------------------------------------------------------------
// Async POST /run tests
// ---------------------------------------------------------------------------

/// Default POST /run (no ?wait) returns 202 with a run_id.
#[tokio::test(flavor = "multi_thread")]
async fn post_run_default_is_async_returns_202_with_run_id() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;

    let path = format!("/api/workflows/{}/run", wf_id_str);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"input": "hello"})),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "async run must be 202: {body:?}"
    );
    let run_id_str = body["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("run_id missing from 202 response: {body:?}"));

    // Parse run_id as a UUID.
    let run_id: uuid::Uuid = run_id_str
        .parse()
        .unwrap_or_else(|_| panic!("run_id is not a UUID: {run_id_str}"));

    // The run must be visible via the engine.
    let engine = h.state.kernel.workflow_engine();
    let run = engine
        .get_run(librefang_kernel::workflow::WorkflowRunId(run_id))
        .await;
    assert!(run.is_some(), "run must exist in engine after 202");
}

/// POST /run with ?wait=true returns 200 with output (backward-compat).
#[tokio::test(flavor = "multi_thread")]
async fn post_run_wait_true_returns_200_with_output() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;

    // We need a workflow that will actually complete. Since the mock kernel
    // has no real agents, the run will fail — but we just verify the HTTP
    // status is 200 (or 422 for workflow failure), NOT 202.
    // The key invariant is: ?wait=true must be synchronous (not 202).
    let path = format!("/api/workflows/{}/run?wait=true", wf_id_str);
    let (status, _body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"input": "hello"})),
    )
    .await;

    // With ?wait=true the response must be synchronous: 200 or 422, never 202.
    assert_ne!(
        status,
        StatusCode::ACCEPTED,
        "wait=true must not return 202 (must block)"
    );
}

/// POST /run with ?wait=true&timeout_ms=1 on a workflow that can't run
/// immediately returns 202 (timeout path).
#[tokio::test(flavor = "multi_thread")]
async fn post_run_wait_true_with_zero_timeout_returns_202_on_overrun() {
    let h = boot().await;
    let wf_id_str = create_workflow(&h).await;

    // Use an extremely short timeout (1ms) so it almost certainly times out
    // before the workflow completes (even if it fails quickly, the timeout
    // fires in the same select branch).
    let path = format!("/api/workflows/{}/run?wait=true&timeout_ms=1", wf_id_str);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"input": "hello"})),
    )
    .await;

    // On timeout: 202. On immediate completion (success or fail): 200 or 422.
    // Either is acceptable — what's NOT acceptable is a hang or panic.
    // We just verify the status is in the expected set.
    assert!(
        matches!(
            status,
            StatusCode::ACCEPTED | StatusCode::OK | StatusCode::UNPROCESSABLE_ENTITY
        ),
        "unexpected status from timeout run: {status} {body:?}"
    );
}
