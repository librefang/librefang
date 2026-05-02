//! Integration tests for the `/api/agents` route family.
//!
//! Refs #3571 — agents-domain slice. These tests exercise the production
//! router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the
//! real auth middleware, route registration, and handler logic are all in
//! play. No real LLM calls (provider is `ollama` with a fake model) — every
//! test is hermetic.
//!
//! Routes covered:
//!   GET   /api/agents              (list — empty filter + populated)
//!   GET   /api/agents/{id}         (happy path + invalid id 400 + unknown 404)
//!   PATCH /api/agents/{id}         (success, invalid payload, unknown 404,
//!                                   read-after-write via GET, auth gate 401)
//!
//! Run: cargo test -p librefang-api --test agents_routes_integration

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
// Harness — boots the production router with a configurable api_key.
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

async fn boot(api_key: &str) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network access.
    librefang_runtime::registry_sync::sync_registry(
        tmp.path(),
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

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
    state.kernel.spawn_agent(manifest).expect("spawn_agent")
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

/// Bearer token used by all authenticated test requests. Every harness
/// (except the explicit auth-gate test) boots with this api_key so the
/// production middleware accepts the requests as authenticated.
const TEST_TOKEN: &str = "test-secret";

fn get(path: &str) -> Request<Body> {
    get_with(path, Some(TEST_TOKEN))
}

fn get_with(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

fn patch_json(path: &str, body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/agents
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_default_assistant_only() {
    // The kernel auto-spawns a single default assistant on boot — so the
    // "empty user-spawn" baseline is exactly one entry. We further filter by
    // a unique q= to assert the empty case truly returns zero matches.
    let h = boot(TEST_TOKEN).await;

    let (status, body) = send(
        h.app.clone(),
        get("/api/agents?q=__definitely_no_such_agent__"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert!(
        items.is_empty(),
        "expected empty filter result, got {:?}",
        items
    );
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_spawned_agents() {
    let h = boot(TEST_TOKEN).await;
    let id_a = spawn_named(&h.state, "alpha-agent");
    let id_b = spawn_named(&h.state, "beta-agent");

    let (status, body) = send(h.app.clone(), get("/api/agents")).await;
    assert_eq!(status, StatusCode::OK);

    let items = body["items"].as_array().expect("items array");
    let ids: Vec<String> = items
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&id_a.to_string()), "missing alpha: {:?}", ids);
    assert!(ids.contains(&id_b.to_string()), "missing beta: {:?}", ids);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_rejects_invalid_sort_field() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents?sort=not_a_field")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_happy_path() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "lookup-target");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id.to_string());
    assert_eq!(body["name"], "lookup-target");
    assert!(body["model"].is_object());
    assert!(body["capabilities"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_agent_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", unknown))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "agent_not_found");
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_name_and_description() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "patch-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({
                "name": "renamed-agent",
                "description": "updated via PATCH"
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read-after-write — GET should reflect the new name + description.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed-agent");
    assert_eq!(body["description"], "updated via PATCH");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_mcp_servers_payload_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "bad-payload");

    // mcp_servers must be an array of strings; nested objects are rejected.
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"mcp_servers": [{"oops": true}]}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", unknown),
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            "/api/agents/not-a-uuid",
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Auth gate — PATCH is a mutation, NOT in PUBLIC_ROUTES_DASHBOARD_READS, so
// once an api_key is configured a non-loopback request without a Bearer
// token must be rejected with 401. (oneshot has no ConnectInfo, so the
// loopback fast-path does NOT apply — the request is treated as remote.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_without_token_returns_401_when_api_key_set() {
    let h = boot("test-secret").await;
    let id = spawn_named(&h.state, "auth-gated");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "should-not-apply"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Sanity: with the correct Bearer token the same request succeeds.
    let (status_ok, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "did-apply"}),
            Some("test-secret"),
        ),
    )
    .await;
    assert_eq!(status_ok, StatusCode::OK);
}
