//! Wire protocol for LibreFang sidecar channel adapters.
//!
//! Newline-delimited JSON over stdio:
//!
//! * **Events** (adapter → LibreFang, written to **stdout**): `ready`, `message`, `error`, `typing`, `qr_ready`, `qr_status`.
//! * **Commands** (LibreFang → adapter, read from **stdin**): `send`, `ready_ack`, `typing`, `reaction`, `interactive`, `stream_start`, `stream_delta`, `stream_end`, `heartbeat`, `shutdown`.
//!
//! Wire shape is byte-equivalent with the Python SDK (`librefang.sidecar.protocol`) and with the Rust supervisor (`crates/librefang-channels/src/sidecar.rs`).
//! The three implementations are kept honest against each other by the shared corpus at `conformance/sidecar/corpus/` — see the integration test `tests/conformance.rs`.
//!
//! See `docs/architecture/sidecar-protocol.md` for the normative spec.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── ChannelContent builders ─────────────────────────────────────────
//
// `Content::*` mirrors the externally-tagged `crate::types::ChannelContent` enum in `librefang-channels`.
// We return `Value` instead of defining our own typed enum to avoid drifting from the kernel-side source of truth — the conformance corpus is the executable contract.
// Adapter authors who want a typed variant can construct a struct that serializes to the same shape.

/// Builders for every `ChannelContent` variant the wire accepts.
pub struct Content;

impl Content {
    pub fn text(s: impl Into<String>) -> Value {
        json!({"Text": s.into()})
    }

    pub fn image(
        url: impl Into<String>,
        caption: Option<String>,
        mime_type: Option<String>,
    ) -> Value {
        json!({"Image": {"url": url.into(), "caption": caption, "mime_type": mime_type}})
    }

    pub fn file(url: impl Into<String>, filename: impl Into<String>) -> Value {
        json!({"File": {"url": url.into(), "filename": filename.into()}})
    }

    /// Inline file bytes. Mirrors Rust `FileData{data: Vec<u8>}`: serde
    /// emits the bytes as a JSON number array (no base64 shim). Prefer
    /// [`Content::file`] with a URL for large payloads.
    pub fn file_data(
        data: &[u8],
        filename: impl Into<String>,
        mime_type: impl Into<String>,
    ) -> Value {
        json!({
            "FileData": {
                "data": data.to_vec(),
                "filename": filename.into(),
                "mime_type": mime_type.into(),
            }
        })
    }

    pub fn voice(url: impl Into<String>, caption: Option<String>, duration_seconds: u32) -> Value {
        json!({"Voice": {"url": url.into(), "caption": caption, "duration_seconds": duration_seconds}})
    }

    pub fn video(
        url: impl Into<String>,
        caption: Option<String>,
        duration_seconds: u32,
        filename: Option<String>,
    ) -> Value {
        json!({
            "Video": {
                "url": url.into(), "caption": caption,
                "duration_seconds": duration_seconds, "filename": filename,
            }
        })
    }

    pub fn location(lat: f64, lon: f64) -> Value {
        json!({"Location": {"lat": lat, "lon": lon}})
    }

    pub fn command(name: impl Into<String>, args: Vec<String>) -> Value {
        json!({"Command": {"name": name.into(), "args": args}})
    }

    /// `buttons` is a 2D grid: outer = rows, inner = buttons in that row.
    /// Build each button with [`Content::button`].
    pub fn interactive(text: impl Into<String>, buttons: Vec<Vec<Value>>) -> Value {
        json!({"Interactive": {"text": text.into(), "buttons": buttons}})
    }

    pub fn button_callback(action: impl Into<String>, message_text: Option<String>) -> Value {
        json!({"ButtonCallback": {"action": action.into(), "message_text": message_text}})
    }

    pub fn delete_message(message_id: impl Into<String>) -> Value {
        json!({"DeleteMessage": {"message_id": message_id.into()}})
    }

    pub fn edit_interactive(
        message_id: impl Into<String>,
        text: impl Into<String>,
        buttons: Vec<Vec<Value>>,
    ) -> Value {
        json!({"EditInteractive": {"message_id": message_id.into(), "text": text.into(), "buttons": buttons}})
    }

    pub fn audio(
        url: impl Into<String>,
        caption: Option<String>,
        duration_seconds: u32,
        title: Option<String>,
        performer: Option<String>,
    ) -> Value {
        let mut p = json!({
            "url": url.into(), "caption": caption,
            "duration_seconds": duration_seconds,
        });
        if let Some(t) = title {
            p["title"] = Value::String(t);
        }
        if let Some(perf) = performer {
            p["performer"] = Value::String(perf);
        }
        json!({"Audio": p})
    }

    pub fn animation(
        url: impl Into<String>,
        caption: Option<String>,
        duration_seconds: u32,
    ) -> Value {
        json!({"Animation": {"url": url.into(), "caption": caption, "duration_seconds": duration_seconds}})
    }

    pub fn sticker(file_id: impl Into<String>) -> Value {
        json!({"Sticker": {"file_id": file_id.into()}})
    }

    pub fn media_group(items: Vec<Value>) -> Value {
        json!({"MediaGroup": {"items": items}})
    }

    pub fn poll(
        question: impl Into<String>,
        options: Vec<String>,
        is_quiz: bool,
        correct_option_id: Option<u32>,
        explanation: Option<String>,
    ) -> Value {
        let mut p = json!({"question": question.into(), "options": options, "is_quiz": is_quiz});
        if let Some(id) = correct_option_id {
            p["correct_option_id"] = json!(id);
        }
        if let Some(e) = explanation {
            p["explanation"] = Value::String(e);
        }
        json!({"Poll": p})
    }

    pub fn poll_answer(poll_id: impl Into<String>, option_ids: Vec<i64>) -> Value {
        json!({"PollAnswer": {"poll_id": poll_id.into(), "option_ids": option_ids}})
    }

    /// One `InteractiveButton` for use in [`Content::interactive`] /
    /// [`Content::edit_interactive`]. `style` and `url` are skipped when
    /// `None` (matches the Python SDK + kernel serde shape).
    pub fn button(
        label: impl Into<String>,
        action: impl Into<String>,
        style: Option<String>,
        url: Option<String>,
    ) -> Value {
        let mut b = json!({"label": label.into(), "action": action.into()});
        if let Some(s) = style {
            b["style"] = Value::String(s);
        }
        if let Some(u) = url {
            b["url"] = Value::String(u);
        }
        b
    }
}

// ── ChannelUser ────────────────────────────────────────────────────

/// Sender identity carried on `send.params.user` and consumable inside
/// adapter code. `librefang_user` is serialized as `null` when absent
/// — it is *not* skipped — matching `crate::types::ChannelUser` in
/// `librefang-channels`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ChannelUser {
    pub platform_id: String,
    pub display_name: String,
    pub librefang_user: Option<String>,
}

// ── Outbound events (adapter → daemon) ─────────────────────────────
//
// Helpers return `Value` so the caller can write a newline-delimited
// JSON line directly via `serde_json::to_string` + push `'\n'`. The
// `run` / `run_stdio` runtime does this transparently.

pub mod events {
    use super::*;

    /// Build a `ready` event declaring the adapter's capabilities.
    /// Capability strings gate the matching `ChannelAdapter` methods
    /// on the daemon side: `typing`, `reaction`, `interactive`,
    /// `thread`, `streaming`, `typing_events`.
    pub fn ready(
        capabilities: Vec<String>,
        account_id: Option<String>,
        suppress_error_responses: bool,
        notification_recipients: Vec<Value>,
        header_rules: Vec<Value>,
        protocol_version: Option<u32>,
    ) -> Value {
        json!({
            "method": "ready",
            "params": {
                "capabilities": capabilities,
                "account_id": account_id,
                "suppress_error_responses": suppress_error_responses,
                "notification_recipients": notification_recipients,
                "header_rules": header_rules,
                "protocol_version": protocol_version,
            },
        })
    }

    /// Build a `message` event.
    ///
    /// `content` (a [`super::Content`] result) supersedes `text`; legacy text-only adapters may pass only `text`.
    /// Plain-text `content` is mirrored into `text` so a pre-#5219 daemon still delivers the message non-empty.
    #[allow(clippy::too_many_arguments)]
    pub fn message(
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        text: Option<String>,
        content: Option<Value>,
        message_id: Option<String>,
        channel_id: Option<String>,
        platform: Option<String>,
        username: Option<String>,
        librefang_user: Option<String>,
        is_group: bool,
        thread_id: Option<String>,
        group_members: Option<Vec<Value>>,
        group_participants: Option<Vec<Value>>,
        metadata: Option<serde_json::Map<String, Value>>,
    ) -> Value {
        let mut params = serde_json::Map::new();
        params.insert("user_id".into(), Value::String(user_id.into()));
        params.insert("user_name".into(), Value::String(user_name.into()));
        // Back-compat with pre-#5219 daemon: mirror `Content::text` into
        // `text` so a content-only plain-text message still has a text
        // payload for a legacy reader. Non-text content cannot be
        // flattened losslessly.
        let mut effective_text = text;
        if let Some(ref c) = content {
            if effective_text.is_none() {
                if let Some(s) = c.get("Text").and_then(Value::as_str) {
                    effective_text = Some(s.to_string());
                }
            }
        }
        if let Some(t) = effective_text {
            params.insert("text".into(), Value::String(t));
        }
        if let Some(c) = content {
            params.insert("content".into(), c);
        }
        if let Some(mid) = message_id {
            params.insert("message_id".into(), Value::String(mid));
        }
        if let Some(c) = channel_id {
            params.insert("channel_id".into(), Value::String(c));
        }
        if let Some(p) = platform {
            params.insert("platform".into(), Value::String(p));
        }
        if let Some(u) = username {
            params.insert("username".into(), Value::String(u));
        }
        if let Some(lu) = librefang_user {
            params.insert("librefang_user".into(), Value::String(lu));
        }
        if is_group {
            params.insert("is_group".into(), Value::Bool(true));
        }
        if let Some(t) = thread_id {
            params.insert("thread_id".into(), Value::String(t));
        }
        if let Some(gm) = group_members {
            if !gm.is_empty() {
                params.insert("group_members".into(), Value::Array(gm));
            }
        }
        if let Some(gp) = group_participants {
            if !gp.is_empty() {
                params.insert("group_participants".into(), Value::Array(gp));
            }
        }
        if let Some(m) = metadata {
            if !m.is_empty() {
                params.insert("metadata".into(), Value::Object(m));
            }
        }
        json!({"method": "message", "params": Value::Object(params)})
    }

    /// Minimal `message` builder — most adapters should use [`message`]
    /// or the [`MessageBuilder`] (see below). Kept for ergonomic
    /// text-only emit.
    pub fn message_text(
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        text: impl Into<String>,
    ) -> Value {
        json!({"method": "message", "params": {
            "user_id": user_id.into(),
            "user_name": user_name.into(),
            "text": text.into(),
        }})
    }

    pub fn error(msg: impl Into<String>) -> Value {
        json!({"method": "error", "params": {"message": msg.into()}})
    }

    pub fn typing(
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        is_typing: bool,
    ) -> Value {
        json!({"method": "typing", "params": {
            "user_id": user_id.into(), "user_name": user_name.into(), "is_typing": is_typing,
        }})
    }

    pub fn qr_ready(
        qr_code: impl Into<String>,
        qr_url: Option<String>,
        message: Option<String>,
        expires_at: Option<String>,
    ) -> Value {
        let mut params = json!({"qr_code": qr_code.into()});
        if let Some(u) = qr_url {
            params["qr_url"] = Value::String(u);
        }
        if let Some(m) = message {
            params["message"] = Value::String(m);
        }
        if let Some(e) = expires_at {
            params["expires_at"] = Value::String(e);
        }
        json!({"method": "qr_ready", "params": params})
    }

    pub fn qr_status(status: impl Into<String>, message: Option<String>) -> Value {
        let mut params = json!({"status": status.into()});
        if let Some(m) = message {
            params["message"] = Value::String(m);
        }
        json!({"method": "qr_status", "params": params})
    }
}

/// Fluent builder for the rich `message` event — friendlier than `events::message(...)` when most fields are absent.
///
/// ```ignore
/// use librefang_sidecar::{Content, MessageBuilder};
/// let ev = MessageBuilder::new("42", "Alice")
///     .text("hello")
///     .content(Content::text("hello"))
///     .channel_id("-100123")
///     .platform("telegram")
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct MessageBuilder {
    user_id: String,
    user_name: String,
    text: Option<String>,
    content: Option<Value>,
    message_id: Option<String>,
    channel_id: Option<String>,
    platform: Option<String>,
    username: Option<String>,
    librefang_user: Option<String>,
    is_group: bool,
    thread_id: Option<String>,
    group_members: Option<Vec<Value>>,
    group_participants: Option<Vec<Value>>,
    metadata: Option<serde_json::Map<String, Value>>,
}

impl MessageBuilder {
    pub fn new(user_id: impl Into<String>, user_name: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: user_name.into(),
            ..Default::default()
        }
    }
    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.text = Some(t.into());
        self
    }
    pub fn content(mut self, c: Value) -> Self {
        self.content = Some(c);
        self
    }
    pub fn message_id(mut self, id: impl Into<String>) -> Self {
        self.message_id = Some(id.into());
        self
    }
    pub fn channel_id(mut self, id: impl Into<String>) -> Self {
        self.channel_id = Some(id.into());
        self
    }
    pub fn platform(mut self, p: impl Into<String>) -> Self {
        self.platform = Some(p.into());
        self
    }
    pub fn username(mut self, u: impl Into<String>) -> Self {
        self.username = Some(u.into());
        self
    }
    pub fn librefang_user(mut self, u: impl Into<String>) -> Self {
        self.librefang_user = Some(u.into());
        self
    }
    pub fn is_group(mut self, g: bool) -> Self {
        self.is_group = g;
        self
    }
    pub fn thread_id(mut self, t: impl Into<String>) -> Self {
        self.thread_id = Some(t.into());
        self
    }
    pub fn group_members(mut self, m: Vec<Value>) -> Self {
        self.group_members = Some(m);
        self
    }
    pub fn group_participants(mut self, p: Vec<Value>) -> Self {
        self.group_participants = Some(p);
        self
    }
    pub fn metadata(mut self, m: serde_json::Map<String, Value>) -> Self {
        self.metadata = Some(m);
        self
    }
    pub fn build(self) -> Value {
        events::message(
            self.user_id,
            self.user_name,
            self.text,
            self.content,
            self.message_id,
            self.channel_id,
            self.platform,
            self.username,
            self.librefang_user,
            self.is_group,
            self.thread_id,
            self.group_members,
            self.group_participants,
            self.metadata,
        )
    }
}

// ── Inbound commands (daemon → adapter) ─────────────────────────────

/// One inbound interactive button — appears nested inside
/// [`InteractiveMessage::buttons`]. `style` and `url` are skipped when
/// absent on the wire, so `serde` defaults them to `None`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct InteractiveButton {
    pub label: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Outer shape of the `interactive` command's `message` payload.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct InteractiveMessage {
    pub text: String,
    pub buttons: Vec<Vec<InteractiveButton>>,
}

/// Params of the inbound `send` command (the daemon asking the adapter
/// to deliver a message to its platform).
///
/// Named `SendCommand` (not `Send`) so the SDK does not shadow
/// `std::marker::Send` for any caller that uses
/// `use librefang_sidecar::*;` — a glob import of a struct called `Send`
/// breaks every `T: Send` trait bound with cryptic diagnostics. The
/// `method` field on the wire is still `"send"`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct SendCommand {
    pub channel_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub user: ChannelUser,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct TypingCmd {
    pub channel_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Reaction {
    pub channel_id: String,
    pub message_id: String,
    pub reaction: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct Interactive {
    pub channel_id: String,
    pub message: InteractiveMessage,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct StreamStart {
    pub channel_id: String,
    pub stream_id: String,
    /// Thread to stream the reply into; `None` for a top-level reply.
    /// The post-#5219 daemon emits this for threaded streamed replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct StreamDelta {
    pub stream_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct StreamEnd {
    pub stream_id: String,
}

/// Forward-compat envelope: a command method this SDK version does not model.
/// Adapters must tolerate these rather than crash — the daemon relies on the symmetry (e.g. it sends `ready_ack` to older adapters that don't know the method).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCommand {
    pub method: String,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Send(SendCommand),
    ReadyAck,
    Shutdown,
    Heartbeat,
    Typing(TypingCmd),
    Reaction(Reaction),
    Interactive(Interactive),
    StreamStart(StreamStart),
    StreamDelta(StreamDelta),
    StreamEnd(StreamEnd),
    Unknown(UnknownCommand),
}

/// Parsed envelope shape — internal helper used by [`parse_command`].
#[derive(Deserialize)]
struct Envelope {
    method: String,
    #[serde(default)]
    params: Value,
}

/// Parse one stdin line into a typed [`Command`].
///
/// Returns `Err` on malformed JSON, on a JSON value that is not an object (a bare number/array), AND on a typed-params variant whose `params` shape does not deserialize.
/// The reader loop in [`crate::runtime::run`] catches all three, emits a protocol-level `error` event with the deserialization message, and continues — so a wire-shape skew is surfaced instead of silently degrading the affected command to default values.
///
/// Unknown `method` strings become [`Command::Unknown`] rather than an error, so a newer daemon can introduce a method without breaking an older adapter.
pub fn parse_command(line: &str) -> Result<Command, serde_json::Error> {
    let v: Value = serde_json::from_str(line)?;
    if !v.is_object() {
        // Mirror Python's behavior: surface as a JSON error so the runtime can react identically across implementations.
        return Err(serde::de::Error::custom("expected a JSON object"));
    }
    let env: Envelope = serde_json::from_value(v.clone())?;
    let params = env.params;
    let params_or_empty = if params.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        params
    };
    let cmd = match env.method.as_str() {
        "send" => Command::Send(serde_json::from_value(params_or_empty)?),
        "ready_ack" => Command::ReadyAck,
        "shutdown" => Command::Shutdown,
        "heartbeat" => Command::Heartbeat,
        "typing" => Command::Typing(serde_json::from_value(params_or_empty)?),
        "reaction" => Command::Reaction(serde_json::from_value(params_or_empty)?),
        "interactive" => Command::Interactive(serde_json::from_value(params_or_empty)?),
        "stream_start" => Command::StreamStart(serde_json::from_value(params_or_empty)?),
        "stream_delta" => Command::StreamDelta(serde_json::from_value(params_or_empty)?),
        "stream_end" => Command::StreamEnd(serde_json::from_value(params_or_empty)?),
        other => Command::Unknown(UnknownCommand {
            method: other.to_string(),
            raw: v,
        }),
    };
    Ok(cmd)
}

// ── Self-description schema (--describe) ────────────────────────────
//
// Emitted by `cargo run --example <adapter> -- --describe` or the
// adapter's binary equivalent. Mirrors the FieldType enum in
// librefang-api's CHANNEL_REGISTRY so the dashboard can render either
// kind of channel with one form component.

/// Field type for the dashboard's schema-driven config form.
/// `Secret` is routed to `~/.librefang/secrets.env` on save (never written to `config.toml`).
/// Every other type is stored in the `[sidecar_channels.env]` table of `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Secret,
    Number,
    List,
    Bool,
    Select,
}

#[derive(Debug, Clone, Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    pub required: bool,
    pub placeholder: String,
    pub advanced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl Field {
    pub fn new(key: impl Into<String>, label: impl Into<String>, field_type: FieldType) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            field_type,
            required: false,
            placeholder: String::new(),
            advanced: false,
            options: None,
        }
    }
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }
    pub fn options(mut self, opts: Vec<String>) -> Self {
        self.options = Some(opts);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Schema {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub fields: Vec<Field>,
}

impl Schema {
    pub fn new(
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        fields: Vec<Field>,
    ) -> Self {
        Self {
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            fields,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_text_shape() {
        assert_eq!(Content::text("hi"), json!({"Text": "hi"}));
    }

    #[test]
    fn content_button_skips_absent_optionals() {
        // Style/url omitted → no key in output, not null.
        let b = Content::button("Yes", "y", None, None);
        assert_eq!(b, json!({"label": "Yes", "action": "y"}));
        // Style/url present → emitted.
        let b = Content::button(
            "Docs",
            "d",
            Some("primary".into()),
            Some("https://x".into()),
        );
        assert_eq!(
            b,
            json!({"label": "Docs", "action": "d", "style": "primary", "url": "https://x"})
        );
    }

    #[test]
    fn channel_user_serializes_librefang_user_as_null() {
        let u = ChannelUser {
            platform_id: "c1".into(),
            display_name: "Bob".into(),
            librefang_user: None,
        };
        let v = serde_json::to_value(&u).unwrap();
        // librefang_user MUST appear as null, not be skipped — see
        // sidecar-protocol.md "ChannelUser" note.
        assert_eq!(
            v,
            json!({"platform_id": "c1", "display_name": "Bob", "librefang_user": null})
        );
    }

    #[test]
    fn send_minimal_omits_content_and_thread_id() {
        let s = SendCommand {
            channel_id: "c1".into(),
            text: "hi".into(),
            content: None,
            thread_id: None,
            user: ChannelUser {
                platform_id: "c1".into(),
                display_name: "Bob".into(),
                librefang_user: None,
            },
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(
            v.get("content").is_none(),
            "content must be omitted, not null"
        );
        assert!(
            v.get("thread_id").is_none(),
            "thread_id must be omitted, not null"
        );
    }

    #[test]
    fn parse_command_handles_bare_method() {
        // {"method": "shutdown"} with no params field must parse.
        assert_eq!(
            parse_command(r#"{"method":"shutdown"}"#).unwrap(),
            Command::Shutdown
        );
        assert_eq!(
            parse_command(r#"{"method":"heartbeat"}"#).unwrap(),
            Command::Heartbeat
        );
        assert_eq!(
            parse_command(r#"{"method":"ready_ack"}"#).unwrap(),
            Command::ReadyAck
        );
    }

    #[test]
    fn parse_command_handles_null_params() {
        // {"method":"shutdown","params":null} — JSON-RPC libs emit this
        // form for parameterless notifications. Must parse identically.
        assert_eq!(
            parse_command(r#"{"method":"shutdown","params":null}"#).unwrap(),
            Command::Shutdown
        );
    }

    #[test]
    fn parse_command_unknown_method_is_not_an_error() {
        let cmd = parse_command(r#"{"method":"future_method","params":{"x":1}}"#).unwrap();
        match cmd {
            Command::Unknown(u) => {
                assert_eq!(u.method, "future_method");
                assert_eq!(u.raw["params"]["x"], json!(1));
            }
            _ => panic!("expected Unknown"),
        }
    }

    #[test]
    fn parse_command_rejects_non_object() {
        // Bare number / array should error, matching Python's behavior.
        assert!(parse_command("42").is_err());
        assert!(parse_command("[1,2]").is_err());
    }

    #[test]
    fn parse_command_rejects_malformed_json() {
        assert!(parse_command("{not json").is_err());
    }

    #[test]
    fn message_builder_mirrors_content_text_into_text() {
        let v = MessageBuilder::new("u", "User")
            .content(Content::text("hello"))
            .build();
        // Content::text → params.content = {"Text": "hello"} AND
        // params.text = "hello" (back-compat with pre-#5219 daemon).
        assert_eq!(v["params"]["text"], json!("hello"));
        assert_eq!(v["params"]["content"], json!({"Text": "hello"}));
    }

    #[test]
    fn ready_event_shape() {
        let v = events::ready(vec!["typing".into()], None, false, vec![], vec![], Some(1));
        assert_eq!(v["method"], json!("ready"));
        assert_eq!(v["params"]["capabilities"], json!(["typing"]));
        assert_eq!(v["params"]["protocol_version"], json!(1));
    }
}
