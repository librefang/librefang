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
use crate::terminal_tmux::{validate_window_name, TmuxController, DEFAULT_TMUX_SESSION_NAME};
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
        .route(
            "/terminal/windows",
            axum::routing::get(list_windows).post(create_window),
        )
        .route(
            "/terminal/windows/{window_id}",
            axum::routing::delete(delete_window).patch(rename_window),
        )
        .route("/terminal/ws", axum::routing::get(terminal_ws))
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "input")]
    Input { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "switch_window")]
    SwitchWindow { window: String },
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
    #[serde(rename = "active_window")]
    ActiveWindow { window_id: String },
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
            ClientMessage::SwitchWindow { window } => {
                if !crate::terminal_tmux::validate_window_id(window) {
                    return Err(format!("Invalid window id: {window:?}"));
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
                    let truncated: String = data.chars().take(32).collect();
                    format!("{truncated}...")
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
            ServerMessage::ActiveWindow { window_id } => {
                write!(f, "active_window(window_id={window_id})")
            }
        }
    }
}

pub async fn terminal_health(
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if let Err(resp) = authorize_terminal_request(&headers, &uri, addr, &state).await {
        return resp;
    }

    let (tmux_enabled, max_windows, tmux_path) = {
        let cfg = state.kernel.config_ref();
        (
            cfg.terminal.tmux_enabled,
            cfg.terminal.max_windows,
            std::path::PathBuf::from(cfg.terminal.tmux_binary_path.as_deref().unwrap_or("tmux")),
        )
    };
    let tmux_available = tmux_enabled && TmuxController::is_available(&tmux_path).await;
    Json(serde_json::json!({
        "ok": true,
        "tmux": tmux_available,
        "max_windows": max_windows,
        "os": std::env::consts::OS,
    }))
    .into_response()
}

/// Authentication method recorded for a successful terminal WS connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    ApiKey,
    Session,
    UserKey,
    /// No auth configured, connection is local (loopback, not proxied).
    LocalBypass,
    /// No auth configured, remote connection accepted because
    /// `allow_remote` + `allow_unauthenticated_remote` are both true.
    RemoteOpen,
}

impl AuthMethod {
    fn as_str(self) -> &'static str {
        match self {
            AuthMethod::ApiKey => "api_key",
            AuthMethod::Session => "session",
            AuthMethod::UserKey => "user_key",
            AuthMethod::LocalBypass => "local_bypass",
            AuthMethod::RemoteOpen => "remote_open",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStatus {
    Valid(AuthMethod),
    InvalidToken,
    NoToken,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub is_local: bool,
    pub is_proxied: bool,
    pub require_proxy_headers: bool,
    pub allow_remote: bool,
    pub allow_unauthenticated_remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDecision {
    Authenticated(AuthMethod),
    LocalBypass,
    RemoteOpen,
    Reject {
        status: axum::http::StatusCode,
        reason: &'static str,
    },
}

/// Pure policy-matrix function — takes the three orthogonal inputs
/// (token status, whether auth is configured, connection context) and returns
/// the single outcome. No I/O, no async, fully unit-testable.
///
/// Matrix semantics:
/// * A valid token always wins.
/// * If auth is configured, token must be valid — missing/invalid → 401.
/// * If auth is NOT configured, the token (if any) is ignored and we fall
///   through to the locality / allow_remote checks. This keeps no-token and
///   bogus-token behaviour identical from the client's point of view.
/// * `require_proxy_headers=true` rejects bare-loopback (loopback without
///   X-Forwarded-For / X-Real-IP) — used when running behind a proxy that is
///   expected to be the only path in.
/// * Remote + no-auth requires BOTH `allow_remote` and
///   `allow_unauthenticated_remote` to be true; otherwise refused.
pub fn decide_auth(
    token_status: TokenStatus,
    auth_configured: bool,
    ctx: AuthContext,
) -> AuthDecision {
    use axum::http::StatusCode;

    if let TokenStatus::Valid(m) = token_status {
        return AuthDecision::Authenticated(m);
    }

    if auth_configured {
        return AuthDecision::Reject {
            status: StatusCode::UNAUTHORIZED,
            reason: match token_status {
                TokenStatus::InvalidToken => "invalid_token",
                TokenStatus::NoToken => "missing_token",
                TokenStatus::Valid(_) => unreachable!(),
            },
        };
    }

    // From here on, auth is NOT configured. Token (if any) is meaningless;
    // outcome depends solely on connection locality and remote policy.
    if ctx.is_local {
        if ctx.require_proxy_headers && !ctx.is_proxied {
            return AuthDecision::Reject {
                status: StatusCode::FORBIDDEN,
                reason: "loopback_without_proxy_headers",
            };
        }
        return AuthDecision::LocalBypass;
    }

    if ctx.allow_remote && ctx.allow_unauthenticated_remote {
        return AuthDecision::RemoteOpen;
    }

    AuthDecision::Reject {
        status: StatusCode::FORBIDDEN,
        reason: if ctx.allow_remote {
            "remote_no_auth_unauthenticated_not_allowed"
        } else {
            "remote_no_auth"
        },
    }
}

/// Authorizes a terminal request by checking the enabled flag, origin, proxy
/// headers, and the auth-policy decision matrix. Does **not** enforce the
/// per-IP WebSocket rate limit — that remains in the WebSocket handler itself
/// because it is WS-specific.
///
/// Returns `Ok(AuthMethod)` on success, or `Err(ready-made Response)` on any
/// rejection so callers can return it immediately.
pub(super) async fn authorize_terminal_request(
    headers: &axum::http::HeaderMap,
    uri: &axum::http::Uri,
    addr: SocketAddr,
    state: &AppState,
) -> Result<AuthMethod, axum::response::Response> {
    use axum::response::IntoResponse as _;

    let cfg = state.kernel.config_ref();
    let locality = detect_connection_locality(&addr, headers);

    if !cfg.terminal.enabled {
        warn!(
            ip = %locality.source_ip,
            proxied = locality.is_proxied,
            reason = "disabled",
            "Terminal WebSocket rejected — terminal is disabled"
        );
        return Err(axum::http::StatusCode::FORBIDDEN.into_response());
    }

    // Warn if terminal is enabled without any authentication configured.
    let valid_tokens = crate::server::valid_api_tokens(state.kernel.as_ref());
    let user_api_keys = crate::server::configured_user_api_keys(state.kernel.as_ref());
    let dashboard_auth = crate::server::has_dashboard_credentials(state.kernel.as_ref());
    let auth_configured = !valid_tokens.is_empty() || !user_api_keys.is_empty() || dashboard_auth;
    if !auth_configured {
        if cfg.terminal.allow_remote && cfg.terminal.allow_unauthenticated_remote {
            tracing::error!(
                "Terminal is enabled with allow_remote=true AND \
                 allow_unauthenticated_remote=true but NO authentication configured — \
                 unauthenticated shell access is exposed to the network. \
                 Set api_key, dashboard credentials, or users to prevent this."
            );
        } else if cfg.terminal.allow_remote {
            tracing::warn!(
                "Terminal has allow_remote=true without auth; remote connections \
                 will be refused unless allow_unauthenticated_remote is also set to true"
            );
        } else {
            warn!("Terminal is enabled without any authentication configured — any local connection gets unauthenticated shell access");
        }
    }

    let require_proxy_headers = cfg.terminal.require_proxy_headers;
    let listen_port = cfg.listen_port();

    let provided_token = ws_auth_token(headers, uri);

    // Validate the token (if any) before origin checks so that authenticated
    // requests are never rejected on origin alone. Origin validation is a CSRF
    // defense — it only matters when the browser silently attaches credentials
    // (cookies). Explicit Bearer tokens cannot be forged cross-origin, so
    // authenticated requests bypass origin checks entirely.
    let token_status = if let Some(token_str) = provided_token.as_deref() {
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
        let user_key_auth = !session_auth
            && user_api_keys
                .iter()
                .any(|user| crate::password_hash::verify_password(token_str, &user.api_key_hash));

        if api_auth {
            TokenStatus::Valid(AuthMethod::ApiKey)
        } else if session_auth {
            TokenStatus::Valid(AuthMethod::Session)
        } else if user_key_auth {
            TokenStatus::Valid(AuthMethod::UserKey)
        } else {
            TokenStatus::InvalidToken
        }
    } else {
        TokenStatus::NoToken
    };

    // Only enforce origin validation for unauthenticated requests.
    // Authenticated requests (valid token) are already protected against CSRF
    // because the token must be explicitly provided — browsers cannot inject it
    // cross-origin.
    if !matches!(token_status, TokenStatus::Valid(_)) {
        if let Err(reason) = validate_ws_origin(
            headers,
            listen_port,
            &cfg.terminal.allowed_origins,
            cfg.terminal.allow_remote,
        ) {
            if !cfg.terminal.allow_remote {
                warn!(
                    ip = %locality.source_ip,
                    proxied = locality.is_proxied,
                    reason = "origin_mismatch",
                    origin = %reason,
                    "Terminal rejected — origin validation failed"
                );
                return Err(axum::http::StatusCode::FORBIDDEN.into_response());
            }
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                reason = "origin_mismatch",
                origin = %reason,
                "Terminal origin mismatch — continuing to auth decision"
            );
        }
    }

    let decision = decide_auth(
        token_status,
        auth_configured,
        AuthContext {
            is_local: locality.is_local(),
            is_proxied: locality.is_proxied,
            require_proxy_headers,
            allow_remote: cfg.terminal.allow_remote,
            allow_unauthenticated_remote: cfg.terminal.allow_unauthenticated_remote,
        },
    );

    let auth_method = match decision {
        AuthDecision::Authenticated(m) => m,
        AuthDecision::LocalBypass => AuthMethod::LocalBypass,
        AuthDecision::RemoteOpen => AuthMethod::RemoteOpen,
        AuthDecision::Reject { status, reason } => {
            warn!(
                ip = %locality.source_ip,
                proxied = locality.is_proxied,
                reason = reason,
                "Terminal WebSocket rejected"
            );
            return Err(status.into_response());
        }
    };

    Ok(auth_method)
}

// ── REST: tmux window management ─────────────────────────────────────────────

async fn tmux_controller(state: &AppState) -> Result<TmuxController, axum::response::Response> {
    use axum::response::IntoResponse as _;
    let cfg = state.kernel.config_ref();
    if !cfg.terminal.tmux_enabled {
        return Err(axum::http::StatusCode::FORBIDDEN.into_response());
    }
    let tmux_path =
        std::path::PathBuf::from(cfg.terminal.tmux_binary_path.as_deref().unwrap_or("tmux"));
    Ok(TmuxController::new(
        tmux_path,
        DEFAULT_TMUX_SESSION_NAME.to_string(),
    ))
}

async fn list_windows(
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    use axum::response::IntoResponse as _;

    if let Err(resp) = authorize_terminal_request(&headers, &uri, addr, &state).await {
        return resp;
    }

    // Rate limiting is handled by the global GCRA middleware in server.rs.

    let ctrl = match tmux_controller(&state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(e) = ctrl.ensure_session().await {
        warn!(error = %e, "tmux ensure_session failed in list_windows");
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    match ctrl.list_windows().await {
        Ok(windows) => Json(serde_json::json!(windows)).into_response(),
        Err(e) => {
            warn!(error = %e, "tmux list_windows failed");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(serde::Deserialize)]
struct CreateWindowRequest {
    name: Option<String>,
}

async fn create_window(
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWindowRequest>,
) -> impl IntoResponse {
    use axum::response::IntoResponse as _;

    if let Err(resp) = authorize_terminal_request(&headers, &uri, addr, &state).await {
        return resp;
    }

    // Rate limiting is handled by the global GCRA middleware in server.rs.

    // Validate name before any tmux calls.
    if let Some(ref name) = body.name {
        if !validate_window_name(name) {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid_window_name"})),
            )
                .into_response();
        }
    }

    let ctrl = match tmux_controller(&state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(e) = ctrl.ensure_session().await {
        warn!(error = %e, "tmux ensure_session failed in create_window");
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    // Enforce window limit.
    // NOTE: The list_windows() check and new_window() call are two separate tmux
    // subprocess invocations, so a concurrent caller could slip in between. This
    // is accepted as a soft limit — the global GCRA rate limiter makes this race
    // benign in practice.
    let max_windows = state.kernel.config_ref().terminal.max_windows;
    match ctrl.list_windows().await {
        Ok(existing) => {
            if existing.len() >= max_windows as usize {
                return (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({"error": "window_limit_reached", "max": max_windows})),
                )
                    .into_response();
            }
        }
        Err(e) => {
            warn!(error = %e, "tmux list_windows failed in create_window");
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    match ctrl.new_window(body.name.as_deref()).await {
        Ok(window) => Json(serde_json::json!(window)).into_response(),
        Err(e) => {
            warn!(error = %e, "tmux new_window failed");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_window(
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(window_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    use axum::response::IntoResponse as _;

    if !crate::terminal_tmux::validate_window_id(&window_id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_window_id"})),
        )
            .into_response();
    }

    if let Err(resp) = authorize_terminal_request(&headers, &uri, addr, &state).await {
        return resp;
    }

    let ctrl = match tmux_controller(&state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(e) = ctrl.ensure_session().await {
        warn!(error = %e, "tmux ensure_session failed in delete_window");
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    match ctrl.kill_window(&window_id).await {
        Ok(()) => axum::http::StatusCode::OK.into_response(),
        Err(e) => {
            warn!(error = %e, %window_id, "tmux kill_window failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "kill_window_failed"})),
            )
                .into_response()
        }
    }
}

#[derive(serde::Deserialize)]
struct RenameWindowRequest {
    name: String,
}

async fn rename_window(
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(window_id): axum::extract::Path<String>,
    Json(body): Json<RenameWindowRequest>,
) -> impl IntoResponse {
    use axum::response::IntoResponse as _;

    if !crate::terminal_tmux::validate_window_id(&window_id) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_window_id"})),
        )
            .into_response();
    }

    if !validate_window_name(&body.name) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_window_name"})),
        )
            .into_response();
    }

    if let Err(resp) = authorize_terminal_request(&headers, &uri, addr, &state).await {
        return resp;
    }

    let ctrl = match tmux_controller(&state).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    if let Err(e) = ctrl.ensure_session().await {
        warn!(error = %e, "tmux ensure_session failed in rename_window");
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    match ctrl.rename_window(&window_id, &body.name).await {
        Ok(()) => axum::http::StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            warn!(error = %e, %window_id, "tmux rename_window failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "rename_window_failed"})),
            )
                .into_response()
        }
    }
}

// ── WebSocket handler ────────────────────────────────────────────────────────

pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let auth_method = match authorize_terminal_request(&headers, &uri, addr, &state).await {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let locality = detect_connection_locality(&addr, &headers);

    match auth_method {
        AuthMethod::LocalBypass => {
            warn!(
                ip = %locality.source_ip,
                local = locality.is_local(),
                proxied = locality.is_proxied,
                auth = auth_method.as_str(),
                "Terminal WebSocket connected"
            );
        }
        AuthMethod::RemoteOpen => {
            // Per-connection error-level log for every accepted unauthenticated
            // remote session — operators should see this in their error feed
            // each time it happens, not only once at handler entry.
            tracing::error!(
                ip = %locality.source_ip,
                local = locality.is_local(),
                proxied = locality.is_proxied,
                auth = auth_method.as_str(),
                "Terminal WebSocket connected with NO authentication over a remote connection \
                 (allow_remote=true, allow_unauthenticated_remote=true, no api_key/users/dashboard)"
            );
        }
        _ => {
            info!(
                ip = %locality.source_ip,
                local = locality.is_local(),
                proxied = locality.is_proxied,
                auth = auth_method.as_str(),
                "Terminal WebSocket connected"
            );
        }
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
        handle_terminal_ws(socket, state, ip, guard, initial_cols, initial_rows, uri)
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
    uri: axum::http::Uri,
) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    // Decide whether to attach to tmux or spawn a plain shell.
    let cfg = state.kernel.config_ref();
    let tmux_path_val = cfg
        .terminal
        .tmux_binary_path
        .as_deref()
        .unwrap_or("tmux")
        .to_string();
    let tmux_enabled = cfg.terminal.tmux_enabled;
    drop(cfg);

    let tmux_path_buf = std::path::PathBuf::from(&tmux_path_val);
    let tmux_avail =
        tmux_enabled && crate::terminal_tmux::TmuxController::is_available(&tmux_path_buf).await;

    // Validate optional ?window= query param.
    let requested_window = ws_query_param(&uri, "window");
    let valid_window = match &requested_window {
        Some(w) if crate::terminal_tmux::validate_window_id(w) => Some(w.clone()),
        Some(w) => {
            warn!(requested = %w, "Invalid ?window= query param — connecting without window");
            None
        }
        None => None,
    };

    let (mut pty, mut pty_rx) = if tmux_avail {
        // Ensure tmux session exists and optionally select a window.
        let ctrl = crate::terminal_tmux::TmuxController::new(
            tmux_path_buf,
            DEFAULT_TMUX_SESSION_NAME.to_string(),
        );
        if let Err(e) = ctrl.ensure_session().await {
            warn!(error = %e, "tmux session init failed");
            let _ = send_json(
                &sender,
                &serde_json::json!({
                    "type": "error",
                    "content": "tmux session init failed"
                }),
            )
            .await;
            return;
        }
        if let Some(wid) = &valid_window {
            if let Err(e) = ctrl.select_window(wid).await {
                warn!(error = %e, window = %wid, "tmux select window failed");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "tmux select window failed"
                    }),
                )
                .await;
                // Continue anyway — still attach to session.
            }
        }
        match PtySession::spawn_tmux_attached(
            &tmux_path_val,
            DEFAULT_TMUX_SESSION_NAME,
            initial_cols,
            initial_rows,
        ) {
            Ok((pty, rx)) => (pty, rx),
            Err(e) => {
                warn!(error = %e, "Failed to spawn tmux terminal");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Failed to spawn tmux terminal"
                    }),
                )
                .await;
                return;
            }
        }
    } else {
        match PtySession::spawn(initial_cols, initial_rows) {
            Ok((pty, rx)) => (pty, rx),
            Err(e) => {
                warn!(error = %e, "Failed to spawn terminal");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Failed to spawn terminal"
                    }),
                )
                .await;
                return;
            }
        }
    };

    // Track current PTY dimensions for re-resize after window switch.
    let mut current_cols = initial_cols.unwrap_or(120);
    let mut current_rows = initial_rows.unwrap_or(40);

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
    // Use the terminal-specific input budget: PTY sessions send one WS
    // message per keystroke, so the generic `ws_messages_per_minute` (sized
    // for chat where a "message" is a whole utterance) was two orders of
    // magnitude too low and made interactive programs like vim appear to
    // freeze after ~10 keys.
    let max_input_per_min: usize = rl_cfg.ws_terminal_messages_per_minute as usize;
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
                                            warn!(error = %e, "PTY write failed");
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": "PTY write failed"
                                                }),
                                            )
                                            .await;
                                        }
                                    }
                                    ClientMessage::Resize { cols, rows } => {
                                        current_cols = *cols;
                                        current_rows = *rows;
                                        if let Err(e) = pty.resize(*cols, *rows) {
                                            warn!(error = %e, "PTY resize failed");
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": "PTY resize failed"
                                                }),
                                            )
                                            .await;
                                        }
                                    }
                                    ClientMessage::SwitchWindow { window } => {
                                        let window_id = window.clone();
                                        let (tmux_enabled, tmux_path) = {
                                            let cfg = state.kernel.config_ref();
                                            (
                                                cfg.terminal.tmux_enabled,
                                                std::path::PathBuf::from(
                                                    cfg.terminal.tmux_binary_path.as_deref().unwrap_or("tmux"),
                                                ),
                                            )
                                        };
                                        if tmux_enabled {
                                            let ctrl = crate::terminal_tmux::TmuxController::new(
                                                tmux_path,
                                                DEFAULT_TMUX_SESSION_NAME.to_string(),
                                            );
                                            match ctrl.select_window(&window_id).await {
                                                Ok(()) => {
                                                    // Re-resize PTY after tmux window switch.
                                                    if let Err(e) = pty.resize(current_cols, current_rows) {
                                                        warn!(
                                                            error = %e,
                                                            cols = current_cols,
                                                            rows = current_rows,
                                                            "failed to resize PTY after window switch"
                                                        );
                                                    }
                                                    let _ = send_json(
                                                        &sender,
                                                        &serde_json::to_value(
                                                            ServerMessage::ActiveWindow {
                                                                window_id,
                                                            },
                                                        )
                                                        .unwrap(),
                                                    )
                                                    .await;
                                                }
                                                Err(e) => {
                                                    warn!(error = %e, "tmux switch window failed");
                                                    let _ = send_json(
                                                        &sender,
                                                        &serde_json::json!({
                                                            "type": "error",
                                                            "content": "Switch failed"
                                                        }),
                                                    )
                                                    .await;
                                                }
                                            }
                                        } else {
                                            let _ = send_json(
                                                &sender,
                                                &serde_json::json!({
                                                    "type": "error",
                                                    "content": "tmux not available"
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

    // ── Security: SwitchWindow validation ────────────────────────────────────

    #[test]
    fn switch_window_rejects_missing_at() {
        let msg = ClientMessage::SwitchWindow {
            window: "1".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_rejects_shell_injection() {
        let msg = ClientMessage::SwitchWindow {
            window: "@1;ls".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_rejects_command_substitution() {
        let msg = ClientMessage::SwitchWindow {
            window: "@$(whoami)".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_rejects_path_traversal() {
        let msg = ClientMessage::SwitchWindow {
            window: "@../".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_rejects_spaces() {
        let msg = ClientMessage::SwitchWindow {
            window: "@1 2".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_rejects_ten_digits() {
        let msg = ClientMessage::SwitchWindow {
            window: "@1234567890".to_string(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn switch_window_accepts_valid_id() {
        let msg = ClientMessage::SwitchWindow {
            window: "@42".to_string(),
        };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn switch_window_accepts_max_9_digits() {
        let msg = ClientMessage::SwitchWindow {
            window: "@123456789".to_string(),
        };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn switch_window_rejects_empty() {
        let msg = ClientMessage::SwitchWindow {
            window: String::new(),
        };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn active_window_serializes() {
        let msg = ServerMessage::ActiveWindow {
            window_id: "@3".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"active_window""#));
        assert!(json.contains(r#""window_id":"@3""#));
    }

    #[test]
    fn window_name_rejects_shell_injection_in_create() {
        // window names are passed via `Command::arg` (not a shell), so
        // validate_window_name only rejects control chars and the pipe character
        // used as the list-windows format separator. Shell metacharacters like
        // `;`, `$()`, and backticks are allowed — they pose no injection risk
        // when passed as a direct argument.
        assert!(!crate::terminal_tmux::validate_window_name("a|b"));
        assert!(!crate::terminal_tmux::validate_window_name("foo\0bar"));
    }

    #[test]
    fn window_name_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(!crate::terminal_tmux::validate_window_name(&long));
    }

    #[test]
    fn window_name_accepts_valid() {
        assert!(crate::terminal_tmux::validate_window_name("editor"));
        assert!(crate::terminal_tmux::validate_window_name("my-app_01"));
    }

    #[test]
    fn window_name_rejects_all_special_chars() {
        // validate_window_name only rejects the pipe separator and control chars.
        // Other punctuation is intentionally allowed (see 5d271afa which broadened
        // the allowlist to support Unicode, CJK, emoji, and common punctuation).
        for bad in &["a|b", "foo\0bar", "foo\x1fbar"] {
            assert!(
                !crate::terminal_tmux::validate_window_name(bad),
                "should reject: {bad:?}"
            );
        }
    }

    #[test]
    fn window_name_rejects_control_chars() {
        for bad in &["foo\0bar", "foo\x01bar", "foo\x1fbar", "foo\x7fbar"] {
            assert!(
                !crate::terminal_tmux::validate_window_name(bad),
                "should reject control char"
            );
        }
    }

    #[test]
    fn window_id_rejects_negative() {
        assert!(!crate::terminal_tmux::validate_window_id("@-1"));
    }

    #[test]
    fn window_id_rejects_unicode() {
        assert!(!crate::terminal_tmux::validate_window_id("@①"));
        assert!(!crate::terminal_tmux::validate_window_id("@１"));
    }

    #[test]
    fn resize_accepts_min_valid() {
        let msg = ClientMessage::Resize { cols: 1, rows: 1 };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn resize_accepts_max_valid() {
        let msg = ClientMessage::Resize {
            cols: MAX_COLS,
            rows: MAX_ROWS,
        };
        assert!(msg.validate().is_ok());
    }

    #[test]
    fn input_one_byte_over_limit_rejected() {
        let data = "x".repeat(64 * 1024 + 1);
        let msg = ClientMessage::Input { data };
        assert!(msg.validate().is_err());
    }

    #[test]
    fn max_ws_msg_size_is_64kb() {
        assert_eq!(crate::routes::terminal::MAX_WS_MSG_SIZE, 64 * 1024);
    }

    #[test]
    fn binary_output_serializes_data_and_flag() {
        let msg = ServerMessage::Output {
            data: "SGVsbG8=".to_string(),
            binary: Some(true),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""binary":true"#));
        assert!(json.contains(r#""data":"SGVsbG8=""#));
    }
}

// ---------------------------------------------------------------------------
// WebSocket auth policy unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod terminal_ws_auth_tests {
    use std::net::SocketAddr;

    use crate::ws::{detect_connection_locality, validate_ws_origin};
    use axum::http::HeaderMap;

    #[test]
    fn validate_ws_origin_allows_matching_port_on_localhost() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_ok());
    }

    #[test]
    fn validate_ws_origin_rejects_mismatched_port() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:8080".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_err());
    }

    #[test]
    fn validate_ws_origin_wildcard_requires_allow_remote() {
        // Wildcard + allow_remote=false → rejected
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.example:9999".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &["*".to_string()], false).is_err());

        // Wildcard + allow_remote=true → allowed
        assert!(validate_ws_origin(&headers, Some(4545), &["*".to_string()], true).is_ok());

        // Also works with https
        let mut headers2 = HeaderMap::new();
        headers2.insert("origin", "https://other.host:1234".parse().unwrap());
        assert!(validate_ws_origin(&headers2, Some(4545), &["*".to_string()], true).is_ok());
    }

    #[test]
    fn validate_ws_origin_allows_specific_allowed_origins() {
        let allowed = vec!["https://my.domain.com".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://my.domain.com".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &allowed, false).is_ok());
    }

    #[test]
    fn validate_ws_origin_rejects_non_matching_allowed_origins() {
        let allowed = vec!["https://my.domain.com".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://evil.example".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &allowed, false).is_err());
    }

    // -----------------------------------------------------------------------
    // Combined auth decision chain tests
    // These test the locality + auth decision logic that mirrors terminal_ws.
    // The handler itself is hard to unit-test without full AppState, so we test
    // the decision primitives in combination.
    // -----------------------------------------------------------------------

    #[test]
    fn locality_local_no_proxy_is_local() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let headers = HeaderMap::new();
        let locality = detect_connection_locality(&addr, &headers);
        assert!(locality.is_local());
        assert!(locality.is_loopback);
        assert!(!locality.is_proxied);
    }

    #[test]
    fn locality_loopback_with_proxy_header_not_local() {
        // require_proxy_headers=true scenario: loopback + XFF = not local → denied local_bypass
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());
        let locality = detect_connection_locality(&addr, &headers);
        assert!(!locality.is_local()); // loopback but proxied → not local
        assert!(locality.is_loopback);
        assert!(locality.is_proxied);
    }

    #[test]
    fn origin_localhost_same_port_passes() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_ok());
    }

    #[test]
    fn origin_external_same_port_rejected_without_allowed() {
        // This is the CSRF fix: external host on same port → rejected
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://attacker.example:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_err());
    }

    #[test]
    fn origin_external_allowed_via_explicit_entry() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://myserver.lan:4545".parse().unwrap());
        assert!(validate_ws_origin(
            &headers,
            Some(4545),
            &["http://myserver.lan:4545".to_string()],
            false
        )
        .is_ok());
    }

    #[test]
    fn wildcard_origin_rejected_without_allow_remote() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.example:9999".parse().unwrap());
        // wildcard + allow_remote=false → rejected
        assert!(validate_ws_origin(&headers, Some(4545), &["*".to_string()], false).is_err());
    }

    #[test]
    fn wildcard_origin_allowed_with_allow_remote() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.example:9999".parse().unwrap());
        // wildcard + allow_remote=true → allowed
        assert!(validate_ws_origin(&headers, Some(4545), &["*".to_string()], true).is_ok());
    }

    #[test]
    fn locality_remote_no_proxy_not_local() {
        let addr: SocketAddr = "8.8.8.8:12345".parse().unwrap();
        let headers = HeaderMap::new();
        let locality = detect_connection_locality(&addr, &headers);
        assert!(!locality.is_local());
        assert!(!locality.is_loopback);
    }

    #[test]
    fn origin_ipv6_loopback_same_port_passes() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://[::1]:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_ok());
    }

    #[test]
    fn origin_127_0_0_1_same_port_passes() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://127.0.0.1:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &[], false).is_ok());
    }

    #[test]
    fn origin_validation_fails_closed_on_unknown_listen_port() {
        // Malformed api_listen → listen_port() returns None. Even a same-port
        // localhost origin must not be auto-allowed in that case.
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, None, &[], false).is_err());
    }

    #[test]
    fn origin_host_comparison_is_case_insensitive() {
        // Per RFC 3986 host components are case-insensitive.
        let allowed = vec!["http://My.Domain.com:4545".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://my.domain.com:4545".parse().unwrap());
        assert!(validate_ws_origin(&headers, Some(4545), &allowed, false).is_ok());
    }
}

#[cfg(test)]
mod auth_policy_matrix_tests {
    //! Exhaustive coverage of the terminal auth decision table.
    //! Mirrors the scenarios operators actually face:
    //!
    //! axes: (auth_configured) × (token = valid/invalid/missing) ×
    //!       (local/remote) × (allow_remote) × (allow_unauthenticated_remote) ×
    //!       (require_proxy_headers & is_proxied)
    use super::{decide_auth, AuthContext, AuthDecision, AuthMethod, TokenStatus};
    use axum::http::StatusCode;

    fn ctx_local() -> AuthContext {
        AuthContext {
            is_local: true,
            is_proxied: false,
            require_proxy_headers: false,
            allow_remote: false,
            allow_unauthenticated_remote: false,
        }
    }

    fn ctx_remote() -> AuthContext {
        AuthContext {
            is_local: false,
            is_proxied: false,
            require_proxy_headers: false,
            allow_remote: false,
            allow_unauthenticated_remote: false,
        }
    }

    // ── valid token always wins, regardless of other knobs ────────────────
    #[test]
    fn valid_token_authenticates_even_when_remote_and_no_allow_remote() {
        let d = decide_auth(TokenStatus::Valid(AuthMethod::ApiKey), true, ctx_remote());
        assert_eq!(d, AuthDecision::Authenticated(AuthMethod::ApiKey));
    }

    // ── auth_configured: missing/invalid token rejected ───────────────────
    #[test]
    fn auth_configured_invalid_token_returns_401() {
        let d = decide_auth(TokenStatus::InvalidToken, true, ctx_local());
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::UNAUTHORIZED,
                reason: "invalid_token"
            }
        ));
    }

    #[test]
    fn auth_configured_missing_token_returns_401() {
        let d = decide_auth(TokenStatus::NoToken, true, ctx_local());
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::UNAUTHORIZED,
                reason: "missing_token"
            }
        ));
    }

    // ── !auth_configured: token content ignored ───────────────────────────
    #[test]
    fn no_auth_configured_local_no_token_is_local_bypass() {
        let d = decide_auth(TokenStatus::NoToken, false, ctx_local());
        assert_eq!(d, AuthDecision::LocalBypass);
    }

    #[test]
    fn no_auth_configured_local_invalid_token_is_also_local_bypass() {
        // Policy consistency: bogus token and no token produce the SAME outcome
        // when auth is not configured — there is nothing to check against.
        let d = decide_auth(TokenStatus::InvalidToken, false, ctx_local());
        assert_eq!(d, AuthDecision::LocalBypass);
    }

    // ── remote + no auth: needs BOTH allow_remote AND allow_unauth ────────
    #[test]
    fn no_auth_remote_bare_allow_remote_without_unauth_refused() {
        let mut c = ctx_remote();
        c.allow_remote = true;
        // allow_unauthenticated_remote stays false → hard-refuse
        let d = decide_auth(TokenStatus::NoToken, false, c);
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::FORBIDDEN,
                reason: "remote_no_auth_unauthenticated_not_allowed",
            }
        ));
    }

    #[test]
    fn no_auth_remote_with_both_flags_is_remote_open() {
        let mut c = ctx_remote();
        c.allow_remote = true;
        c.allow_unauthenticated_remote = true;
        let d = decide_auth(TokenStatus::NoToken, false, c);
        assert_eq!(d, AuthDecision::RemoteOpen);
    }

    #[test]
    fn no_auth_remote_invalid_token_matches_no_token_behavior() {
        // #1 in the review: same client intent (remote + no auth), bogus token
        // vs no token should produce the SAME outcome — no longer 401 vs 200.
        let mut c = ctx_remote();
        c.allow_remote = true;
        c.allow_unauthenticated_remote = true;

        let d_bogus = decide_auth(TokenStatus::InvalidToken, false, c);
        let d_none = decide_auth(TokenStatus::NoToken, false, c);
        assert_eq!(d_bogus, d_none);
        assert_eq!(d_bogus, AuthDecision::RemoteOpen);
    }

    #[test]
    fn no_auth_remote_no_allow_remote_is_rejected() {
        let d = decide_auth(TokenStatus::NoToken, false, ctx_remote());
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::FORBIDDEN,
                reason: "remote_no_auth",
            }
        ));
    }

    // ── require_proxy_headers: reject bare loopback ───────────────────────
    #[test]
    fn require_proxy_headers_rejects_bare_loopback() {
        let mut c = ctx_local();
        c.require_proxy_headers = true;
        c.is_proxied = false;
        let d = decide_auth(TokenStatus::NoToken, false, c);
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::FORBIDDEN,
                reason: "loopback_without_proxy_headers",
            }
        ));
    }

    #[test]
    fn require_proxy_headers_with_proxy_header_allowed() {
        let mut c = AuthContext {
            is_local: true,
            is_proxied: true,
            require_proxy_headers: true,
            allow_remote: false,
            allow_unauthenticated_remote: false,
        };
        // Note: real detect_connection_locality would demote proxied loopback
        // to non-local; here we test the decision function in isolation.
        c.is_local = true;
        let d = decide_auth(TokenStatus::NoToken, false, c);
        assert_eq!(d, AuthDecision::LocalBypass);
    }

    // ── valid token variants always authenticate ──────────────────────────
    #[test]
    fn valid_token_session_authenticates() {
        let d = decide_auth(TokenStatus::Valid(AuthMethod::Session), true, ctx_remote());
        assert_eq!(d, AuthDecision::Authenticated(AuthMethod::Session));
    }

    #[test]
    fn valid_token_user_key_authenticates() {
        let d = decide_auth(TokenStatus::Valid(AuthMethod::UserKey), true, ctx_remote());
        assert_eq!(d, AuthDecision::Authenticated(AuthMethod::UserKey));
    }

    // ── auth_configured + remote + invalid token → 401 ────────────────────
    #[test]
    fn auth_configured_remote_invalid_token_still_401() {
        let mut c = ctx_remote();
        c.allow_remote = true;
        c.allow_unauthenticated_remote = true;
        let d = decide_auth(TokenStatus::InvalidToken, true, c);
        assert!(matches!(
            d,
            AuthDecision::Reject {
                status: StatusCode::UNAUTHORIZED,
                reason: "invalid_token"
            }
        ));
    }
}
