//! Integration tests for UAR A2A protocol and discovery routes.
//!
//! Validates Phase 3 of the UAR integration plan: that the A2A endpoints
//! described in `crates/librefang-api/src/routes/uar.rs` are wired into the
//! main router, return the correct shapes, and remain unauthenticated even
//! when an API key is configured.
//!
//! Tests that require live LLM calls are intentionally omitted — the
//! `message/send` JSON-RPC method is exercised via tasks/get + tasks/cancel
//! roundtrips that do not invoke any provider.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct UarHarness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for UarHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn start_uar_harness(api_key: &str) -> UarHarness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
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

    UarHarness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    serde_json::from_slice(&bytes).expect("body should be valid JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn well_known_agent_card_is_unauthenticated_and_returns_card() {
    // Even with an api_key set, the well-known endpoint must remain public.
    // The instance-level card is served by `routes::network::a2a_agent_card`;
    // the UAR module reuses that endpoint instead of registering its own.
    let harness = start_uar_harness("secret-key").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let card = body_json(response).await;
    assert_eq!(card["name"], "LibreFang Agent OS");
    assert!(card["url"].as_str().unwrap().contains("/a2a"));
    // network::a2a_agent_card serializes with camelCase per A2A RC v1.0.
    assert_eq!(card["capabilities"]["stateTransitionHistory"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_agents_returns_artifacts_array_unauthenticated() {
    let harness = start_uar_harness("secret-key").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/uar/discovery/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert!(body.get("agents").and_then(|v| v.as_array()).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a2a_rejects_non_jsonrpc_requests() {
    let harness = start_uar_harness("").await;

    let payload = serde_json::json!({
        "jsonrpc": "1.0",
        "id": 1,
        "method": "message/send",
        "params": {}
    });

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body.get("error").is_some());
    assert_eq!(body["error"]["code"], -32600);
}

#[tokio::test(flavor = "multi_thread")]
async fn a2a_unknown_method_returns_method_not_found() {
    let harness = start_uar_harness("").await;

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "totally/made-up",
        "params": {}
    });

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 7);
    assert_eq!(body["error"]["code"], -32601);
}

#[tokio::test(flavor = "multi_thread")]
async fn a2a_tasks_get_unknown_returns_task_not_found() {
    let harness = start_uar_harness("").await;

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tasks/get",
        "params": { "id": "ghost-task-id" }
    });

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["error"]["code"], -32001);
}

#[tokio::test(flavor = "multi_thread")]
async fn a2a_tasks_cancel_unknown_returns_task_not_found() {
    let harness = start_uar_harness("").await;

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tasks/cancel",
        "params": { "id": "ghost-task-id" }
    });

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["error"]["code"], -32001);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_card_route_returns_400_for_invalid_id() {
    let harness = start_uar_harness("").await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/uar/discovery/agents/not-a-uuid/card")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_card_route_returns_404_for_missing_agent() {
    let harness = start_uar_harness("").await;
    let unknown_id = uuid::Uuid::new_v4();

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/uar/discovery/agents/{}/card", unknown_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
