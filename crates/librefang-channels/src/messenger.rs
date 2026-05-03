//! Facebook Messenger Platform channel adapter.
//!
//! Uses the Facebook Messenger Platform Send API (Graph API v18.0) for sending
//! messages and a webhook HTTP server for receiving inbound events. The webhook
//! supports both GET (verification challenge) and POST (message events).
//! Authentication uses the page access token as a query parameter on the Send API.

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

/// Verify a Facebook Messenger X-Hub-Signature header using HMAC-SHA1.
///
/// The header format is `sha1=<hex-digest>`.
/// The expected digest is `HMAC-SHA1(app_secret, raw_body)`.
fn verify_hub_signature(app_secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    let hex_sig = match signature_header.strip_prefix("sha1=") {
        Some(s) => s,
        None => return false,
    };

    let Ok(expected_bytes) = hex::decode(hex_sig) else {
        return false;
    };

    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(app_secret) else {
        warn!("Messenger: failed to create HMAC-SHA1 instance");
        return false;
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();

    crate::http_client::ct_eq(&result, &expected_bytes)
}

/// Default Facebook Graph API base URL for sending messages.
const GRAPH_API_BASE: &str = "https://graph.facebook.com/v18.0";

/// Returns the default Graph API base URL. Used to initialise `MessengerAdapter::api_base`.
#[inline]
fn default_messenger_api_base() -> String {
    GRAPH_API_BASE.to_string()
}

/// Maximum Messenger message text length (characters).
const MAX_MESSAGE_LEN: usize = 2000;

/// Facebook Messenger Platform adapter.
///
/// Inbound messages arrive via a webhook HTTP server that supports:
/// - GET requests for Facebook's webhook verification challenge
/// - POST requests for incoming message events
///
/// Outbound messages are sent via the Messenger Send API using
/// the page access token for authentication.
pub struct MessengerAdapter {
    /// SECURITY: Page access token for the Send API, zeroized on drop.
    page_token: Zeroizing<String>,
    /// SECURITY: Verify token for webhook registration, zeroized on drop.
    verify_token: Zeroizing<String>,
    /// SECURITY: App secret for HMAC-SHA1 webhook signature verification.
    /// If empty, signature verification is skipped with a loud warning.
    app_secret: Zeroizing<String>,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Base URL for the Facebook Graph API. Defaults to `https://graph.facebook.com/v18.0`.
    /// Overridable in tests via `with_api_base()` to point at a wiremock server.
    api_base: String,
}

impl MessengerAdapter {
    /// Create a new Messenger adapter.
    ///
    /// # Arguments
    /// * `page_token` - Facebook page access token for the Send API.
    /// * `verify_token` - Token used to verify the webhook during Facebook's setup.
    /// * `app_secret` - Facebook App Secret for HMAC-SHA1 webhook verification.
    ///   Pass an empty string to skip verification (logs a warning).
    /// * `webhook_port` - Local port (accepted from config, unused with shared server).
    pub fn new(
        page_token: String,
        verify_token: String,
        app_secret: String,
        _webhook_port: u16,
    ) -> Self {
        if app_secret.is_empty() {
            warn!(
                "Messenger: no app_secret configured — webhook signature \
                 verification is DISABLED. Set app_secret_env to harden \
                 this endpoint."
            );
        }
        Self {
            page_token: Zeroizing::new(page_token),
            verify_token: Zeroizing::new(verify_token),
            app_secret: Zeroizing::new(app_secret),
            client: crate::http_client::new_client(),
            account_id: None,
            api_base: default_messenger_api_base(),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the Graph API base URL. Intended for tests that point the
    /// adapter at a wiremock server instead of `https://graph.facebook.com/v18.0`.
    #[cfg(test)]
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    /// Validate the page token by calling the Graph API to get page info.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/me?access_token={}",
            self.api_base,
            self.page_token.as_str()
        );

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Messenger authentication failed {status}: {body}").into());
        }

        let body: serde_json::Value = resp.json().await?;
        let page_name = body["name"].as_str().unwrap_or("Messenger Bot").to_string();
        Ok(page_name)
    }

    /// Send a text message to a Messenger user via the Send API.
    async fn api_send_message(
        &self,
        recipient_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/me/messages?access_token={}",
            self.api_base,
            self.page_token.as_str()
        );

        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "recipient": {
                    "id": recipient_id,
                },
                "message": {
                    "text": chunk,
                },
                "messaging_type": "RESPONSE",
            });

            let resp = self.client.post(&url).json(&body).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("Messenger Send API error {status}: {resp_body}").into());
            }
        }

        Ok(())
    }

    /// Send a typing indicator (sender action) to a Messenger user.
    async fn api_send_action(
        &self,
        recipient_id: &str,
        action: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/me/messages?access_token={}",
            self.api_base,
            self.page_token.as_str()
        );

        let body = serde_json::json!({
            "recipient": {
                "id": recipient_id,
            },
            "sender_action": action,
        });

        let _ = self.client.post(&url).json(&body).send().await;
        Ok(())
    }

    /// Mark a message as seen via sender action.
    #[allow(dead_code)]
    async fn mark_seen(
        &self,
        recipient_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_action(recipient_id, "mark_seen").await
    }

    /// Build the Axum router for Messenger webhook events.
    ///
    /// The router handles:
    /// - GET `/webhook` — Facebook webhook verification challenge
    /// - POST `/webhook` — Incoming message events (HMAC-SHA1 verified via X-Hub-Signature)
    fn build_webhook_router(&self, tx: mpsc::Sender<ChannelMessage>) -> axum::Router {
        let verify_token = Arc::new(self.verify_token.clone());
        let app_secret = Arc::new(self.app_secret.clone());
        let account_id = Arc::new(self.account_id.clone());
        let tx = Arc::new(tx);

        axum::Router::new().route(
            "/webhook",
            axum::routing::get({
                // Facebook webhook verification handler
                let vt = Arc::clone(&verify_token);
                move |query: axum::extract::Query<HashMap<String, String>>| {
                    let vt = Arc::clone(&vt);
                    async move {
                        let mode = query.get("hub.mode").map(|s| s.as_str()).unwrap_or("");
                        let token = query
                            .get("hub.verify_token")
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let challenge = query.get("hub.challenge").cloned().unwrap_or_default();

                        if mode == "subscribe" && token == vt.as_str() {
                            info!("Messenger webhook verified");
                            (axum::http::StatusCode::OK, challenge)
                        } else {
                            warn!("Messenger webhook verification failed");
                            (axum::http::StatusCode::FORBIDDEN, String::new())
                        }
                    }
                }
            })
            .post({
                // Incoming message handler — verify HMAC-SHA1 before processing.
                let tx = Arc::clone(&tx);
                let app_secret = Arc::clone(&app_secret);
                let account_id = Arc::clone(&account_id);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = Arc::clone(&tx);
                    let app_secret = Arc::clone(&app_secret);
                    let account_id = Arc::clone(&account_id);
                    async move {
                        // Verify X-Hub-Signature (HMAC-SHA1 with app_secret).
                        // The "no app_secret configured" warning was logged
                        // once at construction time — request path stays quiet.
                        if !app_secret.is_empty() {
                            let Some(sig) =
                                headers.get("x-hub-signature").and_then(|v| v.to_str().ok())
                            else {
                                warn!("Messenger: missing X-Hub-Signature header");
                                return axum::http::StatusCode::BAD_REQUEST;
                            };
                            if !verify_hub_signature(app_secret.as_bytes(), &body, sig) {
                                warn!("Messenger: invalid X-Hub-Signature");
                                return axum::http::StatusCode::UNAUTHORIZED;
                            }
                        }

                        let json_body: serde_json::Value = match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        };

                        let object = json_body["object"].as_str().unwrap_or("");
                        if object != "page" {
                            return axum::http::StatusCode::OK;
                        }

                        if let Some(entries) = json_body["entry"].as_array() {
                            for entry in entries {
                                let msgs = parse_messenger_entry(entry);
                                for mut msg in msgs {
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
                        }

                        axum::http::StatusCode::OK
                    }
                }
            }),
        )
    }
}

/// Parse Facebook Messenger webhook entry into `ChannelMessage` values.
///
/// A single webhook POST can contain multiple entries, each with multiple
/// messaging events. This function processes one entry and returns all
/// valid messages found.
fn parse_messenger_entry(entry: &serde_json::Value) -> Vec<ChannelMessage> {
    let mut messages = Vec::new();

    let messaging = match entry["messaging"].as_array() {
        Some(arr) => arr,
        None => return messages,
    };

    for event in messaging {
        // Only handle message events (not delivery, read, postback, etc.)
        let message = match event.get("message") {
            Some(m) => m,
            None => continue,
        };

        // Skip echo messages (sent by the page itself)
        if message["is_echo"].as_bool().unwrap_or(false) {
            continue;
        }

        let text = match message["text"].as_str() {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        let sender_id = event["sender"]["id"].as_str().unwrap_or("").to_string();
        let recipient_id = event["recipient"]["id"].as_str().unwrap_or("").to_string();
        let msg_id = message["mid"].as_str().unwrap_or("").to_string();
        let timestamp = event["timestamp"].as_u64().unwrap_or(0);

        let content = if text.starts_with('/') {
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
            "sender_id".to_string(),
            serde_json::Value::String(sender_id.clone()),
        );
        metadata.insert(
            "recipient_id".to_string(),
            serde_json::Value::String(recipient_id),
        );
        metadata.insert(
            "timestamp".to_string(),
            serde_json::Value::Number(serde_json::Number::from(timestamp)),
        );

        // Check for quick reply payload
        if let Some(qr) = message.get("quick_reply") {
            if let Some(payload) = qr["payload"].as_str() {
                metadata.insert(
                    "quick_reply_payload".to_string(),
                    serde_json::Value::String(payload.to_string()),
                );
            }
        }

        // Check for NLP entities (if enabled on the page)
        if let Some(nlp) = message.get("nlp") {
            if let Some(entities) = nlp.get("entities") {
                metadata.insert("nlp_entities".to_string(), entities.clone());
            }
        }

        messages.push(ChannelMessage {
            channel: ChannelType::Custom("messenger".to_string()),
            platform_message_id: msg_id,
            sender: ChannelUser {
                platform_id: sender_id,
                display_name: String::new(), // Messenger doesn't include name in webhook
                librefang_user: None,
            },
            content,
            target_agent: None,
            timestamp: Utc::now(),
            is_group: false, // Messenger Bot API is always 1:1
            thread_id: None,
            metadata,
        });
    }

    messages
}

#[async_trait]
impl ChannelAdapter for MessengerAdapter {
    fn name(&self) -> &str {
        "messenger"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("messenger".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Validate credentials
        let page_name = match self.validate().await {
            Ok(name) => name,
            Err(e) => {
                warn!("Messenger adapter validation failed: {e}");
                return None;
            }
        };
        info!("Messenger adapter authenticated as {page_name}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let router = self.build_webhook_router(tx);

        info!("Messenger: registered webhook route on shared server at /channels/messenger");

        Some((
            router,
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
        ))
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Webhook mode is handled by create_webhook_routes().
        // If we reach here, return an empty stream as fallback.
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
            ChannelContent::Image { url, caption, .. } => {
                // Send image attachment via Messenger
                let api_url = format!(
                    "{}/me/messages?access_token={}",
                    self.api_base,
                    self.page_token.as_str()
                );

                let body = serde_json::json!({
                    "recipient": {
                        "id": user.platform_id,
                    },
                    "message": {
                        "attachment": {
                            "type": "image",
                            "payload": {
                                "url": url,
                                "is_reusable": true,
                            }
                        }
                    },
                    "messaging_type": "RESPONSE",
                });

                let resp = self.client.post(&api_url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let resp_body = resp.text().await.unwrap_or_default();
                    warn!("Messenger image send error {status}: {resp_body}");
                }

                // Send caption as a separate text message
                if let Some(cap) = caption {
                    if !cap.is_empty() {
                        self.api_send_message(&user.platform_id, &cap).await?;
                    }
                }
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
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_action(&user.platform_id, "typing_on").await
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- send() path tests (issue #3820) -----
    //
    // Uses `wiremock` to stand up a local HTTP server and points `MessengerAdapter`
    // at it via `with_api_base()`. This exercises the `POST /me/messages` call made
    // by `ChannelAdapter::send`.

    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_adapter(api_base: String) -> MessengerAdapter {
        MessengerAdapter::new(
            "test-page-token".to_string(),
            "test-verify-token".to_string(),
            "test-app-secret".to_string(),
            8080,
        )
        .with_api_base(api_base)
    }

    fn dummy_user(user_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn messenger_send_posts_messages_with_page_token_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/messages"))
            .and(body_json(serde_json::json!({
                "recipient": { "id": "user-psid-123" },
                "message": { "text": "hello from librefang" },
                "messaging_type": "RESPONSE"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "recipient_id": "user-psid-123",
                "message_id": "mid.test123"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("user-psid-123"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn messenger_send_non_text_content_falls_back_to_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/messages"))
            .and(body_json(serde_json::json!({
                "recipient": { "id": "user-psid-456" },
                "message": { "text": "(Unsupported content type)" },
                "messaging_type": "RESPONSE"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "recipient_id": "user-psid-456",
                "message_id": "mid.test456"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("user-psid-456"),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send must succeed with unsupported content");
    }

    #[test]
    fn test_messenger_adapter_creation() {
        let adapter = MessengerAdapter::new(
            "page-token-123".to_string(),
            "verify-token-456".to_string(),
            "app-secret-789".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "messenger");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("messenger".to_string())
        );
    }

    #[test]
    fn test_messenger_both_tokens() {
        let adapter = MessengerAdapter::new(
            "page-tok".to_string(),
            "verify-tok".to_string(),
            "app-sec".to_string(),
            9000,
        );
        assert_eq!(adapter.page_token.as_str(), "page-tok");
        assert_eq!(adapter.verify_token.as_str(), "verify-tok");
        assert_eq!(adapter.app_secret.as_str(), "app-sec");
    }

    #[test]
    fn test_verify_hub_signature_valid() {
        use hmac::{Hmac, Mac};
        use sha1::Sha1;

        let secret = b"my-app-secret";
        let body = b"test payload body";

        let mut mac = Hmac::<Sha1>::new_from_slice(secret).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let hex_sig = format!("sha1={}", hex::encode(result));

        assert!(verify_hub_signature(secret, body, &hex_sig));
    }

    #[test]
    fn test_verify_hub_signature_invalid() {
        let secret = b"my-app-secret";
        let body = b"test payload body";
        assert!(!verify_hub_signature(secret, body, "sha1=deadbeef"));
        assert!(!verify_hub_signature(secret, body, "bad-format"));
        assert!(!verify_hub_signature(secret, body, ""));
    }

    #[test]
    fn test_parse_messenger_entry_text_message() {
        let entry = serde_json::json!({
            "id": "page-id-123",
            "time": 1458692752478_u64,
            "messaging": [
                {
                    "sender": { "id": "user-123" },
                    "recipient": { "id": "page-456" },
                    "timestamp": 1458692752478_u64,
                    "message": {
                        "mid": "mid.123",
                        "text": "Hello from Messenger!"
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].channel,
            ChannelType::Custom("messenger".to_string())
        );
        assert_eq!(msgs[0].sender.platform_id, "user-123");
        assert!(
            matches!(msgs[0].content, ChannelContent::Text(ref t) if t == "Hello from Messenger!")
        );
    }

    #[test]
    fn test_parse_messenger_entry_command() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "user-1" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "message": {
                        "mid": "mid.456",
                        "text": "/models list"
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert_eq!(msgs.len(), 1);
        match &msgs[0].content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "models");
                assert_eq!(args, &["list"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_messenger_entry_skips_echo() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "page-1" },
                    "recipient": { "id": "user-1" },
                    "timestamp": 0,
                    "message": {
                        "mid": "mid.789",
                        "text": "Echo message",
                        "is_echo": true,
                        "app_id": 12345
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_parse_messenger_entry_skips_delivery() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "user-1" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "delivery": {
                        "mids": ["mid.123"],
                        "watermark": 1458668856253_u64
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_parse_messenger_entry_quick_reply() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "user-1" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "message": {
                        "mid": "mid.qr",
                        "text": "Red",
                        "quick_reply": {
                            "payload": "DEVELOPER_DEFINED_PAYLOAD_FOR_RED"
                        }
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].metadata.contains_key("quick_reply_payload"));
    }

    #[test]
    fn test_parse_messenger_entry_empty_text() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "user-1" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "message": {
                        "mid": "mid.empty",
                        "text": ""
                    }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_parse_messenger_entry_multiple_messages() {
        let entry = serde_json::json!({
            "id": "page-id",
            "messaging": [
                {
                    "sender": { "id": "user-1" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "message": { "mid": "mid.1", "text": "First" }
                },
                {
                    "sender": { "id": "user-2" },
                    "recipient": { "id": "page-1" },
                    "timestamp": 0,
                    "message": { "mid": "mid.2", "text": "Second" }
                }
            ]
        });

        let msgs = parse_messenger_entry(&entry);
        assert_eq!(msgs.len(), 2);
    }
}
