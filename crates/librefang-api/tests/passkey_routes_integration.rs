//! Integration tests for the passkey (WebAuthn/FIDO2) routes (#5981).
//!
//! These exercise the parts of the flow that don't require a virtual
//! authenticator: route wiring, the public/auth gating (middleware allowlist
//! and `is_owner_only_write`), the opt-in engine gate (503 when disabled), and
//! the challenge-issuing path of the authentication ceremony.
//! The WebAuthn cryptography itself (attestation / assertion verification) is
//! the `webauthn-rs` crate's own tested responsibility and is not re-verified
//! here.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
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

fn base_config() -> KernelConfig {
    KernelConfig {
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
    }
}

async fn boot(config: KernelConfig) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        ..config
    };
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;
    Harness {
        app,
        _tmp: tmp,
        state,
    }
}

/// Passkey enabled, api_key set so auth is enforced on non-public routes.
async fn boot_enabled() -> Harness {
    let mut cfg = base_config();
    cfg.api_key = "test-secret-key".to_string();
    cfg.dashboard_user = "admin".to_string();
    cfg.passkey_enabled = true;
    cfg.passkey_rp_id = "localhost".to_string();
    cfg.passkey_rp_origin = "http://localhost".to_string();
    boot(cfg).await
}

async fn send(harness: &Harness, method: Method, path: &str, token: Option<&str>) -> StatusCode {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let req = req
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    harness
        .app
        .clone()
        .oneshot(req)
        .await
        .expect("request")
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn authentication_options_is_public_and_engine_gated() {
    // Engine enabled but no passkeys registered → 400 no_passkeys, reached
    // WITHOUT a bearer token. Proves the route is wired AND in the public
    // allowlist (otherwise the auth middleware would 401 first).
    let h = boot_enabled().await;
    let status = send(
        &h,
        Method::POST,
        "/api/auth/passkey/authentication-options",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "public authentication-options with no credentials should be 400 no_passkeys, got {status}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authentication_verify_is_public() {
    // A garbage body reaches the handler (public) and fails at ceremony
    // lookup → 400, never 401. Proves the verify endpoint is public too.
    let h = boot_enabled().await;
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/passkey/authentication-verify")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"ceremony_id":"nope","credential":{"id":"AAAA","rawId":"AAAA","response":{"authenticatorData":"AAAA","clientDataJSON":"AAAA","signature":"AAAA"},"type":"public-key"}}"#,
        ))
        .unwrap();
    // The handler extracts `ConnectInfo<SocketAddr>` (for the session cookie's
    // Secure attribute); a bare tower `oneshot` has no connect info, so inject
    // it the way `axum::serve(..).into_make_service_with_connect_info` would.
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
    ));
    let status = h.app.clone().oneshot(req).await.unwrap().status();
    assert_ne!(status, StatusCode::UNAUTHORIZED, "verify must be public");
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown ceremony → 400");
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_options_requires_auth() {
    // Registration is auth-gated: no token → 401 from the middleware, before
    // the handler (and before the engine check).
    let h = boot_enabled().await;
    let status = send(
        &h,
        Method::POST,
        "/api/auth/passkey/registration-options",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "registration-options without a token must be 401"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_and_revoke_require_auth() {
    let h = boot_enabled().await;
    let list = send(&h, Method::GET, "/api/auth/passkey/credentials", None).await;
    assert_eq!(list, StatusCode::UNAUTHORIZED, "list must require auth");
    let revoke = send(
        &h,
        Method::DELETE,
        "/api/auth/passkey/credentials/some-id",
        None,
    )
    .await;
    assert_eq!(revoke, StatusCode::UNAUTHORIZED, "revoke must require auth");
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoints_503_when_disabled() {
    // Default config: passkey_enabled = false → engine None → 503 on the
    // public authentication-options (api_key unset, so auth isn't the gate).
    let h = boot(base_config()).await;
    let status = send(
        &h,
        Method::POST,
        "/api/auth/passkey/authentication-options",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "passkey disabled → 503, got {status}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_credentials_authenticated_is_empty() {
    // With a valid bearer token the list endpoint returns 200 + an empty
    // credentials array (no passkeys registered yet). Proves the store path
    // is wired and the auth principal flows through.
    let h = boot_enabled().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/passkey/credentials")
        .header("Authorization", "Bearer test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["credentials"].as_array().map(|a| a.len()), Some(0));
}
