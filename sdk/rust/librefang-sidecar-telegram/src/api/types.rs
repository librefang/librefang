//! Telegram Bot API value types — only what the adapter actually reads from `getUpdates` responses or writes into outbound calls.
//!
//! Field names mirror the Bot API (snake_case) so serde defaults work without `rename`.
//! Every optional field uses `#[serde(default)]` so the supervisor never drops an event because a future Bot API release added an extra field at the top of an existing struct.

use serde::{Deserialize, Serialize};

pub type UpdatesResponse = ApiResponse<Vec<Update>>;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ResponseParameters {
    pub retry_after: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub edited_message: Option<Message>,
    #[serde(default)]
    pub callback_query: Option<CallbackQuery>,
    #[serde(default)]
    pub poll_answer: Option<PollAnswer>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Message {
    pub message_id: i64,
    #[serde(default)]
    pub message_thread_id: Option<i64>,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub sender_chat: Option<Chat>,
    #[allow(dead_code)]
    // Required by Telegram's Message contract even though translation does not expose the timestamp.
    pub date: i64,
    #[serde(default)]
    pub edit_date: Option<i64>,
    pub chat: Chat,
    #[serde(default)]
    pub reply_to_message: Option<Box<Message>>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub entities: Vec<MessageEntity>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    // Retained for wire completeness; captions currently use plain text only.
    pub caption_entities: Vec<MessageEntity>,
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    #[serde(default)]
    pub document: Option<Document>,
    #[serde(default)]
    pub audio: Option<Audio>,
    #[serde(default)]
    pub voice: Option<Voice>,
    #[serde(default)]
    pub animation: Option<Animation>,
    #[serde(default)]
    pub video: Option<Video>,
    #[serde(default)]
    pub video_note: Option<VideoNote>,
    #[serde(default)]
    pub sticker: Option<Sticker>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub contact: Option<Contact>,
    #[serde(default)]
    #[allow(dead_code)] // Validated for compatibility; routing uses message_thread_id.
    pub is_topic_message: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub is_forum: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct MessageEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub offset: i64,
    pub length: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PhotoSize {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: u32,
    pub height: u32,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Document {
    pub file_id: String,
    pub file_unique_id: String,
    pub thumbnail: Option<PhotoSize>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Audio {
    pub file_id: String,
    pub duration: u32,
    pub performer: Option<String>,
    pub title: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Voice {
    pub file_id: String,
    pub duration: u32,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Animation {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub duration: u32,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Video {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub duration: u32,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct VideoNote {
    pub file_id: String,
    pub length: u32,
    pub duration: u32,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Sticker {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: u32,
    pub height: u32,
    pub is_animated: bool,
    pub is_video: bool,
    pub emoji: Option<String>,
    pub set_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Contact {
    pub phone_number: String,
    pub first_name: String,
    pub last_name: Option<String>,
    pub user_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CallbackQuery {
    pub id: String,
    pub from: Option<User>,
    pub message: Option<Message>,
    pub chat_instance: String,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PollAnswer {
    pub poll_id: String,
    pub user: Option<User>,
    pub option_ids: Vec<u32>,
}

// ── Response envelopes for "send" endpoints ─────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<T>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub error_code: Option<i32>,
    #[serde(default)]
    pub parameters: Option<ResponseParameters>,
}

impl<T> Default for ApiResponse<T> {
    fn default() -> Self {
        Self {
            ok: false,
            result: None,
            description: None,
            error_code: None,
            parameters: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SendMessageResult {
    pub message_id: i64,
    pub chat: Chat,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GetFileResult {
    pub file_id: String,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct PollResult {
    pub id: String,
}

// ── Outbound types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    #[serde(flatten)]
    pub action: InlineKeyboardAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum InlineKeyboardAction {
    Url { url: String },
    CallbackData { callback_data: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct NoDefault;

    #[test]
    fn api_response_default_does_not_require_result_default() {
        let response = ApiResponse::<NoDefault>::default();
        assert!(!response.ok);
        assert!(response.result.is_none());
        assert!(response.description.is_none());
        assert!(response.error_code.is_none());
        assert!(response.parameters.is_none());
    }

    #[test]
    fn inline_keyboard_action_serializes_as_exactly_one_bot_api_field() {
        let url = InlineKeyboardButton {
            text: "Docs".into(),
            action: InlineKeyboardAction::Url {
                url: "https://example.com".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(url).unwrap(),
            json!({"text": "Docs", "url": "https://example.com"})
        );

        let callback = InlineKeyboardButton {
            text: "Run".into(),
            action: InlineKeyboardAction::CallbackData {
                callback_data: "run".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(callback).unwrap(),
            json!({"text": "Run", "callback_data": "run"})
        );
    }

    #[test]
    fn updates_response_is_the_generic_telegram_envelope() {
        fn as_updates(response: ApiResponse<Vec<Update>>) -> UpdatesResponse {
            response
        }

        let response: ApiResponse<Vec<Update>> = serde_json::from_value(json!({
            "ok": true,
            "result": []
        }))
        .expect("generic update envelope");
        let response = as_updates(response);
        assert!(response.ok);
        assert!(response.result.as_ref().is_some_and(Vec::is_empty));
    }

    #[test]
    fn required_update_identity_fields_fail_closed() {
        assert!(serde_json::from_value::<UpdatesResponse>(json!({"result": []})).is_err());
        assert!(serde_json::from_value::<Update>(json!({})).is_err());
        assert!(serde_json::from_value::<Message>(json!({
            "message_id": 1,
            "date": 2
        }))
        .is_err());

        let error_response: UpdatesResponse = serde_json::from_value(json!({
            "ok": false,
            "error_code": 429,
            "description": "retry later"
        }))
        .expect("Telegram error envelopes may omit result");
        assert!(error_response.result.is_none());

        let response: UpdatesResponse = serde_json::from_value(json!({
            "ok": true,
            "result": [{
                "update_id": 42,
                "message": {
                    "message_id": 7,
                    "date": 123,
                    "chat": {"id": 9, "type": "private"}
                }
            }]
        }))
        .expect("optional update and message fields may be absent");
        let updates = response.result.expect("successful response result");
        assert_eq!(updates[0].update_id, 42);
        assert_eq!(updates[0].message.as_ref().unwrap().chat.id, 9);
    }
}
