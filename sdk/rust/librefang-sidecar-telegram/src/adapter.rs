//! The `TelegramAdapter` ties everything together: produce-side long-poll loop, on_send / on_command dispatch.

use crate::access::AllowList;
use crate::api::client::DEFAULT_LONGPOLL_TIMEOUT_SECS;
use crate::api::{BotClient, Error};
use crate::dispatcher::{
    dispatch_content, is_message_not_modified, is_parse_entities_error, send_text,
};
use crate::reaction::{map_reaction, DoneReactionPolicy};
use crate::translator::{extract_sender, sender_from_user, update_to_event, Sender};
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
const STREAM_STATE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ACTIVE_STREAMS: usize = 128;
const MAX_STREAM_BUFFER_BYTES: usize = 1024 * 1024;

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

/// Render a best-effort command failure for logging.
/// Debug-formats the error so embedded control characters (newlines, ANSI escapes) are escaped rather than able to forge extra log lines.
fn best_effort_command_error(operation: &str, error: &(impl std::fmt::Display + ?Sized)) -> String {
    let rendered = error.to_string();
    format!("[telegram] {operation} failed: {rendered:?}")
}

macro_rules! tg_trace {
    ($($arg:tt)*) => { trace(format_args!($($arg)*)) };
}

/// Apply the configured identity boundary to an extracted Telegram sender.
/// Missing identity is compatible only with an explicitly open allowlist; restricted deployments fail closed instead of relying on the translator to reject every sender-less update variant independently.
fn sender_passes_allowlist(allowlist: &AllowList, sender: Option<&Sender>) -> bool {
    match sender {
        Some(sender) => allowlist.permits(&sender.user_id, sender.username.as_deref()),
        None => allowlist.is_open(),
    }
}

/// Whether the streaming send path is advertised as a capability.
/// Reads `TELEGRAM_STREAMING` live on each `capabilities()` call and defaults to enabled.
/// `"0"` / `"false"` / `"no"` / `"off"` (case-insensitive, trimmed) disable it; everything else — including an unset variable — leaves it enabled.
///
/// When disabled the `"streaming"` capability is dropped, so the daemon routes every send through the plain (non-streaming) path.
/// That trades incremental updates for a single final message, avoiding the visible placeholder flash on fast / short LLM turns.
fn streaming_enabled() -> bool {
    match std::env::var("TELEGRAM_STREAMING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub struct TelegramAdapter {
    client: Arc<BotClient>,
    allowlist: AllowList,
    done_reaction_policy: DoneReactionPolicy,
    /// Per-stream state for `stream_start` / `stream_delta` / `stream_end`.
    /// Keyed by stream_id; tracks the message_id we are editing, the accumulated text, and the last-edit time so deltas can be throttled.
    streams: Arc<Mutex<HashMap<String, StreamState>>>,
}

struct StreamState {
    chat_id: i64,
    message_id: i64,
    // The topic/thread the placeholder was posted in, so the final
    // answer can be delivered as a fresh (notifying) message in the same thread.
    thread_id: Option<i64>,
    buf: String,
    last_edit: Instant,
    last_activity: Instant,
}

fn prune_stale_streams(streams: &mut HashMap<String, StreamState>, now: Instant) -> usize {
    let before = streams.len();
    streams
        .retain(|_, state| now.saturating_duration_since(state.last_activity) < STREAM_STATE_TTL);
    before - streams.len()
}

fn reserve_stream_slot(streams: &mut HashMap<String, StreamState>, stream_id: &str) {
    if streams.contains_key(stream_id) {
        return;
    }
    while streams.len() >= MAX_ACTIVE_STREAMS {
        let Some(oldest) = streams
            .iter()
            .min_by_key(|(_, state)| state.last_activity)
            .map(|(stream_id, _)| stream_id.clone())
        else {
            break;
        };
        streams.remove(&oldest);
    }
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
        let done_reaction_policy = if std::env::var("TELEGRAM_CLEAR_DONE_REACTION")
            .ok()
            .map(|s| {
                matches!(
                    s.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
        {
            DoneReactionPolicy::Suppress
        } else {
            DoneReactionPolicy::Emit
        };
        Ok(Self {
            client,
            allowlist,
            done_reaction_policy,
            streams: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn parse_chat_id(channel_id: &str) -> Option<i64> {
        channel_id.parse::<i64>().ok()
    }

    fn parse_thread_id(thread: Option<&str>) -> Result<Option<i64>, String> {
        match thread {
            Some(value) => value
                .parse::<i64>()
                .map(Some)
                .map_err(|_| format!("non-numeric thread_id: {value}")),
            None => Ok(None),
        }
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

    /// Deliver the final streaming answer as a *fresh* message instead
    /// of an edit of the `"…"` placeholder. Telegram edits never fire a push
    /// notification, so a user who backgrounded the client after the placeholder
    /// ping received nothing when the answer landed. A new message notifies
    /// reliably; the placeholder is then deleted. Mirrors `edit_with_fallback`'s
    /// HTML→plain fallback, and if the fresh send fails outright it falls back to
    /// editing the placeholder in place (answer still visible, just no push).
    async fn finalize_as_new_message(
        client: &BotClient,
        chat_id: i64,
        placeholder_id: i64,
        thread_id: Option<i64>,
        html_body: &str,
    ) {
        if html_body.trim().is_empty() {
            // No answer to deliver — leave the placeholder as-is (matches the
            // empty-body early return in `edit_with_fallback`).
            return;
        }
        let sent = match client
            .send_message(chat_id, html_body, Some("HTML"), thread_id, None)
            .await
        {
            Ok(_) => true,
            Err(e) if is_parse_entities_error(&e) => {
                let plain = crate::dispatcher::html_to_plain(html_body);
                match client
                    .send_message(chat_id, &plain, None, thread_id, None)
                    .await
                {
                    Ok(_) => true,
                    Err(e2) => {
                        eprintln!("[telegram] stream finalize (plain fallback) failed: {e2}");
                        false
                    }
                }
            }
            Err(e) => {
                eprintln!("[telegram] stream finalize failed: {e}");
                false
            }
        };
        if sent {
            // Drop the "…" placeholder now the real answer has landed. Deletion
            // can fail (message too old / no rights) — harmless, the new message
            // already notified; the placeholder just lingers.
            if let Err(e) = client.delete_message(chat_id, placeholder_id).await {
                eprintln!("[telegram] stream placeholder delete failed: {e}");
            }
        } else {
            // Fresh send failed outright — fall back to the old edit-in-place so
            // the answer is still visible (no notification, but not lost).
            Self::edit_with_fallback(client, chat_id, placeholder_id, html_body).await;
        }
    }
}

#[async_trait]
impl SidecarAdapter for TelegramAdapter {
    fn capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = vec![
            "typing".into(),
            "reaction".into(),
            "interactive".into(),
            "thread".into(),
        ];
        // `"streaming"` is opt-out via `TELEGRAM_STREAMING`; appended last so the base capability order stays deterministic.
        if streaming_enabled() {
            caps.push("streaming".into());
        }
        caps
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
        let thread_id = Self::parse_thread_id(cmd.thread_id.as_deref())
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;

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
                let Some(chat_id) = Self::parse_chat_id(&t.channel_id) else {
                    return Err(format!("non-numeric channel_id: {}", t.channel_id).into());
                };
                tg_trace!("on_command Typing chat={chat_id}");
                if let Err(error) = self.client.send_chat_action(chat_id, "typing").await {
                    eprintln!("{}", best_effort_command_error("typing action", &error));
                }
                Ok(())
            }
            Command::Reaction(r) => {
                let Some(chat_id) = Self::parse_chat_id(&r.channel_id) else {
                    return Err(format!("non-numeric channel_id: {}", r.channel_id).into());
                };
                let Ok(message_id) = r.message_id.parse::<i64>() else {
                    return Err(format!("non-numeric message_id: {}", r.message_id).into());
                };
                tg_trace!(
                    "on_command Reaction chat={chat_id} msg={message_id} reaction={}",
                    r.reaction
                );
                let emojis = map_reaction(&r.reaction, self.done_reaction_policy);
                let reactions: Vec<Value> = emojis
                    .into_iter()
                    .map(|e| json!({"type": "emoji", "emoji": e}))
                    .collect();
                if let Err(error) = self
                    .client
                    .set_message_reaction(chat_id, message_id, reactions)
                    .await
                {
                    eprintln!("{}", best_effort_command_error("reaction update", &error));
                }
                Ok(())
            }
            Command::Interactive(i) => {
                let Some(chat_id) = Self::parse_chat_id(&i.channel_id) else {
                    return Err(format!("non-numeric channel_id: {}", i.channel_id).into());
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
                    return Err(format!("non-numeric channel_id: {}", s.channel_id).into());
                };
                let thread_id = Self::parse_thread_id(s.thread_id.as_deref()).map_err(
                    |error| -> Box<dyn std::error::Error + Send + Sync> { error.into() },
                )?;
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
                let now = Instant::now();
                let removed = prune_stale_streams(&mut map, now);
                reserve_stream_slot(&mut map, &s.stream_id);
                if removed > 0 {
                    tg_trace!("evicted {removed} stale stream state(s)");
                }
                map.insert(
                    s.stream_id.clone(),
                    StreamState {
                        chat_id,
                        message_id: res.message_id,
                        thread_id,
                        buf: String::new(),
                        // `Instant::now() - 2s` panics if the system has been up less than 2 s (cold-boot container, embedded sidecar). saturating_sub returns `Instant::now()` in that case — the first delta will be throttled instead of firing immediately, which is fine.
                        last_edit: Instant::now()
                            .checked_sub(Duration::from_secs(2))
                            .unwrap_or_else(Instant::now),
                        last_activity: now,
                    },
                );
                Ok(())
            }
            Command::StreamDelta(d) => {
                let mut map = self.streams.lock().await;
                let Some(state) = map.get_mut(&d.stream_id) else {
                    eprintln!(
                        "[telegram] StreamDelta for unknown stream_id={:?}, dropped",
                        d.stream_id
                    );
                    return Ok(());
                };
                if state.buf.len().saturating_add(d.text.len()) > MAX_STREAM_BUFFER_BYTES {
                    map.remove(&d.stream_id);
                    return Err(format!(
                        "stream {} exceeded the {} byte buffer limit",
                        d.stream_id, MAX_STREAM_BUFFER_BYTES
                    )
                    .into());
                }
                state.buf.push_str(&d.text);
                state.last_activity = Instant::now();
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
                    eprintln!(
                        "[telegram] StreamEnd for unknown stream_id={:?}, dropped",
                        e.stream_id
                    );
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
                let thread_id = state.thread_id;
                drop(map);
                Self::finalize_as_new_message(&self.client, chat_id, message_id, thread_id, &body)
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
            let removed = {
                let mut streams = self.streams.lock().await;
                prune_stale_streams(&mut streams, Instant::now())
            };
            if removed > 0 {
                tg_trace!("evicted {removed} stale stream state(s)");
            }
            match self
                .client
                .get_updates(offset, DEFAULT_LONGPOLL_TIMEOUT_SECS, ALLOWED_UPDATES)
                .await
            {
                Ok(resp) => {
                    // Reset backoff on a successful round.
                    backoff = Duration::from_secs(1);
                    let updates = resp.result.as_deref().unwrap_or_default();
                    if !updates.is_empty() {
                        tg_trace!(
                            "getUpdates -> {} updates (next offset {})",
                            updates.len(),
                            offset
                        );
                    }
                    for upd in updates {
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
                        if !sender_passes_allowlist(&self.allowlist, sender.as_ref()) {
                            if let Some(sender) = &sender {
                                tg_trace!(
                                    "update {} {kind} dropped by allowlist user={}",
                                    upd.update_id,
                                    sender.user_id
                                );
                            } else {
                                tg_trace!(
                                    "update {} {kind} dropped by allowlist (missing sender)",
                                    upd.update_id
                                );
                            }
                            continue;
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

    async fn on_shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut streams = self.streams.lock().await;
        let removed = streams.len();
        streams.clear();
        if removed > 0 {
            tg_trace!("cleared {removed} stream state(s) during shutdown");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn best_effort_command_errors_are_single_line_and_control_escaped() {
        let error = Error::Other("upstream\r\nforged\u{1b}[31m".into());
        let message = best_effort_command_error("typing action", &error);

        assert_eq!(
            message,
            "[telegram] typing action failed: \"upstream\\r\\nforged\\u{1b}[31m\""
        );
        assert!(!message.contains('\r'));
        assert!(!message.contains('\n'));
        assert!(!message.contains('\u{1b}'));
    }

    #[test]
    #[ignore = "subprocess fixture for stderr capture"]
    fn best_effort_command_failure_fixture() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            let mut request_lines = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request).expect("read fixture request");
                let request = String::from_utf8_lossy(&request[..read]);
                request_lines.push(request.lines().next().unwrap_or_default().to_string());

                let body = b"upstream\r\nforged\x1b[31m";
                write!(
                    stream,
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .expect("write fixture headers");
                stream.write_all(body).expect("write fixture body");
            }
            request_lines
        });

        let root = format!("http://{address}");
        let client =
            BotClient::with_roots("fixture-token", &root, &root).expect("construct fixture client");
        let adapter = TelegramAdapter {
            client: Arc::new(client),
            allowlist: AllowList::from_env_value(None),
            done_reaction_policy: DoneReactionPolicy::Emit,
            streams: Arc::new(Mutex::new(HashMap::new())),
        };
        let runtime = tokio::runtime::Runtime::new().expect("fixture runtime");
        runtime.block_on(async {
            adapter
                .on_command(Command::Typing(librefang_sidecar::TypingCmd {
                    channel_id: "42".into(),
                }))
                .await
                .expect("typing remains best effort");
            adapter
                .on_command(Command::Reaction(librefang_sidecar::Reaction {
                    channel_id: "42".into(),
                    message_id: "7".into(),
                    reaction: "👍".into(),
                    ..Default::default()
                }))
                .await
                .expect("reaction remains best effort");
            adapter
                .on_command(Command::StreamDelta(librefang_sidecar::StreamDelta {
                    stream_id: "missing\r\nforged\u{1b}[31m".into(),
                    text: "lost".into(),
                }))
                .await
                .expect("unknown stream delta remains best effort");
        });

        let request_lines = server.join().expect("fixture server thread");
        assert!(request_lines[0].contains("/sendChatAction"));
        assert!(request_lines[1].contains("/setMessageReaction"));
    }

    #[test]
    fn best_effort_commands_log_real_failures_and_stay_successful() {
        let output =
            std::process::Command::new(std::env::current_exe().expect("current test binary"))
                .args([
                    "--exact",
                    "adapter::tests::best_effort_command_failure_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .output()
                .expect("run subprocess fixture");
        assert!(output.status.success(), "fixture failed: {output:?}");

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(combined.contains("typing action failed"));
        assert!(combined.contains("reaction update failed"));
        assert!(combined.contains(
            "StreamDelta for unknown stream_id=\"missing\\r\\nforged\\u{1b}[31m\", dropped"
        ));
        assert!(combined.contains("upstream\\r\\nforged\\u{1b}[31m"));
        assert!(!combined.contains("upstream\r\nforged"));
    }

    #[tokio::test]
    async fn commands_reject_non_numeric_channel_ids_consistently() {
        let adapter = test_adapter();
        let invalid_channel = "telegram:not-a-number".to_string();
        let commands = [
            Command::Typing(librefang_sidecar::TypingCmd {
                channel_id: invalid_channel.clone(),
            }),
            Command::Reaction(librefang_sidecar::Reaction {
                channel_id: invalid_channel.clone(),
                message_id: "1".into(),
                reaction: "👍".into(),
                ..Default::default()
            }),
            Command::Interactive(librefang_sidecar::Interactive {
                channel_id: invalid_channel.clone(),
                message: Default::default(),
            }),
            Command::StreamStart(librefang_sidecar::StreamStart {
                channel_id: invalid_channel,
                stream_id: "stream-1".into(),
                thread_id: None,
            }),
        ];

        for command in commands {
            let error = adapter
                .on_command(command)
                .await
                .expect_err("invalid channel id must fail");
            assert!(error.to_string().contains("non-numeric channel_id"));
        }

        let error = adapter
            .on_command(Command::Reaction(librefang_sidecar::Reaction {
                channel_id: "42".into(),
                message_id: "not-a-number".into(),
                reaction: "👍".into(),
                ..Default::default()
            }))
            .await
            .expect_err("invalid reaction message id must fail");
        assert!(error.to_string().contains("non-numeric message_id"));

        let error = adapter
            .on_send(SendCommand {
                channel_id: "42".into(),
                thread_id: Some("not-a-number".into()),
                ..Default::default()
            })
            .await
            .expect_err("invalid send thread id must fail before the request");
        assert!(error.to_string().contains("non-numeric thread_id"));

        let error = adapter
            .on_command(Command::StreamStart(librefang_sidecar::StreamStart {
                channel_id: "42".into(),
                stream_id: "stream-with-bad-thread".into(),
                thread_id: Some("not-a-number".into()),
            }))
            .await
            .expect_err("invalid stream thread id must fail before the request");
        assert!(error.to_string().contains("non-numeric thread_id"));
    }

    #[test]
    fn missing_sender_is_denied_unless_allowlist_is_open() {
        let restricted = AllowList::from_env_value(Some("12345"));
        assert!(!sender_passes_allowlist(&restricted, None));

        let open = AllowList::from_env_value(None);
        assert!(sender_passes_allowlist(&open, None));

        let allowed = crate::translator::Sender {
            user_id: "12345".into(),
            name: "Allowed".into(),
            username: None,
        };
        assert!(sender_passes_allowlist(&restricted, Some(&allowed)));
    }

    fn stream_state(last_activity: Instant) -> StreamState {
        StreamState {
            chat_id: 1,
            message_id: 2,
            thread_id: None,
            buf: String::new(),
            last_edit: last_activity,
            last_activity,
        }
    }

    #[tokio::test]
    async fn stale_and_excess_streams_are_evicted_and_shutdown_clears_the_rest() {
        let stale = Instant::now();
        let now = stale + STREAM_STATE_TTL + Duration::from_secs(1);
        let mut streams = HashMap::new();
        streams.insert("stale".to_string(), stream_state(stale));
        streams.insert("fresh".to_string(), stream_state(now));

        prune_stale_streams(&mut streams, now);
        assert!(!streams.contains_key("stale"));
        assert!(streams.contains_key("fresh"));

        for index in 0..MAX_ACTIVE_STREAMS {
            streams.insert(format!("stream-{index}"), stream_state(now));
        }
        reserve_stream_slot(&mut streams, "new-stream");
        assert!(streams.len() < MAX_ACTIVE_STREAMS);

        let adapter = test_adapter();
        let mut at_limit = stream_state(now);
        at_limit.buf = "x".repeat(MAX_STREAM_BUFFER_BYTES);
        adapter
            .streams
            .lock()
            .await
            .insert("overflow".to_string(), at_limit);
        let error = adapter
            .on_command(Command::StreamDelta(librefang_sidecar::StreamDelta {
                stream_id: "overflow".to_string(),
                text: "x".to_string(),
            }))
            .await
            .expect_err("an oversized stream must fail closed");
        assert!(error.to_string().contains("byte buffer limit"));
        assert!(!adapter.streams.lock().await.contains_key("overflow"));

        *adapter.streams.lock().await = streams;
        adapter.on_shutdown().await.expect("shutdown cleanup");
        assert!(adapter.streams.lock().await.is_empty());
    }

    /// Restores `TELEGRAM_STREAMING` to its pre-test value on drop, so the mutation stays contained even if an assertion panics mid-test.
    struct EnvGuard(Option<String>);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("TELEGRAM_STREAMING", v),
                None => std::env::remove_var("TELEGRAM_STREAMING"),
            }
        }
    }

    fn set_streaming(v: Option<&str>) {
        match v {
            Some(v) => std::env::set_var("TELEGRAM_STREAMING", v),
            None => std::env::remove_var("TELEGRAM_STREAMING"),
        }
    }

    fn test_adapter() -> TelegramAdapter {
        TelegramAdapter {
            client: Arc::new(BotClient::new("123456:test-token").expect("dummy client")),
            allowlist: AllowList::from_env_value(None),
            done_reaction_policy: DoneReactionPolicy::Emit,
            streams: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // `TELEGRAM_STREAMING` is process-global state; this is the only test in the crate that touches it and every case runs sequentially in one function, so nothing races with a parallel test.
    // The guard restores the original value.
    #[test]
    fn streaming_enabled_parsing_and_capability_gate() {
        let _guard = EnvGuard(std::env::var("TELEGRAM_STREAMING").ok());

        // Unset defaults to enabled.
        set_streaming(None);
        assert!(streaming_enabled(), "unset should default to enabled");

        // Explicit truthy values stay enabled.
        set_streaming(Some("true"));
        assert!(streaming_enabled());
        set_streaming(Some("1"));
        assert!(streaming_enabled());

        // Disable tokens, case-insensitive and trimmed.
        for v in ["0", "false", "no", "off", " OFF ", "False", "Off"] {
            set_streaming(Some(v));
            assert!(!streaming_enabled(), "{v:?} should disable streaming");
        }

        // Unknown values fall back to enabled.
        set_streaming(Some("maybe"));
        assert!(streaming_enabled(), "unknown value should stay enabled");

        let adapter = test_adapter();

        // Disabled -> "streaming" dropped, the four base capabilities intact.
        set_streaming(Some("0"));
        let caps = adapter.capabilities();
        assert!(
            !caps.iter().any(|c| c == "streaming"),
            "streaming must be dropped when disabled"
        );
        for base in ["typing", "reaction", "interactive", "thread"] {
            assert!(
                caps.iter().any(|c| c == base),
                "missing base capability {base}"
            );
        }

        // Enabled -> "streaming" present again.
        set_streaming(Some("true"));
        assert!(
            adapter.capabilities().iter().any(|c| c == "streaming"),
            "streaming must be advertised when enabled"
        );
    }
}
