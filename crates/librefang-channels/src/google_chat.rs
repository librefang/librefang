//! Google Chat channel adapter.
//!
//! Uses Google Chat REST API with service account JWT authentication for sending
//! messages and a webhook listener for receiving inbound messages from Google Chat
//! spaces.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use futures::Stream;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, SignerMut};
use rsa::RsaPrivateKey;
use sha2::Sha256;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

const MAX_MESSAGE_LEN: usize = 4096;
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 3600;
const GOOGLE_CHAT_SCOPE: &str = "https://www.googleapis.com/auth/chat.bot";

/// Fields extracted from a Google service account JSON key file.
#[derive(Debug, Clone, serde::Deserialize)]
struct ServiceAccountKey {
    /// Service account email address (used as JWT `iss` and `sub`).
    client_email: String,
    /// PEM-encoded RSA private key.
    private_key: String,
    /// Token endpoint URL (typically `https://oauth2.googleapis.com/token`).
    #[serde(default = "default_token_uri")]
    token_uri: String,
    /// Optional pre-supplied access token (for testing/migration).
    #[serde(default)]
    access_token: Option<String>,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

/// Google Chat channel adapter using service account authentication and REST API.
///
/// Inbound messages arrive via a configurable webhook HTTP listener.
/// Outbound messages are sent via the Google Chat REST API using an OAuth2 access
/// token obtained from a service account JWT.
pub struct GoogleChatAdapter {
    /// SECURITY: Service account key JSON is zeroized on drop.
    service_account_key: Zeroizing<String>,
    /// Space IDs to listen to (e.g., "spaces/AAAA").
    space_ids: Vec<String>,
    /// Port for the inbound webhook HTTP listener.
    webhook_port: u16,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Cached OAuth2 access token with expiry instant.
    cached_token: Arc<RwLock<Option<(String, Instant)>>>,
}

impl GoogleChatAdapter {
    /// Create a new Google Chat adapter.
    ///
    /// # Arguments
    /// * `service_account_key` - JSON content of the Google service account key file.
    /// * `space_ids` - Google Chat space IDs to interact with.
    /// * `webhook_port` - Local port to bind the inbound webhook listener on.
    pub fn new(service_account_key: String, space_ids: Vec<String>, webhook_port: u16) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            service_account_key: Zeroizing::new(service_account_key),
            space_ids,
            webhook_port,
            client: reqwest::Client::new(),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Get a valid access token, refreshing if expired or missing.
    ///
    /// Authentication priority:
    /// 1. Cached token (if not expired)
    /// 2. JWT-based service account auth (if `private_key` + `client_email` present)
    /// 3. Direct `access_token` field in key JSON (legacy/testing fallback)
    async fn get_access_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Check cache first
        {
            let cache = self.cached_token.read().await;
            if let Some((ref token, expiry)) = *cache {
                if Instant::now() + Duration::from_secs(TOKEN_REFRESH_MARGIN_SECS) < expiry {
                    return Ok(token.clone());
                }
            }
        }

        let sa_key: ServiceAccountKey = serde_json::from_str(&self.service_account_key)
            .map_err(|e| format!("Invalid service account key JSON: {e}"))?;

        // Try JWT-based authentication if private_key is present
        if !sa_key.private_key.is_empty() && !sa_key.client_email.is_empty() {
            let token = self.exchange_jwt_for_token(&sa_key).await?;
            return Ok(token);
        }

        // Fallback: use a direct access_token field (for testing or pre-authorized tokens)
        let token = sa_key.access_token.filter(|t| !t.is_empty()).ok_or(
            "Service account key has no private_key for JWT auth and no access_token fallback",
        )?;

        let expiry = Instant::now() + Duration::from_secs(DEFAULT_TOKEN_LIFETIME_SECS);
        *self.cached_token.write().await = Some((token.clone(), expiry));

        Ok(token)
    }

    /// Build a signed JWT assertion and exchange it for an OAuth2 access token.
    async fn exchange_jwt_for_token(
        &self,
        sa_key: &ServiceAccountKey,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let now = Utc::now().timestamp();

        // Build JWT header
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT"
        });

        // Build JWT claims
        let claims = serde_json::json!({
            "iss": sa_key.client_email,
            "sub": sa_key.client_email,
            "scope": GOOGLE_CHAT_SCOPE,
            "aud": sa_key.token_uri,
            "iat": now,
            "exp": now + DEFAULT_TOKEN_LIFETIME_SECS as i64,
        });

        let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let signing_input = format!("{header_b64}.{claims_b64}");

        // Parse PEM private key and sign with RS256
        let private_key = RsaPrivateKey::from_pkcs8_pem(&sa_key.private_key)
            .map_err(|e| format!("Failed to parse RSA private key: {e}"))?;

        let mut signing_key = SigningKey::<Sha256>::new(private_key);
        let signature = signing_key.sign(signing_input.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        let jwt = format!("{signing_input}.{signature_b64}");

        // Exchange JWT for access token at the token endpoint
        let resp = self
            .client
            .post(&sa_key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| format!("Token exchange request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Token exchange failed ({status}): {body}").into());
        }

        let token_resp: serde_json::Value = resp.json().await?;
        let access_token = token_resp["access_token"]
            .as_str()
            .ok_or("Token response missing access_token field")?
            .to_string();

        let expires_in = token_resp["expires_in"]
            .as_u64()
            .unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);

        let expiry = Instant::now() + Duration::from_secs(expires_in);
        *self.cached_token.write().await = Some((access_token.clone(), expiry));

        debug!(
            "Google Chat: obtained access token via JWT, expires in {}s",
            expires_in
        );

        Ok(access_token)
    }

    /// Send a text message to a Google Chat space.
    async fn api_send_message(
        &self,
        space_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let token = self.get_access_token().await?;
        let url = format!("https://chat.googleapis.com/v1/{}/messages", space_id);

        let chunks = split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Google Chat API error {status}: {body}").into());
            }
        }

        Ok(())
    }

    /// Check if a space ID is in the allowed list.
    #[allow(dead_code)]
    fn is_allowed_space(&self, space_id: &str) -> bool {
        self.space_ids.is_empty() || self.space_ids.iter().any(|s| s == space_id)
    }
}

#[async_trait]
impl ChannelAdapter for GoogleChatAdapter {
    fn name(&self) -> &str {
        "google_chat"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("google_chat".to_string())
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        // Validate we can parse the service account key
        let _key: serde_json::Value = serde_json::from_str(&self.service_account_key)
            .map_err(|e| format!("Invalid service account key: {e}"))?;

        info!(
            "Google Chat adapter starting webhook listener on port {}",
            self.webhook_port
        );

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let port = self.webhook_port;
        let space_ids = self.space_ids.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = Arc::new(self.account_id.clone());

        tokio::spawn(async move {
            // Bind a minimal HTTP listener for inbound webhooks
            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    warn!("Google Chat: failed to bind webhook on port {port}: {e}");
                    return;
                }
            };

            info!("Google Chat webhook listener bound on {addr}");

            loop {
                let (stream, _peer) = tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("Google Chat adapter shutting down");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok(conn) => conn,
                            Err(e) => {
                                warn!("Google Chat: accept error: {e}");
                                continue;
                            }
                        }
                    }
                };

                let tx = tx.clone();
                let space_ids = space_ids.clone();
                let account_id = Arc::clone(&account_id);

                tokio::spawn(async move {
                    // Read HTTP request from the TCP stream
                    let mut reader = tokio::io::BufReader::new(stream);
                    let mut request_line = String::new();
                    if tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut request_line)
                        .await
                        .is_err()
                    {
                        return;
                    }

                    // Read headers to find Content-Length
                    let mut content_length: usize = 0;
                    loop {
                        let mut header_line = String::new();
                        if tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut header_line)
                            .await
                            .is_err()
                        {
                            return;
                        }
                        let trimmed = header_line.trim();
                        if trimmed.is_empty() {
                            break;
                        }
                        if let Some(val) = trimmed.strip_prefix("Content-Length:") {
                            if let Ok(len) = val.trim().parse::<usize>() {
                                content_length = len;
                            }
                        }
                        if let Some(val) = trimmed.strip_prefix("content-length:") {
                            if let Ok(len) = val.trim().parse::<usize>() {
                                content_length = len;
                            }
                        }
                    }

                    // Read body
                    let mut body_buf = vec![0u8; content_length.min(65536)];
                    use tokio::io::AsyncReadExt;
                    if content_length > 0
                        && reader
                            .read_exact(&mut body_buf[..content_length.min(65536)])
                            .await
                            .is_err()
                    {
                        return;
                    }

                    // Send 200 OK response
                    use tokio::io::AsyncWriteExt;
                    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                    let _ = reader.get_mut().write_all(resp).await;

                    // Parse the Google Chat event payload
                    let payload: serde_json::Value =
                        match serde_json::from_slice(&body_buf[..content_length.min(65536)]) {
                            Ok(v) => v,
                            Err(_) => return,
                        };

                    let event_type = payload["type"].as_str().unwrap_or("");
                    if event_type != "MESSAGE" {
                        return;
                    }

                    let message = &payload["message"];
                    let text = message["text"].as_str().unwrap_or("");
                    if text.is_empty() {
                        return;
                    }

                    let space_name = payload["space"]["name"].as_str().unwrap_or("");
                    if !space_ids.is_empty() && !space_ids.iter().any(|s| s == space_name) {
                        return;
                    }

                    let sender_name = message["sender"]["displayName"]
                        .as_str()
                        .unwrap_or("unknown");
                    let sender_id = message["sender"]["name"].as_str().unwrap_or("unknown");
                    let message_name = message["name"].as_str().unwrap_or("").to_string();
                    let thread_name = message["thread"]["name"].as_str().map(String::from);
                    let space_type = payload["space"]["type"].as_str().unwrap_or("ROOM");
                    let is_group = space_type != "DM";

                    let msg_content = if text.starts_with('/') {
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

                    let mut channel_msg = ChannelMessage {
                        channel: ChannelType::Custom("google_chat".to_string()),
                        platform_message_id: message_name,
                        sender: ChannelUser {
                            platform_id: space_name.to_string(),
                            display_name: sender_name.to_string(),
                            librefang_user: None,
                        },
                        content: msg_content,
                        target_agent: None,
                        timestamp: Utc::now(),
                        is_group,
                        thread_id: thread_name,
                        metadata: {
                            let mut m = HashMap::new();
                            m.insert(
                                "sender_id".to_string(),
                                serde_json::Value::String(sender_id.to_string()),
                            );
                            m
                        },
                    };

                    // Inject account_id for multi-bot routing
                    if let Some(ref aid) = *account_id {
                        channel_msg
                            .metadata
                            .insert("account_id".to_string(), serde_json::json!(aid));
                    }
                    let _ = tx.send(channel_msg).await;
                });
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
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

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_chat_adapter_creation() {
        let adapter = GoogleChatAdapter::new(
            r#"{"access_token":"test-token","project_id":"test"}"#.to_string(),
            vec!["spaces/AAAA".to_string()],
            8090,
        );
        assert_eq!(adapter.name(), "google_chat");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("google_chat".to_string())
        );
    }

    #[test]
    fn test_google_chat_allowed_spaces() {
        let adapter = GoogleChatAdapter::new(
            r#"{"access_token":"tok"}"#.to_string(),
            vec!["spaces/AAAA".to_string()],
            8090,
        );
        assert!(adapter.is_allowed_space("spaces/AAAA"));
        assert!(!adapter.is_allowed_space("spaces/BBBB"));

        let open = GoogleChatAdapter::new(r#"{"access_token":"tok"}"#.to_string(), vec![], 8090);
        assert!(open.is_allowed_space("spaces/anything"));
    }

    #[tokio::test]
    async fn test_google_chat_token_caching_fallback() {
        let adapter = GoogleChatAdapter::new(
            r#"{"access_token":"cached-tok","project_id":"p"}"#.to_string(),
            vec![],
            8091,
        );

        // First call should parse and cache (uses access_token fallback)
        let token = adapter.get_access_token().await.unwrap();
        assert_eq!(token, "cached-tok");

        // Second call should return from cache
        let token2 = adapter.get_access_token().await.unwrap();
        assert_eq!(token2, "cached-tok");
    }

    #[tokio::test]
    async fn test_google_chat_no_credentials_error() {
        let adapter = GoogleChatAdapter::new(
            r#"{"client_email":"test@test.iam.gserviceaccount.com"}"#.to_string(),
            vec![],
            8093,
        );

        // No private_key and no access_token should error
        let result = adapter.get_access_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no private_key") || err.contains("no access_token"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_google_chat_invalid_key() {
        let adapter = GoogleChatAdapter::new("not-json".to_string(), vec![], 8092);
        // Can't call async get_access_token in sync test, but verify construction works
        assert_eq!(adapter.webhook_port, 8092);
    }

    #[test]
    fn test_service_account_key_parsing() {
        let json = r#"{
            "client_email": "bot@project.iam.gserviceaccount.com",
            "private_key": "-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----\n",
            "token_uri": "https://oauth2.googleapis.com/token"
        }"#;
        let key: ServiceAccountKey = serde_json::from_str(json).unwrap();
        assert_eq!(key.client_email, "bot@project.iam.gserviceaccount.com");
        assert!(key.private_key.contains("BEGIN PRIVATE KEY"));
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
        assert!(key.access_token.is_none());
    }

    #[test]
    fn test_service_account_key_default_token_uri() {
        let json = r#"{
            "client_email": "bot@project.iam.gserviceaccount.com",
            "private_key": "key"
        }"#;
        let key: ServiceAccountKey = serde_json::from_str(json).unwrap();
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[tokio::test]
    async fn test_jwt_construction_with_invalid_key() {
        // Verify that an invalid PEM key produces a clear error
        let adapter = GoogleChatAdapter::new(
            r#"{
                "client_email": "bot@test.iam.gserviceaccount.com",
                "private_key": "not-a-valid-pem-key",
                "token_uri": "https://oauth2.googleapis.com/token"
            }"#
            .to_string(),
            vec![],
            8094,
        );

        let result = adapter.get_access_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to parse RSA private key"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_google_chat_with_account_id() {
        let adapter = GoogleChatAdapter::new(r#"{"access_token":"tok"}"#.to_string(), vec![], 8095)
            .with_account_id(Some("bot-1".to_string()));
        assert_eq!(adapter.account_id, Some("bot-1".to_string()));
    }
}
