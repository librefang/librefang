//! Gitter channel adapter.
//!
//! Connects to the Gitter Streaming API for real-time messages and posts
//! replies via the REST API. Uses Bearer token authentication and
//! newline-delimited JSON streaming.

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
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use zeroize::Zeroizing;

const MAX_MESSAGE_LEN: usize = 4096;
const GITTER_STREAM_URL: &str = "https://stream.gitter.im/v1/rooms";
const GITTER_API_URL: &str = "https://api.gitter.im/v1/rooms";

/// Returns the default Gitter REST API base URL. Used to initialise `GitterAdapter::api_base`.
#[inline]
fn default_gitter_api_base() -> String {
    GITTER_API_URL.to_string()
}

/// Gitter streaming channel adapter.
///
/// Receives messages via the Gitter Streaming API (newline-delimited JSON)
/// and sends replies via the REST API.
pub struct GitterAdapter {
    /// SECURITY: Bearer token is zeroized on drop.
    token: Zeroizing<String>,
    /// Gitter room ID to listen on.
    room_id: String,
    /// Base URL for the Gitter REST API. Defaults to `https://api.gitter.im/v1/rooms`.
    /// Overridable in tests via `with_api_base()` to point at a wiremock server.
    api_base: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GitterAdapter {
    /// Create a new Gitter adapter.
    ///
    /// # Arguments
    /// * `token` - Gitter personal access token.
    /// * `room_id` - Gitter room ID to listen on and send to.
    pub fn new(token: String, room_id: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            token: Zeroizing::new(token),
            room_id,
            api_base: default_gitter_api_base(),
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

    /// Override the Gitter REST API base URL. Intended for tests that point the adapter at
    /// a wiremock server instead of `https://api.gitter.im/v1/rooms`.
    #[cfg(test)]
    pub fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    /// Validate token by fetching the authenticated user.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = "https://api.gitter.im/v1/user";
        let resp = self
            .client
            .get(url)
            .bearer_auth(self.token.as_str())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(format!("Gitter auth failed (HTTP {})", resp.status()).into());
        }

        let body: serde_json::Value = resp.json().await?;
        // /v1/user returns an array with a single user object
        let username = body
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|u| u["username"].as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(username)
    }

    /// Fetch room info to resolve display name.
    async fn get_room_name(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/{}", self.api_base, self.room_id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(self.token.as_str())
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(format!("Gitter: failed to fetch room (HTTP {})", resp.status()).into());
        }

        let body: serde_json::Value = resp.json().await?;
        let name = body["name"].as_str().unwrap_or("unknown-room").to_string();
        Ok(name)
    }

    /// Send a text message to the room via REST API.
    async fn api_send_message(
        &self,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/{}/chatMessages", self.api_base, self.room_id);
        let chunks = split_message(text, MAX_MESSAGE_LEN);

        for chunk in chunks {
            let body = serde_json::json!({
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(self.token.as_str())
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(format!("Gitter API error {status}: {err_body}").into());
            }
        }

        Ok(())
    }

    /// Parse a newline-delimited JSON message from the streaming API.
    fn parse_stream_message(line: &str) -> Option<(String, String, String, String)> {
        let val: serde_json::Value = serde_json::from_str(line).ok()?;
        let id = val["id"].as_str()?.to_string();
        let text = val["text"].as_str()?.to_string();
        let username = val["fromUser"]["username"].as_str()?.to_string();
        let display_name = val["fromUser"]["displayName"]
            .as_str()
            .unwrap_or(&username)
            .to_string();

        if text.is_empty() {
            return None;
        }

        Some((id, text, username, display_name))
    }
}

#[async_trait]
impl ChannelAdapter for GitterAdapter {
    fn name(&self) -> &str {
        "gitter"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("gitter".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let own_username = self.validate().await?;
        let room_name = self.get_room_name().await.unwrap_or_default();
        info!("Gitter adapter authenticated as {own_username} in room {room_name}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let room_id = self.room_id.clone();
        let token = self.token.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();

        tokio::spawn(async move {
            let stream_client = crate::http_client::client_builder()
                .timeout(Duration::from_secs(0)) // No timeout for streaming
                .build()
                .expect("HTTP client build");

            let mut backoff = Duration::from_secs(1);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!("{}/{}/chatMessages", GITTER_STREAM_URL, room_id);

                let response = match stream_client
                    .get(&url)
                    .bearer_auth(token.as_str())
                    .header("Accept", "application/json")
                    .send()
                    .await
                {
                    Ok(r) => {
                        if !r.status().is_success() {
                            warn!("Gitter: stream returned HTTP {}", r.status());
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(Duration::from_secs(120));
                            continue;
                        }
                        backoff = Duration::from_secs(1);
                        r
                    }
                    Err(e) => {
                        warn!("Gitter: stream connection error: {e}, backing off {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(120));
                        continue;
                    }
                };

                info!("Gitter: streaming connection established for room {room_id}");

                // Read the streaming response as bytes, splitting on newlines
                let mut stream = response.bytes_stream();
                use futures::StreamExt;

                let mut line_buffer = String::new();

                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                info!("Gitter adapter shutting down");
                                return;
                            }
                        }
                        chunk = stream.next() => {
                            match chunk {
                                Some(Ok(bytes)) => {
                                    let text = String::from_utf8_lossy(&bytes);
                                    line_buffer.push_str(&text);

                                    // Process complete lines
                                    while let Some(newline_pos) = line_buffer.find('\n') {
                                        let line = line_buffer[..newline_pos].trim().to_string();
                                        line_buffer = line_buffer[newline_pos + 1..].to_string();

                                        // Skip heartbeat (empty lines / whitespace-only)
                                        if line.is_empty() || line.chars().all(|c| c.is_whitespace()) {
                                            continue;
                                        }

                                        if let Some((id, text, username, display_name)) =
                                            Self::parse_stream_message(&line)
                                        {
                                            // Skip own messages
                                            if username == own_username {
                                                continue;
                                            }

                                            let content = if text.starts_with('/') {
                                                let parts: Vec<&str> = text.splitn(2, ' ').collect();
                                                let cmd = parts[0].trim_start_matches('/');
                                                let args: Vec<String> = parts
                                                    .get(1)
                                                    .map(|a| {
                                                        a.split_whitespace()
                                                            .map(String::from)
                                                            .collect()
                                                    })
                                                    .unwrap_or_default();
                                                ChannelContent::Command {
                                                    name: cmd.to_string(),
                                                    args,
                                                }
                                            } else {
                                                ChannelContent::Text(text)
                                            };

                                            let mut msg = ChannelMessage {
                                                channel: ChannelType::Custom(
                                                    "gitter".to_string(),
                                                ),
                                                platform_message_id: id,
                                                sender: ChannelUser {
                                                    platform_id: username.clone(),
                                                    display_name,
                                                    librefang_user: None,
                                                },
                                                content,
                                                target_agent: None,
                                                timestamp: Utc::now(),
                                                is_group: true,
                                                thread_id: None,
                                                metadata: {
                                                    let mut m = HashMap::new();
                                                    m.insert(
                                                        "room_id".to_string(),
                                                        serde_json::Value::String(
                                                            room_id.clone(),
                                                        ),
                                                    );
                                                    m
                                                },
                                            };

                                            // Inject account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    msg.metadata.insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                if tx.send(msg).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                                Some(Err(e)) => {
                                    warn!("Gitter: stream read error: {e}");
                                    break; // Reconnect
                                }
                                None => {
                                    info!("Gitter: stream ended, reconnecting...");
                                    break;
                                }
                            }
                        }
                    }
                }

                // Exponential backoff before reconnect
                if !*shutdown_rx.borrow() {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }

            info!("Gitter streaming loop stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            _ => "(Unsupported content type)".to_string(),
        };
        self.api_send_message(&text).await
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Gitter does not have a typing indicator API.
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    /// Expose the configured multi-bot `account_id` (typically the Gitter
    /// room / org identifier) so the bridge approval listener builds the
    /// same `gitter:<account_id>` key the router stores in `channel_defaults`,
    /// scoping ApprovalRequested delivery to the room bound to the
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
    // Uses `wiremock` to stand up a local HTTP server and points `GitterAdapter`
    // at it via `with_api_base()`. Exercises the `POST /{room_id}/chatMessages`
    // call made by `ChannelAdapter::send`.

    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_adapter(api_base: String) -> GitterAdapter {
        GitterAdapter::new("test-gitter-token".to_string(), "abc123room".to_string())
            .with_api_base(api_base)
    }

    fn dummy_user() -> ChannelUser {
        ChannelUser {
            platform_id: "abc123room".to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn gitter_send_posts_chat_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/abc123room/chatMessages"))
            .and(header("Authorization", "Bearer test-gitter-token"))
            .and(body_json(serde_json::json!({
                "text": "hello from librefang",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg-001",
                "text": "hello from librefang",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user(),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn gitter_send_unsupported_content_uses_placeholder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/abc123room/chatMessages"))
            .and(body_json(serde_json::json!({
                "text": "(Unsupported content type)",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg-002",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user(),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send with unsupported content must succeed");
    }

    #[test]
    fn test_gitter_adapter_creation() {
        let adapter = GitterAdapter::new("test-token".to_string(), "abc123room".to_string());
        assert_eq!(adapter.name(), "gitter");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("gitter".to_string())
        );
    }

    #[test]
    fn test_gitter_room_id() {
        let adapter = GitterAdapter::new("tok".to_string(), "my-room-id".to_string());
        assert_eq!(adapter.room_id, "my-room-id");
    }

    #[test]
    fn test_gitter_parse_stream_message() {
        let json = r#"{"id":"msg1","text":"Hello world","fromUser":{"username":"alice","displayName":"Alice B"}}"#;
        let result = GitterAdapter::parse_stream_message(json);
        assert!(result.is_some());
        let (id, text, username, display_name) = result.unwrap();
        assert_eq!(id, "msg1");
        assert_eq!(text, "Hello world");
        assert_eq!(username, "alice");
        assert_eq!(display_name, "Alice B");
    }

    #[test]
    fn test_gitter_parse_stream_message_missing_fields() {
        let json = r#"{"id":"msg1"}"#;
        assert!(GitterAdapter::parse_stream_message(json).is_none());
    }

    #[test]
    fn test_gitter_parse_stream_message_empty_text() {
        let json =
            r#"{"id":"msg1","text":"","fromUser":{"username":"alice","displayName":"Alice"}}"#;
        assert!(GitterAdapter::parse_stream_message(json).is_none());
    }

    #[test]
    fn test_gitter_parse_stream_message_no_display_name() {
        let json = r#"{"id":"msg1","text":"hi","fromUser":{"username":"bob"}}"#;
        let result = GitterAdapter::parse_stream_message(json);
        assert!(result.is_some());
        let (_, _, username, display_name) = result.unwrap();
        assert_eq!(username, "bob");
        assert_eq!(display_name, "bob"); // Falls back to username
    }

    #[test]
    fn test_gitter_parse_invalid_json() {
        assert!(GitterAdapter::parse_stream_message("not json").is_none());
        assert!(GitterAdapter::parse_stream_message("").is_none());
    }

    #[test]
    fn test_gitter_account_id_default_none() {
        let adapter = GitterAdapter::new("token".to_string(), "room-1".to_string());
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_gitter_account_id_returns_configured_value() {
        // #5003: two Gitter rooms / orgs must resolve under distinct
        // `gitter:<account_id>` keys via the trait override.
        let adapter = GitterAdapter::new("token".to_string(), "room-1".to_string())
            .with_account_id(Some("org-42".to_string()));
        assert_eq!(adapter.account_id(), Some("org-42"));
    }
}
