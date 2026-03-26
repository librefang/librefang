//! Slack Socket Mode adapter for the LibreFang channel bridge.
//!
//! Uses Slack Socket Mode WebSocket (app token) for receiving events and the
//! Web API (bot token) for sending responses. No external Slack crate.

use crate::bridge::SENDER_USER_ID_KEY;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
    InteractiveButton, InteractiveMessage,
};
use async_trait::async_trait;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

const SLACK_API_BASE: &str = "https://slack.com/api";
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const DEFAULT_MAX_MESSAGE_LEN: usize = 3000;

/// Slack Socket Mode adapter.
pub struct SlackAdapter {
    /// SECURITY: Tokens are zeroized on drop to prevent memory disclosure.
    app_token: Zeroizing<String>,
    bot_token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_channels: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Whether to unfurl (expand previews for) links in sent messages.
    /// When `None`, Slack uses its own default behavior.
    unfurl_links: Option<bool>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Bot's own user ID (populated after auth.test).
    bot_user_id: Arc<RwLock<Option<String>>>,
    /// When true, replies are posted as top-level channel messages instead of threads.
    force_flat_replies: bool,
    /// Maximum outbound message length before splitting.
    max_msg_len: usize,
}

impl SlackAdapter {
    pub fn new(app_token: String, bot_token: String, allowed_channels: Vec<String>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            app_token: Zeroizing::new(app_token),
            bot_token: Zeroizing::new(bot_token),
            client: crate::http_client::new_client(),
            allowed_channels,
            account_id: None,
            unfurl_links: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            force_flat_replies: false,
            max_msg_len: DEFAULT_MAX_MESSAGE_LEN,
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the maximum outbound message length. Returns self for builder chaining.
    pub fn with_max_message_length(mut self, len: Option<u32>) -> Self {
        if let Some(v) = len {
            self.max_msg_len = v as usize;
        }
        self
    }

    /// Force replies to be posted as top-level channel messages instead of threads.
    pub fn with_force_flat_replies(mut self, force: bool) -> Self {
        self.force_flat_replies = force;
        self
    }

    /// Set the unfurl_links option. Returns self for builder chaining.
    pub fn with_unfurl_links(mut self, unfurl_links: Option<bool>) -> Self {
        self.unfurl_links = unfurl_links;
        self
    }

    /// Validate the bot token by calling auth.test.
    async fn validate_bot_token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let resp: serde_json::Value = self
            .client
            .post(format!("{SLACK_API_BASE}/auth.test"))
            .header(
                "Authorization",
                format!("Bearer {}", self.bot_token.as_str()),
            )
            .send()
            .await?
            .json()
            .await?;

        if resp["ok"].as_bool() != Some(true) {
            let err = resp["error"].as_str().unwrap_or("unknown error");
            return Err(format!("Slack auth.test failed: {err}").into());
        }

        let user_id = resp["user_id"].as_str().unwrap_or("unknown").to_string();
        Ok(user_id)
    }

    /// Send a message to a Slack channel via chat.postMessage.
    async fn api_send_message(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_message_opts(channel_id, text, None).await
    }

    /// Send an interactive message with Block Kit action buttons.
    ///
    /// Uses Slack's `chat.postMessage` with `blocks` containing a section for
    /// the text and action blocks for button rows.
    async fn api_send_interactive_message(
        &self,
        channel_id: &str,
        text: &str,
        buttons: &[Vec<InteractiveButton>],
        thread_ts: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut blocks = vec![serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": text,
            }
        })];

        // Each row of buttons becomes an "actions" block
        for (row_idx, row) in buttons.iter().enumerate() {
            let elements: Vec<serde_json::Value> = row
                .iter()
                .enumerate()
                .map(|(btn_idx, btn)| {
                    let mut element = serde_json::json!({
                        "type": "button",
                        "text": {
                            "type": "plain_text",
                            "text": btn.label,
                            "emoji": true,
                        },
                        "action_id": format!("interactive_{}_{}", row_idx, btn_idx),
                        "value": btn.action,
                    });
                    if let Some(ref style) = btn.style {
                        // Slack supports "primary" and "danger" button styles
                        if style == "primary" || style == "danger" {
                            element["style"] = serde_json::json!(style);
                        }
                    }
                    if let Some(ref url) = btn.url {
                        element["url"] = serde_json::json!(url);
                    }
                    element
                })
                .collect();

            blocks.push(serde_json::json!({
                "type": "actions",
                "block_id": format!("interactive_row_{row_idx}"),
                "elements": elements,
            }));
        }

        let mut body = serde_json::json!({
            "channel": channel_id,
            "text": text,  // fallback for notifications
            "blocks": blocks,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        if let Some(unfurl) = self.unfurl_links {
            body["unfurl_links"] = serde_json::json!(unfurl);
        }

        let resp: serde_json::Value = self
            .client
            .post(format!("{SLACK_API_BASE}/chat.postMessage"))
            .header(
                "Authorization",
                format!("Bearer {}", self.bot_token.as_str()),
            )
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp["ok"].as_bool() != Some(true) {
            let err = resp["error"].as_str().unwrap_or("unknown");
            warn!("Slack chat.postMessage (interactive) failed: {err}");
        }
        Ok(())
    }

    /// Send a message to a Slack channel, optionally as a thread reply.
    ///
    /// When `thread_ts` is `Some`, the `thread_ts` field is included in the
    /// `chat.postMessage` payload so the message appears as a thread reply
    /// rather than a top-level channel message.
    async fn api_send_message_opts(
        &self,
        channel_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chunks = split_message(text, self.max_msg_len);

        for chunk in chunks {
            let mut body = serde_json::json!({
                "channel": channel_id,
                "text": chunk,
            });

            if let Some(ts) = thread_ts {
                body["thread_ts"] = serde_json::json!(ts);
            }

            if let Some(unfurl) = self.unfurl_links {
                body["unfurl_links"] = serde_json::json!(unfurl);
            }

            let resp: serde_json::Value = self
                .client
                .post(format!("{SLACK_API_BASE}/chat.postMessage"))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.bot_token.as_str()),
                )
                .json(&body)
                .send()
                .await?
                .json()
                .await?;

            if resp["ok"].as_bool() != Some(true) {
                let err = resp["error"].as_str().unwrap_or("unknown");
                warn!("Slack chat.postMessage failed: {err}");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Slack
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Validate bot token first
        let bot_user_id_val = self.validate_bot_token().await?;
        *self.bot_user_id.write().await = Some(bot_user_id_val.clone());
        info!("Slack bot authenticated (user_id: {bot_user_id_val})");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let app_token = self.app_token.clone();
        let bot_user_id = self.bot_user_id.clone();
        let allowed_channels = self.allowed_channels.clone();
        let account_id = self.account_id.clone();
        let client = self.client.clone();
        let mut shutdown = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;

            loop {
                if *shutdown.borrow() {
                    break;
                }

                // Get a fresh WebSocket URL
                let ws_url_result = get_socket_mode_url(&client, &app_token)
                    .await
                    .map_err(|e| e.to_string());
                let ws_url = match ws_url_result {
                    Ok(url) => url,
                    Err(err_msg) => {
                        warn!("Slack: failed to get WebSocket URL: {err_msg}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                info!("Connecting to Slack Socket Mode...");

                let ws_result = tokio_tungstenite::connect_async(&ws_url).await;
                let ws_stream = match ws_result {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!("Slack WebSocket connection failed: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                backoff = INITIAL_BACKOFF;
                info!("Slack Socket Mode connected");

                let (mut ws_tx, mut ws_rx) = ws_stream.split();

                let should_reconnect = 'inner: loop {
                    let msg = tokio::select! {
                        msg = ws_rx.next() => msg,
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                let _ = ws_tx.close().await;
                                return;
                            }
                            continue;
                        }
                    };

                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            warn!("Slack WebSocket error: {e}");
                            break 'inner true;
                        }
                        None => {
                            info!("Slack WebSocket closed");
                            break 'inner true;
                        }
                    };

                    let text = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => t,
                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                            info!("Slack Socket Mode closed by server");
                            break 'inner true;
                        }
                        _ => continue,
                    };

                    let payload: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("Slack: failed to parse message: {e}");
                            continue;
                        }
                    };

                    let envelope_type = payload["type"].as_str().unwrap_or("");

                    match envelope_type {
                        "hello" => {
                            debug!("Slack Socket Mode hello received");
                        }

                        "events_api" => {
                            // Acknowledge the envelope
                            let envelope_id = payload["envelope_id"].as_str().unwrap_or("");
                            if !envelope_id.is_empty() {
                                let ack = serde_json::json!({ "envelope_id": envelope_id });
                                if let Err(e) = ws_tx
                                    .send(tokio_tungstenite::tungstenite::Message::Text(
                                        serde_json::to_string(&ack).unwrap().into(),
                                    ))
                                    .await
                                {
                                    error!("Slack: failed to send ack: {e}");
                                    break 'inner true;
                                }
                            }

                            // Extract the event
                            let event = &payload["payload"]["event"];
                            if let Some(mut msg) =
                                parse_slack_event(event, &bot_user_id, &allowed_channels).await
                            {
                                // Tag message with account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    msg.metadata
                                        .insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                debug!(
                                    "Slack message from {}: {:?}",
                                    msg.sender.display_name, msg.content
                                );
                                if tx.send(msg).await.is_err() {
                                    return;
                                }
                            }
                        }

                        "interactive" => {
                            // Handle interactive payloads (block_actions, etc.)
                            let envelope_id = payload["envelope_id"].as_str().unwrap_or("");
                            if !envelope_id.is_empty() {
                                let ack = serde_json::json!({ "envelope_id": envelope_id });
                                if let Err(e) = ws_tx
                                    .send(tokio_tungstenite::tungstenite::Message::Text(
                                        serde_json::to_string(&ack).unwrap().into(),
                                    ))
                                    .await
                                {
                                    error!("Slack: failed to send interactive ack: {e}");
                                    break 'inner true;
                                }
                            }

                            let interaction = &payload["payload"];
                            if let Some(mut msg) = parse_slack_block_action(
                                interaction,
                                &bot_user_id,
                                &allowed_channels,
                            )
                            .await
                            {
                                if let Some(ref aid) = account_id {
                                    msg.metadata
                                        .insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                debug!(
                                    "Slack block_action from {}: {:?}",
                                    msg.sender.display_name, msg.content
                                );
                                if tx.send(msg).await.is_err() {
                                    return;
                                }
                            }
                        }

                        "disconnect" => {
                            let reason = payload["reason"].as_str().unwrap_or("unknown");
                            info!("Slack disconnect request: {reason}");
                            break 'inner true;
                        }

                        _ => {
                            debug!("Slack envelope type: {envelope_type}");
                        }
                    }
                };

                if !should_reconnect || *shutdown.borrow() {
                    break;
                }

                warn!("Slack: reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }

            info!("Slack Socket Mode loop stopped");
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let channel_id = &user.platform_id;
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(channel_id, &text).await?;
            }
            ChannelContent::Interactive { text, buttons } => {
                self.api_send_interactive_message(channel_id, &text, &buttons, None)
                    .await?;
            }
            _ => {
                self.api_send_message(channel_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_interactive(
        &self,
        user: &ChannelUser,
        message: &InteractiveMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_interactive_message(&user.platform_id, &message.text, &message.buttons, None)
            .await
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(text) => text,
            _ => "(Unsupported content type)".to_string(),
        };

        // When force_flat_replies is enabled, skip threading so the message
        // appears as a top-level channel message instead of a thread reply.
        let ts = if self.force_flat_replies {
            None
        } else {
            Some(thread_id)
        };

        self.api_send_message_opts(&user.platform_id, &text, ts)
            .await?;
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

/// Helper to get Socket Mode WebSocket URL.
async fn get_socket_mode_url(
    client: &reqwest::Client,
    app_token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let resp: serde_json::Value = client
        .post(format!("{SLACK_API_BASE}/apps.connections.open"))
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await?
        .json()
        .await?;

    if resp["ok"].as_bool() != Some(true) {
        let err = resp["error"].as_str().unwrap_or("unknown error");
        return Err(format!("Slack apps.connections.open failed: {err}").into());
    }

    resp["url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Missing 'url' in connections.open response".into())
}

/// Parse a Slack event into a `ChannelMessage`.
async fn parse_slack_event(
    event: &serde_json::Value,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    allowed_channels: &[String],
) -> Option<ChannelMessage> {
    let event_type = event["type"].as_str()?;
    if event_type != "message" && event_type != "app_mention" {
        return None;
    }

    // Handle message_changed subtype: extract inner message
    let subtype = event["subtype"].as_str();
    let (msg_data, is_edit) = match subtype {
        Some("message_changed") => {
            // Edited messages have the new content in event.message
            match event.get("message") {
                Some(inner) => (inner, true),
                None => return None,
            }
        }
        Some(_) => return None, // Skip other subtypes (joins, leaves, etc.)
        None => (event, false),
    };

    // Filter out bot's own messages
    if msg_data.get("bot_id").is_some() {
        return None;
    }
    let user_id = msg_data["user"]
        .as_str()
        .or_else(|| event["user"].as_str())?;
    if let Some(ref bid) = *bot_user_id.read().await {
        if user_id == bid {
            return None;
        }
    }

    let channel = event["channel"].as_str()?;

    // Filter by allowed channels. DMs are exempt and handled by dm_policy.
    if !channel.starts_with('D')
        && !allowed_channels.is_empty()
        && !allowed_channels.contains(&channel.to_string())
    {
        return None;
    }

    let text = msg_data["text"].as_str().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let ts = if is_edit {
        msg_data["ts"]
            .as_str()
            .unwrap_or(event["ts"].as_str().unwrap_or("0"))
    } else {
        event["ts"].as_str().unwrap_or("0")
    };

    // Parse timestamp (Slack uses epoch.microseconds format)
    let timestamp = ts
        .split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|epoch| chrono::DateTime::from_timestamp(epoch, 0))
        .unwrap_or_else(chrono::Utc::now);

    // Parse commands (messages starting with /)
    let content = if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd_name = &parts[0][1..];
        let args = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command {
            name: cmd_name.to_string(),
            args,
        }
    } else {
        ChannelContent::Text(text.to_string())
    };

    let mut metadata = HashMap::new();
    metadata.insert(SENDER_USER_ID_KEY.to_string(), serde_json::json!(user_id));
    if event_type == "app_mention" {
        metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
    }

    // Slack channel prefixes: D = direct message, other channel types are group contexts.
    let is_group = !channel.starts_with('D');

    // Extract thread_ts for threading context. Slack uses `thread_ts` to identify
    // the parent message of a thread. For edited messages, check the inner message first.
    let thread_id = msg_data["thread_ts"]
        .as_str()
        .or_else(|| event["thread_ts"].as_str())
        .map(|s| s.to_string());

    Some(ChannelMessage {
        channel: ChannelType::Slack,
        platform_message_id: ts.to_string(),
        sender: ChannelUser {
            // For DMs, platform_id is the DM channel ID (D*), not the user ID.
            // The actual sender user ID is in metadata[SENDER_USER_ID_KEY].
            platform_id: channel.to_string(),
            display_name: user_id.to_string(), // Slack user IDs as display name
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp,
        is_group,
        thread_id,
        metadata,
    })
}

/// Parse a Slack `block_actions` interactive payload into a `ChannelMessage`.
///
/// Called when a user clicks a button in an interactive Block Kit message.
/// Each clicked action's `value` is delivered as a `ButtonCallback` content variant.
async fn parse_slack_block_action(
    interaction: &serde_json::Value,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    allowed_channels: &[String],
) -> Option<ChannelMessage> {
    let interaction_type = interaction["type"].as_str()?;
    if interaction_type != "block_actions" {
        debug!("Slack: ignoring non-block_actions interactive type: {interaction_type}");
        return None;
    }

    let user = interaction.get("user")?;
    let user_id = user["id"].as_str()?;

    // Filter out bot's own interactions
    if let Some(ref bid) = *bot_user_id.read().await {
        if user_id == bid {
            return None;
        }
    }

    // Extract channel info
    let channel = interaction
        .get("channel")
        .and_then(|c| c["id"].as_str())
        .unwrap_or("");

    if channel.is_empty() {
        return None;
    }

    // Filter by allowed channels (DMs exempt)
    if !channel.starts_with('D')
        && !allowed_channels.is_empty()
        && !allowed_channels.contains(&channel.to_string())
    {
        return None;
    }

    // Extract the first action (most common case: single button click)
    let actions = interaction["actions"].as_array()?;
    let action = actions.first()?;
    let action_value = action["value"].as_str().unwrap_or("");
    let action_id = action["action_id"].as_str().unwrap_or("");

    if action_value.is_empty() {
        return None;
    }

    // Extract message text from the original message (if available)
    let message_text = interaction
        .get("message")
        .and_then(|m| m["text"].as_str())
        .map(String::from);

    let message_ts = interaction
        .get("message")
        .and_then(|m| m["ts"].as_str())
        .unwrap_or("0");

    let trigger_id = interaction["trigger_id"].as_str().unwrap_or("");

    let timestamp = message_ts
        .split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|epoch| chrono::DateTime::from_timestamp(epoch, 0))
        .unwrap_or_else(chrono::Utc::now);

    let is_group = !channel.starts_with('D');

    let thread_id = interaction
        .get("message")
        .and_then(|m| m["thread_ts"].as_str())
        .map(|s| s.to_string());

    let mut metadata = HashMap::new();
    metadata.insert(SENDER_USER_ID_KEY.to_string(), serde_json::json!(user_id));
    metadata.insert("action_id".to_string(), serde_json::json!(action_id));
    metadata.insert("trigger_id".to_string(), serde_json::json!(trigger_id));
    metadata.insert("block_action".to_string(), serde_json::Value::Bool(true));

    Some(ChannelMessage {
        channel: ChannelType::Slack,
        platform_message_id: message_ts.to_string(),
        sender: ChannelUser {
            platform_id: channel.to_string(),
            display_name: user_id.to_string(),
            librefang_user: None,
        },
        content: ChannelContent::ButtonCallback {
            action: action_value.to_string(),
            message_text,
        },
        target_agent: None,
        timestamp,
        is_group,
        thread_id,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_slack_event_basic() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "Hello agent!",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(msg.channel, ChannelType::Slack);
        assert_eq!(msg.sender.platform_id, "C789");
        assert_eq!(
            msg.metadata
                .get(SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("U456")
        );
        assert!(msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello agent!"));
    }

    #[tokio::test]
    async fn test_parse_slack_event_filters_bot() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "Bot message",
            "ts": "1700000000.000100",
            "bot_id": "B999"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_slack_event_filters_own_user() {
        let bot_id = Arc::new(RwLock::new(Some("U456".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "My message",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_slack_event_channel_filter() {
        let bot_id = Arc::new(RwLock::new(None));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "Hello",
            "ts": "1700000000.000100"
        });

        // Not in allowed channels
        let msg =
            parse_slack_event(&event, &bot_id, &["C111".to_string(), "C222".to_string()]).await;
        assert!(msg.is_none());

        // In allowed channels
        let msg = parse_slack_event(&event, &bot_id, &["C789".to_string()]).await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_slack_event_skips_other_subtypes() {
        let bot_id = Arc::new(RwLock::new(None));
        // Non-message_changed subtypes should still be filtered
        let event = serde_json::json!({
            "type": "message",
            "subtype": "channel_join",
            "user": "U456",
            "channel": "C789",
            "text": "joined",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_slack_command() {
        let bot_id = Arc::new(RwLock::new(None));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "/agent hello-world",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_slack_app_mention_sets_was_mentioned() {
        let bot_id = Arc::new(RwLock::new(None));
        let event = serde_json::json!({
            "type": "app_mention",
            "user": "U456",
            "channel": "C789",
            "text": "<@U123> hello there",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_parse_slack_app_mention_filters_bot_self() {
        let bot_id = Arc::new(RwLock::new(Some("U456".to_string())));
        let event = serde_json::json!({
            "type": "app_mention",
            "user": "U456",
            "channel": "C789",
            "text": "<@U456> hello",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await;
        assert!(
            msg.is_none(),
            "app_mention from the bot itself should be filtered out"
        );
    }

    #[tokio::test]
    async fn test_parse_slack_app_mention_filters_disallowed_channel() {
        let bot_id = Arc::new(RwLock::new(None));
        let event = serde_json::json!({
            "type": "app_mention",
            "user": "U456",
            "channel": "C789",
            "text": "<@U123> hello",
            "ts": "1700000000.000100"
        });

        // Channel C789 is not in the allowed list
        let msg =
            parse_slack_event(&event, &bot_id, &["C111".to_string(), "C222".to_string()]).await;
        assert!(
            msg.is_none(),
            "app_mention in a channel not in allowed_channels should be filtered out"
        );
    }

    #[tokio::test]
    async fn test_parse_slack_event_message_changed() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "subtype": "message_changed",
            "channel": "C789",
            "message": {
                "user": "U456",
                "text": "Edited message text",
                "ts": "1700000000.000100"
            },
            "ts": "1700000001.000200"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(msg.channel, ChannelType::Slack);
        assert_eq!(msg.sender.platform_id, "C789");
        assert_eq!(
            msg.metadata
                .get(SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("U456")
        );
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message text"));
    }

    #[tokio::test]
    async fn test_parse_slack_event_dm_detected() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "D789",
            "text": "Hello via DM",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert!(!msg.is_group);
        assert_eq!(msg.sender.platform_id, "D789");
        assert_eq!(
            msg.metadata
                .get(SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("U456")
        );
    }

    #[tokio::test]
    async fn test_parse_slack_event_thread_ts() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "Thread reply",
            "ts": "1700000001.000200",
            "thread_ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(msg.thread_id, Some("1700000000.000100".to_string()));
    }

    #[tokio::test]
    async fn test_parse_slack_event_no_thread_ts() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "user": "U456",
            "channel": "C789",
            "text": "Top-level message",
            "ts": "1700000000.000100"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(msg.thread_id, None);
    }

    #[tokio::test]
    async fn test_parse_slack_event_message_changed_thread_ts() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let event = serde_json::json!({
            "type": "message",
            "subtype": "message_changed",
            "channel": "C789",
            "message": {
                "user": "U456",
                "text": "Edited thread reply",
                "ts": "1700000001.000200",
                "thread_ts": "1700000000.000100"
            },
            "ts": "1700000002.000300"
        });

        let msg = parse_slack_event(&event, &bot_id, &[]).await.unwrap();
        assert_eq!(msg.thread_id, Some("1700000000.000100".to_string()));
    }

    #[test]
    fn test_slack_adapter_creation() {
        let adapter = SlackAdapter::new(
            "xapp-test".to_string(),
            "xoxb-test".to_string(),
            vec!["C123".to_string()],
        );
        assert_eq!(adapter.name(), "slack");
        assert_eq!(adapter.channel_type(), ChannelType::Slack);
    }

    #[tokio::test]
    async fn test_parse_slack_block_action_basic() {
        let bot_id = Arc::new(RwLock::new(Some("B123".to_string())));
        let interaction = serde_json::json!({
            "type": "block_actions",
            "user": { "id": "U456" },
            "channel": { "id": "C789" },
            "actions": [{
                "action_id": "interactive_0_0",
                "value": "approve_req_001",
                "type": "button"
            }],
            "message": {
                "text": "Approve this request?",
                "ts": "1700000000.000100",
            },
            "trigger_id": "trigger_123"
        });

        let msg = parse_slack_block_action(&interaction, &bot_id, &[])
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Slack);
        assert_eq!(msg.sender.platform_id, "C789");
        assert!(msg.is_group);
        match &msg.content {
            ChannelContent::ButtonCallback {
                action,
                message_text,
            } => {
                assert_eq!(action, "approve_req_001");
                assert_eq!(message_text.as_deref(), Some("Approve this request?"));
            }
            other => panic!("Expected ButtonCallback, got {other:?}"),
        }
        assert_eq!(
            msg.metadata.get("block_action").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            msg.metadata.get("action_id").and_then(|v| v.as_str()),
            Some("interactive_0_0")
        );
    }

    #[tokio::test]
    async fn test_parse_slack_block_action_filters_bot() {
        let bot_id = Arc::new(RwLock::new(Some("U456".to_string())));
        let interaction = serde_json::json!({
            "type": "block_actions",
            "user": { "id": "U456" },
            "channel": { "id": "C789" },
            "actions": [{ "action_id": "a", "value": "v" }],
            "message": { "text": "msg", "ts": "1700000000.000100" },
            "trigger_id": "t"
        });

        let msg = parse_slack_block_action(&interaction, &bot_id, &[]).await;
        assert!(msg.is_none(), "Bot's own block_action should be filtered");
    }

    #[tokio::test]
    async fn test_parse_slack_block_action_filters_channel() {
        let bot_id = Arc::new(RwLock::new(None));
        let interaction = serde_json::json!({
            "type": "block_actions",
            "user": { "id": "U456" },
            "channel": { "id": "C789" },
            "actions": [{ "action_id": "a", "value": "v" }],
            "message": { "text": "msg", "ts": "1700000000.000100" },
            "trigger_id": "t"
        });

        let msg = parse_slack_block_action(&interaction, &bot_id, &["C111".to_string()]).await;
        assert!(
            msg.is_none(),
            "Channel not in allowed list should be filtered"
        );
    }

    #[tokio::test]
    async fn test_parse_slack_block_action_ignores_non_block_actions() {
        let bot_id = Arc::new(RwLock::new(None));
        let interaction = serde_json::json!({
            "type": "view_submission",
            "user": { "id": "U456" },
            "channel": { "id": "C789" },
            "actions": [{ "action_id": "a", "value": "v" }],
        });

        let msg = parse_slack_block_action(&interaction, &bot_id, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_slack_block_action_dm() {
        let bot_id = Arc::new(RwLock::new(None));
        let interaction = serde_json::json!({
            "type": "block_actions",
            "user": { "id": "U456" },
            "channel": { "id": "D789" },
            "actions": [{ "action_id": "a", "value": "dm_action" }],
            "message": { "text": "DM buttons", "ts": "1700000000.000100" },
            "trigger_id": "t"
        });

        let msg = parse_slack_block_action(&interaction, &bot_id, &[])
            .await
            .unwrap();
        assert!(!msg.is_group);
        assert_eq!(msg.sender.platform_id, "D789");
        match &msg.content {
            ChannelContent::ButtonCallback { action, .. } => {
                assert_eq!(action, "dm_action");
            }
            other => panic!("Expected ButtonCallback, got {other:?}"),
        }
    }
}
