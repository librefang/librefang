//! An MCP server that will not start has to say why, and has to say it somewhere the operator is already looking.
//!
//! Three defects, all reproduced against a live daemon before they were written down here.
//!
//! 1. `POST /api/mcp/servers/{name}/reconnect` answered **500** for an external MCP server that never completed its handshake.
//!    A dependency that did not answer is not an internal fault of this daemon, and a 500 sends the operator looking for the problem in the wrong process.
//! 2. The daemon knew the reason — `MCP handshake failed for 'npx': connection closed: initialize response` was in the journal — but the response body was the generic scrub, so the only way to learn it was SSH.
//! 3. The dashboard's Logs page reads the audit trail, never the daemon's `tracing` output, so a failing MCP server left that screen empty by construction.
//!
//! The scrub itself is not the bug and is not touched: an MCP error can quote a URL with a token in its query or an authenticated server's response body.
//! What travels now is a typed reason — failure class, transport kind, and the endpoint with its secret-bearing parts removed — which `raw_reconnect_error_never_reaches_the_client` pins.
//!
//! Every test here drives the **full** `server::build_router`, not a hand-picked route subset, and asserts `content-type` alongside the status: a test that reaches axum's fallback instead of the handler answers 404 with no body, and without the content-type assertion that is indistinguishable from a real answer.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::mcp_oauth::McpAuthState;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{McpServerConfigEntry, McpTransportEntry};
use std::sync::Arc;
use tower::ServiceExt;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Distinctive enough that a substring search over a whole response body is a meaningful assertion.
const SENTINEL: &str = "tok_S3nt1nelMustNeverAppearInAnyResponseBody";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot_with_servers(servers: Vec<McpServerConfigEntry>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.mcp_servers.extend(servers.clone());
    }));
    let state = test.state.clone();
    state.kernel.clone().set_self_handle();

    // The production router, so a missing registration in `server.rs` fails here
    // rather than passing against a fallback. `build_router` does not call
    // `start_background_agents`, so nothing dials an MCP server behind our back:
    // no boot-time connect, no health loop, and therefore no audit entry this
    // test did not ask for.
    let (app, _built_state) = server::build_router(
        state.kernel.clone(),
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    Harness {
        app,
        state,
        _test: test,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// A stdio server whose command exits immediately, so the connect fails fast and offline.
/// The argument and the environment entry carry [`SENTINEL`]: they are the parts of a stdio
/// transport that can hold a credential, and no response may echo them.
fn entry_stdio_that_fails(name: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Stdio {
            command: "false".to_string(),
            args: vec![format!("--token={SENTINEL}")],
        }),
        timeout_secs: 5,
        env: vec![format!("MCP_TOKEN={SENTINEL}")],
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

/// An HTTP server on a port nothing listens on, with the credential in the query string —
/// the URL shape `connect_target` has to reduce to scheme, host and port.
fn entry_http_that_fails(name: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Http {
            url: format!("http://127.0.0.1:1/mcp?token={SENTINEL}"),
        }),
        timeout_secs: 5,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

/// The same failure with the credential in **userinfo** rather than the query.
///
/// `scrub_url` builds its answer from `host_str()`, which excludes userinfo, so this
/// passes today — it is a guard, not a demonstration. It earns its place because the
/// obvious refactor breaks it: `Url::authority()` folds scheme/host/port into one call
/// and *does* include `user:password@`, so anyone merging the three URL arms into one
/// would reintroduce the leak with every existing test still green.
fn entry_http_with_userinfo(name: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Http {
            url: format!("http://user:{SENTINEL}@127.0.0.1:1/mcp"),
        }),
        timeout_secs: 5,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

/// A server whose endpoint answers 401, configured so OAuth discovery resolves
/// without any OAuth server at all.
///
/// The three discovery tiers make this cheap, which is worth spelling out because
/// "this needs a mock OAuth server" was the reason this branch nearly shipped
/// untested. Tier 1 needs a `resource_metadata` parameter inside
/// `WWW-Authenticate`, which the mock deliberately omits. Tier 2 fetches
/// `/.well-known/oauth-authorization-server`, and `well_known_url` returns `None`
/// outright for a loopback host under the #3592 SSRF guard — wiremock binds
/// 127.0.0.1, so that tier never makes a request. Tier 3 is a pure config
/// fallback: `auth_url` + `token_url` and nothing else.
fn entry_http_needing_oauth(name: &str, url: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: Some(McpTransportEntry::Http {
            url: url.to_string(),
        }),
        timeout_secs: 5,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: Some(librefang_types::config::McpOAuthConfig {
            auth_url: Some("https://oauth.invalid/authorize".to_string()),
            token_url: Some("https://oauth.invalid/token".to_string()),
            ..Default::default()
        }),
        taint_scanning: true,
        taint_policy: None,
    }
}

/// A configured entry with no transport block — nothing to dial.
fn entry_without_transport(name: &str) -> McpServerConfigEntry {
    McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: None,
        timeout_secs: 5,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

struct Answer {
    status: StatusCode,
    content_type: String,
    body: String,
    json: serde_json::Value,
}

async fn reconnect(h: &Harness, name: &str) -> Answer {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/mcp/servers/{name}/reconnect"))
        .body(Body::empty())
        .expect("request builds");
    // Load-bearing, not incidental: the auth layer treats a missing `ConnectInfo`
    // as a non-loopback peer and fails closed with 401, and this route is
    // Admin-gated. Without a loopback peer every test below would assert against
    // a 401 that the handler never produced.
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            12345,
        ))));
    let resp = h.app.clone().oneshot(req).await.expect("router responds");
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body reads");
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    Answer {
        status,
        content_type,
        body,
        json,
    }
}

/// Number of MCP connect failures currently in the audit trail for `name`.
fn mcp_connect_failures(h: &Harness, name: &str) -> usize {
    h.state
        .kernel
        .audit()
        .recent(1000)
        .iter()
        .filter(|e| {
            matches!(e.action, librefang_kernel::audit::AuditAction::McpConnect)
                && e.detail.contains(name)
        })
        .count()
}

// ---------------------------------------------------------------------------
// Defect 1 — whose fault the failure is
// ---------------------------------------------------------------------------

/// A stdio MCP server that exits before completing the handshake is an external
/// dependency that did not answer, so the status has to be 502 and not 500.
#[tokio::test(flavor = "multi_thread")]
async fn stdio_server_that_never_answers_is_a_bad_gateway_not_an_internal_error() {
    let h = boot_with_servers(vec![entry_stdio_that_fails("wedged-stdio")]).await;

    let answer = reconnect(&h, "wedged-stdio").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "the handler must have answered, not axum's fallback: content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(
        answer.status,
        StatusCode::BAD_GATEWAY,
        "an MCP server that does not answer is not this daemon's fault: {}",
        answer.body
    );
    assert_eq!(answer.json["error"]["code"], "mcp_connect_failed");
}

/// Same verdict for a URL transport nothing is listening on.
#[tokio::test(flavor = "multi_thread")]
async fn http_server_that_refuses_the_connection_is_a_bad_gateway() {
    let h = boot_with_servers(vec![entry_http_that_fails("wedged-http")]).await;

    let answer = reconnect(&h, "wedged-http").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(
        answer.status,
        StatusCode::BAD_GATEWAY,
        "an endpoint that refuses the connection is not this daemon's fault: {}",
        answer.body
    );
    assert_eq!(answer.json["error"]["code"], "mcp_connect_failed");
}

/// The two failures that are about the server's own stored configuration are neither
/// a 500 nor a 502 — nothing upstream was ever dialed, and no retry will help until
/// an operator changes something. `code`, not the status, is what tells them apart.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_with_no_transport_is_a_conflict_not_an_internal_error() {
    let h = boot_with_servers(vec![entry_without_transport("no-transport")]).await;

    let answer = reconnect(&h, "no-transport").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(
        answer.status,
        StatusCode::CONFLICT,
        "an entry with nothing to dial is a config conflict: {}",
        answer.body
    );
    assert_eq!(answer.json["error"]["code"], "mcp_transport_missing");
}

/// The case above seeds `NeedsAuth` by hand, so it only exercises the pre-check.
/// This one is the reconnect that *discovers* the 401 — the branch where `connect`
/// returns the `OAUTH_NEEDS_AUTH` sentinel rather than an error message.
///
/// Three effects, all asserted, because the sibling branch in `connect_mcp_servers`
/// only demonstrates one of them: it logs and records state, while this path also
/// has to answer with a different error class and write to `mcp_auth_states` — a
/// map the reconnect path did not touch before.
#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_that_discovers_a_401_answers_needs_auth_and_records_the_state() {
    let upstream = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(401).insert_header("www-authenticate", r#"Bearer realm="mcp""#),
        )
        .mount(&upstream)
        .await;

    let h = boot_with_servers(vec![entry_http_needing_oauth(
        "discovers-401",
        &upstream.uri(),
    )])
    .await;

    // Nothing has seeded the map: the pre-check must fall through to the connect.
    assert!(
        h.state
            .kernel
            .mcp_auth_states_ref()
            .lock()
            .await
            .get("discovers-401")
            .is_none(),
        "the map must start empty or this test proves nothing"
    );

    let answer = reconnect(&h, "discovers-401").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(
        answer.status,
        StatusCode::CONFLICT,
        "a server asking for a sign-in did not fail to answer: {}",
        answer.body
    );
    assert_eq!(
        answer.json["error"]["code"], "mcp_needs_auth",
        "{}",
        answer.body
    );

    assert!(
        matches!(
            h.state
                .kernel
                .mcp_auth_states_ref()
                .lock()
                .await
                .get("discovers-401"),
            Some(McpAuthState::NeedsAuth)
        ),
        "the discovered state has to be recorded, or the dashboard never offers the sign-in"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_awaiting_oauth_is_a_conflict_not_an_internal_error() {
    let h = boot_with_servers(vec![entry_http_that_fails("pending-oauth")]).await;
    {
        let mut states = h.state.kernel.mcp_auth_states_ref().lock().await;
        states.insert("pending-oauth".to_string(), McpAuthState::NeedsAuth);
    }

    let answer = reconnect(&h, "pending-oauth").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(
        answer.status,
        StatusCode::CONFLICT,
        "an unfinished OAuth flow is not a server fault: {}",
        answer.body
    );
    assert_eq!(answer.json["error"]["code"], "mcp_needs_auth");
}

/// A server nobody configured is still a 404 — the pre-check that answers it is what
/// keeps `McpReconnectError::NotConfigured` unreachable through this route, and this
/// pins that it stays a 404 rather than drifting into the new error mapping.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_server_is_still_not_found() {
    let h = boot_with_servers(Vec::new()).await;

    let answer = reconnect(&h, "never-heard-of-it").await;

    assert!(
        answer.content_type.starts_with("application/json"),
        "content-type {:?}, body {}",
        answer.content_type,
        answer.body
    );
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.body);
}

// ---------------------------------------------------------------------------
// Defect 2 — the reason reaching the caller, and only the safe part of it
// ---------------------------------------------------------------------------

/// The operator has to be able to read *what* failed and *what was dialed* out of the
/// response, without opening a terminal on the host.
#[tokio::test(flavor = "multi_thread")]
async fn the_failure_body_names_the_transport_and_the_command() {
    let h = boot_with_servers(vec![entry_stdio_that_fails("wedged-stdio")]).await;

    let answer = reconnect(&h, "wedged-stdio").await;

    assert_eq!(
        answer.json["details"]["id"], "wedged-stdio",
        "{}",
        answer.body
    );
    assert_eq!(
        answer.json["details"]["transport"], "stdio",
        "{}",
        answer.body
    );
    assert_eq!(
        answer.json["details"]["target"], "false",
        "the program name is the actionable half of a stdio failure: {}",
        answer.body
    );
    assert_ne!(
        answer.json["error"]["message"], "Internal server error",
        "the generic scrub tells the operator nothing: {}",
        answer.body
    );
}

/// A URL transport reports scheme, host and port — never the path or the query.
#[tokio::test(flavor = "multi_thread")]
async fn a_url_target_is_reduced_to_scheme_host_and_port() {
    let h = boot_with_servers(vec![entry_http_that_fails("wedged-http")]).await;

    let answer = reconnect(&h, "wedged-http").await;

    assert_eq!(
        answer.json["details"]["transport"], "http",
        "{}",
        answer.body
    );
    assert_eq!(
        answer.json["details"]["target"], "http://127.0.0.1:1",
        "path and query are where a token lives, so they must be gone: {}",
        answer.body
    );
}

/// The boundary the typed reason exists to respect: whatever the runtime said, and
/// whatever the operator put in the transport's arguments, environment or query
/// string, none of it may appear in the response.
///
/// This is the test that fails if someone later "improves" the message by echoing
/// `e.to_string()`.
#[tokio::test(flavor = "multi_thread")]
async fn raw_reconnect_error_never_reaches_the_client() {
    let h = boot_with_servers(vec![
        entry_stdio_that_fails("wedged-stdio"),
        entry_http_that_fails("wedged-http"),
        entry_http_with_userinfo("wedged-userinfo"),
    ])
    .await;

    for name in ["wedged-stdio", "wedged-http", "wedged-userinfo"] {
        let answer = reconnect(&h, name).await;
        assert!(
            !answer.body.contains(SENTINEL),
            "{name} leaked a credential into the response body: {}",
            answer.body
        );
    }
}

// ---------------------------------------------------------------------------
// Defect 3 — the failure reaching the screen the operator actually opens
// ---------------------------------------------------------------------------

/// The Logs page renders the audit trail, so an MCP failure that records nothing there
/// is invisible to everyone without shell access. The delta is measured across one
/// reconnect rather than asserting "at least one entry exists", because a bare
/// existence check would also pass on an entry left over from anything else.
///
/// **Deliberately says nothing about the status code.** The first draft asserted 502
/// here as well, and on the pre-fix baseline that assertion fired first — so the test
/// went red without its own subject ever being evaluated, and would have gone green the
/// moment the status was fixed whether or not anything was ever audited. The status is
/// the other tests' job; this one has to fail for exactly one reason.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_mcp_connect_is_recorded_in_the_audit_trail() {
    let h = boot_with_servers(vec![entry_stdio_that_fails("wedged-stdio")]).await;

    let before = mcp_connect_failures(&h, "wedged-stdio");
    let _ = reconnect(&h, "wedged-stdio").await;
    let after = mcp_connect_failures(&h, "wedged-stdio");

    assert_eq!(
        after,
        before + 1,
        "the reconnect failure has to leave a trace on the Logs page, not only in the journal"
    );

    let entry = h
        .state
        .kernel
        .audit()
        .recent(1000)
        .into_iter()
        .rev()
        .find(|e| matches!(e.action, librefang_kernel::audit::AuditAction::McpConnect))
        .expect("the entry counted above is readable");

    assert!(
        entry.outcome.starts_with("error"),
        "the dashboard derives a row's level from the outcome text, so anything else renders this failure as info: {:?}",
        entry.outcome
    );
    assert!(
        entry.detail.contains("wedged-stdio") && entry.detail.contains("stdio"),
        "the entry has to name the server and its transport to be worth reading: {:?}",
        entry.detail
    );
    assert!(
        !entry.detail.contains(SENTINEL),
        "the audit trail is served over HTTP too, so the same boundary applies: {:?}",
        entry.detail
    );
}
