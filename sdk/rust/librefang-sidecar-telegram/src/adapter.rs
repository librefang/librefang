//! The `TelegramAdapter` ties everything together: produce-side long-poll loop, on_send / on_command dispatch.

use crate::access::AllowList;
use crate::api::client::{DEFAULT_LONGPOLL_TIMEOUT_SECS, LONGPOLL_CLIENT_TIMEOUT_SECS};
use crate::api::{BotClient, Error};
use crate::dispatcher::{dispatch_content, send_text};
use crate::reaction::map_reaction;
use crate::translator::{extract_sender, update_to_event};
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
            dispatch_content(&self.client, chat_id, &content, thread_id)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            return Ok(());
        }
        // Legacy text-only fallback.
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
                // Send an empty placeholder so we have a message_id to edit later.
                let res = self
                    .client
                    .send_message(chat_id, "…", Some("HTML"), thread_id, None)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
                let mut map = self.streams.lock().await;
                let _ = thread_id; // captured into the placeholder message above; no need to keep it for edits (Telegram edits a message by id alone)
                map.insert(
                    s.stream_id.clone(),
                    StreamState {
                        chat_id,
                        message_id: res.message_id,
                        buf: String::new(),
                        last_edit: Instant::now() - Duration::from_secs(2),
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
                    state.last_edit = Instant::now();
                    drop(map);
                    let _ = self
                        .client
                        .edit_message_text(chat_id, message_id, &body, Some("HTML"), None)
                        .await;
                }
                Ok(())
            }
            Command::StreamEnd(e) => {
                let mut map = self.streams.lock().await;
                let Some(state) = map.remove(&e.stream_id) else {
                    return Ok(());
                };
                let body = crate::format::format_and_sanitize(&state.buf);
                drop(map);
                let _ = self
                    .client
                    .edit_message_text(state.chat_id, state.message_id, &body, Some("HTML"), None)
                    .await;
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
                    for upd in &resp.result {
                        offset = upd.update_id + 1;
                        // Access control — skip updates from disallowed senders.
                        let sender = if let Some(msg) = &upd.message {
                            Some(extract_sender(msg))
                        } else if let Some(msg) = &upd.edited_message {
                            Some(extract_sender(msg))
                        } else if let Some(cq) = &upd.callback_query {
                            cq.from.as_ref().map(|u| crate::translator::Sender {
                                user_id: u.id.to_string(),
                                name: u.first_name.clone(),
                                username: u.username.clone(),
                            })
                        } else {
                            None
                        };
                        if let Some(sender) = sender {
                            if !self
                                .allowlist
                                .permits(&sender.user_id, sender.username.as_deref())
                            {
                                continue;
                            }
                        }
                        if let Some(event) = update_to_event(&self.client, upd).await {
                            emit(event);
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
            let _ = LONGPOLL_CLIENT_TIMEOUT_SECS;
        }
    }
}
