//! Integration tests for the `/api/terminal/*` REST routes.
//!
//! Refs #3571 — the "~80% of registered HTTP routes have no integration test"
//! audit. This file covers the **terminal-domain REST surface** declared by
//! `routes::terminal::router()`:
//!
//! * `GET    /terminal/health`
//! * `GET    /terminal/windows`
//! * `POST   /terminal/windows`
//! * `DELETE /terminal/windows/{window_id}`
//! * `PATCH  /terminal/windows/{window_id}`
//!
//! The fifth registered route, `GET /terminal/ws`, performs a WebSocket
//! upgrade and **cannot** be exercised by `tower::oneshot` — see the PR
//! body for why it is intentionally skipped.
//!
//! Coverage target is the validation + auth-gate layers only:
//! happy-path window creation/listing requires a real `tmux` subprocess,
//! which we do NOT spawn in unit tests. Instead we drive the configuration
//! flag `tmux_enabled = false` to make `tmux_controller` short-circuit with
//! 403 — that is sufficient to pin the auth-gate ordering without flaking
//! on hosts that lack the `tmux` binary.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// Build a harness with the terminal sub-router mounted at `/api`. The
/// caller can mutate the kernel config via `tweak` — terminal tests need
/// to flip `tmux_enabled`, `enabled`, and `api_key` between cases.
async fn boot_with<F>(tweak: F) -> Harness
where
    F: FnOnce(&mut librefang_types::config::KernelConfig) + Send + 'static,
{
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(tweak));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::terminal::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

/// Default harness — terminal enabled, tmux disabled (so we don't depend
/// on the `tmux` binary being installed), no api_key (LocalBypass auth).
async fn boot() -> Harness {
    boot_with(|cfg| {
        cfg.terminal.tmux_enabled = false;
    })
    .await
}

/// Construct a request and inject loopback `ConnectInfo` so the
/// terminal authorizer sees a localhost caller. Without this extension
/// the `ConnectInfo<SocketAddr>` extractor fails the request before the
/// handler even runs (mirrors the helper in `api_integration_test.rs`).
fn request(method: Method, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let mut req = builder.body(Body::from(body_bytes)).unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    req
}

fn request_with_bearer(
    method: Method,
    uri: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut req = request(method, uri, body);
    req.headers_mut()
        .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
    req
}

async fn send(h: &Harness, req: Request<Body>) -> (StatusCode, serde_json::Value) {
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

// ─── /terminal/health ─────────────────────────────────────────────────────

/// `terminal_health` returns 200 with a JSON body describing tmux state and
/// the active window cap. We disable tmux so the boolean is deterministic
/// regardless of the host's PATH.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_returns_status_payload() {
    let h = boot().await;
    let (status, body) = send(&h, request(Method::GET, "/api/terminal/health", None)).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["tmux"], false, "tmux disabled in this harness");
    assert!(body["max_windows"].is_number(), "{body:?}");
    assert!(body["os"].is_string(), "{body:?}");
}

/// `terminal.enabled = false` is the master kill-switch — every REST route
/// returns 403 regardless of auth state. Pins the ordering: the disabled
/// check fires before any tmux-availability or token validation work.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_403_when_terminal_disabled() {
    let h = boot_with(|cfg| {
        cfg.terminal.enabled = false;
        cfg.terminal.tmux_enabled = false;
    })
    .await;
    let (status, _) = send(&h, request(Method::GET, "/api/terminal/health", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// With an `api_key` configured, an unauthenticated terminal-health request
/// must be rejected at the auth boundary even though `terminal.enabled` is
/// true. This is the primary regression guard against accidentally turning
/// the terminal control plane into an open endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_401_when_api_key_set_and_no_bearer() {
    let h = boot_with(|cfg| {
        cfg.terminal.tmux_enabled = false;
        cfg.api_key = "test-secret-key".to_string();
    })
    .await;
    let (status, _) = send(&h, request(Method::GET, "/api/terminal/health", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Same setup, but with a wrong Bearer — must still be 401, never 200.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_401_when_bearer_is_wrong() {
    let h = boot_with(|cfg| {
        cfg.terminal.tmux_enabled = false;
        cfg.api_key = "test-secret-key".to_string();
    })
    .await;
    let (status, _) = send(
        &h,
        request_with_bearer(Method::GET, "/api/terminal/health", "wrong-token", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Correct Bearer token with `api_key` configured → 200. Confirms the
/// happy auth path actually verifies tokens (not just rejects them).
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_200_with_correct_bearer() {
    let h = boot_with(|cfg| {
        cfg.terminal.tmux_enabled = false;
        cfg.api_key = "test-secret-key".to_string();
    })
    .await;
    let (status, body) = send(
        &h,
        request_with_bearer(Method::GET, "/api/terminal/health", "test-secret-key", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["ok"], true);
}

/// `?token=` query-string auth was removed in #3610 (security: leaked into
/// access logs). Any caller still attempting the legacy form must be
/// rejected with 401, even on loopback with no api_key configured.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_health_rejects_legacy_query_token() {
    let h = boot().await;
    let (status, _) = send(
        &h,
        request(Method::GET, "/api/terminal/health?token=anything", None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ─── /terminal/windows (list / create) ────────────────────────────────────

/// With `tmux_enabled = false`, the list-windows route must short-circuit
/// at the `tmux_controller` gate with 403. Pins both the gate ordering
/// (auth → tmux config) and the "tmux off" UX.
#[tokio::test(flavor = "multi_thread")]
async fn list_windows_403_when_tmux_disabled() {
    let h = boot().await;
    let (status, _) = send(&h, request(Method::GET, "/api/terminal/windows", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Auth wraps tmux state — when api_key is set, an unauthenticated GET
/// must return 401, NOT the 403 the disabled-tmux branch would emit.
/// Confirms `authorize_terminal_request` runs before `tmux_controller`.
#[tokio::test(flavor = "multi_thread")]
async fn list_windows_401_takes_precedence_over_tmux_disabled() {
    let h = boot_with(|cfg| {
        cfg.terminal.tmux_enabled = false;
        cfg.api_key = "test-secret-key".to_string();
    })
    .await;
    let (status, _) = send(&h, request(Method::GET, "/api/terminal/windows", None)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "auth must reject before the tmux-disabled branch fires"
    );
}

/// Invalid window names (containing `|`, the field separator the tmux
/// list-windows parser depends on) must be rejected with 400 BEFORE any
/// tmux subprocess is spawned. This is the primary input-sanitisation
/// guard for the create path.
#[tokio::test(flavor = "multi_thread")]
async fn create_window_400_on_invalid_name_pipe() {
    // tmux is NOT enabled — but validation is supposed to fire first,
    // so we should still get 400 (not the 403 that tmux_controller would
    // emit for `tmux_enabled = false`).
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(
            Method::POST,
            "/api/terminal/windows",
            Some(serde_json::json!({"name": "bad|name"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_name");
}

/// Control characters in window names break the parser too — newlines
/// in particular. Same 400 path as the `|` case.
#[tokio::test(flavor = "multi_thread")]
async fn create_window_400_on_invalid_name_newline() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(
            Method::POST,
            "/api/terminal/windows",
            Some(serde_json::json!({"name": "line1\nline2"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_name");
}

/// Empty-name and overlong-name are also rejected by `validate_window_name`.
#[tokio::test(flavor = "multi_thread")]
async fn create_window_400_on_empty_name() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(
            Method::POST,
            "/api/terminal/windows",
            Some(serde_json::json!({"name": ""})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_name");
}

/// With a valid (or absent) name and tmux disabled, we should land on
/// the 403 tmux-controller gate — i.e. validation passed and we reached
/// the tmux check. Anchors the "happy validation, then config-gated 403"
/// ordering for any future refactor.
#[tokio::test(flavor = "multi_thread")]
async fn create_window_403_when_tmux_disabled_and_name_omitted() {
    let h = boot().await;
    let (status, _) = send(
        &h,
        request(
            Method::POST,
            "/api/terminal/windows",
            Some(serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── /terminal/windows/{window_id} (delete / rename) ──────────────────────

/// `delete_window` validates the path segment BEFORE auth — a malformed
/// id like `not-an-id` (no `@` prefix) must be 400 even on a fully open
/// loopback harness. This is also what protects against path-traversal
/// shapes like `../foo` reaching the tmux command line.
#[tokio::test(flavor = "multi_thread")]
async fn delete_window_400_on_invalid_window_id() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(Method::DELETE, "/api/terminal/windows/not-an-id", None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_id");
}

/// Path-traversal-shaped id is rejected the same way. Url-encoded `..`
/// is `..` once axum decodes it; the validator only allows `@<digits>`.
#[tokio::test(flavor = "multi_thread")]
async fn delete_window_400_on_traversal_shaped_id() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(Method::DELETE, "/api/terminal/windows/@1;ls", None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_id");
}

/// With a syntactically valid id and tmux disabled, we should reach the
/// tmux-controller gate (403) — proving validation passed and auth was
/// satisfied via LocalBypass.
#[tokio::test(flavor = "multi_thread")]
async fn delete_window_403_when_tmux_disabled_and_id_valid() {
    let h = boot().await;
    let (status, _) = send(
        &h,
        request(Method::DELETE, "/api/terminal/windows/@1", None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// `rename_window` validates id first — invalid id is 400 regardless of
/// what the body looks like.
#[tokio::test(flavor = "multi_thread")]
async fn rename_window_400_on_invalid_window_id() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(
            Method::PATCH,
            "/api/terminal/windows/garbage",
            Some(serde_json::json!({"name": "fine"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_id");
}

/// Valid id but invalid new name → 400 with `invalid_window_name`. This
/// is the second validation gate inside `rename_window`.
#[tokio::test(flavor = "multi_thread")]
async fn rename_window_400_on_invalid_new_name() {
    let h = boot().await;
    let (status, body) = send(
        &h,
        request(
            Method::PATCH,
            "/api/terminal/windows/@1",
            Some(serde_json::json!({"name": "with|pipe"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert_eq!(body["error"], "invalid_window_name");
}

/// Both validations clean → tmux-disabled gate fires with 403.
#[tokio::test(flavor = "multi_thread")]
async fn rename_window_403_when_tmux_disabled_and_inputs_valid() {
    let h = boot().await;
    let (status, _) = send(
        &h,
        request(
            Method::PATCH,
            "/api/terminal/windows/@7",
            Some(serde_json::json!({"name": "renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
