//! Owner-scoping on the agent-chat WebSocket upgrade (#6753 follow-up).
//!
//! `middleware::min_role_for_privileged_get` lowers `/api/agents/{id}/ws` to `UserRole::User` because the socket grants the same capability as `POST /api/agents/{id}/message`.
//! The REST twin pairs that role with an explicit `can_access_agent` call (`routes/agents/messaging.rs`), and the upgrade handler used to carry only an existence check — so any `User`-role token could open a socket on an agent authored by someone else and drive a full LLM turn on it.
//! These tests pin the two halves of the rule the REST twin already satisfies: a non-owner `User` is refused, and the owner is still let through.
//!
//! The router mounts `agent_ws` **alone**, with no auth middleware layer, for the same reason `hash_only_master_key_ws_auth.rs` does: with the middleware in front a 404 could be attributed to either layer, and the handler's own authorization is the thing under test.
//! `agent_ws` authenticates the bearer itself against the kernel's `[[users]]` snapshot, so the identity the assertions turn on is resolved entirely inside the handler.
//!
//! Raw TCP rather than an HTTP client because `axum::extract::WebSocketUpgrade` answers 426 before the handler body runs unless `Connection`, `Upgrade`, `Sec-WebSocket-Version` and `Sec-WebSocket-Key` are all intact, and a general-purpose client manages `Connection` itself.
//! No `Origin` header is sent, which the `validate_ws_origin` step treats as a non-browser client and lets through, so every status below is the authorization verdict.

use axum::routing::get;
use axum::Router;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::UserConfig;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ALICE_KEY: &str = "alice-ws-owner-scope-key";
const BOB_KEY: &str = "bob-ws-owner-scope-key";
const ADMIN_KEY: &str = "admin-ws-owner-scope-key";

/// The upgrade completed, so the caller reached the socket that drives agent turns.
const UPGRADED: u16 = 101;
/// The not-found shape the REST twin uses for a non-owner, deliberately not 403: a `User` who does not own the agent must not be able to confirm from the status that the id exists.
const REFUSED: u16 = 404;

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

/// Boot a daemon with three `[[users]]` api keys and no master credential.
///
/// The keys live in `KernelConfig.users`, not only on `AppState`, because the upgrade path resolves them through `server::configured_user_api_keys(&auth_snapshot)` rather than from the middleware's table.
/// Leaving `api_key` empty keeps the master credential out of the picture while `auth_required` still holds — `master_auth_required` is satisfied by a non-empty user-key list — so the handler resolves a real named principal for every request below instead of the synthetic root `Owner`.
async fn start() -> TestServer {
    let users = [
        ("Alice", "user", ALICE_KEY),
        ("Bob", "user", BOB_KEY),
        ("Admin", "admin", ADMIN_KEY),
    ];
    let configs: Vec<UserConfig> = users
        .iter()
        .map(|(name, role, key)| UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(
                librefang_api::password_hash::hash_password(key).expect("hash test key"),
            ),
            ..Default::default()
        })
        .collect();

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = String::new();
        cfg.api_key_hash = String::new();
        cfg.users = configs;
    }));
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

/// Register an agent whose manifest `author` is `author` — the field `can_access_agent` compares against the authenticated user's name.
fn spawn_authored(server: &TestServer, author: &str) -> AgentId {
    server
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("ws-owner-scope-{}", uuid::Uuid::new_v4()),
            source_template: None,
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored test agent")
}

/// Perform a WebSocket handshake for `agent_id` as the holder of `bearer`, and return the HTTP status of the response.
async fn ws_handshake_status(server: &TestServer, agent_id: AgentId, bearer: &str) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(server.addr)
        .await
        .expect("connect to test server");
    let request = format!(
        "GET /api/agents/{agent_id}/ws HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Authorization: Bearer {bearer}\r\n\r\n"
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
async fn non_owner_user_cannot_upgrade_a_websocket_on_another_users_agent() {
    // The gap itself. Pre-fix the handler checked only that the agent existed, so Bob's `User` key completed the handshake on Alice's agent and every message he sent afterwards ran a turn — tool execution and provider spend included — under her manifest.
    let server = start().await;
    let alices_agent = spawn_authored(&server, "Alice");
    assert_eq!(
        ws_handshake_status(&server, alices_agent, BOB_KEY).await,
        REFUSED,
        "a User-role token that does not author the agent must not reach the socket that drives its turns"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_user_can_still_upgrade_a_websocket_on_their_own_agent() {
    // The other half of the rule: the check must not close the socket for the person it belongs to, which is what a blanket role bump to Admin would have done.
    let server = start().await;
    let alices_agent = spawn_authored(&server, "Alice");
    assert_eq!(
        ws_handshake_status(&server, alices_agent, ALICE_KEY).await,
        UPGRADED,
        "the agent's author must keep the dashboard chat socket"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_can_upgrade_a_websocket_on_any_agent() {
    // `can_access_agent` lets `Admin`+ inspect every agent, and the WS path must adopt that whole rule rather than only its owner clause — operators drive other people's agents from the dashboard.
    let server = start().await;
    let alices_agent = spawn_authored(&server, "Alice");
    assert_eq!(
        ws_handshake_status(&server, alices_agent, ADMIN_KEY).await,
        UPGRADED,
        "an Admin key must not be owner-scoped out of an agent socket"
    );
}
