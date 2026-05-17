//! Integration tests for the HITL operator-step HTTP actions endpoint
//! (#5133 — `POST /api/workflows/runs/{run_id}/operator`).
//!
//! The workflow engine is driven directly to reach the operator-step
//! pause (mock `agent_resolver` / `send_message`, matching the kernel-only
//! pattern used by `workflow_pause_resume_test.rs`); the HTTP layer is
//! exercised via `tower::oneshot` against the real `workflows::router()`.
//! No LLM credentials are required.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_kernel::workflow::{
    ErrorMode, OperatorAction, OperatorTimeoutAction, StepAgent, StepMode, Workflow, WorkflowId,
    WorkflowRunId, WorkflowRunState, WorkflowStep,
};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, SessionMode};
use std::sync::Arc;
use tower::ServiceExt;

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
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
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

/// produce (Sequential, mock-resolved) → review (Operator, LAST step).
///
/// The operator step is intentionally the last step so an Approve / Edit
/// resolve drives the run straight to `Completed` with the resolved
/// operator output as the workflow output — without needing the mock
/// kernel to resolve a downstream agent (the mock registry has no real
/// agents, which would fail a trailing Sequential step and muddy the
/// assertion; see `workflow_pause_resume_test.rs`' note on the same mock
/// limitation). The Reject path still proves "no downstream step runs"
/// because the operator step records its own `_operator:operator` result
/// and the run goes Failed before any further step.
fn produce_then_operator(actions: Vec<OperatorAction>) -> Workflow {
    Workflow {
        id: WorkflowId::new(),
        name: "op-action-it".to_string(),
        description: "hitl http test".to_string(),
        steps: vec![
            WorkflowStep {
                name: "produce".to_string(),
                agent: StepAgent::ByName {
                    name: "producer".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "review".to_string(),
                agent: StepAgent::ByName {
                    name: "_op".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Operator {
                    notify: vec!["telegram:@op".to_string()],
                    actions,
                    timeout_secs: None,
                    timeout_action: OperatorTimeoutAction::Continue,
                },
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
        ],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
        input_schema: None,
    }
}

/// Register the workflow, create a run, and execute it to the operator
/// pause. Returns the paused run id.
async fn run_to_operator_pause(h: &Harness, wf: Workflow) -> WorkflowRunId {
    let engine = h.state.kernel.workflow_engine();
    let wf_id = engine.register(wf).await;
    let run_id = engine
        .create_run(wf_id, "seed".to_string())
        .await
        .expect("create_run");
    engine
        .execute_run(
            run_id,
            |_a: &StepAgent| Some((AgentId::new(), "mock".to_string(), false)),
            |_id: AgentId, prompt: String, _m: Option<SessionMode>| async move {
                // Producer sees the seed; downstream steps echo their input
                // so the test can assert what flowed through.
                if prompt.contains("seed") {
                    Ok(("ARTIFACT".to_string(), 1u64, 1u64))
                } else {
                    Ok((format!("consumed:{prompt}"), 1u64, 1u64))
                }
            },
        )
        .await
        .expect("execute_run pauses cleanly at the operator step");
    let run = engine.get_run(run_id).await.unwrap();
    assert!(
        matches!(run.state, WorkflowRunState::Paused { .. }),
        "must be paused at operator step, got {:?}",
        run.state
    );
    run_id
}

/// Poll the engine for a terminal state (max ~3s).
async fn wait_terminal(h: &Harness, run_id: WorkflowRunId) -> WorkflowRunState {
    let engine = h.state.kernel.workflow_engine();
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let st = engine.get_run(run_id).await.unwrap().state;
        if matches!(
            st,
            WorkflowRunState::Completed | WorkflowRunState::Failed | WorkflowRunState::Cancelled
        ) {
            return st;
        }
    }
    engine.get_run(run_id).await.unwrap().state
}

/// Approve via HTTP → 200, run resumes and reaches Completed with the
/// approved artifact as the workflow output.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_approve_resumes_and_completes() {
    let h = boot().await;
    let run_id = run_to_operator_pause(
        &h,
        produce_then_operator(vec![OperatorAction::Approve, OperatorAction::Reject]),
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "approve"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve must be 200: {body:?}");
    assert_eq!(body["state"].as_str().unwrap_or(""), "running", "{body:?}");

    let final_state = wait_terminal(&h, run_id).await;
    assert!(
        matches!(final_state, WorkflowRunState::Completed),
        "run must Complete after HTTP Approve; got {final_state:?}"
    );
    // Side effect: the operator step resolved to the producer's artifact,
    // which (being the last step) is the workflow output.
    let run = h
        .state
        .kernel
        .workflow_engine()
        .get_run(run_id)
        .await
        .unwrap();
    assert_eq!(
        run.output.as_deref(),
        Some("ARTIFACT"),
        "Approve must carry the artifact through as the final output; got {:?}",
        run.output
    );
}

/// Reject via HTTP → 200, run transitions to Failed; no further step runs.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_reject_fails_run() {
    let h = boot().await;
    let run_id = run_to_operator_pause(
        &h,
        produce_then_operator(vec![OperatorAction::Approve, OperatorAction::Reject]),
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "reject"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reject must be 200: {body:?}");

    let final_state = wait_terminal(&h, run_id).await;
    assert!(
        matches!(final_state, WorkflowRunState::Failed),
        "run must be Failed after HTTP Reject; got {final_state:?}"
    );
    let run = h
        .state
        .kernel
        .workflow_engine()
        .get_run(run_id)
        .await
        .unwrap();
    assert!(
        run.error.as_deref().unwrap_or("").contains("reject"),
        "Failed reason must mention reject; got {:?}",
        run.error
    );
    assert!(
        run.output.is_none(),
        "rejected run must have no workflow output; got {:?}",
        run.output
    );
}

/// Edit via HTTP → the operator payload (not the artifact) becomes the
/// operator step's output, which (last step) is the workflow output.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_edit_substitutes_payload() {
    let h = boot().await;
    let run_id = run_to_operator_pause(&h, produce_then_operator(vec![OperatorAction::Edit])).await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "edit", "payload": "EDITED"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "edit must be 200: {body:?}");

    let final_state = wait_terminal(&h, run_id).await;
    assert!(
        matches!(final_state, WorkflowRunState::Completed),
        "run must Complete after Edit; got {final_state:?}"
    );
    let run = h
        .state
        .kernel
        .workflow_engine()
        .get_run(run_id)
        .await
        .unwrap();
    assert_eq!(
        run.output.as_deref(),
        Some("EDITED"),
        "Edit must replace the artifact with the operator payload as the \
         final output; got {:?}",
        run.output
    );
}

/// Edit without a payload → 400 (synchronous validation).
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_edit_without_payload_is_400() {
    let h = boot().await;
    let run_id = run_to_operator_pause(&h, produce_then_operator(vec![OperatorAction::Edit])).await;
    let (status, _b) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "edit"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Run must still be Paused — a rejected request must not mutate state.
    assert!(h
        .state
        .kernel
        .workflow_engine()
        .get_run(run_id)
        .await
        .unwrap()
        .state
        .is_paused());
}

/// Unknown action → 400.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_unknown_action_is_400() {
    let h = boot().await;
    let run_id =
        run_to_operator_pause(&h, produce_then_operator(vec![OperatorAction::Approve])).await;
    let (status, _b) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "explode"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Operator action on an unknown run → 404.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_unknown_run_is_404() {
    let h = boot().await;
    let (status, _b) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{}/operator", uuid::Uuid::new_v4()),
        serde_json::json!({"action": "approve"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Operator action on a run that is NOT paused at an operator step → 409.
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_non_operator_pause_is_409() {
    let h = boot().await;
    let engine = h.state.kernel.workflow_engine();
    // A plain 1-step Sequential workflow paused via the generic pause API
    // is Paused but NOT at an operator step.
    let wf = Workflow {
        id: WorkflowId::new(),
        name: "plain".to_string(),
        description: String::new(),
        steps: vec![WorkflowStep {
            name: "s1".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 30,
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
        .create_run(wf_id, "in".to_string())
        .await
        .expect("create_run");
    let _t = engine.pause_run(run_id, "manual").await.expect("pause_run");
    engine
        .execute_run(
            run_id,
            |_a: &StepAgent| Some((AgentId::new(), "m".to_string(), false)),
            |_i: AgentId, _p: String, _m: Option<SessionMode>| async {
                Ok(("x".to_string(), 0u64, 0u64))
            },
        )
        .await
        .expect("pauses");

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "approve"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "non-operator pause must be 409: {body:?}"
    );
    assert_eq!(
        body["error"].as_str().unwrap_or(""),
        "not_operator_pause",
        "{body:?}"
    );
}

/// An action not authorised at the step → the resolve fails; the run
/// stays Paused (the request is accepted-then-rejected async, so we
/// assert the durable side effect: no state change).
#[tokio::test(flavor = "multi_thread")]
async fn operator_http_unauthorised_action_leaves_run_paused() {
    let h = boot().await;
    // Only Approve authorised; operator attempts Reject.
    let run_id =
        run_to_operator_pause(&h, produce_then_operator(vec![OperatorAction::Approve])).await;
    let (status, _b) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/runs/{run_id}/operator"),
        serde_json::json!({"action": "reject"}),
    )
    .await;
    // The endpoint accepts the well-formed request (200) and the
    // authorisation check fails inside the spawned resolve, leaving the
    // run Paused (no Failed/Completed transition).
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        h.state
            .kernel
            .workflow_engine()
            .get_run(run_id)
            .await
            .unwrap()
            .state
            .is_paused(),
        "run must remain Paused — Reject was not authorised at this step"
    );
}
