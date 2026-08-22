//! Origin validation on the agent-chat WebSocket upgrade.
//!
//! The daemon serves its own dashboard, and that dashboard's WebSocket was
//! being refused by the daemon that served it: a browser at
//! `http://192.168.1.161:4545` sent `Origin: http://192.168.1.161:4545`, which
//! is neither loopback nor a member of an empty `cors_origin`, so
//! `validate_ws_origin` fell through to its final `Err` and the upgrade came
//! back 403. The operator's `allowed_origins = ["*"]` did not help — that key
//! belongs to `[terminal]` and feeds the terminal route only — and moving it to
//! the top-level `cors_origin` would not have helped either, because the
//! wildcard branch was gated on the `LIBREFANG_ALLOW_NO_AUTH` environment
//! variable, an auth switch unrelated to origins.
//!
//! These tests pin the two properties that make the product's own dashboard
//! work, and the one that keeps cross-site WebSocket hijacking (#3731) blocked:
//! the server's own origin is always accepted, `cors_origin = ["*"]` accepts
//! any origin, and a concrete list still rejects everything outside it.

use axum::routing::get;
use axum::Router;
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A syntactically invalid agent id.
///
/// Origin validation runs before the agent lookup, so an id that cannot parse
/// makes the handler return 400 the instant the origin check has *passed*.
/// That puts "accepted" (400) and "rejected" (403) one status apart with no
/// agent fixture and no waiting on the handler's five-retry existence lookup.
const UNPARSEABLE_AGENT_ID: &str = "not-a-uuid";

/// The address the fixture pretends to be reachable at — the shape of the
/// deployment that broke: a LAN IP, not loopback, on the daemon's own port.
const SERVER_HOST: &str = "192.168.1.161:4545";

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

/// Boot a daemon with `cors_origin` set to `origins` and no auth configured.
///
/// Auth is left off deliberately: it isolates the status under test to the
/// origin check. With a credential in play a 401 could preempt the 403 and the
/// assertions would no longer be attributable to `validate_ws_origin`.
///
/// `api_listen` is pinned to the port in [`SERVER_HOST`] rather than the
/// ephemeral port the fixture actually binds, because that is what production
/// does — the config value is the advertised port — and because a fixture whose
/// `listen_port` matched the ephemeral socket would let the loopback branch
/// accept origins the self-origin rule is supposed to be carrying.
async fn start(origins: Vec<String>) -> TestServer {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = String::new();
        cfg.api_key_hash = String::new();
        cfg.api_listen = format!("0.0.0.0:{}", SERVER_HOST.split(':').nth(1).unwrap());
        cfg.cors_origin = origins.clone();
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

struct Response {
    status: u16,
    body: String,
}

/// Perform a WebSocket handshake over a raw socket with an explicit `Host` and
/// `Origin`, and return the status line and body.
///
/// Raw TCP rather than an HTTP client for two reasons. `WebSocketUpgrade`
/// rejects the request with 426 before the handler body runs unless every
/// handshake header is intact, and a general-purpose client manages `Connection`
/// and `Host` itself — but `Host` is precisely what the self-origin rule reads,
/// so the test has to set it to a value that is not the socket it dialled.
async fn ws_handshake(server: &TestServer, host: &str, origin: Option<&str>) -> Response {
    let mut stream = tokio::net::TcpStream::connect(server.addr)
        .await
        .expect("connect to test server");
    let origin_line = match origin {
        Some(o) => format!("Origin: {o}\r\n"),
        None => String::new(),
    };
    let request = format!(
        "GET /api/agents/{UNPARSEABLE_AGENT_ID}/ws HTTP/1.1\r\n\
         Host: {host}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         {origin_line}\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write handshake");
    // Read until the response is complete rather than until EOF: these are
    // HTTP/1.1 keep-alive responses, so the server holds the socket open and a
    // `read_to_end` here never returns. "Complete" is the headers plus as many
    // body bytes as `Content-Length` promises; a short timeout backstops any
    // response that declares none.
    let mut raw = Vec::new();
    loop {
        let mut buf = [0u8; 1024];
        let read =
            match tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
                .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => panic!("read response: {e}"),
            };
        raw.extend_from_slice(&buf[..read]);
        let text = String::from_utf8_lossy(&raw);
        let Some((head, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        let content_length = head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse::<usize>().ok());
        match content_length {
            Some(len) if body.len() >= len => break,
            None => break,
            _ => {}
        }
    }
    let head = String::from_utf8_lossy(&raw).into_owned();
    let status_line = head.lines().next().unwrap_or_default().to_string();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in response: {status_line:?}"));
    let body = head
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Response { status, body }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_own_origin_is_accepted_with_no_allowed_origins_configured() {
    // The reported failure, reduced: nothing configured, and the browser's
    // Origin is the very host:port it fetched the dashboard from.
    let server = start(Vec::new()).await;
    let res = ws_handshake(&server, SERVER_HOST, Some(&format!("http://{SERVER_HOST}"))).await;
    assert_eq!(
        res.status, 400,
        "a dashboard served by this daemon must never be rejected by it; 400 is \
         the unparseable-agent-id rejection that follows a passed origin check, \
         and 403 here is the bug. Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wildcard_accepts_an_unrelated_origin() {
    let server = start(vec!["*".to_string()]).await;
    let res = ws_handshake(&server, SERVER_HOST, Some("https://anything.example")).await;
    assert_eq!(
        res.status, 400,
        "`cors_origin = [\"*\"]` must accept any origin on the WebSocket, as the \
         CORS layer already does for the same list. Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concrete_list_still_rejects_an_origin_outside_it() {
    // The security property the fix must not trade away: with an explicit list,
    // an origin outside it is refused even though it reached the same daemon.
    let server = start(vec!["https://dash.example.com".to_string()]).await;
    let res = ws_handshake(&server, SERVER_HOST, Some("http://evil.example")).await;
    assert_eq!(
        res.status, 403,
        "an origin outside a concrete allow list must stay rejected — this is \
         the cross-site WebSocket hijacking guard (#3731). Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn listed_origin_is_accepted() {
    let server = start(vec!["https://dash.example.com".to_string()]).await;
    let res = ws_handshake(&server, SERVER_HOST, Some("https://dash.example.com")).await;
    assert_eq!(
        res.status, 400,
        "an origin present in the list must be accepted. Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_list_containing_the_wildcard_still_accepts() {
    // A list that mixes `"*"` with concrete entries is accepted on this route.
    //
    // Deliberately NOT the regression test for the `"*"`-aborts-the-scan bug,
    // even though it is the shape that triggered it: this route passes
    // `allow_wildcard = true`, so the wildcard branch returns before the scan
    // runs and this assertion would hold with the bug still in place. The scan
    // itself is pinned where it is actually reachable — with
    // `allow_wildcard = false`, in `ws::tests::validate_ws_origin_wildcard_entry_does_not_abort_the_list_scan`
    // and `..._does_not_poison_the_rejection_reason`. What this case is good
    // for is the end-to-end guarantee that a mixed list does not break the
    // upgrade.
    let server = start(vec![
        "*".to_string(),
        "https://dash.example.com".to_string(),
    ])
    .await;
    let res = ws_handshake(&server, SERVER_HOST, Some("https://dash.example.com")).await;
    assert_eq!(
        res.status, 400,
        "a mixed list must not break the upgrade. Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rejection_body_names_the_origin_and_the_config_key() {
    let server = start(vec!["https://dash.example.com".to_string()]).await;
    let res = ws_handshake(&server, SERVER_HOST, Some("http://evil.example")).await;
    assert_eq!(res.status, 403);
    assert!(
        res.body.contains("http://evil.example"),
        "the rejection must name the origin it refused, so the failure is \
         recognisable without reading the daemon log: {}",
        res.body
    );
    assert!(
        res.body.contains("cors_origin"),
        "the rejection must name the knob that admits the origin: {}",
        res.body
    );
    assert!(
        !res.body.contains("dash.example.com"),
        "the allow list's contents must not leak to a caller that has not \
         authenticated — that detail belongs in the WARN log: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cross_site_origin_reaching_the_same_host_is_still_rejected() {
    // The attack the self-origin rule must not open. The browser sends the
    // daemon's `Host` (it dialled the daemon) but the attacker page's own
    // `Origin`, so the two differ and the rule does not fire.
    let server = start(Vec::new()).await;
    let res = ws_handshake(&server, SERVER_HOST, Some("http://evil.example")).await;
    assert_eq!(
        res.status, 403,
        "Host is the daemon's but Origin is the attacker's: accepting this \
         would make the self-origin rule a hijacking hole. Body: {}",
        res.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_browser_client_without_an_origin_is_accepted() {
    let server = start(Vec::new()).await;
    let res = ws_handshake(&server, SERVER_HOST, None).await;
    assert_eq!(
        res.status, 400,
        "curl and native clients omit Origin and must keep working. Body: {}",
        res.body
    );
}
