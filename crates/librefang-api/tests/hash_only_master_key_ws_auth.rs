//! The WebSocket upgrade path against a hash-only master credential (#6613).
//!
//! `agent_ws` authenticates the upgrade itself rather than relying on the HTTP
//! auth middleware, because a browser cannot set `Authorization` on a WebSocket
//! handshake and the middleware's own comment points at this handler as the
//! place that check lives. That duplication is where the #6613 bypass sat: the
//! handler derived "is auth configured?" as `!valid_api_tokens(..).is_empty()`,
//! a daemon whose master key exists only as `api_key_hash` lists no plaintext
//! token, so the expression reported the daemon as open, the whole
//! `if auth_required` block was skipped, and the upgrade proceeded
//! unauthenticated — re-opening the openfang #1034 B2 branch for the new config
//! shape. A caller presenting the *correct* key was rejected at the same time,
//! because there was nothing to compare against.
//!
//! The router here mounts `agent_ws` **alone**, with no auth middleware layer.
//! That is deliberate: with the middleware in front, a 401 could come from
//! either layer and the assertion would not be attributable to the handler's own
//! derivation — which is the thing that was wrong. Mounted alone, every status
//! below is the handler's verdict.
//!
//! The peer is loopback, for the same reason: loopback is the origin the
//! no-auth-configured branch treats as trusted, so it is exactly where the
//! bypass was reachable. A remote peer would be rejected by that branch
//! regardless of how the credential was derived, and the test would still pass
//! with the fix reverted.

use axum::routing::get;
use axum::Router;
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MASTER_KEY: &str = "hash-only-master-key";

/// A syntactically invalid agent id.
///
/// The success case must not assert 200: after auth, `agent_ws` looks the agent
/// up with five 200 ms retries and then completes a real WebSocket handshake.
/// An unparseable id makes the handler return 400 the moment auth has passed, so
/// "authenticated" and "rejected" are one status apart with no agent fixture and
/// no second of sleeping. Pre-fix, a *missing* bearer also reached this 400 —
/// which is precisely the bug, and why 401-vs-400 is the discriminator these
/// tests turn on.
///
/// 400 rather than 404 because `impl FromStr for AgentId` is
/// `Uuid::parse_str` with no fallback (`librefang-types/src/agent.rs`), so this
/// string fails to parse and never reaches the existence lookup. Worth stating:
/// `AgentId::from_name` *does* derive a UUID v5 from an arbitrary name, and if
/// `FromStr` ever grew that fallback this would parse, survive to the retry
/// loop, and return 404 a second later instead.
const UNPARSEABLE_AGENT_ID: &str = "not-a-uuid";

struct TestServer {
    addr: SocketAddr,
    state: std::sync::Arc<librefang_api::routes::AppState>,
    _test: TestAppState,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot a daemon whose master credential is **only** an `api_key_hash`.
///
/// Both surfaces are set, because production keeps them in step via
/// `server::refresh_master_credential` and a fixture that set one would let the
/// handler see an unconfigured daemon and pass for the wrong reason:
/// `KernelConfig.api_key_hash` is what `auth_snapshot()` reports, and the live
/// `master_key` handle (via `with_api_key_hash`) is what the upgrade path reads.
/// `api_key` stays empty throughout — that emptiness is the trap.
async fn start() -> TestServer {
    let hash = librefang_api::password_hash::hash_device_token(MASTER_KEY);
    let config_hash = hash.clone();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = String::new();
        cfg.api_key_hash = config_hash;
    }))
    .with_api_key_hash(&hash);
    let state = test.state.clone();
    state.kernel.clone().set_self_handle();

    let app = Router::new()
        .route("/api/agents/{id}/ws", get(librefang_api::ws::agent_ws))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    TestServer {
        addr,
        state,
        _test: test,
    }
}

/// Perform a WebSocket handshake over a raw socket and return the status code.
///
/// Raw TCP rather than an HTTP client because `axum::extract::WebSocketUpgrade`
/// rejects the request with 426 before the handler body runs unless the
/// handshake headers are all present and intact — `Connection: Upgrade`,
/// `Upgrade: websocket`, `Sec-WebSocket-Version: 13`, and `Sec-WebSocket-Key`.
/// A general-purpose client manages `Connection` itself, so writing the request
/// bytes is what guarantees the status under test comes from the auth code and
/// not from a mangled handshake. No new dev-dependency, either.
async fn ws_handshake_status(server: &TestServer, bearer: Option<&str>) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(server.addr)
        .await
        .expect("connect to test server");
    let auth_line = match bearer {
        Some(token) => format!("Authorization: Bearer {token}\r\n"),
        None => String::new(),
    };
    let request = format!(
        "GET /api/agents/{UNPARSEABLE_AGENT_ID}/ws HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         {auth_line}\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write handshake");
    let mut buf = [0u8; 256];
    let read = stream.read(&mut buf).await.expect("read response");
    let head = String::from_utf8_lossy(&buf[..read]);
    let status_line = head.lines().next().unwrap_or_default().to_string();
    status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in response: {status_line:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_only_master_key_authenticates_a_websocket_upgrade() {
    let server = start().await;
    assert_eq!(
        ws_handshake_status(&server, Some(MASTER_KEY)).await,
        400,
        "the key behind api_key_hash must authenticate the upgrade — 400 is the \
         unparseable-agent-id rejection that follows a *passed* auth check, and \
         401 here would mean a hash-only daemon cannot authenticate anyone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_only_master_key_rejects_a_wrong_bearer_on_a_websocket_upgrade() {
    let server = start().await;
    assert_eq!(
        ws_handshake_status(&server, Some("wrong-key")).await,
        401,
        "a token that does not verify against api_key_hash must be rejected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_only_master_key_rejects_a_missing_bearer_from_loopback() {
    // The case with teeth, and the one that motivated the fix. Pre-fix,
    // `auth_required` was false on a hash-only daemon, the loopback peer
    // satisfied the no-auth branch, the whole auth block was skipped, and this
    // returned 400 — meaning an anonymous caller had reached the agent lookup
    // on a daemon the operator believes is bearer-gated.
    let server = start().await;
    assert_eq!(
        ws_handshake_status(&server, None).await,
        401,
        "a hash-gated daemon must not fall back to the unauthenticated loopback \
         upgrade"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_only_master_key_rejects_an_empty_bearer_on_a_websocket_upgrade() {
    // `api_key_lock` holds "" on a hash-only daemon. Were the empty candidate
    // not filtered out of the `\n`-joined composite, a constant-time compare of
    // two empty slices would authenticate `Authorization: Bearer `.
    let server = start().await;
    assert_eq!(ws_handshake_status(&server, Some("")).await, 401);
}
