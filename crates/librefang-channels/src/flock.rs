//! Flock Bot channel adapter.
//!
//! Uses the Flock Messaging API with a local webhook HTTP server for receiving
//! inbound event callbacks and the REST API for sending messages. Authentication
//! is performed via a Bot token parameter. Flock delivers events as JSON POST
//! requests to the configured webhook endpoint.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
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

/// Returns the default Flock API base URL. Used to initialise `FlockAdapter::api_base`.
#[inline]
fn default_flock_api_base() -> String {
    FLOCK_API_BASE.to_string()
}

/// Flock Bot channel adapter using webhook for receiving and REST API for sending.
///
/// Listens for inbound event callbacks via a configurable HTTP webhook server
/// and sends outbound messages via the Flock `chat.sendMessage` endpoint.
/// Supports channel-receive and app-install event types.
pub struct FlockAdapter {
    /// SECURITY: Bot token is zeroized on drop.
    bot_token: Zeroizing<String>,
    /// Base URL for the Flock REST API. Defaults to `https://api.flock.com/v2`.
    /// Overridable in tests via `with_api_base()` to point at a wiremock server.
    api_base: String,
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
            api_base: default_flock_api_base(),
            client: crate::http_client::new_client(),
            account_id: None,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the Flock API base URL. Intended for tests that point the adapter at
    /// a wiremock server instead of `https://api.flock.com/v2`.
    #[cfg(test)]
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    /// Validate credentials by fetching bot/app info.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/users.getInfo?token={}",
            self.api_base,
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
        let url = format!("{}/chat.sendMessage", self.api_base);
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
        let url = format!("{}/chat.sendMessage", self.api_base);

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

    /// Expose the configured multi-bot `account_id` (typically the Flock
    /// team / app identifier) so the bridge approval listener builds the
    /// same `flock:<account_id>` key the router stores in `channel_defaults`,
    /// scoping ApprovalRequested delivery to the team bound to the
    /// requesting agent (#5003, follow-up to #4985 / #4994).
    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- send() path tests (issue #3820) -----
    //
    // Uses `wiremock` to stand up a local HTTP server and points `FlockAdapter`
    // at it via `with_api_base()`. Exercises the `chat.sendMessage` call made by
    // `ChannelAdapter::send`.

    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_adapter(api_base: String) -> FlockAdapter {
        FlockAdapter::new("test-flock-token".to_string(), 8181).with_api_base(api_base)
    }

    fn dummy_user(channel_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: channel_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn flock_send_posts_chat_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.sendMessage"))
            .and(body_json(serde_json::json!({
                "token": "test-flock-token",
                "to": "g:channel123",
                "text": "hello from librefang",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "uid": "msg-001",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("g:channel123"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn flock_send_unsupported_content_uses_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.sendMessage"))
            .and(body_json(serde_json::json!({
                "token": "test-flock-token",
                "to": "u:user456",
                "text": "(Unsupported content type)",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "uid": "msg-002",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("u:user456"),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send with unsupported content must succeed");
    }

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

    #[test]
    fn test_flock_account_id_default_none() {
        let adapter = FlockAdapter::new("token".to_string(), 0);
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_flock_account_id_returns_configured_value() {
        // #5003: two Flock teams must resolve under distinct
        // `flock:<account_id>` keys via the trait override.
        let adapter =
            FlockAdapter::new("token".to_string(), 0).with_account_id(Some("team-42".to_string()));
        assert_eq!(adapter.account_id(), Some("team-42"));
    }
}
