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
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

// Poll interval is now configurable via SignalConfig.

// ---------------------------------------------------------------------------
// SSRF guard
// ---------------------------------------------------------------------------

/// Returns `true` when the address is in a range that should be blocked by
/// default (loopback, link-local, RFC-1918 private, and IPv6 ULA/link-local).
fn is_private_or_loopback(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_loopback()                                           // 127.0.0.0/8
                || v4.is_link_local()                                  // 169.254.0.0/16
                || v4.is_private()                                     // 10/8, 172.16/12, 192.168/16
                || v4.is_broadcast()                                   // 255.255.255.255
                || v4.is_unspecified()                                 // 0.0.0.0
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 CGNAT
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()        // ::1
                || v6.is_unspecified()   // ::
                // fc00::/7  (ULA — unique local)
                || (v6.octets()[0] & 0xFE) == 0xFC
                // fe80::/10 (link-local)
                || (v6.octets()[0] == 0xFE && (v6.octets()[1] & 0xC0) == 0x80)
        }
    }
}

/// Validate the `api_url` for SSRF safety.
///
/// Rules:
/// - Scheme must be `http` or `https`.
/// - Hostname must resolve to a non-private address unless `allow_local` is
///   set.  We perform a blocking DNS lookup here; for a config-time check the
///   latency is acceptable.
///
/// Returns `Ok(())` on success, `Err(message)` on violation.
fn validate_api_url(api_url: &str, allow_local: bool) -> Result<(), String> {
    let url =
        url::Url::parse(api_url).map_err(|e| format!("Signal api_url is not a valid URL: {e}"))?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "Signal api_url scheme '{other}' is not allowed; use http or https"
            ));
        }
    }

    if allow_local {
        // Operator has explicitly acknowledged local addresses.
        return Ok(());
    }

    // Resolve the hostname and reject private/loopback addresses.
    let host = url
        .host_str()
        .ok_or_else(|| "Signal api_url has no host".to_string())?;

    let port = url.port_or_known_default().unwrap_or(80);

    // std::net::ToSocketAddrs performs a synchronous DNS resolution.
    let addrs = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|e| format!("Signal api_url DNS resolution failed for '{host}': {e}"))?;

    for addr in addrs {
        if is_private_or_loopback(addr.ip()) {
            return Err(format!(
                "Signal api_url '{}' resolves to a private/loopback address ({}). \
                 Set `allow_local = true` in [channels.signal] if this is intentional.",
                api_url,
                addr.ip()
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Signal adapter via signal-cli REST API.
pub struct SignalAdapter {
    /// URL of signal-cli REST API (e.g., "http://localhost:8080").
    api_url: String,
    /// Registered phone number.
    phone_number: String,
    /// HTTP client (has connect + total timeouts pre-configured).
    client: reqwest::Client,
    /// Allowed phone numbers (empty = allow all).
    allowed_users: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Poll interval for checking new messages.
    poll_interval: Duration,
    /// Optional Bearer token for the signal-cli REST API.
    api_key: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl SignalAdapter {
    /// Create a new Signal adapter.
    ///
    /// # Errors
    /// Returns an error if `api_url` fails SSRF validation (scheme not http/https,
    /// or resolves to a private/loopback address without `allow_local = true`).
    pub fn new(
        api_url: String,
        phone_number: String,
        allowed_users: Vec<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::with_options(api_url, phone_number, allowed_users, None, false)
    }

    /// Create a new Signal adapter with security options.
    ///
    /// * `api_key`    – if `Some`, sent as `Authorization: Bearer <key>` on every request.
    /// * `allow_local` – when `true`, SSRF validation skips the private-IP check.
    ///
    /// # Errors
    /// Returns an error if `api_url` fails SSRF validation.
    pub fn with_options(
        api_url: String,
        phone_number: String,
        allowed_users: Vec<String>,
        api_key: Option<String>,
        allow_local: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        validate_api_url(&api_url, allow_local)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

        let client = crate::http_client::client_builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("Failed to build HTTP client for Signal adapter: {e}").into()
            })?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Ok(Self {
            api_url,
            phone_number,
            client,
            allowed_users,
            account_id: None,
            poll_interval: Duration::from_secs(2),
            api_key,
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

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Attach the `Authorization: Bearer` header when an API key is configured.
    fn auth_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            builder.bearer_auth(key)
        } else {
            builder
        }
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

        let req = self.auth_request(self.client.post(&url)).json(&body);
        let resp = req.send().await?;

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
        let req = self.auth_request(self.client.get(url));
        let resp = req.send().await?;
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

        let req = self.auth_request(self.client.get(&url));
        let resp = req.send().await?;

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
        let api_key = self.api_key.clone();

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
                let mut req = client.get(&url);
                if let Some(ref key) = api_key {
                    req = req.bearer_auth(key);
                }
                let resp = match req.send().await {
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

    /// Expose the configured multi-bot `account_id` (typically the Signal
    /// account phone number or device hash) so the bridge approval listener
    /// builds the same `signal:<account_id>` key the router stores in
    /// `channel_defaults`, scoping ApprovalRequested delivery to the
    /// account bound to the requesting agent (#5003, follow-up to
    /// #4985 / #4994).
    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_adapter_creation() {
        // localhost is allowed when allow_local = true
        let adapter = SignalAdapter::with_options(
            "http://localhost:8080".to_string(),
            "+1234567890".to_string(),
            vec![],
            None,
            true,
        )
        .expect("should build with allow_local=true");
        assert_eq!(adapter.name(), "signal");
        assert_eq!(adapter.channel_type(), ChannelType::Signal);
    }

    #[test]
    fn test_signal_account_id_default_none() {
        let adapter = SignalAdapter::with_options(
            "http://localhost:8080".to_string(),
            "+1234567890".to_string(),
            vec![],
            None,
            true,
        )
        .expect("should build");
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_signal_account_id_returns_configured_value() {
        // #5003: two Signal accounts (each with its own phone number /
        // device hash) must resolve under distinct `signal:<account_id>`
        // keys via the trait override.
        let adapter = SignalAdapter::with_options(
            "http://localhost:8080".to_string(),
            "+1234567890".to_string(),
            vec![],
            None,
            true,
        )
        .expect("should build")
        .with_account_id(Some("+1234567890".to_string()));
        assert_eq!(adapter.account_id(), Some("+1234567890"));
    }

    #[test]
    fn test_signal_allowed_check() {
        let adapter = SignalAdapter::with_options(
            "http://localhost:8080".to_string(),
            "+1234567890".to_string(),
            vec!["+9876543210".to_string()],
            None,
            true,
        )
        .expect("should build");
        assert!(adapter.is_allowed("+9876543210"));
        assert!(!adapter.is_allowed("+1111111111"));
    }

    #[test]
    fn test_ssrf_rejects_loopback_without_allow_local() {
        // 127.0.0.1 is loopback — must be rejected unless allow_local is set.
        let err = validate_api_url("http://127.0.0.1:8080", false);
        assert!(err.is_err(), "loopback must be rejected");
    }

    #[test]
    fn test_ssrf_allows_loopback_with_allow_local() {
        let ok = validate_api_url("http://127.0.0.1:8080", true);
        assert!(ok.is_ok(), "loopback must be allowed when allow_local=true");
    }

    #[test]
    fn test_ssrf_rejects_bad_scheme() {
        let err = validate_api_url("ftp://example.com/api", false);
        assert!(err.is_err(), "ftp scheme must be rejected");
    }

    #[test]
    fn test_ssrf_rejects_file_scheme() {
        let err = validate_api_url("file:///etc/passwd", false);
        assert!(err.is_err(), "file scheme must be rejected");
    }

    #[test]
    fn test_ssrf_rejects_private_ipv4() {
        // RFC-1918 addresses must be blocked.
        assert!(validate_api_url("http://192.168.1.1/api", false).is_err());
        assert!(validate_api_url("http://10.0.0.1/api", false).is_err());
        assert!(validate_api_url("http://172.16.0.1/api", false).is_err());
    }

    #[test]
    fn test_is_private_or_loopback() {
        use std::net::Ipv4Addr;
        assert!(is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            127, 0, 0, 1
        ))));
        assert!(is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 1
        ))));
        assert!(is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            10, 0, 0, 1
        ))));
        assert!(is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 1
        ))));
        assert!(is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 1
        ))));
        // Public addresses must NOT be blocked.
        assert!(!is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            1, 1, 1, 1
        ))));
        assert!(!is_private_or_loopback(IpAddr::V4(Ipv4Addr::new(
            8, 8, 8, 8
        ))));
    }

    // ----- send() path tests (issue #3820) -----
    //
    // Signal's adapter is a thin client over the signal-cli REST daemon, so
    // a `wiremock::MockServer` stands in for that daemon. The mock listens
    // on 127.0.0.1, which the SSRF guard rejects by default — so all of
    // these tests use `with_options(..., allow_local = true)`. Asserts the
    // `POST {api_url}/v2/send` call shape made by `ChannelAdapter::send`,
    // including the JSON body schema and (when configured) the bearer
    // header.

    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn signal_user(recipient: &str) -> ChannelUser {
        ChannelUser {
            platform_id: recipient.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn signal_send_posts_v2_send_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/send"))
            .and(body_json(serde_json::json!({
                "message": "hello signal",
                "number": "+15555550100",
                "recipients": ["+15555550199"],
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "timestamp": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = SignalAdapter::with_options(
            server.uri(),
            "+15555550100".to_string(),
            vec![],
            None,
            true, // allow_local — wiremock binds to 127.0.0.1
        )
        .expect("signal adapter ctor with allow_local must succeed");
        adapter
            .send(
                &signal_user("+15555550199"),
                ChannelContent::Text("hello signal".into()),
            )
            .await
            .expect("signal send must succeed against mock");
    }

    #[tokio::test]
    async fn signal_send_attaches_bearer_when_api_key_set() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/send"))
            .and(header("authorization", "Bearer api-key-xyz"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "timestamp": 0,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = SignalAdapter::with_options(
            server.uri(),
            "+15555550100".to_string(),
            vec![],
            Some("api-key-xyz".to_string()),
            true,
        )
        .expect("signal adapter ctor must succeed");
        adapter
            .send(
                &signal_user("+15555550199"),
                ChannelContent::Text("ping".into()),
            )
            .await
            .expect("signal send must succeed against mock");
    }

    #[tokio::test]
    async fn signal_send_returns_err_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/send"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = SignalAdapter::with_options(
            server.uri(),
            "+15555550100".to_string(),
            vec![],
            None,
            true,
        )
        .expect("signal adapter ctor must succeed");
        let err = adapter
            .send(
                &signal_user("+15555550199"),
                ChannelContent::Text("x".into()),
            )
            .await
            .expect_err("signal send must propagate non-2xx as Err");
        assert!(
            err.to_string().contains("500"),
            "error should mention status code, got: {err}"
        );
    }
}
