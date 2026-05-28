//! Inbound translation: Telegram `Update` → LibreFang sidecar message-event Value.
//!
//! Mirrors the Python adapter's `_update_to_event` / `_extract_content` / `_sender` / `_apply_reply` / `_callback_to_event` / `_poll_answer_to_event`.
//! All file_id values that need a public URL go through `BotClient::get_file` so the daemon's media-fetch path can pull them with the Authorization header rule the adapter declares in its `ready` event.

use crate::api::types::{CallbackQuery, Chat, Message, PollAnswer, Update, User};
use crate::api::BotClient;
use librefang_sidecar::MessageBuilder;
use serde_json::{json, Value};

pub struct Sender {
    pub user_id: String,
    pub name: String,
    pub username: Option<String>,
}

/// Prefer `message.from`; fall back to `message.sender_chat` (channel posts) with sensible defaults.
pub fn extract_sender(msg: &Message) -> Sender {
    if let Some(user) = &msg.from {
        let mut name = user.first_name.clone();
        if let Some(last) = &user.last_name {
            if !last.is_empty() {
                name.push(' ');
                name.push_str(last);
            }
        }
        if name.is_empty() {
            name = "Unknown".into();
        }
        return Sender {
            user_id: user.id.to_string(),
            name,
            username: user.username.clone(),
        };
    }
    if let Some(chat) = &msg.sender_chat {
        return sender_from_chat(chat);
    }
    Sender {
        user_id: "0".into(),
        name: "Unknown".into(),
        username: None,
    }
}

fn sender_from_chat(chat: &Chat) -> Sender {
    let name = chat
        .title
        .clone()
        .or_else(|| chat.first_name.clone())
        .unwrap_or_else(|| "Unknown Channel".into());
    Sender {
        user_id: chat.id.to_string(),
        name,
        username: chat.username.clone(),
    }
}

/// Parse a leading bot-command entity (e.g. `/start arg1 arg2`).
/// Returns `(name, args)` when the message text starts with a `bot_command` at offset 0, `None` otherwise.
///
/// Bot API documents `MessageEntity.length` as a UTF-16 code-unit count, so to slice `text` correctly we walk Unicode scalars accumulating their `len_utf16()` until we've consumed `cmd_len` code units. This is correct for ASCII (the only kind Telegram currently emits for bot_command) AND survives any future extension to non-BMP characters.
///
/// **Deliberate divergence from the Python reference adapter**: Python uses `txt.split(" ", 1)` and ignores `entity.length`, so `/help:foo` yields name=`"help:foo"` and args=`[]` (bug — the `:foo` folds into the command name). This Rust impl honours the Bot API entity boundary instead, so the same input yields name=`"help"` and args=`[":foo"]`. The supervisor's command router benefits from the more accurate split; downstream code that grepped logs for the Python-shaped buggy command name will need to update its pattern.
fn parse_command(msg: &Message) -> Option<(String, Vec<String>)> {
    let text = msg.text.as_deref()?;
    let first = msg.entities.first()?;
    if first.entity_type != "bot_command" || first.offset != 0 {
        return None;
    }
    let cmd_len_u16 = usize::try_from(first.length).ok()?;
    // Defensive clamp: a misbehaving proxy could report a length larger than the actual text. Without the clamp the loop walks to end-of-text and `head` ends up containing the entire message including spaces — `bare` would then be `"foo bar baz"` instead of `"foo"`.
    if cmd_len_u16 > text.encode_utf16().count() {
        return None;
    }
    let mut units = 0usize;
    let mut byte_end = 0usize;
    for ch in text.chars() {
        if units >= cmd_len_u16 {
            break;
        }
        units += ch.len_utf16();
        byte_end += ch.len_utf8();
    }
    let head = &text[..byte_end];
    let rest = &text[byte_end..];
    let cmd_raw = head.strip_prefix('/')?;
    let bare = match cmd_raw.find('@') {
        Some(at) => &cmd_raw[..at],
        None => cmd_raw,
    };
    if bare.is_empty() {
        return None;
    }
    let args: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
    Some((bare.to_string(), args))
}

/// Build a text placeholder used when getFile fails so the user's caption (often the actual question) survives even though the media URL is unavailable.
///
/// Label / shape mirrors the Python adapter so cross-language grep over conversation logs sees the same strings: `[Photo received: cap]`, `[Document received: cap]`, `[Audio received, Ns: cap]`, `[Voice message, Ns]` (Python form — no caption is appended), `[Animation received, Ns: cap]`, `[Video received, Ns: cap]`, `[Video note, Ns]`. Duration-bearing variants always include `Ns`; the caption suffix is only added when non-empty AND the Python adapter includes one for that media type.
fn media_placeholder(label: &str, duration_secs: Option<u32>, caption: Option<&str>) -> TgContent {
    let cap = caption
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| format!(": {c}"))
        .unwrap_or_default();
    let dur = duration_secs.map(|d| format!(", {d}s")).unwrap_or_default();
    TgContent::Text(format!("[{label}{dur}{cap}]"))
}

/// Best-effort file-id → public URL. Returns None on lookup failure (the caller falls back to a text placeholder).
pub async fn file_url(client: &BotClient, file_id: &str) -> Option<String> {
    match client.get_file(file_id).await {
        Ok(res) => res.file_path.map(|p| client.file_url(&p)),
        Err(_) => None,
    }
}

/// Map a Telegram `Message` to a single `TgContent`. Returns None for unsupported variants (the caller drops the message).
///
/// When `getFile` fails on a media payload (transient CDN blip, expired file_id), we fall back to a `[Kind received: <caption>]` text placeholder rather than dropping the whole update — the user's caption is often the actual question.
pub async fn extract_content(client: &BotClient, msg: &Message) -> Option<TgContent> {
    if msg.text.is_some() {
        if let Some((name, args)) = parse_command(msg) {
            return Some(TgContent::Command { name, args });
        }
        return msg.text.clone().map(TgContent::Text);
    }
    if let Some(photos) = msg.photo.last() {
        let caption = msg.caption.clone();
        return Some(match file_url(client, &photos.file_id).await {
            Some(url) => TgContent::Image {
                url,
                caption,
                mime_type: Some("image/jpeg".into()),
            },
            None => media_placeholder("Photo received", None, caption.as_deref()),
        });
    }
    if let Some(doc) = &msg.document {
        let filename = doc.file_name.clone().unwrap_or_else(|| "document".into());
        return Some(match file_url(client, &doc.file_id).await {
            Some(url) => TgContent::File { url, filename },
            // Python parity: `[Document received: {filename}]`. The user's caption — often more useful — is intentionally NOT substituted in here so cross-language log grep matches.
            None => media_placeholder("Document received", None, Some(&filename)),
        });
    }
    if let Some(audio) = &msg.audio {
        return Some(match file_url(client, &audio.file_id).await {
            Some(url) => TgContent::Audio {
                url,
                caption: msg.caption.clone(),
                duration_seconds: audio.duration,
                title: audio.title.clone(),
                performer: audio.performer.clone(),
            },
            None => media_placeholder(
                "Audio received",
                Some(audio.duration),
                msg.caption.as_deref(),
            ),
        });
    }
    if let Some(voice) = &msg.voice {
        return Some(match file_url(client, &voice.file_id).await {
            Some(url) => TgContent::Voice {
                url,
                caption: msg.caption.clone(),
                duration_seconds: voice.duration,
            },
            // Python's `[Voice message, Ns]` does NOT append the caption — match for parity.
            None => media_placeholder("Voice message", Some(voice.duration), None),
        });
    }
    if let Some(anim) = &msg.animation {
        return Some(match file_url(client, &anim.file_id).await {
            Some(url) => TgContent::Animation {
                url,
                caption: msg.caption.clone(),
                duration_seconds: anim.duration,
            },
            None => media_placeholder(
                "Animation received",
                Some(anim.duration),
                msg.caption.as_deref(),
            ),
        });
    }
    if let Some(video) = &msg.video {
        return Some(match file_url(client, &video.file_id).await {
            Some(url) => TgContent::Video {
                url,
                caption: msg.caption.clone(),
                duration_seconds: video.duration,
                filename: video.file_name.clone(),
            },
            None => media_placeholder(
                "Video received",
                Some(video.duration),
                msg.caption.as_deref(),
            ),
        });
    }
    if let Some(vn) = &msg.video_note {
        return Some(match file_url(client, &vn.file_id).await {
            Some(url) => TgContent::Video {
                url,
                caption: None,
                duration_seconds: vn.duration,
                filename: None,
            },
            // Python's `[Video note, Ns]` — note the lowercase 'note' and no caption.
            None => media_placeholder("Video note", Some(vn.duration), None),
        });
    }
    if let Some(loc) = &msg.location {
        return Some(TgContent::Location {
            lat: loc.latitude,
            lon: loc.longitude,
        });
    }
    if let Some(sticker) = &msg.sticker {
        return Some(TgContent::Sticker {
            file_id: sticker.file_id.clone(),
        });
    }
    if let Some(c) = &msg.contact {
        let mut s = format!("Contact: {}", c.first_name);
        if let Some(l) = &c.last_name {
            s.push(' ');
            s.push_str(l);
        }
        s.push_str(&format!(" ({})", c.phone_number));
        return Some(TgContent::Text(s));
    }
    None
}

/// Reply context: prefix `[Replying to <sender>: "..."]` to a text-shaped TgContent. If the replied-to message is itself a photo AND the current content is plain Text, the function upgrades it to an Image carrying the replied photo's URL — without this the agent never sees the photo the user is reacting to. Mirrors the Python adapter's `_apply_reply` behaviour.
pub async fn apply_reply(client: &BotClient, content: TgContent, msg: &Message) -> TgContent {
    let Some(reply) = msg.reply_to_message.as_ref() else {
        return content;
    };
    let replier = reply
        .from
        .as_ref()
        .map(|u| u.first_name.clone())
        .unwrap_or_else(|| "someone".into());
    let body = reply
        .text
        .as_deref()
        .or(reply.caption.as_deref())
        .unwrap_or("");
    let trimmed = truncate_bytes(body, 200);
    let prefix = format!("[Replying to {replier}: \"{trimmed}\"]\n");
    // Photo-reply upgrade: if the user replied to a photo with text, fetch the photo's URL and turn the inbound into an Image so the agent can actually see what's being replied to.
    if matches!(&content, TgContent::Text(_)) {
        if let Some(photo) = reply.photo.last() {
            if let Some(url) = file_url(client, &photo.file_id).await {
                let TgContent::Text(t) = content else {
                    unreachable!("matched above")
                };
                return TgContent::Image {
                    url,
                    caption: Some(format!("{prefix}{t}")),
                    mime_type: Some("image/jpeg".into()),
                };
            }
        }
    }
    match content {
        TgContent::Text(t) => TgContent::Text(format!("{prefix}{t}")),
        TgContent::Image {
            url,
            caption,
            mime_type,
        } => TgContent::Image {
            url,
            caption: Some(match caption {
                Some(c) => format!("{prefix}{c}"),
                None => prefix,
            }),
            mime_type,
        },
        other => other,
    }
}

fn truncate_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn build_metadata(msg: &Message, sender: &Sender, edited: bool) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("chat_id".into(), json!(msg.chat.id.to_string()));
    m.insert("platform".into(), json!("telegram"));
    m.insert("message_id".into(), json!(msg.message_id));
    if let Some(t) = msg.message_thread_id {
        m.insert("thread_id".into(), json!(t.to_string()));
    }
    if let Some(uname) = &sender.username {
        m.insert("sender_username".into(), json!(uname));
    }
    m.insert("sender_user_id".into(), json!(sender.user_id.clone()));
    // edited_message reuses the original message_id; without `edited: true` (and `edit_date` when Telegram provides it) the supervisor cannot distinguish an edit from a fresh turn.
    if edited {
        m.insert("edited".into(), json!(true));
        if let Some(ts) = msg.edit_date {
            m.insert("edit_date".into(), json!(ts));
        }
    }
    m
}

/// Build a message-event Value from a Telegram `Message`. `edited=true` for `update.edited_message`.
pub async fn message_event(client: &BotClient, msg: &Message, edited: bool) -> Option<Value> {
    // Defensive: `Chat.id` defaults to 0 if the deserialiser couldn't find it (the struct carries `#[serde(default)]` so missing fields don't fail the parse). Routing every malformed Update to chat 0 would silently merge them into a synthetic "chat 0" session — drop instead.
    if msg.chat.id == 0 {
        return None;
    }
    let content = extract_content(client, msg).await?;
    let content = apply_reply(client, content, msg).await;
    let sender = extract_sender(msg);
    let chat_id = msg.chat.id.to_string();
    // `channel` posts have separate semantics (no sender user, broadcast-only); treat them as DM-like for routing, matching the Python adapter.
    let is_group = matches!(msg.chat.chat_type.as_str(), "group" | "supergroup");
    let metadata = build_metadata(msg, &sender, edited);
    let mut builder = MessageBuilder::new(chat_id.clone(), sender.name.clone())
        .content(content_to_value(&content))
        .channel_id(chat_id)
        .platform("telegram")
        .is_group(is_group)
        .message_id(msg.message_id.to_string())
        .metadata(metadata);
    if let Some(uname) = sender.username {
        builder = builder.username(uname);
    }
    if let Some(t) = msg.message_thread_id {
        builder = builder.thread_id(t.to_string());
    }
    Some(builder.build())
}

/// Convert a `TgContent` enum into the SDK's wire-shape JSON value.
pub fn content_to_value(c: &TgContent) -> Value {
    match c {
        TgContent::Text(s) => librefang_sidecar::protocol::Content::text(s.clone()),
        TgContent::Image {
            url,
            caption,
            mime_type,
        } => librefang_sidecar::protocol::Content::image(
            url.clone(),
            caption.clone(),
            mime_type.clone(),
        ),
        TgContent::File { url, filename } => {
            librefang_sidecar::protocol::Content::file(url.clone(), filename.clone())
        }
        TgContent::Voice {
            url,
            caption,
            duration_seconds,
        } => librefang_sidecar::protocol::Content::voice(
            url.clone(),
            caption.clone(),
            *duration_seconds,
        ),
        TgContent::Video {
            url,
            caption,
            duration_seconds,
            filename,
        } => librefang_sidecar::protocol::Content::video(
            url.clone(),
            caption.clone(),
            *duration_seconds,
            filename.clone(),
        ),
        TgContent::Audio {
            url,
            caption,
            duration_seconds,
            title,
            performer,
        } => librefang_sidecar::protocol::Content::audio(
            url.clone(),
            caption.clone(),
            *duration_seconds,
            title.clone(),
            performer.clone(),
        ),
        TgContent::Animation {
            url,
            caption,
            duration_seconds,
        } => librefang_sidecar::protocol::Content::animation(
            url.clone(),
            caption.clone(),
            *duration_seconds,
        ),
        TgContent::Sticker { file_id } => {
            librefang_sidecar::protocol::Content::sticker(file_id.clone())
        }
        TgContent::Location { lat, lon } => {
            librefang_sidecar::protocol::Content::location(*lat, *lon)
        }
        TgContent::Command { name, args } => {
            librefang_sidecar::protocol::Content::command(name.clone(), args.clone())
        }
        TgContent::ButtonCallback {
            action,
            message_text,
        } => librefang_sidecar::protocol::Content::button_callback(
            action.clone(),
            message_text.clone(),
        ),
        TgContent::PollAnswer {
            poll_id,
            option_ids,
        } => librefang_sidecar::protocol::Content::poll_answer(
            poll_id.clone(),
            option_ids.iter().map(|n| *n as i64).collect(),
        ),
    }
}

/// Local, ergonomic TgContent enum the translator uses. Mirrors the wire ChannelTgContent variants we need for inbound translation; outbound construction uses the SDK's builders directly.
pub enum TgContent {
    Text(String),
    Image {
        url: String,
        caption: Option<String>,
        mime_type: Option<String>,
    },
    File {
        url: String,
        filename: String,
    },
    Voice {
        url: String,
        caption: Option<String>,
        duration_seconds: u32,
    },
    Video {
        url: String,
        caption: Option<String>,
        duration_seconds: u32,
        filename: Option<String>,
    },
    Audio {
        url: String,
        caption: Option<String>,
        duration_seconds: u32,
        title: Option<String>,
        performer: Option<String>,
    },
    Animation {
        url: String,
        caption: Option<String>,
        duration_seconds: u32,
    },
    Sticker {
        file_id: String,
    },
    Location {
        lat: f64,
        lon: f64,
    },
    Command {
        name: String,
        args: Vec<String>,
    },
    ButtonCallback {
        action: String,
        message_text: Option<String>,
    },
    PollAnswer {
        poll_id: String,
        option_ids: Vec<u32>,
    },
}

/// callback_query update → ButtonCallback content event.
pub fn callback_event(cq: &CallbackQuery) -> Option<Value> {
    let user = cq.from.as_ref()?;
    let action = cq.data.clone().unwrap_or_default();
    let message_text = cq.message.as_ref().and_then(|m| m.text.clone());
    let content = TgContent::ButtonCallback {
        action,
        message_text,
    };
    let sender = sender_from_user(user);
    let chat_id = cq
        .message
        .as_ref()
        .map(|m| m.chat.id.to_string())
        .unwrap_or_default();
    // Without is_group, a group-button callback looks like a DM and mis-routes the agent reply (DM session instead of group session).
    let is_group = cq
        .message
        .as_ref()
        .map(|m| matches!(m.chat.chat_type.as_str(), "group" | "supergroup"))
        .unwrap_or(false);
    let mut metadata = serde_json::Map::new();
    metadata.insert("chat_id".into(), json!(chat_id.clone()));
    metadata.insert("platform".into(), json!("telegram"));
    metadata.insert("callback_query_id".into(), json!(cq.id.clone()));
    if let Some(m) = &cq.message {
        metadata.insert("message_id".into(), json!(m.message_id));
    }
    metadata.insert("sender_user_id".into(), json!(sender.user_id.clone()));
    if let Some(uname) = &sender.username {
        metadata.insert("sender_username".into(), json!(uname));
    }
    let mut builder = MessageBuilder::new(chat_id.clone(), sender.name.clone())
        .content(content_to_value(&content))
        .channel_id(chat_id)
        .platform("telegram")
        .is_group(is_group)
        .metadata(metadata);
    if let Some(uname) = sender.username {
        builder = builder.username(uname);
    }
    Some(builder.build())
}

pub(crate) fn sender_from_user(user: &User) -> Sender {
    let mut name = user.first_name.clone();
    if let Some(last) = &user.last_name {
        if !last.is_empty() {
            name.push(' ');
            name.push_str(last);
        }
    }
    if name.is_empty() {
        name = "Unknown".into();
    }
    Sender {
        user_id: user.id.to_string(),
        name,
        username: user.username.clone(),
    }
}

/// poll_answer update → PollAnswer content event.
pub fn poll_answer_event(pa: &PollAnswer) -> Option<Value> {
    let user = pa.user.as_ref()?;
    let content = TgContent::PollAnswer {
        poll_id: pa.poll_id.clone(),
        option_ids: pa.option_ids.clone(),
    };
    let sender = sender_from_user(user);
    // Polls don't carry a chat_id on the answer; the caller doesn't have one either, so route by sender id as a synthetic chat. Daemon side falls back to per-user threading.
    let chat_id = sender.user_id.clone();
    let mut metadata = serde_json::Map::new();
    metadata.insert("chat_id".into(), json!(chat_id.clone()));
    metadata.insert("platform".into(), json!("telegram"));
    metadata.insert("poll_id".into(), json!(pa.poll_id.clone()));
    metadata.insert("sender_user_id".into(), json!(sender.user_id.clone()));
    if let Some(uname) = &sender.username {
        metadata.insert("sender_username".into(), json!(uname));
    }
    let mut builder = MessageBuilder::new(chat_id.clone(), sender.name.clone())
        .content(content_to_value(&content))
        .channel_id(chat_id)
        .platform("telegram")
        .metadata(metadata);
    if let Some(uname) = sender.username {
        builder = builder.username(uname);
    }
    Some(builder.build())
}

/// Top-level: dispatch by update kind. Returns a `Value` per emitted event, or None if the update is a no-op for us.
pub async fn update_to_event(client: &BotClient, update: &Update) -> Option<Value> {
    if let Some(msg) = &update.message {
        return message_event(client, msg, false).await;
    }
    if let Some(msg) = &update.edited_message {
        return message_event(client, msg, true).await;
    }
    if let Some(cq) = &update.callback_query {
        // Dismiss Telegram's inline-button spinner immediately. Without this, the user's UI shows a loading state for up to 30 seconds while waiting for our event to reach the agent and produce an outbound reply. We fire-and-forget — a 400 here doesn't block the event emit.
        let _ = client.answer_callback_query(&cq.id).await;
        return callback_event(cq);
    }
    if let Some(pa) = &update.poll_answer {
        return poll_answer_event(pa);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::MessageEntity;

    fn cmd_msg(text: &str, length: i64) -> Message {
        Message {
            message_id: 1,
            text: Some(text.into()),
            entities: vec![MessageEntity {
                entity_type: "bot_command".into(),
                offset: 0,
                length,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn parse_command_basic() {
        let msg = cmd_msg("/start", 6);
        assert_eq!(parse_command(&msg), Some(("start".into(), vec![])));
    }

    #[test]
    fn parse_command_with_args() {
        let msg = cmd_msg("/echo hello world", 5);
        assert_eq!(
            parse_command(&msg),
            Some(("echo".into(), vec!["hello".into(), "world".into()]))
        );
    }

    #[test]
    fn parse_command_with_botname_suffix() {
        // `/help@my_bot` is 12 UTF-16 units; ` please` is the trailing argument.
        let msg = cmd_msg("/help@my_bot please", 12);
        assert_eq!(
            parse_command(&msg),
            Some(("help".into(), vec!["please".into()]))
        );
    }

    #[test]
    fn parse_command_uses_entity_length_not_whitespace() {
        // Regression: an earlier impl split on whitespace and folded `:foo` into the command name. Bot API entity.length=5 says the command is `/help`; trailing `:foo` is non-args text (no whitespace separator) and ends up as a single arg token via split_whitespace.
        let msg = cmd_msg("/help:foo", 5);
        assert_eq!(
            parse_command(&msg),
            Some(("help".into(), vec![":foo".into()]))
        );
    }

    #[test]
    fn parse_command_handles_unicode_command_name() {
        // Non-BMP characters count as 2 UTF-16 code units. Verify we don't mis-slice if Telegram ever permits non-ASCII in bot_command entities.
        // `/🦀` = `/` (1 unit) + 🦀 U+1F980 (2 units) = 3 UTF-16 units total.
        let msg = cmd_msg("/🦀 hello", 3);
        assert_eq!(
            parse_command(&msg),
            Some(("🦀".into(), vec!["hello".into()]))
        );
    }

    #[test]
    fn parse_command_rejects_bare_slash() {
        let msg = cmd_msg("/", 1);
        assert_eq!(parse_command(&msg), None);
    }

    #[test]
    fn parse_command_rejects_bogus_overrun_length() {
        // Defensive: a misbehaving proxy reports length larger than text. Without the clamp, the function would return ("foo bar baz", []) (args folded into name). With the clamp it returns None.
        let msg = cmd_msg("/foo bar baz", 999);
        assert_eq!(parse_command(&msg), None);
    }

    #[test]
    fn parse_command_rejects_bare_at() {
        let msg = cmd_msg("/@my_bot", 8);
        assert_eq!(parse_command(&msg), None);
    }

    #[test]
    fn parse_command_returns_none_without_bot_command_entity() {
        let msg = Message {
            message_id: 1,
            text: Some("/start".into()),
            entities: vec![],
            ..Default::default()
        };
        assert_eq!(parse_command(&msg), None);
    }

    fn assert_text(c: TgContent, expected: &str) {
        match c {
            TgContent::Text(s) => assert_eq!(s, expected),
            _ => panic!("expected TgContent::Text"),
        }
    }

    #[test]
    fn media_placeholder_matches_python_labels() {
        // Cross-language parity: the strings have to match `sdk/python/librefang/sidecar/adapters/telegram.py:1224,1231,1242,1250,1260,1271,1279`.
        assert_text(
            media_placeholder("Photo received", None, Some("look")),
            "[Photo received: look]",
        );
        assert_text(
            media_placeholder("Photo received", None, None),
            "[Photo received]",
        );
        assert_text(
            media_placeholder("Document received", None, Some("report.pdf")),
            "[Document received: report.pdf]",
        );
        assert_text(
            media_placeholder("Audio received", Some(60), Some("song.mp3")),
            "[Audio received, 60s: song.mp3]",
        );
        assert_text(
            media_placeholder("Voice message", Some(5), None),
            "[Voice message, 5s]",
        );
        assert_text(
            media_placeholder("Animation received", Some(3), None),
            "[Animation received, 3s]",
        );
        assert_text(
            media_placeholder("Video received", Some(12), Some("a clip")),
            "[Video received, 12s: a clip]",
        );
        assert_text(
            media_placeholder("Video note", Some(8), None),
            "[Video note, 8s]",
        );
    }
}
