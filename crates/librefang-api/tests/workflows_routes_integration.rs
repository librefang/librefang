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
//! - LLM-backed `POST /api/workflows/{id}/run` and agent-turn `POST /api/schedules/{id}/run` coverage — our test kernel has no model credentials.
//!   Manual schedule delivery is covered with a deterministic zero-step workflow.
//! - `POST /api/workflows/{id}/dry-run` agent-execution coverage — the
//!   step-context path walks into agent-registry lookups for agents we
//!   haven't registered, so `agent_found` is always false here. The
//!   prompt-resolution half (object-shaped `input` → `{{var}}` binding)
//!   *is* covered: it computes `resolved_prompt` without an agent. See
//!   `dry_run_binds_object_input_keys_to_template_vars`.
//! - `POST /api/triggers` creation requires a registered `AgentId` plus a
//!   `register_trigger_with_target` call into a fully-wired kernel; the
//!   negative-validation paths (missing agent_id / pattern, bad ids) are
//!   covered here, while the agent-backed success path and the per-agent
//!   cap → 400 path live in `trigger_workflow_test.rs` (which seeds a real
//!   agent via `spawn_agent`).

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use futures::Stream;
use librefang_api::routes::{self, AppState};
use librefang_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use tower::ServiceExt;

struct RecordingChannelAdapter {
    sent: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ChannelAdapter for RecordingChannelAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(Box::pin(futures::stream::empty()))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let ChannelContent::Text(text) = content {
            self.sent
                .lock()
                .expect("recording adapter lock")
                .push(format!("{}:{text}", user.platform_id));
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

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
async fn workflow_create_parses_per_step_session_mode() {
    // Regression: routes/workflows.rs previously hardcoded `session_mode: None`
    // at both POST and PATCH step-construction sites, so HTTP-supplied workflows
    // silently dropped the documented "per-step > manifest > kernel default"
    // resolution down to "manifest > default". This test pins all four cases —
    // explicit `new`, explicit `persistent`, absent (→ null), and malformed
    // (→ lenient null) — at the route boundary.
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "session-mode-mix",
            "steps": [
                {"name": "s_new",    "agent_id": agent_id, "session_mode": "new"},
                {"name": "s_persist","agent_id": agent_id, "session_mode": "persistent"},
                {"name": "s_absent", "agent_id": agent_id},
                {"name": "s_garbage","agent_id": agent_id, "session_mode": 42},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"]
        .as_str()
        .expect("workflow_id")
        .to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let steps = body["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 4);
    assert_eq!(
        steps[0]["session_mode"], "new",
        "explicit 'new' must round-trip"
    );
    assert_eq!(
        steps[1]["session_mode"], "persistent",
        "explicit 'persistent' must round-trip"
    );
    assert!(
        steps[2]["session_mode"].is_null(),
        "absent session_mode must serialize as null (fall through to manifest/default)"
    );
    assert!(
        steps[3]["session_mode"].is_null(),
        "malformed session_mode must be silently ignored at the boundary (lenient parse)"
    );
}

/// POST /api/workflows with a well-formed `input_schema` array must
/// accept it and GET /api/workflows/{id} must round-trip every declared
/// row verbatim (#4982 — gap 2 parameter discovery). Pins the route
/// boundary; the kernel-side resolution path is covered by
/// `workflow_describe_returns_explicit_input_schema` in the kernel
/// integration tests.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_accepts_input_schema_and_round_trips() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "with-schema",
            "description": "input_schema round-trip",
            "steps": [
                {"name": "draft", "agent_id": agent_id, "prompt": "Topic={{topic}}"}
            ],
            "input_schema": [
                {"name": "topic", "param_type": "string", "required": true, "description": "Article topic"},
                {"name": "cover", "param_type": "file",   "required": false}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"]
        .as_str()
        .expect("workflow_id")
        .to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let schema = body["input_schema"].as_array().expect("input_schema array");
    assert_eq!(schema.len(), 2, "both declared rows must survive");
    // Lookup by name — POST doesn't promise ordering at the route boundary.
    let by_name: std::collections::HashMap<&str, &serde_json::Value> = schema
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p))
        .collect();
    assert_eq!(by_name["topic"]["param_type"], "string");
    assert_eq!(by_name["topic"]["required"], true);
    assert_eq!(by_name["topic"]["description"], "Article topic");
    assert_eq!(by_name["cover"]["param_type"], "file");
    assert_eq!(by_name["cover"]["required"], false);
    // List-view advertises has_input_schema=true so the agent knows to
    // call workflow_describe before workflow_run.
    let (status, list_body) = get(&h, "/api/workflows").await;
    assert_eq!(status, StatusCode::OK);
    let row = list_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == wf_id)
        .expect("created workflow appears in list");
    assert_eq!(row["name"], "with-schema");
}

/// PUT /api/workflows/{id} replaces `input_schema` when an explicit
/// `input_schema` key is supplied (#4982 — gap 2). Pins the documented
/// "explicit key replaces; absent key preserves" PATCH-style semantics
/// of `parse_input_schema`.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_update_replaces_input_schema() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    // Seed with one schema row.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "to-update",
            "steps": [{"name": "s", "agent_id": agent_id, "prompt": "go"}],
            "input_schema": [
                {"name": "topic", "param_type": "string", "required": true}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    // PUT a different schema — must replace, not merge.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/workflows/{wf_id}"),
        Some(serde_json::json!({
            "input_schema": [
                {"name": "cover", "param_type": "image", "required": false}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let schema = body["input_schema"].as_array().expect("input_schema array");
    assert_eq!(
        schema.len(),
        1,
        "PUT replaces; old 'topic' row must be gone"
    );
    assert_eq!(schema[0]["name"], "cover");
    assert_eq!(schema[0]["param_type"], "image");
    assert_eq!(schema[0]["required"], false);
}

/// POST /api/workflows with a malformed `input_schema` row must skip the
/// bad row (lenient `parse_input_schema` policy — same shape as
/// `parse_step_session_mode`) and persist the well-formed rows. Returns
/// 201 rather than 4xx; the bad row simply doesn't appear in GET. Pins
/// the documented `parse_input_schema` behavior (#4982 — gap 2).
#[tokio::test(flavor = "multi_thread")]
async fn workflow_create_skips_malformed_input_schema_rows() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "partial-schema",
            "steps": [{"name": "s", "agent_id": agent_id, "prompt": "go"}],
            "input_schema": [
                {"name": "topic", "param_type": "string", "required": true},
                // Missing the required `name` field — must be skipped.
                {"param_type": "string", "required": true},
                {"name": "cover", "param_type": "file", "required": false},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (status, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let schema = body["input_schema"].as_array().expect("input_schema array");
    assert_eq!(
        schema.len(),
        2,
        "malformed row must be silently skipped, leaving the 2 well-formed rows"
    );
    let names: Vec<&str> = schema.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"topic"));
    assert!(names.contains(&"cover"));
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
async fn schedule_manual_run_delivers_workflow_output_to_channel_targets() {
    use chrono::Utc;
    use librefang_types::agent::AgentId;
    use librefang_types::scheduler::{
        CronAction, CronDelivery, CronDeliveryTarget, CronJob, CronJobId, CronSchedule,
    };

    let h = boot().await;
    let sent = Arc::new(Mutex::new(Vec::new()));
    h._state.kernel.channel_adapters_ref().insert(
        "telegram".to_string(),
        Arc::new(RecordingChannelAdapter { sent: sent.clone() }),
    );

    // An empty workflow is deterministic: its output is its input, so the route can be exercised without an LLM provider or live Telegram bot.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({"name": "manual-delivery", "steps": []})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let workflow_id = body["workflow_id"].as_str().expect("workflow id");

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: AgentId::new(),
        name: "manual telegram delivery".to_string(),
        enabled: true,
        schedule: CronSchedule::Every { every_secs: 3600 },
        action: CronAction::Workflow {
            workflow_id: workflow_id.to_string(),
            input: Some("scheduled hello".to_string()),
            timeout_secs: Some(30),
        },
        delivery: CronDelivery::None,
        delivery_targets: vec![CronDeliveryTarget::Channel {
            channel_type: "telegram".to_string(),
            recipient: "test-chat-id".to_string(),
            thread_id: None,
            account_id: None,
        }],
        peer_id: None,
        session_mode: None,
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };
    let job_id = h
        ._state
        .kernel
        .cron()
        .add_job(job, false)
        .expect("add schedule");

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/schedules/{job_id}/run"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["output"], "scheduled hello");
    assert_eq!(
        sent.lock().expect("recording adapter lock").as_slice(),
        ["test-chat-id:scheduled hello"],
        "manual schedule run must use the same delivery_targets fan-out as a timed fire"
    );
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
        peer_id: None,
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

/// Regression: a workflow whose step prompt references a named
/// placeholder (`{{challenge}}`) must resolve that placeholder from an
/// object-shaped run `input` (the brainstorm-template repro). Before the
/// fix the `/run` and `/dry-run` handlers only accepted `input` as a
/// string, so the per-parameter form value never reached
/// `seed_input_vars_from_json` and the entry agent saw the literal
/// `{{challenge}}` ("no challenge provided, cannot run").
///
/// `dry-run` is used as the probe because it computes `resolved_prompt`
/// through the exact same `seed_input_vars_from_json` + `expand_variables`
/// path a real run uses, but without needing LLM credentials. The step
/// prompt deliberately uses BOTH a named placeholder (`{{challenge}}`)
/// and the free-text `{{input}}` so the cases below also pin that a
/// parameterised workflow can still receive readable free-form context
/// via `{{input}}` instead of a JSON blob.
#[tokio::test(flavor = "multi_thread")]
async fn dry_run_binds_object_input_keys_to_template_vars() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "brainstorm-repro",
            "steps": [
                {"name": "ideate", "agent_id": agent_id,
                 "prompt": "Brainstorm: {{challenge}} | Context: {{input}}"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"]
        .as_str()
        .expect("workflow_id")
        .to_string();

    // Object input with a named key + a free-text `input` key: the named
    // key binds `{{challenge}}` and the string `input` key renders as
    // `{{input}}` (NOT a `{...}` JSON dump). This is the dashboard
    // parameter-form + additional-context shape.
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/{wf_id}/dry-run"),
        Some(serde_json::json!({
            "input": { "challenge": "reduce churn", "input": "q3 notes" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["steps"][0]["resolved_prompt"], "Brainstorm: reduce churn | Context: q3 notes",
        "named key binds {{{{challenge}}}} and the string `input` key \
         renders as {{{{input}}}} free-text, not a JSON blob: {body:?}"
    );

    // Object input WITHOUT a string `input` key: `{{challenge}}` still
    // binds; `{{input}}` falls back to the raw blob (the pre-existing
    // #4982 whole-input contract — agent `workflow_run` callers rely on
    // this, so it must stay unchanged).
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/{wf_id}/dry-run"),
        Some(serde_json::json!({ "input": { "challenge": "reduce churn" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let resolved = body["steps"][0]["resolved_prompt"]
        .as_str()
        .expect("resolved_prompt");
    assert!(
        resolved.starts_with("Brainstorm: reduce churn | Context: {"),
        "no `input` key → {{{{challenge}}}} binds, {{{{input}}}} is the raw \
         blob (unchanged #4982 contract): {resolved}"
    );

    // Legacy plain string: named placeholders never bind (a string is
    // the whole-blob `{{input}}`, not a per-key source). Pins the
    // string-vs-object boundary.
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/workflows/{wf_id}/dry-run"),
        Some(serde_json::json!({ "input": "reduce churn" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["steps"][0]["resolved_prompt"], "Brainstorm: {{challenge}} | Context: reduce churn",
        "a plain-string input must NOT bind named placeholders: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// input_schema oversize guard (issue: bulk-with-capacity-no-validate)
// ---------------------------------------------------------------------------

/// POST /api/workflows with an oversize `input_schema` array must still
/// succeed (the parser is lenient by design — log + truncate, same style
/// as malformed individual entries), but the persisted schema MUST be
/// capped at `MAX_INPUT_SCHEMA_PARAMS` (100). Without the cap, an
/// `"input_schema": [{}, {}, ...]` array within the 8 MiB body limit
/// would cause `Vec::with_capacity(arr.len())` to pre-allocate millions
/// of entries.
#[tokio::test(flavor = "multi_thread")]
async fn workflow_input_schema_oversize_is_truncated() {
    let h = boot().await;
    let agent_id = uuid::Uuid::new_v4().to_string();

    // 150 well-formed param entries — over the 100 cap.
    let oversized: Vec<serde_json::Value> = (0..150)
        .map(|i| {
            serde_json::json!({
                "name": format!("p{i}"),
                "param_type": "string",
                "required": false,
            })
        })
        .collect();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "oversize-schema",
            "description": "regression: oversize input_schema truncates",
            "steps": [
                {"name": "draft", "agent_id": agent_id, "prompt": "x"}
            ],
            "input_schema": oversized,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    let wf_id = body["workflow_id"].as_str().unwrap().to_string();

    let (_, body) = get(&h, &format!("/api/workflows/{wf_id}")).await;
    let schema = body["input_schema"].as_array().expect("input_schema array");
    assert!(
        schema.len() <= 100,
        "input_schema must be capped at MAX_INPUT_SCHEMA_PARAMS (100), got {}",
        schema.len(),
    );
}

// =============================================================================
// /api/schedules ↔ /api/cron/jobs field parity  (#6611)
// =============================================================================
//
// The two routes are deliberate alternate views over the same `CronJob` store: `/api/cron/jobs` serializes the struct whole, `/api/schedules` renders a flattened presentation.
// The flattened view had fallen behind on `peer_id` (which selects the `SenderContext.user_id` a fire runs under), `session_mode` (persistent-shared vs isolated-per-fire), and `delivery` (the primary output destination, still read at fire time alongside `delivery_targets`).
//
// `CronJob::session_mode` carries `skip_serializing_if = "Option::is_none"`, so an unset value is absent from the cron view rather than null — which is why the schedules view emits an explicit null instead, matching how it already renders `tz` / `last_run` / `next_run`.

/// Seed a cron job carrying every field the schedules view used to drop.
async fn seed_cron_job_with_routing_fields(h: &Harness) -> String {
    use chrono::Utc;
    use librefang_types::agent::{AgentId, SessionMode};
    use librefang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};

    let job = CronJob {
        id: CronJobId::new(),
        agent_id: AgentId::new(),
        name: "routing-fixture".to_string(),
        enabled: true,
        schedule: CronSchedule::Cron {
            expr: "0 9 * * 1-5".to_string(),
            tz: Some("UTC".to_string()),
        },
        action: CronAction::SystemEvent {
            text: "ping".to_string(),
        },
        delivery: CronDelivery::LastChannel,
        delivery_targets: Vec::new(),
        peer_id: Some("peer-42".to_string()),
        session_mode: Some(SessionMode::New),
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
    };
    h._state
        .kernel
        .cron()
        .add_job(job, false)
        .expect("seed cron add_job")
        .0
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_view_carries_the_same_routing_fields_as_cron_jobs() {
    let h = boot().await;
    let id = seed_cron_job_with_routing_fields(&h).await;

    let (status, schedules) = get(&h, "/api/schedules").await;
    assert_eq!(status, StatusCode::OK, "{schedules:?}");
    let schedule = schedules["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|s| s["id"].as_str() == Some(id.as_str()))
        .expect("seeded job must appear in /api/schedules");

    let (status, cron) = get(&h, "/api/cron/jobs").await;
    assert_eq!(status, StatusCode::OK, "{cron:?}");
    let job = cron["jobs"]
        .as_array()
        .expect("jobs array")
        .iter()
        .find(|j| j["id"].as_str() == Some(id.as_str()))
        .expect("seeded job must appear in /api/cron/jobs");

    for field in ["peer_id", "session_mode", "delivery"] {
        assert_eq!(
            schedule[field], job[field],
            "`{field}` must agree between /api/schedules and /api/cron/jobs: \
             schedule={schedule:?} job={job:?}"
        );
    }
    assert_eq!(
        schedule["peer_id"].as_str(),
        Some("peer-42"),
        "{schedule:?}"
    );
    assert_eq!(
        schedule["session_mode"].as_str(),
        Some("new"),
        "{schedule:?}"
    );
    assert_eq!(
        schedule["delivery"]["kind"].as_str(),
        Some("last_channel"),
        "{schedule:?}"
    );
}

/// `GET /api/schedules/{id}` carries the routing fields in its own right.
///
/// An earlier version of this test asserted only `detail == list row`. Both
/// sides render through `cron_job_to_schedule_json`, so dropping the fields
/// from the helper dropped them from both sides equally and the equality still
/// held — the test passed against the unfixed code. The values are therefore
/// pinned here directly, and the cross-view equality is kept as a second,
/// weaker assertion about which helper the detail route uses.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_detail_carries_the_same_routing_fields_as_the_list_row() {
    let h = boot().await;
    let id = seed_cron_job_with_routing_fields(&h).await;

    let (status, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{detail:?}");
    assert_eq!(detail["peer_id"].as_str(), Some("peer-42"), "{detail:?}");
    assert_eq!(detail["session_mode"].as_str(), Some("new"), "{detail:?}");
    assert_eq!(
        detail["delivery"]["kind"].as_str(),
        Some("last_channel"),
        "{detail:?}"
    );

    let (_, schedules) = get(&h, "/api/schedules").await;
    let row = schedules["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|s| s["id"].as_str() == Some(id.as_str()))
        .expect("seeded job must appear in /api/schedules");

    assert_eq!(
        &detail, row,
        "detail and list row render the same job through the same helper"
    );
}

/// A job that leaves `peer_id` / `session_mode` unset still carries both keys,
/// as explicit nulls. The reporter could not confirm the `session_mode` half of
/// this defect precisely because the cron view omits the key when unset, so a
/// stable key set is what makes "not configured" readable from the response.
#[tokio::test(flavor = "multi_thread")]
async fn schedules_view_renders_unset_routing_fields_as_null() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    let (status, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{detail:?}");
    assert!(
        detail.get("peer_id").is_some() && detail["peer_id"].is_null(),
        "peer_id must be present-and-null, not absent: {detail:?}"
    );
    assert!(
        detail.get("session_mode").is_some() && detail["session_mode"].is_null(),
        "session_mode must be present-and-null, not absent: {detail:?}"
    );
    assert_eq!(
        detail["delivery"]["kind"].as_str(),
        Some("none"),
        "{detail:?}"
    );
}

// =============================================================================
// POST/PUT /api/schedules — the write half of the routing fields  (#6611)
// =============================================================================

/// Create a workflow and return its id, so a schedule can be posted without a
/// registered agent (`create_schedule` accepts `workflow_id` *or* `agent_id`,
/// and this harness has no agent registry entries).
async fn seed_workflow(h: &Harness) -> String {
    let (status, body) = json_request(
        h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": "schedule-target",
            "steps": [{
                "name": "s1",
                "agent_id": uuid::Uuid::new_v4().to_string(),
                "prompt": "hi",
            }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    body["workflow_id"]
        .as_str()
        .expect("workflow_id present")
        .to_string()
}

/// `POST /api/schedules` persists `peer_id`, `session_mode`, and `delivery`.
///
/// All three used to be unsettable through this route: `peer_id` was hardcoded
/// to `None`, `delivery` to `CronDelivery::None`, and `session_mode` was parsed
/// with a `.ok()` that swallowed a misspelling into `None`. A caller had to
/// reach for `/api/cron/jobs` instead, on a route pair that is otherwise two
/// views of one store. The assertion reads the job back out of the scheduler
/// rather than trusting the create response, so an echo of the request body
/// cannot pass.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_persists_peer_id_session_mode_and_delivery() {
    let h = boot().await;
    let workflow_id = seed_workflow(&h).await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/schedules",
        Some(serde_json::json!({
            "name": "routing-write",
            "cron": "0 9 * * 1-5",
            "workflow_id": workflow_id,
            "peer_id": "peer-42",
            "session_mode": "new",
            "delivery": {"kind": "channel", "channel": "telegram", "to": "12345"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");

    // The response renders through the same helper the read half covers.
    assert_eq!(body["peer_id"].as_str(), Some("peer-42"), "{body:?}");
    assert_eq!(body["session_mode"].as_str(), Some("new"), "{body:?}");
    assert_eq!(
        body["delivery"]["kind"].as_str(),
        Some("channel"),
        "{body:?}"
    );

    // Stored state is what actually decides how the job fires.
    let id = body["id"].as_str().expect("id present");
    let job_id =
        librefang_types::scheduler::CronJobId(uuid::Uuid::parse_str(id).expect("id is a uuid"));
    let job = h
        ._state
        .kernel
        .cron()
        .get_job(job_id)
        .expect("created job must be in the scheduler");
    assert_eq!(job.peer_id.as_deref(), Some("peer-42"));
    assert_eq!(
        job.session_mode,
        Some(librefang_types::agent::SessionMode::New)
    );
    match &job.delivery {
        librefang_types::scheduler::CronDelivery::Channel { channel, to } => {
            assert_eq!(channel, "telegram");
            assert_eq!(to, "12345");
        }
        other => panic!("delivery must be the posted channel variant, got {other:?}"),
    }
}

/// Omitting the three fields keeps the historical defaults: no peer, no
/// session-mode override, and fire-and-forget delivery. `cron_create` (the
/// kernel-side tool path) defaults `delivery` to `LastChannel` instead, so this
/// pins that making the field settable did not import that default and start
/// delivering output an existing client never asked for.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_without_routing_fields_keeps_historical_defaults() {
    let h = boot().await;
    let workflow_id = seed_workflow(&h).await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/schedules",
        Some(serde_json::json!({
            "name": "routing-defaults",
            "cron": "0 9 * * 1-5",
            "workflow_id": workflow_id,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    assert!(body["peer_id"].is_null(), "{body:?}");
    assert!(body["session_mode"].is_null(), "{body:?}");
    assert_eq!(body["delivery"]["kind"].as_str(), Some("none"), "{body:?}");
}

/// A malformed value on any of the three is a 400, not a silent drop and not a
/// 500. The `session_mode` case is the regression: `.ok()` used to turn a typo
/// into `None`, so a caller asking for per-fire isolation got the shared
/// persistent session and a `201`.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_rejects_malformed_routing_fields() {
    let h = boot().await;
    let workflow_id = seed_workflow(&h).await;

    for (field, value, needle) in [
        (
            "session_mode",
            serde_json::json!("persistant"),
            "session_mode",
        ),
        ("peer_id", serde_json::json!(""), "peer_id"),
        ("peer_id", serde_json::json!(7), "peer_id"),
        (
            "delivery",
            serde_json::json!({"kind": "channel", "channel": "telegram"}),
            "delivery",
        ),
    ] {
        let mut req = serde_json::json!({
            "name": "routing-bad",
            "cron": "0 9 * * 1-5",
            "workflow_id": workflow_id,
        });
        req[field] = value.clone();
        let (status, body) = json_request(&h, Method::POST, "/api/schedules", Some(req)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{field}` = {value} must be a 400: {body:?}"
        );
        let message = body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or_default();
        assert!(
            message.contains(needle),
            "error must name the offending field `{needle}`: {body:?}"
        );
    }
}

/// An SSRF-blocked `delivery` webhook is a 400 as well. Before `delivery`
/// became settable this branch was unreachable from this route; once it is
/// settable, `add_job` rejects the host via `LibreFangError::InvalidInput`,
/// which `internal_scrub` alone would have reported as a 500 with the reason
/// scrubbed out — the caller could not tell a refused host from a server
/// fault. Mirrors the same split already present in `update_schedule` (#4732).
#[tokio::test(flavor = "multi_thread")]
async fn schedule_create_rejects_ssrf_delivery_webhook_with_400() {
    let h = boot().await;
    let workflow_id = seed_workflow(&h).await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/schedules",
        Some(serde_json::json!({
            "name": "routing-ssrf",
            "cron": "0 9 * * 1-5",
            "workflow_id": workflow_id,
            "delivery": {"kind": "webhook", "url": "http://169.254.169.254/latest/meta-data/"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    let (_, list) = get(&h, "/api/schedules").await;
    assert!(
        list["items"].as_array().expect("items array").is_empty(),
        "a rejected create must not leave a job behind: {list:?}"
    );
}

/// `PUT /api/schedules/{id}` patches `delivery`.
///
/// The kernel's `update_job` has always supported the field; this route just
/// never put it in the `updates` map, so the schedules view could show a
/// `delivery` it had no way to change.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_patches_delivery() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({
            "delivery": {"kind": "channel", "channel": "slack", "to": "C123"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(
        detail["delivery"]["kind"].as_str(),
        Some("channel"),
        "{detail:?}"
    );
    assert_eq!(detail["delivery"]["channel"].as_str(), Some("slack"));
    assert_eq!(detail["delivery"]["to"].as_str(), Some("C123"));
}

/// A malformed `delivery` on update is a 400, not the catch-all 404. The
/// kernel reports a serde failure as `LibreFangError::Internal`, which this
/// route's fall-through arm renders as "Schedule not found" — a status that
/// sends the caller looking for a missing id instead of at their payload.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_rejects_malformed_delivery_with_400() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({"delivery": {"kind": "channel", "channel": "slack"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(
        detail["delivery"]["kind"].as_str(),
        Some("none"),
        "rejected patch must leave delivery untouched: {detail:?}"
    );
}

/// Omitting `delivery` leaves the stored value alone — the same
/// null-vs-omitted contract `delivery_targets` documents.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_without_delivery_preserves_it() {
    let h = boot().await;
    let id = seed_cron_job_with_routing_fields(&h).await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({"name": "renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(detail["name"].as_str(), Some("renamed"), "{detail:?}");
    assert_eq!(
        detail["delivery"]["kind"].as_str(),
        Some("last_channel"),
        "{detail:?}"
    );
}

/// An attempt to *change* `peer_id` or `session_mode` on an existing schedule
/// is refused with a 400 that says why, instead of returning 200 for a patch
/// that never applied. `CronScheduler::update_job` has no branch for either
/// field, so accepting the request would reproduce the reported defect from
/// the other direction: the caller is told their change landed when the stored
/// job is untouched.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_refuses_to_change_peer_id_or_session_mode() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    for (field, value) in [
        ("peer_id", serde_json::json!("peer-99")),
        ("session_mode", serde_json::json!("new")),
    ] {
        let (status, body) = json_request(
            &h,
            Method::PUT,
            &format!("/api/schedules/{id}"),
            Some(serde_json::json!({field: value})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "changing `{field}` must not report success: {body:?}"
        );
        let message = body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or_default();
        assert!(
            message.contains(field),
            "error must name `{field}`: {body:?}"
        );
    }

    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert!(detail["peer_id"].is_null(), "{detail:?}");
    assert!(detail["session_mode"].is_null(), "{detail:?}");
}

/// Spelling out the default `session_mode` is a no-op, not a change.
///
/// A stored `None` and a requested `"persistent"` resolve identically —
/// `cron_fire_session_override` only diverges on `New` — so comparing the raw
/// `Option`s would refuse a caller who merely wrote the default out longhand.
/// The stored value is `None` far more often than `Some(Persistent)`, which
/// makes this the likely shape of a hand-written round trip.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_treats_explicit_persistent_as_no_change_against_unset() {
    let h = boot().await;
    let id = seed_cron_job(&h).await;

    // Precondition: the seeded job leaves `session_mode` unset.
    let (_, before) = get(&h, &format!("/api/schedules/{id}")).await;
    assert!(before["session_mode"].is_null(), "{before:?}");

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({"name": "spelled-out", "session_mode": "persistent"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "`persistent` against an unset value is the same effective mode: {body:?}"
    );

    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(detail["name"].as_str(), Some("spelled-out"), "{detail:?}");

    // `new` really is a change, so it is still refused.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({"session_mode": "new"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// The unknown-id case still answers 404, not the field-level 400.
///
/// The "cannot be changed" comparison reads the stored job first, so it has to
/// skip the check when there is no stored job — otherwise every `peer_id` sent
/// to a nonexistent schedule would report a field problem and send the caller
/// looking at their payload instead of at the id.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_unknown_id_with_peer_id_returns_404_not_400() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{unknown}"),
        Some(serde_json::json!({"peer_id": "peer-99", "session_mode": "new"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// Echoing the stored values back is a no-op, not a rejection: the read half
/// of #6611 puts both fields in every GET response, so a client that does
/// GET → mutate one field → PUT sends them unchanged and must not be blocked.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_accepts_unchanged_peer_id_and_session_mode() {
    let h = boot().await;
    let id = seed_cron_job_with_routing_fields(&h).await;

    let (_, before) = get(&h, &format!("/api/schedules/{id}")).await;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/schedules/{id}"),
        Some(serde_json::json!({
            "name": "echoed-back",
            "peer_id": before["peer_id"].clone(),
            "session_mode": before["session_mode"].clone(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (_, detail) = get(&h, &format!("/api/schedules/{id}")).await;
    assert_eq!(detail["name"].as_str(), Some("echoed-back"), "{detail:?}");
    assert_eq!(detail["peer_id"], before["peer_id"]);
    assert_eq!(detail["session_mode"], before["session_mode"]);
}
