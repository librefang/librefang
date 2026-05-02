//! Additional integration tests for the `routes::mcp_auth` handlers,
//! complementing the cases already covered in `mcp_oauth_flow_test.rs`.
//!
//! Refs #3571 (mcp_auth slice). The existing flow test covers the core
//! `auth_status` / `auth_callback` / `auth_revoke` happy + a handful of
//! reject paths. This file fills gaps the issue called out — exercising
//! code paths that are otherwise dead-code from a test perspective:
//!
//! * `auth_status` for every seeded `McpAuthState` variant (the dashboard
//!   discriminates on the `state` tag — a typo or rename here would
//!   silently break the UI).
//! * `auth_start` early-exit branches that are reachable without the
//!   live `.well-known` discovery network round-trip (unknown server,
//!   stdio transport).
//! * `auth_callback` validation paths past the format gate but before
//!   the outbound token-exchange (stdio transport, missing PKCE state,
//!   empty flow id).
//!
//! Out of scope (would require either a mock HTTP server bound to a real
//! port or outbound network — both unsafe in parallel test binaries):
//! * `auth_start` happy path (needs `.well-known` discovery).
//! * `auth_callback` happy path (needs an authorization server to
//!   exchange the code).
//! * `auth_callback` SSRF / host-pin guards past PKCE state load
//!   (would need a real `KernelOAuthProvider` vault seeded with
//!   `LIBREFANG_VAULT_KEY`, which is global env mutation).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_runtime::mcp_oauth::McpAuthState;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{McpServerConfigEntry, McpTransportEntry};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn entry_http(name: &str, url: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Http {
            url: url.to_string(),
        }),
        timeout_secs: 30,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

fn entry_stdio(name: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Stdio {
            command: "/bin/true".to_string(),
            args: Vec::new(),
        }),
        timeout_secs: 30,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

fn boot_with_servers(servers: Vec<McpServerConfigEntry>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.mcp_servers.extend(servers.clone());
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::skills::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn send(h: &Harness, method: Method, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, body) = send(h, Method::GET, path).await;
    let v = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    (status, v)
}

// ---------------------------------------------------------------------------
// auth_status — every seeded state variant
// ---------------------------------------------------------------------------

/// `Authorized` must serialize with the snake_case discriminator
/// `state = "authorized"`. The dashboard's `useMcpAuthStatus` query
/// branches on this tag to render the "Connected" badge.
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_authorized_state_serializes_with_snake_case_tag() {
    let h = boot_with_servers(vec![entry_http("srv-a", "https://example.invalid/mcp")]);
    {
        let mut states = h.state.kernel.mcp_auth_states_ref().lock().await;
        states.insert(
            "srv-a".to_string(),
            McpAuthState::Authorized {
                expires_at: Some("2099-01-01T00:00:00Z".to_string()),
                tokens: None,
            },
        );
    }
    let (status, body) = get_json(&h, "/api/mcp/servers/srv-a/auth/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["server"], "srv-a");
    assert_eq!(body["auth"]["state"], "authorized");
    assert_eq!(body["auth"]["expires_at"], "2099-01-01T00:00:00Z");
}

/// `NeedsAuth` is the post-revoke / post-401-detection state. The UI
/// surfaces a "Sign in" CTA from this tag; misspelling it would leave
/// users stranded with no way to start the flow.
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_needs_auth_state_serializes_with_snake_case_tag() {
    let h = boot_with_servers(vec![entry_http("srv-b", "https://example.invalid/mcp")]);
    {
        let mut states = h.state.kernel.mcp_auth_states_ref().lock().await;
        states.insert("srv-b".to_string(), McpAuthState::NeedsAuth);
    }
    let (status, body) = get_json(&h, "/api/mcp/servers/srv-b/auth/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["auth"]["state"], "needs_auth");
}

/// `Error { message }` must surface the operator-facing message in the
/// JSON payload. The dashboard renders this directly so the user can
/// distinguish "vault locked" from "discovery failed" etc.
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_error_state_includes_message() {
    let h = boot_with_servers(vec![entry_http("srv-c", "https://example.invalid/mcp")]);
    {
        let mut states = h.state.kernel.mcp_auth_states_ref().lock().await;
        states.insert(
            "srv-c".to_string(),
            McpAuthState::Error {
                message: "discovery failed: connection refused".to_string(),
            },
        );
    }
    let (status, body) = get_json(&h, "/api/mcp/servers/srv-c/auth/status").await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["auth"]["state"], "error");
    assert_eq!(
        body["auth"]["message"],
        "discovery failed: connection refused"
    );
}

// ---------------------------------------------------------------------------
// auth_start — early-exit paths that don't need network
// ---------------------------------------------------------------------------

/// `auth_start` for an unknown server name must 404 before any network
/// I/O. A 200 here would let unauthenticated callers trigger
/// `.well-known` probes against arbitrary URLs they didn't configure.
#[tokio::test(flavor = "multi_thread")]
async fn auth_start_unknown_server_is_404() {
    let h = boot_with_servers(Vec::new());
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/mcp/servers/does-not-exist/auth/start",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
}

/// `auth_start` for a stdio-transport server must 400 with a clear
/// message. OAuth is meaningless for subprocess transport — letting
/// the call fall through to discovery would attempt to fetch
/// `.well-known/oauth-authorization-server` against the (empty) URL
/// and emit a confusing network error to the dashboard.
#[tokio::test(flavor = "multi_thread")]
async fn auth_start_stdio_transport_is_rejected_with_400() {
    let h = boot_with_servers(vec![entry_stdio("local-stdio")]);
    let (status, body) = send(&h, Method::POST, "/api/mcp/servers/local-stdio/auth/start").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(
        body.to_lowercase().contains("http") && body.to_lowercase().contains("sse"),
        "expected HTTP/SSE-transport error message, got: {body}"
    );
    // Must not have polluted auth state — the rejection is pre-discovery.
    let states = h.state.kernel.mcp_auth_states_ref().lock().await;
    assert!(
        !states.contains_key("local-stdio"),
        "stdio rejection must not write auth state, got {states:?}"
    );
}

// ---------------------------------------------------------------------------
// auth_callback — paths past format-gate but before outbound network
// ---------------------------------------------------------------------------

/// Callback for a stdio-transport server reaches the transport-match
/// branch and returns the "no HTTP/SSE transport" failure. Important
/// because a missing match arm here would 500 instead of returning the
/// browser-friendly text response.
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_stdio_transport_returns_no_http_sse_message() {
    let h = boot_with_servers(vec![entry_stdio("local-stdio")]);
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/mcp/servers/local-stdio/auth/callback?code=abc&state=flow.nonce",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Authorization Failed"),
        "expected failure text, got: {body}"
    );
    assert!(
        body.contains("HTTP/SSE") || body.to_lowercase().contains("transport"),
        "expected transport error detail, got: {body}"
    );
}

/// Callback whose `state` is well-formed (`flow.nonce`) but whose
/// PKCE state is absent from the vault must fail with an explanatory
/// message. This is the dominant real-world failure mode when
/// `LIBREFANG_VAULT_KEY` is missing — the user retries from the
/// dashboard and the message tells them why.
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_valid_format_but_no_pkce_state_in_vault_fails() {
    let h = boot_with_servers(vec![entry_http("srv-d", "https://example.invalid/mcp")]);
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/mcp/servers/srv-d/auth/callback?code=abc&state=fl0w.n0nce",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Authorization Failed"),
        "expected failure text, got: {body}"
    );
    // The current handler surfaces "No pending auth flow" with a
    // LIBREFANG_VAULT_KEY hint.
    assert!(
        body.contains("No pending auth flow") || body.to_lowercase().contains("pkce"),
        "expected pkce/missing-flow detail, got: {body}"
    );
}

/// Callback with `state=".nonce"` (empty flow_id, present separator)
/// must be rejected by the malformed-state gate. The split_once check
/// requires a non-empty flow_id; a regression that accepted this would
/// allow a single global vault key collision across all flows.
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_empty_flow_id_is_malformed() {
    let h = boot_with_servers(vec![entry_http("srv-e", "https://example.invalid/mcp")]);
    let (status, body) = send(
        &h,
        Method::GET,
        "/api/mcp/servers/srv-e/auth/callback?code=abc&state=.justnonce",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Authorization Failed"),
        "expected failure text, got: {body}"
    );
    assert!(
        body.contains("Malformed state") || body.contains("flow ID"),
        "expected malformed-state error, got: {body}"
    );
    let states = h.state.kernel.mcp_auth_states_ref().lock().await;
    assert!(
        !states.contains_key("srv-e"),
        "malformed-state probe must not mutate auth state"
    );
}
