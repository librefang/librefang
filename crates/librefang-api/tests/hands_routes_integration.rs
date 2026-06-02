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
    boot_router_with_config(api_key, Vec::new()).await
}

/// Boot a router with auth + an explicit hands SSRF allowlist.
///
/// The marketplace-install tests stand up a mock registry on `127.0.0.1`,
/// which the install handler's `check_ssrf` guard now rejects unless the
/// loopback host is exempt. Threading `registry_allowed_hosts` here is how
/// those tests keep their loopback mock reachable; pass an empty list to
/// exercise the default public-only policy.
async fn boot_router_with_config(api_key: &str, registry_allowed_hosts: Vec<String>) -> Harness {
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
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        hands: librefang_types::config::HandsConfig {
            registry_allowed_hosts,
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

/// Boot a router whose hands SSRF allowlist exempts the loopback mock
/// registry. Used by the marketplace-install tests that bind their fake
/// HandsHub on `127.0.0.1`.
async fn boot_router_allowing_loopback() -> Harness {
    boot_router_with_config(TEST_API_KEY, vec!["127.0.0.1".to_string()]).await
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

// ---------------------------------------------------------------------------
// POST /api/hands/marketplace/install — install a hand from a remote registry
//
// We never touch the network: a local axum listener stands in for the
// HandsHub registry, serving the two endpoints `HandsHubClient` calls —
// `GET /api/v1/index` and `GET /api/v1/hands/{id}/bundle`. The index entry
// advertises the real SHA-256 of the served bundle bytes so the installer's
// checksum gate passes; the second test corrupts that digest to assert the
// gate actually fails the install.
// ---------------------------------------------------------------------------

const MARKETPLACE_HAND_TOML: &str = r#"
id = "remote-uptime"
name = "Remote Uptime"
description = "Installed from the marketplace."
category = "data"

[routing]
aliases = []

[agent]
name = "remote-uptime-agent"
description = "Test hand agent"
system_prompt = "Test prompt"
"#;

/// Build the exact bundle bytes the mock registry serves for `remote-uptime`,
/// together with their SHA-256 hex digest. The digest is what the index
/// entry advertises, so the two must be derived from the same bytes.
fn marketplace_bundle_bytes_and_sha() -> (Vec<u8>, String) {
    use sha2::{Digest, Sha256};
    let bundle = serde_json::json!({
        "toml": MARKETPLACE_HAND_TOML,
        "skill": "# Remote skill\n",
    });
    let bytes = serde_json::to_vec(&bundle).expect("serialize bundle");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha = hex::encode(hasher.finalize());
    (bytes, sha)
}

/// Spawn a mock HandsHub registry. `advertised_sha` is the digest placed in
/// the index entry — pass the real digest for the happy path, a wrong one to
/// exercise the checksum-mismatch rejection. Returns the base URL
/// (`http://127.0.0.1:PORT/api/v1`) and the server task handle.
async fn spawn_mock_registry(advertised_sha: String) -> (String, tokio::task::JoinHandle<()>) {
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    let (bundle_bytes, _) = marketplace_bundle_bytes_and_sha();
    let state = Arc::new((bundle_bytes, advertised_sha));

    async fn index_handler(State(s): State<Arc<(Vec<u8>, String)>>) -> impl IntoResponse {
        let index = serde_json::json!({
            "hands": [
                {
                    "id": "remote-uptime",
                    "name": "Remote Uptime",
                    "description": "Installed from the marketplace.",
                    "category": "data",
                    "version": "1.0.0",
                    "expected_sha256": s.1,
                }
            ]
        });
        ([("content-type", "application/json")], index.to_string())
    }

    async fn bundle_handler(State(s): State<Arc<(Vec<u8>, String)>>) -> impl IntoResponse {
        ([("content-type", "application/json")], s.0.clone())
    }

    let app: Router = Router::new()
        .route("/api/v1/index", get(index_handler))
        .route("/api/v1/hands/{id}/bundle", get(bundle_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/api/v1"), handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_succeeds_and_registers_hand() {
    let h = boot_router_allowing_loopback().await;

    let (_, real_sha) = marketplace_bundle_bytes_and_sha();
    let (registry_url, server) = spawn_mock_registry(real_sha).await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": registry_url,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "marketplace install must succeed: {body}"
    );
    assert_eq!(body["hand_id"].as_str(), Some("remote-uptime"), "{body}");
    assert_eq!(body["version"].as_str(), Some("1.0.0"), "{body}");
    assert_eq!(
        body["checksum_verified"].as_bool(),
        Some(true),
        "index advertised a matching digest, so the checksum must be verified: {body}"
    );
    assert_eq!(
        body["definition"]["id"].as_str(),
        Some("remote-uptime"),
        "response must carry the installed HandDefinition: {body}"
    );

    // Side-effect: the hand is now in the registry and surfaces on GET /api/hands.
    let (list_status, list) = get_json(&h.app, "/api/hands").await;
    assert_eq!(list_status, StatusCode::OK);
    let found = list["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|d| d["id"].as_str() == Some("remote-uptime"))
        })
        .unwrap_or(false);
    assert!(
        found,
        "installed hand must appear in GET /api/hands: {list}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_rejects_checksum_mismatch() {
    let h = boot_router_allowing_loopback().await;

    // Advertise a digest that does not match the served bundle — the download
    // step must fail the SHA-256 check before anything is written to disk.
    let wrong_sha = "0".repeat(64);
    let (registry_url, server) = spawn_mock_registry(wrong_sha).await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": registry_url,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "checksum mismatch must be rejected with 400: {body}"
    );

    // Side-effect: nothing was installed.
    let (_, list) = get_json(&h.app, "/api/hands").await;
    let found = list["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|d| d["id"].as_str() == Some("remote-uptime"))
        })
        .unwrap_or(false);
    assert!(
        !found,
        "a rejected install must not register the hand: {list}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_rejects_ssrf_registry_url() {
    // The loopback exemption is present (the harness allows `127.0.0.1`), but a
    // caller-supplied `registry_url` aimed at the cloud-metadata endpoint
    // 169.254.169.254 must still be rejected — that range is unconditionally
    // blocked regardless of the allowlist, and the install must not write
    // anything to disk before the network call. This is the regression guard
    // for the SSRF hole where `registry_url` flowed straight into
    // `HandsHubClient::with_url`.
    let h = boot_router_allowing_loopback().await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": "http://169.254.169.254/api/v1",
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an SSRF registry_url must be rejected with 400: {body}"
    );

    // Side-effect: nothing was installed.
    let (_, list) = get_json(&h.app, "/api/hands").await;
    let found = list["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|d| d["id"].as_str() == Some("remote-uptime"))
        })
        .unwrap_or(false);
    assert!(
        !found,
        "an SSRF-rejected install must not register the hand: {list}"
    );
}

// ---------------------------------------------------------------------------
// #5954 security regressions: SSRF-redirect bypass (F1), bundle id mismatch
// (F3), and the third-party-registry checksum requirement (F4).
//
// These reuse the hand-rolled axum mock style (no new `wiremock` dep on the
// api crate) but tailor each registry to one attack: a 302 on /bundle, a
// bundle whose declared id differs from the requested one, and an index that
// advertises no checksum.
// ---------------------------------------------------------------------------

/// Absolute on-disk path a hand with `id` would occupy once installed
/// (`<home>/workspaces/<id>/`). Used to assert no residue after a rejection.
fn installed_hand_dir(h: &Harness, id: &str) -> std::path::PathBuf {
    h._tmp.path().join("workspaces").join(id)
}

/// Spawn a mock registry whose `/bundle` endpoint 302-redirects to `location`.
/// The index still advertises a matching digest so the rejection is solely the
/// redirect, not a checksum failure.
async fn spawn_redirecting_registry(
    location: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    let (_, real_sha) = marketplace_bundle_bytes_and_sha();
    let state = Arc::new(real_sha);

    async fn index_handler(State(sha): State<Arc<String>>) -> impl IntoResponse {
        let index = serde_json::json!({
            "hands": [{
                "id": "remote-uptime",
                "name": "Remote Uptime",
                "description": "Installed from the marketplace.",
                "category": "data",
                "version": "1.0.0",
                "expected_sha256": *sha,
            }]
        });
        ([("content-type", "application/json")], index.to_string())
    }

    let app: Router = Router::new()
        .route("/api/v1/index", get(index_handler))
        .route(
            "/api/v1/hands/{id}/bundle",
            // A 302 to `location`. `get_with_retry` refuses every 3xx, so the
            // exact redirect status does not matter; `Redirect::temporary`
            // gives a clean `IntoResponse` with the Location header set.
            get(move || async move { axum::response::Redirect::temporary(location) }),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/api/v1"), handle)
}

/// Spawn a mock registry that serves a bundle whose declared HAND.toml `id` is
/// `bundle_id` (potentially different from the requested id). The index
/// advertises the real digest of the served bytes so the checksum passes and
/// only the id-mismatch guard can fail.
async fn spawn_mismatched_id_registry(bundle_id: &str) -> (String, tokio::task::JoinHandle<()>) {
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    let toml =
        MARKETPLACE_HAND_TOML.replace("id = \"remote-uptime\"", &format!("id = \"{bundle_id}\""));
    let bundle = serde_json::json!({ "toml": toml, "skill": "" });
    let bytes = serde_json::to_vec(&bundle).unwrap();
    let sha = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(hasher.finalize())
    };
    let state = Arc::new((bytes, sha));

    async fn index_handler(State(s): State<Arc<(Vec<u8>, String)>>) -> impl IntoResponse {
        let index = serde_json::json!({
            "hands": [{
                "id": "remote-uptime",
                "name": "Remote Uptime",
                "category": "data",
                "version": "1.0.0",
                "expected_sha256": s.1,
            }]
        });
        ([("content-type", "application/json")], index.to_string())
    }
    async fn bundle_handler(State(s): State<Arc<(Vec<u8>, String)>>) -> impl IntoResponse {
        ([("content-type", "application/json")], s.0.clone())
    }

    let app: Router = Router::new()
        .route("/api/v1/index", get(index_handler))
        .route("/api/v1/hands/{id}/bundle", get(bundle_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/api/v1"), handle)
}

/// Spawn a mock registry whose index advertises NO `expected_sha256`. The
/// served bundle is otherwise valid; this exercises the F4 trust gate.
async fn spawn_unverified_registry() -> (String, tokio::task::JoinHandle<()>) {
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;

    let (bundle_bytes, _) = marketplace_bundle_bytes_and_sha();
    let state = Arc::new(bundle_bytes);

    async fn index_handler() -> impl IntoResponse {
        let index = serde_json::json!({
            "hands": [{
                "id": "remote-uptime",
                "name": "Remote Uptime",
                "category": "data",
                "version": "1.0.0"
                // intentionally no expected_sha256
            }]
        });
        ([("content-type", "application/json")], index.to_string())
    }
    async fn bundle_handler(State(b): State<Arc<Vec<u8>>>) -> impl IntoResponse {
        ([("content-type", "application/json")], (*b).clone())
    }

    let app: Router = Router::new()
        .route("/api/v1/index", get(index_handler))
        .route("/api/v1/hands/{id}/bundle", get(bundle_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/api/v1"), handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_rejects_bundle_redirect_to_metadata_ip() {
    // ATTACK (F1): a registry that passes the SSRF string check 302-redirects
    // the /bundle fetch at the cloud-metadata endpoint. Auto-redirect is
    // disabled in the HandsHub client, so the install must fail with no
    // on-disk residue — the redirect is never followed to 169.254.169.254.
    let h = boot_router_allowing_loopback().await;
    let (registry_url, server) =
        spawn_redirecting_registry("http://169.254.169.254/latest/meta-data/").await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": registry_url,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a /bundle redirect must be refused (auto-redirect is disabled): {body}"
    );
    assert!(
        !installed_hand_dir(&h, "remote-uptime").exists(),
        "a rejected redirect install must leave nothing on disk"
    );
    let (_, list) = get_json(&h.app, "/api/hands").await;
    let found = list["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .any(|d| d["id"].as_str() == Some("remote-uptime"))
        })
        .unwrap_or(false);
    assert!(
        !found,
        "a rejected redirect install must not register the hand: {list}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_rejects_bundle_id_mismatch() {
    // ATTACK (F3): caller asks for `remote-uptime`, registry serves a bundle
    // whose HAND.toml declares `evil-other`. Name confusion must be refused
    // before anything is written under either id.
    let h = boot_router_allowing_loopback().await;
    let (registry_url, server) = spawn_mismatched_id_registry("evil-other").await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": registry_url,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a bundle declaring a different id must be rejected: {body}"
    );
    assert!(
        !installed_hand_dir(&h, "remote-uptime").exists()
            && !installed_hand_dir(&h, "evil-other").exists(),
        "an id-mismatched install must leave nothing on disk under either id"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_install_rejects_unverified_third_party_registry() {
    // POLICY (F4): a caller-supplied (third-party) registry that advertises NO
    // expected_sha256 must be refused — unverified installs are only tolerated
    // from the compiled-in default registry. The bundle bytes are valid; the
    // rejection is purely the missing-checksum trust gate.
    let h = boot_router_allowing_loopback().await;
    let (registry_url, server) = spawn_unverified_registry().await;

    let (status, body) = json_request(
        &h.app,
        Method::POST,
        "/api/hands/marketplace/install",
        Some(serde_json::json!({
            "hand_id": "remote-uptime",
            "registry_url": registry_url,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unverified install from a third-party registry must be refused: {body}"
    );
    assert!(
        !installed_hand_dir(&h, "remote-uptime").exists(),
        "a refused unverified install must leave nothing on disk"
    );

    server.abort();
}
