//! Unified Feishu/Lark Open Platform channel adapter.
//!
//! Feishu (CN) and Lark (international) are the same ByteDance product with
//! different API domains. This adapter auto-detects the region based on
//! configuration or explicit `region` setting.
//!
//! ## Receive modes
//!
//! - **Webhook** (legacy): HTTP server that receives event callbacks from the
//!   Feishu/Lark platform. Requires a public IP or reverse proxy.
//! - **WebSocket** (default): Long-lived WebSocket connection to the Feishu/Lark
//!   event gateway. No public IP required. Lower latency.
//!
//! Authentication is performed via a tenant access token obtained from the
//! `/auth/v3/tenant_access_token/internal` endpoint. The token is cached and
//! refreshed automatically (2-hour expiry).

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// Region
// ---------------------------------------------------------------------------

/// Feishu/Lark API region.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeishuRegion {
    /// China mainland — `open.feishu.cn`
    #[default]
    Cn,
    /// International — `open.larksuite.com`
    Intl,
}

impl FeishuRegion {
    /// Base URL for REST API calls.
    fn api_base(self) -> &'static str {
        match self {
            Self::Cn => "https://open.feishu.cn",
            Self::Intl => "https://open.larksuite.com",
        }
    }

    /// Human-readable label used in log messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cn => "Feishu",
            Self::Intl => "Lark",
        }
    }

    /// Detect region from an `app_id` prefix or domain hint.
    ///
    /// Feishu CN app IDs start with `cli_`, while Lark international IDs
    /// typically start with `cli_a` (longer prefix). This is a best-effort
    /// heuristic — explicit configuration should be preferred.
    pub fn detect(app_id: &str, domain_hint: Option<&str>) -> Self {
        if let Some(hint) = domain_hint {
            let h = hint.to_lowercase();
            if h.contains("larksuite") || h.contains("lark") {
                return Self::Intl;
            }
            if h.contains("feishu") {
                return Self::Cn;
            }
        }
        // Default: treat as CN (the more common deployment).
        // NOTE: We cannot reliably distinguish by app_id alone, so we fall
        // back to Cn unless explicitly configured otherwise.
        let _ = app_id;
        Self::Cn
    }
}

// ---------------------------------------------------------------------------
// Receive mode
// ---------------------------------------------------------------------------

/// How the adapter receives inbound events from the Feishu/Lark platform.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeishuReceiveMode {
    /// HTTP webhook server (requires public IP / reverse proxy).
    Webhook,
    /// Long-lived WebSocket connection (default, no public IP required).
    #[default]
    Websocket,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum Feishu message text length (characters).
const MAX_MESSAGE_LEN: usize = 4096;

/// Token refresh buffer — refresh 5 minutes before actual expiry.
const TOKEN_REFRESH_BUFFER_SECS: u64 = 300;

/// Initial back-off for WebSocket reconnection.
const WS_INITIAL_BACKOFF: Duration = Duration::from_secs(2);

/// Maximum back-off between WebSocket reconnection attempts.
const WS_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Event dedup window — ignore events with the same event_id within this window.
/// Feishu retries webhook callbacks up to 3 times within ~1 minute, so 5 minutes
/// provides ample coverage.
const EVENT_DEDUP_WINDOW: Duration = Duration::from_secs(300);

/// Maximum number of seen events before triggering a purge of expired entries.
const EVENT_DEDUP_MAX_ENTRIES: usize = 10_000;

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Unified Feishu/Lark Open Platform adapter.
///
/// Supports both Feishu (CN) and Lark (international) regions, and both
/// webhook and WebSocket receive modes.
pub struct FeishuAdapter {
    /// Feishu app ID.
    app_id: String,
    /// SECURITY: Feishu app secret, zeroized on drop.
    app_secret: Zeroizing<String>,
    /// Region (CN or international).
    region: FeishuRegion,
    /// How to receive inbound events.
    receive_mode: FeishuReceiveMode,
    /// Port on which the inbound webhook HTTP server listens (webhook mode only).
    webhook_port: u16,
    /// Optional verification token for webhook event validation.
    verification_token: Option<String>,
    /// Optional encrypt key for webhook event decryption.
    /// TODO: implement AES-CBC decryption for encrypted event payloads.
    encrypt_key: Option<String>,
    /// HTTP client for API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Cached tenant access token and its expiry instant.
    cached_token: Arc<RwLock<Option<(String, Instant)>>>,
    /// Event dedup cache — maps event_id → first-seen Instant.
    /// Prevents duplicate processing when Feishu retries webhook/WS events.
    seen_events: Arc<Mutex<HashMap<String, Instant>>>,
}

impl FeishuAdapter {
    /// Create a new Feishu/Lark adapter with the given region and receive mode.
    pub fn new(
        app_id: String,
        app_secret: String,
        webhook_port: u16,
        region: FeishuRegion,
        receive_mode: FeishuReceiveMode,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            app_id,
            app_secret: Zeroizing::new(app_secret),
            region,
            receive_mode,
            webhook_port,
            verification_token: None,
            encrypt_key: None,
            client: crate::http_client::new_client(),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            cached_token: Arc::new(RwLock::new(None)),
            seen_events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Set webhook verification credentials. Returns self for builder chaining.
    pub fn with_verification(
        mut self,
        verification_token: Option<String>,
        encrypt_key: Option<String>,
    ) -> Self {
        self.verification_token = verification_token;
        self.encrypt_key = encrypt_key;
        self
    }

    /// Region-aware token URL.
    #[cfg(test)]
    fn token_url(&self) -> String {
        format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            self.region.api_base()
        )
    }

    /// Region-aware send message URL.
    fn send_url(&self) -> String {
        format!("{}/open-apis/im/v1/messages", self.region.api_base())
    }

    /// Region-aware bot info URL.
    fn bot_info_url(&self) -> String {
        format!("{}/open-apis/bot/v3/info", self.region.api_base())
    }

    /// Obtain a valid tenant access token, refreshing if expired or missing.
    async fn get_token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        get_token_static(
            &self.client,
            self.region,
            &self.app_id,
            &self.app_secret,
            &self.cached_token,
        )
        .await
    }

    /// Validate credentials by fetching bot info.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let label = self.region.label();
        let token = self.get_token().await?;

        let resp = self
            .client
            .get(self.bot_info_url())
            .bearer_auth(&token)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("{label} authentication failed {status}: {body}").into());
        }

        let body: serde_json::Value = resp.json().await?;
        let code = body["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = body["msg"].as_str().unwrap_or("unknown error");
            return Err(format!("{label} bot info error: {msg}").into());
        }

        let default_name = format!("{label} Bot");
        let bot_name = body["bot"]["app_name"]
            .as_str()
            .unwrap_or(&default_name)
            .to_string();
        Ok(bot_name)
    }

    /// Send a text message to a Feishu/Lark chat.
    async fn api_send_message(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let label = self.region.label();
        let token = self.get_token().await?;
        let encoded_type: String =
            url::form_urlencoded::byte_serialize(receive_id_type.as_bytes()).collect();
        let url = format!("{}?receive_id_type={}", self.send_url(), encoded_type);

        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let content = serde_json::json!({
                "text": chunk,
            });

            let body = serde_json::json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": content.to_string(),
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("{label} send message error {status}: {resp_body}").into());
            }

            let resp_body: serde_json::Value = resp.json().await?;
            let code = resp_body["code"].as_i64().unwrap_or(-1);
            if code != 0 {
                let msg = resp_body["msg"].as_str().unwrap_or("unknown error");
                warn!("{label} send message API error: {msg}");
            }
        }

        Ok(())
    }

    /// Reply to a message in a thread.
    #[allow(dead_code)]
    async fn api_reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let label = self.region.label();
        let token = self.get_token().await?;
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reply",
            self.region.api_base(),
            message_id
        );

        let content = serde_json::json!({
            "text": text,
        });

        let body = serde_json::json!({
            "msg_type": "text",
            "content": content.to_string(),
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("{label} reply message error {status}: {resp_body}").into());
        }

        Ok(())
    }

    /// Send an interactive card message to a Feishu/Lark chat.
    ///
    /// Uses the Feishu IM API with `msg_type: "interactive"` to send a card
    /// built from a `serde_json::Value` card template.
    pub async fn send_card(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        card: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let label = self.region.label();
        let token = self.get_token().await?;
        let encoded_type: String =
            url::form_urlencoded::byte_serialize(receive_id_type.as_bytes()).collect();
        let url = format!("{}?receive_id_type={}", self.send_url(), encoded_type);

        // Feishu API requires `content` to be a JSON-encoded string, not a nested object.
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": card.to_string(),
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("{label} send card error {status}: {resp_body}").into());
        }

        let resp_body: serde_json::Value = resp.json().await?;
        let code = resp_body["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = resp_body["msg"].as_str().unwrap_or("unknown error");
            error!("{label} send card API error (code={code}): {msg}");
            return Err(format!("{label} send card API error (code={code}): {msg}").into());
        }

        Ok(())
    }

    /// Send an approval card to a Feishu/Lark chat for an agent permission request.
    ///
    /// Renders an interactive card with Approve / Deny buttons. When the user
    /// clicks a button, Feishu sends a `card.action.trigger` callback to the
    /// webhook, which the adapter converts into a `/approve` or `/reject`
    /// command message.
    pub async fn send_approval_card(
        &self,
        chat_id: &str,
        request_id: &str,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        risk_level: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let card = build_approval_card(request_id, agent_id, tool_name, action_summary, risk_level);
        self.send_card(chat_id, "chat_id", &card).await
    }

    // -----------------------------------------------------------------------
    // Webhook receive mode
    // -----------------------------------------------------------------------

    /// Start the webhook HTTP server and return a message stream.
    fn start_webhook(&self, tx: mpsc::Sender<ChannelMessage>) {
        let port = self.webhook_port;
        let verification_token = self.verification_token.clone();
        let seen_events = Arc::clone(&self.seen_events);
        let encrypt_key = self.encrypt_key.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = Arc::new(self.account_id.clone());
        let label = self.region.label();
        let region = self.region;

        tokio::spawn(async move {
            let verification_token = Arc::new(verification_token);
            let encrypt_key = Arc::new(encrypt_key);
            let tx = Arc::new(tx);

            let app = axum::Router::new().route(
                "/feishu/webhook",
                axum::routing::post({
                    let vt = Arc::clone(&verification_token);
                    let ek = Arc::clone(&encrypt_key);
                    let tx = Arc::clone(&tx);
                    let seen = Arc::clone(&seen_events);
                    move |body: axum::extract::Json<serde_json::Value>| {
                        let vt = Arc::clone(&vt);
                        let ek = Arc::clone(&ek);
                        let tx = Arc::clone(&tx);
                        let seen = Arc::clone(&seen);
                        async move {
                            let payload =
                                match decrypt_feishu_payload_if_needed(&body.0, ek.as_deref()) {
                                    Ok(value) => value,
                                    Err(err) => {
                                        warn!("{label}: failed to decrypt webhook payload: {err}");
                                        return (
                                            axum::http::StatusCode::BAD_REQUEST,
                                            axum::Json(serde_json::json!({})),
                                        );
                                    }
                                };

                            // Handle URL verification challenge
                            if let Some(challenge) = payload.get("challenge") {
                                // Verify token if configured
                                if let Some(ref expected_token) = *vt {
                                    let token = payload["token"].as_str().unwrap_or("");
                                    if token != expected_token {
                                        warn!("{}: invalid verification token", label);
                                        return (
                                            axum::http::StatusCode::FORBIDDEN,
                                            axum::Json(serde_json::json!({})),
                                        );
                                    }
                                }
                                return (
                                    axum::http::StatusCode::OK,
                                    axum::Json(serde_json::json!({
                                        "challenge": challenge,
                                    })),
                                );
                            }

                            // Deduplicate by event_id — Feishu retries webhook
                            // callbacks up to 3 times when the response is slow.
                            if is_duplicate_event(&payload, &seen) {
                                debug!("{label}: duplicate event, skipping");
                                return (
                                    axum::http::StatusCode::OK,
                                    axum::Json(serde_json::json!({})),
                                );
                            }

                            // Handle event callback
                            if let Some(schema) = payload["schema"].as_str() {
                                if schema == "2.0" {
                                    // V2 event format — try message first, then card action
                                    let parsed = parse_feishu_event(&payload, region)
                                        .or_else(|| parse_card_action(&payload, region));
                                    if let Some(mut msg) = parsed {
                                        // Inject account_id for multi-bot routing
                                        if let Some(ref aid) = *account_id {
                                            msg.metadata.insert(
                                                "account_id".to_string(),
                                                serde_json::json!(aid),
                                            );
                                        }
                                        let _ = tx.send(msg).await;
                                    }
                                }
                            } else {
                                // V1 event format (legacy)
                                if let Some(mut msg) = parse_feishu_event_v1(&payload, region) {
                                    if let Some(ref aid) = *account_id {
                                        msg.metadata.insert(
                                            "account_id".to_string(),
                                            serde_json::json!(aid),
                                        );
                                    }
                                    let _ = tx.send(msg).await;
                                }
                            }

                            (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({})),
                            )
                        }
                    }
                }),
            );

            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            info!("{label} webhook server listening on {addr}");

            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    warn!("{label} webhook bind failed: {e}");
                    return;
                }
            };

            let server = axum::serve(listener, app);

            tokio::select! {
                result = server => {
                    if let Err(e) = result {
                        warn!("{label} webhook server error: {e}");
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("{label} adapter shutting down (webhook)");
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // WebSocket receive mode
    // -----------------------------------------------------------------------

    /// Start a WebSocket connection to the Feishu/Lark event gateway and
    /// forward received events as `ChannelMessage`s.
    ///
    /// Uses the two-step endpoint discovery protocol:
    /// 1. POST `/callback/ws/endpoint` with app credentials to get the real WS URL.
    /// 2. Connect to the returned URL via standard WebSocket upgrade (HTTP 101).
    fn start_websocket(&self, tx: mpsc::Sender<ChannelMessage>) {
        let app_id = self.app_id.clone();
        let app_secret = self.app_secret.clone();
        let region = self.region;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = Arc::new(self.account_id.clone());
        let client = self.client.clone();
        let seen_events = Arc::clone(&self.seen_events);

        tokio::spawn(async move {
            let label = region.label();
            let mut backoff = WS_INITIAL_BACKOFF;

            loop {
                // Step 1: Get the real WebSocket URL from the endpoint API.
                let (ws_url, client_config) =
                    match get_ws_endpoint(&client, region, &app_id, &app_secret).await {
                        Ok(pair) => pair,
                        Err(e) => {
                            error!("{label}: failed to obtain WS endpoint: {e}");
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(WS_MAX_BACKOFF);
                            continue;
                        }
                    };

                debug!("{label}: WS endpoint URL: {ws_url}");
                debug!("{label}: WS client_config: {client_config:?}");
                info!("{label}: connecting to WebSocket event gateway...");

                // Step 2: Connect to the returned WebSocket URL.
                let ws_result = tokio_tungstenite::connect_async(&ws_url).await;
                let (ws_stream, _) = match ws_result {
                    Ok(pair) => pair,
                    Err(e) => {
                        error!("{label}: WebSocket connection failed: {e}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(WS_MAX_BACKOFF);
                        continue;
                    }
                };

                info!("{label}: WebSocket connected");
                backoff = WS_INITIAL_BACKOFF;

                use futures::{SinkExt, StreamExt};
                let (mut ws_tx, mut ws_rx) = ws_stream.split();

                // Use server-provided ping interval, default 120s (matching Go SDK).
                let ping_secs = client_config
                    .as_ref()
                    .map(|c| c.ping_interval)
                    .filter(|&p| p > 0)
                    .unwrap_or(120) as u64;
                let mut ping_interval = tokio::time::interval(Duration::from_secs(ping_secs));
                ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                loop {
                    tokio::select! {
                        // Shutdown signal
                        _ = shutdown_rx.changed() => {
                            info!("{label}: shutting down WebSocket");
                            let _ = ws_tx.close().await;
                            return;
                        }
                        // Heartbeat ping
                        _ = ping_interval.tick() => {
                            let ping = serde_json::json!({"type": "ping"});
                            if let Err(e) = ws_tx.send(
                                tokio_tungstenite::tungstenite::Message::Text(ping.to_string().into())
                            ).await {
                                warn!("{label}: WS ping send failed: {e}");
                                break;
                            }
                        }
                        // Incoming message
                        msg = ws_rx.next() => {
                            match msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    debug!("{label}: WS recv text frame, len={}: {}", text.len(), &text[..text.len().min(500)]);
                                    let payload: serde_json::Value = match serde_json::from_str(&text) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            warn!("{label}: WS payload parse error: {e}");
                                            continue;
                                        }
                                    };

                                    // Handle pong / service messages
                                    if let Some(msg_type) = payload["type"].as_str() {
                                        if msg_type == "pong" {
                                            debug!("{label}: WS pong received");
                                            continue;
                                        }
                                    }

                                    // Deduplicate by event_id (WS reconnects
                                    // can re-deliver events)
                                    if is_duplicate_event(&payload, &seen_events) {
                                        debug!("{label}: WS duplicate event, skipping");
                                        continue;
                                    }

                                    // The WS gateway wraps the normal event callback
                                    // in an envelope: { "header": {...}, "event": {...} }
                                    // which matches the v2 schema.
                                    let parsed = parse_feishu_event(&payload, region)
                                        .or_else(|| parse_card_action(&payload, region));
                                    if let Some(mut channel_msg) = parsed {
                                        if let Some(ref aid) = *account_id {
                                            channel_msg.metadata.insert(
                                                "account_id".to_string(),
                                                serde_json::json!(aid),
                                            );
                                        }
                                        if tx.send(channel_msg).await.is_err() {
                                            info!("{label}: channel receiver dropped, exiting WS loop");
                                            return;
                                        }
                                    }
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                                    info!("{label}: WebSocket closed by server: {frame:?}");
                                    break;
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data))) => {
                                    debug!("{label}: WS recv ping frame, len={}", data.len());
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(data))) => {
                                    debug!("{label}: WS recv pong frame, len={}", data.len());
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) => {
                                    debug!("{label}: WS recv binary frame, len={}", data.len());

                                    // Feishu sends events as protobuf-wrapped binary frames.
                                    // Extract the embedded JSON payload by scanning for the
                                    // first '{' that starts a valid JSON object.
                                    let json_payload = data.iter()
                                        .position(|&b| b == b'{')
                                        .and_then(|start| {
                                            // Find the matching closing brace by scanning backwards
                                            let slice = &data[start..];
                                            std::str::from_utf8(slice).ok()
                                        })
                                        .and_then(|text| {
                                            // The JSON may be followed by protobuf trailer bytes;
                                            // find the last '}' to get the real JSON boundary.
                                            text.rfind('}').map(|end| &text[..=end])
                                        });

                                    if let Some(json_str) = json_payload {
                                        debug!("{label}: WS binary JSON extracted, len={}", json_str.len());
                                        let payload: serde_json::Value = match serde_json::from_str(json_str) {
                                            Ok(v) => v,
                                            Err(e) => {
                                                warn!("{label}: WS binary JSON parse error: {e}");
                                                continue;
                                            }
                                        };

                                        if is_duplicate_event(&payload, &seen_events) {
                                            debug!("{label}: WS binary duplicate event, skipping");
                                            continue;
                                        }

                                        let parsed = parse_feishu_event(&payload, region)
                                            .or_else(|| parse_card_action(&payload, region));
                                        if let Some(mut channel_msg) = parsed {
                                            if let Some(ref aid) = *account_id {
                                                channel_msg.metadata.insert(
                                                    "account_id".to_string(),
                                                    serde_json::json!(aid),
                                                );
                                            }
                                            if tx.send(channel_msg).await.is_err() {
                                                info!("{label}: channel receiver dropped, exiting WS loop");
                                                return;
                                            }
                                        }
                                    } else {
                                        debug!("{label}: WS binary frame has no JSON payload");
                                    }
                                }
                                Some(Ok(other)) => {
                                    debug!("{label}: WS recv unknown frame type: {other:?}");
                                }
                                Some(Err(e)) => {
                                    warn!("{label}: WebSocket error: {e}");
                                    break;
                                }
                                None => {
                                    info!("{label}: WebSocket stream ended");
                                    break;
                                }
                            }
                        }
                    }
                }

                // Reconnect with back-off
                warn!("{label}: WebSocket disconnected, reconnecting in {backoff:?}...");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(WS_MAX_BACKOFF);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// WebSocket endpoint discovery types (Feishu /callback/ws/endpoint API)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct WsEndpointResp {
    code: i64,
    msg: String,
    data: Option<WsEndpointData>,
}

#[derive(serde::Deserialize)]
struct WsEndpointData {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<WsClientConfig>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct WsClientConfig {
    #[serde(rename = "ReconnectCount", default)]
    reconnect_count: i32,
    #[serde(rename = "ReconnectInterval", default)]
    reconnect_interval: i64,
    #[serde(rename = "ReconnectNonce", default)]
    reconnect_nonce: i64,
    #[serde(rename = "PingInterval", default)]
    ping_interval: i64,
}

/// Obtain the real WebSocket URL from the Feishu/Lark endpoint discovery API.
///
/// This implements the two-step protocol used by the official Go SDK:
/// 1. POST to `/callback/ws/endpoint` with app credentials.
/// 2. The response contains the actual `wss://` URL to connect to.
async fn get_ws_endpoint(
    client: &reqwest::Client,
    region: FeishuRegion,
    app_id: &str,
    app_secret: &str,
) -> Result<(String, Option<WsClientConfig>), String> {
    let url = format!("{}/callback/ws/endpoint", region.api_base());
    let body = serde_json::json!({
        "AppID": app_id,
        "AppSecret": app_secret,
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("endpoint request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
    if !status.is_success() {
        return Err(format!("endpoint returned HTTP {status}: {text}"));
    }
    let parsed: WsEndpointResp =
        serde_json::from_str(&text).map_err(|e| format!("parse endpoint response: {e}"))?;
    if parsed.code != 0 {
        return Err(format!(
            "endpoint error code {}: {}",
            parsed.code, parsed.msg
        ));
    }
    let data = parsed.data.ok_or("endpoint response missing data field")?;
    if data.url.is_empty() {
        return Err("endpoint returned empty URL".to_string());
    }
    Ok((data.url, data.client_config))
}

/// Static helper so the WS task (which cannot borrow `&self`) can refresh the
/// tenant access token.
async fn get_token_static(
    client: &reqwest::Client,
    region: FeishuRegion,
    app_id: &str,
    app_secret: &str,
    cached_token: &RwLock<Option<(String, Instant)>>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache
    {
        let guard = cached_token.read().await;
        if let Some((ref token, expiry)) = *guard {
            if Instant::now() < expiry {
                return Ok(token.clone());
            }
        }
    }

    let label = region.label();
    let url = format!(
        "{}/open-apis/auth/v3/tenant_access_token/internal",
        region.api_base()
    );

    let body = serde_json::json!({
        "app_id": app_id,
        "app_secret": app_secret,
    });

    let resp = client.post(&url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let resp_body = resp.text().await.unwrap_or_default();
        return Err(format!("{label} token request failed {status}: {resp_body}").into());
    }

    let resp_body: serde_json::Value = resp.json().await?;
    let code = resp_body["code"].as_i64().unwrap_or(-1);
    if code != 0 {
        let msg = resp_body["msg"].as_str().unwrap_or("unknown error");
        return Err(format!("{label} token error: {msg}").into());
    }

    let tenant_access_token = resp_body["tenant_access_token"]
        .as_str()
        .ok_or("Missing tenant_access_token")?
        .to_string();
    let expire = resp_body["expire"].as_u64().unwrap_or(7200);

    let expiry =
        Instant::now() + Duration::from_secs(expire.saturating_sub(TOKEN_REFRESH_BUFFER_SECS));
    *cached_token.write().await = Some((tenant_access_token.clone(), expiry));

    Ok(tenant_access_token)
}

fn decrypt_feishu_payload_if_needed(
    payload: &serde_json::Value,
    encrypt_key: Option<&str>,
) -> Result<serde_json::Value, String> {
    let Some(encrypted_payload) = payload.get("encrypt").and_then(serde_json::Value::as_str) else {
        return Ok(payload.clone());
    };

    let Some(key) = encrypt_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(
            "encrypted payload received but no Feishu encrypt_key is configured".to_string(),
        );
    };

    decrypt_feishu_payload(encrypted_payload, key)
}

fn decrypt_feishu_payload(
    encrypted_payload: &str,
    encrypt_key: &str,
) -> Result<serde_json::Value, String> {
    use base64::Engine;
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

    let raw = base64::engine::general_purpose::STANDARD
        .decode(encrypted_payload.trim())
        .map_err(|error| format!("base64 decode error: {error}"))?;

    if raw.len() < 16 {
        return Err("encrypted payload too short".to_string());
    }

    let (iv, ciphertext) = raw.split_at(16);
    if ciphertext.is_empty() {
        return Err("encrypted payload ciphertext is empty".to_string());
    }
    if ciphertext.len() % 16 != 0 {
        return Err("encrypted payload ciphertext is not block-aligned".to_string());
    }

    let mut buffer = ciphertext.to_vec();
    let key = Sha256::digest(encrypt_key.as_bytes());
    let decrypted = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, iv)
        .map_err(|error| format!("decrypt init failed: {error}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|error| format!("AES-CBC decrypt failed: {error}"))?;

    serde_json::from_slice::<serde_json::Value>(decrypted)
        .map_err(|error| format!("decrypted payload is not valid JSON: {error}"))
}

// ---------------------------------------------------------------------------
// Approval card builder
// ---------------------------------------------------------------------------

/// Build a Feishu interactive message card for an approval request.
///
/// The card displays the agent, tool, action summary, and risk level,
/// with Approve and Deny action buttons. Each button carries the
/// `request_id` and the chosen action (`approve` / `reject`) in its
/// `value` payload so the callback handler can resolve the request.
pub fn build_approval_card(
    request_id: &str,
    agent_id: &str,
    tool_name: &str,
    action_summary: &str,
    risk_level: &str,
) -> serde_json::Value {
    // Choose header color based on risk level
    let header_color = match risk_level {
        "critical" => "red",
        "high" => "orange",
        "medium" => "yellow",
        _ => "blue",
    };

    serde_json::json!({
        "config": {
            "wide_screen_mode": true
        },
        "header": {
            "title": {
                "tag": "plain_text",
                "content": format!("Agent Permission Request [{risk_level}]")
            },
            "template": header_color
        },
        "elements": [
            {
                "tag": "div",
                "fields": [
                    {
                        "is_short": true,
                        "text": {
                            "tag": "lark_md",
                            "content": format!("**Agent:** {agent_id}")
                        }
                    },
                    {
                        "is_short": true,
                        "text": {
                            "tag": "lark_md",
                            "content": format!("**Tool:** `{tool_name}`")
                        }
                    }
                ]
            },
            {
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": format!("**Action:** {action_summary}")
                }
            },
            {
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": format!("**Request ID:** `{request_id}`")
                }
            },
            {
                "tag": "hr"
            },
            {
                "tag": "action",
                "actions": [
                    {
                        "tag": "button",
                        "text": {
                            "tag": "plain_text",
                            "content": "Approve"
                        },
                        "type": "primary",
                        "value": {
                            "action": "approve",
                            "request_id": request_id
                        }
                    },
                    {
                        "tag": "button",
                        "text": {
                            "tag": "plain_text",
                            "content": "Deny"
                        },
                        "type": "danger",
                        "value": {
                            "action": "reject",
                            "request_id": request_id
                        }
                    }
                ]
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse a Feishu card action callback into a `/approve` or `/reject` command.
///
/// Handles `card.action.trigger` events from interactive card button clicks.
/// Extracts the `action` and `request_id` from the button value payload and
/// converts them into a `ChannelMessage` with a `Command` content type.
fn parse_card_action(event: &serde_json::Value, region: FeishuRegion) -> Option<ChannelMessage> {
    // Defensive: only handle card.action.trigger events
    let header = event.get("header")?;
    if header["event_type"].as_str() != Some("card.action.trigger") {
        return None;
    }

    let event_data = event.get("event")?;
    let action = event_data.get("action")?;
    let value = action.get("value")?;

    let action_type = value["action"].as_str()?;
    let request_id = value["request_id"].as_str()?;

    // Only handle approve / reject actions
    let cmd_name = match action_type {
        "approve" => "approve",
        "reject" => "reject",
        _ => return None,
    };

    // Extract operator info
    let operator = event_data.get("operator")?;
    let open_id = operator
        .get("open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // The chat that the card was sent to (from the token or context)
    let open_chat_id = event_data
        .get("open_chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let open_message_id = event_data
        .get("open_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut metadata = HashMap::new();
    metadata.insert(
        "chat_id".to_string(),
        serde_json::Value::String(open_chat_id.clone()),
    );
    metadata.insert(
        "message_id".to_string(),
        serde_json::Value::String(open_message_id.clone()),
    );
    metadata.insert("card_action".to_string(), serde_json::Value::Bool(true));
    metadata.insert(
        "operator_id".to_string(),
        serde_json::Value::String(open_id.clone()),
    );

    let channel_label = region.label().to_lowercase();
    Some(ChannelMessage {
        channel: ChannelType::Custom(channel_label),
        platform_message_id: open_message_id,
        sender: ChannelUser {
            // Use operator open_id as platform_id so downstream approval
            // routing can identify *who* clicked the button.
            platform_id: open_id.clone(),
            display_name: open_id,
            librefang_user: None,
        },
        content: ChannelContent::Command {
            name: cmd_name.to_string(),
            args: vec![request_id.to_string()],
        },
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata,
    })
}

/// Check whether an event has already been processed (by its `event_id` header).
///
/// Returns `true` if the event is a duplicate that should be skipped.
/// Performs lazy purge of expired entries when the cache grows too large.
fn is_duplicate_event(payload: &serde_json::Value, seen: &Mutex<HashMap<String, Instant>>) -> bool {
    let event_id = payload
        .get("header")
        .and_then(|h| h.get("event_id"))
        .and_then(|v| v.as_str());

    let Some(event_id) = event_id else {
        // No event_id in header (e.g. challenge, pong) — not dedup-able.
        return false;
    };

    let now = Instant::now();
    let mut map = seen.lock().unwrap_or_else(|e| e.into_inner());

    // Purge expired entries when the map is too large.
    if map.len() >= EVENT_DEDUP_MAX_ENTRIES {
        map.retain(|_, ts| now.duration_since(*ts) < EVENT_DEDUP_WINDOW);
    }

    match map.entry(event_id.to_string()) {
        std::collections::hash_map::Entry::Occupied(e) => {
            if now.duration_since(*e.get()) < EVENT_DEDUP_WINDOW {
                return true; // duplicate
            }
            // Expired — treat as new
            false
        }
        std::collections::hash_map::Entry::Vacant(e) => {
            e.insert(now);
            false
        }
    }
}

/// Parse a Feishu/Lark v2 webhook/WS event into a `ChannelMessage`.
///
/// Handles `im.message.receive_v1` events with text message type.
fn parse_feishu_event(event: &serde_json::Value, region: FeishuRegion) -> Option<ChannelMessage> {
    let header = event.get("header")?;
    let event_type = header["event_type"].as_str().unwrap_or("");

    if event_type != "im.message.receive_v1" {
        return None;
    }

    let event_data = event.get("event")?;
    let message = event_data.get("message")?;
    let sender = event_data.get("sender")?;

    let msg_type = message["message_type"].as_str().unwrap_or("");
    if msg_type != "text" {
        return None;
    }

    // Parse the content JSON string
    let content_str = message["content"].as_str().unwrap_or("{}");
    let content_json: serde_json::Value = serde_json::from_str(content_str).unwrap_or_default();
    let mut text = content_json["text"].as_str().unwrap_or("").to_string();

    // Strip mention placeholders like "@_user_1 " that Feishu injects for @mentions
    if let Some(mentions) = message.get("mentions").and_then(|m| m.as_array()) {
        for mention in mentions {
            if let Some(key) = mention["key"].as_str() {
                text = text.replace(key, "");
            }
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let message_id = message["message_id"].as_str().unwrap_or("").to_string();
    let chat_id = message["chat_id"].as_str().unwrap_or("").to_string();
    let chat_type = message["chat_type"].as_str().unwrap_or("p2p");
    let root_id = message["root_id"].as_str().map(|s| s.to_string());

    let sender_id = sender
        .get("sender_id")
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_type = sender["sender_type"].as_str().unwrap_or("user");

    // Skip bot messages
    if sender_type == "bot" {
        return None;
    }

    let is_group = chat_type == "group";
    let channel_label = region.label().to_lowercase();

    let msg_content = if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd_name = parts[0].trim_start_matches('/');
        let args: Vec<String> = parts
            .get(1)
            .map(|a| a.split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        ChannelContent::Command {
            name: cmd_name.to_string(),
            args,
        }
    } else {
        ChannelContent::Text(text.to_string())
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "chat_id".to_string(),
        serde_json::Value::String(chat_id.clone()),
    );
    metadata.insert(
        "message_id".to_string(),
        serde_json::Value::String(message_id.clone()),
    );
    metadata.insert(
        "chat_type".to_string(),
        serde_json::Value::String(chat_type.to_string()),
    );
    metadata.insert(
        "sender_id".to_string(),
        serde_json::Value::String(sender_id.clone()),
    );
    metadata.insert(
        "region".to_string(),
        serde_json::Value::String(channel_label.clone()),
    );
    // Check if the bot was @mentioned in group messages.
    // Feishu puts mention info in the mentions array; each mention has a "name"
    // field. The bot's own mention shows up with the key pattern "@_user_N".
    let was_mentioned = message
        .get("mentions")
        .and_then(|m| m.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);
    if let Some(mentions) = message.get("mentions") {
        metadata.insert("mentions".to_string(), mentions.clone());
    }
    metadata.insert(
        "was_mentioned".to_string(),
        serde_json::Value::Bool(was_mentioned),
    );

    Some(ChannelMessage {
        channel: ChannelType::Custom(channel_label),
        platform_message_id: message_id,
        sender: ChannelUser {
            platform_id: chat_id,
            display_name: sender_id,
            librefang_user: None,
        },
        content: msg_content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: root_id,
        metadata,
    })
}

/// Parse a Feishu/Lark v1 (legacy) webhook event.
fn parse_feishu_event_v1(body: &serde_json::Value, region: FeishuRegion) -> Option<ChannelMessage> {
    let event = body.get("event")?;
    let event_type = event["type"].as_str().unwrap_or("");
    if event_type != "message" {
        return None;
    }

    // V1 events don't have a `sender_type` field like v2.
    // However, if `open_id` is empty the event likely came from the bot itself
    // or is malformed — skip it to prevent potential echo loops.
    let open_id_check = event["open_id"].as_str().unwrap_or("");
    if open_id_check.is_empty() {
        return None;
    }

    let text = event["text"].as_str().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let open_id = event["open_id"].as_str().unwrap_or("").to_string();
    let chat_id = event["open_chat_id"].as_str().unwrap_or("").to_string();
    let msg_id = event["open_message_id"].as_str().unwrap_or("").to_string();
    let is_group = event["chat_type"].as_str().unwrap_or("") == "group";

    let channel_label = region.label().to_lowercase();

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
        ChannelContent::Text(text.to_string())
    };

    Some(ChannelMessage {
        channel: ChannelType::Custom(channel_label),
        platform_message_id: msg_id,
        sender: ChannelUser {
            platform_id: chat_id,
            display_name: open_id,
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

// ---------------------------------------------------------------------------
// ChannelAdapter impl
// ---------------------------------------------------------------------------

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        match self.region {
            FeishuRegion::Cn => "feishu",
            FeishuRegion::Intl => "lark",
        }
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom(self.name().to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let label = self.region.label();

        // Validate credentials
        let bot_name = self.validate().await?;
        info!("{label} adapter authenticated as {bot_name}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        match self.receive_mode {
            FeishuReceiveMode::Webhook => {
                info!(
                    "{label}: starting in webhook receive mode (port {})",
                    self.webhook_port
                );
                self.start_webhook(tx);
            }
            FeishuReceiveMode::Websocket => {
                info!("{label}: starting in WebSocket receive mode");
                self.start_websocket(tx);
            }
        }

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&user.platform_id, "chat_id", &text)
                    .await?;
            }
            _ => {
                self.api_send_message(&user.platform_id, "chat_id", "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Feishu/Lark does not support typing indicators via REST API
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};

    #[test]
    fn test_feishu_adapter_creation() {
        let adapter = FeishuAdapter::new(
            "cli_abc123".to_string(),
            "app-secret-456".to_string(),
            9000,
            FeishuRegion::Cn,
            FeishuReceiveMode::Websocket,
        );
        assert_eq!(adapter.name(), "feishu");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("feishu".to_string())
        );
        assert_eq!(adapter.webhook_port, 9000);
        assert_eq!(adapter.region, FeishuRegion::Cn);
        assert_eq!(adapter.receive_mode, FeishuReceiveMode::Websocket);
    }

    #[test]
    fn test_lark_adapter_creation() {
        let adapter = FeishuAdapter::new(
            "cli_abc123".to_string(),
            "app-secret-456".to_string(),
            9000,
            FeishuRegion::Intl,
            FeishuReceiveMode::Webhook,
        );
        assert_eq!(adapter.name(), "lark");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("lark".to_string())
        );
        assert_eq!(adapter.region, FeishuRegion::Intl);
        assert_eq!(adapter.receive_mode, FeishuReceiveMode::Webhook);
    }

    #[test]
    fn test_feishu_with_verification() {
        let adapter = FeishuAdapter::new(
            "cli_abc123".to_string(),
            "secret".to_string(),
            9000,
            FeishuRegion::Cn,
            FeishuReceiveMode::Webhook,
        )
        .with_verification(
            Some("verify-token".to_string()),
            Some("encrypt-key".to_string()),
        );
        assert_eq!(adapter.verification_token, Some("verify-token".to_string()));
        assert_eq!(adapter.encrypt_key, Some("encrypt-key".to_string()));
    }

    fn encrypt_test_payload(payload: &serde_json::Value, encrypt_key: &str) -> String {
        let key = Sha256::digest(encrypt_key.as_bytes());
        let iv = [0x42; 16];
        let plaintext = serde_json::to_vec(payload).unwrap();
        let mut buffer = vec![0u8; plaintext.len() + 16];
        buffer[..plaintext.len()].copy_from_slice(&plaintext);
        let ciphertext = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .unwrap()
            .to_vec();

        let mut raw = iv.to_vec();
        raw.extend(ciphertext);
        base64::engine::general_purpose::STANDARD.encode(raw)
    }

    #[test]
    fn test_decrypt_feishu_payload_if_needed_plain_passthrough() {
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1"
            }
        });
        let output = decrypt_feishu_payload_if_needed(&payload, None).unwrap();
        assert_eq!(output, payload);
    }

    #[test]
    fn test_decrypt_feishu_payload_if_needed_success() {
        let expected = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"hello\"}"
                }
            }
        });
        let encrypted = encrypt_test_payload(&expected, "my-feishu-encrypt-key");
        let payload = serde_json::json!({ "encrypt": encrypted });

        let output =
            decrypt_feishu_payload_if_needed(&payload, Some("my-feishu-encrypt-key")).unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn test_decrypt_feishu_payload_if_needed_missing_key_rejected() {
        let payload = serde_json::json!({ "encrypt": "ZmFrZQ==" });
        assert!(decrypt_feishu_payload_if_needed(&payload, None).is_err());
    }

    #[test]
    fn test_feishu_app_id_stored() {
        let adapter = FeishuAdapter::new(
            "cli_test".to_string(),
            "secret".to_string(),
            8080,
            FeishuRegion::Cn,
            FeishuReceiveMode::Websocket,
        );
        assert_eq!(adapter.app_id, "cli_test");
    }

    #[test]
    fn test_region_api_base() {
        assert_eq!(FeishuRegion::Cn.api_base(), "https://open.feishu.cn");
        assert_eq!(FeishuRegion::Intl.api_base(), "https://open.larksuite.com");
    }

    #[test]
    fn test_region_ws_endpoint_url() {
        assert_eq!(
            format!("{}/callback/ws/endpoint", FeishuRegion::Cn.api_base()),
            "https://open.feishu.cn/callback/ws/endpoint"
        );
        assert_eq!(
            format!("{}/callback/ws/endpoint", FeishuRegion::Intl.api_base()),
            "https://open.larksuite.com/callback/ws/endpoint"
        );
    }

    #[test]
    fn test_region_detect_from_domain() {
        assert_eq!(
            FeishuRegion::detect("cli_abc", Some("open.feishu.cn")),
            FeishuRegion::Cn
        );
        assert_eq!(
            FeishuRegion::detect("cli_abc", Some("open.larksuite.com")),
            FeishuRegion::Intl
        );
        assert_eq!(FeishuRegion::detect("cli_abc", None), FeishuRegion::Cn);
    }

    #[test]
    fn test_region_default_is_cn() {
        assert_eq!(FeishuRegion::default(), FeishuRegion::Cn);
    }

    #[test]
    fn test_receive_mode_default_is_websocket() {
        assert_eq!(FeishuReceiveMode::default(), FeishuReceiveMode::Websocket);
    }

    #[test]
    fn test_region_urls() {
        let adapter = FeishuAdapter::new(
            "cli_test".to_string(),
            "secret".to_string(),
            8080,
            FeishuRegion::Cn,
            FeishuReceiveMode::Websocket,
        );
        assert!(adapter.token_url().contains("feishu.cn"));
        assert!(adapter.send_url().contains("feishu.cn"));
        assert!(adapter.bot_info_url().contains("feishu.cn"));

        let adapter_intl = FeishuAdapter::new(
            "cli_test".to_string(),
            "secret".to_string(),
            8080,
            FeishuRegion::Intl,
            FeishuReceiveMode::Websocket,
        );
        assert!(adapter_intl.token_url().contains("larksuite.com"));
        assert!(adapter_intl.send_url().contains("larksuite.com"));
        assert!(adapter_intl.bot_info_url().contains("larksuite.com"));
    }

    #[test]
    fn test_parse_feishu_event_v2_text() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-001",
                "event_type": "im.message.receive_v1",
                "create_time": "1234567890000",
                "token": "verify-token",
                "app_id": "cli_abc123",
                "tenant_key": "tenant-key-1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_abc123",
                        "user_id": "user-1"
                    },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_abc123",
                    "root_id": null,
                    "chat_id": "oc_chat123",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"Hello from Feishu!\"}"
                }
            }
        });

        let msg = parse_feishu_event(&event, FeishuRegion::Cn).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("feishu".to_string()));
        assert_eq!(msg.platform_message_id, "om_abc123");
        assert!(!msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from Feishu!"));
        // Verify region metadata
        assert_eq!(
            msg.metadata.get("region"),
            Some(&serde_json::Value::String("feishu".to_string()))
        );
    }

    #[test]
    fn test_parse_lark_event_v2_text() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-001",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_abc123" },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_abc123",
                    "chat_id": "oc_chat123",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"Hello from Lark!\"}"
                }
            }
        });

        let msg = parse_feishu_event(&event, FeishuRegion::Intl).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("lark".to_string()));
        assert_eq!(
            msg.metadata.get("region"),
            Some(&serde_json::Value::String("lark".to_string()))
        );
    }

    #[test]
    fn test_parse_feishu_event_group_message() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-002",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_abc123" },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_grp1",
                    "chat_id": "oc_grp123",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"Group message\"}"
                }
            }
        });

        let msg = parse_feishu_event(&event, FeishuRegion::Cn).unwrap();
        assert!(msg.is_group);
    }

    #[test]
    fn test_parse_feishu_event_command() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-003",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_abc123" },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_cmd1",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"/help all\"}"
                }
            }
        });

        let msg = parse_feishu_event(&event, FeishuRegion::Cn).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "help");
                assert_eq!(args, &["all"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_feishu_event_skips_bot() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-004",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_bot" },
                    "sender_type": "bot"
                },
                "message": {
                    "message_id": "om_bot1",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"Bot message\"}"
                }
            }
        });

        assert!(parse_feishu_event(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_feishu_event_non_text() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-005",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_user1" },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_img1",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_type": "image",
                    "content": "{\"image_key\":\"img_v2_abc123\"}"
                }
            }
        });

        assert!(parse_feishu_event(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_feishu_event_wrong_type() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-006",
                "event_type": "im.chat.member_bot.added_v1"
            },
            "event": {}
        });

        assert!(parse_feishu_event(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_feishu_event_thread_id() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-007",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_user1" },
                    "sender_type": "user"
                },
                "message": {
                    "message_id": "om_thread1",
                    "root_id": "om_root1",
                    "chat_id": "oc_chat1",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"Thread reply\"}"
                }
            }
        });

        let msg = parse_feishu_event(&event, FeishuRegion::Cn).unwrap();
        assert_eq!(msg.thread_id, Some("om_root1".to_string()));
    }

    #[test]
    fn test_parse_feishu_event_v1_legacy() {
        let body = serde_json::json!({
            "event": {
                "type": "message",
                "text": "Hello legacy",
                "open_id": "ou_user1",
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_legacy1",
                "chat_type": "p2p"
            }
        });

        let msg = parse_feishu_event_v1(&body, FeishuRegion::Cn).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("feishu".to_string()));
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello legacy"));
    }

    #[test]
    fn test_parse_feishu_event_v1_empty_text() {
        let body = serde_json::json!({
            "event": {
                "type": "message",
                "text": "",
                "open_id": "ou_user1",
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_empty",
                "chat_type": "p2p"
            }
        });

        assert!(parse_feishu_event_v1(&body, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_feishu_event_v1_empty_open_id() {
        let body = serde_json::json!({
            "event": {
                "type": "message",
                "text": "should be skipped",
                "open_id": "",
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_echo",
                "chat_type": "p2p"
            }
        });

        assert!(parse_feishu_event_v1(&body, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_region_serde_roundtrip() {
        let cn: FeishuRegion = serde_json::from_str("\"cn\"").unwrap();
        assert_eq!(cn, FeishuRegion::Cn);
        let intl: FeishuRegion = serde_json::from_str("\"intl\"").unwrap();
        assert_eq!(intl, FeishuRegion::Intl);

        assert_eq!(serde_json::to_string(&FeishuRegion::Cn).unwrap(), "\"cn\"");
        assert_eq!(
            serde_json::to_string(&FeishuRegion::Intl).unwrap(),
            "\"intl\""
        );
    }

    #[test]
    fn test_receive_mode_serde_roundtrip() {
        let ws: FeishuReceiveMode = serde_json::from_str("\"websocket\"").unwrap();
        assert_eq!(ws, FeishuReceiveMode::Websocket);
        let wh: FeishuReceiveMode = serde_json::from_str("\"webhook\"").unwrap();
        assert_eq!(wh, FeishuReceiveMode::Webhook);
    }

    // -----------------------------------------------------------------------
    // Interactive card tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_approval_card_structure() {
        let card = build_approval_card(
            "abc-123",
            "agent-1",
            "shell_exec",
            "rm -rf /tmp/cache",
            "critical",
        );

        // Header should have red template for critical risk
        assert_eq!(card["header"]["template"], "red");
        assert!(card["header"]["title"]["content"]
            .as_str()
            .unwrap()
            .contains("critical"));

        // Should have elements array
        let elements = card["elements"].as_array().unwrap();
        assert!(!elements.is_empty());

        // Last element should be the action buttons
        let action_element = elements.last().unwrap();
        assert_eq!(action_element["tag"], "action");

        let actions = action_element["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);

        // Approve button
        assert_eq!(actions[0]["text"]["content"], "Approve");
        assert_eq!(actions[0]["type"], "primary");
        assert_eq!(actions[0]["value"]["action"], "approve");
        assert_eq!(actions[0]["value"]["request_id"], "abc-123");

        // Deny button
        assert_eq!(actions[1]["text"]["content"], "Deny");
        assert_eq!(actions[1]["type"], "danger");
        assert_eq!(actions[1]["value"]["action"], "reject");
        assert_eq!(actions[1]["value"]["request_id"], "abc-123");
    }

    #[test]
    fn test_build_approval_card_risk_colors() {
        let critical = build_approval_card("id", "a", "t", "s", "critical");
        assert_eq!(critical["header"]["template"], "red");

        let high = build_approval_card("id", "a", "t", "s", "high");
        assert_eq!(high["header"]["template"], "orange");

        let medium = build_approval_card("id", "a", "t", "s", "medium");
        assert_eq!(medium["header"]["template"], "yellow");

        let low = build_approval_card("id", "a", "t", "s", "low");
        assert_eq!(low["header"]["template"], "blue");
    }

    #[test]
    fn test_build_approval_card_fields_displayed() {
        let card = build_approval_card(
            "req-456",
            "my-agent",
            "file_write",
            "write /etc/config",
            "high",
        );

        let card_str = card.to_string();
        assert!(card_str.contains("my-agent"));
        assert!(card_str.contains("file_write"));
        assert!(card_str.contains("write /etc/config"));
        assert!(card_str.contains("req-456"));
    }

    #[test]
    fn test_parse_card_action_approve() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-001",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_user1"
                },
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_card1",
                "action": {
                    "value": {
                        "action": "approve",
                        "request_id": "abc-123-def"
                    },
                    "tag": "button"
                }
            }
        });

        let msg = parse_card_action(&event, FeishuRegion::Cn).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("feishu".to_string()));
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "approve");
                assert_eq!(args, &["abc-123-def"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
        assert_eq!(msg.sender.display_name, "ou_user1");
        assert_eq!(msg.sender.platform_id, "ou_user1");
        assert_eq!(
            msg.metadata.get("card_action"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn test_parse_card_action_lark_region() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-intl",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": { "open_id": "ou_user1" },
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_card_intl",
                "action": {
                    "value": { "action": "approve", "request_id": "intl-1" },
                    "tag": "button"
                }
            }
        });

        let msg = parse_card_action(&event, FeishuRegion::Intl).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("lark".to_string()));
    }

    #[test]
    fn test_parse_card_action_reject() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-002",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_admin"
                },
                "open_chat_id": "oc_chat2",
                "open_message_id": "om_card2",
                "action": {
                    "value": {
                        "action": "reject",
                        "request_id": "xyz-789"
                    },
                    "tag": "button"
                }
            }
        });

        let msg = parse_card_action(&event, FeishuRegion::Cn).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "reject");
                assert_eq!(args, &["xyz-789"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_card_action_unknown_action_returns_none() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-003",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_user1"
                },
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_card3",
                "action": {
                    "value": {
                        "action": "unknown_action",
                        "request_id": "abc-123"
                    },
                    "tag": "button"
                }
            }
        });

        assert!(parse_card_action(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_card_action_missing_value_returns_none() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-004",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_user1"
                },
                "action": {
                    "tag": "button"
                }
            }
        });

        assert!(parse_card_action(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_card_action_wrong_event_type_returns_none() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-006",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "operator": {
                    "open_id": "ou_user1"
                },
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_card6",
                "action": {
                    "value": {
                        "action": "approve",
                        "request_id": "abc-123"
                    },
                    "tag": "button"
                }
            }
        });

        assert!(parse_card_action(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_card_action_missing_header_returns_none() {
        let event = serde_json::json!({
            "schema": "2.0",
            "event": {
                "operator": {
                    "open_id": "ou_user1"
                },
                "open_chat_id": "oc_chat1",
                "open_message_id": "om_card7",
                "action": {
                    "value": {
                        "action": "approve",
                        "request_id": "abc-123"
                    },
                    "tag": "button"
                }
            }
        });

        assert!(parse_card_action(&event, FeishuRegion::Cn).is_none());
    }

    #[test]
    fn test_parse_card_action_missing_operator_returns_none() {
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt-card-005",
                "event_type": "card.action.trigger"
            },
            "event": {
                "action": {
                    "value": {
                        "action": "approve",
                        "request_id": "abc-123"
                    },
                    "tag": "button"
                }
            }
        });

        assert!(parse_card_action(&event, FeishuRegion::Cn).is_none());
    }

    // ── Event dedup tests ──────────────────────────────────────────────

    #[test]
    fn test_dedup_first_event_passes() {
        let seen = Mutex::new(HashMap::new());
        let payload = serde_json::json!({
            "header": { "event_id": "evt-100", "event_type": "im.message.receive_v1" },
            "event": {}
        });
        assert!(!is_duplicate_event(&payload, &seen));
    }

    #[test]
    fn test_dedup_same_event_blocked() {
        let seen = Mutex::new(HashMap::new());
        let payload = serde_json::json!({
            "header": { "event_id": "evt-200", "event_type": "im.message.receive_v1" },
            "event": {}
        });
        assert!(!is_duplicate_event(&payload, &seen));
        assert!(is_duplicate_event(&payload, &seen)); // second time = duplicate
    }

    #[test]
    fn test_dedup_different_events_pass() {
        let seen = Mutex::new(HashMap::new());
        let p1 = serde_json::json!({ "header": { "event_id": "evt-a" } });
        let p2 = serde_json::json!({ "header": { "event_id": "evt-b" } });
        assert!(!is_duplicate_event(&p1, &seen));
        assert!(!is_duplicate_event(&p2, &seen));
    }

    #[test]
    fn test_dedup_no_header_passes() {
        let seen = Mutex::new(HashMap::new());
        let payload = serde_json::json!({ "challenge": "test" });
        assert!(!is_duplicate_event(&payload, &seen));
        assert!(!is_duplicate_event(&payload, &seen)); // still passes
    }

    #[test]
    fn test_dedup_no_event_id_passes() {
        let seen = Mutex::new(HashMap::new());
        let payload = serde_json::json!({ "header": { "event_type": "foo" } });
        assert!(!is_duplicate_event(&payload, &seen));
    }
}
