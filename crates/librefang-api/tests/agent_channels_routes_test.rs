//! Integration tests for the `/api/agents/{id}/channels` route family.
//!
//! Refs #4961 — per-agent channel allowlist. Tests exercise the production
//! router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the
//! real auth middleware, route registration, and handler logic are in play.
//! No real LLM calls — every test is hermetic.
//!
//! Routes covered:
//!   GET /api/agents/{id}/channels   (default shape, populated allowlist)
//!   PUT /api/agents/{id}/channels   (set + read-back, clear, bad id 400,
//!                                    unknown agent 404)
//!
//! Run: cargo test -p librefang-api --test agent_channels_routes_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

const TEST_TOKEN: &str = "test-secret";

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// GET /api/agents/{id}/channels on a freshly spawned agent must return the
/// backward-compatible default: empty assigned list and mode = "all".
#[tokio::test(flavor = "multi_thread")]
async fn get_channels_default_shape() {
    let h = boot().await;
    let id = spawn_named(&h.state, "chan-default");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(
        body["assigned"],
        serde_json::json!([]),
        "fresh agent must have empty assigned list"
    );
    assert_eq!(body["mode"], "all", "empty assigned must yield mode=all");
    assert!(
        body["available"].is_array(),
        "available must be an array (may be empty when no sidecars configured)"
    );
}

/// PUT then GET round-trip: setting a non-empty allowlist is reflected on read-back.
#[tokio::test(flavor = "multi_thread")]
async fn put_channels_roundtrip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "chan-roundtrip");

    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": ["telegram", "discord"]}),
        ),
    )
    .await;

    assert_eq!(put_status, StatusCode::OK, "PUT body={put_body:?}");
    assert_eq!(put_body["status"], "ok");

    let (get_status, get_body) =
        send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    let assigned = get_body["assigned"].as_array().expect("assigned array");
    let mut names: Vec<&str> = assigned.iter().filter_map(|v| v.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["discord", "telegram"]);
    assert_eq!(get_body["mode"], "allowlist");
}

/// PUT an empty channels array must clear the allowlist and revert to mode=all.
#[tokio::test(flavor = "multi_thread")]
async fn put_channels_clear_allowlist() {
    let h = boot().await;
    let id = spawn_named(&h.state, "chan-clear");

    // First set a non-empty allowlist.
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": ["slack"]}),
        ),
    )
    .await;

    // Then clear it.
    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": []}),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK);

    let (get_status, get_body) =
        send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;
    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    assert_eq!(
        get_body["assigned"],
        serde_json::json!([]),
        "allowlist should be empty after clear"
    );
    assert_eq!(get_body["mode"], "all");
}

/// GET or PUT with a non-UUID agent ID must return 400.
#[tokio::test(flavor = "multi_thread")]
async fn channels_bad_agent_id_returns_400() {
    let h = boot().await;

    let (get_status, _) = send(h.app.clone(), get("/api/agents/not-a-uuid/channels")).await;
    assert_eq!(get_status, StatusCode::BAD_REQUEST, "GET must be 400");

    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            "/api/agents/not-a-uuid/channels",
            serde_json::json!({"channels": ["telegram"]}),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::BAD_REQUEST, "PUT must be 400");
}

/// GET with a valid UUID that doesn't exist must return 404.
/// PUT with a valid UUID that doesn't exist returns 400 (agent-not-found
/// error propagated through the kernel, consistent with set_agent_skills /
/// set_agent_mcp_servers which also return 400 for all kernel errors).
#[tokio::test(flavor = "multi_thread")]
async fn channels_unknown_agent_returns_error() {
    let h = boot().await;
    let unknown = AgentId::new();

    let (get_status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{unknown}/channels")),
    )
    .await;
    assert_eq!(get_status, StatusCode::NOT_FOUND, "GET must be 404");

    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unknown}/channels"),
            serde_json::json!({"channels": ["telegram"]}),
        ),
    )
    .await;
    assert_eq!(
        put_status,
        StatusCode::BAD_REQUEST,
        "PUT must be 400 (kernel error)"
    );
}
