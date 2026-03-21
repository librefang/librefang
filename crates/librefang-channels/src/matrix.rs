//! Matrix channel adapter.
//!
//! Uses the Matrix Client-Server API (via reqwest) for sending and receiving messages.
//! Implements /sync long-polling with exponential backoff on failures for automatic
//! reconnection after connection drops.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Maximum backoff duration on sync failures.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Initial backoff duration on sync failures.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Matrix /sync long-polling timeout in milliseconds.
const SYNC_TIMEOUT_MS: u64 = 30000;
const MAX_MESSAGE_LEN: usize = 4096;

/// Matrix channel adapter using the Client-Server API.
pub struct MatrixAdapter {
    /// Matrix homeserver URL (e.g., `"https://matrix.org"`).
    homeserver_url: String,
    /// Bot's user ID (e.g., "@librefang:matrix.org").
    user_id: String,
    /// SECURITY: Access token is zeroized on drop.
    access_token: Zeroizing<String>,
    /// HTTP client.
    client: reqwest::Client,
    /// Allowed room IDs (empty = all joined rooms).
    allowed_rooms: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Whether to automatically accept room invites.
    /// Used when processing `/sync` invite events (not yet wired).
    #[allow(dead_code)]
    auto_accept_invites: bool,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Sync token for resuming /sync.
    since_token: Arc<RwLock<Option<String>>>,
}

impl MatrixAdapter {
    /// Create a new Matrix adapter.
    pub fn new(
        homeserver_url: String,
        user_id: String,
        access_token: String,
        allowed_rooms: Vec<String>,
        auto_accept_invites: bool,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            homeserver_url,
            user_id,
            access_token: Zeroizing::new(access_token),
            client: crate::http_client::new_client(),
            allowed_rooms,
            account_id: None,
            auto_accept_invites,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            since_token: Arc::new(RwLock::new(None)),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Send a text message to a Matrix room.
    async fn api_send_message(
        &self,
        room_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let txn_id = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            self.homeserver_url, room_id, txn_id
        );

        let chunks = crate::types::split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "msgtype": "m.text",
                "body": chunk,
            });

            let resp = self
                .client
                .put(&url)
                .bearer_auth(&*self.access_token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Matrix API error {status}: {body}").into());
            }
        }

        Ok(())
    }

    /// Validate credentials by calling /whoami.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver_url);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&*self.access_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err("Matrix authentication failed".into());
        }

        let body: serde_json::Value = resp.json().await?;
        let user_id = body["user_id"].as_str().unwrap_or("unknown").to_string();

        Ok(user_id)
    }

    #[allow(dead_code)]
    fn is_allowed_room(&self, room_id: &str) -> bool {
        self.allowed_rooms.is_empty() || self.allowed_rooms.iter().any(|r| r == room_id)
    }
}

#[async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn name(&self) -> &str {
        "matrix"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Matrix
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Validate credentials
        let validated_user = self.validate().await?;
        info!("Matrix adapter authenticated as {validated_user}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let homeserver = self.homeserver_url.clone();
        let access_token = self.access_token.clone();
        let user_id = self.user_id.clone();
        let allowed_rooms = self.allowed_rooms.clone();
        let client = self.client.clone();
        let since_token = Arc::clone(&self.since_token);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;

            loop {
                // Build /sync URL
                let since = since_token.read().await.clone();
                let mut url = format!(
                    "{}/_matrix/client/v3/sync?timeout={}&filter={{\"room\":{{\"timeline\":{{\"limit\":10}}}}}}",
                    homeserver, SYNC_TIMEOUT_MS
                );
                if let Some(ref token) = since {
                    url.push_str(&format!("&since={token}"));
                }

                let resp = tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("Matrix adapter shutting down");
                        break;
                    }
                    result = client.get(&url).bearer_auth(&*access_token).send() => {
                        match result {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("Matrix /sync network error: {e}, retrying in {backoff:?}");
                                tokio::time::sleep(backoff).await;
                                backoff = calculate_backoff(backoff);
                                continue;
                            }
                        }
                    }
                };

                if !resp.status().is_success() {
                    let status = resp.status();
                    warn!("Matrix /sync failed ({status}), retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = calculate_backoff(backoff);
                    continue;
                }

                // Reset backoff on success
                if backoff > INITIAL_BACKOFF {
                    debug!("Matrix /sync recovered, resetting backoff");
                }
                backoff = INITIAL_BACKOFF;

                let body: serde_json::Value = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Matrix sync parse error: {e}");
                        continue;
                    }
                };

                // Update since token
                if let Some(next) = body["next_batch"].as_str() {
                    *since_token.write().await = Some(next.to_string());
                }

                // Process room events
                if let Some(rooms) = body["rooms"]["join"].as_object() {
                    for (room_id, room_data) in rooms {
                        if !allowed_rooms.is_empty() && !allowed_rooms.iter().any(|r| r == room_id)
                        {
                            continue;
                        }

                        if let Some(events) = room_data["timeline"]["events"].as_array() {
                            for event in events {
                                let event_type = event["type"].as_str().unwrap_or("");
                                if event_type != "m.room.message" {
                                    continue;
                                }

                                let sender = event["sender"].as_str().unwrap_or("");
                                if sender == user_id {
                                    continue; // Skip own messages
                                }

                                let content = event["content"]["body"].as_str().unwrap_or("");
                                if content.is_empty() {
                                    continue;
                                }

                                let msg_content = if content.starts_with('/') {
                                    let parts: Vec<&str> = content.splitn(2, ' ').collect();
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
                                    ChannelContent::Text(content.to_string())
                                };

                                let event_id = event["event_id"].as_str().unwrap_or("").to_string();

                                let mut channel_msg = ChannelMessage {
                                    channel: ChannelType::Matrix,
                                    platform_message_id: event_id,
                                    sender: ChannelUser {
                                        platform_id: room_id.clone(),
                                        display_name: sender.to_string(),
                                        librefang_user: None,
                                    },
                                    content: msg_content,
                                    target_agent: None,
                                    timestamp: Utc::now(),
                                    is_group: true,
                                    thread_id: None,
                                    metadata: HashMap::new(),
                                };

                                // Inject account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    channel_msg
                                        .metadata
                                        .insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                if tx.send(channel_msg).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });

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
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/typing/{}",
            self.homeserver_url, user.platform_id, self.user_id
        );

        let body = serde_json::json!({
            "typing": true,
            "timeout": 5000,
        });

        let _ = self
            .client
            .put(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

/// Calculate exponential backoff capped at MAX_BACKOFF.
pub fn calculate_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_adapter_creation() {
        let adapter = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "access_token".to_string(),
            vec![],
            false,
        );
        assert_eq!(adapter.name(), "matrix");
    }

    #[test]
    fn test_matrix_allowed_rooms() {
        let adapter = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "token".to_string(),
            vec!["!room1:matrix.org".to_string()],
            false,
        );
        assert!(adapter.is_allowed_room("!room1:matrix.org"));
        assert!(!adapter.is_allowed_room("!room2:matrix.org"));

        let open = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "token".to_string(),
            vec![],
            false,
        );
        assert!(open.is_allowed_room("!any:matrix.org"));
    }

    #[test]
    fn test_backoff_calculation() {
        let b1 = calculate_backoff(Duration::from_secs(1));
        assert_eq!(b1, Duration::from_secs(2));

        let b2 = calculate_backoff(Duration::from_secs(2));
        assert_eq!(b2, Duration::from_secs(4));

        let b3 = calculate_backoff(Duration::from_secs(32));
        assert_eq!(b3, Duration::from_secs(60)); // capped at MAX_BACKOFF

        let b4 = calculate_backoff(Duration::from_secs(60));
        assert_eq!(b4, Duration::from_secs(60)); // stays at MAX_BACKOFF
    }

    #[test]
    fn test_backoff_constants() {
        assert_eq!(INITIAL_BACKOFF, Duration::from_secs(1));
        assert_eq!(MAX_BACKOFF, Duration::from_secs(60));
        assert!(INITIAL_BACKOFF < MAX_BACKOFF);
    }

    #[test]
    fn test_backoff_progression() {
        // Verify the full backoff sequence from INITIAL to MAX
        let mut current = INITIAL_BACKOFF;
        let expected = [1, 2, 4, 8, 16, 32, 60, 60];
        for &exp_secs in &expected {
            assert_eq!(
                current.as_secs(),
                if current == INITIAL_BACKOFF && exp_secs == 1 {
                    1
                } else {
                    current.as_secs()
                }
            );
            current = calculate_backoff(current);
            // After calculate_backoff, verify next value
        }
        // Simpler: just walk the sequence
        let mut b = INITIAL_BACKOFF;
        assert_eq!(b, Duration::from_secs(1));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(2));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(4));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(8));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(16));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(32));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(60));
        b = calculate_backoff(b);
        assert_eq!(b, Duration::from_secs(60)); // stays capped
    }
}
