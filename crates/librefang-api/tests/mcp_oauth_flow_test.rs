//! Integration tests for the MCP OAuth callback / status / revoke endpoints.
//!
//! Targets the route handlers in `routes::mcp_auth`. Issue references:
//! #3402, #3403.
//!
//! Scope notes:
//! - `auth_start` is intentionally NOT exercised here — it requires a live
//!   `.well-known` discovery against the configured MCP server URL, which
//!   would either need a mock HTTP server stood up per test or would race
//!   on outbound network. The other four endpoints (`status`, `callback`,
//!   `revoke`) cover the security-critical state transitions.
//! - The skills router is mounted directly under `/api`, mirroring the
//!   pairing tests — this skips the global auth middleware so the tests
//!   focus on handler behaviour.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot_with_mcp_server(name: &str, url: &str) -> Harness {
    let name = name.to_string();
    let url = url.to_string();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.mcp_servers
            .push(librefang_types::config::McpServerConfigEntry {
                name: name.clone(),
                template_id: None,
                transport: Some(librefang_types::config::McpTransportEntry::Http {
                    url: url.clone(),
                }),
                timeout_secs: 30,
                env: Vec::new(),
                headers: Vec::new(),
                oauth: None,
                taint_scanning: true,
                taint_policy: None,
            });
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

fn boot_no_servers() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
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

async fn get(h: &Harness, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    (status, body)
}

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, body) = get(h, path).await;
    let value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn delete(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .body(Body::empty())
        .unwrap();
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
// auth_status
// ---------------------------------------------------------------------------

/// Status for an unknown MCP server name must 404 — the dashboard infers
/// "server not yet installed" from this response, and a default-200 with a
/// fake state would mask wiring bugs.
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_unknown_server_is_404() {
    let h = boot_no_servers();
    let (status, body) = get_json(&h, "/api/mcp/servers/does-not-exist/auth/status").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got body: {body:?}");
}

/// For a configured but never-connected server with no recorded auth state,
/// the handler reports `state = "unknown"` (not `"not_required"` — that
/// label is reserved for servers known to be reachable without OAuth).
#[tokio::test(flavor = "multi_thread")]
async fn auth_status_known_server_with_no_state_is_unknown() {
    let h = boot_with_mcp_server("test-srv", "https://example.invalid/mcp");
    let (status, body) = get_json(&h, "/api/mcp/servers/test-srv/auth/status").await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");
    assert_eq!(body["server"], "test-srv");
    assert_eq!(
        body["auth"]["state"], "unknown",
        "no in-memory state and no live connection => 'unknown', got {body:?}"
    );
}

// ---------------------------------------------------------------------------
// auth_callback
// ---------------------------------------------------------------------------

/// Callback without a `state` query parameter must be rejected — the state
/// param is the only proof that this callback was initiated by a flow on
/// this daemon. A missing-state path that progressed any further would be
/// a CSRF foothold (#3730).
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_missing_state_is_rejected() {
    let h = boot_with_mcp_server("test-srv", "https://example.invalid/mcp");
    let (status, body) = get(&h, "/api/mcp/servers/test-srv/auth/callback?code=abc").await;
    // The handler responds 200 with text/plain "Authorization Failed" — a
    // browser-friendly response rather than a JSON error. The security
    // contract is that no auth state was mutated.
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Authorization Failed"),
        "expected failure text, got: {body}"
    );
    assert!(
        body.to_lowercase().contains("missing state"),
        "expected explanatory message, got: {body}"
    );

    // No auth state should have been recorded — a missing-state probe must
    // not be able to write to the auth_states map.
    let states = h.state.kernel.mcp_auth_states_ref().lock().await;
    assert!(
        !states.contains_key("test-srv"),
        "auth state must not be created from a missing-state callback, got {states:?}"
    );
}

/// Callback with a state value that lacks the `{flow_id}.{nonce}` separator
/// must be rejected before any vault lookup — the malformed state proves
/// the call did not originate from `auth_start` on this daemon.
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_malformed_state_is_rejected() {
    let h = boot_with_mcp_server("test-srv", "https://example.invalid/mcp");
    let (status, body) = get(
        &h,
        "/api/mcp/servers/test-srv/auth/callback?code=abc&state=no-dot-here",
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
}

/// Callback for an unknown server name must fail — the path-level lookup
/// is the first gate, so a callback hitting a non-existent server can't
/// poison auth state for any real one.
#[tokio::test(flavor = "multi_thread")]
async fn auth_callback_unknown_server_fails() {
    let h = boot_no_servers();
    let (status, body) = get(
        &h,
        "/api/mcp/servers/ghost/auth/callback?code=abc&state=flow.nonce",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Authorization Failed"),
        "expected failure text, got: {body}"
    );
    assert!(
        body.contains("not found"),
        "expected 'not found' detail, got: {body}"
    );
}

// ---------------------------------------------------------------------------
// auth_revoke
// ---------------------------------------------------------------------------

/// Revoke for a known server must return 200 and reset the in-memory auth
/// state to `NeedsAuth`. The dashboard relies on this transition to
/// surface the "Sign in" CTA after the user clicks "Sign out".
#[tokio::test(flavor = "multi_thread")]
async fn auth_revoke_known_server_resets_state_to_needs_auth() {
    let h = boot_with_mcp_server("test-srv", "https://example.invalid/mcp");

    // Seed an "authorized" state so we can observe the transition.
    {
        let mut states = h.state.kernel.mcp_auth_states_ref().lock().await;
        states.insert(
            "test-srv".to_string(),
            librefang_runtime::mcp_oauth::McpAuthState::Authorized {
                expires_at: None,
                tokens: None,
            },
        );
    }

    let (status, body) = delete(&h, "/api/mcp/servers/test-srv/auth/revoke").await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");
    assert_eq!(body["server"], "test-srv");

    let states = h.state.kernel.mcp_auth_states_ref().lock().await;
    let s = states.get("test-srv").expect("state retained after revoke");
    let serialized = serde_json::to_value(s).unwrap();
    assert_eq!(
        serialized["state"], "needs_auth",
        "revoke must transition to needs_auth, got {serialized:?}"
    );
}

/// Revoke against an unknown server must 404 — silently 200'ing here would
/// hide config typos and let the UI think it just signed out a server that
/// was never installed.
#[tokio::test(flavor = "multi_thread")]
async fn auth_revoke_unknown_server_is_404() {
    let h = boot_no_servers();
    let (status, body) = delete(&h, "/api/mcp/servers/does-not-exist/auth/revoke").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got body: {body:?}");
}

// ---------------------------------------------------------------------------
// store_oauth_metadata wiring
//
// The OAuth callback handler (`auth_callback` in `routes::mcp_auth`) calls
// `state.kernel.oauth_provider_ref().store_oauth_metadata(...)` AFTER a
// successful token exchange and BEFORE the per-flow PKCE cleanup loop. This
// promotes `token_endpoint` (and an optional DCR `client_id`) from the
// per-flow staging namespace into the durable per-server namespace that
// `KernelOAuthProvider::try_refresh` reads from.
//
// Driving the full callback path end-to-end would require mocking the OAuth
// token endpoint at a loopback HTTP URL — which the SSRF guard
// (`is_ssrf_blocked_url`) explicitly forbids. So we test the wiring at the
// trait surface instead: we exercise the same `oauth_provider_ref()` that
// the callback handler resolves, against the same kernel home directory, and
// assert the side effect on the shared vault. This protects against future
// refactors that swap the trait provider for a no-op shim or repoint the
// home directory away from where `try_refresh` looks.
// ---------------------------------------------------------------------------

/// `oauth_provider_ref().store_oauth_metadata` MUST persist `token_endpoint`
/// and `client_id` under the bare per-server vault namespace (`{server_url}/...`)
/// — the same keys `try_refresh` reads from. If the trait provider were
/// silently replaced with a no-op, refresh would fail on the first
/// access-token expiry and this test would catch it before users do.
#[tokio::test(flavor = "multi_thread")]
async fn store_oauth_metadata_via_kernel_writes_bare_namespace() {
    use librefang_kernel::mcp_oauth_provider::KernelOAuthProvider;

    let h = boot_no_servers();

    let server_url = "https://mcp.example.com/mcp";
    let token_endpoint = "https://mcp.example.com/token";
    let client_id = "dcr-client-xyz";

    // Drive the same path the callback uses to reach the trait provider.
    h.state
        .kernel
        .oauth_provider_ref()
        .store_oauth_metadata(server_url, token_endpoint, Some(client_id))
        .await
        .expect("store_oauth_metadata via trait provider");

    // Read back through a fresh `KernelOAuthProvider` rooted at the kernel's
    // own home_dir — this mirrors how `try_refresh` resolves the vault and
    // confirms the callback's promotion lands on the keys refresh reads.
    let provider = KernelOAuthProvider::new(h.state.kernel.home_dir().to_path_buf());

    assert_eq!(
        provider
            .vault_get(&KernelOAuthProvider::vault_key(
                server_url,
                "token_endpoint"
            ))
            .expect("vault_get token_endpoint"),
        Some(token_endpoint.to_string()),
        "token_endpoint must be readable under {{server_url}}/token_endpoint — \
         this is the key try_refresh reads from"
    );
    assert_eq!(
        provider
            .vault_get(&KernelOAuthProvider::vault_key(server_url, "client_id"))
            .expect("vault_get client_id"),
        Some(client_id.to_string()),
        "DCR client_id must be readable under {{server_url}}/client_id for refresh"
    );
}

/// Bonus: `try_refresh`'s precondition — a stored `token_endpoint` —
/// is satisfied by the trait wiring. The kernel-side `try_refresh` is
/// private, so we assert the closest observable proxy: after the
/// callback's `store_oauth_metadata` runs, `vault_get_or_warn` resolves
/// the same key `try_refresh` reads, returning `Some(_)` rather than
/// `None` (which would surface as `MissingTokenEndpoint` /
/// "No token_endpoint stored for refresh" at refresh time).
#[tokio::test(flavor = "multi_thread")]
async fn store_oauth_metadata_unblocks_try_refresh_token_endpoint_lookup() {
    use librefang_kernel::mcp_oauth_provider::KernelOAuthProvider;

    let h = boot_no_servers();
    let server_url = "https://mcp.example.com/mcp";
    let token_endpoint = "https://mcp.example.com/token";

    h.state
        .kernel
        .oauth_provider_ref()
        .store_oauth_metadata(server_url, token_endpoint, None)
        .await
        .expect("store_oauth_metadata");

    let provider = KernelOAuthProvider::new(h.state.kernel.home_dir().to_path_buf());
    let lookup = provider.vault_get_or_warn(&KernelOAuthProvider::vault_key(
        server_url,
        "token_endpoint",
    ));
    assert_eq!(
        lookup,
        Some(token_endpoint.to_string()),
        "try_refresh's `vault_get_or_warn(...token_endpoint)` must return the \
         promoted value — None here would mean refresh fails with \
         'No token_endpoint stored for refresh'"
    );
}
