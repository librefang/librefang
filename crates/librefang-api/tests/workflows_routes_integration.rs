//! Integration tests for the `/api/workflows`, `/api/triggers`, `/api/schedules`,
//! `/api/workflow-templates`, and `/api/cron/jobs` route families.
//!
//! Refs #3571 (workflows-domain slice). Mirrors the harness pattern from
//! `users_test.rs`: boot a real kernel against a tempdir-backed config and
//! dispatch through the actual `routes::workflows::router()` via
//! `tower::oneshot`.
//!
//! Coverage is intentionally limited to read endpoints + safe error paths
//! that don't require LLM credentials, network, or shared global state.
//! Mutating endpoints are exercised only when the kernel-side machinery
//! (workflow engine, cron scheduler, template registry) accepts payloads
//! without spinning up an agent or hitting an external service.
//!
//! Out of scope (skipped intentionally):
//! - `POST /api/workflows/{id}/run` and `POST /api/schedules/{id}/run` —
//!   actually invoke an LLM-backed agent loop, which our test kernel has no
//!   credentials for.
//! - `POST /api/workflows/{id}/dry-run` — same reason; the dry-run path
//!   instantiates step contexts that walk into agent-registry lookups for
//!   agents we haven't registered.
//! - `POST /api/triggers` — requires a registered `AgentId` plus a
//!   `register_trigger_with_target` call into a fully-wired kernel; the
//!   creation path is exercised indirectly via the negative-validation tests.
//!
//! These slots become testable once a fixture lands that registers a fake
//! agent + a no-op LLM driver. Tracked under #3571 follow-up.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
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
        _state: state,
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
            // Handlers that derive Json<...> still need a content-type even
            // when the body is empty `{}` — sending bare `null` would 415.
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
    // GET handlers don't read a JSON body; send no content-type to mirror
    // how curl would hit them in production.
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
// /api/workflows
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn workflows_list_starts_empty() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflows").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body["items"].as_array().expect("items array");
    assert!(
        arr.is_empty(),
        "fresh kernel must have no workflows: {body:?}"
    );
    assert_eq!(body["total"], 0, "{body:?}");
    assert_eq!(body["offset"], 0, "{body:?}");
    assert!(body["limit"].is_null(), "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_get_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflows/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflows/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid workflow ID"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_then_list_then_get_round_trips() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "demo",
            "description": "round-trip",
            "steps": [
                {"name": "s1", "agent_id": agent_id, "prompt": "hi {{input}}"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"]
        .as_str()
        .expect("workflow_id present")
        .to_string();
    assert!(uuid::Uuid::parse_str(&wf_id).is_ok(), "valid uuid: {wf_id}");

    // list now contains it
    let (status, body) = get(&h, "/api/workflows").await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["items"].as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(body["total"], 1, "{body:?}");
    assert_eq!(arr[0]["id"], wf_id);
    assert_eq!(arr[0]["name"], "demo");
    assert_eq!(arr[0]["steps"], 1);
    assert_eq!(arr[0]["run_count"], 0);
    assert!(arr[0]["success_rate"].is_null(), "no terminal runs yet");

    // get single
    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["id"], wf_id);
    assert_eq!(body["name"], "demo");
    let steps = body["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["name"], "s1");
    assert_eq!(steps[0]["prompt_template"], "hi {{input}}");

    // list runs is an array (empty for a never-run workflow)
    let (status, runs) = get(&h, &format!("/api/workflows/{wf_id}/runs")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(runs.as_array().unwrap().is_empty(), "{runs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_rejects_missing_steps() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({"name": "no-steps"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("'steps'"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_rejects_step_without_agent() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "bad",
            "steps": [{"name": "s1", "prompt": "hi"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("agent_id"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_update_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/workflows/00000000-0000-0000-0000-000000000000",
        Some(serde_json::json!({"name": "x", "steps": []})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_delete_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::DELETE, "/api/workflows/garbage", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_run_get_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = get(
        &h,
        "/api/workflows/runs/00000000-0000-0000-0000-000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_run_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflows/runs/not-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid run ID"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_save_as_template_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows/00000000-0000-0000-0000-000000000000/save-as-template",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

// ---------------------------------------------------------------------------
// /api/triggers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn triggers_list_starts_empty() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/triggers").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 0);
    assert!(body["triggers"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_get_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/triggers/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/triggers/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_create_rejects_missing_agent_id() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({"pattern": "task_posted"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("agent_id"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_create_rejects_invalid_agent_id() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({"agent_id": "not-uuid", "pattern": "task_posted"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid agent_id"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_create_rejects_missing_pattern() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({"agent_id": uuid::Uuid::new_v4().to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("pattern"),
        "{body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/schedules  (cron-job-backed)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn schedules_list_starts_empty() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/schedules").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 0);
    assert!(body["schedules"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/schedules/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid schedule ID"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_get_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/schedules/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_rejects_missing_name() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/schedules",
        Some(serde_json::json!({"cron": "* * * * *"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("'name'"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_rejects_missing_cron() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/schedules",
        Some(serde_json::json!({"name": "demo"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("'cron'"),
        "{body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/cron/jobs
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cron_jobs_list_starts_empty() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/cron/jobs").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 0);
    assert!(body["jobs"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_jobs_list_rejects_invalid_agent_id_filter() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/cron/jobs?agent_id=not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid agent_id"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_jobs_list_with_unknown_agent_id_is_empty() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();
    let (status, body) = get(&h, &format!("/api/cron/jobs?agent_id={unknown}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/cron/jobs/garbage").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_get_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/cron/jobs/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_status_invalid_id_returns_400() {
    let h = boot().await;
    let (status, _body) = get(&h, "/api/cron/jobs/garbage/status").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_delete_invalid_id_returns_400() {
    let h = boot().await;
    let (status, _) = json_request(&h, Method::DELETE, "/api/cron/jobs/garbage", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_delete_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/cron/jobs/00000000-0000-0000-0000-000000000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_toggle_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/cron/jobs/00000000-0000-0000-0000-000000000000/enable",
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

// ---------------------------------------------------------------------------
// /api/workflow-templates
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn workflow_templates_list_returns_array() {
    // The template registry may ship built-in templates; we don't assert
    // emptiness, only shape.
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflow-templates").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["templates"].is_array(), "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_template_get_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflow-templates/no-such-template").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "{body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_template_instantiate_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflow-templates/no-such-template/instantiate",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_templates_list_supports_query_filters() {
    // Free-text + category filters should return 200 with an array even
    // when nothing matches.
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflow-templates?q=zzzz-no-match&category=nope").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body["templates"].as_array().expect("array");
    assert!(arr.is_empty(), "filters should winnow to zero: {body:?}");
}
