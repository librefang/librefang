//! Integration tests for prompt version and experiment routes.
//!
//! These tests boot the real `librefang-api` router in multi-tenant mode and
//! exercise the `/api/agents/{agent_id}/prompts/*` and `/api/prompts/*`
//! endpoints end-to-end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct TestHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl TestHarness {
    async fn send(&self, req: Request<Body>) -> axum::response::Response {
        self.app.clone().oneshot(req).await.unwrap()
    }
}

async fn start_mt_router() -> TestHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    TestHarness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&body).expect("valid json")
}

fn json_request(
    method: axum::http::Method,
    uri: String,
    account_id: &str,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", account_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn empty_request(method: axum::http::Method, uri: String, account_id: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", account_id)
        .body(Body::empty())
        .expect("request")
}

fn values_array<'a>(value: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    if let Some(items) = value.as_array() {
        items.as_slice()
    } else {
        value
            .as_object()
            .and_then(|obj| obj.get(key))
            .and_then(|v| v.as_array())
            .map(|items| items.as_slice())
            .unwrap_or(&[])
    }
}

const TEST_MANIFEST_TEMPLATE: &str = r#"
name = "{name}"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

async fn spawn_agent(h: &TestHarness, tenant: &str, name: &str) -> String {
    let resp = h
        .send(json_request(
            axum::http::Method::POST,
            "/api/agents".to_string(),
            tenant,
            serde_json::json!({
                "manifest_toml": TEST_MANIFEST_TEMPLATE.replace("{name}", name),
            }),
        ))
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = read_json(resp).await;
    body["agent_id"].as_str().expect("agent id").to_string()
}

async fn create_prompt_version(
    h: &TestHarness,
    tenant: &str,
    agent_id: &str,
    system_prompt: &str,
) -> serde_json::Value {
    let resp = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/agents/{agent_id}/prompts/versions"),
            tenant,
            serde_json::json!({
                "system_prompt": system_prompt,
                "description": "integration test prompt version",
                "tools": ["search"],
                "variables": ["subject"]
            }),
        ))
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    read_json(resp).await
}

fn create_experiment_body(version_id: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "prompt-ab-test",
        "traffic_split": [100],
        "started_at": chrono::Utc::now().to_rfc3339(),
        "variants": [
            {
                "name": "control",
                "description": "baseline prompt",
                "prompt_version_id": version_id
            }
        ]
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_version_crud_and_cross_tenant_access_is_denied() {
    let h = start_mt_router().await;
    let tenant_a_agent = spawn_agent(&h, "tenant-a", "agent-alpha").await;
    let _tenant_b_agent = spawn_agent(&h, "tenant-b", "agent-beta").await;

    let created = create_prompt_version(&h, "tenant-a", &tenant_a_agent, "You are tenant A.").await;
    let version_id = created["id"].as_str().expect("version id").to_string();
    assert_eq!(created["agent_id"], tenant_a_agent);
    assert_eq!(created["system_prompt"], "You are tenant A.");

    let list_resp = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/agents/{tenant_a_agent}/prompts/versions"),
            "tenant-a",
        ))
        .await;
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_json = read_json(list_resp).await;
    let versions = values_array(&list_json, "versions");
    assert!(
        versions.iter().any(|v| v["id"] == version_id),
        "created prompt version should be listed for owning tenant"
    );

    let cross_tenant_get = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/versions/{version_id}"),
            "tenant-b",
        ))
        .await;
    assert_eq!(cross_tenant_get.status(), StatusCode::NOT_FOUND);

    let own_get = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/versions/{version_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(own_get.status(), StatusCode::OK);
    let own_get_json = read_json(own_get).await;
    assert_eq!(own_get_json["id"], version_id);
    assert_eq!(own_get_json["system_prompt"], "You are tenant A.");

    let activate = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/prompts/versions/{version_id}/activate"),
            "tenant-a",
            serde_json::json!({ "agent_id": tenant_a_agent }),
        ))
        .await;
    assert_eq!(activate.status(), StatusCode::OK);
    assert_eq!(read_json(activate).await["success"], true);

    let delete = h
        .send(empty_request(
            axum::http::Method::DELETE,
            format!("/api/prompts/versions/{version_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(delete.status(), StatusCode::OK);
    assert_eq!(read_json(delete).await["success"], true);

    let after_delete = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/versions/{version_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(after_delete.status(), StatusCode::OK);
    assert!(read_json(after_delete).await.is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_routes_reject_invalid_ids_and_agent_mismatch() {
    let h = start_mt_router().await;
    let tenant_a_agent = spawn_agent(&h, "tenant-a", "agent-alpha").await;
    let tenant_a_second_agent = spawn_agent(&h, "tenant-a", "agent-gamma").await;
    let created =
        create_prompt_version(&h, "tenant-a", &tenant_a_agent, "Route guard prompt.").await;
    let version_id = created["id"].as_str().expect("version id").to_string();

    let invalid_list = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/agents/not-a-uuid/prompts/versions".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(invalid_list.status(), StatusCode::BAD_REQUEST);

    let invalid_get = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/prompts/versions/not-a-uuid".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(invalid_get.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let invalid_activate = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/prompts/versions/{version_id}/activate"),
            "tenant-a",
            serde_json::json!({}),
        ))
        .await;
    assert_eq!(invalid_activate.status(), StatusCode::BAD_REQUEST);

    let mismatch_activate = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/prompts/versions/{version_id}/activate"),
            "tenant-a",
            serde_json::json!({ "agent_id": tenant_a_second_agent }),
        ))
        .await;
    assert_eq!(mismatch_activate.status(), StatusCode::BAD_REQUEST);
    let mismatch_json = read_json(mismatch_activate).await;
    assert!(mismatch_json["error"]
        .as_str()
        .unwrap_or_default()
        .contains("does not belong to agent"));

    let invalid_delete = h
        .send(empty_request(
            axum::http::Method::DELETE,
            "/api/prompts/versions/not-a-uuid".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(invalid_delete.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_experiment_lifecycle_and_cross_tenant_access_is_denied() {
    let h = start_mt_router().await;
    let tenant_a_agent = spawn_agent(&h, "tenant-a", "agent-alpha").await;
    let _tenant_b_agent = spawn_agent(&h, "tenant-b", "agent-beta").await;
    let version =
        create_prompt_version(&h, "tenant-a", &tenant_a_agent, "Experiment prompt.").await;
    let version_id = version["id"].as_str().expect("version id").to_string();

    let create_exp = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/agents/{tenant_a_agent}/prompts/experiments"),
            "tenant-a",
            create_experiment_body(&version_id),
        ))
        .await;
    let create_exp_status = create_exp.status();
    let create_exp_json = read_json(create_exp).await;
    assert_eq!(
        create_exp_status,
        StatusCode::OK,
        "create experiment failed: {create_exp_json}"
    );
    let exp_json = create_exp_json;
    let exp_id = exp_json["id"].as_str().expect("experiment id").to_string();
    assert_eq!(exp_json["agent_id"], tenant_a_agent);
    assert_eq!(exp_json["name"], "prompt-ab-test");

    let list_exp = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/agents/{tenant_a_agent}/prompts/experiments"),
            "tenant-a",
        ))
        .await;
    assert_eq!(list_exp.status(), StatusCode::OK);
    let list_exp_json = read_json(list_exp).await;
    let experiments = values_array(&list_exp_json, "experiments");
    assert!(
        experiments.iter().any(|e| e["id"] == exp_id),
        "created experiment should be listed for owning tenant"
    );

    let cross_tenant_get = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/experiments/{exp_id}"),
            "tenant-b",
        ))
        .await;
    assert_eq!(cross_tenant_get.status(), StatusCode::NOT_FOUND);

    let start = h
        .send(empty_request(
            axum::http::Method::POST,
            format!("/api/prompts/experiments/{exp_id}/start"),
            "tenant-a",
        ))
        .await;
    let start_status = start.status();
    let start_json = read_json(start).await;
    assert_eq!(
        start_status,
        StatusCode::OK,
        "start experiment failed: {start_json}"
    );
    assert_eq!(start_json["success"], true);

    let running = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/experiments/{exp_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(running.status(), StatusCode::OK);
    assert_eq!(read_json(running).await["status"], "running");

    let pause = h
        .send(empty_request(
            axum::http::Method::POST,
            format!("/api/prompts/experiments/{exp_id}/pause"),
            "tenant-a",
        ))
        .await;
    assert_eq!(pause.status(), StatusCode::OK);
    assert_eq!(read_json(pause).await["success"], true);

    let paused = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/experiments/{exp_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(paused.status(), StatusCode::OK);
    assert_eq!(read_json(paused).await["status"], "paused");

    let metrics = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/experiments/{exp_id}/metrics"),
            "tenant-a",
        ))
        .await;
    assert_eq!(metrics.status(), StatusCode::OK);
    let metrics_json = read_json(metrics).await;
    let metric_items = values_array(&metrics_json, "metrics");
    assert_eq!(metric_items.len(), 1);
    assert_eq!(metric_items[0]["variant_name"], "control");

    let complete = h
        .send(empty_request(
            axum::http::Method::POST,
            format!("/api/prompts/experiments/{exp_id}/complete"),
            "tenant-a",
        ))
        .await;
    assert_eq!(complete.status(), StatusCode::OK);
    assert_eq!(read_json(complete).await["success"], true);

    let completed = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/prompts/experiments/{exp_id}"),
            "tenant-a",
        ))
        .await;
    assert_eq!(completed.status(), StatusCode::OK);
    assert_eq!(read_json(completed).await["status"], "completed");

    let invalid_id = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/prompts/experiments/not-a-uuid".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(invalid_id.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
