//! LINE Messaging API channel adapter.
//!
//! Uses the LINE Messaging API v2 for sending push/reply messages and a lightweight
//! axum HTTP webhook server for receiving inbound events. Webhook signature
//! verification uses HMAC-SHA256 with the channel secret. Authentication for
//! outbound calls uses `Authorization: Bearer {channel_access_token}`.

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

/// LINE push message API endpoint.
const LINE_PUSH_URL: &str = "https://api.line.me/v2/bot/message/push";

/// LINE reply message API endpoint.
const LINE_REPLY_URL: &str = "https://api.line.me/v2/bot/message/reply";

/// LINE profile API endpoint.
#[allow(dead_code)]
const LINE_PROFILE_URL: &str = "https://api.line.me/v2/bot/profile";

/// Maximum LINE message text length (characters).
const MAX_MESSAGE_LEN: usize = 5000;

/// LINE Messaging API adapter.
///
/// Inbound messages arrive via an axum HTTP webhook server that accepts POST
/// requests from the LINE Platform. Each request body is validated using
/// HMAC-SHA256 (`X-Line-Signature` header) with the channel secret.
///
/// Outbound messages are sent via the push message API with a bearer token.
pub struct LineAdapter {
    /// SECURITY: Channel secret for webhook signature verification, zeroized on drop.
    channel_secret: Zeroizing<String>,
    /// SECURITY: Channel access token for outbound API calls, zeroized on drop.
    access_token: Zeroizing<String>,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
}

impl LineAdapter {
    /// Create a new LINE adapter.
    ///
    /// # Arguments
    /// * `channel_secret` - Channel secret for HMAC-SHA256 signature verification.
    /// * `access_token` - Long-lived channel access token for sending messages.
    /// * `webhook_port` - Local port for the inbound webhook HTTP server.
    pub fn new(channel_secret: String, access_token: String, _webhook_port: u16) -> Self {
        Self {
            channel_secret: Zeroizing::new(channel_secret),
            access_token: Zeroizing::new(access_token),
            client: crate::http_client::new_client(),
            account_id: None,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Verify the X-Line-Signature header using HMAC-SHA256.
    ///
    /// The signature is computed as `Base64(HMAC-SHA256(channel_secret, body))`.
    #[allow(dead_code)]
    fn verify_signature(&self, body: &[u8], signature: &str) -> bool {
        verify_line_signature(self.channel_secret.as_bytes(), body, signature)
    }

    /// Validate the channel access token by fetching the bot's own profile.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Verify token by calling the bot info endpoint
        let resp = self
            .client
            .get("https://api.line.me/v2/bot/info")
            .bearer_auth(self.access_token.as_str())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("LINE authentication failed {status}: {body}").into());
        }

        let body: serde_json::Value = resp.json().await?;
        let display_name = body["displayName"]
            .as_str()
            .unwrap_or("LINE Bot")
            .to_string();
        Ok(display_name)
    }

    /// Fetch a user's display name from the LINE profile API.
    #[allow(dead_code)]
    async fn get_user_display_name(&self, user_id: &str) -> String {
        let url = format!("{}/{}", LINE_PROFILE_URL, user_id);
        match self
            .client
            .get(&url)
            .bearer_auth(self.access_token.as_str())
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                body["displayName"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string()
            }
            _ => "Unknown".to_string(),
        }
    }

    /// Send a push message to a LINE user or group.
    async fn api_push_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "to": to,
                "messages": [
                    {
                        "type": "text",
                        "text": chunk,
                    }
                ]
            });

            let resp = self
                .client
                .post(LINE_PUSH_URL)
                .bearer_auth(self.access_token.as_str())
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                return Err(format!("LINE push API error {status}: {resp_body}").into());
            }
        }

        Ok(())
    }

    /// Send a reply message using a reply token (must be used within 30s).
    #[allow(dead_code)]
    async fn api_reply_message(
        &self,
        reply_token: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chunks = split_message(text, MAX_MESSAGE_LEN);
        // LINE reply API allows up to 5 messages per reply
        let messages: Vec<serde_json::Value> = chunks
            .into_iter()
            .take(5)
            .map(|chunk| {
                serde_json::json!({
                    "type": "text",
                    "text": chunk,
                })
            })
            .collect();

        let body = serde_json::json!({
            "replyToken": reply_token,
            "messages": messages,
        });

        let resp = self
            .client
            .post(LINE_REPLY_URL)
            .bearer_auth(self.access_token.as_str())
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("LINE reply API error {status}: {resp_body}").into());
        }

        Ok(())
    }
}

/// Verify X-Line-Signature using HMAC-SHA256 with constant-time comparison.
///
/// The expected signature is `Base64(HMAC-SHA256(channel_secret, body))`.
fn verify_line_signature(secret: &[u8], body: &[u8], signature: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        warn!("LINE: failed to create HMAC instance");
        return false;
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();

    use base64::Engine;
    let Ok(expected) = base64::engine::general_purpose::STANDARD.decode(signature) else {
        warn!("LINE: invalid base64 in X-Line-Signature");
        return false;
    };

    crate::http_client::ct_eq(&result, &expected)
}

/// Parse a LINE webhook event into a `ChannelMessage`.
///
/// Handles `message` events with text type. Returns `None` for unsupported
/// event types (follow, unfollow, postback, beacon, etc.).
fn parse_line_event(event: &serde_json::Value) -> Option<ChannelMessage> {
    let event_type = event["type"].as_str().unwrap_or("");
    if event_type != "message" {
        return None;
    }

    let message = event.get("message")?;
    let msg_type = message["type"].as_str().unwrap_or("");

    // Only handle text messages for now
    if msg_type != "text" {
        return None;
    }

    let text = message["text"].as_str().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let source = event.get("source")?;
    let source_type = source["type"].as_str().unwrap_or("user");
    let user_id = source["userId"].as_str().unwrap_or("").to_string();

    // Determine the target (user, group, or room) for replies
    let (reply_to, is_group) = match source_type {
        "group" => {
            let group_id = source["groupId"].as_str().unwrap_or("").to_string();
            (group_id, true)
        }
        "room" => {
            let room_id = source["roomId"].as_str().unwrap_or("").to_string();
            (room_id, true)
        }
        _ => (user_id.clone(), false),
    };

    let msg_id = message["id"].as_str().unwrap_or("").to_string();
    let reply_token = event["replyToken"].as_str().unwrap_or("").to_string();

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
        "user_id".to_string(),
        serde_json::Value::String(user_id.clone()),
    );
    metadata.insert(
        "reply_to".to_string(),
        serde_json::Value::String(reply_to.clone()),
    );
    if !reply_token.is_empty() {
        metadata.insert(
            "reply_token".to_string(),
            serde_json::Value::String(reply_token),
        );
    }
    metadata.insert(
        "source_type".to_string(),
        serde_json::Value::String(source_type.to_string()),
    );

    Some(ChannelMessage {
        channel: ChannelType::Custom("line".to_string()),
        platform_message_id: msg_id,
        sender: ChannelUser {
            platform_id: reply_to,
            display_name: user_id,
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
impl ChannelAdapter for LineAdapter {
    fn name(&self) -> &str {
        "line"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("line".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Validate credentials
        let bot_name = match self.validate().await {
            Ok(name) => name,
            Err(e) => {
                warn!("LINE adapter validation failed: {e}");
                return None;
            }
        };
        info!("LINE adapter authenticated as {bot_name}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let tx = Arc::new(tx);
        let channel_secret = Arc::new(self.channel_secret.clone());
        let account_id = Arc::new(self.account_id.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let secret = Arc::clone(&channel_secret);
                let tx = Arc::clone(&tx);
                let account_id = Arc::clone(&account_id);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let secret = Arc::clone(&secret);
                    let tx = Arc::clone(&tx);
                    let account_id = Arc::clone(&account_id);
                    async move {
                        // Verify X-Line-Signature — header is mandatory.
                        // The HMAC must cover the *raw wire bytes*, not bytes
                        // round-tripped through `serde_json::Value` (which
                        // would normalize key order, whitespace, and number
                        // formatting and never match LINE's actual digest).
                        let Some(signature) = headers
                            .get("x-line-signature")
                            .and_then(|v| v.to_str().ok())
                        else {
                            warn!("LINE: missing X-Line-Signature header");
                            return axum::http::StatusCode::BAD_REQUEST;
                        };

                        if !verify_line_signature(secret.as_bytes(), &body, signature) {
                            warn!("LINE: invalid webhook signature");
                            return axum::http::StatusCode::UNAUTHORIZED;
                        }

                        let body_json: serde_json::Value = match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        };

                        // Parse events array
                        if let Some(events) = body_json["events"].as_array() {
                            for event in events {
                                if let Some(mut msg) = parse_line_event(event) {
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
        );

        info!("LINE adapter registered webhook routes on shared server at /channels/line/webhook");

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
        // Webhook mode is handled by create_webhook_routes().
        // This start() is only reached as a fallback (shouldn't happen
        // in normal operation since BridgeManager prefers create_webhook_routes).
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
                self.api_push_message(&user.platform_id, &text).await?;
            }
            ChannelContent::Image { url, caption, .. } => {
                // LINE supports image messages with a preview
                let body = serde_json::json!({
                    "to": user.platform_id,
                    "messages": [
                        {
                            "type": "image",
                            "originalContentUrl": url,
                            "previewImageUrl": url,
                        }
                    ]
                });

                let resp = self
                    .client
                    .post(LINE_PUSH_URL)
                    .bearer_auth(self.access_token.as_str())
                    .json(&body)
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let resp_body = resp.text().await.unwrap_or_default();
                    warn!("LINE image push error {status}: {resp_body}");
                }

                // Send caption as separate text if present
                if let Some(cap) = caption {
                    if !cap.is_empty() {
                        self.api_push_message(&user.platform_id, &cap).await?;
                    }
                }
            }
            _ => {
                self.api_push_message(&user.platform_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // LINE does not support typing indicators via REST API
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
    fn test_line_adapter_creation() {
        let adapter = LineAdapter::new(
            "channel-secret-123".to_string(),
            "access-token-456".to_string(),
            8080,
        );
        assert_eq!(adapter.name(), "line");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("line".to_string())
        );
    }

    #[test]
    fn test_line_adapter_both_tokens() {
        let adapter = LineAdapter::new("secret".to_string(), "token".to_string(), 9000);
        // Verify both secrets are stored as Zeroizing
        assert_eq!(adapter.channel_secret.as_str(), "secret");
        assert_eq!(adapter.access_token.as_str(), "token");
    }

    #[test]
    fn test_parse_line_event_text_message() {
        let event = serde_json::json!({
            "type": "message",
            "replyToken": "reply-token-123",
            "source": {
                "type": "user",
                "userId": "U1234567890"
            },
            "message": {
                "id": "msg-001",
                "type": "text",
                "text": "Hello from LINE!"
            }
        });

        let msg = parse_line_event(&event).unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("line".to_string()));
        assert_eq!(msg.platform_message_id, "msg-001");
        assert!(!msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from LINE!"));
        assert!(msg.metadata.contains_key("reply_token"));
    }

    #[test]
    fn test_parse_line_event_group_message() {
        let event = serde_json::json!({
            "type": "message",
            "replyToken": "reply-token-456",
            "source": {
                "type": "group",
                "groupId": "C1234567890",
                "userId": "U1234567890"
            },
            "message": {
                "id": "msg-002",
                "type": "text",
                "text": "Group message"
            }
        });

        let msg = parse_line_event(&event).unwrap();
        assert!(msg.is_group);
        assert_eq!(msg.sender.platform_id, "C1234567890");
    }

    #[test]
    fn test_parse_line_event_command() {
        let event = serde_json::json!({
            "type": "message",
            "replyToken": "rt",
            "source": {
                "type": "user",
                "userId": "U123"
            },
            "message": {
                "id": "msg-003",
                "type": "text",
                "text": "/status all"
            }
        });

        let msg = parse_line_event(&event).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "status");
                assert_eq!(args, &["all"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_line_event_non_message() {
        let event = serde_json::json!({
            "type": "follow",
            "replyToken": "rt",
            "source": {
                "type": "user",
                "userId": "U123"
            }
        });

        assert!(parse_line_event(&event).is_none());
    }

    #[test]
    fn test_parse_line_event_non_text() {
        let event = serde_json::json!({
            "type": "message",
            "replyToken": "rt",
            "source": {
                "type": "user",
                "userId": "U123"
            },
            "message": {
                "id": "msg-004",
                "type": "sticker",
                "packageId": "1",
                "stickerId": "1"
            }
        });

        assert!(parse_line_event(&event).is_none());
    }

    #[test]
    fn test_verify_line_signature_round_trip() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = b"channel-secret-bytes";
        let body = br#"{"events":[{"type":"message","message":{"text":"hi"}}],"destination":"U1"}"#;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        let digest = mac.finalize().into_bytes();
        let sig = base64::engine::general_purpose::STANDARD.encode(digest);

        assert!(verify_line_signature(secret, body, &sig));
        // Wrong secret → reject.
        assert!(!verify_line_signature(b"different-secret", body, &sig));
        // Mutated body → reject.
        let mutated =
            br#"{"events":[{"type":"message","message":{"text":"HI"}}],"destination":"U1"}"#;
        assert!(!verify_line_signature(secret, mutated, &sig));
    }

    /// Regression test for the wire-bytes vs JSON-roundtrip bug: LINE's
    /// HMAC must verify the raw bytes the platform sent, not the bytes
    /// produced by re-serializing `serde_json::Value`. The two diverge in
    /// at least key ordering and whitespace, so the roundtripped form
    /// will never match the original digest.
    #[test]
    fn test_line_signature_breaks_when_body_round_tripped_through_value() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let secret = b"channel-secret-bytes";
        // Wire body has b before a, plus extra whitespace — a real LINE
        // payload's exact byte layout is up to LINE.
        let wire_body = br#"{"b":1,  "a":2}"#;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(wire_body);
        let digest = mac.finalize().into_bytes();
        let sig = base64::engine::general_purpose::STANDARD.encode(digest);

        assert!(verify_line_signature(secret, wire_body, &sig));

        // Round-trip through serde_json::Value re-orders keys and removes
        // whitespace, so the digest no longer matches. (This is exactly
        // what the previous handler was doing, which would have rejected
        // every legitimate LINE webhook.)
        let value: serde_json::Value = serde_json::from_slice(wire_body).unwrap();
        let round_tripped = serde_json::to_vec(&value).unwrap();
        assert_ne!(wire_body.as_slice(), round_tripped.as_slice());
        assert!(!verify_line_signature(secret, &round_tripped, &sig));
    }

    #[test]
    fn test_parse_line_event_room_source() {
        let event = serde_json::json!({
            "type": "message",
            "replyToken": "rt",
            "source": {
                "type": "room",
                "roomId": "R1234567890",
                "userId": "U123"
            },
            "message": {
                "id": "msg-005",
                "type": "text",
                "text": "Room message"
            }
        });

        let msg = parse_line_event(&event).unwrap();
        assert!(msg.is_group);
        assert_eq!(msg.sender.platform_id, "R1234567890");
    }

    /// Regression for #3439: empty / non-base64 sig must never pass HMAC.
    #[test]
    fn test_verify_line_signature_rejects_empty_signature() {
        let secret = b"channel-secret-bytes";
        let body = br#"{"events":[]}"#;
        assert!(!verify_line_signature(secret, body, ""));
        assert!(!verify_line_signature(secret, body, "   "));
        assert!(!verify_line_signature(secret, body, "not-base64!@#"));
    }
}
