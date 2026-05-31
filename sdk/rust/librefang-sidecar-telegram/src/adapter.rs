//! The `TelegramAdapter` ties everything together: produce-side long-poll loop, on_send / on_command dispatch.

use crate::access::AllowList;
use crate::api::client::DEFAULT_LONGPOLL_TIMEOUT_SECS;
use crate::api::{BotClient, Error};
use crate::dispatcher::{
    dispatch_content, is_message_not_modified, is_parse_entities_error, send_text,
};
use crate::reaction::map_reaction;
use crate::translator::{extract_sender, sender_from_user, update_to_event};
use async_trait::async_trait;
use librefang_sidecar::{Command, EmitFn, SendCommand, SidecarAdapter};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const ALLOWED_UPDATES: &[&str] = &["message", "edited_message", "callback_query", "poll_answer"];
const MAX_BACKOFF_SECS: u64 = 300;
const STREAM_EDIT_INTERVAL_MS: u64 = 1000;

/// Cached state of the `TELEGRAM_LOG` env var. When non-empty AND not `"off"` / `"0"`, the adapter emits one-line happy-path traces to stderr (which the supervisor captures into the daemon's main log) for every inbound update and every outbound command. Errors always log regardless.
static HAPPY_PATH_LOG: once_cell::sync::Lazy<bool> = once_cell::sync::Lazy::new(|| {
    std::env::var("TELEGRAM_LOG")
        .ok()
        .map(|v| {
            let t = v.trim();
            !t.is_empty() && t != "0" && !t.eq_ignore_ascii_case("off")
        })
        .unwrap_or(false)
});

/// Emit a one-line trace to stderr if `TELEGRAM_LOG` is enabled. Argument is a closure so the format work is skipped when logging is off.
fn trace(args: std::fmt::Arguments<'_>) {
    if *HAPPY_PATH_LOG {
        eprintln!("[telegram] {args}");
    }
}

macro_rules! tg_trace {
    ($($arg:tt)*) => { trace(format_args!($($arg)*)) };
}

pub struct TelegramAdapter {
    client: Arc<BotClient>,
    allowlist: AllowList,
    clear_done_reaction: bool,
    /// Per-stream state for `stream_start` / `stream_delta` / `stream_end`.
    /// Keyed by stream_id; tracks the message_id we are editing, the accumulated text, and the last-edit time so deltas can be throttled.
    streams: Arc<Mutex<HashMap<String, StreamState>>>,
}

struct StreamState {
    chat_id: i64,
    message_id: i64,
    buf: String,
    last_edit: Instant,
}

impl TelegramAdapter {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").map_err(
            |_| -> Box<dyn std::error::Error + Send + Sync> {
                "TELEGRAM_BOT_TOKEN must be set".into()
            },
        )?;
        let client = Arc::new(BotClient::new(token)?);
        let allowlist = AllowList::from_env_value(std::env::var("ALLOWED_USERS").ok().as_deref());
        let clear_done_reaction = std::env::var("TELEGRAM_CLEAR_DONE_REACTION")
            .ok()
            .map(|s| {
                matches!(
                    s.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        Ok(Self {
            client,
            allowlist,
            clear_done_reaction,
            streams: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn parse_chat_id(channel_id: &str) -> Option<i64> {
        channel_id.parse::<i64>().ok()
    }

    fn parse_thread_id(thread: Option<&str>) -> Option<i64> {
        thread.and_then(|s| s.parse::<i64>().ok())
    }

    /// Edit a streaming message with HTML formatting and a plain-text fallback on `can't parse entities`. The plain fallback is derived from `html_body` via `dispatcher::html_to_plain` so the user sees readable prose (matching `send_text`'s fallback shape) rather than literal markdown / HTML markup. `message is not modified` is treated as success on both paths. Other failures are logged; token-bearing errors are already redacted at the BotClient layer.
    ///
    /// Empty / whitespace-only bodies are no-ops — Telegram rejects `editMessageText` with `400 message text is empty`, so we skip the call entirely and leave the previous content (the `…` placeholder, or the last successful edit) in place.
    async fn edit_with_fallback(
        client: &BotClient,
        chat_id: i64,
        message_id: i64,
        html_body: &str,
    ) {
        if html_body.trim().is_empty() {
            return;
        }
        match client
            .edit_message_text(chat_id, message_id, html_body, Some("HTML"), None)
            .await
        {
            Ok(_) => {}
            Err(e) if is_message_not_modified(&e) => {}
            Err(e) if is_parse_entities_error(&e) => {
                let plain = crate::dispatcher::html_to_plain(html_body);
                match client
                    .edit_message_text(chat_id, message_id, &plain, None, None)
                    .await
                {
                    Ok(_) => {}
                    Err(e2) if is_message_not_modified(&e2) => {}
                    Err(e2) => {
                        eprintln!("[telegram] stream edit (plain fallback) failed: {e2}");
                    }
                }
            }
            Err(e) => {
                eprintln!("[telegram] stream edit failed: {e}");
            }
        }
    }
}

#[async_trait]
impl SidecarAdapter for TelegramAdapter {
    fn capabilities(&self) -> Vec<String> {
        vec![
            "typing".into(),
            "reaction".into(),
            "interactive".into(),
            "thread".into(),
            "streaming".into(),
        ]
    }

    fn header_rules(&self) -> Vec<Value> {
        // The daemon's media-fetch hits api.telegram.org with the file URLs we hand it.
        // No auth header is required for `/file/bot<token>/...` URLs — the token is part of the path — so we don't need to declare any.
        Vec::new()
    }

    async fn on_send(
        &self,
        cmd: SendCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let Some(chat_id) = Self::parse_chat_id(&cmd.channel_id) else {
            return Err(format!("non-numeric channel_id: {}", cmd.channel_id).into());
        };
        let thread_id = Self::parse_thread_id(cmd.thread_id.as_deref());

        if let Some(content) = cmd.content {
            let tag = content
                .as_object()
                .and_then(|o| o.keys().next().cloned())
                .unwrap_or_else(|| "?".into());
            tg_trace!("on_send chat={chat_id} content={tag}");
            dispatch_content(&self.client, chat_id, &content, thread_id)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            return Ok(());
        }
        // Legacy text-only fallback.
        tg_trace!(
            "on_send chat={chat_id} content=Text(legacy) len={}",
            cmd.text.len()
        );
        send_text(&self.client, chat_id, &cmd.text, thread_id)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        Ok(())
    }

    async fn on_command(
        &self,
        cmd: Command,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match cmd {
            Command::Send(s) => self.on_send(s).await,
            Command::Typing(t) => {
                if let Some(chat_id) = Self::parse_chat_id(&t.channel_id) {
                    tg_trace!("on_command Typing chat={chat_id}");
                    let _ = self.client.send_chat_action(chat_id, "typing").await;
                }
                Ok(())
            }
            Command::Reaction(r) => {
                let Some(chat_id) = Self::parse_chat_id(&r.channel_id) else {
                    return Ok(());
                };
                let Ok(message_id) = r.message_id.parse::<i64>() else {
                    return Ok(());
                };
                tg_trace!(
                    "on_command Reaction chat={chat_id} msg={message_id} reaction={}",
                    r.reaction
                );
                let emojis = map_reaction(&r.reaction, self.clear_done_reaction);
                let reactions: Vec<Value> = emojis
                    .into_iter()
                    .map(|e| json!({"type": "emoji", "emoji": e}))
                    .collect();
                let _ = self
                    .client
                    .set_message_reaction(chat_id, message_id, reactions)
                    .await;
                Ok(())
            }
            Command::Interactive(i) => {
                let Some(chat_id) = Self::parse_chat_id(&i.channel_id) else {
                    return Ok(());
                };
                tg_trace!("on_command Interactive chat={chat_id}");
                let payload = serde_json::to_value(&i.message)?;
                let content_value = json!({ "Interactive": payload });
                dispatch_content(&self.client, chat_id, &content_value, None)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
                Ok(())
            }
            Command::StreamStart(s) => {
                let Some(chat_id) = Self::parse_chat_id(&s.channel_id) else {
                    return Ok(());
                };
                let thread_id = Self::parse_thread_id(s.thread_id.as_deref());
                tg_trace!(
                    "on_command StreamStart chat={chat_id} stream_id={}",
                    s.stream_id
                );
                // Send an empty placeholder so we have a message_id to edit later. Telegram edits a message by id alone, so we don't carry thread_id on the state.
                let res = self
                    .client
                    .send_message(chat_id, "…", Some("HTML"), thread_id, None)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
                let mut map = self.streams.lock().await;
                map.insert(
                    s.stream_id.clone(),
                    StreamState {
                        chat_id,
                        message_id: res.message_id,
                        buf: String::new(),
                        // `Instant::now() - 2s` panics if the system has been up less than 2 s (cold-boot container, embedded sidecar). saturating_sub returns `Instant::now()` in that case — the first delta will be throttled instead of firing immediately, which is fine.
                        last_edit: Instant::now()
                            .checked_sub(Duration::from_secs(2))
                            .unwrap_or_else(Instant::now),
                    },
                );
                Ok(())
            }
            Command::StreamDelta(d) => {
                let mut map = self.streams.lock().await;
                let Some(state) = map.get_mut(&d.stream_id) else {
                    return Ok(());
                };
                state.buf.push_str(&d.text);
                let elapsed = state.last_edit.elapsed();
                if elapsed >= Duration::from_millis(STREAM_EDIT_INTERVAL_MS) {
                    let chat_id = state.chat_id;
                    let message_id = state.message_id;
                    let body = crate::format::format_and_sanitize(&state.buf);
                    let buf_len = state.buf.len();
                    state.last_edit = Instant::now();
                    drop(map);
                    tg_trace!("StreamDelta edit chat={chat_id} msg={message_id} buf_len={buf_len}");
                    Self::edit_with_fallback(&self.client, chat_id, message_id, &body).await;
                }
                Ok(())
            }
            Command::StreamEnd(e) => {
                let mut map = self.streams.lock().await;
                let Some(state) = map.remove(&e.stream_id) else {
                    return Ok(());
                };
                tg_trace!(
                    "on_command StreamEnd chat={} msg={} buf_len={}",
                    state.chat_id,
                    state.message_id,
                    state.buf.len()
                );
                let body = crate::format::format_and_sanitize(&state.buf);
                let chat_id = state.chat_id;
                let message_id = state.message_id;
                drop(map);
                Self::edit_with_fallback(&self.client, chat_id, message_id, &body).await;
                Ok(())
            }
            // Unknown / forward-compat commands are silently tolerated.
            _ => Ok(()),
        }
    }

    async fn produce(&self, emit: EmitFn) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut offset: i64 = 0;
        let mut backoff = Duration::from_secs(1);
        loop {
            match self
                .client
                .get_updates(offset, DEFAULT_LONGPOLL_TIMEOUT_SECS, ALLOWED_UPDATES)
                .await
            {
                Ok(resp) => {
                    // Reset backoff on a successful round.
                    backoff = Duration::from_secs(1);
                    if !resp.result.is_empty() {
                        tg_trace!(
                            "getUpdates -> {} updates (next offset {})",
                            resp.result.len(),
                            offset
                        );
                    }
                    for upd in &resp.result {
                        offset = upd.update_id + 1;
                        let kind = if upd.message.is_some() {
                            "message"
                        } else if upd.edited_message.is_some() {
                            "edited_message"
                        } else if upd.callback_query.is_some() {
                            "callback_query"
                        } else if upd.poll_answer.is_some() {
                            "poll_answer"
                        } else {
                            "unknown"
                        };
                        // Access control — extract a sender for every update kind the adapter emits, including poll_answer (otherwise the allowlist would silently let any Telegram user vote in the bot's polls and have the PollAnswer event reach the agent).
                        let sender = if let Some(msg) = &upd.message {
                            Some(extract_sender(msg))
                        } else if let Some(msg) = &upd.edited_message {
                            Some(extract_sender(msg))
                        } else if let Some(cq) = &upd.callback_query {
                            cq.from.as_ref().map(sender_from_user)
                        } else if let Some(pa) = &upd.poll_answer {
                            pa.user.as_ref().map(sender_from_user)
                        } else {
                            None
                        };
                        if let Some(sender) = sender {
                            if !self
                                .allowlist
                                .permits(&sender.user_id, sender.username.as_deref())
                            {
                                tg_trace!(
                                    "update {} {kind} dropped by allowlist user={}",
                                    upd.update_id,
                                    sender.user_id
                                );
                                continue;
                            }
                        }
                        if let Some(event) = update_to_event(&self.client, upd).await {
                            tg_trace!("emit {kind} update_id={}", upd.update_id);
                            emit(event);
                        } else {
                            tg_trace!(
                                "update {} {kind} produced no event (unsupported variant)",
                                upd.update_id
                            );
                        }
                    }
                }
                Err(Error::Http(e)) if e.is_timeout() => {
                    // Long-poll timed out — that's normal, just loop.
                    backoff = Duration::from_secs(1);
                }
                Err(e) => {
                    eprintln!(
                        "[telegram] getUpdates error, backing off {:?}: {e}",
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(MAX_BACKOFF_SECS));
                }
            }
            // Tiny breather to let other tasks make progress between poll iterations.
            tokio::task::yield_now().await;
        }
    }
}
