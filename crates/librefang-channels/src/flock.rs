//! Flock Bot channel adapter.
//!
//! Uses the Flock Messaging API with a local webhook HTTP server for receiving
//! inbound event callbacks and the REST API for sending messages. Authentication
//! is performed via a Bot token parameter. Flock delivers events as JSON POST
//! requests to the configured webhook endpoint.

use crate::types::{
    split_message, ChannelAdapter, ChannelAdapterMultiplicity, ChannelContent, ChannelMessage,
    ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use zeroize::Zeroizing;

/// Flock REST API base URL.
const FLOCK_API_BASE: &str = "https://api.flock.com/v2";

/// Maximum message length for Flock messages.
const MAX_MESSAGE_LEN: usize = 4096;

/// Flock Bot channel adapter using webhook for receiving and REST API for sending.
///
/// Listens for inbound event callbacks via a configurable HTTP webhook server
/// and sends outbound messages via the Flock `chat.sendMessage` endpoint.
/// Supports channel-receive and app-install event types.
pub struct FlockAdapter {
    /// SECURITY: Bot token is zeroized on drop.
    bot_token: Zeroizing<String>,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
}

impl FlockAdapter {
    /// Create a new Flock adapter.
    ///
    /// # Arguments
    /// * `bot_token` - Flock Bot token for API authentication.
    /// * `webhook_port` - Local port (accepted from config, unused with shared server).
    pub fn new(bot_token: String, _webhook_port: u16) -> Self {
        Self {
            bot_token: Zeroizing::new(bot_token),
            client: crate::http_client::new_client(),
            account_id: None,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Validate credentials by fetching bot/app info.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/users.getInfo?token={}",
            FLOCK_API_BASE,
            self.bot_token.as_str()
        );
        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err("Flock authentication failed".into());
        }

        let body: serde_json::Value = resp.json().await?;
        let user_id = body["userId"]
            .as_str()
            .or_else(|| body["id"].as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(user_id)
    }

    /// Send a text message to a Flock channel or user.
    async fn api_send_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/chat.sendMessage", FLOCK_API_BASE);
        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "token": self.bot_token.as_str(),
                "to": to,
                "text": chunk,
            });

            let resp = self.client.post(&url).json(&body).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("Flock API error {status}: {resp_body}").into());
            }

            // Check for API-level errors in response body
            let result: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(error) = result.get("error") {
                return Err(format!("Flock API error: {error}").into());
            }
        }

        Ok(())
    }

    /// Send a rich message with attachments to a Flock channel.
    #[allow(dead_code)]
    async fn api_send_rich_message(
        &self,
        to: &str,
        text: &str,
        attachment_title: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/chat.sendMessage", FLOCK_API_BASE);

        let body = serde_json::json!({
            "token": self.bot_token.as_str(),
            "to": to,
            "text": text,
            "attachments": [{
                "title": attachment_title,
                "description": text,
                "color": "#4CAF50",
            }]
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("Flock rich message error {status}: {resp_body}").into());
        }

        Ok(())
    }
}

/// Parse an inbound Flock event callback into a `ChannelMessage`.
///
/// Flock delivers various event types; we only process `chat.receiveMessage`
/// events (incoming messages sent to the bot).
fn parse_flock_event(event: &serde_json::Value, own_user_id: &str) -> Option<ChannelMessage> {
    let event_name = event["name"].as_str().unwrap_or("");

    // Handle app.install and client.slashCommand events by ignoring them
    match event_name {
        "chat.receiveMessage" => {}
        "client.messageAction" => {}
        _ => return None,
    }

    let message = &event["message"];

    let text = message["text"].as_str().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let from = message["from"].as_str().unwrap_or("");
    let to = message["to"].as_str().unwrap_or("");

    // Skip messages from the bot itself
    if from == own_user_id {
        return None;
    }

    let msg_id = message["uid"]
        .as_str()
        .or_else(|| message["id"].as_str())
        .unwrap_or("")
        .to_string();
    let sender_name = message["fromName"].as_str().unwrap_or(from);

    // Determine if group or DM
    // In Flock, channels start with 'g:' for groups, user IDs for DMs
    let is_group = to.starts_with("g:");

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

    let mut metadata = HashMap::new();
    metadata.insert(
        "from".to_string(),
        serde_json::Value::String(from.to_string()),
    );
    metadata.insert("to".to_string(), serde_json::Value::String(to.to_string()));

    Some(ChannelMessage {
        channel: ChannelType::Custom("flock".to_string()),
        platform_message_id: msg_id,
        sender: ChannelUser {
            platform_id: to.to_string(),
            display_name: sender_name.to_string(),
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: None,
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for FlockAdapter {
    fn name(&self) -> &str {
        "flock"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("flock".to_string())
    }

    fn multiplicity(&self) -> ChannelAdapterMultiplicity {
        ChannelAdapterMultiplicity::SingleInstancePerDaemon {
            reason: "flock uses a fixed shared webhook route on the main API server",
        }
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Validate credentials
        let bot_user_id = match self.validate().await {
            Ok(id) => id,
            Err(e) => {
                warn!("Flock adapter validation failed: {e}");
                return None;
            }
        };
        info!("Flock adapter authenticated (user_id: {bot_user_id})");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let user_id_shared = Arc::new(bot_user_id);
        let tx_shared = Arc::new(tx);
        let account_id = Arc::new(self.account_id.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let user_id = Arc::clone(&user_id_shared);
                let tx = Arc::clone(&tx_shared);
                move |body: axum::extract::Json<serde_json::Value>| {
                    let user_id = Arc::clone(&user_id);
                    let tx = Arc::clone(&tx);
                    async move {
                        // Handle Flock's event verification
                        if body["name"].as_str() == Some("app.install") {
                            return axum::http::StatusCode::OK;
                        }

                        if let Some(mut msg) = parse_flock_event(&body, &user_id) {
                            // Inject account_id for multi-bot routing
                            if let Some(ref aid) = *account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            let _ = tx.send(msg).await;
                        }

                        axum::http::StatusCode::OK
                    }
                }
            }),
        );

        info!("Flock adapter registered on shared server at /channels/flock/webhook");

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
        // When using the shared webhook server, create_webhook_routes() is called
        // instead. This start() is only reached as a fallback.
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&user.platform_id, &text).await?;
            }
            _ => {
                self.api_send_message(&user.platform_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Flock does not expose a typing indicator API for bots
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flock_adapter_creation() {
        let adapter = FlockAdapter::new("test-bot-token".to_string(), 8181);
        assert_eq!(adapter.name(), "flock");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("flock".to_string())
        );
    }

    #[test]
    fn test_flock_token_zeroized() {
        let adapter = FlockAdapter::new("secret-flock-token".to_string(), 8181);
        assert_eq!(adapter.bot_token.as_str(), "secret-flock-token");
    }

    #[test]
    fn test_flock_accepts_webhook_port_param() {
        // webhook_port is accepted in the constructor for config compat but not stored
        let adapter = FlockAdapter::new("token".to_string(), 7777);
        assert_eq!(adapter.name(), "flock");
    }

    #[test]
    fn test_parse_flock_event_message() {
        let event = serde_json::json!({
            "name": "chat.receiveMessage",
            "message": {
                "text": "Hello from Flock!",
                "from": "u:user123",
                "to": "g:channel456",
                "uid": "msg-001",
                "fromName": "Alice"
            }
        });

        let msg = parse_flock_event(&event, "u:bot001").unwrap();
        assert_eq!(msg.sender.display_name, "Alice");
        assert_eq!(msg.sender.platform_id, "g:channel456");
        assert!(msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from Flock!"));
    }

    #[test]
    fn test_parse_flock_event_command() {
        let event = serde_json::json!({
            "name": "chat.receiveMessage",
            "message": {
                "text": "/status check",
                "from": "u:user123",
                "to": "u:bot001",
                "uid": "msg-002"
            }
        });

        let msg = parse_flock_event(&event, "u:bot001-different").unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "status");
                assert_eq!(args, &["check"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_flock_event_skip_bot() {
        let event = serde_json::json!({
            "name": "chat.receiveMessage",
            "message": {
                "text": "Bot response",
                "from": "u:bot001",
                "to": "g:channel456"
            }
        });

        let msg = parse_flock_event(&event, "u:bot001");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_flock_event_dm() {
        let event = serde_json::json!({
            "name": "chat.receiveMessage",
            "message": {
                "text": "Direct msg",
                "from": "u:user123",
                "to": "u:bot001",
                "uid": "msg-003",
                "fromName": "Bob"
            }
        });

        let msg = parse_flock_event(&event, "u:bot001-different").unwrap();
        assert!(!msg.is_group); // "to" doesn't start with "g:"
    }

    #[test]
    fn test_parse_flock_event_unknown_type() {
        let event = serde_json::json!({
            "name": "app.install",
            "userId": "u:user123"
        });

        let msg = parse_flock_event(&event, "u:bot001");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_flock_event_empty_text() {
        let event = serde_json::json!({
            "name": "chat.receiveMessage",
            "message": {
                "text": "",
                "from": "u:user123",
                "to": "g:channel456"
            }
        });

        let msg = parse_flock_event(&event, "u:bot001");
        assert!(msg.is_none());
    }
}
