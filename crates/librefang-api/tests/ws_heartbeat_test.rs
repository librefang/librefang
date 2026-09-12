//! The agent-chat WebSocket must notice a peer that has stopped answering.
//!
//! `ws.rs` answers a Ping but never sends one, so the only thing that ever discovers a dead peer is a failing write — which means a socket sitting idle between turns, the state a chat connection spends most of its life in, is never probed at all.
//! Ten daemon lifetimes on the production host recorded 55 `client_close`, one `send_error`, one `receive_error` and zero `idle_timeout` disconnects: the one detection the server has works, and it only fires when there is outbound traffic.
//!
//! Raw TCP rather than `tokio_tungstenite::connect_async` because the client under test has to be one that *does not* answer Pong, and tungstenite answers automatically.
//! Reading the bytes also makes the assertions unambiguous: opcode `0x9` is a Ping, `0x8` is a Close, and neither can be confused with an application-level message that happens to mention them.
//! No `Origin` header is sent, which `validate_ws_origin` treats as a non-browser client and lets through, and the empty `api_key` / `api_key_hash` pair puts the handler on its loopback no-auth branch — so the handshake status below is not an authorization verdict.

use axum::routing::get;
use axum::Router;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Ping cadence for these tests.
///
/// The shipped default is 30 s; one second keeps the whole file inside a few seconds without changing the rule under test, which is expressed in intervals rather than in absolute time.
const PING_INTERVAL_SECS: u64 = 1;

/// How long any single assertion waits before calling it a failure.
///
/// Detection costs two intervals, so this is several times the budget the server needs — a timeout here means nothing was sent at all, not that the deadline was tight.
const BUDGET: Duration = Duration::from_secs(8);

/// `ws_idle_timeout_secs` keeps its 1800 s default in every test below.
///
/// That is the discriminator against a false green: no assertion in this file can be satisfied by the idle timeout, because the idle timeout cannot fire inside `BUDGET`.
const IDLE_TIMEOUT_CANNOT_FIRE_WITHIN: Duration = Duration::from_secs(1800);

const OPCODE_TEXT: u8 = 0x1;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

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

/// Boot a daemon whose chat socket pings every `PING_INTERVAL_SECS`.
async fn start() -> TestServer {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.api_key = String::new();
        cfg.api_key_hash = String::new();
        cfg.rate_limit.ws_ping_interval_secs = PING_INTERVAL_SECS;
        assert!(
            Duration::from_secs(cfg.rate_limit.ws_idle_timeout_secs)
                >= IDLE_TIMEOUT_CANNOT_FIRE_WITHIN,
            "these tests only mean something while the idle timeout is far outside BUDGET"
        );
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

fn spawn_agent(server: &TestServer) -> AgentId {
    server
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("ws-heartbeat-{}", uuid::Uuid::new_v4()),
            source_template: None,
            ..AgentManifest::default()
        })
        .expect("spawn test agent")
}

/// One decoded frame.
struct Frame {
    opcode: u8,
    payload: Vec<u8>,
}

/// Complete the upgrade and return the open socket.
///
/// The response headers are consumed one byte at a time and stop exactly at the blank line, so the first frame the server sends is still in the stream for the caller to read.
async fn upgrade(server: &TestServer, agent_id: AgentId) -> TcpStream {
    let mut stream = TcpStream::connect(server.addr)
        .await
        .expect("connect to test server");
    let request = format!(
        "GET /api/agents/{agent_id}/ws HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write handshake");

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .await
            .expect("read handshake response");
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_default();
    assert_eq!(
        status, "101",
        "the upgrade must complete before any frame assertion means anything; response was:\n{head}"
    );
    stream
}

/// Read one server frame, or `None` once the server has closed the TCP connection.
///
/// Server-to-client frames are never masked, so the payload follows the length header verbatim.
async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Option<Frame>> {
    let mut head = [0u8; 2];
    match stream.read_exact(&mut head).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let opcode = head[0] & 0x0F;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b).await?;
            usize::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b).await?;
            u64::from_be_bytes(b) as usize
        }
        n => usize::from(n),
    };
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok(Some(Frame { opcode, payload }))
}

/// Send a client frame. RFC 6455 requires every client-to-server frame to be masked.
async fn write_masked(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    assert!(
        payload.len() < 126,
        "test frames stay in the short-length form"
    );
    let mask = [0x37u8, 0xfa, 0x21, 0x3d];
    let mut out = vec![0x80 | opcode, 0x80 | payload.len() as u8];
    out.extend_from_slice(&mask);
    out.extend(
        payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i % mask.len()]),
    );
    stream.write_all(&out).await
}

/// Read frames until `want` arrives, panicking with `unmet` if `BUDGET` runs out first.
///
/// Frames the caller did not ask about are skipped rather than asserted on: the socket opens with a `connected` text frame and an `agents_updated` snapshot whose ordering is not part of any rule here.
async fn wait_for_opcode(stream: &mut TcpStream, want: u8, unmet: &str) -> Frame {
    let deadline = tokio::time::Instant::now() + BUDGET;
    let mut seen: Vec<u8> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "{unmet} (waited {BUDGET:?}; opcodes seen: {seen:02x?})"
        );
        match tokio::time::timeout(remaining, read_frame(stream)).await {
            Err(_) => {
                panic!("{unmet} (waited {BUDGET:?}; opcodes seen: {seen:02x?})")
            }
            Ok(Ok(Some(frame))) => {
                seen.push(frame.opcode);
                if frame.opcode == want {
                    return frame;
                }
            }
            // EOF is how a close arrives when the server drops the TCP connection
            // without a close frame, so it satisfies a caller waiting for one.
            Ok(Ok(None)) => {
                assert_eq!(
                    want, OPCODE_CLOSE,
                    "{unmet} (server closed the connection; opcodes seen: {seen:02x?})"
                );
                return Frame {
                    opcode: OPCODE_CLOSE,
                    payload: Vec::new(),
                };
            }
            Ok(Err(e)) => panic!("{unmet} (read failed: {e})"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn open_socket_delivers_its_first_frame() {
    // Not the defect — the control that makes the other three mean something.
    // If the raw client or the handshake were wrong, every assertion in this file
    // would time out for that reason and look exactly like a missing heartbeat.
    let server = start().await;
    let agent_id = spawn_agent(&server);
    let mut stream = upgrade(&server, agent_id).await;

    let frame = wait_for_opcode(
        &mut stream,
        OPCODE_TEXT,
        "an upgraded chat socket must deliver its opening text frame",
    )
    .await;
    let text = String::from_utf8_lossy(&frame.payload);
    assert!(
        text.contains("connected") || text.contains("agents_updated"),
        "opening frame should be the connection announcement or the agent snapshot, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn server_pings_a_client_that_has_gone_silent() {
    // The measured defect. `ws.rs` has a `Message::Ping` arm that answers a peer's
    // ping and nothing that ever sends one, so a silent client is never probed.
    let server = start().await;
    let agent_id = spawn_agent(&server);
    let mut stream = upgrade(&server, agent_id).await;

    wait_for_opcode(
        &mut stream,
        OPCODE_PING,
        "the server must ping a client it has heard nothing from, or it has no way to learn the peer is gone",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_closes_a_client_that_never_answers_a_ping() {
    // The half that matters operationally: sending a ping is only useful if an
    // unanswered one ends the connection. This client completes the handshake and
    // then behaves exactly like a suspended laptop — it never writes again.
    let server = start().await;
    let agent_id = spawn_agent(&server);
    let mut stream = upgrade(&server, agent_id).await;

    wait_for_opcode(
        &mut stream,
        OPCODE_CLOSE,
        "a peer that stops answering must be disconnected within two ping intervals, not held until the idle timeout",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_keeps_a_client_that_answers_pings() {
    // The discriminator against a fix that just closes sockets. This must pass both
    // before and after the change, so a green on the two tests above cannot be
    // bought by disconnecting everyone.
    let server = start().await;
    let agent_id = spawn_agent(&server);
    let mut stream = upgrade(&server, agent_id).await;

    let deadline = tokio::time::Instant::now() + BUDGET;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, read_frame(&mut stream)).await {
            // No traffic for the rest of the budget is a healthy connection.
            Err(_) => break,
            Ok(Ok(Some(frame))) => {
                assert_ne!(
                    frame.opcode, OPCODE_CLOSE,
                    "a client answering every ping must not be disconnected"
                );
                if frame.opcode == OPCODE_PING {
                    write_masked(&mut stream, OPCODE_PONG, &frame.payload)
                        .await
                        .expect("answer the server's ping");
                }
            }
            Ok(Ok(None)) => {
                panic!("a client answering every ping must not be disconnected")
            }
            Ok(Err(e)) => panic!("read failed while answering pings: {e}"),
        }
    }
}
