//! WeChat personal account adapter via the iLink protocol.
//!
//! Connects to Tencent's official iLink API (`ilinkai.weixin.qq.com`) used by
//! the WeChat ClawBot plugin. Supports QR code login and long-polling for
//! real-time message delivery.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType,
    ChannelUser,
};
use async_trait::async_trait;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

/// iLink API base URL.
const ILINK_BASE: &str = "https://ilinkai.weixin.qq.com";

// Backoff durations are now configurable via WeChatConfig.
/// Maximum message length for WeChat text messages.
const MAX_MESSAGE_LEN: usize = 4096;
/// Channel version sent in base_info.
const CHANNEL_VERSION: &str = "1.0.2";

/// iLink item type: text.
const ITEM_TYPE_TEXT: u32 = 1;
/// iLink item type: image.
const ITEM_TYPE_IMAGE: u32 = 2;
/// iLink item type: voice.
const ITEM_TYPE_VOICE: u32 = 3;
/// iLink item type: file.
const ITEM_TYPE_FILE: u32 = 4;
/// iLink item type: video.
const ITEM_TYPE_VIDEO: u32 = 5;

/// Maximum duration to wait for QR code scan before giving up.
const QR_LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// WeChat personal account adapter using the iLink protocol.
pub struct WeChatAdapter {
    /// Bot token obtained from QR login (or persisted from config).
    bot_token: Arc<RwLock<Option<Zeroizing<String>>>>,
    /// HTTP client for iLink API calls.
    client: reqwest::Client,
    /// Allowed user IDs (empty = allow all). Format: `{hash}@im.wechat`.
    allowed_users: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Initial backoff on API failures.
    initial_backoff: Duration,
    /// Maximum backoff on API failures.
    max_backoff: Duration,
    /// Cursor for long-polling getupdates.
    updates_cursor: Arc<RwLock<String>>,
    /// Typing ticket from getconfig.
    typing_ticket: Arc<RwLock<Option<String>>>,
    /// Most recent context_token per user, for reply association.
    user_context_tokens: Arc<RwLock<HashMap<String, String>>>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Whether the adapter is currently connected and polling.
    connected: Arc<AtomicBool>,
    /// Total messages received since start.
    messages_received: Arc<AtomicU64>,
    /// Total messages sent since start.
    messages_sent: Arc<AtomicU64>,
    /// When the adapter was started.
    started_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    /// Last error message.
    last_error: Arc<RwLock<Option<String>>>,
    /// X-WECHAT-UIN header value, generated on construction.
    wechat_uin: String,
    /// REST base URL for the iLink API. Defaults to `ILINK_BASE`; tests
    /// override via `with_base_url` so the adapter talks to a local
    /// wiremock instead.
    api_base: String,
}

/// Generate a random UIN for the X-WECHAT-UIN header.
///
/// The spec calls for `base64(String(randomUint32()))`.
/// Uses UUID v4 random bytes to derive a u32, avoiding weak time-based PRNG.
fn generate_wechat_uin() -> String {
    use base64::Engine;
    let uuid_bytes = uuid::Uuid::new_v4();
    let bytes = uuid_bytes.as_bytes();
    let random_u32 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let num_str = random_u32.to_string();
    base64::engine::general_purpose::STANDARD.encode(num_str.as_bytes())
}

impl WeChatAdapter {
    /// Create a new WeChat adapter.
    ///
    /// If `bot_token` is provided (from a previous session), the adapter will
    /// skip the QR login flow and use it directly. Otherwise, QR login is
    /// performed during `start()`.
    pub fn new(bot_token: Option<String>, allowed_users: Vec<String>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot_token: Arc::new(RwLock::new(bot_token.map(Zeroizing::new))),
            client: crate::http_client::client_builder()
                .timeout(Duration::from_secs(90))
                .build()
                .expect("failed to build HTTP client"),
            allowed_users,
            account_id: None,
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(60),
            updates_cursor: Arc::new(RwLock::new(String::new())),
            typing_ticket: Arc::new(RwLock::new(None)),
            user_context_tokens: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            connected: Arc::new(AtomicBool::new(false)),
            messages_received: Arc::new(AtomicU64::new(0)),
            messages_sent: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            wechat_uin: generate_wechat_uin(),
            api_base: ILINK_BASE.to_string(),
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the iLink REST base URL. `#[cfg(test)]`-only — used by
    /// wiremock-driven tests to point the adapter at a local mock server.
    #[cfg(test)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.api_base = url;
        self
    }

    /// Set backoff configuration. Returns self for builder chaining.
    pub fn with_backoff(mut self, initial_backoff_secs: u64, max_backoff_secs: u64) -> Self {
        self.initial_backoff = Duration::from_secs(initial_backoff_secs);
        self.max_backoff = Duration::from_secs(max_backoff_secs);
        self
    }

    /// Build common iLink request headers.
    fn ilink_headers(&self, token: &str) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        headers.insert("AuthorizationType", "ilink_bot_token".parse().unwrap());
        headers.insert("X-WECHAT-UIN", self.wechat_uin.parse().unwrap());
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token).parse().unwrap(),
        );
        headers
    }

    /// Perform QR code login flow.
    ///
    /// 1. GET /ilink/bot/get_bot_qrcode?bot_type=3 to get a QR code
    /// 2. Poll /ilink/bot/get_qrcode_status until confirmed
    /// 3. Returns the bot_token
    async fn qr_login(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        info!("WeChat: starting QR code login flow");

        // Step 1: Get QR code
        let qr_url = format!("{}/ilink/bot/get_bot_qrcode?bot_type=3", self.api_base);
        let resp = self.client.get(&qr_url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("WeChat QR code request failed ({status}): {body}").into());
        }
        let qr_data: serde_json::Value = resp.json().await?;
        let qrcode = qr_data["qrcode"]
            .as_str()
            .ok_or("WeChat: missing 'qrcode' in response")?
            .to_string();

        info!("WeChat: QR code ready — scan with your WeChat app to log in");
        debug!("WeChat: qrcode={}", qrcode);

        // Step 2: Poll for confirmation (with overall timeout)
        let mut backoff = self.initial_backoff;
        let deadline = tokio::time::Instant::now() + QR_LOGIN_TIMEOUT;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let encoded_qr: String = url::form_urlencoded::byte_serialize(qrcode.as_bytes()).collect();
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err("WeChat: QR login timed out (5 minutes) — restart to try again".into());
            }

            let status_url = format!(
                "{}/ilink/bot/get_qrcode_status?qrcode={}",
                self.api_base, encoded_qr
            );
            let resp = self.client.get(&status_url).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await?;
                    let status = body["status"].as_str().unwrap_or("");
                    match status {
                        "confirmed" => {
                            let token = body["bot_token"]
                                .as_str()
                                .ok_or("WeChat: missing bot_token after QR confirmation")?
                                .to_string();
                            info!("WeChat: QR login successful");
                            return Ok(token);
                        }
                        "expired" => {
                            return Err("WeChat: QR code expired — restart to try again".into());
                        }
                        _ => {
                            debug!("WeChat: QR status={}, waiting...", status);
                        }
                    }
                }
                Ok(r) => {
                    let status = r.status();
                    warn!("WeChat: QR status poll failed ({status}), retrying...");
                }
                Err(e) => {
                    warn!("WeChat: QR status poll error: {e}, retrying...");
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = shutdown_rx.changed() => {
                    return Err("WeChat: QR login cancelled by shutdown".into());
                }
            }
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }

    /// Fetch the typing ticket from getconfig.
    ///
    /// This helper is called from the spawn'd long-poll task in `start()`,
    /// so it cannot reach `self.api_base`. It uses the `ILINK_BASE` const
    /// directly. Tests that need to redirect this endpoint would have to
    /// either change the helper's signature to take a base URL, or stand
    /// up the long-poll loop end-to-end; this PR does neither because
    /// `getconfig` is on the inbound path, not the `send()` path.
    async fn refresh_typing_ticket(
        client: &reqwest::Client,
        headers: &reqwest::header::HeaderMap,
    ) -> Option<String> {
        let url = format!("{}/ilink/bot/getconfig", ILINK_BASE);
        match client
            .post(&url)
            .headers(headers.clone())
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.ok()?;
                body["typing_ticket"].as_str().map(|s| s.to_string())
            }
            Ok(resp) => {
                warn!("WeChat: getconfig failed ({})", resp.status());
                None
            }
            Err(e) => {
                warn!("WeChat: getconfig error: {e}");
                None
            }
        }
    }

    /// Long-poll for new messages via getupdates.
    ///
    /// Same constraint as `refresh_typing_ticket`: this helper is called
    /// from the spawn'd long-poll loop and cannot reach `self.api_base`,
    /// so it uses `ILINK_BASE` directly. Inbound path; not covered by
    /// this PR's `send()`-path tests.
    async fn poll_updates(
        client: &reqwest::Client,
        headers: &reqwest::header::HeaderMap,
        cursor: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/ilink/bot/getupdates", ILINK_BASE);
        let body = serde_json::json!({
            "get_updates_buf": cursor,
            "base_info": {
                "channel_version": CHANNEL_VERSION,
            }
        });
        let resp = client
            .post(&url)
            .headers(headers.clone())
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("WeChat getupdates failed ({status}): {text}").into());
        }
        let data: serde_json::Value = resp.json().await?;
        Ok(data)
    }

    /// Send a text message via iLink sendmessage.
    async fn ilink_send_text(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token_guard = self.bot_token.read().await;
        let token = token_guard.as_ref().ok_or("WeChat: not logged in")?;

        debug!("WeChat ilink_send_text: to={}, context_token_len={}, text_len={}, bot_token_prefix={}...",
            to_user_id, context_token.len(), text.len(),
            &token.as_str().chars().take(10).collect::<String>());

        let headers = self.ilink_headers(token.as_str());
        let url = format!("{}/ilink/bot/sendmessage", self.api_base);

        // Split long messages
        let chunks = split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let client_id = uuid::Uuid::new_v4().to_string();
            let body = serde_json::json!({
                "msg": {
                    "from_user_id": "",
                    "to_user_id": to_user_id,
                    "client_id": client_id,
                    "message_type": 2,
                    "message_state": 2,
                    "context_token": context_token,
                    "item_list": [{
                        "type": ITEM_TYPE_TEXT,
                        "text_item": {
                            "text": chunk,
                        }
                    }]
                },
                "base_info": {
                    "channel_version": CHANNEL_VERSION,
                }
            });

            debug!(
                "WeChat sendmessage request: {}",
                serde_json::to_string(&body).unwrap_or_default()
            );

            let resp = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            let resp_text = resp.text().await.unwrap_or_default();
            debug!("WeChat sendmessage response ({status}): {resp_text}");
            if !status.is_success() {
                error!("WeChat sendmessage failed ({status}): {resp_text}");
                return Err(format!("WeChat sendmessage error {status}: {resp_text}").into());
            }
        }

        self.messages_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Check if a user is allowed.
    #[allow(dead_code)]
    fn is_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == user_id)
    }

    /// Parse an iLink message into a ChannelMessage.
    fn parse_message(msg: &serde_json::Value, account_id: Option<&str>) -> Option<ChannelMessage> {
        let from_user_id = msg["from_user_id"].as_str()?.to_string();
        let to_user_id = msg["to_user_id"].as_str().unwrap_or_default();
        let context_token = msg["context_token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let message_type = msg["message_type"].as_u64().unwrap_or(0);

        // Skip bot-originated messages (from @im.bot addresses)
        if from_user_id.ends_with("@im.bot") {
            return None;
        }

        let items = msg["item_list"].as_array()?;
        if items.is_empty() {
            return None;
        }

        // Parse first item to determine content type
        let item = &items[0];
        let item_type = item["type"].as_u64().unwrap_or(0) as u32;

        let content = match item_type {
            ITEM_TYPE_TEXT => {
                let text = item["text_item"]["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return None;
                }
                ChannelContent::Text(text.to_string())
            }
            ITEM_TYPE_IMAGE => {
                let url = item["image_item"]["url"]
                    .as_str()
                    .or_else(|| item["image_item"]["cdn_url"].as_str())
                    .unwrap_or_default()
                    .to_string();
                ChannelContent::Image {
                    url,
                    caption: None,
                    mime_type: Some("image/jpeg".to_string()),
                }
            }
            ITEM_TYPE_VOICE => {
                let url = item["voice_item"]["url"]
                    .as_str()
                    .or_else(|| item["voice_item"]["cdn_url"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let duration = item["voice_item"]["duration"].as_u64().unwrap_or(0) as u32;
                ChannelContent::Voice {
                    url,
                    caption: None,
                    duration_seconds: duration,
                }
            }
            ITEM_TYPE_FILE => {
                let url = item["file_item"]["url"]
                    .as_str()
                    .or_else(|| item["file_item"]["cdn_url"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let filename = item["file_item"]["file_name"]
                    .as_str()
                    .unwrap_or("file")
                    .to_string();
                ChannelContent::File { url, filename }
            }
            ITEM_TYPE_VIDEO => {
                let url = item["video_item"]["url"]
                    .as_str()
                    .or_else(|| item["video_item"]["cdn_url"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let duration = item["video_item"]["duration"].as_u64().unwrap_or(0) as u32;
                ChannelContent::Video {
                    url,
                    caption: None,
                    duration_seconds: duration,
                    filename: None,
                }
            }
            _ => {
                debug!("WeChat: unsupported item type {item_type}, skipping");
                return None;
            }
        };

        // Build a stable message ID from available fields
        let msg_id = msg["msg_id"]
            .as_str()
            .or_else(|| msg["svr_msg_id"].as_str())
            .unwrap_or_default()
            .to_string();
        let platform_message_id = if msg_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            msg_id
        };

        // Extract display name from user info if available
        let display_name = msg["from_user_name"]
            .as_str()
            .or_else(|| msg["from_user_nick"].as_str())
            .unwrap_or(&from_user_id)
            .to_string();

        let mut metadata = HashMap::new();
        metadata.insert(
            "context_token".to_string(),
            serde_json::Value::String(context_token),
        );
        metadata.insert(
            "to_user_id".to_string(),
            serde_json::Value::String(to_user_id.to_string()),
        );
        metadata.insert(
            "message_type".to_string(),
            serde_json::Value::Number(serde_json::Number::from(message_type)),
        );
        if let Some(account_id) = account_id {
            metadata.insert(
                "account_id".to_string(),
                serde_json::Value::String(account_id.to_string()),
            );
        }

        Some(ChannelMessage {
            channel: ChannelType::WeChat,
            platform_message_id,
            sender: ChannelUser {
                platform_id: from_user_id,
                display_name,
                librefang_user: None,
            },
            content,
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: None,
            metadata,
        })
    }
}

#[async_trait]
impl ChannelAdapter for WeChatAdapter {
    fn name(&self) -> &str {
        "wechat"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::WeChat
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Login flow: use persisted token or perform QR login
        {
            let has_token = self.bot_token.read().await.is_some();
            if !has_token {
                let token = self.qr_login().await?;
                *self.bot_token.write().await = Some(Zeroizing::new(token));
            } else {
                info!("WeChat: using persisted bot token (skipping QR login)");
            }
        }

        // Fetch typing ticket for send_typing support
        {
            let token_guard = self.bot_token.read().await;
            if let Some(ref token) = *token_guard {
                let headers = self.ilink_headers(token.as_str());
                let ticket = Self::refresh_typing_ticket(&self.client, &headers).await;
                *self.typing_ticket.write().await = ticket;
            }
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let client = self.client.clone();
        let bot_token = self.bot_token.clone();
        let cursor = self.updates_cursor.clone();
        let typing_ticket = self.typing_ticket.clone();
        let allowed_users = self.allowed_users.clone();
        let connected = self.connected.clone();
        let messages_received = self.messages_received.clone();
        let last_error = self.last_error.clone();
        let user_context_tokens = self.user_context_tokens.clone();
        let account_id = self.account_id.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let wechat_uin = self.wechat_uin.clone();
        let initial_backoff = self.initial_backoff;
        let max_backoff = self.max_backoff;

        *self.started_at.write().await = Some(chrono::Utc::now());
        self.connected.store(true, Ordering::Relaxed);

        info!("WeChat: starting message polling loop");

        tokio::spawn(async move {
            let mut backoff = initial_backoff;

            loop {
                // Check for shutdown
                if *shutdown_rx.borrow() {
                    break;
                }

                // Build headers from current token
                let token_guard = bot_token.read().await;
                let token = match token_guard.as_ref() {
                    Some(t) => t.clone(),
                    None => {
                        error!("WeChat: bot_token missing during poll loop");
                        break;
                    }
                };
                drop(token_guard);

                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::CONTENT_TYPE,
                    "application/json".parse().unwrap(),
                );
                headers.insert("AuthorizationType", "ilink_bot_token".parse().unwrap());
                headers.insert("X-WECHAT-UIN", wechat_uin.parse().unwrap());
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", token.as_str()).parse().unwrap(),
                );

                let current_cursor = cursor.read().await.clone();
                let poll_result = Self::poll_updates(&client, &headers, &current_cursor).await;

                match poll_result {
                    Ok(data) => {
                        backoff = initial_backoff;
                        connected.store(true, Ordering::Relaxed);

                        // Update cursor for next poll
                        if let Some(new_cursor) = data["get_updates_buf"].as_str() {
                            *cursor.write().await = new_cursor.to_string();
                        }

                        // Refresh typing ticket periodically
                        if let Some(ticket) = data["typing_ticket"].as_str() {
                            *typing_ticket.write().await = Some(ticket.to_string());
                        }

                        // Process messages
                        if let Some(msgs) = data["msgs"].as_array() {
                            debug!("WeChat: poll returned {} message(s)", msgs.len());
                            for msg in msgs {
                                debug!(
                                    "WeChat: raw msg: {}",
                                    serde_json::to_string(msg).unwrap_or_default()
                                );
                                if let Some(channel_msg) =
                                    Self::parse_message(msg, account_id.as_deref())
                                {
                                    // Filter by allowed users
                                    if !allowed_users.is_empty()
                                        && !allowed_users
                                            .iter()
                                            .any(|u| u == &channel_msg.sender.platform_id)
                                    {
                                        debug!(
                                            "WeChat: ignoring message from non-allowed user {}",
                                            channel_msg.sender.platform_id
                                        );
                                        continue;
                                    }
                                    // Track latest context_token per user for reply association
                                    if let Some(ctx) = channel_msg
                                        .metadata
                                        .get("context_token")
                                        .and_then(|v| v.as_str())
                                    {
                                        user_context_tokens.write().await.insert(
                                            channel_msg.sender.platform_id.clone(),
                                            ctx.to_string(),
                                        );
                                    }
                                    messages_received.fetch_add(1, Ordering::Relaxed);
                                    if tx.send(channel_msg).await.is_err() {
                                        info!("WeChat: message channel closed, stopping poll");
                                        break;
                                    }
                                }
                            }
                        }

                        // Respect longpolling_timeout_ms (but don't block longer than
                        // the shutdown check interval)
                        let timeout_ms = data["longpolling_timeout_ms"].as_u64().unwrap_or(0);
                        if timeout_ms == 0 {
                            // No long-poll timeout means we should wait briefly
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                                _ = shutdown_rx.changed() => { break; }
                            }
                        }
                        // If timeout_ms > 0, the server held the connection for that long
                        // already, so we can immediately re-poll.
                    }
                    Err(e) => {
                        let err_msg = format!("{e}");
                        warn!("WeChat: poll error: {err_msg}");
                        *last_error.write().await = Some(err_msg);
                        connected.store(false, Ordering::Relaxed);

                        tokio::select! {
                            _ = tokio::time::sleep(backoff) => {}
                            _ = shutdown_rx.changed() => { break; }
                        }
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }

            connected.store(false, Ordering::Relaxed);
            info!("WeChat: polling loop stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("WeChat: send() called for user={}", user.platform_id);
        // Look up the most recent context_token for this user.
        // The polling loop tracks it per user_id for reply association.
        // For proactive messages (no prior inbound), falls back to empty.
        let context_token = self
            .user_context_tokens
            .read()
            .await
            .get(&user.platform_id)
            .cloned()
            .unwrap_or_default();

        debug!(
            "WeChat: context_token for user={}: {:?}",
            user.platform_id, context_token
        );

        match content {
            ChannelContent::Text(text) => {
                self.ilink_send_text(&user.platform_id, &context_token, &text)
                    .await?;
            }
            ChannelContent::Image { caption, .. } => {
                // For now, send caption as text (media upload requires CDN flow)
                let text = caption
                    .unwrap_or_else(|| "[Image — media upload not yet supported]".to_string());
                self.ilink_send_text(&user.platform_id, &context_token, &text)
                    .await?;
            }
            ChannelContent::File { filename, .. } => {
                let text = format!("[File: {filename} — media upload not yet supported]");
                self.ilink_send_text(&user.platform_id, &context_token, &text)
                    .await?;
            }
            _ => {
                self.ilink_send_text(
                    &user.platform_id,
                    &context_token,
                    "[Unsupported content type]",
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token_guard = self.bot_token.read().await;
        let token = match token_guard.as_ref() {
            Some(t) => t.clone(),
            None => return Ok(()), // Not logged in yet
        };
        drop(token_guard);

        let ticket = self.typing_ticket.read().await.clone();
        let ticket = match ticket {
            Some(t) => t,
            None => return Ok(()), // No typing ticket available
        };

        let headers = self.ilink_headers(token.as_str());
        let url = format!("{}/ilink/bot/sendtyping", self.api_base);
        let body = serde_json::json!({
            "to_user_id": user.platform_id,
            "typing_ticket": ticket,
        });

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) if !r.status().is_success() => {
                let status = r.status();
                debug!("WeChat: sendtyping failed ({status})");
            }
            Err(e) => {
                debug!("WeChat: sendtyping error: {e}");
            }
            _ => {}
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        self.connected.store(false, Ordering::Relaxed);
        info!("WeChat: adapter stopped");
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        let connected = self.connected.load(Ordering::Relaxed);
        let started_at = self.started_at.try_read().ok().and_then(|g| *g);
        let last_error = self.last_error.try_read().ok().and_then(|g| g.clone());

        ChannelStatus {
            connected,
            started_at,
            last_message_at: None,
            messages_received: self.messages_received.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            last_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wechat_adapter_creation() {
        let adapter = WeChatAdapter::new(Some("test_token".to_string()), vec![]);
        assert_eq!(adapter.name(), "wechat");
        assert_eq!(adapter.channel_type(), ChannelType::WeChat);
    }

    #[test]
    fn test_wechat_adapter_no_token() {
        let adapter = WeChatAdapter::new(None, vec!["user1@im.wechat".to_string()]);
        assert_eq!(adapter.name(), "wechat");
        assert!(adapter.is_allowed("user1@im.wechat"));
        assert!(!adapter.is_allowed("user2@im.wechat"));
    }

    #[test]
    fn test_wechat_adapter_allow_all() {
        let adapter = WeChatAdapter::new(None, vec![]);
        assert!(adapter.is_allowed("anyone@im.wechat"));
    }

    #[test]
    fn test_parse_text_message() {
        let msg = serde_json::json!({
            "from_user_id": "abc123@im.wechat",
            "to_user_id": "bot456@im.bot",
            "context_token": "ctx_789",
            "message_type": 2,
            "msg_id": "msg_001",
            "item_list": [{
                "type": 1,
                "text_item": {
                    "text": "Hello, world!"
                }
            }]
        });

        let result = WeChatAdapter::parse_message(&msg, None);
        assert!(result.is_some());
        let channel_msg = result.unwrap();
        assert_eq!(channel_msg.sender.platform_id, "abc123@im.wechat");
        assert_eq!(channel_msg.platform_message_id, "msg_001");
        match channel_msg.content {
            ChannelContent::Text(ref t) => assert_eq!(t, "Hello, world!"),
            _ => panic!("Expected Text content"),
        }
        assert_eq!(
            channel_msg
                .metadata
                .get("context_token")
                .and_then(|v| v.as_str()),
            Some("ctx_789")
        );
    }

    #[test]
    fn test_parse_image_message() {
        let msg = serde_json::json!({
            "from_user_id": "abc@im.wechat",
            "to_user_id": "bot@im.bot",
            "context_token": "ctx",
            "message_type": 2,
            "msg_id": "img_001",
            "item_list": [{
                "type": 2,
                "image_item": {
                    "url": "https://example.com/image.jpg"
                }
            }]
        });

        let result = WeChatAdapter::parse_message(&msg, None);
        assert!(result.is_some());
        match result.unwrap().content {
            ChannelContent::Image { ref url, .. } => {
                assert_eq!(url, "https://example.com/image.jpg");
            }
            _ => panic!("Expected Image content"),
        }
    }

    #[test]
    fn test_parse_bot_message_skipped() {
        let msg = serde_json::json!({
            "from_user_id": "bot@im.bot",
            "to_user_id": "user@im.wechat",
            "context_token": "ctx",
            "message_type": 2,
            "msg_id": "msg_bot",
            "item_list": [{
                "type": 1,
                "text_item": { "text": "echo" }
            }]
        });
        assert!(WeChatAdapter::parse_message(&msg, None).is_none());
    }

    #[test]
    fn test_parse_message_includes_account_id_metadata() {
        let msg = serde_json::json!({
            "from_user_id": "abc123@im.wechat",
            "to_user_id": "bot456@im.bot",
            "context_token": "ctx_789",
            "message_type": 2,
            "msg_id": "msg_001",
            "item_list": [{
                "type": 1,
                "text_item": {
                    "text": "Hello, world!"
                }
            }]
        });

        let result = WeChatAdapter::parse_message(&msg, Some("wechat-main"));
        assert_eq!(
            result
                .unwrap()
                .metadata
                .get("account_id")
                .and_then(|v| v.as_str()),
            Some("wechat-main")
        );
    }

    #[test]
    fn test_generate_wechat_uin() {
        let uin = generate_wechat_uin();
        // Should be valid base64
        assert!(!uin.is_empty());
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD.decode(&uin);
        assert!(decoded.is_ok());
    }

    // ----- send() path tests (issue #3820) -----
    //
    // The `ILINK_BASE` const is now backed by an `api_base` field (only
    // for the `send()` path — start / qrcode / getconfig / getupdates /
    // typing still reference the const directly, as they sit on
    // unrelated paths). A `#[cfg(test)] with_base_url` setter points the
    // adapter at a local `wiremock::MockServer`. Asserts the
    // `POST /ilink/bot/sendmessage` request shape — `Authorization`
    // header carries the bot token; the JSON envelope wraps an
    // `item_list` of text items keyed by the user's `context_token`.

    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn wechat_user(user_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    /// Custom responder: validates the JSON body shape and returns 200 on a
    /// successful match, otherwise records the body so the assertion failure
    /// surfaces a meaningful diff.
    struct AssertWeChatBody {
        expected_to: &'static str,
        expected_text: &'static str,
        expected_context_token: &'static str,
    }
    impl Respond for AssertWeChatBody {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value =
                serde_json::from_slice(&req.body).expect("body must be JSON");
            assert_eq!(
                body["msg"]["to_user_id"].as_str(),
                Some(self.expected_to),
                "to_user_id mismatch in body: {body}"
            );
            assert_eq!(
                body["msg"]["context_token"].as_str(),
                Some(self.expected_context_token),
                "context_token mismatch in body: {body}"
            );
            assert_eq!(
                body["msg"]["item_list"][0]["text_item"]["text"].as_str(),
                Some(self.expected_text),
                "text item mismatch in body: {body}"
            );
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn wechat_send_posts_sendmessage_with_seeded_context_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ilink/bot/sendmessage"))
            .and(header_exists("authorization"))
            .respond_with(AssertWeChatBody {
                expected_to: "user-42@im.wechat",
                expected_text: "hi wechat",
                expected_context_token: "ctx-abc",
            })
            .expect(1)
            .mount(&server)
            .await;

        let adapter =
            WeChatAdapter::new(Some("bot-tok".to_string()), vec![]).with_base_url(server.uri());
        // Seed the per-user context_token that `send()` looks up before
        // calling `ilink_send_text`. mod tests has access to the private
        // `Arc<RwLock<HashMap<...>>>` because it lives in the same module.
        adapter
            .user_context_tokens
            .write()
            .await
            .insert("user-42@im.wechat".to_string(), "ctx-abc".to_string());

        adapter
            .send(
                &wechat_user("user-42@im.wechat"),
                ChannelContent::Text("hi wechat".into()),
            )
            .await
            .expect("wechat send must succeed against mock");
    }

    #[tokio::test]
    async fn wechat_send_returns_err_when_not_logged_in() {
        // No bot_token + no mock server route — `ilink_send_text` should
        // bail with "WeChat: not logged in" before any HTTP call. We do
        // not even need a wiremock listener here, but using one lets us
        // also assert that no request is dispatched.
        let server = MockServer::start().await;
        let adapter = WeChatAdapter::new(None, vec![]).with_base_url(server.uri());
        let err = adapter
            .send(
                &wechat_user("user-42@im.wechat"),
                ChannelContent::Text("x".into()),
            )
            .await
            .expect_err("wechat send must error when bot_token is unset");
        assert!(
            err.to_string().to_lowercase().contains("not logged in"),
            "error should mention not-logged-in state, got: {err}"
        );
        let received = server
            .received_requests()
            .await
            .expect("MockServer should expose received_requests");
        assert!(
            received.is_empty(),
            "wechat must not hit the network when bot_token is unset; got {} request(s)",
            received.len()
        );
    }
}
