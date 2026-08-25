//! `Origin` validation on the agent-chat WebSocket upgrade (#7777).
//!
//! These run against a real axum router over a raw socket, because the rule
//! under test reads two headers — `Origin` and `Host` — and only one of them is
//! under a normal HTTP client's control. `Host` is set by the client library
//! from the address it dialled, so a test that wants to say "the browser dialled
//! 192.168.1.161:4545" while actually connecting to a loopback ephemeral port
//! has to write the request bytes itself. That divergence is not an artifact of
//! the test: it is exactly the shape of the real deployment, where the daemon
//! binds `0.0.0.0` and the browser addresses one particular interface.
//!
//! Raw TCP also keeps `axum::extract::WebSocketUpgrade` happy — it answers 426
//! before the handler body runs unless `Connection`, `Upgrade`,
//! `Sec-WebSocket-Version` and `Sec-WebSocket-Key` are all intact, and a
//! general-purpose client manages `Connection` itself.

use axum::routing::get;
use axum::Router;
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The address the operator in #7777 reaches the dashboard on.
const SERVER_HOST: &str = "192.168.1.161:4545";

/// A syntactically invalid agent id, so a *passing* origin check lands on a
/// status one step later with no agent fixture and no waiting.
///
/// `impl FromStr for AgentId` is `Uuid::parse_str` with no fallback, so this
/// never reaches the existence lookup and returns 400 rather than 404. The
/// discriminator every assertion below turns on is therefore 403 (origin
/// refused) versus 400 (origin accepted, then the id rejected).
const UNPARSEABLE_AGENT_ID: &str = "not-a-uuid";

const ORIGIN_ACCEPTED: u16 = 400;
const ORIGIN_REFUSED: u16 = 403;

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

/// Boot a daemon with no auth configured and the given `cors_origin` list.
///
/// No credential is deliberate: it makes the loopback no-auth branch pass so
/// every status below is the origin check's verdict and not an auth failure,
/// and it is the shipped default the reporter was running.
///
/// `api_listen` is `0.0.0.0:4545` — the deployment from the issue. The socket
/// still binds an ephemeral loopback port; `api_listen` only feeds the
/// `listen_port` argument, which is what the loopback rule consults.
async fn start(cors_origin: Vec<String>) -> TestServer {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = String::new();
        cfg.api_key_hash = String::new();
        cfg.api_listen = "0.0.0.0:4545".to_string();
        cfg.cors_origin = cors_origin;
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

/// Perform a handshake with an explicit `Host` and `Origin`, return the status.
async fn handshake(server: &TestServer, host: &str, origin: &str) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(server.addr)
        .await
        .expect("connect to test server");
    let request = format!(
        "GET /api/agents/{UNPARSEABLE_AGENT_ID}/ws HTTP/1.1\r\n\
         Host: {host}\r\n\
         Origin: {origin}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
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
async fn server_own_lan_origin_is_accepted_with_no_allowed_origins_configured() {
    // The reported failure from #7777: the daemon refuses the dashboard it
    // served. The browser fetched the page from this very address, so `Host`
    // and `Origin` are the same IP literal and port.
    let server = start(Vec::new()).await;
    assert_eq!(
        handshake(&server, SERVER_HOST, &format!("http://{SERVER_HOST}")).await,
        ORIGIN_ACCEPTED,
        "a dashboard served by this daemon over its own LAN address must not be \
         rejected by it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebound_hostname_with_matching_host_and_origin_is_rejected() {
    // The reason the implicit allow is restricted to IP literals, exercised
    // end-to-end rather than only at the unit boundary. An attacker who serves
    // a page from a name they own and rebinds that name to this daemon's
    // address produces matching `Host` and `Origin` with nothing forged, so
    // equality alone cannot be allowed to open the socket.
    let server = start(Vec::new()).await;
    assert_eq!(
        handshake(
            &server,
            "attacker.example:4545",
            "http://attacker.example:4545"
        )
        .await,
        ORIGIN_REFUSED,
        "matching Host and Origin on a hostname is not evidence that this daemon \
         served the page"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostname_deployment_is_accepted_once_its_origin_is_listed() {
    // The supported path for hostname and TLS-proxy deployments, and the reason
    // the rejection above is a narrowing rather than a removal of the feature.
    let server = start(vec!["https://dash.example.com".to_string()]).await;
    assert_eq!(
        handshake(&server, "dash.example.com", "https://dash.example.com").await,
        ORIGIN_ACCEPTED,
        "cors_origin must remain the escape hatch for a hostname deployment"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cross_site_origin_reaching_the_same_host_is_still_rejected() {
    // The #3731 cross-site WebSocket hijacking guard: a page on another origin
    // reaching this daemon sends its own `Origin` beside our `Host`.
    let server = start(Vec::new()).await;
    assert_eq!(
        handshake(&server, SERVER_HOST, "http://evil.example").await,
        ORIGIN_REFUSED
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lan_origin_the_request_was_not_addressed_to_is_rejected() {
    // The self rule compares against the address the browser actually dialled,
    // so one LAN IP does not vouch for another.
    let server = start(Vec::new()).await;
    assert_eq!(
        handshake(&server, "10.0.0.7:4545", &format!("http://{SERVER_HOST}")).await,
        ORIGIN_REFUSED
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn loopback_origin_on_the_listen_port_is_still_accepted() {
    // The pre-existing rule, unaffected by the self rule and reached before it.
    let server = start(Vec::new()).await;
    assert_eq!(
        handshake(&server, "localhost:4545", "http://localhost:4545").await,
        ORIGIN_ACCEPTED
    );
}
