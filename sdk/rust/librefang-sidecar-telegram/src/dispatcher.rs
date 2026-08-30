//! Outbound dispatch: SDK `Content` value → Telegram Bot API call.
//!
//! Mirrors the Python adapter's `_dispatch_content` / `_send_*` family.
//! Plain text prefers `sendRichMessage` (Bot API 10.1+, server-side GFM parsing); everything else — and the pre-10.1 fallback — routes through `format_sanitize_and_chunk` → `sendMessage` (HTML parse mode), with a "can't parse entities" automatic fallback to plain text. The same fallback is applied to single-item captioned media (Image / Voice / Video / Audio / Animation) so a malformed sanitiser output never silently drops the media send. MediaGroup does NOT have a per-item fallback — it's an atomic Bot API call and a parse error on ANY item caption fails the whole group; callers that need fallback-per-item should send items individually.

use crate::api::types::{InlineKeyboardAction as TgAction, InlineKeyboardButton as TgButton};
use crate::api::{BotClient, Error, Result};
use crate::format::chunk::utf16_len;
use crate::format::{format_and_sanitize, format_sanitize_and_chunk};
use serde_json::{json, Value};

const PARSE_MODE_HTML: &str = "HTML";
/// Bot API caption hard limit (per <https://core.telegram.org/bots/api#sendphoto>). Captions longer than this are truncated to fit before we hit the wire — Telegram rejects oversize captions with `MESSAGE_CAPTION_TOO_LONG` and there is no graceful fallback.
const CAPTION_LIMIT_UTF16: usize = 1024;

fn parse_message_id(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}
/// Maximum FileData byte count we'll accept on the wire before erroring. Sized at 64 MiB — comfortably above cloud Bot API's 50 MB document ceiling and well below any plausible RAM exhaustion budget. Anything larger is either a producer bug or an attempt to OOM the sidecar.
const FILE_DATA_BYTE_CAP: usize = 64 * 1024 * 1024;
/// Bot API `sendPoll` question length bounds, 1-300 UTF-16 code units (<https://core.telegram.org/bots/api#sendpoll>).
const POLL_QUESTION_MIN_UTF16: usize = 1;
const POLL_QUESTION_MAX_UTF16: usize = 300;
/// Bot API `sendPoll` per-option text length bounds, 1-100 UTF-16 code units.
const POLL_OPTION_MIN_UTF16: usize = 1;
const POLL_OPTION_MAX_UTF16: usize = 100;
/// Bot API `sendPoll` explanation length bound, 0-200 UTF-16 code units.
const POLL_EXPLANATION_MAX_UTF16: usize = 200;

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

fn required_coordinate(payload: &Value, key: &str) -> Result<f64> {
    payload
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| Error::Other(format!("Location.{key} missing or not a JSON number")))
}

/// Extract and length-validate the poll question, 1-300 UTF-16 code units (Bot API `sendPoll`).
fn validated_poll_question(payload: &Value) -> Result<&str> {
    let question = payload
        .get("question")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("Poll.question missing".into()))?;
    let len = utf16_len(question);
    if !(POLL_QUESTION_MIN_UTF16..=POLL_QUESTION_MAX_UTF16).contains(&len) {
        return Err(Error::Other(format!(
            "Poll.question must be between {POLL_QUESTION_MIN_UTF16} and {POLL_QUESTION_MAX_UTF16} UTF-16 code units, got {len}"
        )));
    }
    Ok(question)
}

/// Length-validate an optional poll explanation, 0-200 UTF-16 code units (Bot API `sendPoll`).
fn validated_poll_explanation(payload: &Value) -> Result<Option<&str>> {
    match payload.get("explanation").and_then(Value::as_str) {
        Some(explanation) => {
            let len = utf16_len(explanation);
            if len > POLL_EXPLANATION_MAX_UTF16 {
                return Err(Error::Other(format!(
                    "Poll.explanation must be at most {POLL_EXPLANATION_MAX_UTF16} UTF-16 code units, got {len}"
                )));
            }
            Ok(Some(explanation))
        }
        None => Ok(None),
    }
}

fn validated_poll(payload: &Value) -> Result<(Vec<Value>, bool, Option<u32>)> {
    let raw_options = payload
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Other("Poll.options missing or not a JSON array".into()))?;
    if !(2..=10).contains(&raw_options.len()) {
        return Err(Error::Other(
            "Poll.options must contain between 2 and 10 answers".into(),
        ));
    }
    let options = raw_options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let text = option.as_str().ok_or_else(|| {
                Error::Other(format!("Poll.options[{index}] must be a JSON string"))
            })?;
            let len = utf16_len(text);
            if !(POLL_OPTION_MIN_UTF16..=POLL_OPTION_MAX_UTF16).contains(&len) {
                return Err(Error::Other(format!(
                    "Poll.options[{index}] must be between {POLL_OPTION_MIN_UTF16} and {POLL_OPTION_MAX_UTF16} UTF-16 code units, got {len}"
                )));
            }
            Ok(json!({"text": text}))
        })
        .collect::<Result<Vec<_>>>()?;
    let is_quiz = payload
        .get("is_quiz")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let correct = match payload.get("correct_option_id") {
        Some(value) => {
            let raw = value.as_u64().ok_or_else(|| {
                Error::Other("Poll.correct_option_id must be a non-negative integer".into())
            })?;
            Some(u32::try_from(raw).map_err(|_| {
                Error::Other("Poll.correct_option_id exceeds the supported integer range".into())
            })?)
        }
        None => None,
    };
    if is_quiz && correct.is_none() {
        return Err(Error::Other(
            "Poll.correct_option_id is required for quiz polls".into(),
        ));
    }
    if correct.is_some_and(|index| index as usize >= options.len()) {
        return Err(Error::Other(
            "Poll.correct_option_id is outside the options array".into(),
        ));
    }
    Ok((options, is_quiz, correct))
}

/// Truncate a raw (un-formatted) caption to the Bot API limit for the plain-text fallback.
fn truncate_raw_caption(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    Some(crate::format::truncate_to_utf16_limit(raw, CAPTION_LIMIT_UTF16).to_string())
}

/// Send a text message.
///
/// Prefers `sendRichMessage` (Bot API 10.1+), which hands the text to Telegram's own
/// GFM-compatible parser. That gets us tables, `_italic_`, `~~strikethrough~~` and
/// nested emphasis — none of which `format::markdown` can express — and raises the
/// size limit from 4096 to 32768, so ordinary replies stop being split mid-sentence.
/// The text is passed through [`crate::format::prepare_rich_markdown`] first so quoted
/// untrusted content cannot inject interactive elements.
///
/// A definitive refusal by Telegram (see [`is_api_rejection`] for exactly which
/// responses count) falls back to [`send_text_legacy_counting`], keeping pre-10.1 (typically
/// self-hosted) Bot API servers working exactly as before.
pub async fn send_text(
    client: &BotClient,
    chat_id: i64,
    text: &str,
    thread_id: Option<i64>,
) -> Result<()> {
    let (_, error) = send_text_counting(client, chat_id, text, thread_id).await;
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `send_text`, additionally reporting how many messages actually reached the chat
/// before any error.
///
/// Streaming's finalize needs the distinction: the legacy path sends chunks
/// sequentially and, as [`send_text_legacy_counting`] warns, an error there means *possible
/// partial delivery*. A caller that retries or re-renders the whole answer after a
/// partial delivery shows the user the first chunk twice.
pub async fn send_text_counting(
    client: &BotClient,
    chat_id: i64,
    text: &str,
    thread_id: Option<i64>,
) -> (usize, Option<Error>) {
    if let Some(markdown) = crate::format::prepare_rich_markdown(text) {
        match client
            .send_rich_message(chat_id, &markdown, thread_id)
            .await
        {
            Ok(_) => return (1, None),
            // Only a rejection *by Telegram* means the rich path is unavailable for this
            // text. A transport failure (timeout, connection reset) leaves the outcome
            // unknown — Telegram may well have created the message — so re-sending the
            // same answer through the legacy path would deliver it twice.
            Err(e) if !is_api_rejection(&e) => return (0, Some(e)),
            Err(e) => {
                eprintln!("[telegram] sendRichMessage rejected, using HTML fallback: {e}");
            }
        }
    }
    send_text_legacy_counting(client, chat_id, text, thread_id).await
}

/// True when Telegram itself answered with a definitive refusal — the message does not
/// exist and sending different content instead cannot duplicate it.
///
/// Everything else leaves the outcome unknown and must not be retried with different
/// content: `Error::Http` / `Io` / `Decode` mean we never got a verdict, and a 5xx can be
/// returned after the message was already created. Re-sending in those cases delivers the
/// same answer twice, which the user sees and we cannot undo.
///
/// Two cases the plain "is it a 4xx" reading gets wrong:
///
/// * **429 is not definitive.** It is the one 4xx that means "try later", not "not like
///   this". `call_json` has already spent its single retry by the time we see it, so
///   treating it as a refusal sends the same answer again into a chat Telegram has just
///   asked us to back off from.
/// * **`code == 0` is definitive.** `call_json` builds it from
///   `parsed.error_code.unwrap_or(0)`, which is only reachable on the HTTP-2xx-with-
///   `ok: false` path — Telegram answered, in JSON, that it did not create the message.
///   Reading the sentinel as "not a 4xx" would silently disable the fallback for any Bot
///   API deployment that reports failures with a 200, which is exactly the self-hosted
///   pre-10.1 server the fallback exists for.
pub(crate) fn is_api_rejection(e: &Error) -> bool {
    matches!(e, Error::Api { code, .. } if *code != 429 && (*code == 0 || (400..500).contains(code)))
}

/// Legacy path: our own Markdown → sanitised Telegram HTML pipeline, chunked to the
/// 4096 UTF-16 limit, with a plain-text retry on `can't parse entities`.
///
/// Delivery is not atomic when formatting produces multiple Telegram messages: chunks are sent sequentially, and an error on a later chunk is returned after earlier chunks have already been delivered.
/// Telegram provides no rollback for those preceding messages, so callers must treat an error as possible partial delivery rather than as proof that the recipient saw nothing.
/// Reports how many chunks were delivered before any error — see
/// [`send_text_counting`] for why callers need that rather than a bare `Result`.
async fn send_text_legacy_counting(
    client: &BotClient,
    chat_id: i64,
    text: &str,
    thread_id: Option<i64>,
) -> (usize, Option<Error>) {
    let mut delivered = 0;
    for chunk in format_sanitize_and_chunk(text) {
        match client
            .send_message(chat_id, &chunk, Some(PARSE_MODE_HTML), thread_id, None)
            .await
        {
            Ok(_) => delivered += 1,
            Err(e) if is_parse_entities_error(&e) => {
                // Plain-text fallback: strip the HTML markup we added so the user sees readable prose rather than literal `<b>foo</b>` tags. Without the strip, the fallback "succeeds" at delivery but leaks our markup.
                let plain = html_to_plain(&chunk);
                match client
                    .send_message(chat_id, &plain, None, thread_id, None)
                    .await
                {
                    Ok(_) => delivered += 1,
                    Err(e) => return (delivered, Some(e)),
                }
            }
            Err(e) => return (delivered, Some(e)),
        }
    }
    (delivered, None)
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
                            action: TgAction::Url { url: u },
                        });
                    } else if let Some(a) = action {
                        let truncated = truncate_bytes_utf8(&a, 64);
                        row_buttons.push(TgButton {
                            text: label,
                            action: TgAction::CallbackData {
                                callback_data: truncated,
                            },
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
            let lat = required_coordinate(payload, "lat")?;
            let lon = required_coordinate(payload, "lon")?;
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
                .and_then(parse_message_id)
                .ok_or_else(|| {
                    Error::Other(
                        "EditInteractive.message_id must be an integer or decimal string".into(),
                    )
                })?;
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
                .and_then(parse_message_id)
                .ok_or_else(|| {
                    Error::Other(
                        "DeleteMessage.message_id must be an integer or decimal string".into(),
                    )
                })?;
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
            let question = validated_poll_question(payload)?;
            let (options, is_quiz, correct) = validated_poll(payload)?;
            let explanation = validated_poll_explanation(payload)?;
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
        let obj = item
            .as_object()
            .ok_or_else(|| Error::Other("MediaGroup item is not a JSON object".into()))?;
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
            .ok_or_else(|| Error::Other(format!("MediaGroup item {tag} missing url")))?
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
    const OGG_FIXED_HEADER_LEN: usize = 27;
    const OPUS_MAGIC: &[u8; 8] = b"OpusHead";

    if bytes.len() < OGG_FIXED_HEADER_LEN
        || &bytes[..4] != b"OggS"
        || bytes[4] != 0
        || bytes[5] & 0x01 != 0
    {
        return false;
    }

    let segment_count = usize::from(bytes[26]);
    let packet_start = OGG_FIXED_HEADER_LEN + segment_count;
    if segment_count == 0 || bytes.len() < packet_start {
        return false;
    }

    let segment_table = &bytes[OGG_FIXED_HEADER_LEN..packet_start];
    let page_body_len: usize = segment_table
        .iter()
        .map(|length| usize::from(*length))
        .sum();
    let Some(page_end) = packet_start.checked_add(page_body_len) else {
        return false;
    };
    if bytes.len() < page_end {
        return false;
    }

    let mut first_packet_len = 0_usize;
    let mut first_packet_complete = false;
    for segment_len in segment_table {
        first_packet_len += usize::from(*segment_len);
        if *segment_len < u8::MAX {
            first_packet_complete = true;
            break;
        }
    }

    first_packet_complete
        && first_packet_len >= OPUS_MAGIC.len()
        && bytes[packet_start..].starts_with(OPUS_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Minimal Bot API stand-in: serves `expected` responses in order and records the
    /// request line + body of each call, so a test can assert *which* method was used.
    fn mock_bot_api(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock address");
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let mut buf = [0_u8; 8192];
                let read = stream.read(&mut buf).expect("read mock request");
                let request = String::from_utf8_lossy(&buf[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                let payload = request
                    .split_once("\r\n\r\n")
                    .map(|(_, b)| b.to_string())
                    .unwrap_or_default();
                seen.push((line, payload));
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("write mock response");
            }
            seen
        });
        (format!("http://{address}"), handle)
    }

    const OK_MESSAGE: &str = r#"{"ok":true,"result":{"message_id":1}}"#;
    const ERR_NO_METHOD: &str =
        r#"{"ok":false,"error_code":404,"description":"Not Found: method not found"}"#;
    /// A rate limit carries no verdict about whether the *method* exists.
    const ERR_RATE_LIMITED: &str = r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":0}}"#;
    /// Some self-hosted Bot API servers answer HTTP 200 with a bare `ok:false` and no
    /// `error_code`, which the client surfaces as code 0.
    const ERR_BARE_FALSE: &str = r#"{"ok":false,"description":"Bad Request"}"#;

    #[test]
    fn send_text_prefers_rich_message_and_skips_the_html_pipeline() {
        let (root, server) = mock_bot_api(vec![(200, OK_MESSAGE)]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(send_text(
                &client,
                42,
                "| a | b |\n|--|--|\n| _x_ | y |",
                None,
            ))
            .expect("send succeeds");

        let seen = server.join().expect("mock thread");
        assert_eq!(seen.len(), 1, "legacy pipeline must not be touched");
        assert!(seen[0].0.contains("/sendRichMessage"), "{}", seen[0].0);
        // The table reaches Telegram as Markdown, not as pre-rendered HTML.
        assert!(seen[0].1.contains("rich_message"));
        assert!(seen[0].1.contains("| a | b |"));
    }

    /// 429 is excluded from the fallback on purpose: it says the request was throttled,
    /// not that `sendRichMessage` is missing. Falling back would re-send the whole answer
    /// through the legacy path, chunk by chunk, into a chat Telegram just asked us to back
    /// off from — turning one throttled call into several.
    ///
    /// `_call` retries a 429 once, so it takes two to reach `is_api_rejection`. The
    /// assertion is on the *kind* of error rather than the request count: `mock_bot_api`
    /// serves exactly as many connections as it was given responses for, so a third
    /// request cannot be observed — it just fails on a dead listener, and a count-based
    /// test would read the same 2 either way. The 429 has to come back out intact.
    #[test]
    fn a_rate_limit_is_not_treated_as_rich_being_unavailable() {
        let (root, server) = mock_bot_api(vec![(200, ERR_RATE_LIMITED), (200, ERR_RATE_LIMITED)]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let error = runtime
            .block_on(send_text(&client, 42, "**bold**", None))
            .expect_err("a 429 must surface, not silently fall back");

        assert!(
            matches!(error, Error::Api { code: 429, .. }),
            "expected the 429 to surface as-is, got {error:?}"
        );
        let seen = server.join().expect("mock thread");
        assert!(seen
            .iter()
            .all(|(path, _)| path.contains("/sendRichMessage")));
    }

    /// A bare `ok:false` with no `error_code` reaches us as code 0. It is still a
    /// definitive refusal, so it must fall back — otherwise the answer is never delivered
    /// at all, on exactly the self-hosted setups the fallback exists for.
    #[test]
    fn a_bare_ok_false_still_falls_back() {
        let (root, server) = mock_bot_api(vec![(200, ERR_BARE_FALSE), (200, OK_MESSAGE)]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(send_text(&client, 42, "**bold**", None))
            .expect("send succeeds via fallback");

        let seen = server.join().expect("mock thread");
        assert_eq!(seen.len(), 2);
        assert!(seen[0].0.contains("/sendRichMessage"));
        assert!(seen[1].0.contains("/sendMessage"));
    }

    #[test]
    fn send_text_falls_back_to_html_when_rich_is_unavailable() {
        // Pre-10.1 Bot API server: sendRichMessage is unknown, sendMessage works.
        let (root, server) = mock_bot_api(vec![(200, ERR_NO_METHOD), (200, OK_MESSAGE)]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(send_text(&client, 42, "**bold**", None))
            .expect("send succeeds via fallback");

        let seen = server.join().expect("mock thread");
        assert_eq!(seen.len(), 2);
        assert!(seen[0].0.contains("/sendRichMessage"));
        assert!(seen[1].0.contains("/sendMessage"));
        assert!(seen[1].1.contains("HTML"));
        assert!(seen[1].1.contains("<b>bold</b>"));
    }

    #[test]
    fn send_text_sanitizes_injected_buttons_before_they_reach_telegram() {
        let (root, server) = mock_bot_api(vec![(200, OK_MESSAGE)]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let quoted =
            r#"The page said: <tg-button type="callback_data" data="wipe">Confirm</tg-button>"#;
        runtime
            .block_on(send_text(&client, 42, quoted, None))
            .expect("send succeeds");

        let seen = server.join().expect("mock thread");
        assert!(seen[0].0.contains("/sendRichMessage"));
        assert!(
            !seen[0].1.contains(r#"said: <tg-button"#),
            "an injected button reached Telegram: {}",
            seen[0].1
        );
        // The payload is JSON, so the backslash escape is itself escaped on the wire.
        assert!(seen[0].1.contains(r#"\\<tg-button"#));
    }

    #[test]
    fn oversize_text_bypasses_rich_and_uses_the_chunking_pipeline() {
        // Beyond the 32768-character rich limit, so the rich path is skipped entirely
        // and the legacy chunker splits into 4096-unit sendMessage calls.
        let huge = "x".repeat(crate::format::RICH_MSG_LIMIT + 1);
        let (root, server) = mock_bot_api(vec![(200, OK_MESSAGE); 9]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(send_text(&client, 42, &huge, None))
            .expect("send succeeds");

        let seen = server.join().expect("mock thread");
        assert!(
            seen.iter().all(|(line, _)| line.contains("/sendMessage")),
            "rich path must not be attempted above the rich limit"
        );
    }

    /// A transport failure is not a verdict from Telegram: the message may well have
    /// been created. Re-sending the same answer through the legacy path would deliver it
    /// twice, so the rich path must surface the error instead of falling back.
    #[test]
    fn transport_failure_on_rich_does_not_resend_through_the_legacy_path() {
        // One accept, then the connection is closed without a response.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            use std::io::Read;
            let mut methods = Vec::new();
            let mut record = |stream: &mut std::net::TcpStream| {
                let mut buf = [0_u8; 4096];
                let read = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]).to_string();
                methods.push(request.lines().next().unwrap_or_default().to_string());
                // Close with no response at all → the client sees a transport error.
            };
            if let Ok((mut stream, _)) = listener.accept() {
                record(&mut stream);
            }
            // Poll briefly for a second connection: a legacy retry would make one. Must
            // be non-blocking, or a correctly-behaving client leaves this thread parked
            // on accept() forever and the test hangs instead of passing.
            listener.set_nonblocking(true).expect("nonblocking");
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(200)))
                            .ok();
                        record(&mut stream);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            methods
        });

        let root = format!("http://{address}");
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (delivered, error) = runtime.block_on(send_text_counting(&client, 42, "hello", None));

        assert_eq!(delivered, 0);
        assert!(error.is_some(), "transport failure must surface");
        assert!(
            !is_api_rejection(error.as_ref().expect("error")),
            "a dropped connection is not an API rejection"
        );
        drop(client);
        let methods = server.join().expect("server thread");
        assert_eq!(
            methods.len(),
            1,
            "legacy path must not run after a transport failure, saw: {methods:?}"
        );
        assert!(methods[0].contains("/sendRichMessage"));
    }

    /// A mid-sequence chunk failure leaves earlier chunks in the chat. The caller has to
    /// be able to see that, or streaming's finalize re-renders the whole answer into the
    /// placeholder and the user reads the first chunk twice.
    #[test]
    fn partial_chunk_delivery_is_reported_to_the_caller() {
        let huge = "x".repeat(crate::format::RICH_MSG_LIMIT + 1);
        let (root, server) = mock_bot_api(vec![
            (200, OK_MESSAGE),                         // chunk 1 lands
            (200, r#"{"ok":false,"error_code":403}"#), // chunk 2 refused
        ]);
        let client = BotClient::with_roots("t", &root, &root).expect("client");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (delivered, error) = runtime.block_on(send_text_counting(&client, 42, &huge, None));

        let seen = server.join().expect("mock thread");
        assert_eq!(seen.len(), 2, "rich must be skipped above the rich limit");
        assert!(seen.iter().all(|(line, _)| line.contains("/sendMessage")));
        assert_eq!(delivered, 1, "one chunk reached the chat");
        assert!(error.is_some());
    }

    fn ogg_page(segment_table: &[u8], body: &[u8]) -> Vec<u8> {
        let mut page = vec![0_u8; 27];
        page[..4].copy_from_slice(b"OggS");
        page[26] = u8::try_from(segment_table.len()).expect("test segment count");
        page.extend_from_slice(segment_table);
        page.extend_from_slice(body);
        page
    }

    #[test]
    fn ogg_opus_detection_uses_the_first_packet_offset() {
        let mut body = b"OpusHead".to_vec();
        body.resize(19, 0);
        assert!(looks_like_ogg_opus(&ogg_page(&[19], &body)));

        let page = ogg_page(&[19, 0], &body);

        assert!(looks_like_ogg_opus(&page));

        let mut continued_packet = page.clone();
        continued_packet[5] = 0x01;
        assert!(!looks_like_ogg_opus(&continued_packet));

        let misplaced = ogg_page(&[19], b"not-opusOpusHead....");
        assert!(!looks_like_ogg_opus(&misplaced));
        assert!(!looks_like_ogg_opus(&ogg_page(&[255], b"OpusHead")));
    }

    #[test]
    fn ogg_opus_detection_rejects_every_truncation_without_panicking() {
        // Malformed / short inputs that are nowhere near a valid page: must not panic on out-of-bounds slicing.
        assert!(!looks_like_ogg_opus(&[]));
        assert!(!looks_like_ogg_opus(b"OggS"));
        assert!(!looks_like_ogg_opus(&[0_u8; 26]));
        assert!(!looks_like_ogg_opus(b"RIFF....WAVEfmt "));

        let mut short_header = vec![0_u8; 20];
        short_header[..4].copy_from_slice(b"OggS");
        assert!(!looks_like_ogg_opus(&short_header));

        // Full fixed header claiming a segment table that is never actually supplied.
        let mut truncated_segment_table = vec![0_u8; 27];
        truncated_segment_table[..4].copy_from_slice(b"OggS");
        truncated_segment_table[26] = 10;
        assert!(!looks_like_ogg_opus(&truncated_segment_table));

        // A network-truncated download can cut a genuine Ogg/Opus page off at any byte offset.
        // Every prefix short of the full page must be rejected, and none of them may panic.
        let mut body = b"OpusHead".to_vec();
        body.resize(19, 0);
        let full_page = ogg_page(&[19], &body);
        for cut in 1..full_page.len() {
            let truncated = &full_page[..full_page.len() - cut];
            assert!(
                !looks_like_ogg_opus(truncated),
                "truncating the last {cut} byte(s) should not still look like a valid Opus page"
            );
        }
        assert!(looks_like_ogg_opus(&full_page));
    }

    #[test]
    fn media_group_rejects_non_object_items() {
        let error = build_media_group(&[
            json!({"Image": {"url": "https://example.com/one.jpg"}}),
            json!("not-an-object"),
        ])
        .expect_err("malformed media group item must fail");

        assert!(error
            .to_string()
            .contains("MediaGroup item is not a JSON object"));
    }

    #[test]
    fn media_group_rejects_item_missing_url() {
        let error = build_media_group(&[
            json!({"Image": {"url": "https://example.com/one.jpg"}}),
            json!({"Video": {"caption": "no url here"}}),
        ])
        .expect_err("media group item without a url must fail");

        assert!(error
            .to_string()
            .contains("MediaGroup item Video missing url"));
    }

    #[test]
    fn media_group_accepts_a_valid_variable_length_group() {
        // Telegram's sendMediaGroup accepts 2-10 items; exercise both ends of that range.
        let two = build_media_group(&[
            json!({"Image": {"url": "https://example.com/one.jpg", "caption": "first"}}),
            json!({"Video": {"url": "https://example.com/two.mp4", "duration_seconds": 12}}),
        ])
        .expect("a valid 2-item group must be accepted");
        assert_eq!(two.as_array().map(Vec::len), Some(2));

        let ten_items: Vec<Value> = (0..10)
            .map(|i| json!({"Image": {"url": format!("https://example.com/{i}.jpg")}}))
            .collect();
        let ten = build_media_group(&ten_items).expect("a valid 10-item group must be accepted");
        assert_eq!(ten.as_array().map(Vec::len), Some(10));
    }

    #[test]
    fn location_coordinates_are_required_numbers() {
        let valid = json!({"lat": 35.6812, "lon": 139.7671});
        assert_eq!(required_coordinate(&valid, "lat").unwrap(), 35.6812);
        assert_eq!(required_coordinate(&valid, "lon").unwrap(), 139.7671);

        for malformed in [
            json!({"lon": 139.7671}),
            json!({"lat": "35.6812", "lon": 139.7671}),
            json!({"lat": null, "lon": 139.7671}),
        ] {
            let error = required_coordinate(&malformed, "lat").expect_err("invalid latitude");
            assert!(error.to_string().contains("Location.lat"));
        }
    }

    #[test]
    fn poll_options_and_quiz_answer_are_validated_locally() {
        for malformed in [
            json!({}),
            json!({"options": []}),
            // Telegram's Bot API requires at least 2 options; a single-option poll must be
            // rejected locally rather than forwarded to a request Telegram will 400 on.
            json!({"options": ["only"]}),
            json!({"options": ["a", 2]}),
            // Telegram's Bot API caps options at 10; 11 must be rejected.
            json!({"options": ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"]}),
            // Telegram's Bot API requires each option to be 1-100 UTF-16 code units;
            // an empty or oversize option must be rejected locally.
            json!({"options": ["", "b"]}),
            json!({"options": ["a".repeat(101), "b".to_string()]}),
            json!({"options": ["only"], "is_quiz": true}),
            json!({"options": ["a", "b"], "is_quiz": true, "correct_option_id": 2}),
            json!({"options": ["a", "b"], "is_quiz": true, "correct_option_id": "1"}),
            json!({"options": ["a", "b"], "is_quiz": true, "correct_option_id": u64::from(u32::MAX) + 1}),
        ] {
            assert!(validated_poll(&malformed).is_err());
        }

        let (options, is_quiz, correct) = validated_poll(&json!({
            "options": ["a", "b"],
            "is_quiz": true,
            "correct_option_id": 1
        }))
        .expect("valid quiz");
        assert_eq!(options, vec![json!({"text": "a"}), json!({"text": "b"})]);
        assert!(is_quiz);
        assert_eq!(correct, Some(1));

        // Exactly 10 options (the upper bound) and exactly 2 (the lower bound) must both
        // pass local validation as regular (non-quiz) polls.
        let ten_options: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
        let (options, is_quiz, correct) =
            validated_poll(&json!({"options": ten_options})).expect("valid regular poll (max)");
        assert_eq!(options.len(), 10);
        assert!(!is_quiz);
        assert_eq!(correct, None);

        let (_, is_quiz, correct) =
            validated_poll(&json!({"options": ["a", "b"]})).expect("valid regular poll (min)");
        assert!(!is_quiz);
        assert_eq!(correct, None);

        // Exactly 100 UTF-16 code units (the upper bound) and exactly 1 (the lower bound)
        // must both pass local validation.
        let (options, _, _) = validated_poll(&json!({
            "options": ["a".repeat(100), "b".to_string()]
        }))
        .expect("valid option lengths at bounds");
        assert_eq!(options[0]["text"].as_str().unwrap().len(), 100);
    }

    #[test]
    fn poll_question_length_is_validated_locally() {
        assert!(validated_poll_question(&json!({})).is_err());
        assert!(validated_poll_question(&json!({"question": ""})).is_err());
        // Telegram's Bot API caps the question at 300 UTF-16 code units; 301 must be rejected.
        assert!(validated_poll_question(&json!({"question": "a".repeat(301)})).is_err());

        assert_eq!(
            validated_poll_question(&json!({"question": "a".repeat(300)})).expect("valid at max"),
            "a".repeat(300)
        );
        assert_eq!(
            validated_poll_question(&json!({"question": "q"})).expect("valid at min"),
            "q"
        );
    }

    #[test]
    fn poll_explanation_length_is_validated_locally() {
        assert!(validated_poll_explanation(&json!({})).unwrap().is_none());
        // Telegram's Bot API caps the explanation at 200 UTF-16 code units; 201 must be rejected.
        assert!(validated_poll_explanation(&json!({"explanation": "a".repeat(201)})).is_err());
        assert_eq!(
            validated_poll_explanation(&json!({"explanation": "a".repeat(200)}))
                .expect("valid at max"),
            Some("a".repeat(200).as_str())
        );
    }

    #[test]
    fn message_id_accepts_integer_or_decimal_string() {
        assert_eq!(parse_message_id(&json!(12345)), Some(12345));
        assert_eq!(parse_message_id(&json!("12345")), Some(12345));
        assert_eq!(parse_message_id(&json!(-7)), Some(-7));
        assert_eq!(parse_message_id(&json!(12.5)), None);
        assert_eq!(parse_message_id(&json!("9223372036854775808")), None);
        assert_eq!(parse_message_id(&Value::Null), None);
    }

    #[test]
    fn inline_keyboard_builder_emits_one_action_per_button() {
        let keyboard = build_inline_keyboard(&json!({
            "buttons": [[
                {"label": "Docs", "url": "https://example.com", "action": "ignored"},
                {"label": "Run", "action": "run"},
                {"label": "Dropped"}
            ]]
        }));
        assert_eq!(
            keyboard,
            json!({"inline_keyboard": [[
                {"text": "Docs", "url": "https://example.com"},
                {"text": "Run", "callback_data": "run"}
            ]]})
        );
    }
}
