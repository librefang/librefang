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
        model_override: None,
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

// =============================================================================
// SSRF coverage on PUT /api/cron/jobs/{id}  (#4732)
// =============================================================================
//
// `add_job` validates webhook hosts at create-time, but `update_job` and
// `set_delivery_targets` historically skipped that check — letting an
// authenticated client install a webhook pointing at the daemon itself,
// RFC 1918 space, or cloud-metadata services by routing through the PUT
// path. Validation now runs on every mutation surface; these tests pin
// the wire-level behaviour so a future refactor can't silently regress
// the boundary.

/// Helper: seed a cron job directly via the kernel and return its id as
/// a UUID-string suitable for the `/api/cron/jobs/{id}` path.
async fn seed_cron_job(h: &Harness) -> String {
    use chrono::Utc;
    use librefang_types::agent::AgentId;
    use librefang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: AgentId::new(),
        name: "ssrf-fixture".to_string(),
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
    let id = h
        ._state
        .kernel
        .cron()
        .add_job(job, false)
        .expect("seed cron add_job");
    id.0.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_update_rejects_ssrf_webhook_in_delivery() {
    use librefang_types::scheduler::{CronDelivery, CronJobId};

    let h = boot().await;
    let id = seed_cron_job(&h).await;
    let job_id = id.parse::<uuid::Uuid>().map(CronJobId).unwrap();

    // Link-local cloud-metadata IP — pre-#4732 update path accepted it.
    let body = serde_json::json!({
        "delivery": {"kind": "webhook", "url": "http://169.254.169.254/latest/meta-data/"}
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/cron/jobs/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "must be 400, not 404 (#4732 mapping): {response:?}"
    );

    // State invariant (#4739 review): rejected update must not partially
    // overwrite `delivery`. Seed sets `CronDelivery::None`.
    let job = h._state.kernel.cron().get_job(job_id).expect("job exists");
    assert!(
        matches!(job.delivery, CronDelivery::None),
        "delivery must remain None after rejection, got {:?}",
        job.delivery
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_update_rejects_ssrf_webhook_in_delivery_targets() {
    use librefang_types::scheduler::CronJobId;

    let h = boot().await;
    let id = seed_cron_job(&h).await;
    let job_id = id.parse::<uuid::Uuid>().map(CronJobId).unwrap();

    // Hex-form loopback — `0x7f000001` == `127.0.0.1`. The pre-#4732
    // string-prefix logic missed numeric IPv4 forms entirely.
    let body = serde_json::json!({
        "delivery_targets": [
            {"type": "webhook", "url": "http://0x7f000001/hook"}
        ]
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/cron/jobs/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "hex-form loopback must be rejected: {response:?}"
    );

    // State invariant (#4739 review): targets must remain empty.
    let job = h._state.kernel.cron().get_job(job_id).expect("job exists");
    assert!(
        job.delivery_targets.is_empty(),
        "delivery_targets must remain empty after rejection, got {:?}",
        job.delivery_targets
    );
}

/// Two-phase mutation guarantee at the wire level (#4739 review):
/// a request mixing a valid `delivery` and an SSRF-laden
/// `delivery_targets` must reject as 400 AND must not smuggle the
/// (in-isolation valid) `delivery` change into stored state.
#[tokio::test(flavor = "multi_thread")]
async fn cron_job_update_partial_mutation_is_atomic() {
    use librefang_types::scheduler::{CronDelivery, CronJobId};

    let h = boot().await;
    let id = seed_cron_job(&h).await;
    let job_id = id.parse::<uuid::Uuid>().map(CronJobId).unwrap();

    let body = serde_json::json!({
        "delivery": {"kind": "webhook", "url": "https://example.com/hook"},
        "delivery_targets": [
            {"type": "webhook", "url": "http://0x7f000001/hook"}
        ]
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/cron/jobs/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mixed valid+SSRF must reject: {response:?}"
    );

    let job = h._state.kernel.cron().get_job(job_id).expect("job exists");
    assert!(
        matches!(job.delivery, CronDelivery::None),
        "valid `delivery` must NOT be smuggled in when later phase fails, got {:?}",
        job.delivery
    );
    assert!(
        job.delivery_targets.is_empty(),
        "delivery_targets must remain empty, got {:?}",
        job.delivery_targets
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_update_rejects_v4_mapped_v6_loopback_in_delivery_targets() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    // IPv4-mapped IPv6 — bracketed `[::ffff:127.0.0.1]` resolves
    // (transparently to most syscalls) to plain 127.0.0.1.
    let body = serde_json::json!({
        "delivery_targets": [
            {"type": "webhook", "url": "http://[::ffff:127.0.0.1]/hook"}
        ]
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/cron/jobs/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "IPv4-mapped IPv6 loopback must be rejected: {response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_job_update_accepts_public_webhook_in_delivery_targets() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    // Sanity check: a public-looking https webhook still succeeds.
    let body = serde_json::json!({
        "delivery_targets": [
            {"type": "webhook", "url": "https://example.com/hook"}
        ]
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/cron/jobs/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "public webhook must still be accepted: {response:?}"
    );
}

// `/api/schedules/{id}` and `/api/cron/jobs/{id}` are different routes
// that ultimately funnel into the same `CronScheduler::update_job` path,
// so both gained the `InvalidInput → 400` mapping in #4732. Without a
// test on this route the mapping is unverified — a future refactor that
// drops the arm would silently regress SSRF rejection back to a 404
// "Schedule not found" on this surface only.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_rejects_ssrf_webhook_in_delivery_targets() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    let body = serde_json::json!({
        "delivery_targets": [
            {"type": "webhook", "url": "http://169.254.169.254/latest/meta-data/"}
        ]
    });
    let (status, response) =
        json_request(&h, Method::PUT, &format!("/api/schedules/{id}"), Some(body)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "must be 400, not 404: SSRF rejection on /api/schedules/{{id}} \
         must surface as bad request, not as a missing-resource error: {response:?}"
    );
}
