//! Pumble Bot channel adapter.
//!
//! Uses the Pumble Bot API with a local webhook HTTP server for receiving
//! inbound event subscriptions and the REST API for sending messages.
//! Authentication is performed via a Bot Bearer token. Inbound events arrive
//! as JSON POST requests to the configured webhook port.

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

/// Pumble REST API base URL.
const PUMBLE_API_BASE: &str = "https://api.pumble.com/v1";

/// Returns the default Pumble REST API base URL. Used to initialise `PumbleAdapter::api_base`.
#[inline]
fn default_pumble_api_base() -> String {
    PUMBLE_API_BASE.to_string()
}

/// Maximum message length for Pumble messages.
const MAX_MESSAGE_LEN: usize = 4000;

/// Pumble Bot channel adapter using webhook for receiving and REST API for sending.
///
/// Listens for inbound events via a configurable HTTP webhook server and sends
/// outbound messages via the Pumble REST API. Supports Pumble's event subscription
/// model including URL verification challenges.
pub struct PumbleAdapter {
    /// SECURITY: Bot token is zeroized on drop.
    bot_token: Zeroizing<String>,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Base URL for the Pumble REST API. Defaults to `https://api.pumble.com/v1`.
    /// Overridable in tests via `with_api_base()` to point at a wiremock server.
    api_base: String,
}

impl PumbleAdapter {
    /// Create a new Pumble adapter.
    ///
    /// # Arguments
    /// * `bot_token` - Pumble Bot access token.
    /// * `webhook_port` - Local port (accepted from config, unused with shared server).
    pub fn new(bot_token: String, _webhook_port: u16) -> Self {
        Self {
            bot_token: Zeroizing::new(bot_token),
            client: crate::http_client::new_client(),
            account_id: None,
            api_base: default_pumble_api_base(),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the Pumble REST API base URL. Intended for tests that point the
    /// adapter at a wiremock server instead of `https://api.pumble.com/v1`.
    #[cfg(test)]
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    /// Validate credentials by fetching bot info from the Pumble API.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/auth.test", self.api_base);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(self.bot_token.as_str())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err("Pumble authentication failed".into());
        }

        let body: serde_json::Value = resp.json().await?;
        let bot_id = body["user_id"]
            .as_str()
            .or_else(|| body["bot_id"].as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(bot_id)
    }

    /// Send a text message to a Pumble channel.
    async fn api_send_message(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/messages", self.api_base);
        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "channel": channel_id,
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.bot_token.as_str())
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("Pumble API error {status}: {resp_body}").into());
            }
        }

        Ok(())
    }
}

/// Parse an inbound Pumble event JSON into a `ChannelMessage`.
///
/// Returns `None` for non-message events, URL verification challenges,
/// or messages from the bot itself.
fn parse_pumble_event(event: &serde_json::Value, own_bot_id: &str) -> Option<ChannelMessage> {
    let event_type = event["type"].as_str().unwrap_or("");

    // Handle URL verification challenge
    if event_type == "url_verification" {
        return None;
    }

    // Only process message events
    if event_type != "message" && event_type != "message.new" {
        return None;
    }

    let text = event["text"]
        .as_str()
        .or_else(|| event["message"]["text"].as_str())
        .unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let user_id = event["user"]
        .as_str()
        .or_else(|| event["user_id"].as_str())
        .unwrap_or("");

    // Skip messages from the bot itself
    if user_id == own_bot_id {
        return None;
    }

    let channel_id = event["channel"]
        .as_str()
        .or_else(|| event["channel_id"].as_str())
        .unwrap_or("")
        .to_string();
    let ts = event["ts"]
        .as_str()
        .or_else(|| event["timestamp"].as_str())
        .unwrap_or("")
        .to_string();
    let thread_ts = event["thread_ts"].as_str().map(String::from);
    let user_name = event["user_name"].as_str().unwrap_or("unknown");
    let channel_type = event["channel_type"].as_str().unwrap_or("channel");
    let is_group = channel_type != "im";

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
        "user_id".to_string(),
        serde_json::Value::String(user_id.to_string()),
    );
    if !ts.is_empty() {
        metadata.insert("ts".to_string(), serde_json::Value::String(ts.clone()));
    }

    Some(ChannelMessage {
        channel: ChannelType::Custom("pumble".to_string()),
        platform_message_id: ts,
        sender: ChannelUser {
            platform_id: channel_id,
            display_name: user_name.to_string(),
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: thread_ts,
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for PumbleAdapter {
    fn name(&self) -> &str {
        "pumble"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("pumble".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Validate credentials
        let bot_id = match self.validate().await {
            Ok(id) => id,
            Err(e) => {
                warn!("Pumble adapter validation failed: {e}");
                return None;
            }
        };
        info!("Pumble adapter authenticated (bot_id: {bot_id})");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let bot_id_shared = Arc::new(bot_id);
        let tx_shared = Arc::new(tx);
        let account_id = Arc::new(self.account_id.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let bot_id = Arc::clone(&bot_id_shared);
                let tx = Arc::clone(&tx_shared);
                move |body: axum::extract::Json<serde_json::Value>| {
                    let bot_id = Arc::clone(&bot_id);
                    let tx = Arc::clone(&tx);
                    async move {
                        // Handle URL verification challenge
                        if body["type"].as_str() == Some("url_verification") {
                            let challenge = body["challenge"].as_str().unwrap_or("").to_string();
                            return (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({ "challenge": challenge })),
                            );
                        }

                        if let Some(mut msg) = parse_pumble_event(&body, &bot_id) {
                            // Inject account_id for multi-bot routing
                            if let Some(ref aid) = *account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            let _ = tx.send(msg).await;
                        }

                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({})),
                        )
                    }
                }
            }),
        );

        info!("Pumble adapter registered on shared server at /channels/pumble/webhook");

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

        let url = format!("{}/messages", self.api_base);
        let chunks = split_message(&text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "channel": user.platform_id,
                "text": chunk,
                "thread_ts": thread_id,
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.bot_token.as_str())
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("Pumble thread reply error {status}: {resp_body}").into());
            }
        }

        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Pumble does not expose a public typing indicator API for bots
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Expose the configured multi-bot `account_id` (typically the Pumble
    /// workspace / bot identifier) so the bridge approval listener builds
    /// the same `pumble:<account_id>` key the router stores in
    /// `channel_defaults`, scoping ApprovalRequested delivery to the
    /// workspace bound to the requesting agent (#5003, follow-up to
    /// #4985 / #4994).
    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- send() path tests (issue #3820) -----
    //
    // Uses `wiremock` to stand up a local HTTP server and points `PumbleAdapter`
    // at it via `with_api_base()`. This exercises the `POST /messages` call made
    // by `ChannelAdapter::send`.

    use wiremock::matchers::{bearer_token, body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_adapter(api_base: String) -> PumbleAdapter {
        PumbleAdapter::new("test-pumble-token".to_string(), 8080).with_api_base(api_base)
    }

    fn dummy_user(channel_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: channel_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn pumble_send_posts_messages_with_bearer_auth_and_channel() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .and(bearer_token("test-pumble-token"))
            .and(body_json(serde_json::json!({
                "channel": "C-channel-abc",
                "text": "hello from librefang"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "ts": "1700000000.000001"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("C-channel-abc"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn pumble_send_non_text_content_falls_back_to_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .and(bearer_token("test-pumble-token"))
            .and(body_json(serde_json::json!({
                "channel": "C-channel-xyz",
                "text": "(Unsupported content type)"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "ts": "1700000000.000002"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("C-channel-xyz"),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send must succeed with unsupported content");
    }

    #[test]
    fn test_pumble_adapter_creation() {
        let adapter = PumbleAdapter::new("test-bot-token".to_string(), 8080);
        assert_eq!(adapter.name(), "pumble");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("pumble".to_string())
        );
    }

    #[test]
    fn test_pumble_token_zeroized() {
        let adapter = PumbleAdapter::new("secret-pumble-token".to_string(), 8080);
        assert_eq!(adapter.bot_token.as_str(), "secret-pumble-token");
    }

    #[test]
    fn test_pumble_accepts_webhook_port_param() {
        // webhook_port is accepted in the constructor for config compat but not stored
        let adapter = PumbleAdapter::new("token".to_string(), 9999);
        assert_eq!(adapter.name(), "pumble");
    }

    #[test]
    fn test_parse_pumble_event_message() {
        let event = serde_json::json!({
            "type": "message",
            "text": "Hello from Pumble!",
            "user": "U12345",
            "channel": "C67890",
            "ts": "1234567890.123456",
            "user_name": "alice",
            "channel_type": "channel"
        });

        let msg = parse_pumble_event(&event, "BOT001").unwrap();
        assert_eq!(msg.sender.display_name, "alice");
        assert_eq!(msg.sender.platform_id, "C67890");
        assert!(msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from Pumble!"));
    }

    #[test]
    fn test_parse_pumble_event_command() {
        let event = serde_json::json!({
            "type": "message",
            "text": "/help agents",
            "user": "U12345",
            "channel": "C67890",
            "ts": "ts1",
            "user_name": "bob"
        });

        let msg = parse_pumble_event(&event, "BOT001").unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "help");
                assert_eq!(args, &["agents"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_pumble_event_skip_bot() {
        let event = serde_json::json!({
            "type": "message",
            "text": "Bot message",
            "user": "BOT001",
            "channel": "C67890",
            "ts": "ts1"
        });

        let msg = parse_pumble_event(&event, "BOT001");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_pumble_event_url_verification() {
        let event = serde_json::json!({
            "type": "url_verification",
            "challenge": "abc123"
        });

        let msg = parse_pumble_event(&event, "BOT001");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_pumble_event_dm() {
        let event = serde_json::json!({
            "type": "message",
            "text": "Direct message",
            "user": "U12345",
            "channel": "D11111",
            "ts": "ts2",
            "user_name": "carol",
            "channel_type": "im"
        });

        let msg = parse_pumble_event(&event, "BOT001").unwrap();
        assert!(!msg.is_group);
    }

    #[test]
    fn test_parse_pumble_event_with_thread() {
        let event = serde_json::json!({
            "type": "message",
            "text": "Thread reply",
            "user": "U12345",
            "channel": "C67890",
            "ts": "ts3",
            "thread_ts": "ts1",
            "user_name": "dave"
        });

        let msg = parse_pumble_event(&event, "BOT001").unwrap();
        assert_eq!(msg.thread_id.as_deref(), Some("ts1"));
    }

    #[test]
    fn test_pumble_account_id_default_none() {
        let adapter = PumbleAdapter::new("token".to_string(), 0);
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_pumble_account_id_returns_configured_value() {
        // #5003: two Pumble workspaces must resolve under distinct
        // `pumble:<account_id>` keys via the trait override.
        let adapter = PumbleAdapter::new("token".to_string(), 0)
            .with_account_id(Some("workspace-42".to_string()));
        assert_eq!(adapter.account_id(), Some("workspace-42"));
    }
}
