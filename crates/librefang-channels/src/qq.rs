//! QQ Bot API v2 adapter for the LibreFang channel bridge.
//!
//! Uses WebSocket for real-time message delivery via QQ Bot API v2.
//! Supports guild, DM, and group message types.

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

const QQ_API_BASE: &str = "https://api.sgroup.qq.com";
const QQ_TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
/// QQ message length limit (approximate).
const QQ_MAX_MESSAGE_LEN: usize = 2000;

/// Intent bit flags for QQ Bot API v2.
const INTENT_GUILDS: u32 = 1 << 0;
const INTENT_GUILD_MEMBERS: u32 = 1 << 1;
const INTENT_DIRECT_MESSAGE: u32 = 1 << 12;
const INTENT_GROUP_AND_C2C: u32 = 1 << 25;
const INTENT_PUBLIC_GUILD_MESSAGES: u32 = 1 << 30;

/// Default intents for the QQ adapter.
const DEFAULT_INTENTS: u32 = INTENT_GUILDS
    | INTENT_GUILD_MEMBERS
    | INTENT_DIRECT_MESSAGE
    | INTENT_GROUP_AND_C2C
    | INTENT_PUBLIC_GUILD_MESSAGES;

/// QQ Bot API v2 adapter using WebSocket.
pub struct QqAdapter {
    app_id: String,
    app_secret: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    connected: Arc<AtomicBool>,
    messages_received: Arc<AtomicU64>,
    messages_sent: Arc<AtomicU64>,
    started_at: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,
    last_error: Arc<RwLock<Option<String>>>,
    /// Current access token (refreshed periodically).
    access_token: Arc<RwLock<Option<String>>>,
}

impl QqAdapter {
    pub fn new(app_id: String, app_secret: String, allowed_users: Vec<String>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            app_id,
            app_secret: Zeroizing::new(app_secret),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            allowed_users,
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            connected: Arc::new(AtomicBool::new(false)),
            messages_received: Arc::new(AtomicU64::new(0)),
            messages_sent: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(RwLock::new(None)),
            last_error: Arc::new(RwLock::new(None)),
            access_token: Arc::new(RwLock::new(None)),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Send a reply to QQ.
    async fn send_qq_message(
        &self,
        endpoint: &str,
        msg_id: &str,
        content: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token = self.access_token.read().await.clone().unwrap_or_default();
        let body = serde_json::json!({
            "content": content,
            "msg_id": msg_id,
            "msg_type": 0,
        });
        let resp = self
            .client
            .post(format!("{}{}", QQ_API_BASE, endpoint))
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            warn!("QQ send failed ({}): {}", status, text);
            return Err(format!("QQ API error {}: {}", status, text).into());
        }
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// Strip markdown formatting to plain text for QQ.
fn strip_markdown(text: &str) -> String {
    let mut s = text.to_string();
    // Remove <think>...</think> reasoning tags
    while let Some(start) = s.find("<think>") {
        if let Some(end) = s[start..].find("</think>") {
            s = format!("{}{}", &s[..start], &s[start + end + 8..]);
        } else {
            break;
        }
    }
    // Code blocks: keep content, remove fences
    let re_codeblock = regex_lite::Regex::new(r"```\w*\n?([\s\S]*?)```").unwrap();
    s = re_codeblock.replace_all(&s, "$1").to_string();
    // Inline code
    let re_inline = regex_lite::Regex::new(r"`([^`]+)`").unwrap();
    s = re_inline.replace_all(&s, "$1").to_string();
    // Bold
    let re_bold = regex_lite::Regex::new(r"\*\*([^*]+)\*\*").unwrap();
    s = re_bold.replace_all(&s, "$1").to_string();
    // Italic
    let re_italic = regex_lite::Regex::new(r"\*([^*]+)\*").unwrap();
    s = re_italic.replace_all(&s, "$1").to_string();
    // Headings
    let re_heading = regex_lite::Regex::new(r"(?m)^#{1,6}\s+").unwrap();
    s = re_heading.replace_all(&s, "").to_string();
    // Table separator rows
    let re_table_sep = regex_lite::Regex::new(r"(?m)^\|[-:| ]+\|$").unwrap();
    s = re_table_sep.replace_all(&s, "").to_string();
    // Links
    let re_link = regex_lite::Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    s = re_link.replace_all(&s, "$1").to_string();
    // Blockquotes
    let re_quote = regex_lite::Regex::new(r"(?m)^>\s?").unwrap();
    s = re_quote.replace_all(&s, "").to_string();
    // Horizontal rules
    let re_hr = regex_lite::Regex::new(r"(?m)^---+$").unwrap();
    s = re_hr.replace_all(&s, "").to_string();
    // Excess newlines
    let re_newlines = regex_lite::Regex::new(r"\n{3,}").unwrap();
    s = re_newlines.replace_all(&s, "\n\n").to_string();
    s.trim().to_string()
}

/// Parse a QQ dispatch event into a ChannelMessage.
fn parse_dispatch_event(
    event_type: &str,
    data: &serde_json::Value,
) -> Option<(ChannelMessage, String, String)> {
    // Extract common fields
    let msg_id = data["id"].as_str().unwrap_or("").to_string();
    let content = data["content"].as_str().unwrap_or("").trim().to_string();
    if content.is_empty() {
        return None;
    }

    let (sender_id, sender_name, is_group, reply_endpoint) = match event_type {
        "MESSAGE_CREATE" | "AT_MESSAGE_CREATE" => {
            let channel_id = data["channel_id"].as_str().unwrap_or("");
            let author = &data["author"];
            let user_id = author["id"].as_str().unwrap_or("").to_string();
            let username = author["username"].as_str().unwrap_or("User").to_string();
            let endpoint = format!("/channels/{}/messages", channel_id);
            (user_id, username, true, endpoint)
        }
        "DIRECT_MESSAGE_CREATE" => {
            let guild_id = data["guild_id"].as_str().unwrap_or("");
            let author = &data["author"];
            let user_id = author["id"].as_str().unwrap_or("").to_string();
            let username = author["username"].as_str().unwrap_or("User").to_string();
            let endpoint = format!("/dms/{}/messages", guild_id);
            (user_id, username, false, endpoint)
        }
        "GROUP_AT_MESSAGE_CREATE" => {
            let group_openid = data["group_openid"].as_str().unwrap_or("");
            let author = &data["author"];
            let user_id = author["member_openid"].as_str().unwrap_or("").to_string();
            let endpoint = format!("/v2/groups/{}/messages", group_openid);
            (user_id, "GroupUser".to_string(), true, endpoint)
        }
        "C2C_MESSAGE_CREATE" => {
            let user_openid = data["author"]["user_openid"].as_str().unwrap_or("");
            let endpoint = format!("/v2/users/{}/messages", user_openid);
            (user_openid.to_string(), "User".to_string(), false, endpoint)
        }
        _ => return None,
    };

    // Strip bot mention prefix (e.g., "/@ Bot " or "<@!botid>")
    let clean_content = content.trim_start_matches('/').trim().to_string();

    let msg = ChannelMessage {
        channel: ChannelType::Custom("qq".to_string()),
        platform_message_id: msg_id.clone(),
        sender: ChannelUser {
            platform_id: sender_id,
            display_name: sender_name,
            librefang_user: None,
        },
        content: ChannelContent::Text(clean_content),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group,
        thread_id: None,
        metadata: HashMap::new(),
    };

    Some((msg, reply_endpoint, msg_id))
}

#[async_trait]
impl ChannelAdapter for QqAdapter {
    fn name(&self) -> &str {
        "QQ"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("qq".to_string())
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        info!("Starting QQ adapter for app_id={}", self.app_id);

        // Ensure rustls CryptoProvider is installed for WSS connections.
        let _ = rustls::crypto::ring::default_provider().install_default();

        *self.started_at.write().await = Some(chrono::Utc::now());

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let mut shutdown_rx = self.shutdown_rx.clone();

        let app_id = self.app_id.clone();
        let app_secret = self.app_secret.clone();
        let client = self.client.clone();
        let allowed_users = self.allowed_users.clone();
        let connected = self.connected.clone();
        let messages_received = self.messages_received.clone();
        let last_error = self.last_error.clone();
        let access_token = self.access_token.clone();
        let account_id = self.account_id.clone();

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                // Step 1: Get access token
                let token = match get_token(&client, &app_id, &app_secret).await {
                    Ok(t) => {
                        *access_token.write().await = Some(t.clone());
                        t
                    }
                    Err(e) => {
                        error!("QQ: failed to get access token: {}", e);
                        *last_error.write().await = Some(format!("Token error: {}", e));
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                // Step 2: Get gateway URL
                let gw_url = match get_gateway(&client, &token).await {
                    Ok(url) => url,
                    Err(e) => {
                        error!("QQ: failed to get gateway URL: {}", e);
                        *last_error.write().await = Some(format!("Gateway error: {}", e));
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                // Step 3: Connect WebSocket
                info!("QQ: connecting to WebSocket gateway...");
                let ws_result = tokio_tungstenite::connect_async(&gw_url).await;
                let (ws_stream, _) = match ws_result {
                    Ok(pair) => pair,
                    Err(e) => {
                        error!("QQ: WebSocket connection failed: {}", e);
                        *last_error.write().await = Some(format!("WS connect error: {}", e));
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                info!("QQ: WebSocket connected");
                backoff = INITIAL_BACKOFF;

                use futures::{SinkExt, StreamExt};
                let (mut ws_tx, mut ws_rx) = ws_stream.split();

                let mut heartbeat_interval: Option<tokio::time::Interval> = None;
                let mut last_seq: Option<u64> = None;
                let mut identified = false;

                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                info!("QQ: shutdown signal received");
                                let _ = ws_tx.close().await;
                                connected.store(false, Ordering::Relaxed);
                                return;
                            }
                        }
                        // Heartbeat tick
                        _ = async {
                            if let Some(ref mut interval) = heartbeat_interval {
                                interval.tick().await
                            } else {
                                // Never fires if no interval set
                                std::future::pending::<tokio::time::Instant>().await
                            }
                        } => {
                            let hb = serde_json::json!({
                                "op": 1,
                                "d": last_seq,
                            });
                            if let Err(e) = ws_tx.send(tokio_tungstenite::tungstenite::Message::Text(hb.to_string().into())).await {
                                warn!("QQ: heartbeat send failed: {}", e);
                                break;
                            }
                            debug!("QQ: heartbeat sent (seq={:?})", last_seq);
                        }
                        // WebSocket message
                        msg = ws_rx.next() => {
                            match msg {
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                    debug!("QQ: WS recv: {}", &text[..text.len().min(500)]);
                                    let payload: serde_json::Value = match serde_json::from_str(&text) {
                                        Ok(v) => v,
                                        Err(_) => continue,
                                    };
                                    let op = payload["op"].as_u64().unwrap_or(99);
                                    match op {
                                        // DISPATCH
                                        0 => {
                                            if let Some(s) = payload["s"].as_u64() {
                                                last_seq = Some(s);
                                            }
                                            let event_type = payload["t"].as_str().unwrap_or("");

                                            // Handle READY event (comes as dispatch op=0 with t="READY")
                                            if event_type == "READY" && !identified {
                                                let user = &payload["d"]["user"];
                                                let bot_name = user["username"].as_str().unwrap_or("QQBot");
                                                info!("QQ: READY! Bot: {}", bot_name);
                                                connected.store(true, Ordering::Relaxed);
                                                *last_error.write().await = None;
                                                identified = true;
                                                continue;
                                            }

                                            let data = &payload["d"];
                                            if let Some((mut msg, _endpoint, _msg_id)) = parse_dispatch_event(event_type, data) {
                                                if allowed_users.is_empty() || allowed_users.iter().any(|u| u == &msg.sender.platform_id) {
                                                    messages_received.fetch_add(1, Ordering::Relaxed);
                                                    // Inject account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    msg.metadata.insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                if tx.send(msg).await.is_err() {
                                                        info!("QQ: receiver dropped, stopping");
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                        // HELLO
                                        10 => {
                                            let interval_ms = payload["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
                                            info!("QQ: HELLO received, heartbeat interval={}ms", interval_ms);
                                            heartbeat_interval = Some(tokio::time::interval(Duration::from_millis(interval_ms)));

                                            // Send IDENTIFY
                                            let identify = serde_json::json!({
                                                "op": 2,
                                                "d": {
                                                    "token": format!("QQBot {}", token),
                                                    "intents": DEFAULT_INTENTS,
                                                    "shard": [0, 1],
                                                }
                                            });
                                            if let Err(e) = ws_tx.send(tokio_tungstenite::tungstenite::Message::Text(identify.to_string().into())).await {
                                                error!("QQ: IDENTIFY send failed: {}", e);
                                                break;
                                            }
                                            info!("QQ: IDENTIFY sent");
                                        }
                                        // HEARTBEAT_ACK
                                        11 => {
                                            debug!("QQ: heartbeat ACK received");
                                        }
                                        // RECONNECT
                                        7 => {
                                            info!("QQ: server requested reconnect");
                                            break;
                                        }
                                        // INVALID SESSION
                                        9 => {
                                            warn!("QQ: invalid session, will reconnect");
                                            tokio::time::sleep(Duration::from_secs(3)).await;
                                            break;
                                        }
                                        _ => {
                                            debug!("QQ: unhandled opcode {}", op);
                                        }
                                    }
                                }
                                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => {
                                    info!("QQ: WebSocket closed by server");
                                    break;
                                }
                                Some(Err(e)) => {
                                    warn!("QQ: WebSocket error: {}", e);
                                    *last_error.write().await = Some(format!("WS error: {}", e));
                                    break;
                                }
                                None => {
                                    info!("QQ: WebSocket stream ended");
                                    break;
                                }
                                _ => {} // Ping/Pong/Binary handled by tungstenite
                            }
                        }
                    }
                }

                connected.store(false, Ordering::Relaxed);
                info!("QQ: reconnecting in {:?}...", backoff);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let text = match content {
            ChannelContent::Text(ref t) => strip_markdown(t),
            _ => return Ok(()), // QQ adapter only handles text for now
        };

        // The platform_id encodes the reply endpoint and msg_id as "endpoint|msg_id"
        // For now, we use a simple approach: reply via the REST API
        // The bridge stores reply context in metadata
        let parts: Vec<&str> = user.platform_id.splitn(2, '|').collect();
        if parts.len() == 2 {
            let endpoint = parts[0];
            let msg_id = parts[1];
            for chunk in split_message(&text, QQ_MAX_MESSAGE_LEN) {
                if let Err(e) = self.send_qq_message(endpoint, msg_id, chunk).await {
                    warn!("QQ: failed to send message: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Stopping QQ adapter");
        let _ = self.shutdown_tx.send(true);
        self.connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        let started_at = self.started_at.try_read().ok().and_then(|g| *g);
        let last_error = self.last_error.try_read().ok().and_then(|g| g.clone());
        ChannelStatus {
            connected: self.connected.load(Ordering::Relaxed),
            started_at,
            last_message_at: None,
            messages_received: self.messages_received.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            last_error,
        }
    }
}

// Helper functions to avoid borrowing self in the spawned task
async fn get_token(
    client: &reqwest::Client,
    app_id: &str,
    app_secret: &Zeroizing<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let body = serde_json::json!({
        "appId": app_id,
        "clientSecret": app_secret.as_str(),
    });
    let resp = client.post(QQ_TOKEN_URL).json(&body).send().await?;
    let data: serde_json::Value = resp.json().await?;
    data["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "missing access_token".into())
}

async fn get_gateway(
    client: &reqwest::Client,
    token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let resp = client
        .get(format!("{}/gateway", QQ_API_BASE))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    let data: serde_json::Value = resp.json().await?;
    data["url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "missing gateway url".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_markdown_basic() {
        assert_eq!(strip_markdown("**bold**"), "bold");
        assert_eq!(strip_markdown("*italic*"), "italic");
        assert_eq!(strip_markdown("# Heading"), "Heading");
        assert_eq!(strip_markdown("[link](http://example.com)"), "link");
    }

    #[test]
    fn test_strip_markdown_think_tags() {
        let input = "<think>reasoning here</think>The actual response";
        assert_eq!(strip_markdown(input), "The actual response");
    }

    #[test]
    fn test_strip_markdown_code_block() {
        let input = "```python\nprint('hello')\n```";
        assert_eq!(strip_markdown(input), "print('hello')");
    }

    #[test]
    fn test_parse_dispatch_guild_message() {
        let data = serde_json::json!({
            "id": "msg123",
            "channel_id": "chan456",
            "content": "Hello bot",
            "author": {
                "id": "user789",
                "username": "TestUser"
            }
        });
        let result = parse_dispatch_event("MESSAGE_CREATE", &data);
        assert!(result.is_some());
        let (msg, endpoint, msg_id) = result.unwrap();
        assert_eq!(msg.sender.platform_id, "user789");
        assert_eq!(msg.sender.display_name, "TestUser");
        assert!(msg.is_group);
        assert_eq!(endpoint, "/channels/chan456/messages");
        assert_eq!(msg_id, "msg123");
    }

    #[test]
    fn test_parse_dispatch_dm() {
        let data = serde_json::json!({
            "id": "dm123",
            "guild_id": "guild456",
            "content": "Private message",
            "author": {
                "id": "user789",
                "username": "Alice"
            }
        });
        let result = parse_dispatch_event("DIRECT_MESSAGE_CREATE", &data);
        assert!(result.is_some());
        let (msg, endpoint, _) = result.unwrap();
        assert!(!msg.is_group);
        assert_eq!(endpoint, "/dms/guild456/messages");
    }

    #[test]
    fn test_parse_dispatch_empty_content() {
        let data = serde_json::json!({
            "id": "msg123",
            "channel_id": "chan456",
            "content": "",
            "author": {"id": "user789", "username": "Test"}
        });
        assert!(parse_dispatch_event("MESSAGE_CREATE", &data).is_none());
    }

    #[test]
    fn test_parse_dispatch_unknown_event() {
        let data = serde_json::json!({"id": "123", "content": "test"});
        assert!(parse_dispatch_event("UNKNOWN_EVENT", &data).is_none());
    }
}
