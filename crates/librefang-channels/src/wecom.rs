//! WeCom intelligent bot adapter (WebSocket long-connection).
//!
//! Connects to `wss://openws.work.weixin.qq.com` using Bot ID and Secret.
//! Receives messages via `aibot_msg_callback` frames and replies via
//! `aibot_respond_msg` / `aibot_send_msg` frames.

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
use tokio::sync::{mpsc, watch, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// WeCom intelligent bot WebSocket endpoint.
const WECOM_WS_URL: &str = "wss://openws.work.weixin.qq.com";

/// Maximum text length per reply frame.
const MAX_MESSAGE_LEN: usize = 4096;

/// Heartbeat interval.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Initial reconnect backoff.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Maximum reconnect backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// WeCom intelligent bot adapter.
pub struct WeComAdapter {
    bot_id: String,
    secret: Zeroizing<String>,
    account_id: Option<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Shared WebSocket sender for outbound frames.
    ws_tx: Arc<RwLock<Option<mpsc::UnboundedSender<String>>>>,
}

impl WeComAdapter {
    pub fn new(bot_id: String, secret: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot_id,
            secret: Zeroizing::new(secret),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            ws_tx: Arc::new(RwLock::new(None)),
        }
    }

    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Build a `aibot_respond_msg` reply frame.
    /// Used when replying to a specific incoming message via its req_id.
    #[allow(dead_code)]
    fn build_reply_frame(req_id: &str, text: &str) -> String {
        serde_json::json!({
            "action": "aibot_respond_msg",
            "data": {
                "req_id": req_id,
                "msgtype": "text",
                "text": {
                    "content": text,
                }
            }
        })
        .to_string()
    }

    /// Build a `aibot_send_msg` proactive message frame.
    fn build_send_frame(user_id: &str, text: &str) -> String {
        serde_json::json!({
            "action": "aibot_send_msg",
            "data": {
                "receiver": {
                    "user_id": user_id,
                },
                "msgtype": "text",
                "text": {
                    "content": text,
                }
            }
        })
        .to_string()
    }
}

/// Parse an incoming WebSocket text frame as JSON.
fn parse_ws_frame(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text).ok()
}

/// Extract a message callback from a parsed frame.
/// Returns (req_id, from_user_id, content, is_group).
fn extract_msg_callback(frame: &serde_json::Value) -> Option<(String, String, String, bool)> {
    let action = frame.get("action")?.as_str()?;
    if action != "aibot_msg_callback" {
        return None;
    }
    let data = frame.get("data")?;
    let req_id = data.get("req_id")?.as_str()?.to_string();
    let from_user = data
        .get("from")
        .and_then(|f| f.get("user_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let msgtype = data.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
    let content = match msgtype {
        "text" => data
            .get("text")
            .and_then(|t| t.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => {
            debug!(msgtype, "Unsupported WeCom bot message type, skipping");
            return None;
        }
    };

    if from_user.is_empty() || content.is_empty() {
        return None;
    }

    let is_group = data
        .get("chat_type")
        .and_then(|v| v.as_str())
        .map(|t| t == "group")
        .unwrap_or(false);

    Some((req_id, from_user, content, is_group))
}

/// Check if a frame is a subscribe success acknowledgement.
fn is_subscribe_success(frame: &serde_json::Value) -> bool {
    let action = frame.get("action").and_then(|v| v.as_str());
    let errcode = frame.get("errcode").and_then(|v| v.as_i64());
    let data_errcode = frame
        .get("data")
        .and_then(|d| d.get("errcode"))
        .and_then(|v| v.as_i64());

    matches!(action, Some("aibot_subscribe"))
        && (errcode == Some(0) || errcode.is_none())
        && (data_errcode == Some(0) || data_errcode.is_none())
}

#[async_trait]
impl ChannelAdapter for WeComAdapter {
    fn name(&self) -> &str {
        "wecom"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("wecom".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (msg_tx, msg_rx) = mpsc::channel::<ChannelMessage>(256);
        let bot_id = self.bot_id.clone();
        let secret = self.secret.clone();
        let account_id = Arc::new(self.account_id.clone());
        let mut shutdown_rx = self.shutdown_rx.clone();
        let ws_tx_shared = Arc::clone(&self.ws_tx);

        info!(bot_id = %bot_id, "Starting WeCom bot adapter (WebSocket)");

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;

            'outer: loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let ws_result = tokio_tungstenite::connect_async(WECOM_WS_URL).await;

                let ws_stream = match ws_result {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!(
                            "WeCom bot WebSocket connection failed: {e}, retrying in {backoff:?}"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                backoff = INITIAL_BACKOFF;
                info!("WeCom bot WebSocket connected");

                let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

                // Channel for outbound frames from the send() method
                let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<String>();
                {
                    let mut guard = ws_tx_shared.write().await;
                    *guard = Some(frame_tx);
                }

                // Send subscribe frame
                let subscribe_frame = serde_json::json!({
                    "action": "aibot_subscribe",
                    "data": {
                        "bot_id": bot_id,
                        "secret": secret.as_str(),
                    }
                })
                .to_string();

                if let Err(e) = ws_sink.send(WsMessage::Text(subscribe_frame.into())).await {
                    warn!("Failed to send subscribe frame: {e}");
                    continue 'outer;
                }

                let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
                heartbeat.tick().await; // consume immediate first tick

                let should_reconnect = 'inner: loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            info!("WeCom bot adapter shutting down");
                            break 'inner false;
                        }
                        _ = heartbeat.tick() => {
                            let ping = serde_json::json!({"action": "ping"}).to_string();
                            if let Err(e) = ws_sink.send(WsMessage::Text(ping.into())).await {
                                warn!("WeCom bot heartbeat failed: {e}");
                                break 'inner true;
                            }
                        }
                        Some(frame_text) = frame_rx.recv() => {
                            if let Err(e) = ws_sink.send(WsMessage::Text(frame_text.into())).await {
                                warn!("WeCom bot send failed: {e}");
                                break 'inner true;
                            }
                        }
                        ws_msg = ws_stream_rx.next() => {
                            match ws_msg {
                                Some(Ok(WsMessage::Text(text))) => {
                                    let text_str: &str = &text;
                                    let Some(frame) = parse_ws_frame(text_str) else {
                                        debug!("WeCom bot: unparseable frame");
                                        continue 'inner;
                                    };

                                    if is_subscribe_success(&frame) {
                                        info!("WeCom bot subscribed successfully");
                                        continue 'inner;
                                    }

                                    // Handle event callbacks
                                    if frame.get("action").and_then(|v| v.as_str())
                                        == Some("aibot_event_callback")
                                    {
                                        debug!(event = ?frame.get("data"), "WeCom bot event");
                                        continue 'inner;
                                    }

                                    // Handle pong
                                    if frame.get("action").and_then(|v| v.as_str())
                                        == Some("pong")
                                    {
                                        continue 'inner;
                                    }

                                    if let Some((req_id, from_user, content, is_group)) =
                                        extract_msg_callback(&frame)
                                    {
                                        let mut msg = ChannelMessage {
                                            channel: ChannelType::Custom("wecom".to_string()),
                                            platform_message_id: req_id.clone(),
                                            sender: ChannelUser {
                                                platform_id: from_user.clone(),
                                                display_name: from_user.clone(),
                                                librefang_user: None,
                                            },
                                            content: ChannelContent::Text(content),
                                            target_agent: None,
                                            timestamp: Utc::now(),
                                            is_group,
                                            thread_id: None,
                                            metadata: HashMap::new(),
                                        };

                                        // Store req_id so the bridge can use aibot_respond_msg
                                        msg.metadata.insert(
                                            "wecom_req_id".to_string(),
                                            serde_json::json!(req_id),
                                        );

                                        if let Some(ref aid) = *account_id {
                                            msg.metadata.insert(
                                                "account_id".to_string(),
                                                serde_json::json!(aid),
                                            );
                                        }

                                        let _ = msg_tx.send(msg).await;
                                    }
                                }
                                Some(Ok(WsMessage::Ping(data))) => {
                                    let _ = ws_sink.send(WsMessage::Pong(data)).await;
                                }
                                Some(Ok(WsMessage::Close(_))) => {
                                    info!("WeCom bot WebSocket closed by server");
                                    break 'inner true;
                                }
                                Some(Err(e)) => {
                                    warn!("WeCom bot WebSocket error: {e}");
                                    break 'inner true;
                                }
                                None => {
                                    info!("WeCom bot WebSocket stream ended");
                                    break 'inner true;
                                }
                                _ => {}
                            }
                        }
                    }
                };

                // Clear the shared sender on disconnect
                {
                    let mut guard = ws_tx_shared.write().await;
                    *guard = None;
                }

                if !should_reconnect {
                    break 'outer;
                }

                warn!("WeCom bot reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            msg_rx,
        )))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let guard = self.ws_tx.read().await;
        let frame_tx = guard.as_ref().ok_or("WeCom bot WebSocket not connected")?;

        match content {
            ChannelContent::Text(text) => {
                let user_id = &user.platform_id;
                for chunk in split_message(&text, MAX_MESSAGE_LEN) {
                    let frame = Self::build_send_frame(user_id, chunk);
                    frame_tx
                        .send(frame)
                        .map_err(|e| format!("WeCom bot send failed: {e}"))?;
                }
            }
            ChannelContent::Command { .. } => {
                warn!("WeCom bot: commands not supported");
            }
            _ => {
                warn!("WeCom bot: unsupported content type");
            }
        }

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

    #[test]
    fn test_adapter_name() {
        let adapter = WeComAdapter::new("bot_id".to_string(), "secret".to_string());
        assert_eq!(adapter.name(), "wecom");
    }

    #[test]
    fn test_adapter_channel_type() {
        let adapter = WeComAdapter::new("bot_id".to_string(), "secret".to_string());
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("wecom".to_string())
        );
    }

    #[test]
    fn test_build_reply_frame() {
        let frame = WeComAdapter::build_reply_frame("req123", "hello");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["action"], "aibot_respond_msg");
        assert_eq!(parsed["data"]["req_id"], "req123");
        assert_eq!(parsed["data"]["text"]["content"], "hello");
    }

    #[test]
    fn test_build_send_frame() {
        let frame = WeComAdapter::build_send_frame("user1", "hi");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["action"], "aibot_send_msg");
        assert_eq!(parsed["data"]["receiver"]["user_id"], "user1");
        assert_eq!(parsed["data"]["text"]["content"], "hi");
    }

    #[test]
    fn test_extract_msg_callback() {
        let frame = serde_json::json!({
            "action": "aibot_msg_callback",
            "data": {
                "req_id": "req123",
                "from": { "user_id": "user1" },
                "msgtype": "text",
                "text": { "content": "hello bot" },
                "chat_type": "single",
            }
        });
        let (req_id, user, content, is_group) = extract_msg_callback(&frame).unwrap();
        assert_eq!(req_id, "req123");
        assert_eq!(user, "user1");
        assert_eq!(content, "hello bot");
        assert!(!is_group);
    }

    #[test]
    fn test_extract_msg_callback_group() {
        let frame = serde_json::json!({
            "action": "aibot_msg_callback",
            "data": {
                "req_id": "req456",
                "from": { "user_id": "user2" },
                "msgtype": "text",
                "text": { "content": "group msg" },
                "chat_type": "group",
            }
        });
        let (_, _, _, is_group) = extract_msg_callback(&frame).unwrap();
        assert!(is_group);
    }

    #[test]
    fn test_extract_msg_callback_ignores_non_text() {
        let frame = serde_json::json!({
            "action": "aibot_msg_callback",
            "data": {
                "req_id": "req789",
                "from": { "user_id": "user3" },
                "msgtype": "image",
            }
        });
        assert!(extract_msg_callback(&frame).is_none());
    }

    #[test]
    fn test_extract_msg_callback_ignores_other_actions() {
        let frame = serde_json::json!({
            "action": "aibot_event_callback",
            "data": { "event": "enter_chat" }
        });
        assert!(extract_msg_callback(&frame).is_none());
    }

    #[test]
    fn test_is_subscribe_success() {
        let frame = serde_json::json!({
            "action": "aibot_subscribe",
            "errcode": 0,
            "errmsg": "ok"
        });
        assert!(is_subscribe_success(&frame));
    }

    #[test]
    fn test_is_subscribe_failure() {
        let frame = serde_json::json!({
            "action": "aibot_subscribe",
            "errcode": 40001,
            "errmsg": "invalid secret"
        });
        assert!(!is_subscribe_success(&frame));
    }

    #[test]
    fn test_max_message_length() {
        assert_eq!(MAX_MESSAGE_LEN, 4096);
    }
}
