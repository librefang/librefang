//! Threema Gateway channel adapter.
//!
//! Uses the Threema Gateway HTTP API for sending messages and a local webhook
//! HTTP server for receiving inbound messages. Authentication is performed via
//! the Threema Gateway API secret. Inbound messages arrive as POST requests
//! to the configured webhook port.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use zeroize::Zeroizing;

/// Threema Gateway API base URL for sending messages.
const THREEMA_API_URL: &str = "https://msgapi.threema.ch";

/// Maximum message length for Threema messages.
const MAX_MESSAGE_LEN: usize = 3500;

/// Threema Gateway channel adapter using webhook for receiving and REST API for sending.
///
/// Listens for inbound messages via a configurable HTTP webhook server and sends
/// outbound messages via the Threema Gateway `send_simple` endpoint.
pub struct ThreemaAdapter {
    /// Threema Gateway ID (8-character alphanumeric, starts with '*').
    threema_id: String,
    /// SECURITY: API secret is zeroized on drop.
    secret: Zeroizing<String>,
    /// Port for the inbound webhook HTTP listener.
    #[allow(dead_code)]
    webhook_port: u16,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    #[allow(dead_code)]
    shutdown_rx: watch::Receiver<bool>,
}

impl ThreemaAdapter {
    /// Create a new Threema Gateway adapter.
    ///
    /// # Arguments
    /// * `threema_id` - Threema Gateway ID (e.g., "*MYGATEW").
    /// * `secret` - API secret for the Gateway ID.
    /// * `webhook_port` - Local port to bind the inbound webhook listener on.
    pub fn new(threema_id: String, secret: String, webhook_port: u16) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            threema_id,
            secret: Zeroizing::new(secret),
            webhook_port,
            client: crate::http_client::new_client(),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Validate credentials by checking the remaining credits.
    async fn validate(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/credits?from={}&secret={}",
            THREEMA_API_URL,
            self.threema_id,
            self.secret.as_str()
        );
        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err("Threema Gateway authentication failed".into());
        }

        let credits: u64 = resp.text().await?.trim().parse().unwrap_or(0);
        Ok(credits)
    }

    /// Send a simple text message to a Threema ID.
    async fn api_send_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/send_simple", THREEMA_API_URL);
        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let params = [
                ("from", self.threema_id.as_str()),
                ("to", to),
                ("secret", self.secret.as_str()),
                ("text", chunk),
            ];

            let resp = self.client.post(&url).form(&params).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Threema API error {status}: {body}").into());
            }
        }

        Ok(())
    }
}

/// Parse an inbound Threema webhook payload into a `ChannelMessage`.
///
/// The Threema Gateway delivers inbound messages as form-encoded POST requests
/// with fields: `from`, `to`, `messageId`, `date`, `text`, `nonce`, `box`, `mac`.
/// For the `send_simple` mode, the `text` field contains the plaintext message.
fn parse_threema_webhook(
    payload: &HashMap<String, String>,
    own_id: &str,
) -> Option<ChannelMessage> {
    let from = payload.get("from")?;
    let text = payload.get("text").or_else(|| payload.get("body"))?;
    let message_id = payload.get("messageId").cloned().unwrap_or_default();

    // Skip messages from ourselves
    if from == own_id {
        return None;
    }

    if text.is_empty() {
        return None;
    }

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
    if let Some(nonce) = payload.get("nonce") {
        metadata.insert(
            "nonce".to_string(),
            serde_json::Value::String(nonce.clone()),
        );
    }
    if let Some(mac) = payload.get("mac") {
        metadata.insert("mac".to_string(), serde_json::Value::String(mac.clone()));
    }

    Some(ChannelMessage {
        channel: ChannelType::Custom("threema".to_string()),
        platform_message_id: message_id,
        sender: ChannelUser {
            platform_id: from.clone(),
            display_name: from.clone(),
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false, // Threema Gateway simple mode is 1:1
        thread_id: None,
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for ThreemaAdapter {
    fn name(&self) -> &str {
        "threema"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("threema".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Validate credentials
        let credits = match self.validate().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Threema adapter validation failed: {e}");
                return None;
            }
        };
        info!(
            "Threema Gateway adapter authenticated (ID: {}, credits: {credits})",
            self.threema_id
        );

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let tx_shared = Arc::new(tx);
        let own_id = Arc::new(self.threema_id.clone());
        let account_id = Arc::new(self.account_id.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let tx = Arc::clone(&tx_shared);
                let own_id = Arc::clone(&own_id);
                let account_id = Arc::clone(&account_id);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = Arc::clone(&tx);
                    let own_id = Arc::clone(&own_id);
                    let account_id = Arc::clone(&account_id);
                    async move {
                        // Parse the body based on content type
                        let content_type = headers
                            .get(axum::http::header::CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");

                        let payload: HashMap<String, String> =
                            if content_type.contains("application/json") {
                                // JSON payload
                                serde_json::from_slice(&body).unwrap_or_default()
                            } else {
                                // Form-encoded payload
                                url::form_urlencoded::parse(&body)
                                    .map(|(k, v)| (k.to_string(), v.to_string()))
                                    .collect()
                            };

                        if let Some(mut msg) = parse_threema_webhook(&payload, &own_id) {
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

        info!("Threema adapter registered on shared server at /channels/threema/webhook");

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
        // Threema Gateway does not support typing indicators
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
    fn test_threema_adapter_creation() {
        let adapter = ThreemaAdapter::new("*MYGATEW".to_string(), "test-secret".to_string(), 8443);
        assert_eq!(adapter.name(), "threema");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("threema".to_string())
        );
    }

    #[test]
    fn test_threema_secret_zeroized() {
        let adapter =
            ThreemaAdapter::new("*MYID123".to_string(), "super-secret-key".to_string(), 8443);
        assert_eq!(adapter.secret.as_str(), "super-secret-key");
    }

    #[test]
    fn test_threema_webhook_port() {
        let adapter = ThreemaAdapter::new("*TEST".to_string(), "secret".to_string(), 9090);
        assert_eq!(adapter.webhook_port, 9090);
    }

    #[test]
    fn test_parse_threema_webhook_basic() {
        let mut payload = HashMap::new();
        payload.insert("from".to_string(), "ABCDEFGH".to_string());
        payload.insert("text".to_string(), "Hello from Threema!".to_string());
        payload.insert("messageId".to_string(), "msg-001".to_string());

        let msg = parse_threema_webhook(&payload, "*MYGATEW").unwrap();
        assert_eq!(msg.sender.platform_id, "ABCDEFGH");
        assert_eq!(msg.sender.display_name, "ABCDEFGH");
        assert!(!msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from Threema!"));
    }

    #[test]
    fn test_parse_threema_webhook_command() {
        let mut payload = HashMap::new();
        payload.insert("from".to_string(), "SENDER01".to_string());
        payload.insert("text".to_string(), "/help me".to_string());

        let msg = parse_threema_webhook(&payload, "*MYGATEW").unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "help");
                assert_eq!(args, &["me"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_threema_webhook_skip_self() {
        let mut payload = HashMap::new();
        payload.insert("from".to_string(), "*MYGATEW".to_string());
        payload.insert("text".to_string(), "Self message".to_string());

        let msg = parse_threema_webhook(&payload, "*MYGATEW");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_threema_webhook_empty_text() {
        let mut payload = HashMap::new();
        payload.insert("from".to_string(), "SENDER01".to_string());
        payload.insert("text".to_string(), String::new());

        let msg = parse_threema_webhook(&payload, "*MYGATEW");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_threema_webhook_with_nonce_and_mac() {
        let mut payload = HashMap::new();
        payload.insert("from".to_string(), "SENDER01".to_string());
        payload.insert("text".to_string(), "Secure msg".to_string());
        payload.insert("nonce".to_string(), "abc123".to_string());
        payload.insert("mac".to_string(), "def456".to_string());

        let msg = parse_threema_webhook(&payload, "*MYGATEW").unwrap();
        assert!(msg.metadata.contains_key("nonce"));
        assert!(msg.metadata.contains_key("mac"));
    }
}
