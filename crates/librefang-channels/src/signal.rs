//! Signal channel adapter.
//!
//! Uses signal-cli's JSON-RPC daemon mode for sending/receiving messages.
//! Requires signal-cli to be installed and registered with a phone number.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// Validate the signal-cli REST API URL to prevent SSRF attacks.
///
/// Rejects:
/// - Non-HTTP/HTTPS schemes.
/// - Loopback hostnames (`localhost`, `127.*`).
/// - Link-local addresses (169.254.x.x).
/// - RFC-1918 private ranges (10.x, 172.16-31.x, 192.168.x).
/// - IPv6 loopback (`::1`) and unspecified (`::`) addresses.
///
/// Note: this is a best-effort blocklist. Agents should also be deployed
/// behind a network perimeter that restricts outbound connections. Always
/// configure signal-cli REST API with authentication (bearer token) and
/// TLS when exposed on a non-loopback interface.
fn validate_signal_api_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid signal api_url: {e}"))?;

    // Only allow http and https schemes.
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "signal api_url must use http or https scheme, got '{}'",
            parsed.scheme()
        ));
    }

    let host = parsed.host_str().unwrap_or("");

    // Block loopback and private-network hostnames/IPs.
    let blocked = host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("169.254.")
        || host.starts_with("10.")
        || host == "::1"
        || host == "::"
        || {
            // 172.16.0.0/12 → 172.16.x.x … 172.31.x.x
            if let Some(rest) = host.strip_prefix("172.") {
                rest.split('.')
                    .next()
                    .and_then(|octet| octet.parse::<u8>().ok())
                    .map(|o| (16..=31).contains(&o))
                    .unwrap_or(false)
            } else {
                false
            }
        }
        || host.starts_with("192.168.");

    if blocked {
        return Err(format!(
            "signal api_url must not point to a private or loopback address (got '{host}'). \
             signal-cli REST API should be deployed with bearer-token authentication and TLS \
             on a non-loopback interface, or accessed only through a secure local proxy."
        ));
    }

    Ok(())
}

// Poll interval is now configurable via SignalConfig.

/// Signal adapter via signal-cli REST API.
pub struct SignalAdapter {
    /// URL of signal-cli REST API (e.g., "http://localhost:8080").
    api_url: String,
    /// Registered phone number.
    phone_number: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Allowed phone numbers (empty = allow all).
    allowed_users: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Poll interval for checking new messages.
    poll_interval: Duration,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl SignalAdapter {
    /// Create a new Signal adapter.
    ///
    /// Returns an error if `api_url` is invalid or points to a private/loopback
    /// address that could be used for SSRF attacks.
    pub fn new(
        api_url: String,
        phone_number: String,
        allowed_users: Vec<String>,
    ) -> Result<Self, String> {
        validate_signal_api_url(&api_url)?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Ok(Self {
            api_url,
            phone_number,
            client: crate::http_client::new_client(),
            allowed_users,
            account_id: None,
            poll_interval: Duration::from_secs(2),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        })
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Set the poll interval. Returns self for builder chaining.
    pub fn with_poll_interval(mut self, poll_interval_secs: u64) -> Self {
        self.poll_interval = Duration::from_secs(poll_interval_secs);
        self
    }

    /// Send a message via signal-cli REST API.
    async fn api_send_message(
        &self,
        recipient: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_message_with_attachments(recipient, text, &[])
            .await
    }

    /// Send a message with optional base64-encoded attachments via signal-cli REST API.
    ///
    /// Each attachment entry is `{"data": "<base64>", "filename": "<name>"}`.
    /// When `attachments` is empty the request degrades to a plain text message.
    async fn api_send_message_with_attachments(
        &self,
        recipient: &str,
        text: &str,
        attachments: &[serde_json::Value],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v2/send", self.api_url);

        let mut body = serde_json::json!({
            "message": text,
            "number": self.phone_number,
            "recipients": [recipient],
        });

        if !attachments.is_empty() {
            body["base64_attachments"] = serde_json::Value::Array(attachments.to_vec());
        }

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Signal API error {status}: {body}").into());
        }

        Ok(())
    }

    /// Download a URL and return the raw bytes.
    async fn fetch_bytes(
        &self,
        url: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(format!("Failed to download attachment ({status}): {url}").into());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// Build a signal-cli base64_attachments entry from raw bytes and a filename.
    fn make_attachment(data: &[u8], filename: &str) -> serde_json::Value {
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        serde_json::json!({
            "data": encoded,
            "filename": filename,
        })
    }

    /// Receive messages from signal-cli REST API.
    #[allow(dead_code)]
    async fn receive_messages(
        &self,
    ) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v1/receive/{}", self.api_url, self.phone_number);

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let messages: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        Ok(messages)
    }

    #[allow(dead_code)]
    fn is_allowed(&self, phone: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == phone)
    }
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    fn name(&self) -> &str {
        "signal"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Signal
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let api_url = self.api_url.clone();
        let phone_number = self.phone_number.clone();
        let allowed_users = self.allowed_users.clone();
        let client = self.client.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();
        let poll_interval = self.poll_interval;

        info!(
            "Starting Signal adapter (polling {} every {:?})",
            api_url, poll_interval
        );

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("Signal adapter shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(poll_interval) => {}
                }

                // Poll for new messages
                let url = format!("{}/v1/receive/{}", api_url, phone_number);
                let resp = match client.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        debug!("Signal poll error: {e}");
                        continue;
                    }
                };

                if !resp.status().is_success() {
                    continue;
                }

                let messages: Vec<serde_json::Value> = match resp.json().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                for msg in messages {
                    let envelope = msg.get("envelope").unwrap_or(&msg);

                    let source = envelope["source"].as_str().unwrap_or("").to_string();

                    if source.is_empty() || source == phone_number {
                        continue;
                    }

                    if !allowed_users.is_empty() && !allowed_users.iter().any(|u| u == &source) {
                        continue;
                    }

                    // Extract text from dataMessage
                    let text = envelope["dataMessage"]["message"].as_str().unwrap_or("");

                    if text.is_empty() {
                        continue;
                    }

                    let source_name = envelope["sourceName"]
                        .as_str()
                        .unwrap_or(&source)
                        .to_string();

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

                    let mut channel_msg = ChannelMessage {
                        channel: ChannelType::Signal,
                        platform_message_id: envelope["timestamp"]
                            .as_u64()
                            .unwrap_or(0)
                            .to_string(),
                        sender: ChannelUser {
                            platform_id: source.clone(),
                            display_name: source_name,
                            librefang_user: None,
                        },
                        content,
                        target_agent: None,
                        timestamp: Utc::now(),
                        is_group: false,
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
                        break;
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
        let recipient = &user.platform_id;
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(recipient, &text).await?;
            }

            // --- Image ---
            ChannelContent::Image { url, caption, .. } => {
                let caption_text = caption.unwrap_or_default();
                match self.fetch_bytes(&url).await {
                    Ok(bytes) => {
                        // Derive filename from the URL path; fall back to "image.jpg".
                        let filename = url
                            .rsplit('/')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("image.jpg");
                        let attachment = Self::make_attachment(&bytes, filename);
                        self.api_send_message_with_attachments(
                            recipient,
                            &caption_text,
                            &[attachment],
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Signal: failed to download image attachment from {url}: {e}");
                        // Fall back to sending the URL as text so the user gets something.
                        let fallback = if caption_text.is_empty() {
                            url
                        } else {
                            format!("{caption_text}\n{url}")
                        };
                        self.api_send_message(recipient, &fallback).await?;
                    }
                }
            }

            // --- File (URL-based) ---
            ChannelContent::File { url, filename } => match self.fetch_bytes(&url).await {
                Ok(bytes) => {
                    let attachment = Self::make_attachment(&bytes, &filename);
                    self.api_send_message_with_attachments(recipient, "", &[attachment])
                        .await?;
                }
                Err(e) => {
                    warn!("Signal: failed to download file attachment from {url}: {e}");
                    self.api_send_message(recipient, &url).await?;
                }
            },

            // --- FileData (raw bytes already in memory) ---
            ChannelContent::FileData {
                data,
                filename,
                mime_type: _,
            } => {
                let attachment = Self::make_attachment(&data, &filename);
                self.api_send_message_with_attachments(recipient, "", &[attachment])
                    .await?;
            }

            // --- Voice memo ---
            ChannelContent::Voice { url, caption, .. } => {
                let caption_text = caption.unwrap_or_default();
                match self.fetch_bytes(&url).await {
                    Ok(bytes) => {
                        let filename = url
                            .rsplit('/')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("voice.ogg");
                        let attachment = Self::make_attachment(&bytes, filename);
                        self.api_send_message_with_attachments(
                            recipient,
                            &caption_text,
                            &[attachment],
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Signal: failed to download voice attachment from {url}: {e}");
                        self.api_send_message(recipient, &url).await?;
                    }
                }
            }

            // --- Video ---
            ChannelContent::Video {
                url,
                caption,
                filename,
                ..
            } => {
                let caption_text = caption.unwrap_or_default();
                let fname = filename.unwrap_or_else(|| {
                    url.rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("video.mp4")
                        .to_string()
                });
                match self.fetch_bytes(&url).await {
                    Ok(bytes) => {
                        let attachment = Self::make_attachment(&bytes, &fname);
                        self.api_send_message_with_attachments(
                            recipient,
                            &caption_text,
                            &[attachment],
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Signal: failed to download video attachment from {url}: {e}");
                        let fallback = if caption_text.is_empty() {
                            url
                        } else {
                            format!("{caption_text}\n{url}")
                        };
                        self.api_send_message(recipient, &fallback).await?;
                    }
                }
            }

            // --- Audio (music/podcast) ---
            ChannelContent::Audio { url, caption, .. } => {
                let caption_text = caption.unwrap_or_default();
                match self.fetch_bytes(&url).await {
                    Ok(bytes) => {
                        let filename = url
                            .rsplit('/')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("audio.mp3");
                        let attachment = Self::make_attachment(&bytes, filename);
                        self.api_send_message_with_attachments(
                            recipient,
                            &caption_text,
                            &[attachment],
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Signal: failed to download audio attachment from {url}: {e}");
                        self.api_send_message(recipient, &url).await?;
                    }
                }
            }

            // --- Animation / GIF ---
            ChannelContent::Animation { url, caption, .. } => {
                let caption_text = caption.unwrap_or_default();
                match self.fetch_bytes(&url).await {
                    Ok(bytes) => {
                        let filename = url
                            .rsplit('/')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("animation.gif");
                        let attachment = Self::make_attachment(&bytes, filename);
                        self.api_send_message_with_attachments(
                            recipient,
                            &caption_text,
                            &[attachment],
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Signal: failed to download animation from {url}: {e}");
                        self.api_send_message(recipient, &url).await?;
                    }
                }
            }

            // --- Unsupported variants: log and skip ---
            ChannelContent::Sticker { file_id } => {
                warn!(
                    "Signal: Sticker (file_id={file_id}) not supported by signal-cli REST API — skipping"
                );
            }
            ChannelContent::MediaGroup { items } => {
                warn!(
                    "Signal: MediaGroup ({} items) not natively supported — sending items individually",
                    items.len()
                );
                for item in items {
                    use crate::types::MediaGroupItem;
                    match item {
                        MediaGroupItem::Photo { url, caption } => {
                            self.send(
                                user,
                                ChannelContent::Image {
                                    url,
                                    caption,
                                    mime_type: None,
                                },
                            )
                            .await?;
                        }
                        MediaGroupItem::Video {
                            url,
                            caption,
                            duration_seconds,
                        } => {
                            self.send(
                                user,
                                ChannelContent::Video {
                                    url,
                                    caption,
                                    duration_seconds,
                                    filename: None,
                                },
                            )
                            .await?;
                        }
                    }
                }
            }
            ChannelContent::Poll {
                question, options, ..
            } => {
                warn!("Signal: Poll not supported by signal-cli REST API — skipping");
                // Send question + options as plain text so the user sees something.
                let text = format!(
                    "{question}\n{}",
                    options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| format!("{}. {o}", i + 1))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                self.api_send_message(recipient, &text).await?;
            }
            ChannelContent::PollAnswer { .. } => {
                warn!("Signal: PollAnswer is inbound-only — skipping outbound send");
            }
            ChannelContent::Location { lat, lon } => {
                // signal-cli REST API does not expose a location message type;
                // send coordinates as text.
                self.api_send_message(recipient, &format!("📍 {lat}, {lon}"))
                    .await?;
            }
            ChannelContent::Command { name, args } => {
                let text = if args.is_empty() {
                    format!("/{name}")
                } else {
                    format!("/{name} {}", args.join(" "))
                };
                self.api_send_message(recipient, &text).await?;
            }
            ChannelContent::Interactive { text, buttons } => {
                // Render as plain text with button labels listed as hints.
                let mut out = text;
                for row in &buttons {
                    out.push('\n');
                    for btn in row {
                        out.push_str(&format!("  [{}]", btn.label));
                    }
                }
                self.api_send_message(recipient, &out).await?;
            }
            ChannelContent::ButtonCallback { .. } => {
                warn!("Signal: ButtonCallback is inbound-only — skipping outbound send");
            }
            ChannelContent::DeleteMessage { .. } => {
                warn!("Signal: DeleteMessage not supported by signal-cli REST API — skipping");
            }
            ChannelContent::EditInteractive { text, buttons, .. } => {
                // No edit API in signal-cli REST; re-send as a new message.
                let mut out = text;
                for row in &buttons {
                    out.push('\n');
                    for btn in row {
                        out.push_str(&format!("  [{}]", btn.label));
                    }
                }
                self.api_send_message(recipient, &out).await?;
            }
        }
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
    fn test_signal_adapter_creation() {
        let adapter = SignalAdapter::new(
            "http://signal-cli.example.com:8080".to_string(),
            "+1234567890".to_string(),
            vec![],
        )
        .expect("valid external URL should be accepted");
        assert_eq!(adapter.name(), "signal");
        assert_eq!(adapter.channel_type(), ChannelType::Signal);
    }

    #[test]
    fn test_signal_allowed_check() {
        let adapter = SignalAdapter::new(
            "http://signal-cli.example.com:8080".to_string(),
            "+1234567890".to_string(),
            vec!["+9876543210".to_string()],
        )
        .expect("valid external URL should be accepted");
        assert!(adapter.is_allowed("+9876543210"));
        assert!(!adapter.is_allowed("+1111111111"));
    }

    // --- validate_signal_api_url tests ---

    #[test]
    fn valid_https_url_is_accepted() {
        assert!(validate_signal_api_url("https://signal-cli.example.com:8080").is_ok());
    }

    #[test]
    fn valid_http_url_is_accepted() {
        assert!(validate_signal_api_url("http://signal-cli.example.com:8080").is_ok());
    }

    #[test]
    fn rejects_localhost() {
        assert!(validate_signal_api_url("http://localhost:8080").is_err());
    }

    #[test]
    fn rejects_127_loopback() {
        assert!(validate_signal_api_url("http://127.0.0.1:8080").is_err());
    }

    #[test]
    fn rejects_link_local() {
        assert!(validate_signal_api_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn rejects_10_dot_private() {
        assert!(validate_signal_api_url("http://10.0.0.1:8080").is_err());
    }

    #[test]
    fn rejects_172_16_private() {
        assert!(validate_signal_api_url("http://172.16.0.1:8080").is_err());
    }

    #[test]
    fn rejects_172_31_private() {
        assert!(validate_signal_api_url("http://172.31.255.255:8080").is_err());
    }

    #[test]
    fn allows_172_32_as_not_private() {
        // 172.32.x.x is outside RFC-1918 range
        assert!(validate_signal_api_url("http://172.32.0.1:8080").is_ok());
    }

    #[test]
    fn rejects_192_168_private() {
        assert!(validate_signal_api_url("http://192.168.1.1:8080").is_err());
    }

    #[test]
    fn rejects_ipv6_loopback() {
        assert!(validate_signal_api_url("http://[::1]:8080").is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(validate_signal_api_url("ftp://signal-cli.example.com:8080").is_err());
        assert!(validate_signal_api_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_invalid_url() {
        assert!(validate_signal_api_url("not a url at all").is_err());
    }
}
