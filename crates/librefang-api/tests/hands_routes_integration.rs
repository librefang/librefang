//! Integration tests for the `/api/hands/*` route family.
//!
//! Covers the hands HTTP surface registered in
//! `routes::skills::router()` (see `crates/librefang-api/src/routes/skills.rs`,
//! routes prefixed with `/hands`). The route family was previously
//! untested at the HTTP level (issue #3571: "~80% of registered HTTP
//! routes have no integration test"). This file is the hands-domain slice
//! of that work.
//!
//! Strategy
//! --------
//! We boot the real `server::build_router` against a freshly-booted kernel
//! backed by a temp-dir home, then drive it with `tower::ServiceExt::oneshot`.
//! All happy-path / error-path requests run with a configured `api_key` and
//! a matching `Authorization: Bearer …` header — `oneshot()` does not
//! attach `ConnectInfo`, so the loopback fast-path in the auth middleware
//! never fires; without a token, every non-public route returns 401 and
//! the handler is never reached. The public-allowlist contract for the
//! read routes (`GET /api/hands` and `GET /api/hands/active`) is already
//! covered by `tests/auth_public_allowlist.rs`, so we don't duplicate it
//! here.
//!
//! A single `mutating_hands_routes_require_auth_when_api_key_set` test
//! drops the Bearer header to assert the auth gate is wired up — i.e.
//! mutating routes are NOT silently in the public allowlist.
//!
//! No fixture hands are installed, so happy paths exercise only the empty /
//! 404 shapes — those are the most likely to silently regress (route
//! registration drift, panics on missing instances, etc.). Mutating
//! endpoints are exercised against unknown ids, asserting the documented
//! error contract (`400` / `404`) without touching shared global state.
//!
//! Run: `cargo test -p librefang-api --test hands_routes_integration`

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    _tmp: tempfile::TempDir,
    _state: Arc<AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network.
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
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
        _state: state,
    }
}

const TEST_API_KEY: &str = "test-secret-key";

/// Boot a router with auth configured and stash the bearer token on the
/// harness so every subsequent request through `send` / `json_request`
/// carries the right header. `oneshot()` does not attach `ConnectInfo`,
/// so without a token every non-public route returns 401 — see the
/// module-level docstring.
async fn boot_router_open() -> Harness {
    boot_router_with_api_key(TEST_API_KEY).await
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, _, bytes) = send(app, Method::GET, path, None, Some(TEST_API_KEY)).await;
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn json_request(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let (status, _, bytes) = send(app, method, path, body, Some(TEST_API_KEY)).await;
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

const NONEXISTENT_HAND: &str = "definitely-not-a-real-hand-zzz";
// Stable arbitrary UUID that no instance will ever match.
const UNKNOWN_INSTANCE: &str = "00000000-0000-4000-8000-000000000000";

// ---------------------------------------------------------------------------
// GET /api/hands — list all hand definitions
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_hands_returns_envelope_with_total_and_array() {
    let h = boot_router_open().await;
    let (status, body) = get_json(&h.app, "/api/hands").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.is_object(),
        "/api/hands must return a JSON object envelope, got: {body}"
    );
    assert!(
        body.get("items").map(|v| v.is_array()).unwrap_or(false),
        "missing/non-array `items` field (canonical PaginatedResponse #3842): {body}"
    );
    assert!(
        body.get("total").map(|v| v.is_u64()).unwrap_or(false),
        "missing/non-numeric `total` field: {body}"
    );
    assert_eq!(
        body.get("offset").and_then(|v| v.as_u64()),
        Some(0),
        "canonical envelope must include `offset`: {body}"
    );
    let arr_len = body["items"].as_array().unwrap().len();
    assert_eq!(
        body["total"].as_u64().unwrap(),
        arr_len as u64,
        "`total` must equal `items.len()`: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_hands_response_is_application_json() {
    let h = boot_router_open().await;
    let (status, headers, _) =
        send(&h.app, Method::GET, "/api/hands", None, Some(TEST_API_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("application/json"),
        "expected JSON content-type, got `{ct}`"
    );
}

// ---------------------------------------------------------------------------
// GET /api/hands/active — list active hand instances
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_active_hands_starts_empty() {
    let h = boot_router_open().await;
    let (status, body) = get_json(&h.app, "/api/hands/active").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["total"].as_u64(),
        Some(0),
        "fresh kernel must have no active hands: {body}"
    );
    assert_eq!(
        body["items"].as_array().map(|a| a.len()),
        Some(0),
        "fresh kernel must have no active hand instances: {body}"
    );
    assert_eq!(
        body.get("offset").and_then(|v| v.as_u64()),
        Some(0),
        "canonical envelope must include `offset` (#3842): {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/hands/{hand_id} — single definition
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_hand_unknown_returns_404() {
    let h = boot_router_open().await;
    let (status, body) = get_json(&h.app, &format!("/api/hands/{NONEXISTENT_HAND}")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    // ApiErrorResponse JSON body is { "error": "..." } — assert it's an
    // object with a populated message rather than pin the exact text.
    assert!(
        body.is_object(),
        "404 body must be a JSON object, got {body}"
    );
    let err = body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("message").and_then(|v| v.as_str()))
        .unwrap_or("");
    assert!(
        err.to_lowercase().contains("not found") || err.to_lowercase().contains("hand"),
        "404 body should describe the missing hand, got {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/hands/{hand_id}/manifest
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_hand_manifest_unknown_returns_404() {
    let h = boot_router_open().await;
    let (status, _) = get_json(&h.app, &format!("/api/hands/{NONEXISTENT_HAND}/manifest")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// GET /api/hands/{hand_id}/settings
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_hand_settings_unknown_returns_404() {
    let h = boot_router_open().await;
    let (status, _) = get_json(&h.app, &format!("/api/hands/{NONEXISTENT_HAND}/settings")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// PUT /api/hands/{hand_id}/settings — no active instance => 404
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_hand_settings_without_active_instance_returns_404() {
    let h = boot_router_open().await;
    let (status, body) = json_request(
        &h.app,
        Method::PUT,
        &format!("/api/hands/{NONEXISTENT_HAND}/settings"),
        Some(serde_json::json!({"foo": "bar"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_object(), "expected JSON error envelope, got {body}");
}

// ---------------------------------------------------------------------------
// POST /api/hands/install — input validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn install_hand_missing_toml_content_returns_400() {
    let h = boot_router_open().await;
    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/install",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let err = body["error"]["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or_default();
    assert!(
        err.to_lowercase().contains("toml_content"),
        "error should call out the missing toml_content field, got {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn install_hand_garbage_toml_returns_400() {
    let h = boot_router_open().await;
    let (status, _body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/install",
        Some(serde_json::json!({
            "toml_content": "this is not valid TOML for a hand <<>>",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Happy-path: `POST /api/hands/install` returns the canonical
/// `HandDefinition` body — not the legacy `{id, name, description, category}`
/// subset — so dashboard / SDK callers can `setQueryData` on the hands
/// list directly without a follow-up GET. Refs #3832.
#[tokio::test(flavor = "multi_thread")]
async fn install_hand_returns_canonical_hand_definition() {
    let h = boot_router_open().await;
    let toml = r#"
id = "uptime-watcher-test"
name = "Uptime Watcher"
description = "Watches uptime."
category = "data"

[routing]
aliases = ["uptime watcher"]

[agent]
name = "uptime-watcher-agent"
description = "Test hand agent"
system_prompt = "Test prompt"
"#;
    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/install",
        Some(serde_json::json!({
            "toml_content": toml,
            "skill_content": "# Test skill\n",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "install_hand body: {body}");
    assert_eq!(body["id"].as_str(), Some("uptime-watcher-test"), "{body}");
    assert_eq!(body["name"].as_str(), Some("Uptime Watcher"), "{body}");
    // Canonical fields beyond the legacy subset — these must be present so
    // a single round-trip is enough for the dashboard.
    assert!(
        body.get("agents").map(|v| v.is_object()).unwrap_or(false),
        "canonical HandDefinition must include `agents` map: {body}"
    );
    assert!(
        body.get("requires").map(|v| v.is_array()).unwrap_or(false),
        "canonical HandDefinition must include `requires` array: {body}"
    );
    assert!(
        body.get("settings").map(|v| v.is_array()).unwrap_or(false),
        "canonical HandDefinition must include `settings` array: {body}"
    );
    assert!(
        body.get("routing").map(|v| v.is_object()).unwrap_or(false),
        "canonical HandDefinition must include `routing` object: {body}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/hands/{hand_id}/secret — input validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_hand_secret_missing_key_returns_400() {
    let h = boot_router_open().await;
    let (status, body) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/{NONEXISTENT_HAND}/secret"),
        Some(serde_json::json!({"value": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.is_object(), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_hand_secret_unknown_hand_returns_400() {
    let h = boot_router_open().await;
    let (status, body) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/{NONEXISTENT_HAND}/secret"),
        Some(serde_json::json!({"key": "FAKE_VAR", "value": "x"})),
    )
    .await;
    // Handler reports "not a requirement of hand …" as 400, not 404.
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let err = body["error"]["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or_default();
    assert!(
        err.contains("requirement") || err.contains("hand"),
        "error should mention the unknown hand / requirement, got {body}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/hands/{hand_id}/activate — unknown hand
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn activate_unknown_hand_returns_400() {
    let h = boot_router_open().await;
    let (status, _) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/{NONEXISTENT_HAND}/activate"),
        Some(serde_json::json!({"config": {}})),
    )
    .await;
    // Handler maps any HandError to 400 via ApiErrorResponse::bad_request.
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Instance-scoped endpoints — unknown UUID
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pause_unknown_instance_returns_400() {
    let h = boot_router_open().await;
    let (status, _) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/instances/{UNKNOWN_INSTANCE}/pause"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_unknown_instance_returns_400() {
    let h = boot_router_open().await;
    let (status, _) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/instances/{UNKNOWN_INSTANCE}/resume"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn deactivate_unknown_instance_returns_400() {
    let h = boot_router_open().await;
    let (status, _) = json_request(
        &h.app,
        Method::DELETE,
        &format!("/api/hands/instances/{UNKNOWN_INSTANCE}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_stats_unknown_instance_returns_404() {
    let h = boot_router_open().await;
    let (status, body) = get_json(
        &h.app,
        &format!("/api/hands/instances/{UNKNOWN_INSTANCE}/stats"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_object(), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn hand_instance_status_unknown_returns_404() {
    let h = boot_router_open().await;
    let (status, body) = get_json(
        &h.app,
        &format!("/api/hands/instances/{UNKNOWN_INSTANCE}/status"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_object(), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_path_with_invalid_uuid_returns_400() {
    // Instance routes use `Path<uuid::Uuid>` extractors. A non-UUID segment
    // must be rejected before the handler runs (axum returns 400 for path
    // deserialization failures). This guards against a regression where a
    // route handler accidentally accepts non-UUID strings and panics.
    let h = boot_router_open().await;
    let (status, _) = get_json(&h.app, "/api/hands/instances/not-a-uuid/status").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// POST /api/hands/reload — happy path
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn reload_hands_returns_counts_envelope() {
    let h = boot_router_open().await;
    let (status, body) = json_request(&h.app, Method::POST, "/api/hands/reload", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str(), Some("ok"), "{body}");
    for field in ["added", "updated", "total"] {
        assert!(
            body.get(field).map(|v| v.is_u64()).unwrap_or(false),
            "missing/non-numeric `{field}` in reload response: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// POST /api/hands/{hand_id}/check-deps — unknown hand handling
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn check_hand_deps_unknown_returns_404() {
    let h = boot_router_open().await;
    let (status, _) = json_request(
        &h.app,
        Method::POST,
        &format!("/api/hands/{NONEXISTENT_HAND}/check-deps"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Auth allowlist regression: mutating routes must NOT be public
// ---------------------------------------------------------------------------

/// `/api/hands` and `/api/hands/active` are intentionally in
/// `PUBLIC_ROUTES_DASHBOARD_READS` (covered by `auth_public_allowlist.rs`).
/// The mutating routes below MUST stay behind the auth gate — a regression
/// that broadens the allowlist would let any unauthenticated caller install
/// or activate hands. This test asserts the negative.
#[tokio::test(flavor = "multi_thread")]
async fn mutating_hands_routes_require_auth_when_api_key_set() {
    let h = boot_router_with_api_key(TEST_API_KEY).await;

    let cases: &[(Method, &str, Option<serde_json::Value>)] = &[
        (
            Method::POST,
            "/api/hands/install",
            Some(serde_json::json!({})),
        ),
        (
            Method::POST,
            "/api/hands/some-hand/activate",
            Some(serde_json::json!({})),
        ),
        (Method::POST, "/api/hands/reload", None),
        (Method::DELETE, "/api/hands/some-hand", None),
    ];

    for (method, path, body) in cases {
        // Deliberately pass `None` as the bearer token to confirm the auth
        // middleware rejects the request before the handler sees it.
        let (status, _, _) = send(&h.app, method.clone(), path, body.clone(), None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must require auth (got {status})"
        );
    }
}
