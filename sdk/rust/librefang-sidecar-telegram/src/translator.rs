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
/// Returns `(name, args)` when the message text starts with a slash command, `None` otherwise.
fn parse_command(msg: &Message) -> Option<(String, Vec<String>)> {
    let text = msg.text.as_deref()?;
    let first = msg.entities.first()?;
    if first.entity_type != "bot_command" || first.offset != 0 {
        return None;
    }
    let cmd_len = first.length.max(0) as usize;
    let mut chars = text.chars();
    let mut cmd: String = String::new();
    let mut taken = 0;
    while taken < cmd_len {
        if let Some(c) = chars.next() {
            cmd.push(c);
            taken += 1;
        } else {
            break;
        }
    }
    let cmd_trimmed = cmd.trim_start_matches('/').to_string();
    // Strip `@botname` suffix if present.
    let bare = match cmd_trimmed.find('@') {
        Some(at) => cmd_trimmed[..at].to_string(),
        None => cmd_trimmed,
    };
    let rest: String = chars.collect();
    let args: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
    Some((bare, args))
}

fn mime_from_filename(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let dot = lower.rfind('.')?;
    let ext = &lower[dot + 1..];
    Some(
        match ext {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "mp4" => "video/mp4",
            "mov" => "video/quicktime",
            "mp3" => "audio/mpeg",
            "ogg" | "oga" => "audio/ogg",
            "opus" => "audio/opus",
            "pdf" => "application/pdf",
            _ => return None,
        }
        .to_string(),
    )
}

/// Best-effort file-id → public URL. Returns None on lookup failure (the caller falls back to a text placeholder).
pub async fn file_url(client: &BotClient, file_id: &str) -> Option<String> {
    match client.get_file(file_id).await {
        Ok(res) => res.file_path.map(|p| client.file_url(&p)),
        Err(_) => None,
    }
}

/// Map a Telegram `Message` to a single `TgContent`. Returns None for unsupported variants (the caller drops the message).
pub async fn extract_content(client: &BotClient, msg: &Message) -> Option<TgContent> {
    if msg.text.is_some() {
        if let Some((name, args)) = parse_command(msg) {
            return Some(TgContent::Command { name, args });
        }
        return msg.text.clone().map(TgContent::Text);
    }
    if let Some(photos) = msg.photo.last() {
        let url = file_url(client, &photos.file_id).await?;
        let caption = msg.caption.clone();
        return Some(TgContent::Image {
            url,
            caption,
            mime_type: Some("image/jpeg".into()),
        });
    }
    if let Some(doc) = &msg.document {
        let url = file_url(client, &doc.file_id).await?;
        let filename = doc.file_name.clone().unwrap_or_else(|| "document".into());
        return Some(TgContent::File { url, filename });
    }
    if let Some(audio) = &msg.audio {
        let url = file_url(client, &audio.file_id).await?;
        return Some(TgContent::Audio {
            url,
            caption: msg.caption.clone(),
            duration_seconds: audio.duration,
            title: audio.title.clone(),
            performer: audio.performer.clone(),
        });
    }
    if let Some(voice) = &msg.voice {
        let url = file_url(client, &voice.file_id).await?;
        return Some(TgContent::Voice {
            url,
            caption: msg.caption.clone(),
            duration_seconds: voice.duration,
        });
    }
    if let Some(anim) = &msg.animation {
        let url = file_url(client, &anim.file_id).await?;
        return Some(TgContent::Animation {
            url,
            caption: msg.caption.clone(),
            duration_seconds: anim.duration,
        });
    }
    if let Some(video) = &msg.video {
        let url = file_url(client, &video.file_id).await?;
        return Some(TgContent::Video {
            url,
            caption: msg.caption.clone(),
            duration_seconds: video.duration,
            filename: video.file_name.clone(),
        });
    }
    if let Some(vn) = &msg.video_note {
        let url = file_url(client, &vn.file_id).await?;
        return Some(TgContent::Video {
            url,
            caption: None,
            duration_seconds: vn.duration,
            filename: None,
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
    if let Some(_contact) = &msg.contact {
        // No TgContent variant for contact yet — surface as text.
        let label = msg
            .contact
            .as_ref()
            .map(|c| {
                let mut s = format!("Contact: {}", c.first_name);
                if let Some(l) = &c.last_name {
                    s.push(' ');
                    s.push_str(l);
                }
                s.push_str(&format!(" ({})", c.phone_number));
                s
            })
            .unwrap_or_else(|| "Contact".into());
        return Some(TgContent::Text(label));
    }
    // Bots send Empty/other types — produce a placeholder so downstream can ignore.
    let _ = mime_from_filename;
    None
}

/// Reply context: prefix `[Replying to <sender>: "..."]` to a text-shaped TgContent.
pub fn apply_reply(content: TgContent, msg: &Message) -> TgContent {
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
                None => prefix.clone(),
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

fn build_metadata(msg: &Message, sender: &Sender) -> serde_json::Map<String, Value> {
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
    m
}

/// Build a message-event Value from a Telegram `Message`.
pub async fn message_event(client: &BotClient, msg: &Message) -> Option<Value> {
    let content = extract_content(client, msg).await?;
    let content = apply_reply(content, msg);
    let sender = extract_sender(msg);
    let chat_id = msg.chat.id.to_string();
    let is_group = matches!(
        msg.chat.chat_type.as_str(),
        "group" | "supergroup" | "channel"
    );
    let metadata = build_metadata(msg, &sender);
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
        .metadata(metadata);
    if let Some(uname) = sender.username {
        builder = builder.username(uname);
    }
    Some(builder.build())
}

fn sender_from_user(user: &User) -> Sender {
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
        return message_event(client, msg).await;
    }
    if let Some(msg) = &update.edited_message {
        return message_event(client, msg).await;
    }
    if let Some(cq) = &update.callback_query {
        return callback_event(cq);
    }
    if let Some(pa) = &update.poll_answer {
        return poll_answer_event(pa);
    }
    None
}
