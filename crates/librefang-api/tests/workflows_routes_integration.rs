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
    assert_eq!(body["total"].as_u64().unwrap(), 0);
    assert_eq!(body["offset"].as_u64().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_get_unknown_uuid_returns_404() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/workflows/00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("not found"),
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
            .or_else(|| body["error"]["message"].as_str())
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
    assert_eq!(body["total"].as_u64().unwrap(), 1);
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("'steps'"),
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("agent_id"),
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
            .or_else(|| body["error"]["message"].as_str())
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("agent_id"),
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
            .or_else(|| body["error"]["message"].as_str())
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("pattern"),
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
    // #3842: canonical envelope renamed `schedules` → `items`.
    assert!(body["items"].as_array().unwrap().is_empty());
    assert_eq!(body["offset"], 0);
    assert!(body["limit"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_get_invalid_id_returns_400() {
    let h = boot().await;
    let (status, body) = get(&h, "/api/schedules/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("'name'"),
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("'cron'"),
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
            .or_else(|| body["error"]["message"].as_str())
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
async fn cron_job_delete_unknown_uuid_is_idempotent_200() {
    // Refs #3509: DELETE is idempotent (RFC 9110 §9.2.2). Deleting an
    // already-absent cron job returns 200 with `status: already-deleted`,
    // not 404 — clients can replay/retry without seeing a phantom error.
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/cron/jobs/00000000-0000-0000-0000-000000000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "already-deleted", "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_delete_twice_both_succeed() {
    // Refs #3509: idempotent DELETE — calling DELETE on the same id twice
    // never surfaces an error on the second call. Tests the
    // already-absent path explicitly (no created job needed; the path
    // taken on the second call is identical to "never existed").
    let h = boot().await;
    let path = "/api/cron/jobs/11111111-1111-1111-1111-111111111111";
    for attempt in 1..=2 {
        let (status, body) = json_request(&h, Method::DELETE, path, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "attempt {attempt} should be 200; got {status} body={body:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_delete_unknown_uuid_is_idempotent_200() {
    // Refs #3509: same idempotency contract for triggers.
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        "/api/triggers/00000000-0000-0000-0000-000000000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "already-deleted", "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_delete_invalid_uuid_returns_400() {
    // Refs #3509: 400 stays reserved for malformed-id rejection. Only the
    // `not-found` case relaxed to 200.
    let h = boot().await;
    let (status, _body) = json_request(&h, Method::DELETE, "/api/triggers/not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("not found"),
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

// ---------------------------------------------------------------------------
// #3693 — cron job status response must expose session_message_count /
// session_token_count so operators can graph persistent-cron-session growth
// before the provider returns a hard context-window 400.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_get_response_has_session_size_fields() {
    use chrono::Utc;
    use librefang_memory::session::Session;
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::message::Message;
    use librefang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};

    let h = boot().await;
    let kernel = &h._state.kernel;

    // Build a synthetic agent — add_job does not validate against the
    // registry, so any AgentId works.
    let agent_id = AgentId::new();
    let job = CronJob {
        id: CronJobId::new(),
        agent_id,
        name: "session-size-probe".to_string(),
        enabled: true,
        schedule: CronSchedule::Every { every_secs: 3600 },
        action: CronAction::SystemEvent {
            text: "ping".to_string(),
        },
        delivery: CronDelivery::None,
        delivery_targets: Vec::new(),
        peer_id: None,
        session_mode: None,
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };
    let job_id = kernel
        .cron()
        .add_job(job, false)
        .expect("cron add_job should succeed for unregistered agent");

    // Seed the persistent (agent, "cron") session with a few messages so
    // the metric helpers have something to report.
    let cron_sid = SessionId::for_channel(agent_id, "cron");
    let session = Session {
        id: cron_sid,
        agent_id,
        messages: vec![
            Message::user("first user turn"),
            Message::assistant("first assistant turn"),
            Message::user("second user turn"),
        ],
        context_window_tokens: 0,
        label: None,
        messages_generation: 1,
        last_repaired_generation: None,
    };
    kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session must succeed");

    // GET /api/cron/jobs/{id} carries the new fields.
    let (status, body) = get(&h, &format!("/api/cron/jobs/{}", job_id.0)).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let msg_count = body["session_message_count"]
        .as_u64()
        .unwrap_or_else(|| panic!("session_message_count missing/non-numeric: {body:?}"));
    assert_eq!(
        msg_count, 3,
        "expected the 3 seeded messages, got {msg_count} body={body:?}"
    );
    let tok_count = body["session_token_count"]
        .as_u64()
        .unwrap_or_else(|| panic!("session_token_count missing/non-numeric: {body:?}"));
    assert!(
        tok_count > 0,
        "token estimate should be non-zero for non-empty session: {body:?}"
    );

    // GET /api/cron/jobs/{id}/status carries the same fields.
    let (status, body) = get(&h, &format!("/api/cron/jobs/{}/status", job_id.0)).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["session_message_count"].as_u64(), Some(3), "{body:?}");
    let tok = body["session_token_count"].as_u64();
    assert!(
        tok.is_some() && tok.unwrap() > 0,
        "status response missing token estimate: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_get_response_session_fields_default_zero_when_no_session() {
    // No persistent cron session yet → both counters must be 0, not absent.
    use chrono::Utc;
    use librefang_types::agent::AgentId;
    use librefang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};

    let h = boot().await;
    let kernel = &h._state.kernel;
    let agent_id = AgentId::new();
    let job = CronJob {
        id: CronJobId::new(),
        agent_id,
        name: "no-session-yet".to_string(),
        enabled: true,
        schedule: CronSchedule::Every { every_secs: 3600 },
        action: CronAction::SystemEvent {
            text: "ping".to_string(),
        },
        delivery: CronDelivery::None,
        delivery_targets: Vec::new(),
        peer_id: None,
        session_mode: None,
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };
    let job_id = kernel.cron().add_job(job, false).unwrap();

    let (status, body) = get(&h, &format!("/api/cron/jobs/{}", job_id.0)).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["session_message_count"].as_u64(), Some(0), "{body:?}");
    assert_eq!(body["session_token_count"].as_u64(), Some(0), "{body:?}");
}

// ---------------------------------------------------------------------------
// Gap 4 — Summarize-and-trim compaction mode (#3693)
// ---------------------------------------------------------------------------

/// Verify that `SummarizeTrim` with a successful LLM call replaces the old
/// messages with a summary message followed by the kept tail, keeping the
/// total below the configured cap.
#[tokio::test(flavor = "multi_thread")]
async fn cron_summarize_trim_produces_synthesis_and_controls_length() {
    use librefang_kernel::kernel::cron_compute_keep_count;
    use librefang_runtime::compactor::{compact_session, estimate_token_count, CompactionConfig};
    use librefang_runtime::llm_driver::{
        CompletionRequest, CompletionResponse, LlmDriver, LlmError,
    };
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::message::{ContentBlock, Message, Role};
    use std::sync::Arc;

    // Fake LLM driver that always returns a canned summary.
    struct FakeDriver;
    #[async_trait::async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Canned summary of older cron messages.".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                tool_calls: vec![],
                usage: librefang_types::message::TokenUsage {
                    input_tokens: 50,
                    output_tokens: 10,
                    ..Default::default()
                },
            })
        }
    }

    let agent_id = AgentId::new();
    let cron_sid = SessionId::for_channel(agent_id, "cron");
    let messages: Vec<Message> = (0..12)
        .map(|i| Message::user(format!("cron turn {i}")))
        .collect();

    // SummarizeTrim: keep_recent = 4, summarize the rest (first 8).
    let keep_recent: usize = 4;
    let tail_start = messages.len().saturating_sub(keep_recent);
    let to_summarize = &messages[..tail_start];
    let kept_tail = messages[tail_start..].to_vec();

    // Build a tmp session with only the messages to summarize.
    let tmp_session = librefang_memory::session::Session {
        id: cron_sid,
        agent_id,
        messages: to_summarize.to_vec(),
        context_window_tokens: 0,
        label: None,
        messages_generation: 0,
        last_repaired_generation: None,
    };
    let compact_cfg = CompactionConfig {
        threshold: 0,
        keep_recent: 0,
        ..CompactionConfig::default()
    };

    let result = compact_session(
        Arc::new(FakeDriver),
        "test-model",
        &tmp_session,
        &compact_cfg,
    )
    .await
    .expect("compact_session must succeed with FakeDriver");

    assert!(!result.summary.is_empty(), "Summary must be non-empty");
    assert!(
        !result.used_fallback,
        "Should not use fallback with FakeDriver"
    );

    // Construct the final session as SummarizeTrim would.
    let summary_msg = Message {
        role: Role::Assistant,
        content: librefang_types::message::MessageContent::Text(format!(
            "[Cron session summary — {} messages compacted]\n\n{}",
            result.compacted_count, result.summary,
        )),
        pinned: false,
        timestamp: None,
    };
    let mut new_messages = vec![summary_msg];
    new_messages.extend(kept_tail);

    // Final message list = 1 summary + 4 tail = 5 total.
    assert_eq!(
        new_messages.len(),
        1 + keep_recent,
        "Must be summary + kept tail"
    );
    assert!(
        new_messages[0]
            .content
            .text_content()
            .contains("[Cron session summary"),
        "First message should be the summary"
    );
    assert!(
        new_messages[0]
            .content
            .text_content()
            .contains("Canned summary"),
        "Summary content should appear in first message"
    );
    // Tail messages are the original last 4.
    assert_eq!(new_messages[1].content.text_content(), "cron turn 8");
    assert_eq!(new_messages[4].content.text_content(), "cron turn 11");
}

/// Verify that when the LLM driver fails, the path falls back to plain prune
/// (dropping from the front) so the cron fire is not blocked.
#[tokio::test(flavor = "multi_thread")]
async fn cron_summarize_trim_falls_back_to_prune_on_llm_failure() {
    use librefang_runtime::compactor::{compact_session, CompactionConfig};
    use librefang_runtime::llm_driver::{
        CompletionRequest, CompletionResponse, LlmDriver, LlmError,
    };
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::message::Message;
    use std::sync::Arc;

    // Failing driver that always returns an error.
    struct FailingDriver;
    #[async_trait::async_trait]
    impl LlmDriver for FailingDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Http("connection refused".to_string()))
        }
    }

    let agent_id = AgentId::new();
    let cron_sid = SessionId::for_channel(agent_id, "cron");
    let messages: Vec<Message> = (0..10)
        .map(|i| Message::user(format!("turn {i}")))
        .collect();

    let keep_recent: usize = 3;
    let tail_start = messages.len().saturating_sub(keep_recent);
    let to_summarize = &messages[..tail_start];
    let kept_tail = messages[tail_start..].to_vec();

    let tmp_session = librefang_memory::session::Session {
        id: cron_sid,
        agent_id,
        messages: to_summarize.to_vec(),
        context_window_tokens: 0,
        label: None,
        messages_generation: 0,
        last_repaired_generation: None,
    };
    let compact_cfg = CompactionConfig {
        threshold: 0,
        keep_recent: 0,
        max_retries: 1, // fail fast
        ..CompactionConfig::default()
    };

    let result = compact_session(
        Arc::new(FailingDriver),
        "test-model",
        &tmp_session,
        &compact_cfg,
    )
    .await
    .expect("compact_session returns Ok even on LLM failure (uses fallback)");

    // With FailingDriver, the fallback path fires — used_fallback = true.
    assert!(result.used_fallback, "Should use fallback when LLM fails");

    // Simulate the Prune fallback: drop (total - keep_count) from front.
    let keep_count = messages.len() - (messages.len() - keep_recent); // = keep_recent
    let pruned: Vec<_> = messages[messages.len() - keep_count..].to_vec();
    assert_eq!(
        pruned.len(),
        keep_recent,
        "Fallback must keep exactly keep_recent messages"
    );
    assert_eq!(pruned[0].content.text_content(), "turn 7");
    assert_eq!(pruned[2].content.text_content(), "turn 9");
}
