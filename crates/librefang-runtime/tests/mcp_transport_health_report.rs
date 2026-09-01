//! #7963 — the tool-call dispatch path reports MCP transport failures to the health monitor, and only transport failures.
//!
//! These tests exercise the real injection site: a live [`McpConnection`] against a stub MCP server, dispatched through `execute_tool_raw`, with a recording [`McpTransportHealthReporter`] standing in for the kernel's health monitor.
//!
//! Testing the wiring rather than only the implementation is deliberate.
//! `McpConnection::health_reporter` is an `Option` and the classification is a branch — both compile perfectly well while doing nothing, which is exactly how #7963 stayed invisible: every piece of the auto-reconnect machinery worked, and nothing connected them.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use librefang_runtime::mcp::{
    empty_taint_rule_sets_handle, McpConnection, McpServerConfig, McpTransport,
    McpTransportHealthReporter,
};
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;

// --------------------------------------------------------------------------- Recording reporter ---------------------------------------------------------------------------

#[derive(Default)]
struct RecordingReporter {
    transport_failures: Mutex<Vec<(String, String)>>,
    ok_calls: Mutex<Vec<(String, usize)>>,
}

impl RecordingReporter {
    fn transport_failures(&self) -> Vec<(String, String)> {
        self.transport_failures.lock().unwrap().clone()
    }

    fn ok_calls(&self) -> Vec<(String, usize)> {
        self.ok_calls.lock().unwrap().clone()
    }
}

impl McpTransportHealthReporter for RecordingReporter {
    fn report_transport_failure(&self, server: &str, error: &str) {
        self.transport_failures
            .lock()
            .unwrap()
            .push((server.to_string(), error.to_string()));
    }

    fn report_call_ok(&self, server: &str, tool_count: usize) {
        self.ok_calls
            .lock()
            .unwrap()
            .push((server.to_string(), tool_count));
    }
}

// --------------------------------------------------------------------------- Stub MCP server (SSE transport: one HTTP POST per JSON-RPC request) ---------------------------------------------------------------------------

/// What the stub does when it receives `tools/call`.
#[derive(Clone, Copy)]
enum CallBehaviour {
    /// Accept the request and never answer — the #7963 wedge: the server is alive, the transport carries nothing back, every call expires at `timeout_secs`.
    Wedge,
    /// Answer with a well-formed JSON-RPC error object, the way a server reports bad arguments or a missing file.
    JsonRpcError,
    /// Answer with a normal successful result.
    Success,
}

fn json_body(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Spawn a stub server that answers `initialize` / `tools/list` so the connection handshakes and registers one tool, then applies `behaviour` to `tools/call`.
///
/// The returned handle is *not* a shutdown handle — dropping a `JoinHandle` detaches the thread rather than stopping it, and the thread is parked in `listener.incoming()` (holding the wedged sockets open, which the wedge test needs). It is returned only so a caller can keep the stub alive for the duration of the test; the thread and its listener go away with the test binary.
fn spawn_stub(behaviour: CallBehaviour) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
    let addr = listener.local_addr().expect("stub addr");
    let handle = std::thread::spawn(move || {
        // Hold every wedged connection open for the lifetime of the stub — dropping the socket would give reqwest an early EOF instead of the timeout we are testing.
        let mut wedged: Vec<TcpStream> = Vec::new();
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let mut buf = [0u8; 8192];
            let Ok(read) = stream.read(&mut buf) else {
                continue;
            };
            if read == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buf[..read]).to_string();
            let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            let parsed: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let method = parsed
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let id = parsed.get("id").and_then(|i| i.as_u64()).unwrap_or(0);

            match method.as_str() {
                "initialize" => json_body(
                    &mut stream,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            // No `resources` capability — keeps the handshake to exactly initialize + tools/list.
                            "capabilities": {},
                            "serverInfo": { "name": "stub", "version": "0" }
                        }
                    })
                    .to_string(),
                ),
                "tools/list" => json_body(
                    &mut stream,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [{
                                "name": "slowop",
                                "description": "a tool",
                                "inputSchema": { "type": "object", "properties": {} }
                            }]
                        }
                    })
                    .to_string(),
                ),
                "tools/call" => match behaviour {
                    CallBehaviour::Wedge => wedged.push(stream),
                    CallBehaviour::JsonRpcError => json_body(
                        &mut stream,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32602, "message": "file not found" }
                        })
                        .to_string(),
                    ),
                    CallBehaviour::Success => json_body(
                        &mut stream,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": "done" }]
                            }
                        })
                        .to_string(),
                    ),
                },
                // `notifications/initialized` and anything else: accept and close without a body.
                _ => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        }
    });
    (format!("http://{addr}/sse"), handle)
}

async fn connect_stub(url: String, reporter: Option<Arc<RecordingReporter>>) -> McpConnection {
    let conn = McpConnection::connect(McpServerConfig {
        name: "wedged".to_string(),
        transport: McpTransport::Sse { url },
        // Short so the wedge test does not sit for a minute.
        timeout_secs: 2,
        env: vec![],
        headers: vec![],
        oauth_provider: None,
        oauth_config: None,
        taint_scanning: true,
        taint_policy: None,
        taint_rule_sets: empty_taint_rule_sets_handle(),
        roots: vec![],
    })
    .await
    .expect("stub MCP server should complete the handshake");
    assert_eq!(
        conn.tools().len(),
        1,
        "the stub advertises exactly one tool"
    );
    match reporter {
        Some(r) => conn.with_health_reporter(r),
        None => conn,
    }
}

fn ctx<'a>(mcp_connections: &'a tokio::sync::Mutex<Vec<McpConnection>>) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: None,
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"),
        skill_registry: None,
        allowed_skills: None,
        mcp_connections: Some(mcp_connections),
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id: None,
        channel: None,
        chat_id: None,
        sender_account_id: None,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
        acting_principal: None,
    }
}

// --------------------------------------------------------------------------- Tests ---------------------------------------------------------------------------

/// The bug: a server that handshook fine and then wedged.
/// The dispatch path must report the timeout so auto-reconnect can eventually engage.
#[tokio::test]
async fn transport_failure_is_reported_from_the_dispatch_path() {
    let (url, _stub) = spawn_stub(CallBehaviour::Wedge);
    let reporter = Arc::new(RecordingReporter::default());
    let conn = connect_stub(url, Some(Arc::clone(&reporter) as Arc<RecordingReporter>)).await;
    let conns = tokio::sync::Mutex::new(vec![conn]);

    let result = execute_tool_raw("t1", "mcp_wedged_slowop", &json!({}), &ctx(&conns)).await;

    assert!(
        result.is_error,
        "a wedged transport must fail the tool call"
    );
    let failures = reporter.transport_failures();
    assert_eq!(
        failures.len(),
        1,
        "exactly one transport failure must be reported, got {failures:?}"
    );
    assert_eq!(failures[0].0, "wedged", "reported under the server's name");
    assert!(
        reporter.ok_calls().is_empty(),
        "a failed call must not report success"
    );
}

/// The other half of the contract, and the reason the classification exists: a server that answers with an error is healthy.
/// Reporting this would tear down a working server every time a model passed bad arguments.
#[tokio::test]
async fn application_error_is_not_reported_from_the_dispatch_path() {
    let (url, _stub) = spawn_stub(CallBehaviour::JsonRpcError);
    let reporter = Arc::new(RecordingReporter::default());
    let conn = connect_stub(url, Some(Arc::clone(&reporter) as Arc<RecordingReporter>)).await;
    let conns = tokio::sync::Mutex::new(vec![conn]);

    let result = execute_tool_raw("t1", "mcp_wedged_slowop", &json!({}), &ctx(&conns)).await;

    assert!(
        result.is_error,
        "the application error is still surfaced to the model"
    );
    assert!(
        result.content.contains("file not found"),
        "the server's message must reach the model: {}",
        result.content
    );
    assert!(
        reporter.transport_failures().is_empty(),
        "an application error must NOT be reported as a transport failure: {:?}",
        reporter.transport_failures()
    );
}

/// A successful call resets the consecutive-failure run, so the threshold only ever counts an unbroken sequence of failures.
#[tokio::test]
async fn successful_call_is_reported_as_ok_from_the_dispatch_path() {
    let (url, _stub) = spawn_stub(CallBehaviour::Success);
    let reporter = Arc::new(RecordingReporter::default());
    let conn = connect_stub(url, Some(Arc::clone(&reporter) as Arc<RecordingReporter>)).await;
    let conns = tokio::sync::Mutex::new(vec![conn]);

    let result = execute_tool_raw("t1", "mcp_wedged_slowop", &json!({}), &ctx(&conns)).await;

    assert!(!result.is_error, "call should succeed: {}", result.content);
    assert_eq!(reporter.ok_calls(), vec![("wedged".to_string(), 1)]);
    assert!(reporter.transport_failures().is_empty());
}

/// A connection with no reporter attached — stand-alone callers, tests — must dispatch exactly as before rather than panicking on the `None`.
#[tokio::test]
async fn a_connection_without_a_reporter_still_dispatches() {
    let (url, _stub) = spawn_stub(CallBehaviour::Success);
    let conn = connect_stub(url, None).await;
    assert!(
        conn.health_reporter().is_none(),
        "connect must not invent a reporter"
    );
    let conns = tokio::sync::Mutex::new(vec![conn]);

    let result = execute_tool_raw("t1", "mcp_wedged_slowop", &json!({}), &ctx(&conns)).await;

    assert!(!result.is_error, "call should succeed: {}", result.content);
}
