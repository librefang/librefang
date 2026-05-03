//! Integration tests for the `/api/a2a/*` route family.
//!
//! Refs #3571 — covers the A2A domain slice of the broader "registered
//! HTTP routes have no integration test" gap. Boots the real
//! `server::build_router` so route registration, the auth middleware,
//! and handler wiring are all exercised end-to-end via `tower::oneshot`.
//!
//! Routes covered:
//!   - GET    /api/a2a/agents
//!   - GET    /api/a2a/agents/{id}
//!   - POST   /api/a2a/discover
//!   - POST   /api/a2a/send
//!   - GET    /api/a2a/tasks/{id}/status
//!   - POST   /api/a2a/agents/{id}/approve
//!
//! Mutating endpoints that initiate real outbound HTTP (`/discover`,
//! `/send`, `/tasks/{id}/status`) are covered only on their
//! validation / trust-gate / error paths — happy-path discovery and
//! send would require a live external A2A server and are intentionally
//! out of scope here.
//!
//! Run: cargo test -p librefang-api --test a2a_routes_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
    api_key: String,
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
        _tmp: tmp,
        state,
        api_key: api_key.to_string(),
    }
}

fn auth_header(h: &Harness) -> (String, String) {
    ("authorization".to_string(), format!("Bearer {}", h.api_key))
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    authed: bool,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if authed {
        let (k, v) = auth_header(h);
        builder = builder.header(k, v);
    }
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
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

// ---------------------------------------------------------------------------
// GET /api/a2a/agents
// ---------------------------------------------------------------------------

/// Empty kernel → empty trusted+pending list, 200 with the canonical
/// `PaginatedResponse{items,total,offset,limit}` envelope per #3842.
/// Confirms the route is wired and the dashboard-reads middleware lets it
/// through without auth.
#[tokio::test(flavor = "multi_thread")]
async fn list_external_agents_empty_returns_envelope() {
    let h = boot("").await;
    let (status, body) = send(&h, Method::GET, "/api/a2a/agents", None, false).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["total"], serde_json::json!(0));
    assert_eq!(body["offset"], serde_json::json!(0));
    assert!(body.get("limit").is_some(), "limit field present (null ok)");
    assert!(body["items"].is_array());
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
}

/// In default dev mode (no api_key configured), /api/a2a/agents must be
/// reachable without credentials — it's in PUBLIC_ROUTES_DASHBOARD_READS and
/// `require_auth_for_reads` defaults to false.
#[tokio::test(flavor = "multi_thread")]
async fn list_external_agents_open_in_no_auth_dev_mode() {
    let h = boot("").await;
    let (status, _) = send(&h, Method::GET, "/api/a2a/agents", None, false).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/a2a/agents should be reachable in no-api-key dev mode"
    );
}

// ---------------------------------------------------------------------------
// GET /api/a2a/agents/{id}
// ---------------------------------------------------------------------------

/// Unknown id (no agents registered) → 404 with structured error.
#[tokio::test(flavor = "multi_thread")]
async fn get_external_agent_unknown_id_returns_404() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/a2a/agents/does-not-exist",
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(
        body.get("error").is_some() || body.get("message").is_some(),
        "expected an error envelope, got {body}"
    );
}

/// /api/a2a/agents/{id} is NOT in any public allowlist → 401 without a token
/// when api_key is configured.
#[tokio::test(flavor = "multi_thread")]
async fn get_external_agent_requires_auth() {
    let h = boot("test-secret-key").await;
    let (status, _) = send(&h, Method::GET, "/api/a2a/agents/anything", None, false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/a2a/discover  (validation paths only — outbound HTTP not exercised)
// ---------------------------------------------------------------------------

/// Missing `url` field in body → 400 from the handler's pre-network validation.
#[tokio::test(flavor = "multi_thread")]
async fn discover_missing_url_returns_400() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/a2a/discover",
        Some(serde_json::json!({})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// Non-http(s) URL → canonicalize_a2a_url rejects, 400 before any network call.
#[tokio::test(flavor = "multi_thread")]
async fn discover_invalid_url_returns_400() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/a2a/discover",
        Some(serde_json::json!({"url": "not-a-real-url"})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// SSRF guard: URL pointing at localhost → rejected with 400 before any
/// outbound socket is opened. Confirms `is_url_safe_for_ssrf` is invoked.
#[tokio::test(flavor = "multi_thread")]
async fn discover_localhost_url_blocked_by_ssrf_guard() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/a2a/discover",
        Some(serde_json::json!({"url": "http://localhost:1/agent"})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

// ---------------------------------------------------------------------------
// POST /api/a2a/send  (validation + trust-gate paths only)
// ---------------------------------------------------------------------------

/// Missing `url` → 400.
#[tokio::test(flavor = "multi_thread")]
async fn send_missing_url_returns_400() {
    let h = boot("test-secret-key").await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/a2a/send",
        Some(serde_json::json!({"message": "hi"})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Missing `message` (URL present and well-formed but un-trusted) → handler
/// rejects either on the missing field or the trust gate; either way the
/// response is 400, never reaching outbound HTTP.
#[tokio::test(flavor = "multi_thread")]
async fn send_missing_message_or_untrusted_returns_400() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/a2a/send",
        Some(serde_json::json!({"url": "https://example.com/agent"})),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// Trust gate: even with a valid URL and message, an unapproved target is
/// rejected with 400 before any outbound request is made. This is the
/// security regression that #3786 introduced and that we must keep tested.
#[tokio::test(flavor = "multi_thread")]
async fn send_to_untrusted_url_blocked_by_trust_gate() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/a2a/send",
        Some(serde_json::json!({
            "url": "https://example.com/agent",
            "message": "hello"
        })),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    let err = body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("message").and_then(|v| v.as_str()))
        .unwrap_or_default();
    assert!(
        err.to_lowercase().contains("trusted") || err.to_lowercase().contains("approve"),
        "expected trust-gate message, got: {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/a2a/tasks/{id}/status  (validation paths only)
// ---------------------------------------------------------------------------

/// Missing `url` query param → 400 from the handler's pre-network check.
#[tokio::test(flavor = "multi_thread")]
async fn external_task_status_missing_url_returns_400() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/a2a/tasks/some-task-id/status",
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// Trust gate: querying status against an un-approved URL → 400 before any
/// outbound request. Mirrors the /send trust gate.
#[tokio::test(flavor = "multi_thread")]
async fn external_task_status_untrusted_url_blocked() {
    let h = boot("test-secret-key").await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/a2a/tasks/task-123/status?url=https%3A%2F%2Fexample.com%2Fagent",
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// /api/a2a/tasks/{id}/status is NOT publicly readable — must 401 without a
/// token. Regression guard against accidental allowlist additions.
#[tokio::test(flavor = "multi_thread")]
async fn external_task_status_requires_auth() {
    let h = boot("test-secret-key").await;
    let (status, _) = send(
        &h,
        Method::GET,
        "/api/a2a/tasks/task-123/status?url=https%3A%2F%2Fexample.com",
        None,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/a2a/agents/{id}/approve
// ---------------------------------------------------------------------------

/// Approving an unknown URL with no pending entry and no trusted entry
/// returns 404. Exercises the kernel-level lookup path.
#[tokio::test(flavor = "multi_thread")]
async fn approve_unknown_pending_returns_404() {
    let h = boot("test-secret-key").await;
    // URL-encoded since the path captures the URL as `{id}`.
    let url = "https%3A%2F%2Fexample.com%2Fagent";
    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/a2a/agents/{url}/approve"),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
}

/// Approve endpoint requires auth.
#[tokio::test(flavor = "multi_thread")]
async fn approve_requires_auth() {
    let h = boot("test-secret-key").await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/a2a/agents/anything/approve",
        None,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
