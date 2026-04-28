//! Generic HTTP webhook channel adapter.
//!
//! Provides a bidirectional webhook integration point. Incoming messages are
//! received via an HTTP server that verifies `X-Webhook-Signature` (HMAC-SHA256
//! of the request body). Outbound messages are POSTed to a configurable
//! callback URL with the same signature scheme.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MESSAGE_LEN: usize = 65535;

/// Generic HTTP webhook channel adapter.
///
/// The most flexible adapter in the LibreFang channel suite. Any system that
/// can send/receive HTTP requests with HMAC-SHA256 signatures can integrate
/// through this adapter.
///
/// ## Inbound (receiving)
///
/// Listens on `listen_port` for `POST /webhook` (or `POST /`) requests.
/// Each request must include an `X-Webhook-Signature` header containing
/// `sha256=<hex-digest>` where the digest is `HMAC-SHA256(secret, body)`.
///
/// Expected JSON body:
/// ```json
/// {
///   "sender_id": "user-123",
///   "sender_name": "Alice",
///   "message": "Hello!",
///   "thread_id": "optional-thread",
///   "is_group": false,
///   "metadata": {}
/// }
/// ```
///
/// ## Outbound (sending)
///
/// If `callback_url` is set, messages are POSTed there with the same signature
/// scheme.
pub struct WebhookAdapter {
    /// SECURITY: Shared secret for HMAC-SHA256 signatures (zeroized on drop).
    secret: Zeroizing<String>,
    /// Optional callback URL for sending messages.
    callback_url: Option<String>,
    /// HTTP client for outbound requests.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// When true, incoming messages are forwarded directly to `deliver_target`
    /// without invoking an agent or LLM.
    deliver_only: bool,
    /// Target channel name for direct delivery (e.g. "telegram"). Used only
    /// when `deliver_only` is true.
    deliver_target: Option<String>,
}

impl WebhookAdapter {
    /// Create a new generic webhook adapter.
    ///
    /// # Arguments
    /// * `secret` - Shared secret for HMAC-SHA256 signature verification.
    /// * `listen_port` - Port to listen for incoming webhook POST requests.
    /// * `callback_url` - Optional URL to POST outbound messages to.
    pub fn new(secret: String, _listen_port: u16, callback_url: Option<String>) -> Self {
        Self {
            secret: Zeroizing::new(secret),
            callback_url,
            client: crate::http_client::new_client(),
            account_id: None,
            deliver_only: false,
            deliver_target: None,
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Enable direct-delivery mode. Returns self for builder chaining.
    ///
    /// When enabled, incoming messages are tagged with `__deliver_only__` and
    /// `__deliver_target__` metadata so the bridge can forward them directly to
    /// the specified channel without invoking an agent or LLM.
    pub fn with_deliver_only(mut self, deliver_only: bool, deliver_target: Option<String>) -> Self {
        self.deliver_only = deliver_only;
        self.deliver_target = deliver_target;
        self
    }

    /// Compute HMAC-SHA256 signature of data with the shared secret.
    ///
    /// Returns the hex-encoded digest prefixed with "sha256=".
    fn compute_signature(secret: &str, data: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(data);
        let result = mac.finalize();
        let hex = hex::encode(result.into_bytes());
        format!("sha256={hex}")
    }

    /// Verify an incoming webhook signature (constant-time comparison).
    fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
        let expected = Self::compute_signature(secret, body);
        if expected.len() != signature.len() {
            return false;
        }
        // Constant-time comparison to prevent timing attacks
        let mut diff = 0u8;
        for (a, b) in expected.bytes().zip(signature.bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }

    /// Verify an incoming webhook request: signature validity **and** timestamp
    /// freshness (replay protection).
    ///
    /// `signature` must be the value of the `X-Webhook-Signature` header
    /// (`"sha256=<hex>"`), or an empty string if the header is absent.
    /// `ts_secs` is the Unix timestamp (seconds) carried in
    /// `X-Webhook-Timestamp`; pass `None` when the header is absent.
    /// `now_secs` is the current Unix timestamp used for the freshness check
    /// (injected so tests can control clock values).
    ///
    /// Returns `Ok(())` if the request is valid, or an `Err` string describing
    /// the rejection reason.
    pub fn verify_request(
        secret: &str,
        body: &[u8],
        signature: &str,
        ts_secs: Option<i64>,
        now_secs: i64,
    ) -> Result<(), &'static str> {
        // 1. Require a non-empty signature header.
        if signature.is_empty() {
            return Err("missing signature");
        }

        // 2. Require a timestamp header.
        let ts = ts_secs.ok_or("missing timestamp")?;

        // 3. Reject stale or future timestamps (replay protection, ±5 min).
        const MAX_SKEW_SECS: i64 = 5 * 60;
        let skew = now_secs - ts;
        if skew > MAX_SKEW_SECS {
            return Err("timestamp too old");
        }
        if skew < -MAX_SKEW_SECS {
            return Err("timestamp in the future");
        }

        // 4. Verify the HMAC signature.
        if !Self::verify_signature(secret, body, signature) {
            return Err("invalid signature");
        }

        Ok(())
    }

    /// Parse an incoming webhook JSON body.
    #[allow(clippy::type_complexity)]
    fn parse_webhook_body(
        body: &serde_json::Value,
    ) -> Option<(
        String,
        String,
        String,
        Option<String>,
        bool,
        HashMap<String, serde_json::Value>,
    )> {
        let message = body["message"].as_str()?.to_string();
        if message.is_empty() {
            return None;
        }

        let sender_id = body["sender_id"]
            .as_str()
            .unwrap_or("webhook-user")
            .to_string();
        let sender_name = body["sender_name"]
            .as_str()
            .unwrap_or("Webhook User")
            .to_string();
        let thread_id = body["thread_id"].as_str().map(String::from);
        let is_group = body["is_group"].as_bool().unwrap_or(false);

        let metadata = body["metadata"]
            .as_object()
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        Some((
            message,
            sender_id,
            sender_name,
            thread_id,
            is_group,
            metadata,
        ))
    }

    /// Check if a callback URL is configured.
    pub fn has_callback(&self) -> bool {
        self.callback_url.is_some()
    }
}

#[async_trait]
impl ChannelAdapter for WebhookAdapter {
    fn name(&self) -> &str {
        "webhook"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("webhook".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let tx = Arc::new(tx);
        let secret = Arc::new(self.secret.clone());
        let account_id = Arc::new(self.account_id.clone());
        let deliver_only = self.deliver_only;
        let deliver_target = Arc::new(self.deliver_target.clone());

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let tx = Arc::clone(&tx);
                let secret = Arc::clone(&secret);
                let account_id = Arc::clone(&account_id);
                let deliver_target = Arc::clone(&deliver_target);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = Arc::clone(&tx);
                    let secret = Arc::clone(&secret);
                    let account_id = Arc::clone(&account_id);
                    let deliver_target = Arc::clone(&deliver_target);
                    async move {
                        let signature = headers
                            .get("X-Webhook-Signature")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");

                        if !WebhookAdapter::verify_signature(&secret, &body, signature) {
                            warn!("Webhook: invalid signature");
                            return (
                                axum::http::StatusCode::FORBIDDEN,
                                "Forbidden: invalid signature",
                            );
                        }

                        let json_body: serde_json::Value = match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(_) => {
                                return (axum::http::StatusCode::BAD_REQUEST, "Invalid JSON");
                            }
                        };

                        if let Some((
                            message,
                            sender_id,
                            sender_name,
                            thread_id,
                            is_group,
                            metadata,
                        )) = WebhookAdapter::parse_webhook_body(&json_body)
                        {
                            let content = if message.starts_with('/') {
                                let parts: Vec<&str> = message.splitn(2, ' ').collect();
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
                                ChannelContent::Text(message)
                            };

                            let mut msg = ChannelMessage {
                                channel: ChannelType::Custom("webhook".to_string()),
                                platform_message_id: format!(
                                    "wh-{}",
                                    Utc::now().timestamp_millis()
                                ),
                                sender: ChannelUser {
                                    platform_id: sender_id,
                                    display_name: sender_name,
                                    librefang_user: None,
                                },
                                content,
                                target_agent: None,
                                timestamp: Utc::now(),
                                is_group,
                                thread_id,
                                metadata,
                            };

                            if let Some(ref aid) = *account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            if deliver_only {
                                msg.metadata.insert(
                                    "__deliver_only__".to_string(),
                                    serde_json::json!(true),
                                );
                                if let Some(ref target) = *deliver_target {
                                    msg.metadata.insert(
                                        "__deliver_target__".to_string(),
                                        serde_json::json!(target),
                                    );
                                }
                            }
                            if tx.send(msg).await.is_err() {
                                // Bridge receiver closed — the message is lost
                                // even though we're about to return 200 OK to
                                // the upstream. Log so operators can notice.
                                warn!("Webhook: bridge channel closed, incoming message dropped");
                            }
                        }

                        (axum::http::StatusCode::OK, "ok")
                    }
                }
            }),
        );

        info!("Webhook adapter registered on shared server at /channels/webhook/incoming");

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
        // instead. This start() is only reached as a fallback (shouldn't happen
        // in normal operation since BridgeManager prefers create_webhook_routes).
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let callback_url = self
            .callback_url
            .as_ref()
            .ok_or("Webhook: no callback_url configured for outbound messages")?;

        let text = match content {
            ChannelContent::Text(t) => t,
            _ => "(Unsupported content type)".to_string(),
        };

        let chunks = split_message(&text, MAX_MESSAGE_LEN);
        let num_chunks = chunks.len();

        for chunk in chunks {
            let body = serde_json::json!({
                "sender_id": "librefang",
                "sender_name": "LibreFang",
                "recipient_id": user.platform_id,
                "recipient_name": user.display_name,
                "message": chunk,
                "timestamp": Utc::now().to_rfc3339(),
            });

            let body_bytes = serde_json::to_vec(&body)?;
            let signature = Self::compute_signature(&self.secret, &body_bytes);

            let resp = self
                .client
                .post(callback_url)
                .header("Content-Type", "application/json")
                .header("X-Webhook-Signature", &signature)
                .body(body_bytes)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(format!("Webhook callback error {status}: {err_body}").into());
            }

            // Small delay between chunks for large messages
            if num_chunks > 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Generic webhooks have no typing indicator concept.
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
    fn test_webhook_adapter_creation() {
        let adapter = WebhookAdapter::new(
            "my-secret".to_string(),
            9000,
            Some("https://example.com/callback".to_string()),
        );
        assert_eq!(adapter.name(), "webhook");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("webhook".to_string())
        );
        assert!(adapter.has_callback());
    }

    #[test]
    fn test_webhook_no_callback() {
        let adapter = WebhookAdapter::new("secret".to_string(), 9000, None);
        assert!(!adapter.has_callback());
    }

    #[test]
    fn test_webhook_signature_computation() {
        let sig = WebhookAdapter::compute_signature("secret", b"hello world");
        assert!(sig.starts_with("sha256="));
        // Verify deterministic
        let sig2 = WebhookAdapter::compute_signature("secret", b"hello world");
        assert_eq!(sig, sig2);
    }

    #[test]
    fn test_webhook_signature_verification() {
        let secret = "test-secret";
        let body = b"test body content";
        let sig = WebhookAdapter::compute_signature(secret, body);
        assert!(WebhookAdapter::verify_signature(secret, body, &sig));
        assert!(!WebhookAdapter::verify_signature(
            secret,
            body,
            "sha256=bad"
        ));
        assert!(!WebhookAdapter::verify_signature("wrong", body, &sig));
    }

    #[test]
    fn test_webhook_signature_different_data() {
        let secret = "same-secret";
        let sig1 = WebhookAdapter::compute_signature(secret, b"data1");
        let sig2 = WebhookAdapter::compute_signature(secret, b"data2");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_webhook_parse_body_full() {
        let body = serde_json::json!({
            "sender_id": "user-123",
            "sender_name": "Alice",
            "message": "Hello webhook!",
            "thread_id": "thread-1",
            "is_group": true,
            "metadata": {
                "custom": "value"
            }
        });
        let result = WebhookAdapter::parse_webhook_body(&body);
        assert!(result.is_some());
        let (message, sender_id, sender_name, thread_id, is_group, metadata) = result.unwrap();
        assert_eq!(message, "Hello webhook!");
        assert_eq!(sender_id, "user-123");
        assert_eq!(sender_name, "Alice");
        assert_eq!(thread_id, Some("thread-1".to_string()));
        assert!(is_group);
        assert_eq!(
            metadata.get("custom"),
            Some(&serde_json::Value::String("value".to_string()))
        );
    }

    #[test]
    fn test_webhook_parse_body_minimal() {
        let body = serde_json::json!({
            "message": "Just a message"
        });
        let result = WebhookAdapter::parse_webhook_body(&body);
        assert!(result.is_some());
        let (message, sender_id, sender_name, thread_id, is_group, _metadata) = result.unwrap();
        assert_eq!(message, "Just a message");
        assert_eq!(sender_id, "webhook-user");
        assert_eq!(sender_name, "Webhook User");
        assert!(thread_id.is_none());
        assert!(!is_group);
    }

    #[test]
    fn test_webhook_parse_body_empty_message() {
        let body = serde_json::json!({ "message": "" });
        assert!(WebhookAdapter::parse_webhook_body(&body).is_none());
    }

    #[test]
    fn test_webhook_parse_body_no_message() {
        let body = serde_json::json!({ "sender_id": "user" });
        assert!(WebhookAdapter::parse_webhook_body(&body).is_none());
    }

    // ── verify_request error-path coverage (closes #3851) ────────────────────

    const SECRET: &str = "test-secret-#3851";

    fn make_sig(body: &[u8]) -> String {
        WebhookAdapter::compute_signature(SECRET, body)
    }

    fn now() -> i64 {
        // Fixed epoch value used as the "current time" in all verify_request
        // tests so the suite is deterministic without real clock access.
        1_700_000_000_i64
    }

    /// 1. Valid signature and fresh timestamp → accepted.
    #[test]
    fn test_verify_request_valid() {
        let body = b"hello world";
        let sig = make_sig(body);
        assert!(
            WebhookAdapter::verify_request(SECRET, body, &sig, Some(now()), now()).is_ok(),
            "valid request must be accepted"
        );
    }

    /// 2. Tampered body (signature no longer matches) → rejected.
    #[test]
    fn test_verify_request_tampered_body() {
        let original = b"original body";
        let sig = make_sig(original);
        let tampered = b"tampered body";
        let result = WebhookAdapter::verify_request(SECRET, tampered, &sig, Some(now()), now());
        assert_eq!(result, Err("invalid signature"), "tampered body must be rejected");
    }

    /// 3. Missing signature header (empty string) → rejected.
    #[test]
    fn test_verify_request_missing_signature() {
        let body = b"some body";
        let result = WebhookAdapter::verify_request(SECRET, body, "", Some(now()), now());
        assert_eq!(result, Err("missing signature"), "absent signature header must be rejected");
    }

    /// 4. Stale timestamp (> 5 minutes in the past) → rejected.
    #[test]
    fn test_verify_request_stale_timestamp() {
        let body = b"some body";
        let sig = make_sig(body);
        let stale_ts = now() - (5 * 60 + 1); // 301 seconds ago
        let result = WebhookAdapter::verify_request(SECRET, body, &sig, Some(stale_ts), now());
        assert_eq!(result, Err("timestamp too old"), "stale timestamp must be rejected");
    }

    /// 5. Future timestamp (> 5 minutes ahead) → rejected.
    #[test]
    fn test_verify_request_future_timestamp() {
        let body = b"some body";
        let sig = make_sig(body);
        let future_ts = now() + (5 * 60 + 1); // 301 seconds ahead
        let result = WebhookAdapter::verify_request(SECRET, body, &sig, Some(future_ts), now());
        assert_eq!(result, Err("timestamp in the future"), "future timestamp must be rejected");
    }

    /// Boundary: timestamp exactly at the ±5 min edge is still accepted.
    #[test]
    fn test_verify_request_boundary_timestamps_accepted() {
        let body = b"boundary";
        let sig = make_sig(body);
        // Exactly 5 minutes old (skew == MAX_SKEW_SECS) is within the window.
        let old_edge = now() - 5 * 60;
        assert!(
            WebhookAdapter::verify_request(SECRET, body, &sig, Some(old_edge), now()).is_ok(),
            "timestamp exactly at -5 min edge must be accepted"
        );
        // Exactly 5 minutes in the future.
        let future_edge = now() + 5 * 60;
        assert!(
            WebhookAdapter::verify_request(SECRET, body, &sig, Some(future_edge), now()).is_ok(),
            "timestamp exactly at +5 min edge must be accepted"
        );
    }
}
