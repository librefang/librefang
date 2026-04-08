//! Telegram Bot API adapter for the LibreFang channel bridge.
//!
//! Uses long-polling via `getUpdates` with exponential backoff on failures.
//! No external Telegram crate — just `reqwest` for full control over error handling.

use crate::formatter;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
    InteractiveButton, InteractiveMessage, LifecycleReaction,
};
use async_trait::async_trait;
use futures::Stream;
use librefang_types::config::OutputFormat;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

// Backoff and long-poll timeout are now configurable via TelegramConfig.

/// Default Telegram Bot API base URL.
const DEFAULT_API_URL: &str = "https://api.telegram.org";

/// Minimum interval between `editMessageText` calls during streaming.
/// Telegram rate-limits bots to ~30 edits/minute per chat, so 1 second
/// provides a safe margin while keeping the UX responsive.
const STREAMING_EDIT_INTERVAL: Duration = Duration::from_millis(1000);

/// Truncate text to `max_len` bytes (respecting char boundaries) and append "..." if truncated.
fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if text.len() > max_len {
        format!("{}...", &text[..text.floor_char_boundary(max_len)])
    } else {
        text.to_string()
    }
}

/// Default retry delay (seconds) when Telegram doesn't specify `retry_after`.
const RETRY_AFTER_DEFAULT_SECS: u64 = 2;

/// Extract `retry_after` from a Telegram 429 response body.
fn extract_retry_after(body: &str, default: u64) -> u64 {
    body.parse::<serde_json::Value>()
        .ok()
        .and_then(|v| v["parameters"]["retry_after"].as_u64())
        .unwrap_or(default)
}

/// Telegram `parse_mode` for HTML formatting.
const PARSE_MODE_HTML: &str = "HTML";

/// Check if a Telegram chat type represents a group.
fn is_group_chat(chat_type: &str) -> bool {
    chat_type == "group" || chat_type == "supergroup"
}

/// Fire-and-forget HTTP POST. Logs errors at debug level.
fn fire_and_forget_post(client: reqwest::Client, url: String, body: serde_json::Value) {
    tokio::spawn(async move {
        match client.post(&url).json(&body).send().await {
            Ok(resp) if !resp.status().is_success() => {
                let body_text = resp.text().await.unwrap_or_default();
                debug!("Telegram fire-and-forget POST failed: {body_text}");
            }
            Err(e) => {
                debug!("Telegram fire-and-forget POST error: {e}");
            }
            _ => {}
        }
    });
}

/// Shared Telegram API context for free functions that need token/client/base_url.
struct TelegramApiCtx<'a> {
    token: &'a str,
    client: &'a reqwest::Client,
    api_base_url: &'a str,
}

impl<'a> TelegramApiCtx<'a> {
    /// Resolve a Telegram file_id to a download URL via the Bot API.
    async fn get_file_url(&self, file_id: &str) -> Option<String> {
        let url = format!("{}/bot{}/getFile", self.api_base_url, self.token);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({"file_id": file_id}))
            .send()
            .await
            .ok()?;
        let body: serde_json::Value = resp.json().await.ok()?;
        if body["ok"].as_bool() != Some(true) {
            return None;
        }
        let file_path = body["result"]["file_path"].as_str()?;
        Some(format!(
            "{}/file/bot{}/{}",
            self.api_base_url, self.token, file_path
        ))
    }
}

/// Telegram Bot API adapter using long-polling.
pub struct TelegramAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Arc<[String]>,
    poll_interval: Duration,
    /// Base URL for Telegram Bot API (supports proxies/mirrors).
    api_base_url: String,
    /// Bot username (without @), populated from `getMe` during `start()`.
    bot_username: std::sync::OnceLock<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Thread-based agent routing: thread_id -> agent name.
    thread_routes: HashMap<String, String>,
    /// Initial backoff on API failures.
    initial_backoff: Duration,
    /// Maximum backoff on API failures.
    max_backoff: Duration,
    /// Telegram long-polling timeout in seconds.
    long_poll_timeout: u64,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Handle for the polling task, used for graceful shutdown.
    poll_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// When true, remove the reaction on Done instead of showing 🎉.
    clear_done_reaction: bool,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// `token` is the raw bot token (read from env by the caller).
    /// `allowed_users` is the list of Telegram user IDs or usernames allowed to interact (empty = allow all).
    /// `api_url` overrides the Telegram Bot API base URL (for proxies/mirrors).
    pub fn new(
        token: String,
        allowed_users: Vec<String>,
        poll_interval: Duration,
        api_url: Option<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let api_base_url = api_url
            .unwrap_or_else(|| DEFAULT_API_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        Self {
            token: Zeroizing::new(token),
            client: crate::http_client::new_client(),
            allowed_users: allowed_users.into(),
            poll_interval,
            api_base_url,
            bot_username: std::sync::OnceLock::new(),
            account_id: None,
            thread_routes: HashMap::new(),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            long_poll_timeout: 30,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            poll_handle: Arc::new(tokio::sync::Mutex::new(None)),
            clear_done_reaction: false,
        }
    }

    /// When enabled, the Done reaction is removed (cleared) instead of
    /// showing a completion emoji.  Returns self for builder chaining.
    pub fn with_clear_done_reaction(mut self, clear: bool) -> Self {
        self.clear_done_reaction = clear;
        self
    }

    /// Set backoff and long-poll timeout configuration. Returns self for builder chaining.
    pub fn with_backoff(
        mut self,
        initial_backoff_secs: u64,
        max_backoff_secs: u64,
        long_poll_timeout_secs: u64,
    ) -> Self {
        self.initial_backoff = Duration::from_secs(initial_backoff_secs);
        self.max_backoff = Duration::from_secs(max_backoff_secs);
        self.long_poll_timeout = long_poll_timeout_secs;
        self
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Set thread-based agent routing. Returns self for builder chaining.
    pub fn with_thread_routes(mut self, thread_routes: HashMap<String, String>) -> Self {
        self.thread_routes = thread_routes;
        self
    }

    /// Parse the platform_id from a ChannelUser as a Telegram chat_id (i64).
    fn parse_chat_id(user: &ChannelUser) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        user.platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id).into())
    }

    /// Validate the bot token by calling `getMe`.
    pub async fn validate_token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/bot{}/getMe", self.api_base_url, self.token.as_str());
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        if resp["ok"].as_bool() != Some(true) {
            let desc = resp["description"].as_str().unwrap_or("unknown error");
            return Err(format!("Telegram getMe failed: {desc}").into());
        }

        let bot_name = resp["result"]["username"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        Ok(bot_name)
    }

    /// Call `sendMessage` on the Telegram API.
    ///
    /// When `thread_id` is provided, includes `message_thread_id` in the request
    /// so the message lands in the correct forum topic.
    async fn api_send_message(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/sendMessage",
            self.api_base_url,
            self.token.as_str()
        );

        // Sanitize: strip unsupported HTML tags so Telegram doesn't reject with 400.
        // Telegram only allows: b, i, u, s, tg-spoiler, a, code, pre, blockquote.
        // Any other tag (e.g. <name>, <thinking>) causes a 400 Bad Request.
        let sanitized = sanitize_telegram_html(text);

        // Telegram has a 4096 character limit per message — split if needed
        let chunks = split_message(&sanitized, 4096);
        for chunk in chunks {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": PARSE_MODE_HTML,
            });
            if let Some(tid) = thread_id {
                body["message_thread_id"] = serde_json::json!(tid);
            }

            let resp = self.client.post(&url).json(&body).send().await?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                warn!("Telegram sendMessage failed ({status}): {body_text}");
                // If HTML parsing failed, retry as plain text (no parse_mode)
                if status == reqwest::StatusCode::BAD_REQUEST
                    && body_text.contains("can't parse entities")
                {
                    let mut plain_body = serde_json::json!({
                        "chat_id": chat_id,
                        "text": chunk,
                    });
                    if let Some(tid) = thread_id {
                        plain_body["message_thread_id"] = serde_json::json!(tid);
                    }
                    let retry = self.client.post(&url).json(&plain_body).send().await?;
                    if !retry.status().is_success() {
                        let retry_text = retry.text().await.unwrap_or_default();
                        warn!("Telegram sendMessage plain fallback also failed: {retry_text}");
                    }
                }
            }
        }
        Ok(())
    }

    /// Generic helper for Telegram media API calls (sendPhoto, sendVoice, sendVideo, etc.)
    ///
    /// Handles URL construction, optional `message_thread_id`, and a single retry
    /// on HTTP 429 rate-limit responses (waiting `retry_after` seconds from the
    /// Telegram response body, defaulting to 2 seconds if the header is missing).
    async fn api_send_media_request(
        &self,
        endpoint: &str,
        chat_id: i64,
        body_fields: serde_json::Value,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/{endpoint}",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = body_fields;
        body["chat_id"] = serde_json::json!(chat_id);
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                warn!("Telegram {endpoint} rate limited, retrying after {retry_after}s");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;

                let resp2 = self.client.post(&url).json(&body).send().await?;
                if !resp2.status().is_success() {
                    let body_text2 = resp2.text().await.unwrap_or_default();
                    warn!("Telegram {endpoint} failed after retry: {body_text2}");
                }
                return Ok(());
            }

            warn!("Telegram {endpoint} failed ({status}): {body_text}");
        }
        Ok(())
    }

    /// Call `sendPhoto` on the Telegram API.
    async fn api_send_photo(
        &self,
        chat_id: i64,
        photo_url: &str,
        caption: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut body = serde_json::json!({ "photo": photo_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        self.api_send_media_request("sendPhoto", chat_id, body, thread_id)
            .await
    }

    /// Call `sendDocument` on the Telegram API.
    async fn api_send_document(
        &self,
        chat_id: i64,
        document_url: &str,
        filename: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "document": document_url,
            "caption": filename,
        });
        self.api_send_media_request("sendDocument", chat_id, body, thread_id)
            .await
    }

    /// Call `sendDocument` with multipart upload for local file data.
    ///
    /// Used by the proactive `channel_send` tool when `file_path` is provided.
    /// Uploads raw bytes as a multipart form instead of passing a URL.
    /// Retries once on HTTP 429 rate-limit responses.
    async fn api_send_document_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/sendDocument",
            self.api_base_url,
            self.token.as_str()
        );

        // Convert to ref-counted Bytes so cloning is O(1) (atomic ref-count bump)
        // instead of O(n) Vec deep-copy. Part::stream() accepts Bytes directly
        // (Into<Body>), unlike Part::bytes() which requires Into<Cow<'static, [u8]>>.
        let data_bytes = bytes::Bytes::from(data);

        let file_part = reqwest::multipart::Part::stream(data_bytes.clone())
            .file_name(filename.to_string())
            .mime_str(mime_type)?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", file_part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        let resp = self.client.post(&url).multipart(form).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                warn!("Telegram sendDocument upload rate limited, retrying after {retry_after}s");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;

                // Rebuild the multipart form — Bytes::clone() is O(1)
                let file_part = reqwest::multipart::Part::stream(data_bytes.clone())
                    .file_name(filename.to_string())
                    .mime_str(mime_type)?;
                let mut retry_form = reqwest::multipart::Form::new()
                    .text("chat_id", chat_id.to_string())
                    .part("document", file_part);
                if let Some(tid) = thread_id {
                    retry_form = retry_form.text("message_thread_id", tid.to_string());
                }

                let resp2 = self.client.post(&url).multipart(retry_form).send().await?;
                if !resp2.status().is_success() {
                    let body_text2 = resp2.text().await.unwrap_or_default();
                    warn!("Telegram sendDocument upload failed after retry: {body_text2}");
                }
                return Ok(());
            }

            warn!("Telegram sendDocument upload failed ({status}): {body_text}");
        }
        Ok(())
    }

    /// Call `sendVoice` on the Telegram API.
    async fn api_send_voice(
        &self,
        chat_id: i64,
        voice_url: &str,
        caption: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut body = serde_json::json!({ "voice": voice_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        self.api_send_media_request("sendVoice", chat_id, body, thread_id)
            .await
    }

    /// Call `sendVideo` on the Telegram API.
    async fn api_send_video(
        &self,
        chat_id: i64,
        video_url: &str,
        caption: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut body = serde_json::json!({ "video": video_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        self.api_send_media_request("sendVideo", chat_id, body, thread_id)
            .await
    }

    /// Call `sendLocation` on the Telegram API.
    async fn api_send_location(
        &self,
        chat_id: i64,
        lat: f64,
        lon: f64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "latitude": lat,
            "longitude": lon,
        });
        self.api_send_media_request("sendLocation", chat_id, body, thread_id)
            .await
    }

    /// Call `sendMessage` with an `InlineKeyboardMarkup` reply_markup.
    ///
    /// Sends a text message with inline keyboard buttons. Each inner Vec of
    /// `InteractiveButton` becomes one row of the keyboard.
    async fn api_send_interactive_message(
        &self,
        chat_id: i64,
        text: &str,
        buttons: &[Vec<InteractiveButton>],
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/sendMessage",
            self.api_base_url,
            self.token.as_str()
        );

        let sanitized = sanitize_telegram_html(text);

        // Build InlineKeyboardMarkup rows
        let keyboard: Vec<Vec<serde_json::Value>> = buttons
            .iter()
            .map(|row| {
                row.iter()
                    .map(|btn| {
                        if let Some(ref url) = btn.url {
                            // URL button — opens a link, no callback
                            serde_json::json!({
                                "text": btn.label,
                                "url": url,
                            })
                        } else {
                            // Callback button — sends callback_query to the bot
                            // Telegram limits callback_data to 64 bytes
                            let action = if btn.action.len() > 64 {
                                btn.action[..64].to_string()
                            } else {
                                btn.action.clone()
                            };
                            serde_json::json!({
                                "text": btn.label,
                                "callback_data": action,
                            })
                        }
                    })
                    .collect()
            })
            .collect();

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": sanitized,
            "parse_mode": PARSE_MODE_HTML,
            "reply_markup": {
                "inline_keyboard": keyboard,
            },
        });

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendMessage (interactive) failed ({status}): {body_text}");
        }
        Ok(())
    }

    /// Call `sendChatAction` to show "typing..." indicator.
    ///
    /// When `thread_id` is provided, the typing indicator appears in the forum topic.
    async fn api_send_typing(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/sendChatAction",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing",
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let _ = self.client.post(&url).json(&body).send().await?;
        Ok(())
    }

    /// Call `sendMessage` and return the message_id of the sent message.
    ///
    /// Used for streaming: we send an initial placeholder, then edit it in-place
    /// as tokens arrive. Returns `None` if the API call fails.
    ///
    /// The initial message is sent with `parse_mode: HTML` after sanitization.
    /// The `formatter::format_for_channel` output is expected as input, which
    /// produces Telegram-compatible HTML from Markdown.
    async fn api_send_message_returning_id(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
    ) -> Option<i64> {
        let url = format!(
            "{}/bot{}/sendMessage",
            self.api_base_url,
            self.token.as_str()
        );
        // No sanitization here — callers (send_streaming) already format via
        // formatter::format_for_channel which produces Telegram-safe HTML.
        // Double-sanitizing would escape already-valid entities.
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": PARSE_MODE_HTML,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let json: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("Telegram sendMessage (streaming init): failed to parse response JSON: {e}");
                        return None;
                    }
                };
                let msg_id = json["result"]["message_id"].as_i64();
                if msg_id.is_none() {
                    warn!(
                        "Telegram sendMessage (streaming init): response missing result.message_id"
                    );
                }
                msg_id
            }
            Ok(resp) => {
                let body_text = resp.text().await.unwrap_or_default();
                warn!("Telegram sendMessage (streaming init) failed: {body_text}");
                None
            }
            Err(e) => {
                warn!("Telegram sendMessage (streaming init) network error: {e}");
                None
            }
        }
    }

    /// Call `editMessageText` on the Telegram API to update an existing message.
    ///
    /// Used during streaming to progressively replace the message content with
    /// accumulated tokens. Silently ignores errors (best-effort) since the final
    /// complete text will be sent as a fallback if editing fails.
    ///
    /// Sends the text with `parse_mode: HTML`. Callers are expected to provide
    /// Telegram-safe HTML (e.g., via `formatter::format_for_channel`).
    async fn api_edit_message(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/editMessageText",
            self.api_base_url,
            self.token.as_str()
        );
        // No sanitization here — callers (send_streaming) already format via
        // formatter::format_for_channel which produces Telegram-safe HTML.
        // Double-sanitizing would escape already-valid entities.
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "parse_mode": PARSE_MODE_HTML,
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            // Telegram returns 400 "message is not modified" when text hasn't changed —
            // this is expected and harmless.
            if !body_text.contains("message is not modified") {
                warn!("Telegram editMessageText failed ({status}): {body_text}");
            }
        }
        Ok(())
    }

    /// Call `setMessageReaction` on the Telegram API (fire-and-forget).
    ///
    /// Sets or replaces the bot's emoji reaction on a message. Each new call
    /// automatically replaces the previous reaction, so there is no need to
    /// explicitly remove old ones.
    fn fire_reaction(&self, chat_id: i64, message_id: i64, emoji: &str) {
        let url = format!(
            "{}/bot{}/setMessageReaction",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": emoji}],
        });
        self.fire_reaction_body(url, body);
    }

    /// Remove all bot reactions from a message.
    fn clear_reactions(&self, chat_id: i64, message_id: i64) {
        let url = format!(
            "{}/bot{}/setMessageReaction",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [],
        });
        self.fire_reaction_body(url, body);
    }

    fn fire_reaction_body(&self, url: String, body: serde_json::Value) {
        fire_and_forget_post(self.client.clone(), url, body);
    }
}

impl TelegramAdapter {
    /// Internal helper: send content with optional forum-topic thread_id.
    ///
    /// Both `send()` and `send_in_thread()` delegate here. When `thread_id` is
    /// `Some(id)`, every outbound Telegram API call includes `message_thread_id`
    /// so the message lands in the correct forum topic.
    async fn send_content(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chat_id = Self::parse_chat_id(user)?;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(chat_id, &text, thread_id).await?;
            }
            ChannelContent::Image { url, caption, .. } => {
                self.api_send_photo(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                self.api_send_document(chat_id, &url, &filename, thread_id)
                    .await?;
            }
            ChannelContent::FileData {
                data,
                filename,
                mime_type,
            } => {
                self.api_send_document_upload(chat_id, data, &filename, &mime_type, thread_id)
                    .await?;
            }
            ChannelContent::Voice { url, caption, .. } => {
                self.api_send_voice(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::Video { url, caption, .. } => {
                self.api_send_video(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::Location { lat, lon } => {
                self.api_send_location(chat_id, lat, lon, thread_id).await?;
            }
            ChannelContent::Command { name, args } => {
                let text = format!("/{name} {}", args.join(" "));
                self.api_send_message(chat_id, text.trim(), thread_id)
                    .await?;
            }
            ChannelContent::Interactive { text, buttons } => {
                self.api_send_interactive_message(chat_id, &text, &buttons, thread_id)
                    .await?;
            }
            ChannelContent::ButtonCallback { action, .. } => {
                // Outbound ButtonCallback doesn't make sense — log and skip
                debug!("Telegram: ignoring outbound ButtonCallback (action={action})");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Validate token first (fail fast) and store bot username for mention detection
        let bot_name = self.validate_token().await?;
        let _ = self.bot_username.set(bot_name.clone());
        info!("Telegram bot @{bot_name} connected");

        // Clear any existing webhook to avoid 409 Conflict during getUpdates polling.
        // This is necessary when the daemon restarts — the old polling session may
        // still be active on Telegram's side for ~30s, causing 409 errors.
        {
            let delete_url = format!(
                "{}/bot{}/deleteWebhook",
                self.api_base_url,
                self.token.as_str()
            );
            match self
                .client
                .post(&delete_url)
                .json(&serde_json::json!({"drop_pending_updates": true}))
                .send()
                .await
            {
                Ok(_) => info!("Telegram: cleared webhook, polling mode active"),
                Err(e) => tracing::warn!("Telegram: deleteWebhook failed (non-fatal): {e}"),
            }
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let token = self.token.clone();
        let client = self.client.clone();
        let allowed_users = self.allowed_users.clone();
        let poll_interval = self.poll_interval;
        let api_base_url = self.api_base_url.clone();
        let bot_username = self.bot_username.get().cloned();
        let account_id = self.account_id.clone();
        let thread_routes = self.thread_routes.clone();
        let mut shutdown = self.shutdown_rx.clone();
        let initial_backoff = self.initial_backoff;
        let max_backoff = self.max_backoff;
        let long_poll_timeout = self.long_poll_timeout;
        let poll_handle = self.poll_handle.clone();

        let handle = tokio::spawn(async move {
            let ctx = TelegramApiCtx {
                token: token.as_str(),
                client: &client,
                api_base_url: &api_base_url,
            };
            let mut offset: Option<i64> = None;
            let mut backoff = initial_backoff;

            loop {
                if *shutdown.borrow() {
                    break;
                }

                // Build getUpdates request
                let url = format!("{}/bot{}/getUpdates", api_base_url, token.as_str());
                let mut params = serde_json::json!({
                    "timeout": long_poll_timeout,
                    "allowed_updates": ["message", "edited_message", "callback_query"],
                });
                if let Some(off) = offset {
                    params["offset"] = serde_json::json!(off);
                }

                // Make the request with a timeout slightly longer than the long-poll timeout
                let request_timeout = Duration::from_secs(long_poll_timeout + 10);
                let result = tokio::select! {
                    res = async {
                        client
                            .post(&url)
                            .json(&params)
                            .timeout(request_timeout)
                            .send()
                            .await
                    } => res,
                    _ = shutdown.changed() => {
                        break;
                    }
                };

                let resp = match result {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!("Telegram getUpdates network error: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = calculate_backoff(backoff, max_backoff);
                        continue;
                    }
                };

                let status = resp.status();

                // Handle rate limiting
                if status.as_u16() == 429 {
                    let body_text = resp.text().await.unwrap_or_default();
                    let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                    warn!("Telegram rate limited, retry after {retry_after}s");
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }

                // Handle conflict (another bot instance or stale session polling).
                // On daemon restart, the old long-poll may still be active on Telegram's
                // side for up to 30s. Retry with backoff instead of stopping permanently.
                if status.as_u16() == 409 {
                    warn!("Telegram 409 Conflict — stale polling session, retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = calculate_backoff(backoff, max_backoff);
                    continue;
                }

                if !status.is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    warn!("Telegram getUpdates failed ({status}): {body_text}, retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = calculate_backoff(backoff, max_backoff);
                    continue;
                }

                // Parse response
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("Telegram getUpdates parse error: {e}");
                        tokio::time::sleep(backoff).await;
                        backoff = calculate_backoff(backoff, max_backoff);
                        continue;
                    }
                };

                if body["ok"].as_bool() != Some(true) {
                    warn!("Telegram getUpdates returned ok=false");
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }

                backoff = initial_backoff;

                let updates = match body["result"].as_array() {
                    Some(arr) => arr,
                    None => {
                        warn!(
                            "Telegram getUpdates returned ok=true but result is not an array: {}",
                            body["result"]
                        );
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                };

                for update in updates {
                    if let Some(update_id) = update["update_id"].as_i64() {
                        offset = Some(update_id + 1);
                    }

                    // Handle callback_query (inline keyboard button clicks)
                    if let Some(callback) = update.get("callback_query") {
                        if let Some(mut msg) =
                            parse_telegram_callback_query(callback, &allowed_users, &ctx)
                        {
                            if let Some(ref aid) = account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            debug!(
                                "Telegram callback from {}: {:?}",
                                msg.sender.display_name, msg.content
                            );
                            if tx.send(msg).await.is_err() {
                                error!(
                                    "Telegram dispatch channel closed — callback dropped. \
                                     Bridge receiver may have been deallocated."
                                );
                                return;
                            }
                        }
                        continue;
                    }

                    let bot_uname = bot_username.clone();
                    let mut msg = match parse_telegram_update(
                        update,
                        &allowed_users,
                        &ctx,
                        bot_uname.as_deref(),
                    )
                    .await
                    {
                        Ok(m) => m,
                        Err(DropReason::Filtered(reason)) => {
                            debug!("Telegram message filtered: {reason}");
                            continue;
                        }
                        Err(DropReason::ParseError(reason)) => {
                            warn!("Telegram message dropped before agent dispatch: {reason}");
                            continue;
                        }
                    };

                    // Tag message with account_id for multi-bot routing
                    if let Some(ref aid) = account_id {
                        msg.metadata
                            .insert("account_id".to_string(), serde_json::json!(aid));
                    }

                    // Thread-based agent routing: if this message's thread_id
                    // matches a configured route, tag it for the bridge dispatcher.
                    if let Some(ref tid) = msg.thread_id {
                        if let Some(agent_name) = thread_routes.get(tid) {
                            msg.metadata.insert(
                                "thread_route_agent".to_string(),
                                serde_json::json!(agent_name),
                            );
                            debug!("Telegram thread {tid} routed to agent '{agent_name}'");
                        }
                    }

                    debug!(
                        "Telegram message from {}: {:?}",
                        msg.sender.display_name, msg.content
                    );

                    if tx.send(msg).await.is_err() {
                        error!(
                            "Telegram dispatch channel closed — message dropped. \
                             Bridge receiver may have been deallocated."
                        );
                        return;
                    }
                }

                tokio::time::sleep(poll_interval).await;
            }

            info!("Telegram polling loop stopped");
        });

        {
            let mut guard = poll_handle.lock().await;
            *guard = Some(handle);
        }

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.send_content(user, content, None).await
    }

    async fn send_interactive(
        &self,
        user: &ChannelUser,
        message: &InteractiveMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chat_id = Self::parse_chat_id(user)?;
        self.api_send_interactive_message(chat_id, &message.text, &message.buttons, None)
            .await
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chat_id = Self::parse_chat_id(user)?;
        self.api_send_typing(chat_id, None).await
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tid: Option<i64> = thread_id.parse().ok();
        self.send_content(user, content, tid).await
    }

    async fn send_reaction(
        &self,
        user: &ChannelUser,
        message_id: &str,
        reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chat_id = Self::parse_chat_id(user)?;
        let msg_id: i64 = message_id
            .parse()
            .map_err(|_| format!("Invalid Telegram message_id: {message_id}"))?;
        // Telegram only supports a limited set of reaction emoji.
        // Map unsupported ones to the closest Telegram-compatible alternative.
        let emoji = match reaction.emoji.as_str() {
            "\u{23F3}" => "\u{1F440}",        // ⏳ → 👀
            "\u{2699}\u{FE0F}" => "\u{26A1}", // ⚙️ → ⚡
            "\u{2705}" => "\u{1F389}",        // ✅ → 🎉
            "\u{274C}" => "\u{1F44E}",        // ❌ → 👎
            other => other,                   // 🤔, ✍️ etc. pass through
        };

        // Optionally clear the reaction on completion instead of showing 🎉.
        let is_done = reaction.emoji == "\u{2705}"; // ✅
        if is_done && self.clear_done_reaction {
            self.clear_reactions(chat_id, msg_id);
        } else {
            self.fire_reaction(chat_id, msg_id, emoji);
        }
        Ok(())
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn send_streaming(
        &self,
        user: &ChannelUser,
        mut delta_rx: mpsc::Receiver<String>,
        thread_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let chat_id = Self::parse_chat_id(user)?;
        let tid: Option<i64> = thread_id.and_then(|t| t.parse().ok());

        // Send typing indicator while we wait for the first token.
        let _ = self.api_send_typing(chat_id, tid).await;

        // Accumulate the full response text.
        let mut full_text = String::new();
        let mut sent_message_id: Option<i64> = None;
        let mut last_edit = Instant::now();

        while let Some(delta) = delta_rx.recv().await {
            full_text.push_str(&delta);

            // Send the initial message on the first token.
            if sent_message_id.is_none() {
                let intermediate =
                    formatter::format_for_channel(&full_text, OutputFormat::TelegramHtml);
                if let Some(msg_id) = self
                    .api_send_message_returning_id(chat_id, &intermediate, tid)
                    .await
                {
                    sent_message_id = Some(msg_id);
                    last_edit = Instant::now();
                }
                continue;
            }

            // Throttle edits to respect Telegram rate limits.
            if last_edit.elapsed() >= STREAMING_EDIT_INTERVAL {
                let intermediate =
                    formatter::format_for_channel(&full_text, OutputFormat::TelegramHtml);
                if let Some(msg_id) = sent_message_id {
                    let _ = self.api_edit_message(chat_id, msg_id, &intermediate).await;
                    last_edit = Instant::now();
                }
            }
        }

        // Final edit with the complete, formatted text to ensure nothing is lost.
        let formatted = formatter::format_for_channel(&full_text, OutputFormat::TelegramHtml);

        if let Some(msg_id) = sent_message_id {
            // Split *before* sanitization — api_edit_message / api_send_message
            // sanitize internally, so pre-sanitizing here would double-escape
            // HTML entities.
            let chunks = split_message(&formatted, 4096);
            if chunks.len() <= 1 {
                // Single message — just edit in place.
                let _ = self.api_edit_message(chat_id, msg_id, &formatted).await;
            } else {
                // Response exceeds 4096 chars — edit the first chunk in place,
                // then send remaining chunks as new messages.
                let _ = self.api_edit_message(chat_id, msg_id, chunks[0]).await;
                for chunk in &chunks[1..] {
                    let _ = self.api_send_message(chat_id, chunk, tid).await;
                }
            }
        } else if !full_text.is_empty() {
            // No streaming message was ever sent (first token never arrived
            // or sendMessage failed) — fall back to a normal send.
            self.api_send_message(chat_id, &formatted, tid).await?;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        let mut guard = self.poll_handle.lock().await;
        if let Some(handle) = guard.take() {
            // Give the polling loop up to 5 seconds to finish
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(())) => info!("Telegram polling loop stopped gracefully"),
                Ok(Err(e)) => warn!("Telegram polling task panicked: {e}"),
                Err(_) => warn!("Telegram polling loop did not stop within 5s timeout"),
            }
        }
        Ok(())
    }
}

/// Reason a Telegram update was not dispatched to an agent.
#[derive(Debug)]
enum DropReason {
    /// Intentional policy filter (e.g. allowed_users). Log at debug level.
    Filtered(String),
    /// Unexpected parse failure or malformed data. Log at warn level.
    ParseError(String),
}

/// Check if `haystack` ends with `suffix`, comparing ASCII case-insensitively.
fn ends_with_ascii_ci(haystack: &str, suffix: &str) -> bool {
    if haystack.len() < suffix.len() {
        return false;
    }
    haystack[haystack.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Detect image MIME type from a Telegram file path or download URL.
///
/// Telegram file paths typically look like `photos/file_42.jpg` so the
/// extension is a reliable signal. Falls back to `None` if no known
/// image extension is found, letting downstream code use magic-byte
/// detection or a safe default.
fn mime_type_from_telegram_path(url_or_path: &str) -> Option<&'static str> {
    if ends_with_ascii_ci(url_or_path, ".jpg") || ends_with_ascii_ci(url_or_path, ".jpeg") {
        Some("image/jpeg")
    } else if ends_with_ascii_ci(url_or_path, ".png") {
        Some("image/png")
    } else if ends_with_ascii_ci(url_or_path, ".gif") {
        Some("image/gif")
    } else if ends_with_ascii_ci(url_or_path, ".webp") {
        Some("image/webp")
    } else if ends_with_ascii_ci(url_or_path, ".bmp") {
        Some("image/bmp")
    } else if ends_with_ascii_ci(url_or_path, ".tiff") || ends_with_ascii_ci(url_or_path, ".tif") {
        Some("image/tiff")
    } else {
        None
    }
}

/// Check whether a Telegram user is allowed based on the `allowed_users` list.
///
/// Matching rules:
/// 1. Empty list → allow everyone.
/// 2. Exact match on `user_id` (compared as string).
/// 3. If `username` is present, normalized case-insensitive match
///    (both the entry and the username are stripped of a leading `@`).
fn telegram_user_allowed(allowed_users: &[String], user_id: i64, username: Option<&str>) -> bool {
    if allowed_users.is_empty() {
        return true;
    }
    let user_id_str = user_id.to_string();
    if allowed_users.iter().any(|u| u == &user_id_str) {
        return true;
    }
    if let Some(uname) = username {
        let normalized = uname.trim_start_matches('@').to_lowercase();
        allowed_users
            .iter()
            .any(|u| u.trim_start_matches('@').to_lowercase() == normalized)
    } else {
        false
    }
}

/// Parse a Telegram `callback_query` update into a `ChannelMessage`.
///
/// Called when a user clicks an inline keyboard button. The callback data
/// is delivered as a `ButtonCallback` content variant, and the bot answers
/// the callback query to dismiss the loading indicator.
fn parse_telegram_callback_query(
    callback: &serde_json::Value,
    allowed_users: &[String],
    ctx: &TelegramApiCtx<'_>,
) -> Option<ChannelMessage> {
    let callback_query_id = callback["id"].as_str()?;
    let from = callback.get("from")?;
    let user_id = from["id"].as_i64()?;
    let username = from["username"].as_str();

    // Security: check allowed_users (supports user ID and username)
    if !telegram_user_allowed(allowed_users, user_id, username) {
        debug!(
            "Telegram callback_query filtered: user {user_id} (username: {}) not in allowed_users",
            username.unwrap_or("none")
        );
        return None;
    }

    let user_id_str = user_id.to_string();

    let first_name = from["first_name"].as_str().unwrap_or("Unknown");
    let last_name = from["last_name"].as_str().unwrap_or("");
    let display_name = if last_name.is_empty() {
        first_name.to_string()
    } else {
        format!("{first_name} {last_name}")
    };

    let callback_data = callback["data"].as_str().unwrap_or("");
    if callback_data.is_empty() {
        return None;
    }

    // Extract chat_id from the original message
    let message = callback.get("message")?;
    let chat_id = message["chat"]["id"].as_i64()?;
    let message_id = message["message_id"].as_i64().unwrap_or(0);
    let message_text = message["text"].as_str().map(String::from);
    let chat_type = message["chat"]["type"].as_str().unwrap_or("private");
    let is_group = is_group_chat(chat_type);

    let timestamp = message["date"]
        .as_i64()
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(chrono::Utc::now);

    // Fire-and-forget answer to dismiss the button loading state
    {
        let url = format!("{}/bot{}/answerCallbackQuery", ctx.api_base_url, ctx.token);
        let body = serde_json::json!({
            "callback_query_id": callback_query_id,
        });
        fire_and_forget_post(ctx.client.clone(), url, body);
    }

    let mut metadata = HashMap::new();
    metadata.insert(
        "callback_query_id".to_string(),
        serde_json::json!(callback_query_id),
    );
    metadata.insert("user_id".to_string(), serde_json::json!(user_id_str));
    metadata.insert(
        "message_id".to_string(),
        serde_json::json!(message_id.to_string()),
    );

    // Thread ID for forum topics
    let thread_id = message["message_thread_id"].as_i64().map(|t| t.to_string());

    Some(ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name,
            librefang_user: None,
        },
        content: ChannelContent::ButtonCallback {
            action: callback_data.to_string(),
            message_text,
        },
        target_agent: None,
        timestamp,
        is_group,
        thread_id,
        metadata,
    })
}

/// Extract sender identity from a Telegram message.
///
/// Tries `from` (user) first, then falls back to `sender_chat` (channel/group).
/// Returns `(user_id, display_name, Option<username>)`.
fn extract_telegram_sender(
    message: &serde_json::Value,
    update_id: i64,
) -> Result<(i64, String, Option<String>), DropReason> {
    if let Some(from) = message.get("from") {
        let uid = match from["id"].as_i64() {
            Some(id) => id,
            None => {
                return Err(DropReason::ParseError(format!(
                    "update {update_id}: from.id is not an integer"
                )));
            }
        };
        let first_name = from["first_name"].as_str().unwrap_or("Unknown");
        let last_name = from["last_name"].as_str().unwrap_or("");
        let name = if last_name.is_empty() {
            first_name.to_string()
        } else {
            format!("{first_name} {last_name}")
        };
        let username = from["username"].as_str().map(String::from);
        Ok((uid, name, username))
    } else if let Some(sender_chat) = message.get("sender_chat") {
        // Messages sent on behalf of a channel or group have `sender_chat` instead of `from`.
        let uid = match sender_chat["id"].as_i64() {
            Some(id) => id,
            None => {
                return Err(DropReason::ParseError(format!(
                    "update {update_id}: sender_chat.id is not an integer"
                )));
            }
        };
        let title = sender_chat["title"].as_str().unwrap_or("Unknown Channel");
        Ok((uid, title.to_string(), None))
    } else {
        Err(DropReason::ParseError(format!(
            "update {update_id} has no from or sender_chat field"
        )))
    }
}

/// Determine the content type from a Telegram message.
///
/// Handles: text, photo, document, audio, voice, video, video_note, location.
/// Falls back to DropReason::Filtered for unsupported types.
async fn extract_telegram_content(
    message: &serde_json::Value,
    update_id: i64,
    ctx: &TelegramApiCtx<'_>,
) -> Result<ChannelContent, DropReason> {
    if let Some(text) = message["text"].as_str() {
        // Parse bot commands (Telegram sends entities for /commands)
        if let Some(entities) = message["entities"].as_array() {
            let is_bot_command = entities.iter().any(|e| {
                e["type"].as_str() == Some("bot_command") && e["offset"].as_i64() == Some(0)
            });
            if is_bot_command {
                let parts: Vec<&str> = text.splitn(2, ' ').collect();
                let cmd_name = parts[0].trim_start_matches('/');
                let cmd_name = cmd_name.split('@').next().unwrap_or(cmd_name);
                let args = if parts.len() > 1 {
                    parts[1].split_whitespace().map(String::from).collect()
                } else {
                    vec![]
                };
                Ok(ChannelContent::Command {
                    name: cmd_name.to_string(),
                    args,
                })
            } else {
                Ok(ChannelContent::Text(text.to_string()))
            }
        } else {
            Ok(ChannelContent::Text(text.to_string()))
        }
    } else if let Some(photos) = message["photo"].as_array() {
        // Photos come as array of sizes; pick the largest (last)
        let file_id = photos
            .last()
            .and_then(|p| p["file_id"].as_str())
            .unwrap_or("");
        let caption = message["caption"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => {
                let mime_type = mime_type_from_telegram_path(&url).map(String::from);
                Ok(ChannelContent::Image {
                    url,
                    caption,
                    mime_type,
                })
            }
            None => Ok(ChannelContent::Text(format!(
                "[Photo received{}]",
                caption
                    .as_deref()
                    .map(|c| format!(": {c}"))
                    .unwrap_or_default()
            ))),
        }
    } else if message.get("document").is_some() {
        let file_id = message["document"]["file_id"].as_str().unwrap_or("");
        let filename = message["document"]["file_name"]
            .as_str()
            .unwrap_or("document")
            .to_string();
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::File { url, filename }),
            None => Ok(ChannelContent::Text(format!(
                "[Document received: {filename}]"
            ))),
        }
    } else if message.get("audio").is_some() {
        // Audio files (mp3, etc.) — treat as voice for transcription
        let file_id = message["audio"]["file_id"].as_str().unwrap_or("");
        let duration = message["audio"]["duration"].as_u64().unwrap_or(0) as u32;
        let caption = message["caption"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Voice {
                url,
                caption,
                duration_seconds: duration,
            }),
            None => Ok(ChannelContent::Text(format!(
                "[Audio received, {duration}s{}]",
                caption
                    .as_deref()
                    .map(|c| format!(": {c}"))
                    .unwrap_or_default()
            ))),
        }
    } else if message.get("voice").is_some() {
        let file_id = message["voice"]["file_id"].as_str().unwrap_or("");
        let duration = message["voice"]["duration"].as_u64().unwrap_or(0) as u32;
        let caption = message["caption"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Voice {
                url,
                caption,
                duration_seconds: duration,
            }),
            None => Ok(ChannelContent::Text(format!(
                "[Voice message, {duration}s]"
            ))),
        }
    } else if message.get("video").is_some() {
        let file_id = message["video"]["file_id"].as_str().unwrap_or("");
        let duration = message["video"]["duration"].as_u64().unwrap_or(0) as u32;
        let caption = message["caption"].as_str().map(String::from);
        let filename = message["video"]["file_name"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Video {
                url,
                caption,
                duration_seconds: duration,
                filename,
            }),
            None => Ok(ChannelContent::Text(format!(
                "[Video received, {duration}s{}]",
                caption
                    .as_deref()
                    .map(|c| format!(": {c}"))
                    .unwrap_or_default()
            ))),
        }
    } else if message.get("video_note").is_some() {
        // Video notes are round video messages (no caption/filename)
        let file_id = message["video_note"]["file_id"].as_str().unwrap_or("");
        let duration = message["video_note"]["duration"].as_u64().unwrap_or(0) as u32;
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Video {
                url,
                caption: None,
                duration_seconds: duration,
                filename: None,
            }),
            None => Ok(ChannelContent::Text(format!("[Video note, {duration}s]"))),
        }
    } else if message.get("location").is_some() {
        let lat = message["location"]["latitude"].as_f64().unwrap_or(0.0);
        let lon = message["location"]["longitude"].as_f64().unwrap_or(0.0);
        Ok(ChannelContent::Location { lat, lon })
    } else {
        // Unsupported message type (stickers, polls, etc.)
        Err(DropReason::Filtered(format!(
            "update {update_id}: unsupported message type (no text/photo/document/voice/video/video_note/location)"
        )))
    }
}

/// Apply reply-to-message context to the content.
///
/// If the message is a reply, prepends the quoted text and optionally includes
/// the quoted photo.
async fn apply_reply_context(
    content: ChannelContent,
    message: &serde_json::Value,
    ctx: &TelegramApiCtx<'_>,
) -> ChannelContent {
    let reply = match message.get("reply_to_message") {
        Some(r) => r,
        None => return content,
    };

    let reply_sender = reply["from"]["first_name"].as_str().unwrap_or("Someone");
    let reply_text = reply["text"].as_str().or_else(|| reply["caption"].as_str());

    // Check if the quoted message has a photo
    let reply_photo_url = if let Some(photos) = reply["photo"].as_array() {
        let file_id = photos
            .last()
            .and_then(|p| p["file_id"].as_str())
            .unwrap_or("");
        if !file_id.is_empty() {
            ctx.get_file_url(file_id).await
        } else {
            None
        }
    } else {
        None
    };

    if let Some(photo_url) = reply_photo_url {
        // Quoted message has a photo.
        // If the user's own message is already an image, keep it and add
        // the quoted photo context as text (don't overwrite the user's photo).
        let quote_context = reply_text
            .map(|q| {
                let truncated = truncate_with_ellipsis(q, 200);
                format!("[Replying to {reply_sender}: \"{truncated}\"]\n")
            })
            .unwrap_or_else(|| format!("[Replying to {reply_sender}'s photo]\n"));

        match content {
            ChannelContent::Image {
                url,
                caption,
                mime_type,
            } => {
                // User sent their own photo as reply — keep it, add quoted context to caption
                let cap = caption.unwrap_or_default();
                ChannelContent::Image {
                    url,
                    caption: Some(format!("{quote_context}{cap}")),
                    mime_type,
                }
            }
            ChannelContent::Text(t) => {
                // User sent text reply to a photo — show the quoted photo
                let caption = format!("{quote_context}{t}");
                let mime_type = mime_type_from_telegram_path(&photo_url).map(String::from);
                ChannelContent::Image {
                    url: photo_url,
                    caption: Some(caption),
                    mime_type,
                }
            }
            other => other,
        }
    } else if let Some(quoted) = reply_text {
        // Quoted message has text only — prepend it
        let truncated = truncate_with_ellipsis(quoted, 200);
        let prefix = format!("[Replying to {reply_sender}: \"{truncated}\"]\n");
        match content {
            ChannelContent::Text(t) => ChannelContent::Text(format!("{prefix}{t}")),
            other => other,
        }
    } else {
        content
    }
}

async fn parse_telegram_update(
    update: &serde_json::Value,
    allowed_users: &[String],
    ctx: &TelegramApiCtx<'_>,
    bot_username: Option<&str>,
) -> Result<ChannelMessage, DropReason> {
    let update_id = update["update_id"].as_i64().unwrap_or(0);
    let message = match update
        .get("message")
        .or_else(|| update.get("edited_message"))
    {
        Some(m) => m,
        None => {
            return Err(DropReason::ParseError(format!(
                "update {update_id} has no message or edited_message field"
            )));
        }
    };

    let (user_id, display_name, username) = extract_telegram_sender(message, update_id)?;

    // Security: check allowed_users (supports user ID and username)
    if !telegram_user_allowed(allowed_users, user_id, username.as_deref()) {
        return Err(DropReason::Filtered(format!(
            "update {update_id}: user {user_id} (username: {}) not in allowed_users list",
            username.as_deref().unwrap_or("none")
        )));
    }

    let chat_id = message["chat"]["id"].as_i64().ok_or_else(|| {
        DropReason::ParseError(format!("update {update_id}: chat.id is not an integer"))
    })?;
    let chat_type = message["chat"]["type"].as_str().unwrap_or("private");
    let is_group = is_group_chat(chat_type);
    let message_id = message["message_id"].as_i64().unwrap_or(0);
    let timestamp = message["date"]
        .as_i64()
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(chrono::Utc::now);

    let content = extract_telegram_content(message, update_id, ctx).await?;
    let content = apply_reply_context(content, message, ctx).await;

    // Extract forum topic thread_id (Telegram sends this as `message_thread_id`
    // for messages inside forum topics / reply threads).
    let thread_id = message["message_thread_id"]
        .as_i64()
        .map(|tid| tid.to_string());

    // Build metadata
    let mut metadata = HashMap::new();

    // Store reply-to-message metadata for downstream consumers.
    if let Some(reply) = message.get("reply_to_message") {
        let reply_message_id = reply["message_id"].as_i64().unwrap_or(0);
        let reply_text = reply["text"]
            .as_str()
            .or_else(|| reply["caption"].as_str())
            .unwrap_or("");
        let reply_sender = reply
            .get("from")
            .and_then(|f| f["first_name"].as_str())
            .unwrap_or("Unknown");
        metadata.insert(
            "reply_to".to_string(),
            serde_json::json!({
                "message_id": reply_message_id,
                "sender": reply_sender,
                "text": reply_text,
            }),
        );
    }

    if is_group {
        if let Some(bot_uname) = bot_username {
            let was_mentioned = check_mention_entities(message, bot_uname);
            if was_mentioned {
                metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
            }
        }
    }

    Ok(ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name,
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp,
        is_group,
        thread_id,
        metadata,
    })
}

/// Convert a UTF-16 code unit offset (as returned by Telegram API) to a byte
/// offset suitable for Rust `&str` slicing.
fn utf16_offset_to_byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        // BMP characters = 1 UTF-16 unit, non-BMP (surrogate pairs) = 2
        utf16_count += if ch as u32 > 0xFFFF { 2 } else { 1 };
    }
    text.len()
}

/// Check whether the bot was @mentioned in a Telegram message.
///
/// Inspects both `entities` (for text messages) and `caption_entities` (for media
/// with captions) for entity type `"mention"` whose text matches `@bot_username`.
fn check_mention_entities(message: &serde_json::Value, bot_username: &str) -> bool {
    let bot_mention = format!("@{}", bot_username.to_lowercase());

    // Check both entities (text messages) and caption_entities (photo/document captions)
    for entities_key in &["entities", "caption_entities"] {
        if let Some(entities) = message[entities_key].as_array() {
            // Get the text that the entities refer to
            let text = if *entities_key == "entities" {
                message["text"].as_str().unwrap_or("")
            } else {
                message["caption"].as_str().unwrap_or("")
            };

            for entity in entities {
                if entity["type"].as_str() != Some("mention") {
                    continue;
                }
                let utf16_offset = entity["offset"].as_i64().unwrap_or(0) as usize;
                let utf16_length = entity["length"].as_i64().unwrap_or(0) as usize;
                let start = utf16_offset_to_byte_offset(text, utf16_offset);
                let end = utf16_offset_to_byte_offset(text, utf16_offset + utf16_length);
                if start < text.len() && end <= text.len() {
                    let mention_text = &text[start..end];
                    if mention_text.to_lowercase() == bot_mention {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Calculate exponential backoff capped at the given maximum.
fn calculate_backoff(current: Duration, max: Duration) -> Duration {
    (current * 2).min(max)
}

/// Sanitize text for Telegram HTML parse mode.
///
/// Escapes angle brackets that are NOT part of Telegram-allowed HTML tags.
/// Allowed tags: b, i, u, s, tg-spoiler, a, code, pre, blockquote.
/// Everything else (e.g. `<name>`, `<thinking>`) gets escaped to `&lt;...&gt;`.
fn sanitize_telegram_html(text: &str) -> String {
    const ALLOWED: &[&str] = &[
        "b",
        "i",
        "u",
        "s",
        "em",
        "strong",
        "a",
        "code",
        "pre",
        "blockquote",
        "tg-spoiler",
        "tg-emoji",
    ];

    let mut result = String::with_capacity(text.len() + text.len() / 4);
    let mut chars = text.char_indices().peekable();
    let mut open_tags: Vec<String> = Vec::new();

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '<' {
            // Try to parse an HTML tag
            if let Some(end_offset) = text[i..].find('>') {
                let tag_end = i + end_offset;
                let tag_content = &text[i + 1..tag_end]; // content between < and >
                let is_closing = tag_content.starts_with('/');
                let tag_name_raw = tag_content
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .next()
                    .unwrap_or("");

                if !tag_name_raw.is_empty()
                    && ALLOWED.iter().any(|a| a.eq_ignore_ascii_case(tag_name_raw))
                {
                    // Allowed tag — keep as-is
                    result.push_str(&text[i..tag_end + 1]);
                    let tag_name = tag_name_raw.to_ascii_lowercase();
                    // Track open/close for balancing
                    if is_closing {
                        if let Some(pos) = open_tags.iter().rposition(|t| t == &tag_name) {
                            open_tags.remove(pos);
                        }
                    } else if !tag_content.ends_with('/') {
                        // Not self-closing
                        open_tags.push(tag_name);
                    }
                } else {
                    // Unknown tag — escape both brackets
                    result.push_str("&lt;");
                    result.push_str(tag_content);
                    result.push_str("&gt;");
                }
                // Advance past the whole tag
                while let Some(&(j, _)) = chars.peek() {
                    chars.next();
                    if j >= tag_end {
                        break;
                    }
                }
            } else {
                // No closing > — escape the lone <
                result.push_str("&lt;");
                chars.next();
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    // Close any unclosed tags (prevents Telegram "can't parse entities" errors)
    for tag in open_tags.into_iter().rev() {
        result.push_str("</");
        result.push_str(&tag);
        result.push('>');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> reqwest::Client {
        crate::http_client::new_client()
    }

    /// Helper to create a TelegramApiCtx for tests.
    fn test_ctx<'a>(client: &'a reqwest::Client) -> TelegramApiCtx<'a> {
        TelegramApiCtx {
            token: "fake:token",
            client,
            api_base_url: DEFAULT_API_URL,
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_update() {
        let update = serde_json::json!({
            "update_id": 123456,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "text": "Hello, agent!"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert_eq!(msg.sender.platform_id, "111222333");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello, agent!"));
    }

    #[tokio::test]
    async fn test_parse_telegram_command() {
        let update = serde_json::json!({
            "update_id": 123457,
            "message": {
                "message_id": 43,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000001,
                "text": "/agent hello-world",
                "entities": [{
                    "type": "bot_command",
                    "offset": 0,
                    "length": 6
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_allowed_users_filter() {
        let update = serde_json::json!({
            "update_id": 123458,
            "message": {
                "message_id": 44,
                "from": {
                    "id": 999,
                    "first_name": "Bob"
                },
                "chat": {
                    "id": 999,
                    "type": "private"
                },
                "date": 1700000002,
                "text": "blocked"
            }
        });

        let client = test_client();

        // Empty allowed_users = allow all
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None).await;
        assert!(msg.is_ok());

        // Non-matching allowed_users = filter out
        let blocked: Vec<String> = vec!["111".to_string(), "222".to_string()];
        let msg = parse_telegram_update(&update, &blocked, &test_ctx(&client), None).await;
        assert!(msg.is_err());

        // Matching allowed_users = allow
        let allowed: Vec<String> = vec!["999".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_ok());
    }

    #[tokio::test]
    async fn test_allowed_users_filter_username() {
        let update = serde_json::json!({
            "update_id": 123459,
            "message": {
                "message_id": 45,
                "from": {
                    "id": 999,
                    "first_name": "Bob",
                    "username": "bobuser"
                },
                "chat": {
                    "id": 999,
                    "type": "private"
                },
                "date": 1700000003,
                "text": "hello"
            }
        });

        let client = test_client();

        // Username match (no @)
        let allowed = vec!["bobuser".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_ok(), "username without @ should match");

        // Username match (with @)
        let allowed = vec!["@bobuser".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_ok(), "username with @ should match");

        // Case-insensitive username match
        let allowed = vec!["BoBuSeR".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_ok(), "username match should be case-insensitive");

        // ID mismatch but username match
        let allowed = vec!["111".to_string(), "bobuser".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(
            msg.is_ok(),
            "should match by username when ID doesn't match"
        );

        // Wrong username, wrong ID → reject
        let allowed = vec!["otheruser".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_err(), "wrong username should be rejected");
    }

    #[tokio::test]
    async fn test_parse_telegram_edited_message() {
        let update = serde_json::json!({
            "update_id": 123459,
            "edited_message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "edit_date": 1700000060,
                "text": "Edited message!"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message!"));
    }

    #[test]
    fn test_backoff_calculation() {
        let max = Duration::from_secs(60);
        let b1 = calculate_backoff(Duration::from_secs(1), max);
        assert_eq!(b1, Duration::from_secs(2));

        let b2 = calculate_backoff(Duration::from_secs(2), max);
        assert_eq!(b2, Duration::from_secs(4));

        let b3 = calculate_backoff(Duration::from_secs(32), max);
        assert_eq!(b3, Duration::from_secs(60)); // capped

        let b4 = calculate_backoff(Duration::from_secs(60), max);
        assert_eq!(b4, Duration::from_secs(60)); // stays at cap
    }

    #[tokio::test]
    async fn test_parse_command_with_botname() {
        let update = serde_json::json!({
            "update_id": 100,
            "message": {
                "message_id": 1,
                "from": { "id": 123, "first_name": "X" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "/agents@mylibrefangbot",
                "entities": [{ "type": "bot_command", "offset": 0, "length": 17 }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agents");
                assert!(args.is_empty());
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_location() {
        let update = serde_json::json!({
            "update_id": 200,
            "message": {
                "message_id": 50,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "location": { "latitude": 51.5074, "longitude": -0.1278 }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert!(matches!(msg.content, ChannelContent::Location { .. }));
    }

    #[tokio::test]
    async fn test_parse_telegram_photo_fallback() {
        // When getFile fails (fake token), photo messages should fall back to
        // a text description rather than being silently dropped.
        let update = serde_json::json!({
            "update_id": 300,
            "message": {
                "message_id": 60,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "photo": [
                    { "file_id": "small_id", "file_unique_id": "a", "width": 90, "height": 90, "file_size": 1234 },
                    { "file_id": "large_id", "file_unique_id": "b", "width": 800, "height": 600, "file_size": 45678 }
                ],
                "caption": "Check this out"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        // With a fake token, getFile will fail, so we get a text fallback
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Photo received"));
                assert!(t.contains("Check this out"));
            }
            ChannelContent::Image { caption, .. } => {
                // If somehow the HTTP call succeeded (unlikely with fake token),
                // verify caption was extracted
                assert_eq!(caption.as_deref(), Some("Check this out"));
            }
            other => panic!("Expected Text or Image fallback for photo, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_document_fallback() {
        let update = serde_json::json!({
            "update_id": 301,
            "message": {
                "message_id": 61,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "document": {
                    "file_id": "doc_id",
                    "file_unique_id": "c",
                    "file_name": "report.pdf",
                    "file_size": 102400
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Document received"));
                assert!(t.contains("report.pdf"));
            }
            ChannelContent::File { filename, .. } => {
                assert_eq!(filename, "report.pdf");
            }
            other => panic!("Expected Text or File for document, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_voice_fallback() {
        let update = serde_json::json!({
            "update_id": 302,
            "message": {
                "message_id": 62,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "voice": {
                    "file_id": "voice_id",
                    "file_unique_id": "d",
                    "duration": 15
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Voice message"));
                assert!(t.contains("15s"));
            }
            ChannelContent::Voice {
                duration_seconds, ..
            } => {
                assert_eq!(*duration_seconds, 15);
            }
            other => panic!("Expected Text or Voice for voice message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_audio_with_caption() {
        let update = serde_json::json!({
            "update_id": 303,
            "message": {
                "message_id": 63,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "audio": {
                    "file_id": "audio_id",
                    "file_unique_id": "e",
                    "duration": 120,
                    "title": "recording.mp3"
                },
                "caption": "riassumi"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                // Fallback when file URL can't be resolved
                assert!(t.contains("Audio received"));
                assert!(t.contains("riassumi"));
            }
            ChannelContent::Voice {
                caption,
                duration_seconds,
                ..
            } => {
                assert_eq!(*duration_seconds, 120);
                assert_eq!(caption.as_deref(), Some("riassumi"));
            }
            other => panic!("Expected Text or Voice for audio message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_forum_topic_thread_id() {
        // Messages inside a Telegram forum topic include `message_thread_id`.
        let update = serde_json::json!({
            "update_id": 400,
            "message": {
                "message_id": 70,
                "message_thread_id": 42,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hello from a forum topic"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.thread_id, Some("42".to_string()));
        assert!(msg.is_group);
    }

    #[tokio::test]
    async fn test_parse_telegram_no_thread_id_in_private_chat() {
        // Private chats should have thread_id = None.
        let update = serde_json::json!({
            "update_id": 401,
            "message": {
                "message_id": 71,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Hello from DM"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.thread_id, None);
        assert!(!msg.is_group);
    }

    #[tokio::test]
    async fn test_parse_telegram_edited_message_in_forum() {
        // Edited messages in forum topics should also preserve thread_id.
        let update = serde_json::json!({
            "update_id": 402,
            "edited_message": {
                "message_id": 72,
                "message_thread_id": 99,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "edit_date": 1700000060,
                "text": "Edited in forum"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.thread_id, Some("99".to_string()));
    }

    #[tokio::test]
    async fn test_parse_sender_chat_fallback() {
        // Messages sent on behalf of a channel have `sender_chat` instead of `from`.
        let update = serde_json::json!({
            "update_id": 500,
            "message": {
                "message_id": 80,
                "sender_chat": {
                    "id": -1001999888777_i64,
                    "title": "My Channel",
                    "type": "channel"
                },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Forwarded from channel"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(msg.sender.display_name, "My Channel");
        assert_eq!(msg.sender.platform_id, "-1001234567890");
        assert!(
            matches!(msg.content, ChannelContent::Text(ref t) if t == "Forwarded from channel")
        );
    }

    #[tokio::test]
    async fn test_sender_chat_allowed_users_id_only() {
        // sender_chat path should only match by numeric ID, not by channel name/username.
        let update = serde_json::json!({
            "update_id": 501,
            "message": {
                "message_id": 81,
                "sender_chat": {
                    "id": -1001999888777_i64,
                    "title": "My Channel",
                    "type": "channel"
                },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Channel post"
            }
        });

        let client = test_client();

        // Allowed by sender_chat ID (as string)
        let allowed = vec!["-1001999888777".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(msg.is_ok(), "sender_chat should be allowed by numeric ID");

        // NOT allowed by channel title alone — sender_chat has no username field
        let allowed = vec!["My Channel".to_string()];
        let msg = parse_telegram_update(&update, &allowed, &test_ctx(&client), None).await;
        assert!(
            msg.is_err(),
            "sender_chat should NOT match by channel title"
        );
    }

    #[tokio::test]
    async fn test_parse_no_from_no_sender_chat_drops() {
        // Updates with neither `from` nor `sender_chat` should be dropped with warn logging.
        let update = serde_json::json!({
            "update_id": 501,
            "message": {
                "message_id": 81,
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "orphan"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None).await;
        assert!(msg.is_err());
    }

    #[tokio::test]
    async fn test_was_mentioned_in_group() {
        // Bot @mentioned in a group message should set metadata["was_mentioned"].
        let update = serde_json::json!({
            "update_id": 600,
            "message": {
                "message_id": 90,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @testbot what do you think?",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_not_mentioned_in_group() {
        // Group message without a mention should NOT have was_mentioned.
        let update = serde_json::json!({
            "update_id": 601,
            "message": {
                "message_id": 91,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Just chatting"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert!(msg.is_group);
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_mentioned_different_bot_not_set() {
        // @mention of a different bot should NOT set was_mentioned.
        let update = serde_json::json!({
            "update_id": 602,
            "message": {
                "message_id": 92,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @otherbot what do you think?",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 9
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert!(msg.is_group);
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_mention_in_caption_entities() {
        // Bot mentioned in a photo caption should set was_mentioned.
        let update = serde_json::json!({
            "update_id": 603,
            "message": {
                "message_id": 93,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "photo": [
                    { "file_id": "photo_id", "file_unique_id": "x", "width": 800, "height": 600 }
                ],
                "caption": "Look @testbot",
                "caption_entities": [{
                    "type": "mention",
                    "offset": 5,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_mention_case_insensitive() {
        // Mention detection should be case-insensitive.
        let update = serde_json::json!({
            "update_id": 604,
            "message": {
                "message_id": 94,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "Hey @TestBot help",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_private_chat_no_mention_check() {
        // Private chats should NOT populate was_mentioned even with entities.
        let update = serde_json::json!({
            "update_id": 605,
            "message": {
                "message_id": 95,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "Hey @testbot",
                "entities": [{
                    "type": "mention",
                    "offset": 4,
                    "length": 8
                }]
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), Some("testbot"))
            .await
            .unwrap();
        assert!(!msg.is_group);
        // In private chats, mention detection is skipped — no metadata set
        assert!(!msg.metadata.contains_key("was_mentioned"));
    }

    #[test]
    fn test_check_mention_entities_direct() {
        let message = serde_json::json!({
            "text": "Hello @mybot world",
            "entities": [{
                "type": "mention",
                "offset": 6,
                "length": 6
            }]
        });
        assert!(check_mention_entities(&message, "mybot"));
        assert!(!check_mention_entities(&message, "otherbot"));
    }

    #[tokio::test]
    async fn test_parse_telegram_reply_to_message() {
        // When a user replies to a specific message, the quoted context should be prepended.
        let update = serde_json::json!({
            "update_id": 700,
            "message": {
                "message_id": 100,
                "from": { "id": 123, "first_name": "Bob" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "I disagree with that",
                "reply_to_message": {
                    "message_id": 99,
                    "from": { "id": 456, "first_name": "Alice" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999900,
                    "text": "The sky is green"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Alice:"), "got: {t}");
                assert!(t.contains("The sky is green"));
                assert!(t.contains("I disagree with that"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_reply_to_message_no_text() {
        // reply_to_message without text/caption should not modify the content.
        let update = serde_json::json!({
            "update_id": 701,
            "message": {
                "message_id": 101,
                "from": { "id": 123, "first_name": "Bob" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "What was that sticker?",
                "reply_to_message": {
                    "message_id": 100,
                    "from": { "id": 456, "first_name": "Alice" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999900,
                    "sticker": { "file_id": "abc123" }
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert!(
            matches!(msg.content, ChannelContent::Text(ref t) if t == "What was that sticker?")
        );
    }

    #[tokio::test]
    async fn test_parse_telegram_reply_truncates_long_text() {
        let long_text = "a".repeat(300);
        let update = serde_json::json!({
            "update_id": 702,
            "message": {
                "message_id": 102,
                "from": { "id": 123, "first_name": "Bob" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "reply",
                "reply_to_message": {
                    "message_id": 99,
                    "from": { "id": 456, "first_name": "Alice" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999900,
                    "text": long_text
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                // Quoted text should be truncated, not the full 300 chars
                assert!(t.contains("..."), "long quote should be truncated with ...");
                assert!(
                    !t.contains(&"a".repeat(300)),
                    "full 300-char text should not appear"
                );
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_reply_stores_metadata() {
        let update = serde_json::json!({
            "update_id": 703,
            "message": {
                "message_id": 103,
                "from": { "id": 123, "first_name": "Bob" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "I agree",
                "reply_to_message": {
                    "message_id": 50,
                    "from": { "id": 456, "first_name": "Alice" },
                    "chat": { "id": 123, "type": "private" },
                    "date": 1699999900,
                    "text": "Let's meet tomorrow"
                }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        let reply_to = msg
            .metadata
            .get("reply_to")
            .expect("reply_to metadata should exist");
        assert_eq!(reply_to["message_id"], 50);
        assert_eq!(reply_to["sender"], "Alice");
        assert_eq!(reply_to["text"], "Let's meet tomorrow");
    }

    #[test]
    fn test_sanitize_telegram_html_basic() {
        // Allowed tags preserved, unknown tags escaped
        let input = "<b>bold</b> <thinking>hmm</thinking>";
        let output = sanitize_telegram_html(input);
        assert!(output.contains("<b>bold</b>"));
        assert!(output.contains("&lt;thinking&gt;"));
    }

    #[test]
    fn test_sanitize_telegram_html_unclosed_tags() {
        // Unclosed tags should be auto-closed at the end
        let input = "<b>bold text";
        let output = sanitize_telegram_html(input);
        assert!(
            output.contains("<b>bold text"),
            "content should be preserved"
        );
        assert!(
            output.ends_with("</b>"),
            "unclosed <b> should be auto-closed"
        );
    }

    #[test]
    fn test_sanitize_telegram_html_nested_tags() {
        // Nested allowed tags should work correctly
        let input = "<pre><code>fn main() {}</code></pre>";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, input, "nested pre+code should be preserved as-is");
    }

    #[test]
    fn test_sanitize_telegram_html_link_with_attributes() {
        // <a> tags with href attribute should be preserved
        let input = r#"<a href="https://example.com">link</a>"#;
        let output = sanitize_telegram_html(input);
        assert!(
            output.contains(r#"href="https://example.com""#),
            "href attribute should be preserved"
        );
        assert!(
            output.contains(">link</a>"),
            "link text should be preserved"
        );
    }

    #[test]
    fn test_sanitize_telegram_html_self_closing_tags() {
        // Tags ending with /> should not be tracked as open
        let input = "before <br/> after";
        let output = sanitize_telegram_html(input);
        // <br/> is not in ALLOWED, so it gets escaped
        assert!(output.contains("before"), "text before should remain");
        assert!(output.contains("after"), "text after should remain");
    }

    #[test]
    fn test_sanitize_telegram_html_empty_angle_brackets() {
        // Lone <> should be escaped
        let input = "text <> more";
        let output = sanitize_telegram_html(input);
        assert!(output.contains("&lt;"), "empty <> should be escaped");
    }

    #[test]
    fn test_sanitize_telegram_html_lone_open_bracket() {
        // Lone < without closing > should be escaped
        let input = "text < more";
        let output = sanitize_telegram_html(input);
        assert!(output.contains("&lt;"), "lone < should be escaped");
        assert!(output.contains(" more"), "rest of text should be preserved");
    }

    #[test]
    fn test_sanitize_telegram_html_unicode() {
        // Unicode/emoji in text should not be corrupted
        let input = "<b>Привет 🌍 мир</b> <unknown>test</unknown>";
        let output = sanitize_telegram_html(input);
        assert!(
            output.contains("<b>Привет 🌍 мир</b>"),
            "unicode in allowed tags should be preserved"
        );
        assert!(
            output.contains("&lt;unknown&gt;"),
            "unknown tags should be escaped"
        );
    }

    #[test]
    fn test_sanitize_telegram_html_idempotent() {
        // Sanitizing twice should produce the same output as sanitizing once
        let input = "<b>bold</b> <thinking>hmm</thinking> <code>inline</code> <foo>bar</foo>";
        let first = sanitize_telegram_html(input);
        let second = sanitize_telegram_html(&first);
        assert_eq!(first, second, "sanitize_telegram_html should be idempotent");
    }

    #[test]
    fn test_sanitize_telegram_html_all_allowed_tags() {
        // Every tag in the ALLOWED list should pass through
        let tags = [
            "b",
            "i",
            "u",
            "s",
            "em",
            "strong",
            "code",
            "pre",
            "blockquote",
        ];
        for tag in tags {
            let input = format!("<{tag}>text</{tag}>");
            let output = sanitize_telegram_html(&input);
            assert_eq!(output, input, "allowed tag <{tag}> should pass through");
        }
    }

    #[test]
    fn test_sanitize_telegram_html_multiple_unknown_tags() {
        // Multiple unknown tags should all be escaped
        let input = "<name>John</name> <age>25</age>";
        let output = sanitize_telegram_html(input);
        assert!(output.contains("&lt;name&gt;"), "<name> should be escaped");
        assert!(
            output.contains("&lt;/name&gt;"),
            "</name> should be escaped"
        );
        assert!(output.contains("&lt;age&gt;"), "<age> should be escaped");
        assert!(output.contains("John"), "inner text should be preserved");
        assert!(output.contains("25"), "inner text should be preserved");
    }

    #[test]
    fn test_supports_streaming() {
        let adapter = TelegramAdapter::new(
            "fake:token".to_string(),
            vec![],
            Duration::from_secs(1),
            None,
        );
        assert!(
            adapter.supports_streaming(),
            "TelegramAdapter must report streaming support"
        );
    }

    #[test]
    fn test_streaming_edit_interval_is_sane() {
        // Ensure the edit interval is at least 500ms to avoid rate limiting,
        // and at most 5s to keep the UX responsive.
        assert!(STREAMING_EDIT_INTERVAL >= Duration::from_millis(500));
        assert!(STREAMING_EDIT_INTERVAL <= Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_basic() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_12345",
            "from": {
                "id": 42,
                "first_name": "Alice",
                "last_name": "Smith"
            },
            "data": "approve_req_001",
            "message": {
                "message_id": 999,
                "chat": { "id": -100123, "type": "supergroup" },
                "text": "Approve this request?",
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client)).unwrap();

        assert_eq!(msg.channel, ChannelType::Telegram);
        assert_eq!(msg.sender.platform_id, "-100123");
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert!(msg.is_group);
        match &msg.content {
            ChannelContent::ButtonCallback {
                action,
                message_text,
            } => {
                assert_eq!(action, "approve_req_001");
                assert_eq!(message_text.as_deref(), Some("Approve this request?"));
            }
            other => panic!("Expected ButtonCallback, got {other:?}"),
        }
        assert!(msg.metadata.contains_key("callback_query_id"));
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_filtered_user() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_99",
            "from": { "id": 42, "first_name": "Alice" },
            "data": "some_action",
            "message": {
                "message_id": 1,
                "chat": { "id": 100, "type": "private" },
                "text": "msg",
                "date": 1700000000
            }
        });

        // User 42 not in allowed list
        let msg =
            parse_telegram_callback_query(&callback, &["999".to_string()], &test_ctx(&client));
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_username_filter() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_100",
            "from": { "id": 42, "first_name": "Alice", "username": "alicebot" },
            "data": "approve",
            "message": {
                "message_id": 2,
                "chat": { "id": 100, "type": "private" },
                "text": "Some prompt",
                "date": 1700000000
            }
        });

        // Username match (no @) — should allow
        let msg =
            parse_telegram_callback_query(&callback, &["alicebot".to_string()], &test_ctx(&client));
        assert!(msg.is_some(), "callback: username without @ should match");

        // Username match (with @) — should allow
        let msg = parse_telegram_callback_query(
            &callback,
            &["@alicebot".to_string()],
            &test_ctx(&client),
        );
        assert!(msg.is_some(), "callback: username with @ should match");

        // Case-insensitive username — should allow
        let msg =
            parse_telegram_callback_query(&callback, &["AlIcEbOt".to_string()], &test_ctx(&client));
        assert!(
            msg.is_some(),
            "callback: case-insensitive username should match"
        );

        // ID mismatch but username match — should allow
        let msg = parse_telegram_callback_query(
            &callback,
            &["999".to_string(), "alicebot".to_string()],
            &test_ctx(&client),
        );
        assert!(
            msg.is_some(),
            "callback: should match by username when ID doesn't match"
        );

        // Wrong username — should reject
        let msg = parse_telegram_callback_query(
            &callback,
            &["wronguser".to_string()],
            &test_ctx(&client),
        );
        assert!(msg.is_none(), "callback: wrong username should be rejected");
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_empty_data() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_1",
            "from": { "id": 42, "first_name": "Alice" },
            "data": "",
            "message": {
                "message_id": 1,
                "chat": { "id": 100, "type": "private" },
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client));
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_dm() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_dm",
            "from": { "id": 42, "first_name": "Bob" },
            "data": "action_dm",
            "message": {
                "message_id": 5,
                "chat": { "id": 42, "type": "private" },
                "text": "Pick option",
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client)).unwrap();
        assert!(!msg.is_group);
        assert_eq!(msg.sender.display_name, "Bob");
    }
}
