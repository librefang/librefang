//! Outbound dispatch: SDK `Content` value → Telegram Bot API call.
//!
//! Mirrors the Python adapter's `_dispatch_content` / `_send_*` family.
//! All text routes go through `format_and_sanitize` → `split_to_utf16_chunks` → `sendMessage` (HTML parse mode), with a "can't parse entities" automatic fallback to plain text. The same fallback is applied to single-item captioned media (Image / Voice / Video / Audio / Animation) so a malformed sanitiser output never silently drops the media send. MediaGroup does NOT have a per-item fallback — it's an atomic Bot API call and a parse error on ANY item caption fails the whole group; callers that need fallback-per-item should send items individually.

use crate::api::types::InlineKeyboardButton as TgButton;
use crate::api::{BotClient, Error, Result};
use crate::format::{format_and_sanitize, split_to_utf16_chunks, TELEGRAM_MSG_LIMIT};
use serde_json::{json, Value};

const PARSE_MODE_HTML: &str = "HTML";
/// Bot API caption hard limit (per <https://core.telegram.org/bots/api#sendphoto>). Captions longer than this are truncated to fit before we hit the wire — Telegram rejects oversize captions with `MESSAGE_CAPTION_TOO_LONG` and there is no graceful fallback.
const CAPTION_LIMIT_UTF16: usize = 1024;
/// Maximum FileData byte count we'll accept on the wire before erroring. Sized at 64 MiB — comfortably above cloud Bot API's 50 MB document ceiling and well below any plausible RAM exhaustion budget. Anything larger is either a producer bug or an attempt to OOM the sidecar.
const FILE_DATA_BYTE_CAP: usize = 64 * 1024 * 1024;

/// Telegram returned `400 Bad Request: can't parse entities ...`. Used by every HTML-parse-mode call to decide whether to fall back to plain text.
pub(crate) fn is_parse_entities_error(e: &Error) -> bool {
    matches!(e, Error::Api { description, .. } if description.contains("can't parse entities"))
}

/// Telegram returned `400 Bad Request: message is not modified`. Common during streaming-edit debounce ticks where no new content has actually accumulated; treat as success rather than spamming the log.
pub(crate) fn is_message_not_modified(e: &Error) -> bool {
    matches!(e, Error::Api { code, description, .. } if *code == 400 && description.contains("message is not modified"))
}

/// Prepare a caption for sending: format → sanitize → truncate to the caption limit. Returns `None` for None/empty input.
fn prepare_caption(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let formatted = format_and_sanitize(raw);
    Some(crate::format::truncate_to_utf16_limit(&formatted, CAPTION_LIMIT_UTF16).to_string())
}

/// Truncate a raw (un-formatted) caption to the Bot API limit for the plain-text fallback.
fn truncate_raw_caption(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    Some(crate::format::truncate_to_utf16_limit(raw, CAPTION_LIMIT_UTF16).to_string())
}

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
            Err(e) if is_parse_entities_error(&e) => {
                // Plain-text fallback: strip the HTML markup we added so the user sees readable prose rather than literal `<b>foo</b>` tags. Without the strip, the fallback "succeeds" at delivery but leaks our markup.
                let plain = html_to_plain(&chunk);
                client
                    .send_message(chat_id, &plain, None, thread_id, None)
                    .await?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

static RE_HTML_TAG: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"<[^>]+>").expect("html-strip tag regex"));

/// Strip HTML tags and decode the small set of entities our markdown pipeline ever emits. Used by the plain-text fallback when Telegram rejects our HTML — we want the user to see readable text, not the raw markup. Entity-decode order matters: replace `&lt;` / `&gt;` / `&quot;` / `&#39;` before `&amp;` so a literal `&amp;lt;` round-trips back to `&lt;` rather than collapsing to `<`.
pub(crate) fn html_to_plain(s: &str) -> String {
    let no_tags = RE_HTML_TAG.replace_all(s, "");
    no_tags
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
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
    // Content is the externally-tagged ChannelContent enum — exactly one key. `obj.iter().next()` returns "the first key by iteration order", which depends on whether `serde_json` was built with the `preserve_order` feature; a multi-key object could silently route to the wrong arm. Reject anything but a single-key object.
    if obj.len() != 1 {
        return Err(Error::Other(format!(
            "Content must be a single-key externally-tagged object, got {} keys",
            obj.len()
        )));
    }
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
            let raw_caption = payload.get("caption").and_then(Value::as_str);
            let formatted = prepare_caption(raw_caption);
            match client
                .send_photo_url(
                    chat_id,
                    url,
                    formatted.as_deref(),
                    Some(PARSE_MODE_HTML),
                    thread_id,
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = truncate_raw_caption(raw_caption);
                    client
                        .send_photo_url(chat_id, url, plain.as_deref(), None, thread_id)
                        .await?;
                }
                Err(e) => return Err(e),
            }
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
                client
                    .send_voice_url(chat_id, url, None, None, thread_id)
                    .await?;
            } else {
                client
                    .send_document_url(chat_id, url, None, None, thread_id)
                    .await?;
            }
        }
        "FileData" => {
            let data_array = payload
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::Other("FileData.data missing".into()))?;
            // Cap up-front allocation. An adversarial / misbehaving producer can declare a 10 billion-element JSON array and force us to reserve ~10 GB of heap before we ever read element 1; bound to a generous-but-safe ceiling so a malicious payload errors during parse rather than during an OOM. Telegram's hard ceiling for sendDocument is 50 MB (cloud Bot API) / 2 GB (local Bot API), so a sub-100 MB cap covers every legitimate upload.
            if data_array.len() > FILE_DATA_BYTE_CAP {
                return Err(Error::Other(format!(
                    "FileData.data: {} bytes exceeds {FILE_DATA_BYTE_CAP}-byte cap",
                    data_array.len()
                )));
            }
            // Decode bytes strictly: any element that is not a non-negative integer in [0,255] is a wire-protocol violation. Silently dropping (`filter_map`) or truncating (`n as u8`) would emit a corrupt file with no diagnostic; reject loudly instead so a misbehaving producer is visible.
            let mut bytes: Vec<u8> = Vec::with_capacity(data_array.len());
            for v in data_array {
                let n = v.as_u64().ok_or_else(|| {
                    Error::Other("FileData.data: element is not a non-negative integer".into())
                })?;
                if n > 255 {
                    return Err(Error::Other(format!(
                        "FileData.data: element {n} out of byte range"
                    )));
                }
                bytes.push(n as u8);
            }
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
            let raw_caption = payload.get("caption").and_then(Value::as_str);
            let formatted = prepare_caption(raw_caption);
            match client
                .send_voice_url(
                    chat_id,
                    url,
                    formatted.as_deref(),
                    Some(PARSE_MODE_HTML),
                    thread_id,
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = truncate_raw_caption(raw_caption);
                    client
                        .send_voice_url(chat_id, url, plain.as_deref(), None, thread_id)
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        "Video" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Video.url missing".into()))?;
            let raw_caption = payload.get("caption").and_then(Value::as_str);
            let formatted = prepare_caption(raw_caption);
            match client
                .send_video_url(
                    chat_id,
                    url,
                    formatted.as_deref(),
                    Some(PARSE_MODE_HTML),
                    thread_id,
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = truncate_raw_caption(raw_caption);
                    client
                        .send_video_url(chat_id, url, plain.as_deref(), None, thread_id)
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        "Audio" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Audio.url missing".into()))?;
            let raw_caption = payload.get("caption").and_then(Value::as_str);
            let formatted = prepare_caption(raw_caption);
            let title = payload.get("title").and_then(Value::as_str);
            let performer = payload.get("performer").and_then(Value::as_str);
            match client
                .send_audio_url(
                    chat_id,
                    url,
                    formatted.as_deref(),
                    Some(PARSE_MODE_HTML),
                    title,
                    performer,
                    thread_id,
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = truncate_raw_caption(raw_caption);
                    client
                        .send_audio_url(
                            chat_id,
                            url,
                            plain.as_deref(),
                            None,
                            title,
                            performer,
                            thread_id,
                        )
                        .await?;
                }
                Err(e) => return Err(e),
            }
        }
        "Animation" => {
            let url = payload
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("Animation.url missing".into()))?;
            let raw_caption = payload.get("caption").and_then(Value::as_str);
            let formatted = prepare_caption(raw_caption);
            match client
                .send_animation_url(
                    chat_id,
                    url,
                    formatted.as_deref(),
                    Some(PARSE_MODE_HTML),
                    thread_id,
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = truncate_raw_caption(raw_caption);
                    client
                        .send_animation_url(chat_id, url, plain.as_deref(), None, thread_id)
                        .await?;
                }
                Err(e) => return Err(e),
            }
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
            match client
                .send_message(
                    chat_id,
                    &formatted,
                    Some(PARSE_MODE_HTML),
                    thread_id,
                    Some(keyboard.clone()),
                )
                .await
            {
                Ok(_) => {}
                Err(e) if is_parse_entities_error(&e) => {
                    // Same fallback shape as send_text / EditInteractive: strip HTML so the buttons still ship even when the body's HTML is malformed. Without this the entire interactive payload (text + keyboard) is silently dropped.
                    let plain = html_to_plain(&formatted);
                    client
                        .send_message(chat_id, &plain, None, thread_id, Some(keyboard))
                        .await?;
                }
                Err(e) => return Err(e),
            }
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
                Err(e) if is_parse_entities_error(&e) => {
                    let plain = html_to_plain(&formatted);
                    client
                        .edit_message_text(chat_id, message_id, &plain, None, Some(keyboard))
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
            // Reject nested MediaGroup BEFORE recursing — an adversarial / buggy agent payload like `MediaGroup{items:[MediaGroup{items:[...]}]}` would otherwise recurse via Box::pin without depth bound and overflow the heap-allocated future stack. Scan ALL keys with `any` so a multi-key item (which is itself a contract violation, but defensive checking is cheap) cannot smuggle a MediaGroup past the guard regardless of `serde_json::Map` iteration order.
            for item in items_array {
                if item
                    .as_object()
                    .is_some_and(|obj| obj.keys().any(|k| k == "MediaGroup"))
                {
                    return Err(Error::Other(
                        "MediaGroup may not contain nested MediaGroup items".into(),
                    ));
                }
            }
            // Bot API requires 2..=10 items per sendMediaGroup. Outside that range, fall back to per-item dispatch (1 item → single send; >10 → chunk into batches of 10) so the user's media still ships. NOTE: the Python reference adapter raises ValueError on >10; this Rust port is deliberately more permissive.
            if items_array.len() == 1 {
                Box::pin(dispatch_content(
                    client,
                    chat_id,
                    &items_array[0],
                    thread_id,
                ))
                .await?;
            } else if items_array.is_empty() {
                // Nothing to send — no-op.
            } else {
                for batch in items_array.chunks(10) {
                    if batch.len() == 1 {
                        Box::pin(dispatch_content(client, chat_id, &batch[0], thread_id)).await?;
                    } else {
                        let media = build_media_group(batch)?;
                        client.send_media_group(chat_id, media, thread_id).await?;
                    }
                }
            }
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
        if obj.len() != 1 {
            return Err(Error::Other(format!(
                "MediaGroup item must be a single-key externally-tagged object, got {} keys",
                obj.len()
            )));
        }
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
        let raw_caption = payload.get("caption").and_then(Value::as_str);
        let formatted_caption = prepare_caption(raw_caption);
        let duration = payload
            .get("duration_seconds")
            .and_then(Value::as_u64)
            .map(|n| n as u32);
        let mut entry = json!({ "type": kind, "media": media });
        if let Some(c) = formatted_caption {
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
