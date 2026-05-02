//! Gotify channel adapter.
//!
//! Connects to a Gotify server via WebSocket for receiving push notifications
//! and sends messages via the REST API. Uses separate app and client tokens
//! for publishing and subscribing respectively.

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

const MAX_MESSAGE_LEN: usize = 65535;

/// Gotify push notification channel adapter.
///
/// Receives messages via the Gotify WebSocket stream (`/stream`) using a
/// client token and sends messages via the REST API (`/message`) using an
/// app token.
pub struct GotifyAdapter {
    /// Gotify server URL (e.g., `"https://gotify.example.com"`).
    server_url: String,
    /// SECURITY: App token for sending messages (zeroized on drop).
    app_token: Zeroizing<String>,
    /// SECURITY: Client token for receiving messages (zeroized on drop).
    client_token: Zeroizing<String>,
    /// HTTP client for REST API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GotifyAdapter {
    /// Create a new Gotify adapter.
    ///
    /// # Arguments
    /// * `server_url` - Base URL of the Gotify server.
    /// * `app_token` - Token for an application (used to send messages).
    /// * `client_token` - Token for a client (used to receive messages via WebSocket).
    pub fn new(server_url: String, app_token: String, client_token: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_url = server_url.trim_end_matches('/').to_string();
        Self {
            server_url,
            app_token: Zeroizing::new(app_token),
            client_token: Zeroizing::new(client_token),
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

    /// Validate the app token by checking the application info.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/current/user?token={}",
            self.server_url,
            self.client_token.as_str()
        );
        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Err(format!("Gotify auth failed (HTTP {})", resp.status()).into());
        }

        let body: serde_json::Value = resp.json().await?;
        let name = body["name"].as_str().unwrap_or("gotify-user").to_string();
        Ok(name)
    }

    /// Build the WebSocket URL for the stream endpoint.
    fn build_ws_url(&self) -> String {
        let base = self
            .server_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        format!("{}/stream?token={}", base, self.client_token.as_str())
    }

    /// Send a message via the Gotify REST API.
    async fn api_send_message(
        &self,
        title: &str,
        message: &str,
        priority: u8,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/message?token={}",
            self.server_url,
            self.app_token.as_str()
        );
        let chunks = split_message(message, MAX_MESSAGE_LEN);

        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_title = if chunks.len() > 1 {
                format!("{} ({}/{})", title, i + 1, chunks.len())
            } else {
                title.to_string()
            };

            let body = serde_json::json!({
                "title": chunk_title,
                "message": chunk,
                "priority": priority,
            });

            let resp = self.client.post(&url).json(&body).send().await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(format!("Gotify API error {status}: {err_body}").into());
            }
        }

        Ok(())
    }

    /// Parse a Gotify WebSocket message (JSON).
    fn parse_ws_message(text: &str) -> Option<(u64, String, String, u8, u64)> {
        let val: serde_json::Value = serde_json::from_str(text).ok()?;
        let id = val["id"].as_u64()?;
        let message = val["message"].as_str()?.to_string();
        let title = val["title"].as_str().unwrap_or("").to_string();
        let priority = val["priority"].as_u64().unwrap_or(0) as u8;
        let app_id = val["appid"].as_u64().unwrap_or(0);

        if message.is_empty() {
            return None;
        }

        Some((id, message, title, priority, app_id))
    }
}

#[async_trait]
impl ChannelAdapter for GotifyAdapter {
    fn name(&self) -> &str {
        "gotify"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("gotify".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let user_name = self.validate().await?;
        info!("Gotify adapter authenticated as {user_name}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let ws_url = self.build_ws_url();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                info!("Gotify: connecting WebSocket...");

                let ws_connect = match tokio_tungstenite::connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        backoff = Duration::from_secs(1);
                        ws_stream
                    }
                    Err(e) => {
                        warn!("Gotify: WebSocket connection failed: {e}, backing off {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(120));
                        continue;
                    }
                };

                info!("Gotify: WebSocket connected");

                use futures::StreamExt;
                let (mut _ws_write, mut ws_read) = ws_connect.split();

                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                info!("Gotify adapter shutting down");
                                return;
                            }
                        }
                        msg = ws_read.next() => {
                            match msg {
                                Some(Ok(ws_msg)) => {
                                    let text = match ws_msg {
                                        tokio_tungstenite::tungstenite::Message::Text(t) => t,
                                        tokio_tungstenite::tungstenite::Message::Ping(_) => continue,
                                        tokio_tungstenite::tungstenite::Message::Pong(_) => continue,
                                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                                            info!("Gotify: WebSocket closed by server");
                                            break;
                                        }
                                        _ => continue,
                                    };

                                    if let Some((id, message, title, priority, app_id)) =
                                        Self::parse_ws_message(&text)
                                    {
                                        let content = if message.starts_with('/') {
                                            let parts: Vec<&str> =
                                                message.splitn(2, ' ').collect();
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
                                            ChannelContent::Text(message)
                                        };

                                        let mut msg = ChannelMessage {
                                            channel: ChannelType::Custom(
                                                "gotify".to_string(),
                                            ),
                                            platform_message_id: format!("gotify-{id}"),
                                            sender: ChannelUser {
                                                platform_id: format!("app-{app_id}"),
                                                display_name: if title.is_empty() {
                                                    format!("app-{app_id}")
                                                } else {
                                                    title.clone()
                                                },
                                                librefang_user: None,
                                            },
                                            content,
                                            target_agent: None,
                                            timestamp: Utc::now(),
                                            is_group: false,
                                            thread_id: None,
                                            metadata: {
                                                let mut m = HashMap::new();
                                                m.insert(
                                                    "title".to_string(),
                                                    serde_json::Value::String(title),
                                                );
                                                m.insert(
                                                    "priority".to_string(),
                                                    serde_json::Value::Number(priority.into()),
                                                );
                                                m.insert(
                                                    "app_id".to_string(),
                                                    serde_json::Value::Number(app_id.into()),
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
                                Some(Err(e)) => {
                                    warn!("Gotify: WebSocket read error: {e}");
                                    break;
                                }
                                None => {
                                    info!("Gotify: WebSocket stream ended");
                                    break;
                                }
                            }
                        }
                    }
                }

                // Exponential backoff before reconnect
                if !*shutdown_rx.borrow() {
                    warn!("Gotify: reconnecting in {backoff:?}...");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }

            info!("Gotify WebSocket loop stopped");
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
        self.api_send_message("LibreFang", &text, 5).await
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Gotify has no typing indicator.
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
    fn test_gotify_adapter_creation() {
        let adapter = GotifyAdapter::new(
            "https://gotify.example.com".to_string(),
            "app-token".to_string(),
            "client-token".to_string(),
        );
        assert_eq!(adapter.name(), "gotify");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("gotify".to_string())
        );
    }

    #[test]
    fn test_gotify_url_normalization() {
        let adapter = GotifyAdapter::new(
            "https://gotify.example.com/".to_string(),
            "app".to_string(),
            "client".to_string(),
        );
        assert_eq!(adapter.server_url, "https://gotify.example.com");
    }

    #[test]
    fn test_gotify_ws_url_https() {
        let adapter = GotifyAdapter::new(
            "https://gotify.example.com".to_string(),
            "app".to_string(),
            "client-tok".to_string(),
        );
        let ws_url = adapter.build_ws_url();
        assert!(ws_url.starts_with("wss://"));
        assert!(ws_url.contains("/stream?token=client-tok"));
    }

    #[test]
    fn test_gotify_ws_url_http() {
        let adapter = GotifyAdapter::new(
            "http://localhost:8080".to_string(),
            "app".to_string(),
            "client-tok".to_string(),
        );
        let ws_url = adapter.build_ws_url();
        assert!(ws_url.starts_with("ws://"));
        assert!(ws_url.contains("/stream?token=client-tok"));
    }

    #[test]
    fn test_gotify_parse_ws_message() {
        let json = r#"{"id":42,"appid":7,"message":"Hello Gotify","title":"Test App","priority":5,"date":"2024-01-01T00:00:00Z"}"#;
        let result = GotifyAdapter::parse_ws_message(json);
        assert!(result.is_some());
        let (id, message, title, priority, app_id) = result.unwrap();
        assert_eq!(id, 42);
        assert_eq!(message, "Hello Gotify");
        assert_eq!(title, "Test App");
        assert_eq!(priority, 5);
        assert_eq!(app_id, 7);
    }

    #[test]
    fn test_gotify_parse_ws_message_empty() {
        let json = r#"{"id":1,"appid":1,"message":"","title":"","priority":0}"#;
        assert!(GotifyAdapter::parse_ws_message(json).is_none());
    }

    #[test]
    fn test_gotify_parse_ws_message_minimal() {
        let json = r#"{"id":1,"message":"hi"}"#;
        let result = GotifyAdapter::parse_ws_message(json);
        assert!(result.is_some());
        let (_, msg, title, priority, app_id) = result.unwrap();
        assert_eq!(msg, "hi");
        assert_eq!(title, "");
        assert_eq!(priority, 0);
        assert_eq!(app_id, 0);
    }

    #[test]
    fn test_gotify_parse_invalid_json() {
        assert!(GotifyAdapter::parse_ws_message("not json").is_none());
    }

    // ----- send() path tests (issue #3820) -----
    //
    // These tests use `wiremock` to stand up a local HTTP server in place of
    // a real Gotify instance, then assert the exact request shape produced by
    // `GotifyAdapter::send` / `api_send_message`. Because `server_url` is a
    // plain field (no hardcoded const), pointing the adapter at the mock is
    // a one-liner.
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn dummy_user() -> ChannelUser {
        ChannelUser {
            platform_id: "app-1".to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn send_posts_to_message_endpoint_with_app_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .and(query_param("token", "app-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 1})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = GotifyAdapter::new(server.uri(), "app-tok".into(), "client-tok".into());
        adapter
            .send(&dummy_user(), ChannelContent::Text("hello".into()))
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn send_body_has_title_message_priority() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .and(header("content-type", "application/json"))
            .and(body_json(serde_json::json!({
                "title": "LibreFang",
                "message": "ping",
                "priority": 5,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 2})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = GotifyAdapter::new(server.uri(), "tok".into(), "ctok".into());
        adapter
            .send(&dummy_user(), ChannelContent::Text("ping".into()))
            .await
            .expect("send must succeed");
    }

    #[tokio::test]
    async fn send_strips_trailing_slash_in_server_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 1})))
            .expect(1)
            .mount(&server)
            .await;

        // Append a slash to the mock URI; adapter should normalize it so we
        // do not POST to `//message`.
        let url_with_slash = format!("{}/", server.uri());
        let adapter = GotifyAdapter::new(url_with_slash, "tok".into(), "ctok".into());
        adapter
            .send(&dummy_user(), ChannelContent::Text("hi".into()))
            .await
            .expect("send must succeed");
    }

    #[tokio::test]
    async fn send_non_text_content_uses_placeholder_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .and(body_json(serde_json::json!({
                "title": "LibreFang",
                "message": "(Unsupported content type)",
                "priority": 5,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 3})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = GotifyAdapter::new(server.uri(), "tok".into(), "ctok".into());
        adapter
            .send(
                &dummy_user(),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send must succeed");
    }

    #[tokio::test]
    async fn send_returns_error_on_http_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = GotifyAdapter::new(server.uri(), "tok".into(), "ctok".into());
        let err = adapter
            .send(&dummy_user(), ChannelContent::Text("nope".into()))
            .await
            .expect_err("send must propagate server error");
        let msg = err.to_string();
        assert!(
            msg.contains("500"),
            "error should mention status, got: {msg}"
        );
    }

    #[tokio::test]
    async fn send_returns_error_on_http_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad token"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = GotifyAdapter::new(server.uri(), "wrong".into(), "ctok".into());
        let err = adapter
            .send(&dummy_user(), ChannelContent::Text("x".into()))
            .await
            .expect_err("401 must surface as Err");
        assert!(err.to_string().contains("401"));
    }

    #[tokio::test]
    async fn send_typing_is_noop_and_makes_no_http_call() {
        let server = MockServer::start().await;
        // No mocks mounted — any HTTP call would 404 the wiremock default and
        // we'd see it in `received_requests`.
        let adapter = GotifyAdapter::new(server.uri(), "tok".into(), "ctok".into());
        adapter
            .send_typing(&dummy_user())
            .await
            .expect("send_typing is infallible for gotify");
        let recv = server.received_requests().await.unwrap_or_default();
        assert!(recv.is_empty(), "send_typing must not hit the network");
    }

    #[tokio::test]
    async fn send_uses_app_token_not_client_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message"))
            .and(query_param("token", "APP"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 1})))
            .expect(1)
            .mount(&server)
            .await;

        // If the adapter accidentally swapped tokens we'd see token=CLIENT
        // and wiremock would reject the request.
        let adapter = GotifyAdapter::new(server.uri(), "APP".into(), "CLIENT".into());
        adapter
            .send(&dummy_user(), ChannelContent::Text("auth-check".into()))
            .await
            .expect("must use app token in send URL");
    }
}
