//! Telegram Bot API adapter for the LibreFang channel bridge.
//!
//! Uses long-polling via `getUpdates` with exponential backoff on failures.
//! No external Telegram crate — just `reqwest` for full control over error handling.

use crate::bridge::SENDER_USER_ID_KEY;
use crate::formatter;
use crate::http_client::{fetch_url_bytes_unchecked, safe_fetch_client};
use crate::message_truncator::{split_to_utf16_chunks, TELEGRAM_MESSAGE_LIMIT};
use crate::types::{
    truncate_utf8, ChannelAdapter, ChannelContent, ChannelMessage, ChannelRoleQuery, ChannelType,
    ChannelUser, InteractiveButton, InteractiveMessage, LifecycleReaction, PlatformRole,
};
use async_trait::async_trait;
use dashmap::DashMap;
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

/// Bound startup control-plane calls so a flaky Local Bot API cannot block
/// daemon boot forever.
const STARTUP_API_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Returns `true` when `url_str` points at a host that Telegram's public
/// cloud cannot reach — loopback, RFC1918 private ranges, link-local,
/// `localhost`, or an unparseable URL. Used to short-circuit URL-based
/// Bot API calls and fall back to multipart upload from inside the
/// container, where these addresses are actually routable.
fn is_private_url(url_str: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url_str) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(addr)) => {
            addr.is_loopback() || addr.is_private() || addr.is_link_local()
        }
        Some(url::Host::Ipv6(addr)) => {
            addr.is_loopback()
                || (addr.segments()[0] & 0xfe00) == 0xfc00  // fc00::/7 unique-local
                || (addr.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
        None => false,
    }
}

/// Best-effort filename extracted from the last path segment of `url_str`,
/// falling back to a generic name if the URL has no usable path.
fn url_filename(url_str: &str, fallback: &str) -> String {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut segs| segs.next_back().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// A Telegram bot command definition for the command menu.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BotCommand {
    /// Command name without the leading `/` (e.g. `"start"`, `"help"`).
    pub command: String,
    /// Human-readable description shown in the Telegram command menu.
    pub description: String,
}

/// Build a `Vec<BotCommand>` for the Telegram command menu.
///
/// Sourced from [`crate::commands::telegram_bot_commands`] — the central
/// command registry — so this list never drifts from the bridge's
/// `handle_command` dispatcher. When `TelegramAdapter::commands` is empty
/// (the default), the adapter registers these automatically on `/start`
/// or the first incoming message. Telegram allows at most 100 commands.
fn builtin_bot_commands() -> Vec<BotCommand> {
    crate::commands::telegram_bot_commands()
        .into_iter()
        .map(|(command, description)| BotCommand {
            command,
            description,
        })
        .collect()
}

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
        let resp = match self
            .client
            .post(&url)
            .json(&serde_json::json!({"file_id": file_id}))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!("Telegram getFile request failed for {file_id}: {e}");
                return None;
            }
        };
        let body: serde_json::Value = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                debug!("Telegram getFile parse failed for {file_id}: {e}");
                return None;
            }
        };
        if body["ok"].as_bool() != Some(true) {
            debug!("Telegram getFile returned ok=false for {file_id}: {body}");
            return None;
        }
        let file_path = body["result"]["file_path"].as_str()?;
        Some(format!(
            "{}/file/bot{}/{}",
            self.api_base_url, self.token, file_path
        ))
    }

    /// Register bot commands via `setMyCommands`.
    ///
    /// Used inside the polling loop to (re-)register commands on `/start`
    /// or the first incoming message. Idempotent — safe to call repeatedly.
    async fn set_my_commands(&self, commands: &[BotCommand]) {
        let url = format!("{}/bot{}/setMyCommands", self.api_base_url, self.token);
        let cmds: Vec<serde_json::Value> = commands
            .iter()
            .map(|c| serde_json::json!({"command": c.command, "description": c.description}))
            .collect();
        let body = serde_json::json!({ "commands": cmds });
        match self.client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(
                    "Telegram: registered {} command(s) via polling trigger",
                    commands.len()
                );
            }
            Ok(resp) => {
                let text = resp.text().await.unwrap_or_default();
                warn!("Telegram setMyCommands failed in polling loop: {text}");
            }
            Err(e) => {
                warn!("Telegram setMyCommands request failed in polling loop: {e}");
            }
        }
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
    /// Bot commands registered in the Telegram command menu.
    commands: Vec<BotCommand>,
    poll_contexts: Arc<DashMap<String, PollContext>>,
    /// Outbound media upload size cap (bytes) for the private-URL
    /// multipart-fallback path. Defaults to
    /// `DEFAULT_TELEGRAM_UPLOAD_BYTES` (50 MiB — Telegram bot API
    /// ceiling) — operators override via
    /// `[channels].file_upload_max_bytes` in `config.toml`, plumbed in
    /// by the bridge through `with_max_upload_bytes`.
    max_upload_bytes: usize,
}

/// Default outbound media upload cap for Telegram (50 MiB, the bot
/// API ceiling). Per-instance override via
/// `TelegramAdapter::with_max_upload_bytes`.
const DEFAULT_TELEGRAM_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

struct PollContext {
    question: String,
    options: Vec<String>,
    last_accessed: Instant,
}

/// Parameters for `api_send_poll` — grouped to satisfy clippy's argument limit.
struct PollParams<'a> {
    chat_id: i64,
    question: &'a str,
    options: &'a [String],
    is_quiz: bool,
    correct_option_id: Option<u8>,
    explanation: Option<&'a str>,
    thread_id: Option<i64>,
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
            commands: Vec::new(),
            poll_contexts: Arc::new(DashMap::new()),
            max_upload_bytes: DEFAULT_TELEGRAM_UPLOAD_BYTES,
        }
    }

    /// When enabled, the Done reaction is removed (cleared) instead of
    /// showing a completion emoji.  Returns self for builder chaining.
    pub fn with_clear_done_reaction(mut self, clear: bool) -> Self {
        self.clear_done_reaction = clear;
        self
    }

    /// Set the bot commands to register in the Telegram command menu on start.
    /// Pass an empty vec to clear existing commands. Returns self for builder chaining.
    pub fn with_commands(mut self, commands: Vec<BotCommand>) -> Self {
        self.commands = commands;
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

    /// Override the outbound media upload size cap. Returns self for
    /// builder chaining. Bridge plumbs the operator's
    /// `[channels].file_upload_max_bytes` here so a single config knob
    /// applies to every Telegram bot instance.
    pub fn with_max_upload_bytes(mut self, max_upload_bytes: usize) -> Self {
        self.max_upload_bytes = max_upload_bytes;
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

        // Telegram's 4096-character limit is measured in UTF-16 code units.
        // Emoji and CJK Extension B characters consume 2 units each, so we
        // must use `split_to_utf16_chunks` rather than a plain byte/codepoint
        // split to avoid 400 errors on messages heavy with such characters.
        let chunks = split_to_utf16_chunks(&sanitized, TELEGRAM_MESSAGE_LIMIT);
        let total = chunks.len();
        for (idx, chunk) in chunks.iter().enumerate() {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": PARSE_MODE_HTML,
            });
            if let Some(tid) = thread_id {
                body["message_thread_id"] = serde_json::json!(tid);
            }

            // Issue #3664: when a multi-chunk send fails part-way through
            // we must surface the error to the caller — silently dropping
            // chunks 2..N gives the user a truncated message with no signal
            // that anything went wrong. We log the chunk index so operators
            // can correlate the failure with the partial delivery.
            let resp = self.client.post(&url).json(&body).send().await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    warn!(
                        "Telegram sendMessage chunk {}/{total} network error: {e}",
                        idx + 1
                    );
                    Box::new(e)
                },
            )?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                warn!(
                    "Telegram sendMessage chunk {}/{total} failed ({status}): {body_text}",
                    idx + 1
                );
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
                    let retry_status = retry.status();
                    if !retry_status.is_success() {
                        let retry_text = retry.text().await.unwrap_or_default();
                        warn!(
                            "Telegram sendMessage chunk {}/{total} plain fallback also failed ({retry_status}): {retry_text}",
                            idx + 1
                        );
                        return Err(format!(
                            "Telegram sendMessage failed on chunk {}/{total}: HTTP {retry_status}",
                            idx + 1
                        )
                        .into());
                    }
                } else {
                    // Non-recoverable failure: stop sending further chunks
                    // and propagate so the caller can decide whether to
                    // retry the full message or report failure upstream.
                    return Err(format!(
                        "Telegram sendMessage failed on chunk {}/{total}: HTTP {status}",
                        idx + 1
                    )
                    .into());
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
                    return Err(
                        format!("Telegram {endpoint} failed after retry: {body_text2}").into(),
                    );
                }
                return Ok(());
            }

            return Err(format!("Telegram {endpoint} failed ({status}): {body_text}").into());
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
        // Private-network URLs can't be fetched by Telegram's cloud. We
        // download them from inside the container (where the address is
        // routable) and re-upload via multipart, so the agent doesn't need
        // a public tunnel just to deliver a locally-served file.
        if is_private_url(document_url) {
            info!(
                url = document_url,
                "Private URL detected on sendDocument, falling back to multipart upload"
            );
            let (bytes, ct) = fetch_url_bytes_unchecked(
                safe_fetch_client(),
                document_url,
                self.max_upload_bytes,
                &[],
            )
            .await?;
            let mime = ct.unwrap_or_else(|| "application/octet-stream".to_string());
            return self
                .api_send_media_upload(
                    "sendDocument",
                    "document",
                    chat_id,
                    bytes,
                    filename,
                    &mime,
                    Some(&[("caption", filename.to_string())]),
                    thread_id,
                )
                .await;
        }
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
        self.api_send_media_upload(
            "sendDocument",
            "document",
            chat_id,
            data,
            filename,
            mime_type,
            None,
            thread_id,
        )
        .await
    }

    /// Generic multipart upload for any Telegram `send{Media}` endpoint.
    ///
    /// `field_name` is the form field the API expects for the payload
    /// (`document`, `voice`, `audio`, `photo`, `video`, …). `extra` merges
    /// optional text fields (captions, title, performer) into the form.
    /// Retries once on HTTP 429 like the URL path.
    #[allow(clippy::too_many_arguments)]
    async fn api_send_media_upload(
        &self,
        endpoint: &'static str,
        field_name: &'static str,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        extra: Option<&[(&str, String)]>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/{endpoint}",
            self.api_base_url,
            self.token.as_str()
        );

        let data_bytes = bytes::Bytes::from(data);

        let build_form =
            || -> Result<reqwest::multipart::Form, Box<dyn std::error::Error + Send + Sync>> {
                let part = reqwest::multipart::Part::stream(data_bytes.clone())
                    .file_name(filename.to_string())
                    .mime_str(mime_type)?;
                let mut form = reqwest::multipart::Form::new()
                    .text("chat_id", chat_id.to_string())
                    .part(field_name, part);
                if let Some(tid) = thread_id {
                    form = form.text("message_thread_id", tid.to_string());
                }
                if let Some(kv) = extra {
                    for (k, v) in kv {
                        form = form.text(k.to_string(), v.clone());
                    }
                }
                Ok(form)
            };

        let resp = self
            .client
            .post(&url)
            .multipart(build_form()?)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                warn!("Telegram {endpoint} upload rate limited, retrying after {retry_after}s");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;

                let resp2 = self
                    .client
                    .post(&url)
                    .multipart(build_form()?)
                    .send()
                    .await?;
                if !resp2.status().is_success() {
                    let body_text2 = resp2.text().await.unwrap_or_default();
                    return Err(format!(
                        "Telegram {endpoint} upload failed after retry: {body_text2}"
                    )
                    .into());
                }
                return Ok(());
            }

            return Err(
                format!("Telegram {endpoint} upload failed ({status}): {body_text}").into(),
            );
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
        if is_private_url(voice_url) {
            info!(
                url = voice_url,
                "Private URL detected on sendVoice, falling back to multipart upload"
            );
            let (bytes, ct) = fetch_url_bytes_unchecked(
                safe_fetch_client(),
                voice_url,
                self.max_upload_bytes,
                &[],
            )
            .await?;
            let mime = ct.unwrap_or_else(|| "audio/ogg".to_string());
            let filename = url_filename(voice_url, "voice.ogg");
            let mut extra: Vec<(&str, String)> = Vec::new();
            if let Some(cap) = caption {
                extra.push(("caption", cap.to_string()));
                extra.push(("parse_mode", PARSE_MODE_HTML.to_string()));
            }
            return self
                .api_send_media_upload(
                    "sendVoice",
                    "voice",
                    chat_id,
                    bytes,
                    &filename,
                    &mime,
                    Some(&extra),
                    thread_id,
                )
                .await;
        }
        let mut body = serde_json::json!({ "voice": voice_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        self.api_send_media_request("sendVoice", chat_id, body, thread_id)
            .await
    }

    /// Call `sendAudio` on the Telegram API.
    ///
    /// Sends a music file (MP3, FLAC, etc.) with optional title and performer metadata.
    /// Distinct from `sendVoice` which is for voice memos.
    async fn api_send_audio(
        &self,
        chat_id: i64,
        audio_url: &str,
        caption: Option<&str>,
        title: Option<&str>,
        performer: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if is_private_url(audio_url) {
            info!(
                url = audio_url,
                "Private URL detected on sendAudio, falling back to multipart upload"
            );
            let (bytes, ct) = fetch_url_bytes_unchecked(
                safe_fetch_client(),
                audio_url,
                self.max_upload_bytes,
                &[],
            )
            .await?;
            let mime = ct.unwrap_or_else(|| "audio/mpeg".to_string());
            let filename = url_filename(audio_url, "audio.mp3");
            let mut extra: Vec<(&str, String)> = Vec::new();
            if let Some(cap) = caption {
                extra.push(("caption", cap.to_string()));
                extra.push(("parse_mode", PARSE_MODE_HTML.to_string()));
            }
            if let Some(t) = title {
                extra.push(("title", t.to_string()));
            }
            if let Some(p) = performer {
                extra.push(("performer", p.to_string()));
            }
            return self
                .api_send_media_upload(
                    "sendAudio",
                    "audio",
                    chat_id,
                    bytes,
                    &filename,
                    &mime,
                    Some(&extra),
                    thread_id,
                )
                .await;
        }
        let mut body = serde_json::json!({ "audio": audio_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        if let Some(t) = title {
            body["title"] = serde_json::Value::String(t.to_string());
        }
        if let Some(p) = performer {
            body["performer"] = serde_json::Value::String(p.to_string());
        }
        self.api_send_media_request("sendAudio", chat_id, body, thread_id)
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

    /// Call `sendAnimation` on the Telegram API.
    ///
    /// Sends an animated GIF or H.264/MPEG-4 AVC video without sound.
    async fn api_send_animation(
        &self,
        chat_id: i64,
        animation_url: &str,
        caption: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut body = serde_json::json!({ "animation": animation_url });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
        }
        self.api_send_media_request("sendAnimation", chat_id, body, thread_id)
            .await
    }

    /// Call `sendSticker` on the Telegram API.
    ///
    /// Sends a sticker by its Telegram `file_id`. Stickers are identified by
    /// file_id (not download URL) — they cannot be sent from an external URL.
    async fn api_send_sticker(
        &self,
        chat_id: i64,
        file_id: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({ "sticker": file_id });
        self.api_send_media_request("sendSticker", chat_id, body, thread_id)
            .await
    }

    /// Call `sendMediaGroup` on the Telegram API.
    ///
    /// Sends 2–10 media items as a single album. Only `Photo` and `Video`
    /// variants of `MediaGroupItem` are supported by the Telegram API for albums.
    ///
    /// The caption of the first item is used as the album caption.
    /// Includes a single retry on HTTP 429 rate-limit responses.
    async fn api_send_media_group(
        &self,
        chat_id: i64,
        items: &[crate::types::MediaGroupItem],
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Telegram sendMediaGroup requires 2–10 items. Validate locally so
        // callers get a readable error instead of an HTTP 400 from the API.
        if items.is_empty() {
            return Ok(());
        }
        if !(2..=10).contains(&items.len()) {
            return Err(format!(
                "Telegram sendMediaGroup requires 2–10 items, got {}",
                items.len()
            )
            .into());
        }
        let url = format!(
            "{}/bot{}/sendMediaGroup",
            self.api_base_url,
            self.token.as_str()
        );

        let media: Vec<serde_json::Value> = items
            .iter()
            .map(|item| match item {
                crate::types::MediaGroupItem::Photo { url, caption } => {
                    let mut v = serde_json::json!({
                        "type": "photo",
                        "media": url,
                    });
                    // Apply caption if present
                    if let Some(cap) = caption {
                        v["caption"] = serde_json::Value::String(cap.clone());
                        v["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
                    }
                    v
                }
                crate::types::MediaGroupItem::Video {
                    url,
                    caption,
                    duration_seconds,
                } => {
                    let mut v = serde_json::json!({
                        "type": "video",
                        "media": url,
                        "duration": duration_seconds,
                    });
                    // Apply caption if present
                    if let Some(cap) = caption {
                        v["caption"] = serde_json::Value::String(cap.clone());
                        v["parse_mode"] = serde_json::Value::String(PARSE_MODE_HTML.to_string());
                    }
                    v
                }
            })
            .collect();

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "media": media,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                warn!("Telegram sendMediaGroup rate limited, retrying after {retry_after}s");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;

                let resp2 = self.client.post(&url).json(&body).send().await?;
                if !resp2.status().is_success() {
                    let body_text2 = resp2.text().await.unwrap_or_default();
                    return Err(format!(
                        "Telegram sendMediaGroup failed after retry: {body_text2}"
                    )
                    .into());
                }
                return Ok(());
            }

            return Err(format!("Telegram sendMediaGroup failed ({status}): {body_text}").into());
        }
        Ok(())
    }

    /// Call `sendPoll` on the Telegram API.
    ///
    /// Sends a regular poll or a quiz (one correct answer). For quiz mode,
    /// set `is_quiz: true` and provide `correct_option_id`.
    ///
    /// Includes a single retry on HTTP 429 rate-limit responses.
    async fn api_send_poll(
        &self,
        params: &PollParams<'_>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/bot{}/sendPoll", self.api_base_url, self.token.as_str());
        let option_values: Vec<serde_json::Value> = params
            .options
            .iter()
            .map(|o| serde_json::json!({"text": o}))
            .collect();

        let mut body = serde_json::json!({
            "chat_id": params.chat_id,
            "question": params.question,
            "options": option_values,
            "type": if params.is_quiz { "quiz" } else { "regular" },
        });
        if params.is_quiz {
            if let Some(id) = params.correct_option_id {
                body["correct_option_id"] = serde_json::json!(id);
            }
            if let Some(exp) = params.explanation {
                body["explanation"] = serde_json::Value::String(exp.to_string());
            }
        }
        if let Some(tid) = params.thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let retry_after = extract_retry_after(&body_text, RETRY_AFTER_DEFAULT_SECS);
                warn!("Telegram sendPoll rate limited, retrying after {retry_after}s");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;

                let resp2 = self.client.post(&url).json(&body).send().await?;
                if !resp2.status().is_success() {
                    let body_text2 = resp2.text().await.unwrap_or_default();
                    return Err(
                        format!("Telegram sendPoll failed after retry: {body_text2}").into(),
                    );
                }
                let resp_body: serde_json::Value = resp2.json().await.unwrap_or_default();
                let poll_id = resp_body["result"]["poll"]["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok(poll_id);
            }

            return Err(format!("Telegram sendPoll failed ({status}): {body_text}").into());
        }
        let resp_body: serde_json::Value = resp.json().await.unwrap_or_default();
        let poll_id = resp_body["result"]["poll"]["id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(poll_id)
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
                            let action = truncate_utf8(&btn.action, 64).to_string();
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

    /// Call `editMessageText` to replace an interactive message with new text and keyboard.
    ///
    /// When `buttons` is empty, sends `inline_keyboard: []` which removes the keyboard.
    /// Silently swallows "message is not modified" errors like `api_edit_message` does.
    async fn api_edit_interactive_message(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        buttons: &[Vec<InteractiveButton>],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/editMessageText",
            self.api_base_url,
            self.token.as_str()
        );
        let sanitized = sanitize_telegram_html(text);
        let keyboard: Vec<Vec<serde_json::Value>> = buttons
            .iter()
            .map(|row| {
                row.iter()
                    .map(|btn| {
                        if let Some(ref u) = btn.url {
                            serde_json::json!({ "text": btn.label, "url": u })
                        } else {
                            let action = truncate_utf8(&btn.action, 64).to_string();
                            serde_json::json!({ "text": btn.label, "callback_data": action })
                        }
                    })
                    .collect()
            })
            .collect();
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": sanitized,
            "parse_mode": PARSE_MODE_HTML,
            "reply_markup": { "inline_keyboard": keyboard },
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if body_text.contains("message is not modified") {
                // no-op
            } else if status == reqwest::StatusCode::BAD_REQUEST
                && body_text.contains("can't parse entities")
            {
                warn!("Telegram editMessageText (interactive) HTML rejected, retrying as plain text: {body_text}");
                let plain_body = serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": text,
                    "reply_markup": { "inline_keyboard": keyboard },
                });
                let retry = self.client.post(&url).json(&plain_body).send().await?;
                if !retry.status().is_success() {
                    let retry_text = retry.text().await.unwrap_or_default();
                    warn!("Telegram editMessageText (interactive) plain fallback also failed: {retry_text}");
                }
            } else {
                warn!("Telegram editMessageText (interactive) failed ({status}): {body_text}");
            }
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

    /// Call `deleteMessage` on the Telegram API.
    ///
    /// Removes a previously sent message. Uses fire-and-forget semantics —
    /// failures are logged at debug level and do not propagate to callers.
    /// Returns Ok(()) in all cases (best-effort deletion).
    async fn api_delete_message(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/deleteMessage",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            // 400 "message to delete not found" is expected when already deleted
            if body_text.contains("message to delete not found")
                || body_text.contains("MESSAGE_ID_INVALID")
            {
                debug!("Telegram deleteMessage: message already gone (chat={chat_id}, msg={message_id})");
            } else {
                warn!("Telegram deleteMessage failed ({status}): {body_text}");
            }
        }
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
            if body_text.contains("message is not modified") {
                // no-op
            } else if status == reqwest::StatusCode::BAD_REQUEST
                && body_text.contains("can't parse entities")
            {
                // HTML tag mismatch — retry without parse_mode so the user sees
                // the response as plain text rather than losing it entirely.
                warn!(
                    "Telegram editMessageText HTML rejected, retrying as plain text: {body_text}"
                );
                let plain_body = serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": text,
                });
                let retry = self.client.post(&url).json(&plain_body).send().await?;
                if !retry.status().is_success() {
                    let retry_text = retry.text().await.unwrap_or_default();
                    warn!("Telegram editMessageText plain fallback also failed: {retry_text}");
                }
            } else {
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

    /// Call `setMyCommands` to register bot commands in the Telegram menu.
    ///
    /// Uses the default scope (all chats) and no language filter.
    /// Called during `start()` with either explicitly configured commands
    /// or the built-in defaults from [`builtin_bot_commands`]. Also
    /// re-triggered from the polling loop on `/start` or first incoming
    /// message (via [`TelegramApiCtx::set_my_commands`]).
    pub async fn api_set_my_commands(
        &self,
        commands: &[BotCommand],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/setMyCommands",
            self.api_base_url,
            self.token.as_str()
        );
        let cmds: Vec<serde_json::Value> = commands
            .iter()
            .map(|c| serde_json::json!({"command": c.command, "description": c.description}))
            .collect();
        let body = serde_json::json!({ "commands": cmds });
        let resp = self
            .client
            .post(&url)
            .timeout(STARTUP_API_TIMEOUT)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram setMyCommands failed: {body_text}");
        }
        Ok(())
    }

    /// Call `deleteMyCommands` to remove all bot commands from the Telegram menu.
    ///
    /// Uses the default scope (all chats) and no language filter.
    pub async fn api_delete_my_commands(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/deleteMyCommands",
            self.api_base_url,
            self.token.as_str()
        );
        let body = serde_json::json!({});
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram deleteMyCommands failed: {body_text}");
        }
        Ok(())
    }

    /// Call `getMyCommands` and return the currently registered bot commands.
    ///
    /// Returns an empty vec on failure (best-effort).
    pub async fn api_get_my_commands(&self) -> Vec<BotCommand> {
        let url = format!(
            "{}/bot{}/getMyCommands",
            self.api_base_url,
            self.token.as_str()
        );
        let resp = match self
            .client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!("Telegram getMyCommands request failed: {e}");
                return vec![];
            }
        };
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                debug!("Telegram getMyCommands parse failed: {e}");
                return vec![];
            }
        };
        if body["ok"].as_bool() != Some(true) {
            return vec![];
        }
        body["result"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(BotCommand {
                            command: v["command"].as_str()?.to_string(),
                            description: v["description"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Look up a user's chat status via Bot API `getChatMember`.
    ///
    /// Returns the raw status string (`creator`, `administrator`, `member`,
    /// `restricted`, `left`, `kicked`) on success. Returns `Ok(None)` when
    /// Telegram says the user is not part of the chat (`left` / `kicked`)
    /// since those map to "no derived role" in the kernel mapping logic.
    /// Errors are surfaced for transport/auth failures.
    pub async fn api_get_chat_member(
        &self,
        chat_id: &str,
        user_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/bot{}/getChatMember",
            self.api_base_url,
            self.token.as_str()
        );
        // chat_id may be a numeric id like "-100123" or a `@username`; both
        // are accepted by the Bot API as a JSON string.
        let user_id_num: i64 = user_id
            .parse()
            .map_err(|_| format!("Invalid Telegram user_id: {user_id}"))?;
        let body = serde_json::json!({
            "chat_id": chat_id,
            "user_id": user_id_num,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        let status_code = resp.status();
        let body_value: serde_json::Value = resp.json().await?;
        parse_chat_member_response(&body_value)
            .map_err(|desc| format!("Telegram getChatMember failed ({status_code}): {desc}").into())
    }
}

/// Translate a Telegram `getChatMember` response into an optional status string.
///
/// Returns `Ok(Some(status))` for `creator` / `administrator` / `member` /
/// `restricted`, `Ok(None)` for `left` / `kicked` (treated as "user not in
/// chat" so the kernel falls through to default-deny), and `Err(description)`
/// when the response indicates `ok = false`.
pub(crate) fn parse_chat_member_response(
    body: &serde_json::Value,
) -> Result<Option<String>, String> {
    if body["ok"].as_bool() != Some(true) {
        let desc = body["description"].as_str().unwrap_or("unknown error");
        return Err(desc.to_string());
    }
    let status = body["result"]["status"].as_str().unwrap_or("");
    if status.is_empty() || status == "left" || status == "kicked" {
        return Ok(None);
    }
    Ok(Some(status.to_string()))
}

#[async_trait]
impl ChannelRoleQuery for TelegramAdapter {
    async fn lookup_role(
        &self,
        chat_id: &str,
        user_id: &str,
    ) -> Result<Option<PlatformRole>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .api_get_chat_member(chat_id, user_id)
            .await?
            .map(PlatformRole::single))
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
            ChannelContent::EditInteractive {
                message_id,
                text,
                buttons,
            } => {
                match message_id.parse::<i64>() {
                    Ok(mid) => {
                        self.api_edit_interactive_message(chat_id, mid, &text, &buttons)
                            .await?;
                    }
                    Err(_) => {
                        warn!("Telegram: EditInteractive has invalid message_id '{message_id}', ignoring");
                    }
                }
            }
            ChannelContent::DeleteMessage { message_id } => {
                let msg_id: i64 = message_id
                    .parse()
                    .map_err(|_| format!("Invalid Telegram message_id for delete: {message_id}"))?;
                self.api_delete_message(chat_id, msg_id).await?;
            }
            ChannelContent::Audio {
                url,
                caption,
                title,
                performer,
                ..
            } => {
                self.api_send_audio(
                    chat_id,
                    &url,
                    caption.as_deref(),
                    title.as_deref(),
                    performer.as_deref(),
                    thread_id,
                )
                .await?;
            }
            ChannelContent::Animation { url, caption, .. } => {
                self.api_send_animation(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::Sticker { file_id } => {
                self.api_send_sticker(chat_id, &file_id, thread_id).await?;
            }
            ChannelContent::MediaGroup { items } => {
                self.api_send_media_group(chat_id, &items, thread_id)
                    .await?;
            }
            ChannelContent::Poll {
                question,
                options,
                is_quiz,
                correct_option_id,
                explanation,
            } => {
                let params = PollParams {
                    chat_id,
                    question: &question,
                    options: &options,
                    is_quiz,
                    correct_option_id,
                    explanation: explanation.as_deref(),
                    thread_id,
                };
                match self.api_send_poll(&params).await {
                    Ok(poll_id) if !poll_id.is_empty() => {
                        self.poll_contexts.insert(
                            poll_id,
                            PollContext {
                                question: question.clone(),
                                options: options.clone(),
                                last_accessed: Instant::now(),
                            },
                        );
                    }
                    Ok(_) => {}
                    Err(e) => return Err(e),
                }
            }
            ChannelContent::PollAnswer { .. } => {
                debug!("Telegram: ignoring outbound PollAnswer");
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
        //
        // IMPORTANT: do NOT set drop_pending_updates=true here. For a messaging
        // adapter, silently discarding user messages queued while the daemon was
        // down is a data-loss bug. The Telegram API default (false) preserves the
        // backlog, and the first getUpdates call after startup will pick up every
        // update that accumulated during downtime.
        {
            let delete_url = format!(
                "{}/bot{}/deleteWebhook",
                self.api_base_url,
                self.token.as_str()
            );
            match self
                .client
                .post(&delete_url)
                .json(&serde_json::json!({"drop_pending_updates": false}))
                .timeout(STARTUP_API_TIMEOUT)
                .send()
                .await
            {
                Ok(_) => info!("Telegram: cleared webhook, polling mode active"),
                Err(e) => warn!("Telegram: deleteWebhook failed (non-fatal): {e}"),
            }
        }

        // Register bot commands in the Telegram menu.
        // Use explicitly configured commands, or fall back to built-in defaults.
        let effective_commands = if self.commands.is_empty() {
            builtin_bot_commands()
        } else {
            self.commands.clone()
        };
        match self.api_set_my_commands(&effective_commands).await {
            Ok(()) => info!(
                "Telegram: registered {} bot command(s)",
                effective_commands.len()
            ),
            Err(e) => warn!("Telegram: failed to register bot commands: {e}"),
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
        let bot_commands = effective_commands;
        let poll_contexts = self.poll_contexts.clone();

        // Spawn background cleanup task for poll_contexts
        let poll_contexts_cleanup = poll_contexts.clone();
        let mut shutdown_cleanup = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // Every 5 minutes
            interval.tick().await; // consume the immediate first tick
            loop {
                // Race tick against shutdown so a stop() call doesn't wait up
                // to 5 minutes for the next tick before exiting the loop.
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = shutdown_cleanup.changed() => {
                        if *shutdown_cleanup.borrow() {
                            break;
                        }
                        continue;
                    }
                }
                if *shutdown_cleanup.borrow() {
                    break;
                }
                let now = Instant::now();
                let mut removed = 0;
                poll_contexts_cleanup.retain(|_k, v| {
                    if now.duration_since(v.last_accessed) > Duration::from_secs(1800) {
                        // 30 minutes since last interaction — drop.
                        removed += 1;
                        false
                    } else {
                        true
                    }
                });
                if removed > 0 {
                    debug!(
                        "Telegram: cleaned up {} stale poll_context entries",
                        removed
                    );
                }
            }
        });

        let handle = tokio::spawn(async move {
            let ctx = TelegramApiCtx {
                token: token.as_str(),
                client: &client,
                api_base_url: &api_base_url,
            };
            let mut offset: Option<i64> = None;
            let mut backoff = initial_backoff;
            // Track whether commands have been registered in this polling session.
            // Reset on /start to allow re-registration.
            let mut commands_registered = false;

            loop {
                if *shutdown.borrow() {
                    break;
                }

                // Build getUpdates request
                let url = format!("{}/bot{}/getUpdates", api_base_url, token.as_str());
                let mut params = serde_json::json!({
                    "timeout": long_poll_timeout,
                    "allowed_updates": ["message", "edited_message", "callback_query", "poll_answer"],
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

                    // Handle poll_answer (user answered a poll)
                    if let Some(poll_answer) = update.get("poll_answer") {
                        let poll_id = poll_answer["poll_id"].as_str().unwrap_or("").to_string();
                        let user_id = poll_answer["user"]["id"].as_i64().unwrap_or(0);
                        let username = poll_answer["user"]["username"].as_str();
                        let first_name = poll_answer["user"]["first_name"]
                            .as_str()
                            .unwrap_or("Unknown");
                        let last_name = poll_answer["user"]["last_name"].as_str().unwrap_or("");
                        let display_name = if last_name.is_empty() {
                            first_name.to_string()
                        } else {
                            format!("{first_name} {last_name}")
                        };

                        if !poll_id.is_empty()
                            && telegram_user_allowed(&allowed_users, user_id, username)
                        {
                            let option_ids: Vec<u8> = poll_answer["option_ids"]
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                                        .collect()
                                })
                                .unwrap_or_default();

                            let mut metadata = HashMap::new();
                            metadata.insert(
                                "user_id".to_string(),
                                serde_json::json!(user_id.to_string()),
                            );
                            // poll_answer's sender.platform_id is already the
                            // user id (DMs only — polls don't fire in groups
                            // with this update shape), so the bridge fallback
                            // resolves correctly. Populate the metadata key
                            // anyway for consistency with the message and
                            // callback_query paths so downstream code never
                            // has to special-case the source update kind.
                            metadata.insert(
                                SENDER_USER_ID_KEY.to_string(),
                                serde_json::json!(user_id.to_string()),
                            );

                            let mut msg = ChannelMessage {
                                channel: ChannelType::Telegram,
                                platform_message_id: poll_id.clone(),
                                sender: ChannelUser {
                                    platform_id: user_id.to_string(),
                                    display_name,
                                    librefang_user: None,
                                },
                                content: ChannelContent::PollAnswer {
                                    poll_id,
                                    option_ids,
                                },
                                target_agent: None,
                                timestamp: chrono::Utc::now(),
                                is_group: false,
                                thread_id: None,
                                metadata,
                            };

                            if let Some(ref aid) = account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }

                            // Use `get_mut` so each answer bumps `last_accessed`.
                            // Without this refresh, a poll's question/options
                            // metadata fell out of the cache exactly 30 minutes
                            // after send, regardless of whether users were still
                            // actively answering — long-running surveys lost
                            // their context silently.
                            if let Some(mut ctx) = poll_contexts.get_mut(&msg.platform_message_id) {
                                ctx.last_accessed = Instant::now();
                                msg.metadata.insert(
                                    "poll_question".to_string(),
                                    serde_json::json!(ctx.question),
                                );
                                msg.metadata.insert(
                                    "poll_options".to_string(),
                                    serde_json::json!(ctx.options),
                                );
                            }

                            debug!(
                                "Telegram poll_answer from {}: {:?}",
                                msg.sender.display_name, msg.content
                            );

                            if tx.send(msg).await.is_err() {
                                error!("Telegram dispatch channel closed — poll_answer dropped.");
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

                    // Auto-register bot commands on first message or /start.
                    // This ensures the command menu is populated even if the
                    // initial registration in start() failed, and refreshes it
                    // whenever a user sends /start.
                    if !bot_commands.is_empty() {
                        let is_start = matches!(
                            &msg.content,
                            ChannelContent::Command { name, .. } if name == "start"
                        );
                        if is_start || !commands_registered {
                            ctx.set_my_commands(&bot_commands).await;
                            commands_registered = true;
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
        let emoji = map_reaction_emoji(&reaction.emoji);

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
            // Use UTF-16-aware splitting: Telegram's 4096-char limit is in
            // UTF-16 code units, so emoji-heavy text must use this variant.
            let chunks = split_to_utf16_chunks(&formatted, TELEGRAM_MESSAGE_LIMIT);
            if chunks.len() <= 1 {
                // Single message — just edit in place.
                if let Err(e) = self.api_edit_message(chat_id, msg_id, &formatted).await {
                    warn!("Telegram: failed to edit streamed message (msg_id={msg_id}): {e}");
                }
            } else {
                // Response exceeds limit — edit the first chunk in place,
                // then send remaining chunks as new messages.
                // Errors are logged (not silently dropped) so operators know
                // when partial delivery occurs (issue #3664).
                if let Err(e) = self.api_edit_message(chat_id, msg_id, chunks[0]).await {
                    warn!("Telegram: failed to edit first chunk (msg_id={msg_id}): {e}");
                }
                let total = chunks.len();
                for (i, chunk) in chunks[1..].iter().enumerate() {
                    if let Err(e) = self.api_send_message(chat_id, chunk, tid).await {
                        // i is 0-indexed into chunks[1..], so the absolute
                        // chunk number is i + 2.
                        warn!(
                            "Telegram: failed to send continuation chunk {}/{total}: {e}",
                            i + 2
                        );
                        // Stop sending further chunks — partial delivery is
                        // preferable to sending out-of-order fragments after
                        // a gap caused by a rate-limit or API error.
                        // Surface the error to the caller so retry / alerting
                        // logic can react (issue #3664).
                        return Err(format!(
                            "Telegram streaming send failed on chunk {}/{total}: {e}",
                            i + 2
                        )
                        .into());
                    }
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

    /// Telegram approval-notification recipients are the configured
    /// `allowed_users` (#4875). Only numeric entries are kept — Telegram
    /// `sendMessage` requires a numeric `chat_id`, and a private chat's
    /// `chat_id` equals the user's numeric `user_id`. Bare `@username`
    /// entries are dropped here because there is no API call that resolves
    /// a username to a `chat_id` without a prior message from that user.
    /// An empty `allowed_users` list means "no operator inbox configured",
    /// so we return an empty Vec rather than broadcasting to strangers.
    fn notification_recipients(&self) -> Vec<ChannelUser> {
        self.allowed_users
            .iter()
            .filter(|u| u.parse::<i64>().is_ok())
            .map(|u| ChannelUser {
                platform_id: u.clone(),
                display_name: String::new(),
                librefang_user: None,
            })
            .collect()
    }
}

fn map_reaction_emoji(emoji: &str) -> &str {
    // Telegram only supports a limited set of reaction emoji.
    // Map unsupported ones to the closest Telegram-compatible alternative.
    match emoji {
        "\u{23F3}" => "\u{1F440}",        // ⏳ → 👀
        "\u{2699}\u{FE0F}" => "\u{26A1}", // ⚙️ → ⚡
        "\u{2705}" => "\u{1F389}",        // ✅ → 🎉
        "\u{274C}" => "\u{1F44E}",        // ❌ → 👎
        other => other,                   // 🤔, ✍️ etc. pass through
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
    haystack.as_bytes()[haystack.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
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
    // See parse_telegram_message: RBAC needs the actual user id, not chat_id.
    metadata.insert(
        SENDER_USER_ID_KEY.to_string(),
        serde_json::json!(user_id_str),
    );
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

/// Source of a Telegram message's sender id.
///
/// `From` means the id came from `message.from.id` and is a real user id
/// (positive integer per Telegram API). `SenderChat` means the id came from
/// `message.sender_chat.id` (anonymous channel admins, channel-on-behalf
/// posts) and is a NEGATIVE chat id — semantically a channel, not a user.
///
/// Downstream consumers (RBAC, per-user rate limiting) key off
/// `metadata[SENDER_USER_ID_KEY]`. Callers must prefix `SenderChat` ids
/// with `tg_channel:` before writing them so they cannot collide with — or
/// be mistaken for — a real user identity in the same key namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramSenderSource {
    From,
    SenderChat,
}

/// Extract sender identity from a Telegram message.
///
/// Tries `from` (user) first, then falls back to `sender_chat` (channel/group).
/// Returns `(user_id, display_name, Option<username>, source)`. Callers must
/// inspect `source` before writing `user_id` into `SENDER_USER_ID_KEY` — see
/// `TelegramSenderSource` for the namespace prefix rule.
fn extract_telegram_sender(
    message: &serde_json::Value,
    update_id: i64,
) -> Result<(i64, String, Option<String>, TelegramSenderSource), DropReason> {
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
        Ok((uid, name, username, TelegramSenderSource::From))
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
        Ok((
            uid,
            title.to_string(),
            None,
            TelegramSenderSource::SenderChat,
        ))
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
        // Audio files (MP3, FLAC, etc.) with optional title/performer metadata.
        // Distinct from voice messages — use ChannelContent::Audio.
        let file_id = message["audio"]["file_id"].as_str().unwrap_or("");
        let duration = message["audio"]["duration"].as_u64().unwrap_or(0) as u32;
        let caption = message["caption"].as_str().map(String::from);
        let title = message["audio"]["title"].as_str().map(String::from);
        let performer = message["audio"]["performer"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Audio {
                url,
                caption,
                duration_seconds: duration,
                title,
                performer,
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
    } else if message.get("animation").is_some() {
        // Animated GIF or MPEG-4 video without sound.
        let file_id = message["animation"]["file_id"].as_str().unwrap_or("");
        let duration = message["animation"]["duration"].as_u64().unwrap_or(0) as u32;
        let caption = message["caption"].as_str().map(String::from);
        match ctx.get_file_url(file_id).await {
            Some(url) => Ok(ChannelContent::Animation {
                url,
                caption,
                duration_seconds: duration,
            }),
            None => Ok(ChannelContent::Text(format!(
                "[Animation received, {duration}s{}]",
                caption
                    .as_deref()
                    .map(|c| format!(": {c}"))
                    .unwrap_or_default()
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
    } else if message.get("sticker").is_some() {
        // Sticker — identified by file_id, not a download URL.
        let file_id = message["sticker"]["file_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if file_id.is_empty() {
            Err(DropReason::ParseError(format!(
                "update {update_id}: sticker missing file_id"
            )))
        } else {
            Ok(ChannelContent::Sticker { file_id })
        }
    } else {
        // Unsupported message type (e.g. dice, contact, venue, invoice)
        Err(DropReason::Filtered(format!(
            "update {update_id}: unsupported message type (no text/photo/document/audio/voice/animation/video/video_note/location/sticker)"
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

    let (user_id, display_name, username, sender_source) =
        extract_telegram_sender(message, update_id)?;

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

    // RBAC and per-user rate limiting key off `metadata[SENDER_USER_ID_KEY]`.
    // For group/supergroup chats `sender.platform_id` is the chat_id, not the
    // sender's user id, so without this entry `bridge::sender_user_id()` would
    // fall back to the chat_id and `auth_manager.identify("telegram", chat_id)`
    // would reject every group message as "Unrecognized user".
    //
    // For `sender_chat` (anonymous channel admins / channel-on-behalf posts)
    // the id is a NEGATIVE chat id, not a user id. Prefix it with
    // `tg_channel:` so it cannot be mistaken for — or collide with — a real
    // user identity in the shared SENDER_USER_ID_KEY namespace consumed by
    // RBAC and per-user rate limiting.
    let sender_user_id_value = match sender_source {
        TelegramSenderSource::From => user_id.to_string(),
        TelegramSenderSource::SenderChat => format!("tg_channel:{user_id}"),
    };
    metadata.insert(
        SENDER_USER_ID_KEY.to_string(),
        serde_json::json!(sender_user_id_value),
    );

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
///
/// For `<a>` tags, only `https`, `http`, `mailto`, and `tg` URL schemes are
/// allowed; tags with disallowed/unparseable schemes (e.g. `javascript:`)
/// are stripped to plain text. All attribute values are HTML-escaped to
/// prevent attribute injection past Telegram's parse_mode=HTML boundary.
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
                    let tag_name_lc = tag_name_raw.to_ascii_lowercase();
                    if is_closing {
                        if let Some(pos) = open_tags.iter().rposition(|t| t == &tag_name_lc) {
                            open_tags.remove(pos);
                            // Preserve original case of close tag.
                            result.push_str(&text[i..tag_end + 1]);
                        } else {
                            result.push_str("&lt;");
                            result.push_str(&escape_html_text(tag_content));
                            result.push_str("&gt;");
                        }
                    } else {
                        // Self-closing or opening: rebuild tag with sanitized attrs.
                        let self_closing = tag_content.ends_with('/');
                        let attrs_raw = &tag_content[tag_name_raw.len()..];
                        let attrs_raw = attrs_raw.trim_end_matches('/').trim();
                        match rebuild_safe_tag(tag_name_raw, attrs_raw, self_closing) {
                            Some(rebuilt) => {
                                result.push_str(&rebuilt);
                                if !self_closing {
                                    open_tags.push(tag_name_lc);
                                }
                            }
                            None => {
                                // Tag rejected (e.g. <a> with bad scheme): drop the
                                // tag entirely; surrounding inner text still renders.
                            }
                        }
                    }
                } else {
                    // Unknown tag — escape both brackets, escape inner content too.
                    result.push_str("&lt;");
                    result.push_str(&escape_html_text(tag_content));
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

/// Escape `<`, `>`, `&`, `"` in arbitrary text (not pre-escaped HTML).
fn escape_html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// URL schemes allowed inside `<a href>` for Telegram HTML.
const ALLOWED_HREF_SCHEMES: &[&str] = &["https", "http", "mailto", "tg"];

/// Returns true if `url` starts with one of the allowed schemes (case-insensitive).
/// Relative URLs (no scheme) are rejected — Telegram requires absolute URLs.
fn is_safe_href(url: &str) -> bool {
    let trimmed = url.trim();
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let scheme = &trimmed[..colon];
    ALLOWED_HREF_SCHEMES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(scheme))
}

/// Parse simple `key="value"` / `key='value'` attributes into an ordered list.
fn parse_attrs(attrs: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Read key
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = attrs[key_start..i].to_ascii_lowercase();
        if key.is_empty() {
            break;
        }
        // Expect '='
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            // Bare attribute, no value
            out.push((key, String::new()));
            continue;
        }
        i += 1; // consume =
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Read value (quoted or bare)
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let val_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let val = attrs[val_start..i].to_string();
            if i < bytes.len() {
                i += 1;
            }
            out.push((key, val));
        } else {
            let val_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push((key, attrs[val_start..i].to_string()));
        }
    }
    out
}

/// Rebuild an opening tag with sanitized attributes. Returns `None` if the
/// tag should be rejected entirely (e.g. `<a>` with an unsafe href).
/// `tag_name` keeps its original case; `self_closing` adds a trailing `/`.
fn rebuild_safe_tag(tag_name: &str, attrs_raw: &str, self_closing: bool) -> Option<String> {
    let attrs = parse_attrs(attrs_raw);
    let mut buf = String::from("<");
    buf.push_str(tag_name);
    match tag_name.to_ascii_lowercase().as_str() {
        "a" => {
            // Find href; reject tag if missing or unsafe.
            let href = attrs.iter().find(|(k, _)| k == "href").map(|(_, v)| v);
            let href = href?;
            if !is_safe_href(href) {
                return None;
            }
            buf.push_str(" href=\"");
            buf.push_str(&escape_html_text(href));
            buf.push('"');
        }
        "code" => {
            // Telegram supports a single `class="language-xxx"` on code.
            if let Some((_, v)) = attrs.iter().find(|(k, _)| k == "class") {
                buf.push_str(" class=\"");
                buf.push_str(&escape_html_text(v));
                buf.push('"');
            }
        }
        "tg-emoji" => {
            // Telegram custom emoji uses `emoji-id="…"`.
            if let Some((_, v)) = attrs.iter().find(|(k, _)| k == "emoji-id") {
                buf.push_str(" emoji-id=\"");
                buf.push_str(&escape_html_text(v));
                buf.push('"');
            }
        }
        _ => {
            // Other allowed tags: drop all attributes.
        }
    }
    if self_closing {
        buf.push('/');
    }
    buf.push('>');
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_resolution_creator_passes_through() {
        let body = serde_json::json!({
            "ok": true,
            "result": { "status": "creator", "user": { "id": 1 } }
        });
        assert_eq!(
            parse_chat_member_response(&body).unwrap(),
            Some("creator".to_string())
        );
    }

    #[test]
    fn role_resolution_administrator_passes_through() {
        let body = serde_json::json!({
            "ok": true,
            "result": { "status": "administrator", "user": { "id": 1 } }
        });
        assert_eq!(
            parse_chat_member_response(&body).unwrap(),
            Some("administrator".to_string())
        );
    }

    #[test]
    fn role_resolution_member_passes_through() {
        let body = serde_json::json!({
            "ok": true,
            "result": { "status": "member", "user": { "id": 1 } }
        });
        assert_eq!(
            parse_chat_member_response(&body).unwrap(),
            Some("member".to_string())
        );
    }

    #[test]
    fn role_resolution_left_returns_none() {
        // `left` and `kicked` should map to None so the kernel falls
        // through to default-deny instead of treating "user not in chat"
        // as a hard error.
        let body = serde_json::json!({
            "ok": true,
            "result": { "status": "left" }
        });
        assert!(parse_chat_member_response(&body).unwrap().is_none());
    }

    #[test]
    fn role_resolution_kicked_returns_none() {
        let body = serde_json::json!({
            "ok": true,
            "result": { "status": "kicked" }
        });
        assert!(parse_chat_member_response(&body).unwrap().is_none());
    }

    #[test]
    fn role_resolution_api_error_is_err() {
        let body = serde_json::json!({
            "ok": false,
            "description": "USER_ID_INVALID"
        });
        let err = parse_chat_member_response(&body).unwrap_err();
        assert!(err.contains("USER_ID_INVALID"));
    }

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
            ChannelContent::Audio {
                caption,
                duration_seconds,
                title,
                ..
            } => {
                assert_eq!(*duration_seconds, 120);
                assert_eq!(caption.as_deref(), Some("riassumi"));
                assert_eq!(title.as_deref(), Some("recording.mp3"));
            }
            other => panic!("Expected Text or Audio for audio message, got {other:?}"),
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
        // sender_chat fallback yields the *channel id*, not a user id. We
        // still write it into SENDER_USER_ID_KEY for shape consistency, but
        // it MUST be prefixed with `tg_channel:` so it cannot collide with
        // — or be mistaken for — a real user identity in the shared
        // namespace consumed by RBAC and per-user rate limiting. RBAC will
        // (correctly) reject it because no `[[users]]` entry maps to a
        // `tg_channel:` prefixed key.
        assert_eq!(
            msg.metadata
                .get(crate::bridge::SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("tg_channel:-1001999888777"),
        );
    }

    #[tokio::test]
    async fn test_parse_from_user_no_prefix() {
        // Regression: ensure normal `from`-sourced messages write the bare
        // integer string into SENDER_USER_ID_KEY (no `tg_channel:` prefix).
        // The prefix is reserved exclusively for the sender_chat fallback
        // path; applying it to real users would break RBAC lookups.
        let update = serde_json::json!({
            "update_id": 502,
            "message": {
                "message_id": 82,
                "from": {
                    "id": 12345_i64,
                    "first_name": "Alice",
                    "username": "alice"
                },
                "chat": { "id": 12345_i64, "type": "private" },
                "date": 1700000000,
                "text": "hi"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert_eq!(
            msg.metadata
                .get(crate::bridge::SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("12345"),
            "from-sourced sender id must be the bare integer, no `tg_channel:` prefix",
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
    async fn test_group_message_carries_sender_user_id_metadata() {
        // Regression: in groups, sender.platform_id is the chat_id. The
        // actual sender user id must be exposed via metadata[SENDER_USER_ID_KEY]
        // so RBAC and per-user rate limiting resolve to the user, not the
        // group, otherwise auth_manager.identify("telegram", chat_id) rejects
        // every group message as "Unrecognized user".
        let update = serde_json::json!({
            "update_id": 700,
            "message": {
                "message_id": 100,
                "from": { "id": 111222333_i64, "first_name": "Alice" },
                "chat": { "id": -1001234567890_i64, "type": "supergroup" },
                "date": 1700000000,
                "text": "ping"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert!(msg.is_group);
        assert_eq!(msg.sender.platform_id, "-1001234567890");
        assert_eq!(
            msg.metadata
                .get(crate::bridge::SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("111222333"),
        );
    }

    #[tokio::test]
    async fn test_private_message_also_carries_sender_user_id_metadata() {
        // In DMs platform_id == user_id, but downstream code shouldn't have to
        // special-case channel mode — always populate the metadata key.
        let update = serde_json::json!({
            "update_id": 701,
            "message": {
                "message_id": 101,
                "from": { "id": 111222333_i64, "first_name": "Alice" },
                "chat": { "id": 111222333_i64, "type": "private" },
                "date": 1700000000,
                "text": "ping"
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        assert!(!msg.is_group);
        assert_eq!(
            msg.metadata
                .get(crate::bridge::SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("111222333"),
        );
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
    fn test_sanitize_telegram_html_tg_spoiler_allowed() {
        let input = "hello <tg-spoiler>hidden</tg-spoiler> world";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, input, "tg-spoiler should pass through");
    }

    #[test]
    fn test_sanitize_telegram_html_tg_emoji_allowed() {
        let input = "<tg-emoji emoji-id=\"123\">😀</tg-emoji> hi";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, input, "tg-emoji should pass through");
    }

    #[test]
    fn test_sanitize_telegram_html_closing_tag_never_opened() {
        let input = "hello </b> world";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "hello &lt;/b&gt; world");
    }

    #[test]
    fn test_sanitize_telegram_html_empty_input() {
        let input = "";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "");
    }

    #[test]
    fn test_sanitize_telegram_html_text_only() {
        let input = "hello world";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_sanitize_telegram_html_lone_open_bracket_at_end() {
        let input = "text <";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "text &lt;");
    }

    #[test]
    fn test_sanitize_telegram_html_br_not_allowed() {
        let input = "line <br> break";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "line &lt;br&gt; break");
    }

    #[test]
    fn test_sanitize_telegram_html_uppercase_allowed_tags() {
        let input = "<B>BOLD</B> <I>italic</I>";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, input, "uppercase allowed tags should pass through");
    }

    #[test]
    fn test_sanitize_telegram_html_mixed_allowed_and_disallowed_nested() {
        let input = "<b><name>John</name></b>";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "<b>&lt;name&gt;John&lt;/name&gt;</b>");
    }

    #[test]
    fn test_sanitize_telegram_html_self_closing_allowed_tag() {
        let input = "<code/>";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "<code/>");
    }

    #[test]
    fn test_sanitize_telegram_html_unclosed_allowed_tag_is_closed() {
        let input = "<b>hello";
        let output = sanitize_telegram_html(input);
        assert_eq!(output, "<b>hello</b>");
    }

    #[test]
    fn test_sanitize_telegram_html_javascript_href_dropped() {
        // javascript: scheme must be stripped, inner text preserved.
        let input = r#"<a href="javascript:alert(1)">click</a>"#;
        let output = sanitize_telegram_html(input);
        assert!(!output.contains("javascript:"), "got: {output}");
        assert!(!output.contains("href="), "got: {output}");
        assert!(output.contains("click"), "got: {output}");
    }

    #[test]
    fn test_sanitize_telegram_html_data_href_dropped() {
        let input = r#"<a href="data:text/html,<script>">x</a>"#;
        let output = sanitize_telegram_html(input);
        assert!(!output.contains("data:"), "got: {output}");
    }

    #[test]
    fn test_sanitize_telegram_html_relative_href_dropped() {
        // No scheme → relative URL → reject.
        let input = r#"<a href="/admin">x</a>"#;
        let output = sanitize_telegram_html(input);
        assert!(!output.contains("href="), "got: {output}");
    }

    #[test]
    fn test_sanitize_telegram_html_safe_schemes_kept() {
        for scheme in ["https", "http", "mailto", "tg"] {
            let input = format!(r#"<a href="{scheme}://x">x</a>"#);
            let output = sanitize_telegram_html(&input);
            assert!(
                output.contains(&format!("{scheme}://x")),
                "scheme {scheme} should be kept: {output}"
            );
        }
    }

    #[test]
    fn test_sanitize_telegram_html_attr_injection_escaped() {
        // Attribute value with embedded quote/bracket must be escaped.
        let input = r#"<a href="https://x"" onclick="hack()"">click</a>"#;
        let output = sanitize_telegram_html(input);
        assert!(!output.contains("onclick"), "got: {output}");
    }

    #[test]
    fn test_sanitize_telegram_html_unknown_tag_attr_escaped() {
        // Unknown tag with HTML-special characters in body must be escaped.
        let input = r#"<thinking attr="&">"#;
        let output = sanitize_telegram_html(input);
        assert!(
            output.contains("&amp;"),
            "ampersand should be escaped: {output}"
        );
    }

    #[test]
    fn test_sanitize_telegram_html_drops_disallowed_attrs_on_b() {
        // Allowed tag with extra attrs: attrs should be dropped.
        let input = r#"<b style="color:red" onclick="x()">bold</b>"#;
        let output = sanitize_telegram_html(input);
        assert!(!output.contains("style"), "got: {output}");
        assert!(!output.contains("onclick"), "got: {output}");
        assert!(output.contains("<b>"), "got: {output}");
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
        // Regression: callback_query in a supergroup must expose the actual
        // user id via SENDER_USER_ID_KEY so RBAC resolves to the user, not
        // to the group's chat_id (which platform_id holds for groups).
        // Mirrors the message-path coverage in
        // test_group_message_carries_sender_user_id_metadata.
        assert_eq!(
            msg.metadata
                .get(crate::bridge::SENDER_USER_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("42"),
        );
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

    #[test]
    fn test_truncate_with_ellipsis_short() {
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_with_ellipsis_long() {
        let result = truncate_with_ellipsis("hello world", 5);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 8);
    }

    #[test]
    fn test_truncate_with_ellipsis_exactly_at_max() {
        assert_eq!(truncate_with_ellipsis("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_with_ellipsis_empty() {
        assert_eq!(truncate_with_ellipsis("", 10), "");
    }

    #[test]
    fn test_extract_retry_after_present() {
        let json = r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":30}}"#;
        assert_eq!(extract_retry_after(json, 99), 30);
    }

    #[test]
    fn test_extract_retry_after_missing() {
        let json = r#"{"ok":true}"#;
        assert_eq!(extract_retry_after(json, 5), 5);
    }

    #[test]
    fn test_extract_retry_after_invalid() {
        assert_eq!(extract_retry_after("not json", 7), 7);
    }

    #[test]
    fn test_is_group_chat_group() {
        assert!(is_group_chat("group"));
    }

    #[test]
    fn test_is_group_chat_supergroup() {
        assert!(is_group_chat("supergroup"));
    }

    #[test]
    fn test_is_group_chat_private() {
        assert!(!is_group_chat("private"));
    }

    #[test]
    fn test_is_group_chat_channel() {
        assert!(!is_group_chat("channel"));
    }

    #[test]
    fn test_is_group_chat_empty() {
        assert!(!is_group_chat(""));
    }

    #[test]
    fn test_ends_with_ascii_ci_true() {
        assert!(ends_with_ascii_ci("photo.jpg", ".JPG"));
    }

    #[test]
    fn test_ends_with_ascii_ci_false() {
        assert!(!ends_with_ascii_ci("photo.jpg", ".png"));
    }

    #[test]
    fn test_ends_with_ascii_ci_suffix_longer() {
        assert!(!ends_with_ascii_ci("a", "longsuffix"));
    }

    #[test]
    fn test_ends_with_ascii_ci_exact() {
        assert!(ends_with_ascii_ci("photo.jpg", ".jpg"));
    }

    #[test]
    fn test_mime_type_from_telegram_path_jpg() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.jpg"),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_jpeg() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.jpeg"),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_png_case_insensitive() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.PNG"),
            Some("image/png")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_gif() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.gif"),
            Some("image/gif")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_webp() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.webp"),
            Some("image/webp")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_bmp() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.bmp"),
            Some("image/bmp")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_tiff() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.tiff"),
            Some("image/tiff")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_tif() {
        assert_eq!(
            mime_type_from_telegram_path("/path/photo.tif"),
            Some("image/tiff")
        );
    }

    #[test]
    fn test_mime_type_from_telegram_path_unknown() {
        assert_eq!(mime_type_from_telegram_path("/path/photo.xyz"), None);
    }

    #[test]
    fn test_mime_type_from_telegram_path_no_ext() {
        assert_eq!(mime_type_from_telegram_path("/path/photo"), None);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_ascii() {
        assert_eq!(utf16_offset_to_byte_offset("hello", 3), 3);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_emoji() {
        assert_eq!(utf16_offset_to_byte_offset("a😀b", 1), 1);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_mixed() {
        assert_eq!(utf16_offset_to_byte_offset("a😀c", 2), 5);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_beyond_end() {
        assert_eq!(utf16_offset_to_byte_offset("hello", 100), 5);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_bmp_char() {
        assert_eq!(utf16_offset_to_byte_offset("aβc", 1), 1);
    }

    #[test]
    fn test_utf16_offset_to_byte_offset_inside_surrogate_pair_rounds_forward() {
        assert_eq!(utf16_offset_to_byte_offset("a😀b", 2), 5);
    }

    #[test]
    fn test_map_reaction_emoji_supported_mappings() {
        assert_eq!(map_reaction_emoji("\u{23F3}"), "\u{1F440}");
        assert_eq!(map_reaction_emoji("\u{2699}\u{FE0F}"), "\u{26A1}");
        assert_eq!(map_reaction_emoji("\u{2705}"), "\u{1F389}");
        assert_eq!(map_reaction_emoji("\u{274C}"), "\u{1F44E}");
    }

    #[test]
    fn test_map_reaction_emoji_passes_through_supported_emoji() {
        assert_eq!(map_reaction_emoji("\u{1F914}"), "\u{1F914}");
    }

    #[test]
    fn test_parse_chat_id_valid() {
        let user = ChannelUser {
            platform_id: "123456".to_string(),
            display_name: "Test User".to_string(),
            librefang_user: None,
        };
        let result = TelegramAdapter::parse_chat_id(&user);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 123456);
    }

    #[test]
    fn test_parse_chat_id_invalid() {
        let user = ChannelUser {
            platform_id: "abc".to_string(),
            display_name: "Test User".to_string(),
            librefang_user: None,
        };
        let result = TelegramAdapter::parse_chat_id(&user);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_telegram_video_message() {
        let update = serde_json::json!({
            "update_id": 800,
            "message": {
                "message_id": 1,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "video": { "file_id": "vid_id", "file_unique_id": "v", "duration": 60, "width": 1920, "height": 1080 }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Video received"), "got: {t}");
                assert!(t.contains("60s"), "got: {t}");
            }
            other => panic!("Expected Text fallback for video message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_video_note_message() {
        let update = serde_json::json!({
            "update_id": 801,
            "message": {
                "message_id": 2,
                "from": { "id": 123, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "video_note": { "file_id": "vn_id", "file_unique_id": "vn", "duration": 30, "length": 240 }
            }
        });

        let client = test_client();
        let msg = parse_telegram_update(&update, &[], &test_ctx(&client), None)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Video note"), "got: {t}");
                assert!(t.contains("30s"), "got: {t}");
            }
            other => panic!("Expected Text fallback for video_note message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_update_no_message_field() {
        let update = serde_json::json!({
            "update_id": 802
        });

        let client = test_client();
        let result = parse_telegram_update(&update, &[], &test_ctx(&client), None).await;
        assert!(result.is_err());
        match result {
            Err(DropReason::ParseError(_)) => {}
            other => panic!("Expected DropReason::ParseError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_sender_chat_without_id() {
        let update = serde_json::json!({
            "update_id": 803,
            "message": {
                "message_id": 3,
                "sender_chat": { "title": "My Channel", "type": "channel" },
                "chat": { "id": -100123, "type": "supergroup" },
                "date": 1700000000,
                "text": "hello"
            }
        });

        let client = test_client();
        let result = parse_telegram_update(&update, &[], &test_ctx(&client), None).await;
        assert!(result.is_err());
        match result {
            Err(DropReason::ParseError(_)) => {}
            other => panic!("Expected DropReason::ParseError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_from_without_id() {
        let update = serde_json::json!({
            "update_id": 804,
            "message": {
                "message_id": 4,
                "from": { "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "hello"
            }
        });

        let client = test_client();
        let result = parse_telegram_update(&update, &[], &test_ctx(&client), None).await;
        assert!(result.is_err());
        match result {
            Err(DropReason::ParseError(_)) => {}
            other => panic!("Expected DropReason::ParseError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_with_message_thread_id() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_thread",
            "from": { "id": 42, "first_name": "Alice" },
            "data": "action",
            "message": {
                "message_id": 1,
                "message_thread_id": 17,
                "chat": { "id": -100123, "type": "supergroup" },
                "text": "question",
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client)).unwrap();
        assert_eq!(msg.thread_id, Some("17".to_string()));
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_without_message_field() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_no_msg",
            "from": { "id": 42 },
            "data": "action"
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client));
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_without_from_field() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_no_from",
            "data": "action",
            "message": {
                "message_id": 1,
                "chat": { "id": 123 },
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client));
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_telegram_callback_query_with_last_name() {
        let client = crate::http_client::new_client();
        let callback = serde_json::json!({
            "id": "cb_last",
            "from": { "id": 42, "first_name": "Alice", "last_name": "Smith" },
            "data": "action",
            "message": {
                "message_id": 1,
                "chat": { "id": 42, "type": "private" },
                "text": "hi",
                "date": 1700000000
            }
        });

        let msg = parse_telegram_callback_query(&callback, &[], &test_ctx(&client)).unwrap();
        assert_eq!(msg.sender.display_name, "Alice Smith");
    }

    #[tokio::test]
    async fn test_apply_reply_context_text_reply_to_text_message() {
        let client = test_client();
        let ctx = test_ctx(&client);

        let message = serde_json::json!({
            "reply_to_message": {
                "message_id": 99,
                "from": { "id": 456, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1699999900,
                "text": "The sky is blue"
            }
        });

        let content = ChannelContent::Text("I disagree".to_string());
        let result = apply_reply_context(content, &message, &ctx).await;
        match result {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Alice: \"The sky is blue\"]\n"));
                assert!(t.contains("I disagree"));
            }
            other => panic!("expected Text with prefix, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_apply_reply_context_text_reply_to_caption_only() {
        let client = test_client();
        let ctx = test_ctx(&client);

        let message = serde_json::json!({
            "reply_to_message": {
                "message_id": 99,
                "from": { "id": 456, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1699999900,
                "caption": "Look at this photo"
            }
        });

        let content = ChannelContent::Text("nice".to_string());
        let result = apply_reply_context(content, &message, &ctx).await;
        match result {
            ChannelContent::Text(t) => {
                assert!(t.starts_with("[Replying to Alice: \"Look at this photo\"]\n"));
                assert!(t.contains("nice"));
            }
            other => panic!("expected Text with prefix, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_apply_reply_context_no_reply_to_message() {
        let client = test_client();
        let ctx = test_ctx(&client);

        let message = serde_json::json!({
            "message_id": 100,
            "from": { "id": 123, "first_name": "Bob" },
            "chat": { "id": 123, "type": "private" },
            "date": 1700000000,
            "text": "Hello"
        });

        let content = ChannelContent::Text("Hello".to_string());
        let result = apply_reply_context(content, &message, &ctx).await;
        match result {
            ChannelContent::Text(t) => assert_eq!(t, "Hello"),
            other => panic!("expected unchanged Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_apply_reply_context_text_reply_to_photo_no_url() {
        let client = test_client();
        let ctx = test_ctx(&client);

        let message = serde_json::json!({
            "reply_to_message": {
                "message_id": 99,
                "from": { "id": 456, "first_name": "Alice" },
                "chat": { "id": 123, "type": "private" },
                "date": 1699999900,
                "photo": [
                    { "file_id": "small_id", "file_unique_id": "a", "width": 90, "height": 90 },
                    { "file_id": "large_id", "file_unique_id": "b", "width": 800, "height": 600 }
                ]
            }
        });

        let content = ChannelContent::Text("I disagree".to_string());
        let result = apply_reply_context(content, &message, &ctx).await;
        match result {
            ChannelContent::Text(t) => {
                assert_eq!(t, "I disagree");
            }
            other => {
                panic!("expected unchanged Text (no photo URL with fake token), got {other:?}")
            }
        }
    }

    // ----- send() path tests (issue #3820) -----
    //
    // Continues the #3820 series (slack/discord/teams/line/dingtalk/messenger/
    // mattermost/bluesky/viber/keybase/mastodon/nextcloud/ntfy/pumble/reddit/
    // gotify already covered). Uses `wiremock` to stand up a local HTTP server
    // and points `TelegramAdapter` at it via the `api_url` constructor parameter.
    // Exercises `ChannelAdapter::send` and `ChannelAdapter::send_in_thread`,
    // asserting URL path (`/bot{token}/{method}`), JSON body shape, the
    // 4096-UTF-16-unit chunking boundary, HTML sanitization on the wire, and
    // failure propagation.

    use wiremock::matchers::{body_json, body_partial_json, method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TOKEN: &str = "123456:test-bot-token";

    fn make_send_adapter(api_base: String) -> TelegramAdapter {
        TelegramAdapter::new(
            TEST_TOKEN.to_string(),
            vec![],
            std::time::Duration::from_secs(5),
            Some(api_base),
        )
    }

    fn dummy_user(chat_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: chat_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn telegram_send_text_calls_send_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .and(body_json(serde_json::json!({
                "chat_id": 12345_i64,
                "text": "hello from librefang",
                "parse_mode": "HTML",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 12345, "type": "private" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send(
                &dummy_user("12345"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn telegram_send_propagates_non_2xx_failure() {
        // Non-recoverable failure (HTTP 500, body without "can't parse entities")
        // must surface to the caller — silently swallowing would leave operators
        // unaware of delivery loss.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        let err = adapter
            .send(&dummy_user("12345"), ChannelContent::Text("kaboom".into()))
            .await
            .expect_err("send must propagate non-2xx as error");
        let msg = err.to_string();
        assert!(
            msg.contains("Telegram sendMessage failed"),
            "expected error to mention sendMessage failure, got: {msg}"
        );
    }

    #[tokio::test]
    async fn telegram_send_long_text_splits_at_4096_boundary() {
        // Telegram caps a single sendMessage at 4096 UTF-16 units. A 6000-char
        // ASCII payload must be split into 2 chunks; both must hit the wire.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 1, "type": "private" } },
            })))
            .expect(2)
            .mount(&server)
            .await;

        let long_text: String = "a".repeat(6000);
        let adapter = make_send_adapter(server.uri());
        adapter
            .send(&dummy_user("1"), ChannelContent::Text(long_text))
            .await
            .expect("send must succeed across two chunks");
        // Wiremock's `expect(2)` is verified on MockServer drop; force it now
        // so the assertion failure (if any) is attributed to this test.
        drop(server);
    }

    #[tokio::test]
    async fn telegram_send_command_formats_with_slash_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .and(body_json(serde_json::json!({
                "chat_id": 7_i64,
                "text": "/start arg1 arg2",
                "parse_mode": "HTML",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 7, "type": "private" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send(
                &dummy_user("7"),
                ChannelContent::Command {
                    name: "start".into(),
                    args: vec!["arg1".into(), "arg2".into()],
                },
            )
            .await
            .expect("command send must succeed");
    }

    #[tokio::test]
    async fn telegram_send_in_thread_includes_message_thread_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .and(body_json(serde_json::json!({
                "chat_id": 9000_i64,
                "text": "topic reply",
                "parse_mode": "HTML",
                "message_thread_id": 42_i64,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 9000, "type": "supergroup" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send_in_thread(
                &dummy_user("9000"),
                ChannelContent::Text("topic reply".into()),
                "42",
            )
            .await
            .expect("send_in_thread must succeed");
    }

    #[tokio::test]
    async fn telegram_send_unknown_html_tag_is_escaped_on_the_wire() {
        // Sanitizer must escape disallowed tags (e.g. <thinking>) before the
        // request leaves the adapter — otherwise Telegram's parse_mode=HTML
        // rejects the message with 400. We assert on the actual JSON body to
        // catch regressions in the sanitize boundary.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendMessage")))
            .and(body_partial_json(serde_json::json!({
                "chat_id": 1_i64,
                "text": "&lt;thinking&gt;hidden&lt;/thinking&gt; visible",
                "parse_mode": "HTML",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 1, "type": "private" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send(
                &dummy_user("1"),
                ChannelContent::Text("<thinking>hidden</thinking> visible".into()),
            )
            .await
            .expect("send with unknown HTML tag must succeed (sanitized)");
    }

    #[tokio::test]
    async fn telegram_send_location_calls_send_location_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendLocation")))
            .and(body_json(serde_json::json!({
                "chat_id": 1_i64,
                "latitude": 37.7749,
                "longitude": -122.4194,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 1, "type": "private" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send(
                &dummy_user("1"),
                ChannelContent::Location {
                    lat: 37.7749,
                    lon: -122.4194,
                },
            )
            .await
            .expect("location send must succeed");
    }

    #[tokio::test]
    async fn telegram_send_image_calls_send_photo_with_caption() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{TEST_TOKEN}/sendPhoto")))
            .and(body_json(serde_json::json!({
                "chat_id": 1_i64,
                "photo": "https://example.com/cat.png",
                "caption": "look at this cat",
                "parse_mode": "HTML",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": { "message_id": 1, "date": 0, "chat": { "id": 1, "type": "private" } },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        adapter
            .send(
                &dummy_user("1"),
                ChannelContent::Image {
                    url: "https://example.com/cat.png".into(),
                    caption: Some("look at this cat".into()),
                    mime_type: None,
                },
            )
            .await
            .expect("image send must succeed");
    }

    #[tokio::test]
    async fn telegram_send_interactive_falls_through_silently_on_5xx() {
        // PRODUCTION OBSERVATION (#3820): Telegram's
        // `api_send_interactive_message` logs warn! on non-2xx but always
        // returns Ok(()). This is fail-open behaviour — inconsistent with
        // `api_send_message` (which propagates non-2xx as Err). This test
        // pins the current behaviour so a future fix is a deliberate change,
        // not an accidental drift. Tracked in the PR body.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(format!(r"^/bot{TEST_TOKEN}/sendMessage$")))
            .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_send_adapter(server.uri());
        let res = adapter
            .send(
                &dummy_user("1"),
                ChannelContent::Interactive {
                    text: "pick one".into(),
                    buttons: vec![vec![InteractiveButton {
                        label: "ok".into(),
                        action: "ok".into(),
                        url: None,
                        style: None,
                    }]],
                },
            )
            .await;
        assert!(
            res.is_ok(),
            "interactive send currently swallows 5xx (see PR #3820 production observation)"
        );
    }
}
