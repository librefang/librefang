//! Terminal WebSocket route handler.
//!
//! Provides a real-time terminal session over WebSocket using a PTY.
//!
//! ## Protocol
//!
//! Client → Server: `{"type":"input","data":"..."}`, `{"type":"resize","cols":N,"rows":N}`, `{"type":"close"}`
//! Server → Client: `{"type":"started","shell":"...","pid":N,"isRoot":bool}`, `{"type":"output","data":"..."}`, `{"type":"exit","code":N}`, `{"type":"error","content":"..."}`

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::Json;
use futures::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::AppState;
use crate::terminal::PtySession;
use crate::ws::{
    detect_connection_locality, send_json, try_acquire_ws_slot, validate_ws_origin, ws_auth_token,
    ws_query_param, WsConnectionGuard,
};

pub const MAX_WS_MSG_SIZE: usize = 64 * 1024;

const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/terminal/health", axum::routing::get(terminal_health))
        .route("/terminal/ws", axum::routing::get(terminal_ws))
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "input")]
    Input { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "close")]
    Close,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "started")]
    Started {
        shell: String,
        pid: u32,
        #[serde(rename = "isRoot")]
        is_root: bool,
    },
    #[serde(rename = "output")]
    Output { data: String, binary: Option<bool> },
    #[serde(rename = "exit")]
    Exit { code: u32, signal: Option<String> },
    #[serde(rename = "error")]
    Error { content: String },
}

impl ClientMessage {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ClientMessage::Resize { cols, rows } => {
                if *cols == 0 || *cols > MAX_COLS {
                    return Err(format!("Invalid cols: {cols}, must be 1..={MAX_COLS}"));
                }
                if *rows == 0 || *rows > MAX_ROWS {
                    return Err(format!("Invalid rows: {rows}, must be 1..={MAX_ROWS}"));
                }
                Ok(())
            }
            ClientMessage::Input { data } => {
                const MAX_INPUT_SIZE: usize = 64 * 1024;
                if data.len() > MAX_INPUT_SIZE {
                    return Err(format!(
                        "Input too large: {} bytes (max {MAX_INPUT_SIZE})",
                        data.len()
                    ));
                }
                Ok(())
            }
            ClientMessage::Close => Ok(()),
        }
    }
}

impl fmt::Display for ServerMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerMessage::Started {
                shell,
                pid,
                is_root,
            } => {
                write!(f, "started(shell={shell}, pid={pid}, is_root={is_root})")
            }
            ServerMessage::Output { data, binary } => {
                let preview = if data.len() > 32 {
                    format!("{}...", &data[..32])
                } else {
                    data.clone()
                };
                write!(
                    f,
                    "output(binary={binary:?}, data=\"{}\")",
                    preview.replace('"', "\\\"")
                )
            }
            ServerMessage::Exit { code, signal } => {
                write!(f, "exit(code={code}")?;
                if let Some(signal) = signal {
                    write!(f, ", signal={signal}")?;
                }
                write!(f, ")")
            }
            ServerMessage::Error { content } => {
                write!(f, "error(content=\"{}\")", content.replace('"', "\\\""))
            }
        }
    }
}

pub async fn terminal_health(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let locality = detect_connection_locality(&addr, &headers);

    if !cfg.terminal.enabled {
        warn!(
            ip = %locality.source_ip,
            proxied = locality.is_proxied,
            reason = "disabled",
            "Terminal WebSocket rejected — terminal is disabled"
        );
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }

    // Warn if terminal is enabled without any authentication configured.
    let valid_tokens = crate::server::valid_api_tokens(state.kernel.as_ref());
    let user_api_keys = crate::server::configured_user_api_keys(state.kernel.as_ref());
    let dashboard_auth = crate::server::has_dashboard_credentials(state.kernel.as_ref());
    let auth_configured = !valid_tokens.is_empty() || !user_api_keys.is_empty() || dashboard_auth;
    if !auth_configured {
        warn!("Terminal is enabled without any authentication configured — any local connection gets unauthenticated shell access");
    }

    let trust_proxy_headers = cfg.terminal.trust_proxy_headers;
    let listen_port = cfg.listen_port();
    if let Err(reason) = validate_ws_origin(&headers, listen_port, &cfg.terminal.allowed_origins) {
        if !cfg.terminal.allow_remote {
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                reason = "origin_mismatch",
                origin = %reason,
                "Terminal WebSocket rejected — origin validation failed"
            );
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
        warn!(
            ip = %locality.source_ip,
            proxied = locality.is_proxied,
            reason = "origin_mismatch",
            origin = %reason,
            "Terminal WebSocket origin mismatch — continuing to auth token check"
        );
    }

    let auth_method: &'static str;
    let provided_token = ws_auth_token(&headers, &uri);

    if let Some(token_str) = provided_token.as_deref() {
        let api_auth = {
            use subtle::ConstantTimeEq;
            valid_tokens.iter().any(|key| {
                token_str.len() == key.len() && token_str.as_bytes().ct_eq(key.as_bytes()).into()
            })
        };

        let session_auth = {
            let mut sessions = state.active_sessions.write().await;
            sessions.retain(|_, st| {
                !crate::password_hash::is_token_expired(
                    st,
                    crate::password_hash::DEFAULT_SESSION_TTL_SECS,
                )
            });
            sessions.contains_key(token_str)
        };

        let mut user_key_auth = false;
        if !session_auth {
            user_key_auth = user_api_keys
                .iter()
                .any(|user| crate::password_hash::verify_password(token_str, &user.api_key_hash));
        }

        if api_auth || session_auth || user_key_auth {
            auth_method = if api_auth {
                "api_key"
            } else if session_auth {
                "session"
            } else {
                "user_key"
            };
        } else if auth_configured || cfg.terminal.allow_remote {
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                reason = "invalid_token",
                "Terminal WebSocket rejected — invalid auth token"
            );
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        } else if locality.is_local() {
            if trust_proxy_headers {
                warn!(
                    ip = %locality.source_ip,
                    proxied = locality.is_proxied,
                    "Terminal local_bypass granted despite trust_proxy_headers being enabled — connection may be proxied"
                );
            }
            auth_method = "local_bypass";
        } else {
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                reason = "remote_no_auth",
                "Terminal WebSocket rejected — remote connection denied without auth"
            );
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
    } else if auth_configured {
        warn!(
            ip = %locality.source_ip,
            proxied = locality.is_proxied,
            reason = "missing_token",
            "Terminal WebSocket rejected — no auth token provided"
        );
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    } else if locality.is_local() {
        if trust_proxy_headers {
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                "Terminal local_bypass granted despite trust_proxy_headers being enabled — connection may be proxied"
            );
        }
        auth_method = "local_bypass";
    } else if cfg.terminal.allow_remote {
        auth_method = "remote_open";
    } else {
        warn!(
            ip = %locality.source_ip,
            proxied = locality.is_proxied,
            reason = "remote_no_auth",
            "Terminal WebSocket rejected — remote connection denied without auth"
        );
        return axum::http::StatusCode::FORBIDDEN.into_response();
    };

    if auth_method == "local_bypass" {
        warn!(
            ip = %locality.source_ip,
            local = locality.is_local(),
            proxied = locality.is_proxied,
            auth = %auth_method,
            "Terminal WebSocket connected"
        );
    } else {
        info!(
            ip = %locality.source_ip,
            local = locality.is_local(),
            proxied = locality.is_proxied,
            auth = %auth_method,
            "Terminal WebSocket connected"
        );
    }

    let ip = addr.ip();
    let max_ws_per_ip = state.kernel.config_ref().rate_limit.max_ws_per_ip;
    let initial_cols = initial_terminal_dimension(&uri, "cols", MAX_COLS);
    let initial_rows = initial_terminal_dimension(&uri, "rows", MAX_ROWS);

    let _terminal_guard = match try_acquire_ws_slot(ip, max_ws_per_ip) {
        Some(g) => g,
        None => {
            warn!(ip = %ip, max_ws_per_ip, "Terminal WebSocket rejected: too many connections from IP");
            return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    };

    ws.on_upgrade(move |socket| {
        let guard = _terminal_guard;
        handle_terminal_ws(socket, state, ip, guard, initial_cols, initial_rows)
    })
    .into_response()
}

fn initial_terminal_dimension(uri: &axum::http::Uri, key: &str, max: u16) -> Option<u16> {
    ws_query_param(uri, key)
        .and_then(|raw| raw.parse::<u16>().ok())
        .filter(|value| (1..=max).contains(value))
}

async fn handle_terminal_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    _client_ip: IpAddr,
    _guard: WsConnectionGuard,
    initial_cols: Option<u16>,
    initial_rows: Option<u16>,
) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    let (mut pty, mut pty_rx) = match PtySession::spawn(initial_cols, initial_rows) {
        Ok((pty, rx)) => (pty, rx),
        Err(e) => {
            let _ = send_json(
                &sender,
                &serde_json::json!({
                    "type": "error",
                    "content": format!("Failed to spawn terminal: {}", e)
                }),
            )
            .await;
            return;
        }
    };

    // Send only the shell basename (e.g. "zsh") instead of the full path
    // (e.g. "/bin/zsh") to avoid leaking server filesystem layout.
    let shell_name = std::path::Path::new(&pty.shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shell")
        .to_string();

    let _ = send_json(
        &sender,
        &serde_json::to_value(&ServerMessage::Started {
            shell: shell_name,
            pid: pty.pid,
            is_root: crate::terminal::is_running_as_root(),
        })
        .unwrap(),
    )
    .await;

    let last_activity_shared = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));

    let sender_clone = Arc::clone(&sender);
    let la = Arc::clone(&last_activity_shared);
    let mut pty_read_handle = tokio::spawn(async move {
        while let Some(data) = pty_rx.recv().await {
            let output_msg = match String::from_utf8(data.clone()) {
                Ok(s) => serde_json::json!({
                    "type": "output",
                    "data": s
                }),
                Err(_) => {
                    use base64::Engine;
                    serde_json::json!({
                        "type": "output",
                        "data": base64::engine::general_purpose::STANDARD.encode(&data),
                        "binary": true
                    })
                }
            };
            if send_json(&sender_clone, &output_msg).await.is_err() {
                break;
            }
            if let Ok(mut la) = la.lock() {
                *la = std::time::Instant::now();
            }
        }
    });

    let rl_cfg = state.kernel.config_ref().rate_limit.clone();
    let ws_idle_timeout = Duration::from_secs(rl_cfg.ws_idle_timeout_secs);
    let max_input_per_min: usize = rl_cfg.ws_messages_per_minute as usize;
    let mut input_times: Vec<std::time::Instant> = Vec::new();
    let input_window: Duration = Duration::from_secs(60);

    enum ExitReason {
        ClientClose,
        Timeout,
        ProcessExited,
    }
    let exit_reason: ExitReason;

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Text(text) => {
                                if let Ok(mut la) = last_activity_shared.lock() {
                                    *la = std::time::Instant::now();
                                }

                                if text.len() > MAX_WS_MSG_SIZE {
                                    let _ = send_json(
                                        &sender,
                                        &serde_json::json!({
                                            "type": "error",
                                            "content": "Message too large (max 64KB)"
                                        }),
                                    )
                                    .await;
                                    continue;
                                }

                                let client_msg: ClientMessage = match serde_json::from_str(&text) {
                                    Ok(msg) => msg,
                                    Err(_) => {
                                        let _ = send_json(
                                            &sender,
                                            &serde_json::json!({
                                                "type": "error",
                                                "content": "Invalid JSON"
                                            }),
                                        )
                                        .await;
                                        continue;
                                    }
                                };

                                if let Err(e) = client_msg.validate() {
                                    let _ = send_json(
                                        &sender,
                                        &serde_json::json!({
                                            "type": "error",
                                            "content": e
                                        }),
                                    )
                                    .await;
                                    continue;
                                }

                                match &client_msg {
                                    ClientMessage::Input { data } => {
                                        let now = std::time::Instant::now();
                                        input_times.retain(|t| now.duration_since(*t) < input_window);
                                        if input_times.len() >= max_input_per_min {
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": format!("Rate limit exceeded. Max {max_input_per_min} inputs per minute.")
                                                }),
                                            )
                                            .await;
                                            continue;
                                        }
                                        input_times.push(now);

                                        if let Err(e) = pty.write(data.as_bytes()) {
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": format!("Write error: {}", e)
                                                }),
                                            )
                                            .await;
                                        }
                                    }
                                    ClientMessage::Resize { cols, rows } => {
                                        if let Err(e) = pty.resize(*cols, *rows) {
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": format!("Resize error: {}", e)
                                                }),
                                            )
                                            .await;
                                        }
                                    }
                                    ClientMessage::Close => {
                                        exit_reason = ExitReason::ClientClose;
                                        break;
                                    }
                                }
                            }
                            Message::Close(_) => {
                                exit_reason = ExitReason::ClientClose;
                                break;
                            }
                            Message::Ping(data) => {
                                if let Ok(mut la) = last_activity_shared.lock() {
                                    *la = std::time::Instant::now();
                                }
                                let mut s = sender.lock().await;
                                let _ = s.send(Message::Pong(data)).await;
                            }
                            _ => {}
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "WebSocket receive error");
                        exit_reason = ExitReason::ClientClose;
                        break;
                    }
                    None => {
                        exit_reason = ExitReason::ClientClose;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(ws_idle_timeout.saturating_sub(last_activity_shared.lock().map(|la| la.elapsed()).unwrap_or(Duration::ZERO))) => {
                exit_reason = ExitReason::Timeout;
                break;
            }
            _ = &mut pty_read_handle => {
                if let Ok(mut la) = last_activity_shared.lock() {
                    *la = std::time::Instant::now();
                }
                // PTY reader ended = child process exited; get real exit code below
                exit_reason = ExitReason::ProcessExited;
                break;
            }
        }
    }

    // For ClientClose and Timeout the child may still be running — kill it first
    // so that wait_exit() returns promptly with the real exit code.
    if !matches!(exit_reason, ExitReason::ProcessExited) {
        pty.kill();
    }

    // Always wait for the real exit code, regardless of why the loop ended.
    let (code, signal) = match pty.wait_exit() {
        Ok(pair) => pair,
        Err(e) => {
            warn!(error = %e, "Failed to wait for child exit");
            (1, None)
        }
    };
    let _ = send_json(
        &sender,
        &serde_json::json!({
            "type": "exit",
            "code": code,
            "signal": signal
        }),
    )
    .await;

    pty_read_handle.abort();
    info!("Terminal WebSocket disconnected");
}

#[cfg(test)]
mod tests {
    use crate::routes::terminal::{
        initial_terminal_dimension, router, ClientMessage, ServerMessage, MAX_COLS, MAX_ROWS,
    };
    use crate::terminal::shell_for_current_os;

    #[test]
    fn test_shell_selection_unix() {
        let (shell, flag) = shell_for_current_os();
        #[cfg(not(windows))]
        {
            assert!(!shell.is_empty());
            assert_eq!(flag, "-c");
        }
        #[cfg(windows)]
        {
            assert!(!shell.is_empty());
            assert_eq!(flag, "/C");
        }
    }

    #[test]
    fn test_resize_validation_bounds() {
        let msg = ClientMessage::Resize { cols: 0, rows: 40 };
        assert!(msg.validate().is_err());

        let msg = ClientMessage::Resize {
            cols: 1001,
            rows: 40,
        };
        assert!(msg.validate().is_err());

        let msg = ClientMessage::Resize { cols: 120, rows: 0 };
        assert!(msg.validate().is_err());

        let msg = ClientMessage::Resize {
            cols: 120,
            rows: 501,
        };
        assert!(msg.validate().is_err());

        let msg = ClientMessage::Resize {
            cols: 120,
            rows: 40,
        };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_input_size_limit() {
        let too_large = "x".repeat(65 * 1024);
        let msg = ClientMessage::Input { data: too_large };
        assert!(msg.validate().is_err());

        let ok = "x".repeat(64 * 1024);
        let msg = ClientMessage::Input { data: ok };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn test_initial_terminal_dimension_parses_valid_query_values() {
        let uri: axum::http::Uri = "/api/terminal/ws?cols=132&rows=43".parse().unwrap();
        assert_eq!(
            initial_terminal_dimension(&uri, "cols", MAX_COLS),
            Some(132)
        );
        assert_eq!(initial_terminal_dimension(&uri, "rows", MAX_ROWS), Some(43));
    }

    #[test]
    fn test_initial_terminal_dimension_rejects_invalid_query_values() {
        let uri: axum::http::Uri = "/api/terminal/ws?cols=2000&rows=0".parse().unwrap();
        assert_eq!(initial_terminal_dimension(&uri, "cols", MAX_COLS), None);
        assert_eq!(initial_terminal_dimension(&uri, "rows", MAX_ROWS), None);
    }

    #[test]
    fn test_client_message_parse() {
        let input = r#"{"type":"input","data":"hello"}"#;
        let msg: ClientMessage = serde_json::from_str(input).unwrap();
        match msg {
            ClientMessage::Input { data } => assert_eq!(data, "hello"),
            _ => panic!("expected Input"),
        }

        let resize = r#"{"type":"resize","cols":80,"rows":24}"#;
        let msg: ClientMessage = serde_json::from_str(resize).unwrap();
        match msg {
            ClientMessage::Resize { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            _ => panic!("expected Resize"),
        }

        let close = r#"{"type":"close"}"#;
        let msg: ClientMessage = serde_json::from_str(close).unwrap();
        match msg {
            ClientMessage::Close => {}
            _ => panic!("expected Close"),
        }
    }

    #[test]
    fn test_server_message_serialize() {
        let msg = ServerMessage::Started {
            shell: "/bin/bash".to_string(),
            pid: 12345,
            is_root: false,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"started""#));
        assert!(json.contains(r#""shell":"/bin/bash""#));

        let msg = ServerMessage::Output {
            data: "hello".to_string(),
            binary: Some(true),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"output""#));
        assert!(json.contains(r#""binary":true"#));
    }

    #[test]
    fn test_terminal_router_creation() {
        let _app = router();
    }
}

// ---------------------------------------------------------------------------
// WebSocket auth policy unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod terminal_ws_auth_tests {
    use crate::ws::validate_ws_origin;
    use axum::http::HeaderMap;

    #[test]
    fn validate_ws_origin_allows_matching_port_on_localhost() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, 4545, &[]).is_ok());
    }

    #[test]
    fn validate_ws_origin_rejects_mismatched_port() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:8080".parse().unwrap());
        assert!(validate_ws_origin(&headers, 4545, &[]).is_err());
    }

    #[test]
    fn validate_ws_origin_wildcard_allows_any_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.example:9999".parse().unwrap());
        assert!(validate_ws_origin(&headers, 4545, &["*".to_string()]).is_ok());

        // Also works with https
        let mut headers2 = HeaderMap::new();
        headers2.insert("origin", "https://other.host:1234".parse().unwrap());
        assert!(validate_ws_origin(&headers2, 4545, &["*".to_string()]).is_ok());
    }

    #[test]
    fn validate_ws_origin_allows_specific_allowed_origins() {
        let allowed = vec!["https://my.domain.com".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://my.domain.com".parse().unwrap());
        assert!(validate_ws_origin(&headers, 4545, &allowed).is_ok());
    }

    #[test]
    fn validate_ws_origin_rejects_non_matching_allowed_origins() {
        let allowed = vec!["https://my.domain.com".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://evil.example".parse().unwrap());
        assert!(validate_ws_origin(&headers, 4545, &allowed).is_err());
    }
}
