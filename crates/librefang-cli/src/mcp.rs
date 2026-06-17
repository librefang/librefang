//! MCP (Model Context Protocol) server for LibreFang.
//!
//! Exposes running agents as MCP tools over JSON-RPC 2.0 stdio.
//! Each agent becomes a callable tool named `librefang_agent_{name}`.
//!
//! Protocol: Content-Length framing over stdin/stdout.
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
                    Err(body["error"]
                        .as_str()
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

    loop {
        match read_message(&mut reader) {
            Ok(Frame::Message(msg)) => {
                let response = handle_message(&backend, &msg);
                if let Some(resp) = response {
                    write_message(&mut writer, &resp);
                }
            }
            Ok(Frame::ProtocolError(id)) => {
                // A single malformed or oversized frame must not kill the
                // session: reply with a JSON-RPC parse error and keep reading.
                let resp = jsonrpc_error(id, -32700, "Parse error");
                write_message(&mut writer, &resp);
            }
            Ok(Frame::Eof) => break,
            // Genuine I/O failure on the underlying stream — nothing left to do.
            Err(_) => break,
        }
    }
}

/// Outcome of reading one Content-Length framed message.
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

fn create_backend(config: Option<std::path::PathBuf>) -> McpBackend {
    // Try daemon first
    if let Some(base_url) = super::find_daemon() {
        let client = crate::http_client::client_builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");
        return McpBackend::Daemon { base_url, client };
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

    // Spawn approval expiry sweep task on the runtime
    rt.block_on(async {
        kernel.clone().spawn_approval_sweep_task();
    });

    McpBackend::InProcess { kernel, rt }
}

/// Read a Content-Length framed JSON-RPC message from the reader.
///
/// Distinguishes true end-of-stream (`Frame::Eof`) from recoverable
/// protocol errors (`Frame::ProtocolError`): a malformed or oversized body
/// is drained from the stream and surfaced as a `ProtocolError` so the run
/// loop can reply with a JSON-RPC error and keep the session alive, rather
/// than collapsing the connection. `Err` is reserved for genuine I/O
/// failures on the underlying stream.
fn read_message(reader: &mut impl BufRead) -> io::Result<Frame> {
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
    loop {
        let mut header = String::new();
        let bytes_read = reader.read_line(&mut header)?;
        if bytes_read == 0 {
            return Ok(Frame::Eof); // True EOF — stream closed.
        }

        let trimmed = header.trim();
        if trimmed.is_empty() {
            break; // End of headers
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

    if malformed {
        return Ok(Frame::ProtocolError(Value::Null));
    }

    // After the headers terminate: a missing or zero-length Content-Length is
    // only a clean EOF / empty keepalive when no header line was read at all.
    // If headers were present but none gave a valid positive length, the frame
    // is malformed and recoverable, not end-of-stream.
    let content_length = match content_length {
        Some(n) if n > 0 => n,
        _ => {
            if saw_header {
                return Ok(Frame::ProtocolError(Value::Null));
            }
            return Ok(Frame::Eof);
        }
    };

    // SECURITY: Reject oversized messages to prevent OOM.
    const MAX_MCP_MESSAGE_SIZE: usize = 10 * 1024 * 1024; // 10MB
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

/// Write a Content-Length framed JSON-RPC response to the writer.
fn write_message(writer: &mut impl Write, msg: &Value) {
    let body = serde_json::to_string(msg).unwrap_or_default();
    if let Err(e) = write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body) {
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
    let id = msg.get("id").cloned();

    // Per JSON-RPC 2.0 spec: requests MUST have an id field.
    // Use null if missing so we always send a response.
    let rid = id.unwrap_or(Value::Null);

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

        "notifications/initialized" => None, // Notification, no response

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
        match read_message(&mut reader).unwrap() {
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
        assert!(matches!(read_message(&mut reader).unwrap(), Frame::Eof));
    }

    #[test]
    fn test_read_message_malformed_body_is_protocol_error() {
        // A Content-Length header with a body that is not valid JSON must
        // surface as a recoverable protocol error, NOT EOF — otherwise the
        // run loop would break and kill the whole session.
        let body = "this is not json";
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = io::BufReader::new(input.as_bytes());
        match read_message(&mut reader).unwrap() {
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

        match read_message(&mut reader).unwrap() {
            Frame::ProtocolError(id) => assert_eq!(id, Value::Null),
            Frame::Eof => panic!("malformed Content-Length must not be reported as EOF"),
            Frame::Message(_) => panic!("malformed Content-Length must not parse as a message"),
        }

        // Subsequent valid frame still parses — stream stayed in sync.
        match read_message(&mut reader).unwrap() {
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
        match read_message(&mut reader).unwrap() {
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
            read_message(&mut reader).unwrap(),
            Frame::ProtocolError(_)
        ));

        // Second frame: the valid request still parses, proving the stream
        // was not desynced and the loop would have continued.
        match read_message(&mut reader).unwrap() {
            Frame::Message(msg) => {
                assert_eq!(msg["method"], "initialize");
                assert_eq!(msg["id"], 7);
            }
            _ => panic!("expected the subsequent valid request to parse"),
        }

        // And then a clean EOF.
        assert!(matches!(read_message(&mut reader).unwrap(), Frame::Eof));
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
            read_message(&mut reader).unwrap(),
            Frame::ProtocolError(_)
        ));

        // Stream stayed in sync: the following valid request parses.
        match read_message(&mut reader).unwrap() {
            Frame::Message(msg) => assert_eq!(msg["id"], 9),
            _ => panic!("expected the post-oversize valid request to parse"),
        }
    }
}
