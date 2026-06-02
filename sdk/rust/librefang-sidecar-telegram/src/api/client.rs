//! Thin reqwest wrapper around the Telegram Bot API.
//!
//! Mirrors `_api_get` / `_api_post` / `_call` / `_call_retrying` from the Python adapter:
//! - GET with query params for read endpoints.
//! - POST with JSON body for write endpoints.
//! - 429 Too-Many-Requests is retried once after the server-supplied `retry_after`.
//! - Multi-part for inline file bytes (Content::FileData and private-URL fetch-and-upload paths).

use super::error::{Error, Result};
use super::types::{ApiResponse, GetFileResult, PollResult, SendMessageResult, UpdatesResponse};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const DEFAULT_LONGPOLL_TIMEOUT_SECS: u64 = 30;
pub const SEND_TIMEOUT_SECS: u64 = 30;
/// Extra buffer added to the long-poll server-side timeout to derive the reqwest per-request deadline. Telegram sometimes returns a few hundred milliseconds after the server timeout elapses.
pub const LONGPOLL_CLIENT_BUFFER_SECS: u64 = 5;
pub const RETRY_AFTER_DEFAULT_SECS: u64 = 5;
/// Cap how long we will sleep on a 429 `retry_after` from Telegram. A flood-wait can return hours; sleeping that long stalls the entire produce loop with no cancellation. Anything above this is surfaced as an Error so the supervisor can choose to restart.
pub const MAX_RETRY_AFTER_SECS: u64 = 300;

#[derive(Clone)]
pub struct BotClient {
    http: Client,
    api_root: String,
    file_root: String,
    token: String,
}

impl BotClient {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(Error::Other("TELEGRAM_BOT_TOKEN is empty".into()));
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(SEND_TIMEOUT_SECS))
            .build()?;
        let api_root = format!("https://api.telegram.org/bot{token}");
        let file_root = format!("https://api.telegram.org/file/bot{token}");
        Ok(Self {
            http,
            api_root,
            file_root,
            token,
        })
    }

    pub fn file_url(&self, file_path: &str) -> String {
        format!("{}/{file_path}", self.file_root)
    }

    /// Replace the bot token with `[REDACTED]` in any string about to be exposed via Display / logs / protocol-error events. Use before constructing an `Error::Api { description }` from a response body — proxies and some Bot API error paths echo the request URL back into the body, which contains `bot<TOKEN>` in the path.
    fn redact(&self, s: String) -> String {
        if self.token.is_empty() {
            return s;
        }
        s.replace(&self.token, "[REDACTED]")
    }

    /// Low-level long-poll GET for `getUpdates` — separate from `call` so the per-request timeout can be longer than the default.
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout_secs: u64,
        allowed_updates: &[&str],
    ) -> Result<UpdatesResponse> {
        let url = format!("{}/getUpdates", self.api_root);
        let allowed_json = serde_json::to_string(allowed_updates).unwrap_or_else(|_| "[]".into());
        let resp = self
            .http
            .get(&url)
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", timeout_secs.to_string()),
                ("allowed_updates", allowed_json),
            ])
            .timeout(Duration::from_secs(
                timeout_secs + LONGPOLL_CLIENT_BUFFER_SECS,
            ))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Api {
                method: "getUpdates".into(),
                code: status.as_u16() as i32,
                description: self.redact(body),
            });
        }
        let parsed: UpdatesResponse = serde_json::from_str(&body)?;
        if !parsed.ok {
            return Err(Error::Api {
                method: "getUpdates".into(),
                code: parsed.error_code.unwrap_or(0),
                description: self.redact(parsed.description.clone().unwrap_or_default()),
            });
        }
        Ok(parsed)
    }

    /// Generic JSON POST with 429 single-retry. Returns the raw `result` Value so callers can deserialise the variant they expect.
    pub async fn call_json<T: Serialize + ?Sized>(
        &self,
        method: &str,
        payload: &T,
    ) -> Result<Value> {
        let url = format!("{}/{method}", self.api_root);
        for attempt in 0..2 {
            let resp = self.http.post(&url).json(payload).send().await?;
            let status = resp.status();
            let body = resp.text().await?;
            if status.is_success() {
                let parsed: ApiResponse<Value> = serde_json::from_str(&body)?;
                if parsed.ok {
                    return Ok(parsed.result.unwrap_or(Value::Null));
                }
                let retry_after = parsed
                    .parameters
                    .as_ref()
                    .and_then(|p| p.retry_after)
                    .unwrap_or(RETRY_AFTER_DEFAULT_SECS);
                if attempt == 0
                    && parsed.error_code == Some(429)
                    && retry_after <= MAX_RETRY_AFTER_SECS
                {
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }
                return Err(Error::Api {
                    method: method.into(),
                    code: parsed.error_code.unwrap_or(0),
                    description: self.redact(parsed.description.unwrap_or_default()),
                });
            } else if status.as_u16() == 429 && attempt == 0 {
                // Some 429s come back with non-2xx HTTP status; honour Retry-After header or fall back to default — but cap so a multi-hour flood-wait doesn't stall the loop.
                let retry_after = resp_retry_after_default(body.as_str());
                if retry_after > MAX_RETRY_AFTER_SECS {
                    return Err(Error::Api {
                        method: method.into(),
                        code: 429,
                        description: self.redact(format!("retry_after={retry_after}s exceeds cap")),
                    });
                }
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                continue;
            } else {
                return Err(Error::Api {
                    method: method.into(),
                    code: status.as_u16() as i32,
                    description: self.redact(body),
                });
            }
        }
        unreachable!(
            "call_json loop body either returns or `continue`s; the for-2 range cannot exhaust"
        )
    }

    pub async fn call_typed<T: Serialize + ?Sized, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        payload: &T,
    ) -> Result<R> {
        let v = self.call_json(method, payload).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
        reply_markup: Option<Value>,
    ) -> Result<SendMessageResult> {
        let mut payload = json!({ "chat_id": chat_id, "text": text });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        if let Some(rm) = reply_markup {
            payload["reply_markup"] = rm;
        }
        self.call_typed("sendMessage", &payload).await
    }

    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        reply_markup: Option<Value>,
    ) -> Result<Value> {
        let mut payload = json!({
            "chat_id": chat_id, "message_id": message_id, "text": text,
        });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(rm) = reply_markup {
            payload["reply_markup"] = rm;
        }
        self.call_json("editMessageText", &payload).await
    }

    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<Value> {
        let payload = json!({ "chat_id": chat_id, "message_id": message_id });
        self.call_json("deleteMessage", &payload).await
    }

    pub async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<Value> {
        let payload = json!({ "chat_id": chat_id, "action": action });
        self.call_json("sendChatAction", &payload).await
    }

    pub async fn send_photo_url(
        &self,
        chat_id: i64,
        photo_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "photo": photo_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendPhoto", &payload).await
    }

    pub async fn send_document_url(
        &self,
        chat_id: i64,
        document_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "document": document_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendDocument", &payload).await
    }

    pub async fn send_voice_url(
        &self,
        chat_id: i64,
        voice_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "voice": voice_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendVoice", &payload).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_audio_url(
        &self,
        chat_id: i64,
        audio_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        title: Option<&str>,
        performer: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "audio": audio_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = title {
            payload["title"] = json!(t);
        }
        if let Some(p) = performer {
            payload["performer"] = json!(p);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendAudio", &payload).await
    }

    pub async fn send_video_url(
        &self,
        chat_id: i64,
        video_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "video": video_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendVideo", &payload).await
    }

    pub async fn send_animation_url(
        &self,
        chat_id: i64,
        animation_url: &str,
        caption: Option<&str>,
        parse_mode: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "animation": animation_url });
        if let Some(pm) = parse_mode {
            payload["parse_mode"] = json!(pm);
        }
        if let Some(c) = caption {
            payload["caption"] = json!(c);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendAnimation", &payload).await
    }

    pub async fn send_sticker_file_id(
        &self,
        chat_id: i64,
        file_id: &str,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "sticker": file_id });
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendSticker", &payload).await
    }

    pub async fn send_location(
        &self,
        chat_id: i64,
        latitude: f64,
        longitude: f64,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({
            "chat_id": chat_id, "latitude": latitude, "longitude": longitude,
        });
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendLocation", &payload).await
    }

    pub async fn send_media_group(
        &self,
        chat_id: i64,
        media: Value,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let mut payload = json!({ "chat_id": chat_id, "media": media });
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        self.call_json("sendMediaGroup", &payload).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_poll(
        &self,
        chat_id: i64,
        question: &str,
        options: Vec<Value>,
        is_quiz: bool,
        correct_option_id: Option<u32>,
        explanation: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<PollResult> {
        let mut payload = json!({
            "chat_id": chat_id, "question": question, "options": options,
            "type": if is_quiz { "quiz" } else { "regular" },
        });
        if let Some(id) = correct_option_id {
            payload["correct_option_id"] = json!(id);
        }
        if let Some(e) = explanation {
            payload["explanation"] = json!(e);
        }
        if let Some(t) = thread_id {
            payload["message_thread_id"] = json!(t);
        }
        let result_envelope: Value = self.call_json("sendPoll", &payload).await?;
        // `result.poll.id` is required by Bot API. `unwrap_or_default` would hide a real protocol regression behind an empty string that downstream callers (e.g. agents waiting on the poll's id to record the answer) would treat as a valid id; surface an error instead.
        let poll_id = result_envelope
            .get("poll")
            .and_then(|p| p.get("id"))
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Other("sendPoll: result.poll.id missing".into()))?;
        Ok(PollResult { id: poll_id })
    }

    pub async fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        reactions: Vec<Value>,
    ) -> Result<Value> {
        let payload = json!({
            "chat_id": chat_id, "message_id": message_id, "reaction": reactions,
        });
        self.call_json("setMessageReaction", &payload).await
    }

    pub async fn answer_callback_query(&self, callback_query_id: &str) -> Result<Value> {
        let payload = json!({ "callback_query_id": callback_query_id });
        self.call_json("answerCallbackQuery", &payload).await
    }

    pub async fn get_file(&self, file_id: &str) -> Result<GetFileResult> {
        let payload = json!({ "file_id": file_id });
        self.call_typed("getFile", &payload).await
    }

    /// Multi-part upload for inline file bytes. Used by Content::FileData and the private-URL fetch-and-upload paths. Honours the same single 429 retry the module doc-comment promises.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_multipart(
        &self,
        method: &str,
        chat_id: i64,
        file_field: &str,
        filename: String,
        bytes: Vec<u8>,
        mime_type: Option<String>,
        extra: Vec<(&str, String)>,
        thread_id: Option<i64>,
    ) -> Result<Value> {
        let url = format!("{}/{method}", self.api_root);
        // reqwest::multipart::Form consumes its parts when sent. To support the rare 429 retry without keeping a streamable body, we clone `bytes` for each attempt; the trade-off is ~1 extra Vec<u8> heap copy on the happy path in exchange for a working retry path without async-body rewinding.
        for attempt in 0..2 {
            let mut part =
                reqwest::multipart::Part::bytes(bytes.clone()).file_name(filename.clone());
            if let Some(mt) = mime_type.as_ref() {
                part = part
                    .mime_str(mt)
                    .map_err(|e| Error::Other(format!("multipart mime: {e}")))?;
            }
            let mut form = reqwest::multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part(file_field.to_string(), part);
            for (k, v) in &extra {
                form = form.text(k.to_string(), v.clone());
            }
            if let Some(t) = thread_id {
                form = form.text("message_thread_id", t.to_string());
            }
            let resp = self.http.post(&url).multipart(form).send().await?;
            let status = resp.status();
            let body = resp.text().await?;
            if status.is_success() {
                let parsed: ApiResponse<Value> = serde_json::from_str(&body)?;
                if parsed.ok {
                    return Ok(parsed.result.unwrap_or(Value::Null));
                }
                let retry_after = parsed
                    .parameters
                    .as_ref()
                    .and_then(|p| p.retry_after)
                    .unwrap_or(RETRY_AFTER_DEFAULT_SECS);
                if attempt == 0
                    && parsed.error_code == Some(429)
                    && retry_after <= MAX_RETRY_AFTER_SECS
                {
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }
                return Err(Error::Api {
                    method: method.into(),
                    code: parsed.error_code.unwrap_or(0),
                    description: self.redact(parsed.description.unwrap_or_default()),
                });
            } else if status.as_u16() == 429 && attempt == 0 {
                let retry_after = resp_retry_after_default(body.as_str());
                if retry_after > MAX_RETRY_AFTER_SECS {
                    return Err(Error::Api {
                        method: method.into(),
                        code: 429,
                        description: self.redact(format!("retry_after={retry_after}s exceeds cap")),
                    });
                }
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                continue;
            } else {
                return Err(Error::Api {
                    method: method.into(),
                    code: status.as_u16() as i32,
                    description: self.redact(body),
                });
            }
        }
        unreachable!("send_multipart loop body either returns or `continue`s; the for-2 range cannot exhaust")
    }
}

/// Try to parse a Retry-After value out of a non-2xx 429 body, falling back to the default.
fn resp_retry_after_default(body: &str) -> u64 {
    serde_json::from_str::<ApiResponse<Value>>(body)
        .ok()
        .and_then(|p| p.parameters.and_then(|x| x.retry_after))
        .unwrap_or(RETRY_AFTER_DEFAULT_SECS)
}
