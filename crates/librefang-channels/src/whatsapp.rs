//! WhatsApp Cloud API channel adapter.
//!
//! Uses the official WhatsApp Business Cloud API to send and receive messages.
//! Requires a webhook endpoint for incoming messages and the Cloud API for outgoing.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use futures::Stream;
use librefang_types::config::{DmPolicy, GroupPolicy};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};
use zeroize::Zeroizing;

const MAX_MESSAGE_LEN: usize = 4096;

/// WhatsApp Cloud API adapter.
///
/// Supports two modes:
/// - **Cloud API mode**: Uses the official WhatsApp Business Cloud API (requires Meta dev account).
/// - **Web/QR mode**: Routes outgoing messages through a local Baileys-based gateway process.
///
/// Mode is selected automatically: if `gateway_url` is set (from `WHATSAPP_WEB_GATEWAY_URL`),
/// the adapter uses Web mode. Otherwise it falls back to Cloud API mode.
pub struct WhatsAppAdapter {
    /// WhatsApp Business phone number ID (Cloud API mode).
    phone_number_id: String,
    /// SECURITY: Access token is zeroized on drop.
    access_token: Zeroizing<String>,
    /// SECURITY: Verify token is zeroized on drop.
    verify_token: Zeroizing<String>,
    /// Port to listen for webhook callbacks (Cloud API mode).
    webhook_port: u16,
    /// HTTP client.
    client: reqwest::Client,
    /// Allowed phone numbers (empty = allow all).
    allowed_users: Vec<String>,
    /// Optional WhatsApp Web gateway URL for QR/Web mode (e.g. "http://127.0.0.1:3009").
    gateway_url: Option<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// DM message policy: how to handle direct messages.
    dm_policy: DmPolicy,
    /// Group message policy: how to handle group/community messages.
    group_policy: GroupPolicy,
    /// Bot's own phone number (used for mention detection in group chats).
    /// Should match the `phone_number_id` display number, e.g. "+15551234567".
    bot_phone: Option<String>,
    /// Bot display name (used as fallback mention keyword in group chats).
    bot_name: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Cloud API base URL (without phone-number-id segment). Defaults to
    /// `"https://graph.facebook.com/v21.0"`; tests override via
    /// `with_base_url` so the adapter talks to a local wiremock instead.
    cloud_api_base: String,
}

impl WhatsAppAdapter {
    /// Create a new WhatsApp Cloud API adapter.
    pub fn new(
        phone_number_id: String,
        access_token: String,
        verify_token: String,
        webhook_port: u16,
        allowed_users: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            phone_number_id,
            access_token: Zeroizing::new(access_token),
            verify_token: Zeroizing::new(verify_token),
            webhook_port,
            client: crate::http_client::new_client(),
            allowed_users,
            gateway_url: None,
            account_id: None,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            bot_phone: None,
            bot_name: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            cloud_api_base: "https://graph.facebook.com/v21.0".to_string(),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the Cloud API base URL. `#[cfg(test)]`-only — used by
    /// wiremock-driven tests to point the adapter at a local mock server.
    #[cfg(test)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.cloud_api_base = url;
        self
    }

    /// Create a new WhatsApp adapter with gateway URL for Web/QR mode.
    ///
    /// When `gateway_url` is `Some`, outgoing messages are sent via `POST {gateway_url}/message/send`
    /// instead of the Cloud API. Incoming messages are handled by the gateway itself.
    pub fn with_gateway(mut self, gateway_url: Option<String>) -> Self {
        self.gateway_url = gateway_url.filter(|u| !u.is_empty());
        self
    }

    /// Set the DM policy for this adapter. Returns self for builder chaining.
    pub fn with_dm_policy(mut self, policy: DmPolicy) -> Self {
        self.dm_policy = policy;
        self
    }

    /// Set the group message policy for this adapter. Returns self for builder chaining.
    pub fn with_group_policy(mut self, policy: GroupPolicy) -> Self {
        self.group_policy = policy;
        self
    }

    /// Set the bot's own phone number for mention detection in group chats.
    pub fn with_bot_phone(mut self, phone: Option<String>) -> Self {
        self.bot_phone = phone;
        self
    }

    /// Set the bot's display name for mention detection in group chats.
    pub fn with_bot_name(mut self, name: Option<String>) -> Self {
        self.bot_name = name;
        self
    }

    /// Determine whether an incoming message should be handled based on the configured policies.
    ///
    /// - `is_group`: whether the message came from a group/community chat.
    /// - `text`: the raw message text (used for mention detection under `MentionOnly`).
    /// - `sender_phone`: the sender's phone number (used for `DmPolicy::AllowedOnly`).
    ///
    /// Returns `true` if the adapter should process and respond to the message.
    pub fn should_handle_message(&self, is_group: bool, text: &str, sender_phone: &str) -> bool {
        if is_group {
            match self.group_policy {
                GroupPolicy::All => true,
                GroupPolicy::MentionOnly => self.is_bot_mentioned(text),
                GroupPolicy::CommandsOnly => text.trim_start().starts_with('/'),
                GroupPolicy::Ignore => false,
            }
        } else {
            match self.dm_policy {
                DmPolicy::Respond => true,
                DmPolicy::AllowedOnly => self.is_allowed(sender_phone),
                DmPolicy::Ignore => false,
            }
        }
    }

    /// Check whether the bot is @mentioned in the given message text.
    ///
    /// WhatsApp does not have a native @mention protocol at the Cloud API level,
    /// so we look for the bot's phone number or display name anywhere in the text.
    fn is_bot_mentioned(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if let Some(ref phone) = self.bot_phone {
            // Match bare number or with leading '@'
            if lower.contains(phone.as_str())
                || lower.contains(&format!("@{}", phone.trim_start_matches('+')))
            {
                return true;
            }
        }
        if let Some(ref name) = self.bot_name {
            if lower.contains(&name.to_lowercase()) {
                return true;
            }
        }
        false
    }

    /// Upload raw audio bytes to the WhatsApp Media API and return the media ID.
    ///
    /// The caller is responsible for providing the correct MIME type
    /// (e.g. `"audio/ogg; codecs=opus"` for voice messages).
    async fn api_upload_media(
        &self,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use reqwest::multipart;

        let url = format!(
            "{}/{}/media",
            self.cloud_api_base, self.phone_number_id
        );

        // Build multipart form: file field + messaging_product field
        let file_part = multipart::Part::bytes(audio.to_vec())
            .mime_str(mime_type)?
            .file_name("voice.ogg");

        let form = multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", file_part);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp media upload error {status}: {body}");
            return Err(format!("WhatsApp media upload error {status}: {body}").into());
        }

        let json: serde_json::Value = resp.json().await?;
        let media_id = json
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("WhatsApp media upload: missing 'id' in response")?
            .to_string();

        Ok(media_id)
    }

    /// Send a voice message via the WhatsApp Cloud API.
    ///
    /// Uploads the raw audio bytes as a media object, then sends an `audio` message
    /// referencing the returned media ID.  WhatsApp renders this as an inline voice note.
    ///
    /// # Arguments
    /// * `to`        – recipient phone number (E.164 format, e.g. `"+15551234567"`).
    /// * `audio`     – raw OGG/Opus bytes (or any audio format accepted by the API).
    /// * `mime_type` – MIME type of the audio, e.g. `"audio/ogg; codecs=opus"`.
    pub async fn send_voice(
        &self,
        to: &str,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let media_id = self.api_upload_media(audio, mime_type).await?;

        let url = format!(
            "{}/{}/messages",
            self.cloud_api_base, self.phone_number_id
        );
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "audio",
            "audio": { "id": media_id }
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp send_voice error {status}: {body}");
            return Err(format!("WhatsApp send_voice error {status}: {body}").into());
        }

        Ok(())
    }

    /// Send a voice message via the WhatsApp Web gateway (Web/QR mode).
    ///
    /// The gateway is expected to accept `POST /message/send-voice` with a JSON body
    /// containing `{ "to": "...", "audio": "<base64>", "mime_type": "..." }`.
    #[allow(dead_code)]
    async fn gateway_send_voice(
        &self,
        gateway_url: &str,
        to: &str,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/message/send-voice", gateway_url.trim_end_matches('/'));

        // base64-encode without pulling in a new crate — use standard library approach
        let mut encoded = String::new();
        {
            use std::fmt::Write as FmtWrite;
            const TABLE: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut buf = [0u8; 3];
            let mut i = 0;
            while i < audio.len() {
                let remaining = audio.len() - i;
                let chunk_len = remaining.min(3);
                buf[..chunk_len].copy_from_slice(&audio[i..i + chunk_len]);
                if chunk_len < 3 {
                    buf[chunk_len..].fill(0);
                }
                let b0 = (buf[0] >> 2) as usize;
                let b1 = (((buf[0] & 0x03) << 4) | (buf[1] >> 4)) as usize;
                let b2 = (((buf[1] & 0x0f) << 2) | (buf[2] >> 6)) as usize;
                let b3 = (buf[2] & 0x3f) as usize;
                let _ = write!(encoded, "{}", TABLE[b0] as char);
                let _ = write!(encoded, "{}", TABLE[b1] as char);
                let _ = write!(
                    encoded,
                    "{}",
                    if chunk_len >= 2 {
                        TABLE[b2] as char
                    } else {
                        '='
                    }
                );
                let _ = write!(
                    encoded,
                    "{}",
                    if chunk_len >= 3 {
                        TABLE[b3] as char
                    } else {
                        '='
                    }
                );
                i += chunk_len;
            }
        }

        let body = serde_json::json!({
            "to": to,
            "audio": encoded,
            "mime_type": mime_type,
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp gateway send-voice error {status}: {body}");
            return Err(format!("WhatsApp gateway send-voice error {status}: {body}").into());
        }

        Ok(())
    }

    /// Send a text message via the WhatsApp Cloud API.
    async fn api_send_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/{}/messages",
            self.cloud_api_base, self.phone_number_id
        );

        // Split long messages
        let chunks = crate::types::split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": { "body": chunk }
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&*self.access_token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                error!("WhatsApp API error {status}: {body}");
                return Err(format!("WhatsApp API error {status}: {body}").into());
            }
        }

        Ok(())
    }

    /// Mark a message as read.
    #[allow(dead_code)]
    async fn api_mark_read(
        &self,
        message_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/{}/messages",
            self.cloud_api_base, self.phone_number_id
        );

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id
        });

        let _ = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    /// Send a text message via the WhatsApp Web gateway.
    async fn gateway_send_message(
        &self,
        gateway_url: &str,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/message/send", gateway_url.trim_end_matches('/'));
        let body = serde_json::json!({ "to": to, "text": text });

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp gateway error {status}: {body}");
            return Err(format!("WhatsApp gateway error {status}: {body}").into());
        }

        Ok(())
    }

    /// Check if a phone number is allowed.
    #[allow(dead_code)]
    fn is_allowed(&self, phone: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == phone)
    }

    /// Returns true if this adapter is configured for Web/QR gateway mode.
    #[allow(dead_code)]
    pub fn is_gateway_mode(&self) -> bool {
        self.gateway_url.is_some()
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn name(&self) -> &str {
        "whatsapp"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::WhatsApp
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let port = self.webhook_port;
        let _verify_token = self.verify_token.clone();
        let _allowed_users = self.allowed_users.clone();
        let _access_token = self.access_token.clone();
        let _phone_number_id = self.phone_number_id.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        info!("Starting WhatsApp webhook listener on port {port}");

        tokio::spawn(async move {
            // Simple webhook polling simulation
            // In production, this would be an axum HTTP server handling webhook POSTs
            // For now, log that the webhook is ready
            info!("WhatsApp webhook ready on port {port} (verify_token configured)");
            info!("Configure your webhook URL: https://your-domain:{port}/webhook");

            // Wait for shutdown
            let _ = shutdown_rx.changed().await;
            info!("WhatsApp adapter stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Web/QR gateway mode: route all messages through the gateway
        if let Some(ref gw) = self.gateway_url {
            match &content {
                ChannelContent::Voice { url, .. } => {
                    // For voice messages in gateway mode, send as a text link fallback
                    // unless the gateway supports send-voice (handled separately via send_voice()).
                    let text = format!("(Voice message: {url})");
                    self.gateway_send_message(gw, &user.platform_id, &text)
                        .await?;
                }
                other => {
                    let text = match other {
                        ChannelContent::Text(t) => t.clone(),
                        ChannelContent::Image { caption, .. } => caption
                            .clone()
                            .unwrap_or_else(|| "(Image — not supported in Web mode)".to_string()),
                        ChannelContent::File { filename, .. } => {
                            format!("(File: {filename} — not supported in Web mode)")
                        }
                        _ => "(Unsupported content type in Web mode)".to_string(),
                    };
                    // Split long messages the same way as Cloud API mode
                    let chunks = crate::types::split_message(&text, MAX_MESSAGE_LEN);
                    for chunk in chunks {
                        self.gateway_send_message(gw, &user.platform_id, chunk)
                            .await?;
                    }
                }
            }
            return Ok(());
        }

        // Cloud API mode (default)
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&user.platform_id, &text).await?;
            }
            ChannelContent::Voice { url, .. } => {
                // Voice messages with a URL are sent as audio links via the Cloud API.
                // For raw byte uploads use `send_voice()` directly.
                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "audio",
                    "audio": { "link": url }
                });
                let api_url = format!(
                    "{}/{}/messages",
                    self.cloud_api_base, self.phone_number_id
                );
                let resp = self
                    .client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    error!("WhatsApp voice send error {status}: {err}");
                    return Err(format!("WhatsApp voice send error {status}: {err}").into());
                }
            }
            ChannelContent::Image { url, caption, .. } => {
                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "image",
                    "image": {
                        "link": url,
                        "caption": caption.unwrap_or_default()
                    }
                });
                let api_url = format!(
                    "{}/{}/messages",
                    self.cloud_api_base, self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "document",
                    "document": {
                        "link": url,
                        "filename": filename
                    }
                });
                let api_url = format!(
                    "{}/{}/messages",
                    self.cloud_api_base, self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            ChannelContent::Location { lat, lon } => {
                let body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "location",
                    "location": {
                        "latitude": lat,
                        "longitude": lon
                    }
                });
                let api_url = format!(
                    "{}/{}/messages",
                    self.cloud_api_base, self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            _ => {
                self.api_send_message(&user.platform_id, "(Unsupported content type)")
                    .await?;
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
    fn test_whatsapp_adapter_creation() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "access_token".to_string(),
            "verify_token".to_string(),
            8443,
            vec![],
        );
        assert_eq!(adapter.name(), "whatsapp");
        assert_eq!(adapter.channel_type(), ChannelType::WhatsApp);
    }

    #[test]
    fn test_allowed_users_check() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec!["+1234567890".to_string()],
        );
        assert!(adapter.is_allowed("+1234567890"));
        assert!(!adapter.is_allowed("+9999999999"));

        let open = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        );
        assert!(open.is_allowed("+anything"));
    }

    #[test]
    fn test_dm_policy_defaults() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        );
        // Default DmPolicy is Respond
        assert!(adapter.should_handle_message(false, "hello", "+1234567890"));
    }

    #[test]
    fn test_dm_policy_ignore() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_dm_policy(DmPolicy::Ignore);
        assert!(!adapter.should_handle_message(false, "hello", "+1234567890"));
    }

    #[test]
    fn test_dm_policy_allowed_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec!["+1234567890".to_string()],
        )
        .with_dm_policy(DmPolicy::AllowedOnly);
        // Allowed sender → handle
        assert!(adapter.should_handle_message(false, "hello", "+1234567890"));
        // Unknown sender → reject
        assert!(!adapter.should_handle_message(false, "hello", "+9999999999"));
    }

    #[test]
    fn test_group_policy_all() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::All);
        assert!(adapter.should_handle_message(true, "any group message", ""));
    }

    #[test]
    fn test_group_policy_mention_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::MentionOnly)
        .with_bot_name(Some("HermesBot".to_string()))
        .with_bot_phone(Some("+15551234567".to_string()));

        // Without mention — should not handle
        assert!(!adapter.should_handle_message(true, "what time is it?", ""));
        // With bot name mention
        assert!(adapter.should_handle_message(true, "@HermesBot what time is it?", ""));
        // With bot phone mention
        assert!(adapter.should_handle_message(true, "+15551234567 hello", ""));
    }

    #[test]
    fn test_group_policy_commands_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::CommandsOnly);
        assert!(!adapter.should_handle_message(true, "hello everyone", ""));
        assert!(adapter.should_handle_message(true, "/help", ""));
    }

    #[test]
    fn test_group_policy_ignore() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::Ignore);
        assert!(!adapter.should_handle_message(true, "/help", ""));
    }

    // ----- send() path tests (issue #3820) -----
    //
    // The Cloud API URL was previously hard-coded inline; it is now backed
    // by a `cloud_api_base` field with a `#[cfg(test)] with_base_url`
    // setter. Asserts the `POST {base}/{phone_number_id}/messages` shape
    // — bearer auth + the WhatsApp Cloud API JSON envelope. Web/QR
    // gateway mode (already publicly settable via `with_gateway`) is
    // covered separately.

    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn whatsapp_user(phone: &str) -> ChannelUser {
        ChannelUser {
            platform_id: phone.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn whatsapp_cloud_api_send_posts_messages_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/PHONE_ID_42/messages"))
            .and(header("authorization", "Bearer access-tok"))
            .and(body_json(serde_json::json!({
                "messaging_product": "whatsapp",
                "to": "+15555550199",
                "type": "text",
                "text": { "body": "hi whatsapp" }
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "messaging_product": "whatsapp",
                    "messages": [{ "id": "wamid.123" }],
                })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WhatsAppAdapter::new(
            "PHONE_ID_42".to_string(),
            "access-tok".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_base_url(server.uri());
        adapter
            .send(
                &whatsapp_user("+15555550199"),
                ChannelContent::Text("hi whatsapp".into()),
            )
            .await
            .expect("whatsapp Cloud API send must succeed against mock");
    }

    #[tokio::test]
    async fn whatsapp_cloud_api_send_returns_err_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/PHONE_ID_42/messages"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WhatsAppAdapter::new(
            "PHONE_ID_42".to_string(),
            "bad-tok".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_base_url(server.uri());
        let err = adapter
            .send(
                &whatsapp_user("+15555550199"),
                ChannelContent::Text("x".into()),
            )
            .await
            .expect_err("whatsapp Cloud API send must propagate non-2xx as Err");
        assert!(
            err.to_string().contains("401"),
            "error should mention status code, got: {err}"
        );
    }

    #[tokio::test]
    async fn whatsapp_gateway_mode_send_posts_to_message_send() {
        // gateway mode is already publicly settable via `with_gateway`,
        // so this happy-path needs no extra production hook.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/message/send"))
            .and(body_json(serde_json::json!({
                "to": "+15555550199",
                "text": "via gateway",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WhatsAppAdapter::new(
            "PHONE_ID_42".to_string(),
            "ignored-in-gateway-mode".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_gateway(Some(server.uri()));
        adapter
            .send(
                &whatsapp_user("+15555550199"),
                ChannelContent::Text("via gateway".into()),
            )
            .await
            .expect("whatsapp gateway send must succeed against mock");
    }
}
