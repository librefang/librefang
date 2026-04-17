//! WebSocket handler for real-time agent chat.
//!
//! Provides a persistent bidirectional channel between the client
//! and an agent. Messages are exchanged as JSON:
//!
//! Client → Server: `{"type":"message","content":"..."}`
//! Server → Client: `{"type":"typing","state":"start|tool|stop"}`
//! Server → Client: `{"type":"text_delta","content":"..."}`
//! Server → Client: `{"type":"response","content":"...","input_tokens":N,"output_tokens":N,"iterations":N}`
//! Server → Client: `{"type":"error","content":"..."}`
//! Server → Client: `{"type":"agents_updated","agents":[...]}`
//! Server → Client: `{"type":"silent_complete"}` (agent chose NO_REPLY)
//! Server → Client: `{"type":"canvas","canvas_id":"...","html":"...","title":"..."}`

use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Uri};
use axum::response::IntoResponse;
use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use librefang_channels::types::SenderContext;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_runtime::llm_driver::{StreamEvent, PHASE_RESPONSE_COMPLETE};
use librefang_runtime::llm_errors;
use librefang_types::agent::AgentId;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use url::Url;

// ---------------------------------------------------------------------------
// Verbose Level
// ---------------------------------------------------------------------------

/// Per-connection tool detail verbosity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum VerboseLevel {
    /// Suppress tool details (only tool name + success/fail).
    Off = 0,
    /// Truncated tool details.
    On = 1,
    /// Full tool details (default).
    Full = 2,
}

impl VerboseLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Full,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Full,
            Self::Full => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Full => "full",
        }
    }
}

// ---------------------------------------------------------------------------
// Connection Tracking
// ---------------------------------------------------------------------------

/// Global connection tracker (DashMap<IpAddr, AtomicUsize>).
pub fn ws_tracker() -> &'static DashMap<IpAddr, AtomicUsize> {
    static TRACKER: std::sync::OnceLock<DashMap<IpAddr, AtomicUsize>> = std::sync::OnceLock::new();
    TRACKER.get_or_init(DashMap::new)
}

/// RAII guard that decrements the connection count on drop.
pub struct WsConnectionGuard {
    ip: IpAddr,
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        if let Some(entry) = ws_tracker().get(&self.ip) {
            let prev = entry.value().fetch_sub(1, Ordering::Relaxed);
            if prev <= 1 {
                drop(entry);
                ws_tracker().remove(&self.ip);
            }
        }
    }
}

/// Try to acquire a WS connection slot for the given IP.
/// Returns None if the IP has reached `max_ws_per_ip`.
pub fn try_acquire_ws_slot(ip: IpAddr, max_ws_per_ip: usize) -> Option<WsConnectionGuard> {
    let entry = ws_tracker()
        .entry(ip)
        .or_insert_with(|| AtomicUsize::new(0));
    let current = entry.value().fetch_add(1, Ordering::Relaxed);
    if current >= max_ws_per_ip {
        entry.value().fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    Some(WsConnectionGuard { ip })
}

pub fn ws_query_param(uri: &Uri, key: &str) -> Option<String> {
    let query = uri.query()?;
    url::form_urlencoded::parse(query.as_bytes()).find_map(|(param, value)| {
        if param == key {
            Some(value.into_owned())
        } else {
            None
        }
    })
}

pub fn ws_auth_token(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(ToOwned::to_owned)
        .or_else(|| ws_query_param(uri, "token"))
}

/// Validates the WebSocket `Origin` header against allowed origins.
/// Returns Ok(()) if: (a) no Origin header (non-browser client), or (b) Origin matches.
/// Returns Err(reason) if Origin is present but doesn't match any allowed origin.
pub fn validate_ws_origin(
    headers: &HeaderMap,
    listen_port: Option<u16>,
    extra_origins: &[String],
    allow_remote: bool,
) -> Result<(), String> {
    let origin = match headers.get("origin") {
        Some(v) => v.to_str().map_err(|_| "Invalid origin header encoding")?,
        None => return Ok(()),
    };

    let parsed = Url::parse(origin).map_err(|_| format!("Invalid origin URL: {origin}"))?;
    let origin_scheme = parsed.scheme();
    let origin_host = parsed
        .host_str()
        .ok_or_else(|| format!("Origin missing host: {origin}"))?;
    if origin_scheme != "http" && origin_scheme != "https" {
        return Err(format!("Origin {origin} not in allowed list"));
    }

    let origin_port = if origin_scheme == "https" {
        parsed.port().unwrap_or(443)
    } else {
        parsed.port().unwrap_or(80)
    };

    // Only loopback hosts (localhost / 127.0.0.1 / ::1) on the same port
    // are auto-allowed. Fail closed when listen_port is unknown — otherwise
    // a malformed api_listen would cause us to trust the wrong localhost:port.
    if let Some(lp) = listen_port {
        if origin_port == lp {
            let normalized = normalize_origin_host(origin_host);
            if normalized.eq_ignore_ascii_case("localhost") {
                return Ok(());
            }
        }
    }

    // Wildcard "*" means allow all origins — only permitted when allow_remote is true.
    // NOTE: The scheme check above (http/https only) runs before this wildcard path,
    // so non-http schemes are always rejected regardless of wildcard.
    if allow_remote && extra_origins.iter().any(|o| o == "*") {
        return Ok(());
    }

    for extra in extra_origins {
        let extra_parsed =
            Url::parse(extra).map_err(|_| format!("Invalid extra origin URL: {extra}"))?;
        let extra_scheme = extra_parsed.scheme();
        let extra_host = extra_parsed
            .host_str()
            .ok_or_else(|| format!("Origin missing host in allowed origin: {extra}"))?;
        let extra_port = if extra_scheme == "https" {
            extra_parsed.port().unwrap_or(443)
        } else {
            extra_parsed.port().unwrap_or(80)
        };

        let normalized_extra_host = normalize_origin_host(extra_host);

        if normalized_extra_host.eq_ignore_ascii_case(normalize_origin_host(origin_host))
            && extra_scheme == origin_scheme
            && origin_port == extra_port
        {
            return Ok(());
        }
    }

    Err(format!("Origin {origin} not in allowed list"))
}

fn normalize_origin_host(host: &str) -> &str {
    match host {
        "localhost" | "127.0.0.1" | "::1" | "[::1]" => "localhost",
        _ => host,
    }
}

// ---------------------------------------------------------------------------
// Connection Locality Detection
// ---------------------------------------------------------------------------

pub struct ConnectionLocality {
    pub source_ip: IpAddr,
    pub is_loopback: bool,
    pub is_proxied: bool,
    pub forwarded_ip: Option<IpAddr>,
}

impl ConnectionLocality {
    pub fn is_local(&self) -> bool {
        self.is_loopback && !self.is_proxied
    }
}

pub fn detect_connection_locality(addr: &SocketAddr, headers: &HeaderMap) -> ConnectionLocality {
    let source_ip = addr.ip();
    let is_loopback = source_ip.is_loopback();

    let proxy_headers = [
        "x-forwarded-for",
        "x-real-ip",
        "cf-connecting-ip",
        "fly-client-ip",
        "true-client-ip",
    ];

    let is_proxied = proxy_headers.iter().any(|h| headers.contains_key(*h));

    let forwarded_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
        })
        .or_else(|| {
            headers
                .get("cf-connecting-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
        });

    debug!(
        source_ip = %source_ip,
        is_loopback,
        is_proxied,
        forwarded_ip = ?forwarded_ip,
        "WS connection locality detected"
    );

    ConnectionLocality {
        source_ip,
        is_loopback,
        is_proxied,
        forwarded_ip,
    }
}

// ---------------------------------------------------------------------------
// WS Upgrade Handler
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/ws — Upgrade to WebSocket for real-time chat.
///
/// SECURITY: Authenticates via Bearer token in Authorization header
/// or `?token=` query parameter (for browser WebSocket clients that
/// cannot set custom headers).
pub async fn agent_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    // SECURITY: Authenticate WebSocket upgrades (bypasses middleware).
    let valid_tokens = crate::server::valid_api_tokens(state.kernel.as_ref());
    let user_api_keys = crate::server::configured_user_api_keys(state.kernel.as_ref());
    let dashboard_auth = crate::server::has_dashboard_credentials(state.kernel.as_ref());
    let auth_required = !valid_tokens.is_empty() || !user_api_keys.is_empty() || dashboard_auth;
    if auth_required {
        // SECURITY: Use constant-time comparison to prevent timing attacks on auth tokens.
        let matches_any = |token: &str| -> bool {
            use subtle::ConstantTimeEq;
            valid_tokens.iter().any(|key| {
                token.len() == key.len() && token.as_bytes().ct_eq(key.as_bytes()).into()
            })
        };

        let provided_token = ws_auth_token(&headers, &uri);
        let mut session_auth = false;
        let mut user_key_auth = false;
        let mut api_auth = false;
        if let Some(token_str) = provided_token.as_deref() {
            api_auth = matches_any(token_str);
            let mut sessions = state.active_sessions.write().await;
            sessions.retain(|_, st| {
                !crate::password_hash::is_token_expired(
                    st,
                    crate::password_hash::DEFAULT_SESSION_TTL_SECS,
                )
            });
            session_auth = sessions.contains_key(token_str);
            drop(sessions);

            // Check per-user API keys (hashed with Argon2).
            if !session_auth {
                user_key_auth = user_api_keys.iter().any(|user| {
                    crate::password_hash::verify_password(token_str, &user.api_key_hash)
                });
            }
        }

        if !api_auth && !session_auth && !user_key_auth {
            warn!("WebSocket upgrade rejected: invalid auth");
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    }

    // SECURITY: Enforce per-IP WebSocket connection limit
    let ip = addr.ip();
    let max_ws_per_ip = state.kernel.config_ref().rate_limit.max_ws_per_ip;

    let guard = match try_acquire_ws_slot(ip, max_ws_per_ip) {
        Some(g) => g,
        None => {
            warn!(ip = %ip, max_ws_per_ip, "WebSocket rejected: too many connections from IP");
            return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Verify agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let id_str = id.clone();
    ws.on_upgrade(move |socket| handle_agent_ws(socket, state, agent_id, id_str, ip, guard))
        .into_response()
}

// ---------------------------------------------------------------------------
// WS Connection Handler
// ---------------------------------------------------------------------------

/// Handle a WebSocket connection to an agent.
///
/// The `_guard` is an RAII handle that decrements the per-IP connection
/// counter when this function returns (connection closes).
async fn handle_agent_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    agent_id: AgentId,
    id_str: String,
    client_ip: IpAddr,
    _guard: WsConnectionGuard,
) {
    info!(agent_id = %id_str, "WebSocket connected");

    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    // Per-connection verbose level (default: Full)
    let verbose = Arc::new(AtomicU8::new(VerboseLevel::Full as u8));

    // Send initial connection confirmation
    let _ = send_json(
        &sender,
        &serde_json::json!({
            "type": "connected",
            "agent_id": id_str,
        }),
    )
    .await;

    // Spawn background task: periodic agent list updates with change detection
    let sender_clone = Arc::clone(&sender);
    let state_clone = Arc::clone(&state);
    let update_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut last_hash: u64 = 0;
        loop {
            interval.tick().await;
            let agents: Vec<serde_json::Value> = state_clone
                .kernel
                .agent_registry()
                .list()
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "model_provider": e.manifest.model.provider,
                        "model_name": e.manifest.model.model,
                    })
                })
                .collect();

            // Change detection: hash the agent list and only send on change
            let mut hasher = DefaultHasher::new();
            for a in &agents {
                serde_json::to_string(a)
                    .unwrap_or_default()
                    .hash(&mut hasher);
            }
            let new_hash = hasher.finish();
            if new_hash == last_hash {
                continue; // No change — skip broadcast
            }
            last_hash = new_hash;

            if send_json(
                &sender_clone,
                &serde_json::json!({
                    "type": "agents_updated",
                    "agents": agents,
                }),
            )
            .await
            .is_err()
            {
                break; // Client disconnected
            }
        }
    });

    // Per-connection rate limiting (configurable via [rate_limit])
    let rl_cfg = state.kernel.config_ref().rate_limit.clone();
    let max_per_min: usize = rl_cfg.ws_messages_per_minute as usize;
    let ws_idle_timeout = Duration::from_secs(rl_cfg.ws_idle_timeout_secs);
    let mut msg_times: Vec<std::time::Instant> = Vec::new();
    let window: Duration = Duration::from_secs(60);

    // Track last activity for idle timeout
    let mut last_activity = std::time::Instant::now();

    // Main message loop with idle timeout
    loop {
        let msg = tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(m) => m,
                    None => break, // Stream ended
                }
            }
            _ = tokio::time::sleep(ws_idle_timeout.saturating_sub(last_activity.elapsed())) => {
                let timeout_secs = ws_idle_timeout.as_secs();
                info!(agent_id = %id_str, timeout_secs, "WebSocket idle timeout");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": format!("Connection closed due to inactivity ({timeout_secs}s timeout)"),
                    }),
                ).await;
                break;
            }
        };

        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "WebSocket receive error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                last_activity = std::time::Instant::now();

                // SECURITY: Reject oversized WebSocket messages (64KB max)
                const MAX_WS_MSG_SIZE: usize = 64 * 1024;
                if text.len() > MAX_WS_MSG_SIZE {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Message too large (max 64KB)",
                        }),
                    )
                    .await;
                    continue;
                }

                // SECURITY: Per-connection rate limiting
                let now = std::time::Instant::now();
                msg_times.retain(|t| now.duration_since(*t) < window);
                if msg_times.len() >= max_per_min {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": format!("Rate limit exceeded. Max {max_per_min} messages per minute."),
                        }),
                    )
                    .await;
                    continue;
                }
                msg_times.push(now);

                handle_text_message(&sender, &state, agent_id, &text, &verbose, client_ip).await;
            }
            Message::Close(_) => {
                info!(agent_id = %id_str, "WebSocket closed by client");
                break;
            }
            Message::Ping(data) => {
                last_activity = std::time::Instant::now();
                let mut s = sender.lock().await;
                let _ = s.send(Message::Pong(data)).await;
            }
            _ => {} // Ignore binary and pong
        }
    }

    // Cleanup
    update_handle.abort();
    info!(agent_id = %id_str, "WebSocket disconnected");
}

// ---------------------------------------------------------------------------
// Message Handler
// ---------------------------------------------------------------------------

/// Handle a text message from the WebSocket client.
async fn handle_text_message(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    text: &str,
    verbose: &Arc<AtomicU8>,
    client_ip: IpAddr,
) {
    // Parse the message
    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // Treat plain text as a message
            serde_json::json!({"type": "message", "content": text})
        }
    };

    let msg_type = parsed["type"].as_str().unwrap_or("message");

    match msg_type {
        "message" => {
            let raw_content = match parsed["content"].as_str() {
                Some(c) if !c.trim().is_empty() => c.to_string(),
                _ => {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Missing or empty 'content' field",
                        }),
                    )
                    .await;
                    return;
                }
            };

            // Per-message toggles from the chat UI.
            // `thinking`: override deep-thinking for this call (None = manifest default).
            // `show_thinking`: whether to surface thinking deltas/content to the UI.
            let thinking_override: Option<bool> = parsed["thinking"].as_bool();
            let show_thinking: bool = parsed["show_thinking"].as_bool().unwrap_or(true);

            // Sanitize inbound user input
            let content = sanitize_user_input(&raw_content);
            if content.is_empty() {
                let _ = send_json(
                    sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Message content is empty after sanitization",
                    }),
                )
                .await;
                return;
            }

            // Reject messages when provider API key is missing
            {
                let registry = state.kernel.agent_registry();
                if let Some(entry) = registry.get(agent_id) {
                    let dm = {
                        let dm_override = state
                            .kernel
                            .default_model_override_ref()
                            .read()
                            .unwrap_or_else(|e| e.into_inner());
                        crate::routes::agents::effective_default_model(
                            &state.kernel.config_ref().default_model,
                            dm_override.as_ref(),
                        )
                    };
                    let provider = if entry.manifest.model.provider.is_empty()
                        || entry.manifest.model.provider == "default"
                    {
                        &dm.provider
                    } else {
                        &entry.manifest.model.provider
                    };
                    let is_missing = state
                        .kernel
                        .model_catalog_ref()
                        .read()
                        .ok()
                        .and_then(|cat| {
                            cat.get_provider(provider)
                                .map(|p| !p.auth_status.is_available())
                        })
                        .unwrap_or(false);
                    if is_missing {
                        let _ = send_json(
                            sender,
                            &serde_json::json!({
                                "type": "error",
                                "content": format!("API key not configured for provider '{}'. Set it in Settings > Providers.", provider),
                            }),
                        )
                        .await;
                        return;
                    }
                }
            }

            // Resolve file attachments into image content blocks
            let mut has_images = false;
            if let Some(attachments) = parsed["attachments"].as_array() {
                let refs: Vec<crate::types::AttachmentRef> = attachments
                    .iter()
                    .filter_map(|a| serde_json::from_value(a.clone()).ok())
                    .collect();
                if !refs.is_empty() {
                    let image_blocks = crate::routes::resolve_attachments(&refs);
                    if !image_blocks.is_empty() {
                        has_images = true;
                        crate::routes::inject_attachments_into_session(
                            &state.kernel,
                            agent_id,
                            image_blocks,
                        );
                    }
                }
            }

            // Warn if the model doesn't support vision but images were attached
            if has_images {
                let model_name = state
                    .kernel
                    .agent_registry()
                    .get(agent_id)
                    .map(|e| e.manifest.model.model.clone())
                    .unwrap_or_default();
                let supports_vision = state
                    .kernel
                    .model_catalog_ref()
                    .read()
                    .ok()
                    .and_then(|cat| cat.find_model(&model_name).map(|m| m.supports_vision))
                    .unwrap_or(false);
                if !supports_vision {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "command_result",
                            "message": format!(
                                "**Vision not supported** — the current model `{}` cannot analyze images. \
                                 Switch to a vision-capable model (e.g. `gemini-2.5-flash`, `claude-sonnet-4-20250514`, `gpt-4o`) \
                                 with `/model <name>` for image analysis.",
                                model_name
                            ),
                        }),
                    )
                    .await;
                }
            }

            // Send typing lifecycle: start
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "typing",
                    "state": "start",
                }),
            )
            .await;

            // Send message to agent with streaming
            let kernel_handle: Arc<dyn KernelHandle> =
                state.kernel.clone() as Arc<dyn KernelHandle>;
            let sender_ctx = SenderContext {
                channel: "webui".to_string(),
                user_id: client_ip.to_string(),
                display_name: "Web UI".to_string(),
                is_group: false,
                was_mentioned: false,
                thread_id: None,
                account_id: None,
                ..Default::default()
            };
            match state
                .kernel
                .send_message_streaming_with_sender_context_routing_and_thinking(
                    agent_id,
                    &content,
                    Some(kernel_handle),
                    &sender_ctx,
                    thinking_override,
                )
                .await
            {
                Ok((mut rx, handle)) => {
                    // Forward stream events to WebSocket with debouncing
                    let sender_stream = Arc::clone(sender);
                    let verbose_clone = Arc::clone(verbose);
                    let rl = state.kernel.config_ref();
                    let debounce_chars = rl.rate_limit.ws_debounce_chars;
                    let debounce_ms = rl.rate_limit.ws_debounce_ms;
                    let show_thinking_stream = show_thinking;
                    // Set by the stream forwarder when it maps the runtime's
                    // `response_complete` phase to an early `typing:stop`.
                    // The post-handle branch reads it to avoid sending a
                    // duplicate `typing:stop` (an idempotent no-op on the
                    // client, but a wasted WS frame + re-render).
                    let early_stop_sent = Arc::new(AtomicBool::new(false));
                    let early_stop_sent_stream = Arc::clone(&early_stop_sent);
                    let stream_task = tokio::spawn(async move {
                        let mut text_buffer = String::new();
                        let far_future = tokio::time::Instant::now() + Duration::from_secs(86400);
                        let mut flush_deadline = far_future;

                        loop {
                            let sleep = tokio::time::sleep_until(flush_deadline);
                            tokio::pin!(sleep);

                            tokio::select! {
                                event = rx.recv() => {
                                    let vlevel = VerboseLevel::from_u8(
                                        verbose_clone.load(Ordering::Relaxed),
                                    );
                                    match event {
                                        None => {
                                            // Stream ended — flush remaining text
                                            let _ = flush_text_buffer(
                                                &sender_stream,
                                                &mut text_buffer,
                                            )
                                            .await;
                                            break;
                                        }
                                        Some(ev) => {
                                            if let StreamEvent::TextDelta { ref text } = ev {
                                                text_buffer.push_str(text);
                                                if text_buffer.len() >= debounce_chars {
                                                    let _ = flush_text_buffer(
                                                        &sender_stream,
                                                        &mut text_buffer,
                                                    )
                                                    .await;
                                                    flush_deadline = far_future;
                                                } else if flush_deadline >= far_future {
                                                    flush_deadline =
                                                        tokio::time::Instant::now()
                                                            + Duration::from_millis(debounce_ms);
                                                }
                                            } else {
                                                // Flush pending text before non-text events
                                                let _ = flush_text_buffer(
                                                    &sender_stream,
                                                    &mut text_buffer,
                                                )
                                                .await;
                                                flush_deadline = far_future;

                                                // Send typing indicator for tool events
                                                if let StreamEvent::ToolUseStart {
                                                    ref name, ..
                                                } = ev
                                                {
                                                    let _ = send_json(
                                                        &sender_stream,
                                                        &serde_json::json!({
                                                            "type": "typing",
                                                            "state": "tool",
                                                            "tool": name,
                                                        }),
                                                    )
                                                    .await;
                                                }

                                                // Map event to JSON with verbose filtering
                                                if let Some(json) = map_stream_event(
                                                    &ev,
                                                    vlevel,
                                                    show_thinking_stream,
                                                ) {
                                                    if let StreamEvent::PhaseChange { phase, .. } = &ev {
                                                        if phase == PHASE_RESPONSE_COMPLETE {
                                                            early_stop_sent_stream
                                                                .store(true, Ordering::Release);
                                                        }
                                                    }
                                                    if send_json(&sender_stream, &json)
                                                        .await
                                                        .is_err()
                                                    {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                _ = &mut sleep => {
                                    // Timer fired — flush text buffer
                                    let _ = flush_text_buffer(
                                        &sender_stream,
                                        &mut text_buffer,
                                    )
                                    .await;
                                    flush_deadline = far_future;
                                }
                            }
                        }
                    });

                    // Wait for the agent loop to complete
                    match handle.await {
                        Ok(Ok(result)) => {
                            // Wait for the stream forwarder to drain remaining
                            // events and flush its text buffer.  The agent loop
                            // has finished so its `tx` is dropped, which makes
                            // `rx.recv()` return `None` and the task exits
                            // naturally.  We give it up to 5 s before aborting
                            // as a safety net.
                            match tokio::time::timeout(Duration::from_secs(30), stream_task).await {
                                Ok(Ok(())) => {}
                                Ok(Err(join_err)) => {
                                    warn!(error = %join_err, "stream forwarder join failed");
                                }
                                Err(_) => {
                                    warn!("stream forwarder did not finish within 30 s — aborting before sending response");
                                }
                            }

                            // Send typing lifecycle: stop (skipped if the
                            // stream forwarder already emitted it via the
                            // `response_complete` phase).
                            if !early_stop_sent.load(Ordering::Acquire) {
                                let _ = send_json(
                                    sender,
                                    &serde_json::json!({
                                        "type": "typing",
                                        "state": "stop",
                                    }),
                                )
                                .await;
                            }

                            // NO_REPLY: agent intentionally chose not to reply
                            if result.silent {
                                let _ = send_json(
                                    sender,
                                    &serde_json::json!({
                                        "type": "silent_complete",
                                        "input_tokens": result.total_usage.input_tokens,
                                        "output_tokens": result.total_usage.output_tokens,
                                    }),
                                )
                                .await;
                                return;
                            }

                            // Extract reasoning trace (optional) and strip
                            // <think>...</think> blocks from model output
                            // (e.g. MiniMax, DeepSeek reasoning tokens).
                            let thinking_trace = if show_thinking {
                                extract_think_content(&result.response)
                            } else {
                                None
                            };
                            let cleaned_response = strip_think_tags(&result.response);

                            // Guard: ensure we never send an empty response
                            let content = if cleaned_response.trim().is_empty() {
                                format!(
                                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                                    result.total_usage.input_tokens,
                                    result.total_usage.output_tokens,
                                    result.iterations,
                                )
                            } else {
                                cleaned_response
                            };

                            // Estimate context pressure from last call
                            let per_call = if result.iterations > 0 {
                                result.total_usage.input_tokens / result.iterations as u64
                            } else {
                                result.total_usage.input_tokens
                            };
                            let ctx_pct = (per_call as f64 / 200_000.0 * 100.0).min(100.0);
                            let pressure = if ctx_pct > 85.0 {
                                "critical"
                            } else if ctx_pct > 70.0 {
                                "high"
                            } else if ctx_pct > 50.0 {
                                "medium"
                            } else {
                                "low"
                            };

                            let mut resp_json = serde_json::json!({
                                "type": "response",
                                "content": content,
                                "input_tokens": result.total_usage.input_tokens,
                                "output_tokens": result.total_usage.output_tokens,
                                "iterations": result.iterations,
                                "cost_usd": result.cost_usd,
                                "context_pressure": pressure,
                            });
                            if !result.memories_saved.is_empty() {
                                resp_json["memories_saved"] =
                                    serde_json::json!(result.memories_saved);
                            }
                            if !result.memories_used.is_empty() {
                                resp_json["memories_used"] =
                                    serde_json::json!(result.memories_used);
                            }
                            if !result.memory_conflicts.is_empty() {
                                resp_json["memory_conflicts"] =
                                    serde_json::json!(result.memory_conflicts);
                            }
                            if let Some(ref t) = thinking_trace {
                                resp_json["thinking"] = serde_json::json!(t);
                            }
                            let _ = send_json(sender, &resp_json).await;
                        }
                        Ok(Err(e)) => {
                            // Let the stream forwarder drain before
                            // sending the error so partial content is
                            // still delivered to the client.
                            let _ = tokio::time::timeout(Duration::from_secs(2), stream_task).await;
                            warn!("Agent message failed: {e}");
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing", "state": "stop",
                                }),
                            )
                            .await;
                            let user_msg = classify_streaming_error(&e);
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "error",
                                    "content": user_msg,
                                }),
                            )
                            .await;
                        }
                        Err(e) => {
                            // Let the stream forwarder drain before
                            // sending the error so partial content is
                            // still delivered to the client.
                            let _ = tokio::time::timeout(Duration::from_secs(2), stream_task).await;
                            warn!("Agent task panicked: {e}");
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing", "state": "stop",
                                }),
                            )
                            .await;
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "error",
                                    "content": "Internal error occurred",
                                }),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    warn!("Streaming setup failed: {e}");
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "typing", "state": "stop",
                        }),
                    )
                    .await;
                    let user_msg = classify_streaming_error(&e);
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": user_msg,
                        }),
                    )
                    .await;
                }
            }
        }
        "command" => {
            let cmd = parsed["command"].as_str().unwrap_or("");
            let args = parsed["args"].as_str().unwrap_or("");
            let response = handle_command(sender, state, agent_id, cmd, args, verbose).await;
            let _ = send_json(sender, &response).await;
        }
        "ping" => {
            let _ = send_json(sender, &serde_json::json!({"type": "pong"})).await;
        }
        other => {
            warn!(msg_type = other, "Unknown WebSocket message type");
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "error",
                    "content": format!("Unknown message type: {other}"),
                }),
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Command Handler
// ---------------------------------------------------------------------------

/// Handle a WS command and return the response JSON.
async fn handle_command(
    _sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    cmd: &str,
    args: &str,
    verbose: &Arc<AtomicU8>,
) -> serde_json::Value {
    match cmd {
        "new" | "reset" => match state.kernel.reset_session(agent_id) {
            Ok(()) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "Session reset. Chat history cleared."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Reset failed: {e}")}),
        },
        "reboot" => match state.kernel.reboot_session(agent_id) {
            Ok(()) => {
                serde_json::json!({"type": "command_result", "command": "reboot", "message": "Session rebooted. Context cleared."})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Reboot failed: {e}")})
            }
        },
        "compact" => match state.kernel.compact_agent_session(agent_id).await {
            Ok(msg) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Compaction failed: {e}")})
            }
        },
        "stop" => match state.kernel.stop_agent_run(agent_id) {
            Ok(true) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "Run cancelled."})
            }
            Ok(false) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "No active run to cancel."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Stop failed: {e}")}),
        },
        "model" => {
            if args.is_empty() {
                if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
                    serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Current model: {} (provider: {})", entry.manifest.model.model, entry.manifest.model.provider)})
                } else {
                    serde_json::json!({"type": "error", "content": "Agent not found"})
                }
            } else {
                match state.kernel.set_agent_model(agent_id, args, None) {
                    Ok(()) => {
                        if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
                            let model = &entry.manifest.model.model;
                            let provider = &entry.manifest.model.provider;
                            serde_json::json!({
                                "type": "command_result",
                                "command": cmd,
                                "message": format!("Model switched to: {model} (provider: {provider})"),
                                "model": model,
                                "provider": provider
                            })
                        } else {
                            serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Model switched to: {args}")})
                        }
                    }
                    Err(e) => {
                        serde_json::json!({"type": "error", "content": format!("Model switch failed: {e}")})
                    }
                }
            }
        }
        "usage" => match state.kernel.session_usage_cost(agent_id) {
            Ok((input, output, cost)) => {
                let mut msg = format!(
                    "Session usage: ~{input} in / ~{output} out (~{} total)",
                    input + output
                );
                if cost > 0.0 {
                    msg.push_str(&format!(" | ${cost:.4}"));
                }
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Usage query failed: {e}")})
            }
        },
        "context" => match state.kernel.context_report(agent_id) {
            Ok(report) => {
                let formatted = librefang_runtime::compactor::format_context_report(&report);
                serde_json::json!({
                    "type": "command_result",
                    "command": cmd,
                    "message": formatted,
                    "context_pressure": format!("{:?}", report.pressure).to_lowercase(),
                })
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Context report failed: {e}")})
            }
        },
        "verbose" => {
            let new_level = match args.to_lowercase().as_str() {
                "off" => VerboseLevel::Off,
                "on" => VerboseLevel::On,
                "full" => VerboseLevel::Full,
                _ => {
                    // Cycle to next level
                    let current = VerboseLevel::from_u8(verbose.load(Ordering::Relaxed));
                    current.next()
                }
            };
            verbose.store(new_level as u8, Ordering::Relaxed);
            serde_json::json!({
                "type": "command_result",
                "command": cmd,
                "message": format!("Verbose level: **{}**", new_level.label()),
            })
        }
        "queue" => {
            let is_running = state.kernel.running_tasks_ref().contains_key(&agent_id);
            let msg = if is_running {
                "Agent is processing a request..."
            } else {
                "Agent is idle."
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "budget" => {
            let budget = state.kernel.budget_config();
            let status = state.kernel.metering_ref().budget_status(&budget);
            let fmt = |v: f64| -> String {
                if v > 0.0 {
                    format!("${v:.2}")
                } else {
                    "unlimited".to_string()
                }
            };
            let msg = format!(
                "Hourly: ${:.4} / {}  |  Daily: ${:.4} / {}  |  Monthly: ${:.4} / {}",
                status.hourly_spend,
                fmt(status.hourly_limit),
                status.daily_spend,
                fmt(status.daily_limit),
                status.monthly_spend,
                fmt(status.monthly_limit),
            );
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "peers" => {
            let msg = if !state.kernel.config_ref().network_enabled {
                "OFP network disabled.".to_string()
            } else {
                match state.kernel.peer_registry_ref() {
                    Some(registry) => {
                        let peers = registry.all_peers();
                        if peers.is_empty() {
                            "No peers connected.".to_string()
                        } else {
                            peers
                                .iter()
                                .map(|p| format!("{} — {} ({:?})", p.node_id, p.address, p.state))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    None => "OFP peer node not started.".to_string(),
                }
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "a2a" => {
            let agents = state
                .kernel
                .a2a_agents()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let msg = if agents.is_empty() {
                "No external A2A agents discovered.".to_string()
            } else {
                agents
                    .iter()
                    .map(|(url, card)| format!("{} — {}", card.name, url))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        _ => serde_json::json!({"type": "error", "content": format!("Unknown command: {cmd}")}),
    }
}

// ---------------------------------------------------------------------------
// Stream Event Mapping (verbose-aware)
// ---------------------------------------------------------------------------

/// Map a stream event to a JSON value, applying verbose filtering.
fn map_stream_event(
    event: &StreamEvent,
    verbose: VerboseLevel,
    show_thinking: bool,
) -> Option<serde_json::Value> {
    match event {
        StreamEvent::TextDelta { .. } => None, // Handled by debounce buffer
        StreamEvent::ThinkingDelta { text } => {
            if show_thinking {
                Some(serde_json::json!({
                    "type": "thinking_delta",
                    "content": text,
                }))
            } else {
                None
            }
        }
        StreamEvent::ToolUseStart { id, name } => Some(serde_json::json!({
            "type": "tool_start",
            "id": id,
            "tool": name,
        })),
        StreamEvent::ToolUseEnd { name, input, .. } if name == "canvas_present" => {
            let html = input.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Canvas");
            Some(serde_json::json!({
                "type": "canvas",
                "canvas_id": uuid::Uuid::new_v4().to_string(),
                "html": html,
                "title": title,
            }))
        }
        StreamEvent::ToolUseEnd { id, name, input } => match verbose {
            VerboseLevel::Off => None,
            VerboseLevel::On => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(100)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
            VerboseLevel::Full => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
        },
        StreamEvent::ToolExecutionResult {
            name,
            result_preview,
            is_error,
        } => match verbose {
            VerboseLevel::Off => Some(serde_json::json!({
                "type": "tool_result",
                "tool": name,
                "is_error": is_error,
            })),
            VerboseLevel::On => {
                let truncated: String = result_preview.chars().take(200).collect();
                Some(serde_json::json!({
                    "type": "tool_result",
                    "tool": name,
                    "result": truncated,
                    "is_error": is_error,
                }))
            }
            VerboseLevel::Full => Some(serde_json::json!({
                "type": "tool_result",
                "tool": name,
                "result": result_preview,
                "is_error": is_error,
            })),
        },
        StreamEvent::PhaseChange { phase, detail } => {
            // Special case: `response_complete` fires when the LLM has emitted
            // the final text but the agent loop is still doing post-processing
            // (session save, proactive memory). Map to an early `typing:stop`
            // so the dashboard can unblock the input while the later
            // `response` event (with tokens/cost) is still in flight.
            if phase == PHASE_RESPONSE_COMPLETE {
                Some(serde_json::json!({
                    "type": "typing",
                    "state": "stop",
                }))
            } else {
                Some(serde_json::json!({
                    "type": "phase",
                    "phase": phase,
                    "detail": detail,
                }))
            }
        }
        _ => None, // Skip ToolInputDelta, ContentComplete
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flush accumulated text buffer as a single text_delta event.
async fn flush_text_buffer(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    buffer: &mut String,
) -> Result<(), axum::Error> {
    if buffer.is_empty() {
        return Ok(());
    }
    let result = send_json(
        sender,
        &serde_json::json!({
            "type": "text_delta",
            "content": buffer.as_str(),
        }),
    )
    .await;
    buffer.clear();
    result
}

/// Helper to send a JSON value over WebSocket.
pub async fn send_json(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    value: &serde_json::Value,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut s = sender.lock().await;
    s.send(Message::Text(text.into()))
        .await
        .map_err(axum::Error::new)
}

/// Sanitize inbound user input.
///
/// - If content looks like a JSON envelope, extract the `content` field.
/// - Strip control characters (except \n, \t).
/// - Trim excessive whitespace.
fn sanitize_user_input(content: &str) -> String {
    // If content looks like a JSON envelope, try to extract the content field
    if content.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(inner) = val.get("content").and_then(|v| v.as_str()) {
                return sanitize_text(inner);
            }
        }
    }
    sanitize_text(content)
}

/// Strip control characters and normalize whitespace.
fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Classify a streaming/setup error into a user-friendly message.
///
/// Uses the proper LLM error classifier from `librefang_runtime::llm_errors`
/// for comprehensive 20-provider coverage with actionable advice.
fn classify_streaming_error(err: &librefang_kernel::error::KernelError) -> String {
    let inner = format!("{err}");

    // Check for agent-specific errors first (not LLM errors)
    if inner.contains("Agent not found") {
        return "Agent not found. It may have been stopped or deleted.".to_string();
    }
    if inner.contains("quota") || inner.contains("Quota") {
        return "Token quota exceeded. Try /compact or /new to free up space.".to_string();
    }

    // Use the LLM error classifier for everything else
    let status = extract_status_code(&inner);
    let classified = llm_errors::classify_error(&inner, status);

    // Build a user-facing message. The classified.sanitized_message now
    // includes a redacted excerpt of the raw error (issue #493 fix), so we
    // use it as the base and only override for cases that need extra context.
    match classified.category {
        llm_errors::LlmErrorCategory::ContextOverflow => {
            "Context is full. Try /compact or /new.".to_string()
        }
        llm_errors::LlmErrorCategory::RateLimit => {
            if let Some(delay_ms) = classified.suggested_delay_ms {
                let secs = (delay_ms / 1000).max(1);
                format!("Rate limited. Wait ~{secs}s and try again.")
            } else {
                "Rate limited. Wait a moment and try again.".to_string()
            }
        }
        llm_errors::LlmErrorCategory::Billing => {
            format!("Billing issue. {}", classified.sanitized_message)
        }
        llm_errors::LlmErrorCategory::Auth => {
            // Show the actual error detail so users can diagnose (issue #493).
            // The sanitized_message already redacts secrets.
            classified.sanitized_message.clone()
        }
        llm_errors::LlmErrorCategory::ModelNotFound => {
            if inner.contains("localhost:11434") || inner.contains("ollama") {
                "Model not found on Ollama. Run `ollama pull <model>` first. Use /model to see options.".to_string()
            } else {
                format!(
                    "{}. Use /model to see options.",
                    classified.sanitized_message
                )
            }
        }
        llm_errors::LlmErrorCategory::Format => {
            // Claude Code CLI errors have actionable messages — pass them through
            if inner.contains("Claude Code CLI") || inner.contains("claude auth") {
                classified.raw_message.clone()
            } else {
                classified.sanitized_message.clone()
            }
        }
        _ => classified.sanitized_message,
    }
}

/// Try to extract an HTTP status code from an error string.
fn extract_status_code(s: &str) -> Option<u16> {
    // "API error (NNN):" — the format produced by LlmError::Api Display impl
    if let Some(idx) = s.find("API error (") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse::<u16>() {
            return Some(code);
        }
    }
    // "status: NNN"
    if let Some(idx) = s.find("status: ") {
        let after = &s[idx + 8..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "HTTP NNN"
    if let Some(idx) = s.find("HTTP ") {
        let after = &s[idx + 5..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "StatusCode(NNN)"
    if let Some(idx) = s.find("StatusCode(") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    None
}

/// Strip `<think>...</think>` blocks from model output.
///
/// Some models (MiniMax, DeepSeek, etc.) wrap their reasoning in `<think>` tags.
/// These are internal chain-of-thought and shouldn't be shown to the user.
pub fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[(start + end + 8)..]; // 8 = "</think>".len()
        } else {
            // Unclosed <think> tag — strip to end
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

/// Extract the concatenated content of all `<think>...</think>` blocks from
/// a model response. Returns `None` when no thinking blocks are present.
///
/// Paired with [`strip_think_tags`] so callers can surface the reasoning to
/// the UI separately from the final answer.
pub fn extract_think_content(text: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        let after_open = &remaining[start + 7..]; // 7 = "<think>".len()
        if let Some(end) = after_open.find("</think>") {
            let thought = after_open[..end].trim();
            if !thought.is_empty() {
                parts.push(thought);
            }
            remaining = &after_open[end + 8..]; // 8 = "</think>".len()
        } else {
            // Unclosed — take everything after the opener as the thought.
            let thought = after_open.trim();
            if !thought.is_empty() {
                parts.push(thought);
            }
            break;
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_module_loads() {
        // Verify module compiles and loads correctly
        let _ = VerboseLevel::Off;
    }

    #[test]
    fn test_extract_think_content_none_when_absent() {
        assert!(extract_think_content("plain answer").is_none());
        assert!(extract_think_content("").is_none());
    }

    #[test]
    fn test_extract_think_content_single_block() {
        let raw = "before<think>reasoning here</think>after";
        assert_eq!(
            extract_think_content(raw).as_deref(),
            Some("reasoning here")
        );
    }

    #[test]
    fn test_extract_think_content_multiple_blocks() {
        let raw = "<think>step one</think>mid<think>step two</think>end";
        assert_eq!(
            extract_think_content(raw).as_deref(),
            Some("step one\n\nstep two")
        );
    }

    #[test]
    fn test_extract_think_content_unclosed_tag() {
        let raw = "intro<think>tail thoughts never closed";
        assert_eq!(
            extract_think_content(raw).as_deref(),
            Some("tail thoughts never closed")
        );
    }

    #[test]
    fn test_extract_and_strip_are_complementary() {
        let raw = "hello <think>secret</think> world";
        assert_eq!(strip_think_tags(raw), "hello  world");
        assert_eq!(extract_think_content(raw).as_deref(), Some("secret"));
    }

    #[test]
    fn test_verbose_level_cycle() {
        assert_eq!(VerboseLevel::Off.next(), VerboseLevel::On);
        assert_eq!(VerboseLevel::On.next(), VerboseLevel::Full);
        assert_eq!(VerboseLevel::Full.next(), VerboseLevel::Off);
    }

    #[test]
    fn test_verbose_level_roundtrip() {
        for v in [VerboseLevel::Off, VerboseLevel::On, VerboseLevel::Full] {
            assert_eq!(VerboseLevel::from_u8(v as u8), v);
        }
    }

    #[test]
    fn test_verbose_level_labels() {
        assert_eq!(VerboseLevel::Off.label(), "off");
        assert_eq!(VerboseLevel::On.label(), "on");
        assert_eq!(VerboseLevel::Full.label(), "full");
    }

    #[test]
    fn test_sanitize_user_input_plain_text() {
        assert_eq!(sanitize_user_input("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_user_input_strips_control_chars() {
        assert_eq!(sanitize_user_input("hello\x00world"), "helloworld");
        // Newlines and tabs are preserved
        assert_eq!(sanitize_user_input("hello\nworld"), "hello\nworld");
        assert_eq!(sanitize_user_input("hello\tworld"), "hello\tworld");
    }

    #[test]
    fn test_sanitize_user_input_extracts_json_content() {
        let envelope = r#"{"type":"message","content":"actual message"}"#;
        assert_eq!(sanitize_user_input(envelope), "actual message");
    }

    #[test]
    fn test_sanitize_user_input_leaves_non_envelope_json() {
        // JSON that doesn't have a content field is left as-is (after control-char stripping)
        let json = r#"{"key":"value"}"#;
        assert_eq!(sanitize_user_input(json), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_extract_status_code() {
        assert_eq!(extract_status_code("status: 429, body: ..."), Some(429));
        assert_eq!(
            extract_status_code("HTTP 503 Service Unavailable"),
            Some(503)
        );
        assert_eq!(extract_status_code("StatusCode(401)"), Some(401));
        assert_eq!(extract_status_code("some random error"), None);
        // LlmError::Api Display format (issue #493 fix)
        assert_eq!(
            extract_status_code("LLM driver error: API error (403): quota exceeded"),
            Some(403)
        );
        assert_eq!(
            extract_status_code("API error (401): invalid api key"),
            Some(401)
        );
    }

    #[test]
    fn test_ws_query_param_decodes_percent_encoded_values() {
        let uri: Uri = "/api/terminal/ws?token=abc%2Bdef%2Fghi%3D&cols=120"
            .parse()
            .unwrap();
        assert_eq!(
            ws_query_param(&uri, "token").as_deref(),
            Some("abc+def/ghi=")
        );
        assert_eq!(ws_query_param(&uri, "cols").as_deref(), Some("120"));
    }

    #[test]
    fn test_ws_auth_token_prefers_bearer_header() {
        let uri: Uri = "/api/terminal/ws?token=query-token".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer header-token".parse().unwrap());
        assert_eq!(
            ws_auth_token(&headers, &uri).as_deref(),
            Some("header-token")
        );
    }

    #[test]
    fn test_sanitize_trims_whitespace() {
        assert_eq!(sanitize_user_input("  hello  "), "hello");
    }

    #[test]
    fn test_strip_think_tags() {
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>The answer is 42."),
            "The answer is 42."
        );
        assert_eq!(
            strip_think_tags("Hello <think>\nsome thinking\n</think> world"),
            "Hello  world"
        );
        assert_eq!(strip_think_tags("No thinking here"), "No thinking here");
        assert_eq!(
            strip_think_tags(
                "<think>all thinking
</think>"
            ),
            ""
        );
    }

    #[test]
    fn validate_ws_origin_missing_origin_allowed() {
        let headers = HeaderMap::new();
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_localhost_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:4545".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_127_0_0_1_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://127.0.0.1:4545".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_ipv6_loopback_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://[::1]:4545".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_wrong_host_port_mismatch_rejected() {
        // Port mismatch: origin port 9999 != listen_port 4545
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.com:9999".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_lan_ip_same_port_rejected() {
        // LAN IP with matching port should be rejected without explicit allowed_origins.
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://192.168.1.5:4545".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_arbitrary_host_same_port_rejected() {
        // Any hostname with matching port is rejected without explicit allowed_origins.
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://myserver.local:8080".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(8080), &[], false);
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_lan_ip_allowed_via_extra_origins() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://192.168.1.5:4545".parse().unwrap());
        let result = validate_ws_origin(
            &headers,
            Some(4545),
            &["http://192.168.1.5:4545".to_string()],
            false,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_port_mismatch_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://localhost:9999".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &[], false);
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_https_with_default_port_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://localhost".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(443), &[], false);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_extra_origins_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://my.domain.com:8080".parse().unwrap());
        let result = validate_ws_origin(
            &headers,
            Some(4545),
            &["http://my.domain.com:8080".to_string()],
            false,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_extra_origins_scheme_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://my.domain.com".parse().unwrap());
        let result = validate_ws_origin(
            &headers,
            Some(4545),
            &["https://my.domain.com".to_string()],
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_wildcard_allows_any_http_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://evil.example:9999".parse().unwrap());
        // Wildcard + allow_remote=false should be rejected
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], false);
        assert!(result.is_err());
        // Wildcard + allow_remote=true should be allowed
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], true);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_wildcard_allows_any_https_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://evil.example".parse().unwrap());
        // Wildcard + allow_remote=false should be rejected
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], false);
        assert!(result.is_err());
        // Wildcard + allow_remote=true should be allowed
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], true);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_ws_origin_wildcard_rejects_non_http_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "file://evil.example".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], true);
        assert!(result.is_err());
    }

    #[test]
    fn validate_ws_origin_wildcard_rejects_malformed_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "not-a-url".parse().unwrap());
        let result = validate_ws_origin(&headers, Some(4545), &["*".to_string()], true);
        assert!(result.is_err());
    }

    #[test]
    fn detect_locality_direct_loopback() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let headers = HeaderMap::new();
        let locality = detect_connection_locality(&addr, &headers);
        assert!(locality.is_local());
        assert!(!locality.is_proxied);
        assert!(locality.forwarded_ip.is_none());
    }

    #[test]
    fn detect_locality_loopback_with_xff() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());
        let locality = detect_connection_locality(&addr, &headers);
        assert!(!locality.is_local());
        assert!(locality.is_proxied);
    }

    #[test]
    fn detect_locality_direct_remote() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "8.8.8.8:12345".parse().unwrap();
        let headers = HeaderMap::new();
        let locality = detect_connection_locality(&addr, &headers);
        assert!(!locality.is_local());
        assert!(!locality.is_proxied);
    }

    #[test]
    fn detect_locality_ipv6_loopback() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "[::1]:12345".parse().unwrap();
        let headers = HeaderMap::new();
        let locality = detect_connection_locality(&addr, &headers);
        assert!(locality.is_local());
    }

    #[test]
    fn detect_locality_x_real_ip_sets_forwarded() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "10.0.0.1:12345".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "1.2.3.4".parse().unwrap());
        let locality = detect_connection_locality(&addr, &headers);
        assert!(locality.is_proxied);
        assert_eq!(locality.forwarded_ip.unwrap().to_string(), "1.2.3.4");
    }

    #[test]
    fn detect_locality_xff_first_ip_parsed() {
        use std::net::SocketAddr;
        let addr: SocketAddr = "10.0.0.1:12345".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "1.1.1.1, 8.8.8.8, 9.9.9.9".parse().unwrap(),
        );
        let locality = detect_connection_locality(&addr, &headers);
        assert_eq!(locality.forwarded_ip.unwrap().to_string(), "1.1.1.1");
    }
}
