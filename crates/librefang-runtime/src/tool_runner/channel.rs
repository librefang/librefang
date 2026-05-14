//! `channel_send` — proactive outbound messaging via configured adapters
//! (Telegram, Discord, email, …). Handles text, media (image/file URL or
//! local file path), and Telegram-style polls. On success, mirrors the
//! sent message back into the channel-owning agent's inbound session so
//! the model retains context for the user's reply (issue #4824).

use super::{check_taint_outbound_text, require_kernel, resolve_file_path_ext};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::path::Path;
use std::sync::Arc;

/// Parse and validate `poll_options` for the `channel_send` tool.
///
/// Telegram requires 2–10 string options per poll. A previous version used
/// `filter_map(as_str)` which silently dropped non-string entries — e.g.
/// `["a", 42, "c"]` became `["a", "c"]`, slipped past the min-2 check, and
/// sent a poll missing the user's third option. This helper fails fast
/// when any entry is the wrong type so the agent can surface the mistake
/// instead of producing a malformed poll.
fn parse_poll_options(raw: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    let arr = raw
        .and_then(|v| v.as_array())
        .ok_or_else(|| "poll_options must be an array of strings".to_string())?;
    let mut out: Vec<String> = Vec::with_capacity(arr.len());
    for (idx, v) in arr.iter().enumerate() {
        match v.as_str() {
            Some(s) => out.push(s.to_string()),
            None => {
                return Err(format!(
                    "poll_options[{idx}] must be a string, got {}",
                    match v {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "boolean",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                        serde_json::Value::String(_) => unreachable!(),
                    }
                ));
            }
        }
    }
    if !(2..=10).contains(&out.len()) {
        return Err(format!(
            "poll_options must have between 2 and 10 options, got {}",
            out.len()
        ));
    }
    Ok(out)
}

/// Mirror a successfully-sent `channel_send` message into the inbound-routing
/// session of the channel-owning agent so it has context for the user's reply.
///
/// This is **best-effort**: any failure is logged at `warn!` level and does NOT
/// propagate — the platform send already succeeded.
///
/// Decision summary (issue #4824):
/// 1. Mirror unconditionally — even when caller == channel owner.
/// 2. Role = `user` with a JSON envelope `{"mirror_from":"<agent>","body":"<text>"}` so
///    the block is visible in prompt context without polluting the system role.
///    JSON escaping prevents prompt-injection via crafted body content.
/// 3. Mirror on partial-failure (platform delivery succeeded, ack lost).
/// 4. Written directly to session storage; no adapter re-emit.
async fn mirror_channel_send_to_session(
    kh: &Arc<dyn KernelHandle>,
    caller_agent_id: Option<&str>,
    channel: &str,
    recipient: &str,
    body: &str,
) {
    use librefang_types::agent::SessionId;
    use librefang_types::message::{Message, MessageContent, Role};

    let owner_id = kh.resolve_channel_owner(channel, recipient);

    let owner = match owner_id {
        Some(id) => id,
        None => {
            // No channel-owning agent configured — nothing to mirror.
            tracing::debug!(
                channel,
                recipient,
                "channel_send mirror: no channel owner agent found, skipping"
            );
            return;
        }
    };

    // session_id mirrors the inbound-routing path in messaging.rs:
    // `SessionId::for_sender_scope(owner, channel, Some(recipient))`
    let session_id = SessionId::for_sender_scope(owner, channel, Some(recipient));

    // LOW: skip the mirror entirely when the caller is anonymous — an
    // "unknown" sender carries no useful context and could mislead the agent.
    let from = match caller_agent_id {
        Some(id) => id,
        None => {
            tracing::debug!(
                channel,
                recipient,
                "channel_send mirror: caller_agent_id is None, skipping mirror"
            );
            return;
        }
    };

    let sent_at = chrono::Utc::now();

    // Stable data contract (#4824): JSON envelope prevents prompt-injection
    // via crafted body text (e.g. `]: <injected>` or embedded newlines).
    // Both fields are JSON-string-escaped by serde_json::to_string.
    let mirror_text = format!(
        "{{\"mirror_from\":{},\"body\":{}}}",
        serde_json::to_string(from).unwrap_or_else(|_| "\"unknown\"".to_string()),
        serde_json::to_string(body).unwrap_or_else(|_| "\"\"".to_string()),
    );

    let msg = Message {
        role: Role::User,
        content: MessageContent::Text(mirror_text),
        pinned: false,
        timestamp: Some(sent_at),
    };

    // `append_to_session` uses `block_in_place` internally so it is safe
    // to call directly from an async context. Mirror is best-effort by
    // design (#4824 decision 3) — errors are logged inside the impl.
    kh.append_to_session(session_id, owner, msg);
}

pub(super) async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    sender_id: Option<&str>,
    caller_agent_id: Option<&str>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();

    // Use recipient from input, or fall back to sender_id from context
    // This allows agents to reply to the original sender without explicitly
    // knowing the platform-specific ID (e.g., Telegram chat_id)
    let recipient = input["recipient"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(sender_id)
        .ok_or("Missing 'recipient' parameter. When replying to the original sender, recipient is auto-filled — ensure channel_send is called in response to a message.")?
        .trim();

    if recipient.is_empty() {
        return Err("Recipient cannot be empty".to_string());
    }

    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());
    let account_id = input["account_id"].as_str().filter(|s| !s.is_empty());

    // Check for media content (image_url, file_url, or file_path)
    let image_url = input["image_url"].as_str().filter(|s| !s.is_empty());
    let file_url = input["file_url"].as_str().filter(|s| !s.is_empty());
    let file_path = input["file_path"].as_str().filter(|s| !s.is_empty());

    if let Some(url) = image_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        if let Some(c) = caption {
            if let Some(violation) = check_taint_outbound_text(c, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }
        let result = kh
            .send_channel_media(
                &channel, recipient, "image", url, caption, None, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            let body = caption.unwrap_or(url);
            mirror_channel_send_to_session(kh, caller_agent_id, &channel, recipient, body).await;
        }
        return result;
    }

    if let Some(url) = file_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        let filename = input["filename"].as_str();
        if let Some(c) = caption {
            if let Some(violation) = check_taint_outbound_text(c, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }
        let result = kh
            .send_channel_media(
                &channel, recipient, "file", url, caption, filename, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            let body = caption.unwrap_or(url);
            mirror_channel_send_to_session(kh, caller_agent_id, &channel, recipient, body).await;
        }
        return result;
    }

    // Local file attachment: read from disk and send as FileData. Honor named
    // workspace prefixes so agents can attach files that live under declared
    // `[workspaces]` mounts.
    if let Some(raw_path) = file_path {
        let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
        let data = tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;

        // Derive filename from the path if not explicitly provided
        let filename = input["filename"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            });

        // Determine MIME type from extension
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "csv" => "text/csv",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "gz" | "gzip" => "application/gzip",
            "tar" => "application/x-tar",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            // OGG / Opus voice payloads — channel adapters (e.g. Telegram)
            // use this MIME to route to native voice-memo endpoints rather
            // than generic file send (#4959).
            "ogg" | "oga" | "opus" => "audio/ogg",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };

        // `Bytes::from(Vec<u8>)` is O(1) — it takes ownership of the
        // Vec's allocation without copying. Subsequent clones (retry,
        // metering wrappers, fan-out) become refcount bumps. See #3553.
        let result = kh
            .send_channel_file_data(
                &channel,
                recipient,
                bytes::Bytes::from(data),
                &filename,
                mime_type,
                thread_id,
                account_id,
            )
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            mirror_channel_send_to_session(kh, caller_agent_id, &channel, recipient, &filename)
                .await;
        }
        return result;
    }

    if let Some(poll_question) = input.get("poll_question").and_then(|v| v.as_str()) {
        for key in ["image_url", "image_path", "file_url", "file_path"] {
            if input
                .get(key)
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                return Err(format!(
                    "poll_question cannot be combined with media/file attachments (got {key})"
                ));
            }
        }

        let poll_options = parse_poll_options(input.get("poll_options"))?;

        if let Some(violation) =
            check_taint_outbound_text(poll_question, &TaintSink::agent_message())
        {
            return Err(violation);
        }
        for opt in &poll_options {
            if let Some(violation) = check_taint_outbound_text(opt, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }

        let is_quiz = input
            .get("poll_is_quiz")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let correct_option_id = input
            .get("poll_correct_option")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8);
        let explanation = input.get("poll_explanation").and_then(|v| v.as_str());
        if let Some(exp) = explanation {
            if let Some(violation) = check_taint_outbound_text(exp, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }

        // Validate quiz mode requirements
        if is_quiz {
            let id = correct_option_id.ok_or_else(|| {
                "poll_correct_option is required when poll_is_quiz is true".to_string()
            })?;
            if id as usize >= poll_options.len() {
                return Err(format!(
                    "poll_correct_option {} is out of bounds (must be between 0 and {})",
                    id,
                    poll_options.len() - 1
                ));
            }
        }

        kh.send_channel_poll(
            &channel,
            recipient,
            poll_question,
            &poll_options,
            is_quiz,
            correct_option_id,
            explanation,
            account_id,
        )
        .await
        .map_err(|e| e.to_string())?;

        mirror_channel_send_to_session(kh, caller_agent_id, &channel, recipient, poll_question)
            .await;

        let mut result = format!("Poll sent to {recipient} on {channel}: {poll_question}");
        if is_quiz {
            result.push_str(" (quiz mode)");
        }
        return Ok(result);
    }

    // Text-only message
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;

    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // For email channels, validate email format and prepend subject
    let final_message = if channel == "email" {
        if !recipient.contains('@') || !recipient.contains('.') {
            return Err(format!("Invalid email address: '{recipient}'"));
        }
        if let Some(subject) = input["subject"].as_str() {
            if !subject.is_empty() {
                format!("Subject: {subject}\n\n{message}")
            } else {
                message.to_string()
            }
        } else {
            message.to_string()
        }
    } else {
        message.to_string()
    };

    if let Some(violation) = check_taint_outbound_text(&final_message, &TaintSink::agent_message())
    {
        return Err(violation);
    }

    let result = kh
        .send_channel_message(&channel, recipient, &final_message, thread_id, account_id)
        .await
        .map_err(|e| e.to_string());
    if result.is_ok() {
        mirror_channel_send_to_session(kh, caller_agent_id, &channel, recipient, &final_message)
            .await;
    }
    result
}
