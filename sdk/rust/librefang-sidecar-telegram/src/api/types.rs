//! Telegram Bot API value types — only what the adapter actually reads from `getUpdates` responses or writes into outbound calls.
//!
//! Field names mirror the Bot API (snake_case) so serde defaults work without `rename`.
//! Every optional field uses `#[serde(default)]` so the supervisor never drops an event because a future Bot API release added an extra field at the top of an existing struct.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct UpdatesResponse {
    pub ok: bool,
    pub result: Vec<Update>,
    pub description: Option<String>,
    pub error_code: Option<i32>,
    pub parameters: Option<ResponseParameters>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ResponseParameters {
    pub retry_after: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub edited_message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
    pub poll_answer: Option<PollAnswer>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Message {
    pub message_id: i64,
    pub message_thread_id: Option<i64>,
    pub from: Option<User>,
    pub sender_chat: Option<Chat>,
    pub date: i64,
    pub chat: Chat,
    pub reply_to_message: Option<Box<Message>>,
    pub text: Option<String>,
    pub entities: Vec<MessageEntity>,
    pub caption: Option<String>,
    pub caption_entities: Vec<MessageEntity>,
    pub photo: Vec<PhotoSize>,
    pub document: Option<Document>,
    pub audio: Option<Audio>,
    pub voice: Option<Voice>,
    pub animation: Option<Animation>,
    pub video: Option<Video>,
    pub video_note: Option<VideoNote>,
    pub sticker: Option<Sticker>,
    pub location: Option<Location>,
    pub contact: Option<Contact>,
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
    pub error_code: Option<i32>,
    pub parameters: Option<ResponseParameters>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
}
