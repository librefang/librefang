//! MCP (Model Context Protocol) server for LibreFang.
//!
//! Exposes running agents as MCP tools over JSON-RPC 2.0 stdio.
//! Each agent becomes a callable tool named `librefang_agent_{name}`.
//!
//! Protocol: newline-delimited JSON over stdin/stdout, as the MCP stdio transport specifies.
//! A peer that frames with LSP-style `Content-Length` headers is still accepted, and is answered in the same framing.
//! Connects to running daemon via HTTP, falls back to in-process kernel.

use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::LibreFangKernel;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// Backend for MCP: either a running daemon or an in-process kernel.
enum McpBackend {
    Daemon {
        base_url: String,
        client: reqwest::blocking::Client,
    },
    InProcess {
        kernel: Arc<LibreFangKernel>,
        rt: tokio::runtime::Runtime,
    },
}

impl McpBackend {
    fn list_agents(&self) -> Vec<(String, String, String)> {
        // Returns (id, name, description) triples
        match self {
            McpBackend::Daemon { base_url, client } => {
                let resp = client
                    .get(format!("{base_url}/api/agents"))
                    .send()
                    .ok()
                    .and_then(|r| r.json::<Value>().ok());
                // Handle paginated { items: [...] } or legacy array response
                let agents_arr = resp.and_then(|v| {
                    v.get("items")
                        .and_then(|items| items.as_array().cloned())
                        .or_else(|| v.as_array().cloned())
                });
                match agents_arr {
                    Some(agents) => agents
                        .iter()
                        .map(|a| {
                            (
                                a["id"].as_str().unwrap_or("").to_string(),
                                a["name"].as_str().unwrap_or("").to_string(),
                                a["description"].as_str().unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    None => Vec::new(),
                }
            }
            McpBackend::InProcess { kernel, .. } => kernel
                .agent_registry_ref()
                .list()
                .iter()
                .map(|e| {
                    (
                        e.id.to_string(),
                        e.name.clone(),
                        e.manifest.description.clone(),
                    )
                })
                .collect(),
        }
    }

    fn send_message(&self, agent_id: &str, message: &str) -> Result<String, String> {
        match self {
            McpBackend::Daemon { base_url, client } => {
                let resp = client
                    .post(format!("{base_url}/api/agents/{agent_id}/message"))
                    .json(&json!({"message": message}))
                    .send()
                    .map_err(|e| format!("HTTP error: {e}"))?;
                let body: Value = resp.json().map_err(|e| format!("Parse error: {e}"))?;
                if let Some(response) = body["response"].as_str() {
                    Ok(response.to_string())
                } else {
                    Err(crate::commands::common::api_error_message(&body)
                        .unwrap_or("Unknown error")
                        .to_string())
                }
            }
            McpBackend::InProcess { kernel, rt } => {
                let aid: librefang_types::agent::AgentId =
                    agent_id.parse().map_err(|_| "Invalid agent ID")?;
                let result = rt
                    .block_on(kernel.send_message(aid, message))
                    .map_err(|e| format!("{e}"))?;
                Ok(result.response)
            }
        }
    }

    /// Find agent ID by tool name (strip `librefang_agent_` prefix, match by name).
    fn resolve_tool_agent(&self, tool_name: &str) -> Option<String> {
        let agent_name = tool_name
            .strip_prefix("librefang_agent_")?
            .replace('_', "-");
        let agents = self.list_agents();
        // Try exact match first (with underscores replaced by hyphens)
        for (id, name, _) in &agents {
            if name.replace(' ', "-").to_lowercase() == agent_name.to_lowercase() {
                return Some(id.clone());
            }
        }
        // Try with underscores
        let agent_name_underscore = tool_name.strip_prefix("librefang_agent_")?;
        for (id, name, _) in &agents {
            if name.replace('-', "_").to_lowercase() == agent_name_underscore.to_lowercase() {
                return Some(id.clone());
            }
        }
        None
    }
}

/// Run the MCP server over stdio.
pub fn run_mcp_server(config: Option<std::path::PathBuf>) {
    let backend = create_backend(config);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    let mut framing = Framing::default();

    loop {
        match read_message(&mut reader, &mut framing) {
            Ok(Frame::Message(msg)) => {
                let response = handle_message(&backend, &msg);
                if let Some(resp) = response {
                    write_message(&mut writer, &resp, framing);
                }
            }
            Ok(Frame::ProtocolError(id)) => {
                // A single malformed or oversized frame must not kill the
                // session: reply with a JSON-RPC parse error and keep reading.
                let resp = jsonrpc_error(id, -32700, "Parse error");
                write_message(&mut writer, &resp, framing);
            }
            Ok(Frame::Eof) => break,
            // Genuine I/O failure on the underlying stream — nothing left to do.
            Err(_) => break,
        }
    }
}

/// How the peer frames its messages; replies use the same framing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Framing {
    /// One JSON value per line, terminated by `\n` (the MCP stdio transport).
    #[default]
    Newline,
    /// LSP-style `Content-Length:` headers followed by a blank line and the body.
    ContentLength,
}

/// SECURITY: messages larger than this are drained and rejected to prevent OOM.
const MAX_MCP_MESSAGE_SIZE: usize = 10 * 1024 * 1024; // 10MB

/// Outcome of reading one framed message.
enum Frame {
    /// Stream closed cleanly (true EOF / connection closed).
    Eof,
    /// A well-formed JSON-RPC message body.
    Message(Value),
    /// A malformed or oversized frame whose body was fully drained. The
    /// payload carries the request id if it could be recovered, else
    /// `Value::Null`. The run loop replies with a JSON-RPC error and continues.
    ProtocolError(Value),
}

/// Build the daemon backend with the configured `api_key` attached as a Bearer default header.
///
/// The daemon no longer exempts loopback callers from auth (#3558), and `GET /api/agents` is public while `POST /api/agents/{id}/message` is not.
/// A bare client therefore listed every agent as a tool but failed every `tools/call` with 401 once `api_key` was set.
fn daemon_backend(base_url: String, api_key: Option<&str>) -> McpBackend {
    let client = crate::commands::common::daemon_client_with_api_key_and_timeout(
        api_key,
        std::time::Duration::from_secs(120),
    );
    McpBackend::Daemon { base_url, client }
}

fn create_backend(config: Option<std::path::PathBuf>) -> McpBackend {
    // Try daemon first
    if let Some(base_url) = super::find_daemon() {
        // Resolve the key from the same config file the in-process fallback would boot from, so `--config` is honoured on both paths.
        // `daemon_config_context` reports config problems on stderr, never on stdout, which carries the MCP protocol.
        let api_key = crate::commands::common::daemon_config_context(config.as_deref()).api_key;
        return daemon_backend(base_url, api_key.as_deref());
    }

    // Fall back to in-process kernel
    let kernel = match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("Failed to boot kernel for MCP: {e}");
            std::process::exit(1);
        }
    };
    let kernel = Arc::new(kernel);
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    rt.block_on(async {
        // `boot` leaves `self_handle` empty — filling it is the caller's job,
        // as in `server.rs` and `desktop/src/server.rs`. `send_message` (the
        // path behind every `librefang_agent_*` MCP tool) calls
        // `self.kernel_handle()`, which is an `.expect`, so without this the
        // in-process backend aborts on the first tool invocation.
        //
        // Registering the `AgentLoopEnd` hooks from inside the runtime also
        // matters: `set_self_handle` spawns its orphan-temp-file sweep with
        // `tokio::spawn` when a runtime is current, and falls back to running
        // it inline when none is.
        kernel.clone().set_self_handle();
        // Spawn approval expiry sweep task on the runtime
        kernel.clone().spawn_approval_sweep_task();
    });

    McpBackend::InProcess { kernel, rt }
}

/// Read one line of at most `MAX_MCP_MESSAGE_SIZE` bytes into `line`.
///
/// Returns `Ok(None)` on EOF, `Ok(Some(true))` for a complete line, and `Ok(Some(false))` for a line that exceeded the cap; the rest of such a line is drained up to and including its `\n` so the stream stays aligned on the next message.
fn read_bounded_line(reader: &mut impl BufRead, line: &mut String) -> io::Result<Option<bool>> {
    line.clear();
    let mut buf = Vec::new();
    let n = io::Read::take(&mut *reader, MAX_MCP_MESSAGE_SIZE as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_MCP_MESSAGE_SIZE && buf.last() != Some(&b'\n') {
        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                break;
            }
            match available.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    reader.consume(pos + 1);
                    break;
                }
                None => {
                    let len = available.len();
                    reader.consume(len);
                }
            }
        }
        return Ok(Some(false));
    }
    line.push_str(&String::from_utf8_lossy(&buf));
    Ok(Some(true))
}

/// Read one JSON-RPC message, accepting both newline-delimited JSON and `Content-Length` framing, and record in `framing` which one the peer used.
///
/// Distinguishes true end-of-stream (`Frame::Eof`) from recoverable
/// protocol errors (`Frame::ProtocolError`): a malformed or oversized body
/// is drained from the stream and surfaced as a `ProtocolError` so the run
/// loop can reply with a JSON-RPC error and keep the session alive, rather
/// than collapsing the connection. `Err` is reserved for genuine I/O
/// failures on the underlying stream.
fn read_message(reader: &mut impl BufRead, framing: &mut Framing) -> io::Result<Frame> {
    // Read headers until empty line.
    //
    // `saw_header` distinguishes a genuine end-of-stream (the very first
    // `read_line` returns 0 bytes — connection closed, no data) from a frame
    // that carried headers but no usable `Content-Length`. The former is a
    // clean `Frame::Eof`; the latter is a malformed frame and must surface as
    // `Frame::ProtocolError` so the run loop replies -32700 and keeps the
    // session alive instead of breaking. `content_length` stays `None` until
    // a `Content-Length` header parses to a valid value; a parse failure is a
    // protocol error in its own right (never silently coerced to 0).
    let mut saw_header = false;
    let mut malformed = false;
    let mut content_length: Option<usize> = None;
    let mut header = String::new();
    loop {
        let complete = match read_bounded_line(reader, &mut header)? {
            None => return Ok(Frame::Eof), // True EOF — stream closed.
            Some(complete) => complete,
        };
        if !complete {
            // An over-long line can only be a newline-delimited message; it has been drained.
            return Ok(Frame::ProtocolError(Value::Null));
        }

        let trimmed = header.trim();
        if trimmed.is_empty() {
            if saw_header {
                break; // End of headers
            }
            // A blank line between newline-delimited messages carries nothing.
            continue;
        }

        if !saw_header && (trimmed.starts_with('{') || trimmed.starts_with('[')) {
            // A newline-delimited message: the whole line is the JSON-RPC body.
            *framing = Framing::Newline;
            return Ok(match serde_json::from_str(trimmed) {
                Ok(v) => Frame::Message(v),
                Err(_) => Frame::ProtocolError(Value::Null),
            });
        }

        saw_header = true;

        if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
            match len_str.parse::<usize>() {
                Ok(n) => content_length = Some(n),
                // Malformed value (e.g. `Content-Length: abc`) — a protocol
                // error, not EOF. Flag it but keep consuming the rest of this
                // frame's header block (down to the terminating empty line) so
                // the stream stays aligned on a frame boundary; returning here
                // would leave the empty-line terminator unread and desync the
                // next frame. Do NOT coerce to 0 and collapse to Eof.
                Err(_) => malformed = true,
            }
        }
    }

    *framing = Framing::ContentLength;

    if malformed {
        return Ok(Frame::ProtocolError(Value::Null));
    }

    // Headers were present but none gave a valid positive length: the frame is malformed and recoverable, not end-of-stream.
    let content_length = match content_length {
        Some(n) if n > 0 => n,
        _ => return Ok(Frame::ProtocolError(Value::Null)),
    };

    if content_length > MAX_MCP_MESSAGE_SIZE {
        // Drain the oversized body to avoid stream desync, then surface a
        // recoverable protocol error so the session continues. The id is
        // unknown (we never parse the body), so report it as Null.
        let mut discard = [0u8; 4096];
        let mut remaining = content_length;
        while remaining > 0 {
            let to_read = remaining.min(4096);
            if reader.read_exact(&mut discard[..to_read]).is_err() {
                // Stream truncated mid-drain — treat as connection closed.
                return Ok(Frame::Eof);
            }
            remaining -= to_read;
        }
        return Ok(Frame::ProtocolError(Value::Null));
    }

    // Read the body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;

    match serde_json::from_slice(&body) {
        Ok(v) => Ok(Frame::Message(v)),
        // Body was fully read but is not valid JSON — id is unrecoverable.
        // Surface a recoverable protocol error instead of faking EOF.
        Err(_) => Ok(Frame::ProtocolError(Value::Null)),
    }
}

/// Encode one JSON-RPC message in the given framing.
fn encode_message(msg: &Value, framing: Framing) -> String {
    let body = serde_json::to_string(msg).unwrap_or_default();
    match framing {
        Framing::Newline => format!("{body}\n"),
        Framing::ContentLength => format!("Content-Length: {}\r\n\r\n{}", body.len(), body),
    }
}

/// Write a JSON-RPC message to the writer in the peer's framing.
fn write_message(writer: &mut impl Write, msg: &Value, framing: Framing) {
    if let Err(e) = writer.write_all(encode_message(msg, framing).as_bytes()) {
        tracing::error!("MCP write error: {e}");
        return;
    }
    if let Err(e) = writer.flush() {
        tracing::error!("MCP flush error: {e}");
    }
}

/// Handle a JSON-RPC message and return an optional response.
fn handle_message(backend: &McpBackend, msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str().unwrap_or("");

    // A message without an `id` is a notification (`notifications/initialized`, `notifications/cancelled`, ...).
    // JSON-RPC 2.0 forbids replying to one, even with an error for an unknown method.
    let rid = msg.get("id").cloned()?;

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "librefang",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            Some(jsonrpc_response(rid, result))
        }

        "ping" => Some(jsonrpc_response(rid, json!({}))),

        "tools/list" => {
            let agents = backend.list_agents();
            let tools: Vec<Value> = agents
                .iter()
                .map(|(_, name, description)| {
                    let tool_name = format!("librefang_agent_{}", name.replace('-', "_"));
                    let desc = if description.is_empty() {
                        format!("Send a message to LibreFang agent '{name}'")
                    } else {
                        description.clone()
                    };
                    json!({
                        "name": tool_name,
                        "description": desc,
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": {
                                    "type": "string",
                                    "description": "Message to send to the agent"
                                }
                            },
                            "required": ["message"]
                        }
                    })
                })
                .collect();
            Some(jsonrpc_response(rid, json!({ "tools": tools })))
        }

        "tools/call" => {
            let params = &msg["params"];
            let tool_name = params["name"].as_str().unwrap_or("");
            let message = params["arguments"]["message"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if message.is_empty() {
                return Some(jsonrpc_error(rid, -32602, "Missing 'message' argument"));
            }

            let agent_id = match backend.resolve_tool_agent(tool_name) {
                Some(id) => id,
                None => {
                    return Some(jsonrpc_error(
                        rid,
                        -32602,
                        &format!("Unknown tool: {tool_name}"),
                    ));
                }
            };

            match backend.send_message(&agent_id, &message) {
                Ok(response) => Some(jsonrpc_response(
                    rid,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": response
                        }]
                    }),
                )),
                Err(e) => Some(jsonrpc_response(
                    rid,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {e}")
                        }],
                        "isError": true
                    }),
                )),
            }
        }

        _ => {
            // Unknown method — always respond with error
            Some(jsonrpc_error(
                rid,
                -32601,
                &format!("Method not found: {method}"),
            ))
        }
    }
}

fn jsonrpc_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-request stub daemon that answers 200 `{"response":"ok"}` only when the request carries `Authorization: Bearer secret-key`, and 401 otherwise.
    /// The returned receiver yields whether an Authorization header was present at all.
    fn auth_checking_daemon() -> (String, std::sync::mpsc::Receiver<bool>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let head = String::from_utf8_lossy(&req).to_ascii_lowercase();
            let has_auth = head.contains("\r\nauthorization:");
            let authed = head.contains("\r\nauthorization: bearer secret-key\r\n");
            let _ = tx.send(has_auth);
            let (status, body) = if authed {
                ("200 OK", r#"{"response":"ok"}"#)
            } else {
                ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (base, rx)
    }

    #[test]
    fn daemon_backend_sends_bearer_on_message_post() {
        let (base, rx) = auth_checking_daemon();
        let backend = daemon_backend(base, Some("secret-key"));
        assert_eq!(backend.send_message("some-id", "hi"), Ok("ok".to_string()));
        assert!(rx.recv().expect("stub saw the request"));
    }

    #[test]
    fn daemon_backend_without_key_sends_no_authorization_header() {
        let (base, rx) = auth_checking_daemon();
        let backend = daemon_backend(base, None);
        assert_eq!(
            backend.send_message("some-id", "hi"),
            Err("unauthorized".to_string())
        );
        assert!(!rx.recv().expect("stub saw the request"));
    }

    #[test]
    fn test_handle_initialize() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        // We can't easily create a backend in tests without a kernel,
        // but we can test the protocol handling
        let backend = McpBackend::Daemon {
            base_url: "http://localhost:9999".to_string(),
            client: crate::http_client::new_client(),
        };
        let resp = handle_message(&backend, &msg).unwrap();
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "librefang");
    }

    #[test]
    fn test_handle_notifications_initialized() {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let backend = McpBackend::Daemon {
            base_url: "http://localhost:9999".to_string(),
            client: crate::http_client::new_client(),
        };
        let resp = handle_message(&backend, &msg);
        assert!(resp.is_none()); // No response for notifications
    }

    #[test]
    fn test_handle_unknown_method() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "unknown/method"
        });
        let backend = McpBackend::Daemon {
            base_url: "http://localhost:9999".to_string(),
            client: crate::http_client::new_client(),
        };
        let resp = handle_message(&backend, &msg).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_jsonrpc_response() {
        let resp = jsonrpc_response(json!(1), json!({"status": "ok"}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "ok");
    }

    #[test]
    fn test_jsonrpc_error() {
        let resp = jsonrpc_error(json!(2), -32601, "Not found");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 2);
        assert_eq!(resp["error"]["code"], -32601);
        assert_eq!(resp["error"]["message"], "Not found");
    }

    #[test]
    fn test_read_message() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = io::BufReader::new(input.as_bytes());
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::Message(msg) => {
                assert_eq!(msg["method"], "initialize");
                assert_eq!(msg["id"], 1);
            }
            _ => panic!("expected Frame::Message for a well-formed body"),
        }
    }

    #[test]
    fn test_read_message_true_eof() {
        // Empty stream — read_line returns 0 bytes immediately.
        let mut reader = io::BufReader::new(&b""[..]);
        assert!(matches!(
            read_message(&mut reader, &mut Framing::default()).unwrap(),
            Frame::Eof
        ));
    }

    #[test]
    fn test_read_message_malformed_body_is_protocol_error() {
        // A Content-Length header with a body that is not valid JSON must
        // surface as a recoverable protocol error, NOT EOF — otherwise the
        // run loop would break and kill the whole session.
        let body = "this is not json";
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = io::BufReader::new(input.as_bytes());
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::ProtocolError(id) => assert_eq!(id, Value::Null),
            Frame::Eof => panic!("malformed body must not be reported as EOF"),
            Frame::Message(_) => panic!("malformed body must not parse as a message"),
        }
    }

    #[test]
    fn test_read_message_malformed_content_length_is_protocol_error() {
        // A non-numeric Content-Length value must surface as a recoverable
        // protocol error, NOT EOF — the old `unwrap_or(0)` collapsed this to
        // `content_length == 0` and reported Eof, killing the session. The
        // following valid frame must still parse from the same stream (no
        // desync): a malformed header has no body, so the next bytes are the
        // next frame's header.
        let good = r#"{"jsonrpc":"2.0","id":3,"method":"initialize"}"#;
        let input = format!(
            "Content-Length: abc\r\n\r\nContent-Length: {}\r\n\r\n{}",
            good.len(),
            good,
        );
        let mut reader = io::BufReader::new(input.as_bytes());

        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::ProtocolError(id) => assert_eq!(id, Value::Null),
            Frame::Eof => panic!("malformed Content-Length must not be reported as EOF"),
            Frame::Message(_) => panic!("malformed Content-Length must not parse as a message"),
        }

        // Subsequent valid frame still parses — stream stayed in sync.
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::Message(msg) => {
                assert_eq!(msg["method"], "initialize");
                assert_eq!(msg["id"], 3);
            }
            _ => panic!("expected the subsequent valid request to parse"),
        }
    }

    #[test]
    fn test_read_message_headers_without_content_length_is_protocol_error() {
        // Headers present (a non-empty line) but no Content-Length, terminated
        // by the empty line. This is a malformed frame, not end-of-stream, so
        // it must be a recoverable ProtocolError rather than Frame::Eof.
        let input = "X-Custom: value\r\n\r\n";
        let mut reader = io::BufReader::new(input.as_bytes());
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::ProtocolError(id) => assert_eq!(id, Value::Null),
            Frame::Eof => panic!("a frame with headers but no Content-Length must not be EOF"),
            Frame::Message(_) => panic!("no body to parse as a message"),
        }
    }

    #[test]
    fn test_read_message_malformed_then_valid_continues() {
        // Feed a malformed frame immediately followed by a valid request.
        // The malformed frame must be a recoverable ProtocolError (so the
        // run loop continues), and the subsequent valid request must still
        // be readable from the same stream — proving the stream stays in
        // sync and the session is not terminated.
        let bad = "not json at all";
        let good = r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#;
        let input = format!(
            "Content-Length: {}\r\n\r\n{}Content-Length: {}\r\n\r\n{}",
            bad.len(),
            bad,
            good.len(),
            good,
        );
        let mut reader = io::BufReader::new(input.as_bytes());

        // First frame: malformed -> recoverable protocol error.
        assert!(matches!(
            read_message(&mut reader, &mut Framing::default()).unwrap(),
            Frame::ProtocolError(_)
        ));

        // Second frame: the valid request still parses, proving the stream
        // was not desynced and the loop would have continued.
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::Message(msg) => {
                assert_eq!(msg["method"], "initialize");
                assert_eq!(msg["id"], 7);
            }
            _ => panic!("expected the subsequent valid request to parse"),
        }

        // And then a clean EOF.
        assert!(matches!(
            read_message(&mut reader, &mut Framing::default()).unwrap(),
            Frame::Eof
        ));
    }

    fn test_backend() -> McpBackend {
        McpBackend::Daemon {
            base_url: "http://localhost:9999".to_string(),
            client: crate::http_client::new_client(),
        }
    }

    /// The MCP stdio transport is one JSON value per line; the old reader took the line as a header and blocked forever waiting for a blank line.
    #[test]
    fn newline_delimited_messages_are_read_and_answered_in_kind() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{}}\n\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        let mut reader = io::BufReader::new(input.as_bytes());
        let mut framing = Framing::ContentLength;
        match read_message(&mut reader, &mut framing).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["method"], "initialize"),
            _ => panic!("expected a newline-delimited message"),
        }
        assert_eq!(framing, Framing::Newline);
        // The blank line between messages is skipped rather than ending the session.
        match read_message(&mut reader, &mut framing).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["id"], 1),
            _ => panic!("expected the second message"),
        }
        assert!(matches!(
            read_message(&mut reader, &mut framing).unwrap(),
            Frame::Eof
        ));

        let encoded = encode_message(
            &json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
            Framing::Newline,
        );
        assert!(
            encoded.ends_with('\n') && !encoded.contains("Content-Length"),
            "{encoded}"
        );
        assert_eq!(encoded.matches('\n').count(), 1);
    }

    #[test]
    fn content_length_peer_is_answered_with_content_length() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = io::BufReader::new(input.as_bytes());
        let mut framing = Framing::Newline;
        assert!(matches!(
            read_message(&mut reader, &mut framing).unwrap(),
            Frame::Message(_)
        ));
        assert_eq!(framing, Framing::ContentLength);
        assert!(encode_message(&json!({}), framing).starts_with("Content-Length: 2\r\n\r\n"));
    }

    #[test]
    fn invalid_newline_message_is_protocol_error_and_session_continues() {
        let input = "{not json\n{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n";
        let mut reader = io::BufReader::new(input.as_bytes());
        let mut framing = Framing::default();
        assert!(matches!(
            read_message(&mut reader, &mut framing).unwrap(),
            Frame::ProtocolError(_)
        ));
        match read_message(&mut reader, &mut framing).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["id"], 4),
            _ => panic!("expected the following message to parse"),
        }
    }

    #[test]
    fn oversized_newline_message_is_drained() {
        let mut input = Vec::new();
        input.push(b'{');
        input.extend(std::iter::repeat_n(b'x', MAX_MCP_MESSAGE_SIZE + 10));
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\n");
        let mut reader = io::BufReader::new(&input[..]);
        let mut framing = Framing::default();
        assert!(matches!(
            read_message(&mut reader, &mut framing).unwrap(),
            Frame::ProtocolError(_)
        ));
        match read_message(&mut reader, &mut framing).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["id"], 8),
            _ => panic!("expected the post-oversize message to parse"),
        }
    }

    /// JSON-RPC forbids replying to a notification, including an unknown one such as `notifications/cancelled`.
    #[test]
    fn notifications_get_no_reply_and_ping_gets_empty_result() {
        let backend = test_backend();
        let cancelled = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 3}});
        assert!(handle_message(&backend, &cancelled).is_none());
        let ping = json!({"jsonrpc": "2.0", "id": "p1", "method": "ping"});
        let resp = handle_message(&backend, &ping).expect("ping is a request");
        assert_eq!(resp["id"], "p1");
        assert_eq!(resp["result"], json!({}));
    }

    #[test]
    fn test_read_message_oversized_is_protocol_error() {
        // Declare a body just over the 10MB cap and actually supply that many
        // bytes. read_message must drain the full body and report a
        // recoverable protocol error (NOT Err, which would kill the session),
        // leaving the stream in sync for the following valid request.
        const MAX: usize = 10 * 1024 * 1024;
        let oversize = MAX + 1;
        let good = r#"{"jsonrpc":"2.0","id":9,"method":"initialize"}"#;
        let mut input: Vec<u8> = Vec::with_capacity(oversize + 128);
        input.extend_from_slice(format!("Content-Length: {oversize}\r\n\r\n").as_bytes());
        input.extend(std::iter::repeat_n(b'x', oversize));
        input.extend_from_slice(
            format!("Content-Length: {}\r\n\r\n{}", good.len(), good).as_bytes(),
        );
        let mut reader = io::BufReader::new(&input[..]);

        // Oversized frame -> recoverable protocol error, body fully drained.
        assert!(matches!(
            read_message(&mut reader, &mut Framing::default()).unwrap(),
            Frame::ProtocolError(_)
        ));

        // Stream stayed in sync: the following valid request parses.
        match read_message(&mut reader, &mut Framing::default()).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["id"], 9),
            _ => panic!("expected the post-oversize valid request to parse"),
        }
    }
}
