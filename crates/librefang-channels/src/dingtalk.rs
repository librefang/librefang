//! DingTalk Robot channel adapter.
//!
//! Supports two modes for receiving messages:
//! - **Webhook mode**: Listens on a local HTTP port for DingTalk callbacks.
//!   Requires a public-facing URL (nginx, tunnel, etc.).
//! - **Stream mode** (default): Opens a WebSocket long-connection to DingTalk's
//!   gateway. No public IP or port required — ideal for NAT/internal deployments.
//!
//! Outbound messages are posted via the Robot Send API (webhook mode) or the
//! DingTalk Open API reply endpoint (stream mode).

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_MESSAGE_LEN: usize = 20000;

/// Default Robot Send API base URL for webhook mode outbound messages.
const DINGTALK_API_BASE: &str = "https://oapi.dingtalk.com";

/// Returns the default DingTalk API base URL. Used to initialise `DingTalkAdapter::api_base`.
#[inline]
fn default_dingtalk_api_base() -> String {
    DINGTALK_API_BASE.to_string()
}

/// Stream gateway registration endpoint.
const DINGTALK_GATEWAY_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";

/// Initial back-off for WebSocket reconnection.
const WS_INITIAL_BACKOFF: Duration = Duration::from_secs(3);

/// Maximum back-off between WebSocket reconnection attempts.
const WS_MAX_BACKOFF: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Receive mode
// ---------------------------------------------------------------------------

/// Connection mode for the DingTalk adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DingTalkMode {
    /// HTTP webhook callback mode — requires a public-facing port.
    Webhook,
    /// WebSocket stream mode — no public IP needed.
    Stream,
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// DingTalk Robot channel adapter.
///
/// Supports both webhook and stream (WebSocket) modes for receiving messages.
pub struct DingTalkAdapter {
    mode: DingTalkMode,
    // -- Webhook mode fields --
    /// SECURITY: Robot access token is zeroized on drop.
    access_token: Zeroizing<String>,
    /// SECURITY: Signing secret for HMAC-SHA256 verification.
    secret: Zeroizing<String>,
    // -- Stream mode fields --
    /// SECURITY: Client ID (AppKey) for stream mode.
    client_id: Zeroizing<String>,
    /// SECURITY: Client Secret (AppSecret) for stream mode.
    client_secret: Zeroizing<String>,
    /// HTTP client for outbound requests.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Base URL for the DingTalk Robot Send API. Defaults to `https://oapi.dingtalk.com`.
    /// Overridable in tests via `with_api_base()` to point at a wiremock server.
    api_base: String,
}

impl DingTalkAdapter {
    /// Create a new DingTalk Robot adapter in **webhook** mode.
    ///
    /// # Arguments
    /// * `access_token` - Robot access token from DingTalk.
    /// * `secret` - Signing secret for request verification.
    /// * `webhook_port` - Local port to listen for DingTalk callbacks.
    pub fn new(access_token: String, secret: String, _webhook_port: u16) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            mode: DingTalkMode::Webhook,
            access_token: Zeroizing::new(access_token),
            secret: Zeroizing::new(secret),
            client_id: Zeroizing::new(String::new()),
            client_secret: Zeroizing::new(String::new()),
            client: crate::http_client::new_client(),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            api_base: default_dingtalk_api_base(),
        }
    }

    /// Create a new DingTalk Robot adapter in **stream** mode.
    ///
    /// # Arguments
    /// * `client_id` - DingTalk App Key (Client ID).
    /// * `client_secret` - DingTalk App Secret (Client Secret).
    pub fn new_stream(client_id: String, client_secret: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            mode: DingTalkMode::Stream,
            access_token: Zeroizing::new(String::new()),
            secret: Zeroizing::new(String::new()),
            client_id: Zeroizing::new(client_id),
            client_secret: Zeroizing::new(client_secret),
            client: crate::http_client::new_client(),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            api_base: default_dingtalk_api_base(),
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the DingTalk API base URL. Intended for tests that point the
    /// adapter at a wiremock server instead of `https://oapi.dingtalk.com`.
    #[cfg(test)]
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    // -----------------------------------------------------------------------
    // Webhook helpers
    // -----------------------------------------------------------------------

    /// Compute the HMAC-SHA256 signature for a DingTalk request.
    ///
    /// DingTalk signature = Base64(HMAC-SHA256(secret, timestamp + "\n" + secret + body_bytes))
    ///
    /// The body bytes are included to prevent HMAC-replay attacks where a valid
    /// captured signature is reused with a different payload.
    fn compute_signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let prefix = format!("{}\n{}", timestamp, secret);
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(prefix.as_bytes());
        mac.update(body);
        let result = mac.finalize();
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(result.into_bytes())
    }

    /// Verify an incoming DingTalk callback signature (constant-time comparison).
    fn verify_signature(secret: &str, timestamp: i64, body: &[u8], signature: &str) -> bool {
        let expected = Self::compute_signature(secret, timestamp, body);
        // Constant-time comparison
        if expected.len() != signature.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in expected.bytes().zip(signature.bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }

    /// Build the signed send URL with access_token, timestamp, and signature.
    ///
    /// Outbound URL signing uses the legacy DingTalk spec (no body bytes in
    /// the HMAC) because the signature is a URL query parameter, not an
    /// inbound-request authenticator.
    fn build_send_url(&self) -> String {
        let timestamp = Utc::now().timestamp_millis();
        // Outbound signing: body is empty (signature is for the URL parameter).
        let sign = Self::compute_signature(&self.secret, timestamp, b"");
        let encoded_sign = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("sign", &sign)
            .finish();
        format!(
            "{}/robot/send?access_token={}&timestamp={}&{}",
            self.api_base,
            self.access_token.as_str(),
            timestamp,
            encoded_sign
        )
    }

    /// Parse a DingTalk webhook JSON body into extracted fields.
    ///
    /// Returns `(text, sender_id, sender_nick, conversation_id, is_group)`.
    fn parse_callback(body: &serde_json::Value) -> Option<(String, String, String, String, bool)> {
        let msg_type = body["msgtype"].as_str()?;
        let text = match msg_type {
            "text" => body["text"]["content"].as_str()?.trim().to_string(),
            _ => return None,
        };
        if text.is_empty() {
            return None;
        }

        let sender_id = body["senderId"].as_str().unwrap_or("unknown").to_string();
        let sender_nick = body["senderNick"].as_str().unwrap_or("Unknown").to_string();
        let conversation_id = body["conversationId"].as_str().unwrap_or("").to_string();
        let is_group = body["conversationType"].as_str() == Some("2");

        Some((text, sender_id, sender_nick, conversation_id, is_group))
    }

    /// Extract the session webhook reply URL from a ChannelUser (stream mode).
    fn stream_reply_url(user: &ChannelUser) -> Option<String> {
        user.librefang_user
            .as_ref()
            .and_then(|v| if v.is_empty() { None } else { Some(v.clone()) })
    }

    // -----------------------------------------------------------------------
    // Stream mode helpers
    // -----------------------------------------------------------------------

    /// Register a WebSocket connection with the DingTalk gateway.
    /// Returns `(ws_endpoint, ticket)` on success.
    async fn register_stream_connection(
        client: &reqwest::Client,
        client_id: &str,
        client_secret: &str,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "subscriptions": [
                { "type": "CALLBACK", "topic": "/v1.0/im/bot/messages/get" }
            ],
            "ua": "librefang"
        });

        let resp = client
            .post(DINGTALK_GATEWAY_URL)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(
                format!("DingTalk gateway registration failed ({status}): {err_body}").into(),
            );
        }

        let result: serde_json::Value = resp.json().await?;
        debug!("DingTalk stream: gateway registration response: {result}");
        let endpoint = result["endpoint"]
            .as_str()
            .ok_or("DingTalk gateway: missing endpoint in response")?
            .to_string();
        let ticket = result["ticket"]
            .as_str()
            .ok_or("DingTalk gateway: missing ticket in response")?
            .to_string();

        Ok((endpoint, ticket))
    }

    /// Parse a stream event callback payload into a `ChannelMessage`.
    ///
    /// Stream events arrive as `{ "type": "CALLBACK", "headers": {...}, "data": "<json-string>" }`.
    /// The `data` field contains the actual message payload as a JSON string.
    fn parse_stream_event(data: &serde_json::Value) -> Option<ChannelMessage> {
        let data_str = data["data"].as_str()?;
        let payload: serde_json::Value = serde_json::from_str(data_str).ok()?;

        let msg_type = payload["msgtype"].as_str().unwrap_or("text");
        debug!("DingTalk stream: parsed callback payload: {payload}");
        let text = match msg_type {
            "text" => payload["text"]["content"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string(),
            _ => return None,
        };
        if text.is_empty() {
            return None;
        }

        let sender_id = payload["senderStaffId"]
            .as_str()
            .or_else(|| payload["senderId"].as_str())
            .unwrap_or("unknown")
            .to_string();
        let sender_nick = payload["senderNick"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string();
        let session_webhook = payload["sessionWebhook"].as_str().unwrap_or("").to_string();
        let session_webhook_expired_time = payload["sessionWebhookExpiredTime"]
            .as_i64()
            .unwrap_or_default();
        let conversation_id = payload["conversationId"].as_str().unwrap_or("").to_string();
        let is_group = payload["conversationType"].as_str() == Some("2");
        let was_mentioned = payload["isInAtList"].as_bool().unwrap_or_else(|| {
            payload["atUsers"]
                .as_array()
                .map(|users| !users.is_empty())
                .unwrap_or(false)
        });

        let content = if text.starts_with('/') {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let cmd = parts[0].trim_start_matches('/');
            let args: Vec<String> = parts
                .get(1)
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_default();
            ChannelContent::Command {
                name: cmd.to_string(),
                args,
            }
        } else {
            ChannelContent::Text(text)
        };

        Some(ChannelMessage {
            channel: ChannelType::Custom("dingtalk".to_string()),
            platform_message_id: payload["msgId"]
                .as_str()
                .or_else(|| data["headers"]["messageId"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("dt-{}", Utc::now().timestamp_millis())),
            sender: ChannelUser {
                platform_id: sender_id,
                display_name: sender_nick,
                // Store session webhook URL in librefang_user for reply routing.
                librefang_user: if session_webhook.is_empty() {
                    None
                } else {
                    Some(session_webhook)
                },
            },
            content,
            target_agent: None,
            timestamp: Utc::now(),
            is_group,
            thread_id: None,
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "conversation_id".to_string(),
                    serde_json::Value::String(conversation_id),
                );
                if session_webhook_expired_time > 0 {
                    m.insert(
                        "session_webhook_expired_time".to_string(),
                        serde_json::Value::Number(session_webhook_expired_time.into()),
                    );
                }
                if was_mentioned {
                    m.insert("was_mentioned".to_string(), serde_json::Value::Bool(true));
                }
                m
            },
        })
    }

    // -----------------------------------------------------------------------
    // Start methods
    // -----------------------------------------------------------------------

    /// Start the stream-based WebSocket listener.
    fn start_stream(&self, tx: mpsc::Sender<ChannelMessage>) {
        let client = self.client.clone();
        let client_id = self.client_id.clone();
        let client_secret = self.client_secret.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = Arc::new(self.account_id.clone());

        info!("DingTalk adapter starting in stream mode (WebSocket long-connection)");

        tokio::spawn(async move {
            let mut backoff = WS_INITIAL_BACKOFF;

            // Reconnect loop — re-registers and reconnects on disconnection.
            loop {
                // Check shutdown before attempting connection
                if *shutdown_rx.borrow() {
                    break;
                }

                // Step 1: Register with the gateway to get a WebSocket endpoint + ticket.
                let (endpoint, ticket) =
                    match Self::register_stream_connection(&client, &client_id, &client_secret)
                        .await
                    {
                        Ok(v) => v,
                        Err(e) => {
                            error!("DingTalk stream: gateway registration failed: {e}");
                            tokio::select! {
                                _ = tokio::time::sleep(backoff) => {
                                    backoff = (backoff * 2).min(WS_MAX_BACKOFF);
                                    continue;
                                }
                                _ = shutdown_rx.changed() => break,
                            }
                        }
                    };

                info!("DingTalk stream: registered, connecting to WebSocket");

                // Step 2: Build WebSocket URL with ticket as query param.
                // URL-encode the ticket — it may contain base64 chars like `+`, `=`.
                let encoded_ticket =
                    url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect::<String>();
                let ws_url = format!("{}?ticket={}", endpoint, encoded_ticket);

                let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        error!("DingTalk stream: WebSocket connection failed: {e}");
                        tokio::select! {
                            _ = tokio::time::sleep(backoff) => {
                                backoff = (backoff * 2).min(WS_MAX_BACKOFF);
                                continue;
                            }
                            _ = shutdown_rx.changed() => break,
                        }
                    }
                };

                info!("DingTalk stream: WebSocket connected");
                backoff = WS_INITIAL_BACKOFF;

                let (mut ws_write, mut ws_read) = ws_stream.split();

                // Step 3: Read messages from the WebSocket.
                loop {
                    tokio::select! {
                        msg = ws_read.next() => {
                            match msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    debug!("DingTalk stream: received text frame, len={}", text.len());

                                    match serde_json::from_str::<serde_json::Value>(&text) {
                                        Ok(frame) => {
                                            let frame_type = frame["type"].as_str().unwrap_or("");
                                            let topic = frame["headers"]["topic"].as_str().unwrap_or("");
                                            let msg_id = frame["headers"]["messageId"].as_str().unwrap_or("");
                                            debug!(frame_type, topic, msg_id, "DingTalk stream: received frame");

                                            match frame_type {
                                                "SYSTEM" => {
                                                    // System events: ping/pong heartbeat
                                                    let sys_topic = frame["headers"]["topic"].as_str().unwrap_or("");
                                                    if sys_topic == "ping" {
                                                        let sys_msg_id = frame["headers"]["messageId"]
                                                            .as_str()
                                                            .unwrap_or("");
                                                        let pong = serde_json::json!({
                                                            "code": 200,
                                                            "headers": {
                                                                "contentType": "application/json",
                                                                "messageId": sys_msg_id,
                                                            },
                                                            "message": "OK",
                                                            "data": frame["data"],
                                                        });
                                                        if let Err(e) = ws_write
                                                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                                                pong.to_string().into(),
                                                            ))
                                                            .await
                                                        {
                                                            warn!("DingTalk stream: failed to send pong: {e}");
                                                            break;
                                                        }
                                                        debug!("DingTalk stream: pong sent");
                                                    }
                                                }
                                                "CALLBACK" => {
                                                    // Bot message callback
                                                    let cb_msg_id = frame["headers"]["messageId"]
                                                        .as_str()
                                                        .unwrap_or("")
                                                        .to_string();

                                                    if let Some(mut channel_msg) =
                                                        Self::parse_stream_event(&frame)
                                                    {
                                                        // Inject account_id for multi-bot routing
                                                        if let Some(ref aid) = *account_id {
                                                            channel_msg.metadata.insert(
                                                                "account_id".to_string(),
                                                                serde_json::json!(aid),
                                                            );
                                                        }
                                                        if tx.send(channel_msg).await.is_err() {
                                                            info!("DingTalk stream: channel receiver dropped, exiting");
                                                            return;
                                                        }
                                                    }

                                                    // ACK the message so DingTalk doesn't redeliver.
                                                    let ack = serde_json::json!({
                                                        "code": 200,
                                                        "headers": {
                                                            "contentType": "application/json",
                                                            "messageId": cb_msg_id,
                                                        },
                                                        "message": "OK",
                                                        "data": "{\"response\": null}",
                                                    });
                                                    if let Err(e) = ws_write
                                                        .send(tokio_tungstenite::tungstenite::Message::Text(
                                                            ack.to_string().into(),
                                                        ))
                                                        .await
                                                    {
                                                        warn!("DingTalk stream: failed to send ACK: {e}");
                                                        break;
                                                    }
                                                }
                                                _ => {
                                                    debug!(frame_type, topic, "DingTalk stream: unhandled frame type/topic");
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("DingTalk stream: invalid JSON frame: {e}");
                                        }
                                    }
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload))) => {
                                    let _ = ws_write
                                        .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                                        .await;
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(_))) => {}
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => {
                                    info!("DingTalk stream: WebSocket closed by server, reconnecting...");
                                    break;
                                }
                                Some(Err(e)) => {
                                    warn!("DingTalk stream: WebSocket error: {e}");
                                    break;
                                }
                                None => {
                                    info!("DingTalk stream: WebSocket stream ended, reconnecting...");
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = shutdown_rx.changed() => {
                            info!("DingTalk stream adapter shutting down");
                            let _ = ws_write
                                .send(tokio_tungstenite::tungstenite::Message::Close(None))
                                .await;
                            return;
                        }
                    }
                }

                // Brief delay before reconnecting with exponential backoff.
                warn!("DingTalk stream: disconnected, reconnecting in {backoff:?}...");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {
                        backoff = (backoff * 2).min(WS_MAX_BACKOFF);
                    }
                    _ = shutdown_rx.changed() => break,
                }
            }

            info!("DingTalk stream adapter stopped");
        });
    }
}

#[async_trait]
impl ChannelAdapter for DingTalkAdapter {
    fn name(&self) -> &str {
        "dingtalk"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("dingtalk".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        if self.mode != DingTalkMode::Webhook {
            return None;
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let tx = Arc::new(tx);
        let secret = Arc::new(self.secret.clone());
        let account_id = Arc::new(self.account_id.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let tx = Arc::clone(&tx);
                let secret = Arc::clone(&secret);
                let account_id = Arc::clone(&account_id);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = Arc::clone(&tx);
                    let secret = Arc::clone(&secret);
                    let account_id = Arc::clone(&account_id);
                    async move {
                        // Extract timestamp and sign from headers.
                        // A missing or non-numeric timestamp is always a hard error —
                        // never silently skip HMAC verification.
                        let timestamp_str = headers
                            .get("timestamp")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        let signature = headers
                            .get("sign")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");

                        let ts = match timestamp_str.parse::<i64>() {
                            Ok(t) => t,
                            Err(_) => {
                                warn!(
                                    "DingTalk: missing or non-numeric timestamp header — \
                                     rejecting request"
                                );
                                return axum::http::StatusCode::BAD_REQUEST;
                            }
                        };

                        // Verify signature — body bytes are included in the HMAC
                        // (#3879) to prevent replay attacks where a captured
                        // (timestamp, sign) pair is reused with a different payload.
                        if !DingTalkAdapter::verify_signature(&secret, ts, &body, signature) {
                            warn!("DingTalk: invalid signature");
                            return axum::http::StatusCode::UNAUTHORIZED;
                        }

                        // Check timestamp freshness (#3441: tightened from
                        // 1 hour to ±5 min — DingTalk's own client signs
                        // with the current millis, so a window wider than
                        // a few minutes is purely replay surface).  A
                        // stale timestamp is treated as a possible replay,
                        // so we reject as unauthorized rather than forbidden —
                        // matching the missing/invalid-signature path.
                        const REPLAY_WINDOW_MS: u64 = 5 * 60 * 1_000;
                        let now = Utc::now().timestamp_millis();
                        // Use saturating_sub to avoid i64 overflow on a
                        // forged extreme timestamp; treat overflow as stale.
                        if now.saturating_sub(ts).unsigned_abs() > REPLAY_WINDOW_MS {
                            warn!("DingTalk: stale timestamp (outside ±5 min window)");
                            return axum::http::StatusCode::UNAUTHORIZED;
                        }

                        // Parse JSON from the raw bytes we already read
                        let json_body: serde_json::Value = match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        };

                        if let Some((text, sender_id, sender_nick, conv_id, is_group)) =
                            DingTalkAdapter::parse_callback(&json_body)
                        {
                            let content = if text.starts_with('/') {
                                let parts: Vec<&str> = text.splitn(2, ' ').collect();
                                let cmd = parts[0].trim_start_matches('/');
                                let args: Vec<String> = parts
                                    .get(1)
                                    .map(|a| a.split_whitespace().map(String::from).collect())
                                    .unwrap_or_default();
                                ChannelContent::Command {
                                    name: cmd.to_string(),
                                    args,
                                }
                            } else {
                                ChannelContent::Text(text)
                            };

                            let mut msg = ChannelMessage {
                                channel: ChannelType::Custom("dingtalk".to_string()),
                                platform_message_id: format!(
                                    "dt-{}",
                                    Utc::now().timestamp_millis()
                                ),
                                sender: ChannelUser {
                                    platform_id: sender_id,
                                    display_name: sender_nick,
                                    librefang_user: None,
                                },
                                content,
                                target_agent: None,
                                timestamp: Utc::now(),
                                is_group,
                                thread_id: None,
                                metadata: {
                                    let mut m = HashMap::new();
                                    m.insert(
                                        "conversation_id".to_string(),
                                        serde_json::Value::String(conv_id),
                                    );
                                    m
                                },
                            };

                            // Inject account_id for multi-bot routing
                            if let Some(ref aid) = *account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            if tx.send(msg).await.is_err() {
                                // Bridge receiver closed — the message is lost
                                // even though we're about to return 200 OK to
                                // the upstream. Log so operators can notice.
                                warn!("DingTalk: bridge channel closed, incoming message dropped");
                            }
                        }

                        axum::http::StatusCode::OK
                    }
                }
            }),
        );

        info!("DingTalk adapter registered webhook routes on shared server at /channels/dingtalk/webhook");

        Some((
            app,
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
        ))
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        match self.mode {
            DingTalkMode::Webhook => {
                // When using the shared webhook server, create_webhook_routes() is called
                // instead. This start() is only reached as a fallback (shouldn't happen
                // in normal operation since BridgeManager prefers create_webhook_routes).
                let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
                Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }
            DingTalkMode::Stream => {
                let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
                self.start_stream(tx);
                Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }
        }
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            _ => "(Unsupported content type)".to_string(),
        };

        let chunks = split_message(&text, MAX_MESSAGE_LEN);
        let num_chunks = chunks.len();

        for chunk in chunks {
            let url = match self.mode {
                DingTalkMode::Webhook => self.build_send_url(),
                DingTalkMode::Stream => Self::stream_reply_url(user)
                    .ok_or("DingTalk stream reply missing sessionWebhook")?,
            };
            let body = serde_json::json!({
                "msgtype": "text",
                "text": {
                    "content": chunk,
                }
            });

            let resp = self.client.post(&url).json(&body).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(format!("DingTalk API error {status}: {err_body}").into());
            }

            let result: serde_json::Value = resp.json().await?;
            // Both webhook and stream session-webhook replies use errcode.
            if let Some(errcode) = result["errcode"].as_i64() {
                if errcode != 0 {
                    return Err(format!(
                        "DingTalk error (errcode={errcode}): {}",
                        result["errmsg"].as_str().unwrap_or("unknown")
                    )
                    .into());
                }
            }

            // Rate limit: small delay between chunks
            if num_chunks > 1 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // DingTalk Robot API does not support typing indicators.
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- send() path tests (issue #3820) -----
    //
    // Uses `wiremock` to stand up a local HTTP server and points `DingTalkAdapter`
    // at it via `with_api_base()`. This exercises the `POST /robot/send` call made
    // by `ChannelAdapter::send` in webhook mode.

    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_webhook_adapter(api_base: String) -> DingTalkAdapter {
        DingTalkAdapter::new(
            "test-access-token".to_string(),
            "test-secret".to_string(),
            8080,
        )
        .with_api_base(api_base)
    }

    fn dummy_user(channel_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: channel_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn dingtalk_send_posts_robot_send_with_text_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/robot/send"))
            .and(body_json(serde_json::json!({
                "msgtype": "text",
                "text": { "content": "hello from librefang" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "errcode": 0,
                "errmsg": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_webhook_adapter(server.uri());
        adapter
            .send(
                &dummy_user("channel-abc"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn dingtalk_send_non_text_content_falls_back_to_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/robot/send"))
            .and(body_json(serde_json::json!({
                "msgtype": "text",
                "text": { "content": "(Unsupported content type)" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "errcode": 0,
                "errmsg": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_webhook_adapter(server.uri());
        adapter
            .send(
                &dummy_user("channel-xyz"),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send must succeed with unsupported content");
    }

    #[test]
    fn test_dingtalk_adapter_creation_webhook() {
        let adapter =
            DingTalkAdapter::new("test-token".to_string(), "test-secret".to_string(), 8080);
        assert_eq!(adapter.name(), "dingtalk");
        assert_eq!(adapter.mode, DingTalkMode::Webhook);
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("dingtalk".to_string())
        );
    }

    #[test]
    fn test_dingtalk_adapter_creation_stream() {
        let adapter =
            DingTalkAdapter::new_stream("client-id".to_string(), "client-secret".to_string());
        assert_eq!(adapter.name(), "dingtalk");
        assert_eq!(adapter.mode, DingTalkMode::Stream);
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("dingtalk".to_string())
        );
    }

    #[test]
    fn test_dingtalk_signature_computation() {
        let timestamp: i64 = 1700000000000;
        let secret = "my-secret";
        let body = b"test-body";
        let sig = DingTalkAdapter::compute_signature(secret, timestamp, body);
        assert!(!sig.is_empty());
        // Verify deterministic output
        let sig2 = DingTalkAdapter::compute_signature(secret, timestamp, body);
        assert_eq!(sig, sig2);
        // Different body must produce a different signature
        let sig3 = DingTalkAdapter::compute_signature(secret, timestamp, b"other-body");
        assert_ne!(sig, sig3);
    }

    #[test]
    fn test_dingtalk_signature_verification() {
        let secret = "test-secret-123";
        let timestamp: i64 = 1700000000000;
        let body = b"webhook-body";
        let sig = DingTalkAdapter::compute_signature(secret, timestamp, body);
        assert!(DingTalkAdapter::verify_signature(
            secret, timestamp, body, &sig
        ));
        assert!(!DingTalkAdapter::verify_signature(
            secret, timestamp, body, "bad-sig"
        ));
        assert!(!DingTalkAdapter::verify_signature(
            "wrong-secret",
            timestamp,
            body,
            &sig
        ));
        // Tampered body must fail even with correct timestamp/secret
        assert!(!DingTalkAdapter::verify_signature(
            secret,
            timestamp,
            b"tampered-body",
            &sig
        ));
    }

    #[test]
    fn test_dingtalk_parse_callback_text() {
        let body = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "Hello bot" },
            "senderId": "user123",
            "senderNick": "Alice",
            "conversationId": "conv456",
            "conversationType": "2",
        });
        let result = DingTalkAdapter::parse_callback(&body);
        assert!(result.is_some());
        let (text, sender_id, sender_nick, conv_id, is_group) = result.unwrap();
        assert_eq!(text, "Hello bot");
        assert_eq!(sender_id, "user123");
        assert_eq!(sender_nick, "Alice");
        assert_eq!(conv_id, "conv456");
        assert!(is_group);
    }

    #[test]
    fn test_dingtalk_parse_callback_unsupported_type() {
        let body = serde_json::json!({
            "msgtype": "image",
            "image": { "downloadCode": "abc" },
        });
        assert!(DingTalkAdapter::parse_callback(&body).is_none());
    }

    #[test]
    fn test_dingtalk_parse_callback_dm() {
        let body = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "DM message" },
            "senderId": "u1",
            "senderNick": "Bob",
            "conversationId": "c1",
            "conversationType": "1",
        });
        let result = DingTalkAdapter::parse_callback(&body);
        assert!(result.is_some());
        let (_, _, _, _, is_group) = result.unwrap();
        assert!(!is_group);
    }

    #[test]
    fn test_dingtalk_verify_signature_rejects_wrong_timestamp() {
        let secret = "test-secret";
        let ts: i64 = 1700000000000;
        let body: &[u8] = b"{\"msgtype\":\"text\"}";
        let good_sig = DingTalkAdapter::compute_signature(secret, ts, body);
        // Different timestamp → signature mismatch
        assert!(!DingTalkAdapter::verify_signature(
            secret,
            ts + 1,
            body,
            &good_sig
        ));
    }

    #[test]
    fn test_dingtalk_send_url_contains_token_and_sign() {
        let adapter = DingTalkAdapter::new("my-token".to_string(), "my-secret".to_string(), 8080);
        let url = adapter.build_send_url();
        assert!(url.contains("access_token=my-token"));
        assert!(url.contains("timestamp="));
        assert!(url.contains("sign="));
    }

    // ----- Stream mode tests -----

    #[test]
    fn test_dingtalk_parse_stream_event() {
        let data_payload = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "Hello from stream" },
            "senderStaffId": "staff123",
            "senderNick": "StreamUser",
            "conversationId": "conv789",
            "conversationType": "1",
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "data": data_payload.to_string(),
        });
        let result = DingTalkAdapter::parse_stream_event(&frame);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.sender.platform_id, "staff123");
        assert_eq!(msg.sender.display_name, "StreamUser");
        assert!(!msg.is_group);
        match msg.content {
            ChannelContent::Text(t) => assert_eq!(t, "Hello from stream"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_dingtalk_parse_stream_event_command() {
        let data_payload = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "/help arg1 arg2" },
            "senderStaffId": "s1",
            "senderNick": "Cmd",
            "conversationId": "c1",
            "conversationType": "2",
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "data": data_payload.to_string(),
        });
        let result = DingTalkAdapter::parse_stream_event(&frame);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(msg.is_group);
        match msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "help");
                assert_eq!(args, vec!["arg1", "arg2"]);
            }
            _ => panic!("Expected command content"),
        }
    }

    #[test]
    fn test_dingtalk_parse_stream_event_preserves_message_id_and_mentions() {
        let data_payload = serde_json::json!({
            "msgtype": "text",
            "msgId": "msg-123",
            "text": { "content": "@bot hello" },
            "senderStaffId": "s1",
            "senderNick": "Mentioned",
            "conversationId": "c1",
            "conversationType": "2",
            "isInAtList": true,
            "atUsers": [{ "dingtalkId": "$:LWCP_v1:$abc" }],
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "headers": {
                "messageId": "frame-456",
            },
            "data": data_payload.to_string(),
        });
        let result = DingTalkAdapter::parse_stream_event(&frame);
        assert!(result.is_some());
        let msg = result.unwrap();
        // Should prefer msgId from payload over headers.messageId
        assert_eq!(msg.platform_message_id, "msg-123");
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_dingtalk_parse_stream_event_empty_text() {
        let data_payload = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "   " },
            "senderStaffId": "s1",
            "senderNick": "Empty",
            "conversationId": "c1",
            "conversationType": "1",
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "data": data_payload.to_string(),
        });
        assert!(DingTalkAdapter::parse_stream_event(&frame).is_none());
    }

    #[test]
    fn test_dingtalk_parse_stream_event_unsupported_type() {
        let data_payload = serde_json::json!({
            "msgtype": "image",
            "image": {},
            "senderStaffId": "s1",
            "senderNick": "Img",
            "conversationId": "c1",
            "conversationType": "1",
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "data": data_payload.to_string(),
        });
        assert!(DingTalkAdapter::parse_stream_event(&frame).is_none());
    }

    #[test]
    fn test_dingtalk_parse_stream_event_session_webhook() {
        let data_payload = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "hello" },
            "senderStaffId": "s1",
            "senderNick": "User",
            "conversationId": "c1",
            "conversationType": "1",
            "sessionWebhook": "https://oapi.dingtalk.com/robot/sendBySession/abc123",
            "sessionWebhookExpiredTime": 1700003600000_i64,
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "data": data_payload.to_string(),
        });
        let result = DingTalkAdapter::parse_stream_event(&frame);
        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(
            msg.sender.librefang_user,
            Some("https://oapi.dingtalk.com/robot/sendBySession/abc123".to_string())
        );
        assert_eq!(
            msg.metadata
                .get("session_webhook_expired_time")
                .and_then(|v| v.as_i64()),
            Some(1700003600000)
        );
    }

    #[test]
    fn test_dingtalk_stream_reply_url() {
        let user_with_webhook = ChannelUser {
            platform_id: "u1".to_string(),
            display_name: "User".to_string(),
            librefang_user: Some("https://oapi.dingtalk.com/robot/sendBySession/abc".to_string()),
        };
        assert_eq!(
            DingTalkAdapter::stream_reply_url(&user_with_webhook),
            Some("https://oapi.dingtalk.com/robot/sendBySession/abc".to_string())
        );

        let user_without_webhook = ChannelUser {
            platform_id: "u2".to_string(),
            display_name: "User2".to_string(),
            librefang_user: None,
        };
        assert!(DingTalkAdapter::stream_reply_url(&user_without_webhook).is_none());

        let user_empty_webhook = ChannelUser {
            platform_id: "u3".to_string(),
            display_name: "User3".to_string(),
            librefang_user: Some(String::new()),
        };
        assert!(DingTalkAdapter::stream_reply_url(&user_empty_webhook).is_none());
    }
}
