//! Outbound dispatch: SDK `Content` value → Telegram Bot API call.
//!
//! Mirrors the Python adapter's `_dispatch_content` / `_send_*` family.
//! All text routes go through `format_and_sanitize` → `split_to_utf16_chunks` → `sendMessage` (HTML parse mode), with a "can't parse entities" automatic fallback to plain text.

use crate::api::types::InlineKeyboardButton as TgButton;
use crate::api::{BotClient, Error, Result};
use crate::format::{format_and_sanitize, split_to_utf16_chunks, TELEGRAM_MSG_LIMIT};
use serde_json::{json, Value};

const PARSE_MODE_HTML: &str = "HTML";

/// Send a text message (formatted + sanitised + chunked).
pub async fn send_text(
    client: &BotClient,
    chat_id: i64,
    text: &str,
    thread_id: Option<i64>,
) -> Result<()> {
    let formatted = format_and_sanitize(text);
    for chunk in split_to_utf16_chunks(&formatted, TELEGRAM_MSG_LIMIT) {
        match client
            .send_message(chat_id, &chunk, Some(PARSE_MODE_HTML), thread_id, None)
            .await
        {
            Ok(_) => {}
            Err(Error::Api { description, .. }) if description.contains("can't parse entities") => {
                // Plain-text fallback so a malformed sanitiser output never blocks a user message.
                client
                    .send_message(chat_id, &chunk, None, thread_id, None)
                    .await?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn build_inline_keyboard(message: &Value) -> Value {
    let mut rows: Vec<Vec<TgButton>> = Vec::new();
    if let Some(buttons) = message.get("buttons").and_then(Value::as_array) {
        for row in buttons {
            let mut row_buttons: Vec<TgButton> = Vec::new();
            if let Some(arr) = row.as_array() {
                for btn in arr {
                    let label = btn
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let action = btn
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let url = btn.get("url").and_then(Value::as_str).map(str::to_string);
                    if let Some(u) = url {
                        row_buttons.push(TgButton {
                            text: label,
                            url: Some(u),
                            callback_data: None,
                        });
                    } else if let Some(a) = action {
                        let truncated = truncate_bytes_utf8(&a, 64);
                        row_buttons.push(TgButton {
                            text: label,
                            url: None,
                            callback_data: Some(truncated),
                        });
                    }
                }
            }
            if !row_buttons.is_empty() {
                rows.push(row_buttons);
            }
        }
    }
    json!({ "inline_keyboard": rows })
}

fn truncate_bytes_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    s[..end].to_string()
}

/// Dispatch a `Content` JSON value (externally-tagged ChannelContent) to the appropriate Bot API call.
pub async fn dispatch_content(
    client: &BotClient,
    chat_id: i64,
    content: &Value,
    thread_id: Option<i64>,
) -> Result<()> {
    let Some(obj) = content.as_object() else {
        return Err(Error::Other("Content is not a JSON object".into()));
    };
    let Some((tag, payload)) = obj.iter().next() else {
        return Err(Error::Other("Content is empty".into()));
    };
    match tag.as_str() {
        "Text" => {
            let text = payload.as_str().unwrap_or("");
            send_text(client, chat_id, text, thread_id).await?;
        }
        "Image" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Image.url missing".into()))?;
            let caption = payload.get("caption").and_then(Value::as_str);
            client
                .send_photo_url(chat_id, url, caption, thread_id)
                .await?;
        }
        "File" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("File.url missing".into()))?;
            let filename = payload
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("file");
            if is_voice_filename(filename) {
                client.send_voice_url(chat_id, url, None, thread_id).await?;
            } else {
                client
                    .send_document_url(chat_id, url, None, thread_id)
                    .await?;
            }
        }
        "FileData" => {
            let data_array = payload
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Other("FileData.data missing".into()))?;
            let bytes: Vec<u8> = data_array
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect();
            let filename = payload
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("file")
                .to_string();
            let mime_type = payload
                .get("mime_type")
                .and_then(Value::as_str)
                .map(str::to_string);
            dispatch_filedata(client, chat_id, bytes, filename, mime_type, thread_id).await?;
        }
        "Voice" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Voice.url missing".into()))?;
            let caption = payload.get("caption").and_then(Value::as_str);
            client
                .send_voice_url(chat_id, url, caption, thread_id)
                .await?;
        }
        "Video" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Video.url missing".into()))?;
            let caption = payload.get("caption").and_then(Value::as_str);
            client
                .send_video_url(chat_id, url, caption, thread_id)
                .await?;
        }
        "Audio" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Audio.url missing".into()))?;
            let caption = payload.get("caption").and_then(Value::as_str);
            let title = payload.get("title").and_then(Value::as_str);
            let performer = payload.get("performer").and_then(Value::as_str);
            client
                .send_audio_url(chat_id, url, caption, title, performer, thread_id)
                .await?;
        }
        "Animation" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Animation.url missing".into()))?;
            let caption = payload.get("caption").and_then(Value::as_str);
            client
                .send_animation_url(chat_id, url, caption, thread_id)
                .await?;
        }
        "Sticker" => {
            let file_id = payload
                .get("file_id")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Sticker.file_id missing".into()))?;
            client
                .send_sticker_file_id(chat_id, file_id, thread_id)
                .await?;
        }
        "Location" => {
            let lat = payload.get("lat").and_then(Value::as_f64).unwrap_or(0.0);
            let lon = payload.get("lon").and_then(Value::as_f64).unwrap_or(0.0);
            client.send_location(chat_id, lat, lon, thread_id).await?;
        }
        "Command" => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
            let args: Vec<String> = payload
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let text = if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {}", args.join(" "))
            };
            send_text(client, chat_id, &text, thread_id).await?;
        }
        "Interactive" => {
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
            let keyboard = build_inline_keyboard(payload);
            let formatted = format_and_sanitize(text);
            client
                .send_message(
                    chat_id,
                    &formatted,
                    Some(PARSE_MODE_HTML),
                    thread_id,
                    Some(keyboard),
                )
                .await?;
        }
        "EditInteractive" => {
            let message_id = payload
                .get("message_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| Error::Other("EditInteractive.message_id missing".into()))?;
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
            let keyboard = build_inline_keyboard(payload);
            let formatted = format_and_sanitize(text);
            match client
                .edit_message_text(
                    chat_id,
                    message_id,
                    &formatted,
                    Some(PARSE_MODE_HTML),
                    Some(keyboard.clone()),
                )
                .await
            {
                Ok(_) => {}
                Err(Error::Api { description, .. })
                    if description.contains("can't parse entities") =>
                {
                    client
                        .edit_message_text(chat_id, message_id, &formatted, None, Some(keyboard))
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        "DeleteMessage" => {
            let message_id = payload
                .get("message_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| Error::Other("DeleteMessage.message_id missing".into()))?;
            client.delete_message(chat_id, message_id).await?;
        }
        "MediaGroup" => {
            let items_array = payload
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Other("MediaGroup.items missing".into()))?;
            let media = build_media_group(items_array)?;
            client.send_media_group(chat_id, media, thread_id).await?;
        }
        "Poll" => {
            let question = payload
                .get("question")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Poll.question missing".into()))?;
            let options: Vec<Value> = payload
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| json!({"text": s}))
                        .collect()
                })
                .unwrap_or_default();
            let is_quiz = payload
                .get("is_quiz")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let correct = payload
                .get("correct_option_id")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let explanation = payload.get("explanation").and_then(Value::as_str);
            client
                .send_poll(
                    chat_id,
                    question,
                    options,
                    is_quiz,
                    correct,
                    explanation,
                    thread_id,
                )
                .await?;
        }
        "ButtonCallback" | "PollAnswer" => {
            // Outbound callbacks / poll answers have no Telegram equivalent — they're inbound-only.
        }
        other => {
            return Err(Error::Other(format!("unsupported Content tag {other}")));
        }
    }
    Ok(())
}

fn build_media_group(items: &[Value]) -> Result<Value> {
    let mut out: Vec<Value> = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some((tag, payload)) = obj.iter().next() else {
            continue;
        };
        let kind = match tag.as_str() {
            "Image" => "photo",
            "Video" => "video",
            other => {
                return Err(Error::Other(format!(
                    "MediaGroup item {other} not supported"
                )))
            }
        };
        let media = payload
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let caption = payload
            .get("caption")
            .and_then(Value::as_str)
            .map(str::to_string);
        let duration = payload
            .get("duration_seconds")
            .and_then(Value::as_u64)
            .map(|n| n as u32);
        let mut entry = json!({ "type": kind, "media": media });
        if let Some(c) = caption {
            entry["caption"] = json!(c);
            entry["parse_mode"] = json!("HTML");
        }
        if let Some(d) = duration {
            entry["duration"] = json!(d);
        }
        out.push(entry);
    }
    Ok(Value::Array(out))
}

/// Inline file bytes — detect Ogg/Opus magic and route to sendVoice, else sendDocument.
async fn dispatch_filedata(
    client: &BotClient,
    chat_id: i64,
    bytes: Vec<u8>,
    filename: String,
    mime_type: Option<String>,
    thread_id: Option<i64>,
) -> Result<()> {
    let is_voice = looks_like_ogg_opus(&bytes)
        || mime_type
            .as_deref()
            .map(|m| m == "audio/ogg" || m == "audio/opus")
            .unwrap_or(false);
    let (method, field) = if is_voice {
        ("sendVoice", "voice")
    } else {
        ("sendDocument", "document")
    };
    client
        .send_multipart(
            method,
            chat_id,
            field,
            filename,
            bytes,
            mime_type,
            vec![],
            thread_id,
        )
        .await?;
    Ok(())
}

fn is_voice_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.rsplit('.').next().unwrap_or(""),
        "ogg" | "oga" | "opus"
    )
}

fn looks_like_ogg_opus(bytes: &[u8]) -> bool {
    if bytes.len() < 36 {
        return false;
    }
    if &bytes[0..4] != b"OggS" {
        return false;
    }
    // OpusHead magic appears at byte 28 in a standard Ogg/Opus stream.
    &bytes[28..36] == b"OpusHead"
}
