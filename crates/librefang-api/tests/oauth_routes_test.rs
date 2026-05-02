//! Integration tests for the external-auth (OAuth2 / OIDC) HTTP surface.
//!
//! These exercise the validation paths of the route handlers in
//! `crates/librefang-api/src/oauth.rs` — i.e. the cases that can be
//! provoked without making real outbound HTTP calls to an identity
//! provider. Happy-path code-exchange and JWKS-validation paths require
//! a live IdP and are intentionally NOT covered here (see issue #3571
//! follow-ups).
//!
//! Routes covered (registered in `server.rs`):
//!   - GET  /api/auth/providers
//!   - GET  /api/auth/login
//!   - GET  /api/auth/login/{provider}
//!   - GET  /api/auth/callback
//!   - POST /api/auth/callback
//!   - GET  /api/auth/userinfo
//!   - POST /api/auth/introspect
//!   - POST /api/auth/refresh

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{ExternalAuthConfig, OidcProvider};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// Build a router that mirrors the OAuth slice of `server.rs::api_v1_routes`.
/// Using a hand-rolled router (rather than the full `api_v1_routes()`) keeps
/// the harness fast and free of LLM/auth middleware that's irrelevant to
/// these tests — the routes themselves are what we want to exercise.
fn oauth_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/auth/providers",
            axum::routing::get(librefang_api::oauth::auth_providers),
        )
        .route(
            "/api/auth/login",
            axum::routing::get(librefang_api::oauth::auth_login),
        )
        .route(
            "/api/auth/login/{provider}",
            axum::routing::get(librefang_api::oauth::auth_login_provider),
        )
        .route(
            "/api/auth/callback",
            axum::routing::get(librefang_api::oauth::auth_callback)
                .post(librefang_api::oauth::auth_callback_post),
        )
        .route(
            "/api/auth/userinfo",
            axum::routing::get(librefang_api::oauth::auth_userinfo),
        )
        .route(
            "/api/auth/introspect",
            axum::routing::post(librefang_api::oauth::auth_introspect),
        )
        .route(
            "/api/auth/refresh",
            axum::routing::post(librefang_api::oauth::auth_refresh),
        )
        .with_state(state)
}

async fn boot_with_external_auth(ext: ExternalAuthConfig) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.external_auth = ext.clone();
    }));
    let state = test.state.clone();
    let app = oauth_router(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

async fn boot_disabled() -> Harness {
    boot_with_external_auth(ExternalAuthConfig::default()).await
}

/// One enabled OIDC provider that points at unreachable URLs. The handlers
/// must reach validation-failure branches before any outbound HTTP fires —
/// these tests assert exactly that. If a regression makes a handler skip
/// validation and dial out, the test will hang (caught by the per-test
/// timeout) or surface a different status from the network failure.
fn enabled_with_one_provider() -> ExternalAuthConfig {
    ExternalAuthConfig {
        enabled: true,
        providers: vec![OidcProvider {
            id: "test".into(),
            display_name: "Test".into(),
            issuer_url: String::new(),
            auth_url: "https://example.invalid/authorize".into(),
            token_url: "https://example.invalid/token".into(),
            userinfo_url: String::new(),
            jwks_uri: String::new(),
            client_id: "client-id".into(),
            client_secret_env: "LIBREFANG_TEST_OAUTH_SECRET_DOES_NOT_EXIST".into(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".into(),
            scopes: vec!["openid".into()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: None,
        }],
        ..Default::default()
    }
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(b) = bearer {
        builder = builder.header("authorization", format!("Bearer {b}"));
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
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ─── /auth/providers ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn providers_disabled_reports_empty_list() {
    let h = boot_disabled().await;
    let (status, body) = send(&h, Method::GET, "/api/auth/providers", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);
    assert_eq!(body["providers"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_enabled_lists_configured_provider() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(&h, Method::GET, "/api/auth/providers", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    let arr = body["providers"].as_array().expect("providers array");
    assert!(
        arr.iter().any(|p| p["id"] == "test"),
        "expected configured provider 'test' in {body:?}"
    );
}

// ─── /auth/login (legacy single-provider) ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn login_disabled_returns_503() {
    let h = boot_disabled().await;
    let (status, body) = send(&h, Method::GET, "/api/auth/login", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("external"),
        "unexpected body: {body:?}"
    );
}

// ─── /auth/login/{provider} ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn login_provider_disabled_returns_503() {
    let h = boot_disabled().await;
    let (status, _) = send(&h, Method::GET, "/api/auth/login/google", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(flavor = "multi_thread")]
async fn login_provider_unknown_returns_404() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/auth/login/no-such-provider",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("no-such-provider"),
        "error must name the missing provider: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn login_provider_known_redirects_to_idp() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/login/test")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    // Build-redirect path returns 307 Temporary Redirect with the IdP URL
    // in the Location header. We do NOT follow it — that would hit the
    // unreachable example.invalid host. The redirect existence is the
    // assertion.
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        location.starts_with("https://example.invalid/authorize"),
        "Location must point at the configured auth_url: {location}"
    );
    assert!(
        location.contains("client_id=client-id"),
        "Location must carry client_id: {location}"
    );
    assert!(
        location.contains("state="),
        "Location must carry a signed state token: {location}"
    );
}

// ─── /auth/callback (GET) ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_disabled_returns_503() {
    let h = boot_disabled().await;
    let (status, _) = send(
        &h,
        Method::GET,
        "/api/auth/callback?code=abc&state=xyz",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_idp_error_param_returns_401() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/auth/callback?error=access_denied&error_description=user+cancelled",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    assert_eq!(body["error"], "access_denied");
    assert_eq!(body["error_description"], "user cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_missing_code_returns_400() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(&h, Method::GET, "/api/auth/callback?state=xyz", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("code"),
        "error must mention missing code: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_missing_state_returns_400() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(&h, Method::GET, "/api/auth/callback?code=abc", None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("state"),
        "error must mention missing state: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_malformed_state_token_rejected() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    // Missing the dot separator — `verify_state_token` splits on `.`.
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/auth/callback?code=abc&state=not-a-real-state-token",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("state"),
        "error must mention state: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_get_state_with_bad_signature_rejected() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    // Looks like a state token (has the `.` separator) but the signature
    // is gibberish — must hit the HMAC-verify failure branch, not the
    // outer `Invalid state format` branch.
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/auth/callback?code=abc&state=eyJwIjoieCJ9.bogussig",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("state"),
        "error must mention state: {body:?}"
    );
}

// ─── /auth/callback (POST) ───────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn callback_post_disabled_returns_503() {
    let h = boot_disabled().await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/auth/callback",
        Some(serde_json::json!({"code": "abc", "state": "xyz"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_post_missing_required_fields_rejected() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    // `CallbackBody` requires both `code` and `state` — axum's Json
    // extractor surfaces the deserialization failure as a 422.
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/auth/callback",
        Some(serde_json::json!({"code": "abc"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_post_malformed_state_returns_400() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/auth/callback",
        Some(serde_json::json!({"code": "abc", "state": "garbage-not-signed"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("state"));
}

// ─── /auth/userinfo ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn userinfo_disabled_reports_api_key_method() {
    let h = boot_disabled().await;
    let (status, body) = send(&h, Method::GET, "/api/auth/userinfo", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth_method"], "api_key");
}

#[tokio::test(flavor = "multi_thread")]
async fn userinfo_enabled_without_bearer_returns_401() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(&h, Method::GET, "/api/auth/userinfo", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("bearer"),
        "error must mention missing Bearer: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn userinfo_enabled_with_unverifiable_bearer_returns_401() {
    // Provider has empty jwks_uri AND empty userinfo_url, so neither
    // validation path can succeed — handler must fall through to the
    // "could not be validated against any provider" branch.
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/auth/userinfo",
        None,
        Some("not-a-real-jwt"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("validated"),
        "error must mention validation: {body:?}"
    );
}

// ─── /auth/introspect ────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn introspect_disabled_returns_inactive() {
    let h = boot_disabled().await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/auth/introspect",
        Some(serde_json::json!({"token": "anything"})),
        None,
    )
    .await;
    // RFC 7662 conventions — handler returns 200 with active=false even
    // when the daemon is configured to reject all introspection requests.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn introspect_enabled_unverifiable_token_returns_inactive() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/auth/introspect",
        Some(serde_json::json!({"token": "not-a-real-jwt"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);
    assert!(
        body["error"].as_str().unwrap_or("").contains("validated"),
        "error must explain why token is inactive: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn introspect_missing_token_field_rejected() {
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    // `IntrospectRequest.token` is required (no `#[serde(default)]`).
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/auth/introspect",
        Some(serde_json::json!({})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ─── /auth/refresh ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn refresh_disabled_returns_503() {
    let h = boot_disabled().await;
    let (status, _) = send(
        &h,
        Method::POST,
        "/api/auth/refresh",
        Some(serde_json::json!({"refresh_token": "anything"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_without_token_or_provider_hint_returns_4xx() {
    // No refresh_token in body, no provider hint, no entry in TOKEN_STORE
    // for this fresh harness — handler must reject rather than fan out
    // to a network call. The exact 4xx code is up to the handler; both
    // 400 (bad request shape) and 404 (not found in store) are
    // acceptable validation outcomes. We pin "client error, NOT 5xx and
    // NOT 200" to catch a regression that lets the handler hang on an
    // outbound HTTP call.
    let h = boot_with_external_auth(enabled_with_one_provider()).await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/auth/refresh",
        Some(serde_json::json!({})),
        None,
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx for empty refresh request, got {status} body={body:?}"
    );
}
