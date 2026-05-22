//! Integration tests for the dashboard SPA fallback (`react_asset`).
//!
//! API-surface-hygiene roundup (#1: SPA route allowlist). Before the fix any
//! extensionless `/dashboard/<word>` resolved to `index.html`, so an attacker
//! could craft a plausible slug (e.g. `/dashboard/security-alert`) and have it
//! render the trusted dashboard chrome — a phishing surface on the operator's
//! own origin. The handler now serves `index.html` only for first segments in
//! the `SPA_ROUTES` allowlist; every other extensionless miss returns 404.
//!
//! These tests boot a full router (with middleware) in open mode so the
//! dashboard shell is publicly reachable, and write a runtime `index.html`
//! under `{home}/dashboard/` so `resolve_dashboard_file` deterministically
//! resolves the SPA shell regardless of whether the embedded `static/react`
//! bundle was populated by the build.

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
    _state: Arc<librefang_api::routes::AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

const INDEX_HTML: &[u8] =
    b"<!doctype html><html><head><title>LibreFang</title></head><body>SPA-SHELL</body></html>";

/// Boot a full router in open mode (no api_key) with a runtime dashboard
/// directory containing a recognizable `index.html`.
async fn boot_open_with_dashboard() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Runtime dashboard dir is consulted before the embedded bundle, so this
    // makes index.html resolution deterministic in CI (where static/react is
    // an empty placeholder).
    let dashboard = tmp.path().join("dashboard");
    std::fs::create_dir_all(&dashboard).expect("create dashboard dir");
    std::fs::write(dashboard.join("index.html"), INDEX_HTML).expect("write index.html");
    // A real asset with an extension, so the asset-hit branch is exercised too.
    std::fs::write(dashboard.join("app.js"), b"console.log('app');").expect("write app.js");

    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: String::new(), // open mode → dashboard shell is public
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

async fn get(app: axum::Router, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// A known SPA route (extensionless, first segment in the allowlist) falls back
/// to the SPA shell with 200.
#[tokio::test(flavor = "multi_thread")]
async fn spa_route_serves_index_html() {
    let h = boot_open_with_dashboard().await;
    let (status, body) = get(h.app.clone(), "/dashboard/agents").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "known SPA route must serve the shell"
    );
    assert!(
        body.contains("SPA-SHELL"),
        "expected the SPA shell body, got: {body}"
    );
}

/// A nested SPA route (e.g. `/dashboard/config/general`) matches on its first
/// segment and serves the shell.
#[tokio::test(flavor = "multi_thread")]
async fn nested_spa_route_serves_index_html() {
    let h = boot_open_with_dashboard().await;
    let (status, body) = get(h.app.clone(), "/dashboard/config/general").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("SPA-SHELL"), "got: {body}");
}

/// An extensionless path that is NOT a known SPA route must 404 rather than
/// rendering the trusted dashboard chrome (phishing-surface fix).
#[tokio::test(flavor = "multi_thread")]
async fn unknown_extensionless_path_returns_404() {
    let h = boot_open_with_dashboard().await;
    let (status, body) = get(h.app.clone(), "/dashboard/security-alert").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown extensionless path must 404, not serve the SPA shell"
    );
    assert!(
        !body.contains("SPA-SHELL"),
        "the SPA shell must NOT be served for a non-route slug; got: {body}"
    );
}

/// A real asset (with an extension) still resolves normally.
#[tokio::test(flavor = "multi_thread")]
async fn real_asset_is_served() {
    let h = boot_open_with_dashboard().await;
    let (status, body) = get(h.app.clone(), "/dashboard/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("console.log"), "got: {body}");
}

/// A missing asset WITH an extension 404s (no SPA fallback for extensioned
/// paths) — pins that the allowlist gate did not change the extensioned-miss
/// behaviour.
#[tokio::test(flavor = "multi_thread")]
async fn missing_extensioned_asset_returns_404() {
    let h = boot_open_with_dashboard().await;
    let (status, _body) = get(h.app.clone(), "/dashboard/nope.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
