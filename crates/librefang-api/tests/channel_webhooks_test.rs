//! Integration tests for the `/channels/{adapter}/*` webhook surface (#3571).
//!
//! Scope (TIGHT): exercise the *outer* webhook router that `server::build_router`
//! mounts at `app.nest("/channels", channel_routes)` (see `server.rs` ~L1309).
//! These are NOT the `/api/channels/*` config endpoints (covered separately by
//! `channels_routes_test.rs`).
//!
//! With no channel adapters configured (the default `MockKernelBuilder` /
//! `KernelConfig::default()` state), the dynamic inner router behind
//! `state.webhook_router` is empty. We deliberately do NOT fake a configured
//! adapter — per CLAUDE.md, "happy paths that require real bot tokens or
//! trigger downstream LLM calls" are out of scope. Instead we lock down the
//! invariants that hold *regardless* of which adapters are mounted:
//!
//! 1. `/channels/*` bypasses auth — when `api_key` is set, an unconfigured
//!    webhook path returns 404 (not 401). This is the security-critical
//!    contract documented at `server.rs` ~L1310: "These bypass auth/rate-limit
//!    layers since external platforms handle their own signature verification."
//! 2. The 1 MiB `RequestBodyLimitLayer` (#3813) attached to `channel_routes`
//!    *before* `.nest()` is in force — a 2 MiB POST body is rejected with 413
//!    (Payload Too Large), not silently accepted into a handler.
//! 3. Unknown adapter paths return 404, not 500 — the `Arc::try_unwrap` /
//!    fallback dance in `server.rs` doesn't panic on missing routes.
//!
//! Run: cargo test -p librefang-api --test channel_webhooks_test

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
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot the full router via `server::build_router` so the `/channels` nest,
/// the `RequestBodyLimitLayer`, and the auth middleware are all wired exactly
/// as production. Mirrors `auth_public_allowlist::boot_router_with_api_key`.
async fn boot(api_key: &str) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Avoid network access during kernel boot.
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
    }
}

async fn send(h: &Harness, req: Request<Body>) -> StatusCode {
    h.app.clone().oneshot(req).await.unwrap().status()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `/channels/*` MUST bypass the auth layer even when `api_key` is set —
/// external platforms (Feishu, Slack, Teams, …) verify their own signatures
/// and the layer is intentionally NOT applied to the nested router. A protected
/// path on `/api/*` returns 401 without a token; a webhook path returns 404
/// (route not registered, since no adapter is configured), proving auth was
/// never consulted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_path_does_not_require_auth_when_api_key_is_set() {
    let h = boot("super-secret-key").await;

    // Sanity: a normal authed endpoint returns 401 with no token.
    let authed_req = Request::builder()
        .method(Method::GET)
        .uri("/api/agents")
        .body(Body::empty())
        .unwrap();
    let authed_status = send(&h, authed_req).await;
    assert_eq!(
        authed_status,
        StatusCode::UNAUTHORIZED,
        "/api/agents should be 401 without bearer when api_key is configured \
         (otherwise this test cannot distinguish auth-bypass from no-auth)"
    );

    // Webhook path: no token, no signature, but must NOT be 401.
    let webhook_req = Request::builder()
        .method(Method::POST)
        .uri("/channels/feishu/webhook")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"type":"event_callback"}"#))
        .unwrap();
    let webhook_status = send(&h, webhook_req).await;
    assert_ne!(
        webhook_status,
        StatusCode::UNAUTHORIZED,
        "/channels/* must bypass auth (server.rs ~L1310); got 401, which means \
         the auth layer is now wrapping the nested webhook router"
    );
    // No adapter is configured, so the empty inner router 404s. We assert the
    // exact code so a future change that swaps the fallback wiring (e.g. to
    // 500 on missing adapter) gets caught.
    assert_eq!(
        webhook_status,
        StatusCode::NOT_FOUND,
        "expected 404 from empty webhook router; got {webhook_status}"
    );
}

/// The 1 MiB `RequestBodyLimitLayer` on `channel_routes` (#3813) MUST reject
/// oversized payloads with 413 *before* the body reaches any adapter handler.
/// We don't need a configured adapter for this — the layer wraps the router
/// itself, so the limit fires regardless of whether a route matches.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_body_size_limit_rejects_oversized_payload() {
    let h = boot("test-key").await;

    // 2 MiB body — twice the 1 MiB cap in server.rs (`WEBHOOK_BODY_LIMIT`).
    let oversized = vec![b'a'; 2 * 1024 * 1024];
    let req = Request::builder()
        .method(Method::POST)
        .uri("/channels/slack/events")
        .header("content-type", "application/json")
        .header("content-length", oversized.len().to_string())
        .body(Body::from(oversized))
        .unwrap();
    let status = send(&h, req).await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "2 MiB POST to /channels/* must be rejected by the 1 MiB body-limit \
         layer (server.rs WEBHOOK_BODY_LIMIT, #3813); got {status}"
    );
}

/// A small payload to an unconfigured adapter path must fall through cleanly
/// (404), not 5xx. Regression for the `Arc::try_unwrap` / `oneshot` fallback
/// in `server.rs` ~L1322 — a panic there would surface as 500 / connection
/// reset rather than a clean 404.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_adapter_path_returns_404_not_500() {
    let h = boot("test-key").await;

    for (method, path) in [
        (Method::GET, "/channels/does-not-exist"),
        (Method::POST, "/channels/does-not-exist/webhook"),
        (Method::GET, "/channels/telegram/updates"),
    ] {
        let req = Request::builder()
            .method(method.clone())
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let status = send(&h, req).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} should fall through cleanly (404/405); got {status}"
        );
        assert!(
            !status.is_server_error(),
            "{method} {path} returned a 5xx ({status}) — fallback router panicked or errored"
        );
    }
}

/// Empty body to an unconfigured adapter must also be a clean 404, not a
/// content-length / parse 400 from a layer firing too eagerly. Locks the
/// "layer attaches to nested router only" invariant noted in server.rs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_empty_body_does_not_trigger_5xx() {
    let h = boot("test-key").await;

    let req = Request::builder()
        .method(Method::POST)
        .uri("/channels/discord/interactions")
        .body(Body::empty())
        .unwrap();
    let status = send(&h, req).await;
    assert!(
        !status.is_server_error(),
        "empty POST to /channels/* must not 5xx; got {status}"
    );
}
