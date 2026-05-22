//! Integration tests for the event-webhook routes' SSRF write-time gate
//! (PR #5353, audit: webhook-create-no-ssrf-check).
//!
//! Scope (TIGHT): prove the `/api/webhooks/events` CREATE (`POST`) and UPDATE
//! (`PUT /{id}`) routes reject internal / loopback / metadata-IP destination
//! URLs with `400 Bad Request`. The validator itself
//! (`webhook_store::validate_webhook_url`) is unit-tested in `webhook_store.rs`;
//! these tests cover the *route wiring* — that the gate is actually invoked at
//! both write sites, including the PATCH-equivalent `PUT` path that the PR
//! specifically hardened (an attacker who created a benign webhook could
//! otherwise repoint it at an internal URL after the fact).
//!
//! The router is booted via `server::build_router` so `/api/webhooks/events` is
//! mounted exactly as in production. `api_key` is left empty so the `/api/*`
//! auth layer is a no-op and the route is reachable without a token (mirrors
//! the default unauthenticated deployment; see `middleware::auth`).
//!
//! Run: cargo test -p librefang-api --test webhooks_routes_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot the full router via `server::build_router` with an empty `api_key` so
/// the `/api/webhooks/events` routes are reachable without auth. Mirrors
/// `channel_webhooks_test::boot`.
async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Avoid network access during kernel boot.
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: String::new(),
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
    }
}

async fn post_events(h: &Harness, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    send_json(h, Method::POST, "/api/webhooks/events", body).await
}

async fn send_json(
    h: &Harness,
    method: Method,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    inject_loopback(&mut req);
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Inject a loopback `ConnectInfo` so the auth middleware grants access with an
/// empty `api_key`. Without a real socket, `oneshot` attaches no `ConnectInfo`
/// and the middleware fails closed ("API key required for non-loopback
/// requests"). Mirrors `credential_pools_routes_test.rs` / `route_smoke.rs`.
fn inject_loopback(req: &mut Request<Body>) {
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
}

/// Extract the human-readable error message from an `ApiErrorResponse` body.
/// `types::ApiErrorResponse` serializes the message at top-level `message` and
/// mirrors it under nested `error.message` (see `types.rs` custom Serialize).
fn err_message(json: &serde_json::Value) -> String {
    json["message"]
        .as_str()
        .or_else(|| json["error"]["message"].as_str())
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// CREATE — POST /api/webhooks/events
// ---------------------------------------------------------------------------

/// SSRF URLs (metadata IP, loopback, private range, link-local) must be
/// rejected at create-time with 400 so they never persist in the store.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_event_webhook_rejects_ssrf_urls() {
    let h = boot().await;

    // Cloud metadata IMDS endpoint — the canonical SSRF target.
    let (status, json) = post_events(
        &h,
        serde_json::json!({
            "url": "http://169.254.169.254/latest/meta-data/",
            "events": ["agent.spawned"],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "metadata IP must be rejected; body={json}"
    );
    let err = err_message(&json);
    assert!(
        err.contains("169.254.169.254") || err.contains("not allowed"),
        "error should mention the rejected host; got {err:?}"
    );

    // The other internal-URL families the gate blocks.
    for url in [
        "http://127.0.0.1:80/x",       // loopback
        "http://localhost:6379/",      // loopback hostname
        "http://10.0.0.1/hook",        // RFC1918 private
        "http://[::ffff:127.0.0.1]/x", // IPv4-mapped IPv6 loopback
    ] {
        let (status, json) = post_events(
            &h,
            serde_json::json!({ "url": url, "events": ["agent.spawned"] }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "internal URL {url} must be rejected; body={json}"
        );
    }
}

/// A public URL still creates successfully — proves the gate isn't rejecting
/// everything (guards against a false-positive that would make the SSRF
/// assertions vacuous).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_event_webhook_allows_public_url() {
    let h = boot().await;
    let (status, json) = post_events(
        &h,
        serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "public URL should pass; body={json}"
    );
    assert_eq!(json["url"], "https://example.com/hook");
}

// ---------------------------------------------------------------------------
// UPDATE — PUT /api/webhooks/events/{id} (the bypass PR #5353 closed)
// ---------------------------------------------------------------------------

/// An attacker who created a benign webhook must NOT be able to repoint it at
/// an internal URL via update. Create a valid webhook, then PUT an SSRF URL and
/// assert 400 — this is the exact bypass the PR added the update-time gate for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_event_webhook_rejects_ssrf_url() {
    let h = boot().await;

    // 1. Create a benign webhook to obtain a real id.
    let (status, created) = post_events(
        &h,
        serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "setup create failed; body={created}"
    );
    let id = created["id"]
        .as_str()
        .expect("created webhook id")
        .to_string();

    // 2. Try to repoint it at the metadata IP — must be rejected.
    let (status, json) = send_json(
        &h,
        Method::PUT,
        &format!("/api/webhooks/events/{id}"),
        serde_json::json!({ "url": "http://169.254.169.254/latest/meta-data/" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "update to SSRF URL must be rejected; body={json}"
    );
    let err = err_message(&json);
    assert!(
        err.contains("169.254.169.254") || err.contains("not allowed"),
        "error should mention the rejected host; got {err:?}"
    );

    // 3. The stored webhook must still hold the original benign URL.
    let mut req = Request::builder()
        .method(Method::GET)
        .uri("/api/webhooks/events")
        .body(Body::empty())
        .unwrap();
    inject_loopback(&mut req);
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let entry = list
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == serde_json::json!(id))
        .expect("webhook still present after rejected update");
    assert_eq!(
        entry["url"], "https://example.com/hook",
        "rejected update must not mutate the stored URL"
    );
}
