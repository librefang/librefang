use super::{check_taint_outbound_text, require_kernel, resolve_file_path_ext};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::path::Path;
use std::sync::Arc;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

fn validate_email(email: &str) -> Result<(), String> {
    static EMAIL_RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$").unwrap()
    });
    if EMAIL_RE.is_match(email) {
        Ok(())
    } else {
        Err(format!("Invalid email address: '{email}'"))
    }
}

pub(super) fn parse_poll_options(raw: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
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
            tracing::debug!(
                channel,
                recipient,
                "channel_send mirror: no channel owner agent found, skipping"
            );
            return;
        }
    };

    let session_id = SessionId::for_sender_scope(owner, channel, Some(recipient));

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

    kh.append_to_session(session_id, owner, msg);
}

async fn mirror_on_success(
    kh: &Arc<dyn KernelHandle>,
    caller_agent_id: Option<&str>,
    channel: &str,
    recipient: &str,
    mirror_body: &str,
    send_result: Result<String, String>,
) -> Result<String, String> {
    if send_result.is_ok() {
        mirror_channel_send_to_session(kh, caller_agent_id, channel, recipient, mirror_body).await;
    }
    send_result
}

fn trim_opt_string(val: Option<&str>) -> Option<&str> {
    val.map(str::trim).filter(|s| !s.is_empty())
}

/// The base channel *type*, stripping any `:<suffix>` an adapter embeds. The
/// WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227) while a
/// `channel_send` targets the bare `"whatsapp"`; the #6443 cross-account guard
/// compares base types so the embedding does not silently disable it. Channels
/// with no colon are returned unchanged.
fn channel_base(channel: &str) -> &str {
    channel.split(':').next().unwrap_or(channel)
}

/// Resolve the `channel_send` target.
///
/// An explicit `recipient` wins. Otherwise the send replies to the current
/// conversation — the platform chat/room/group id (`sender_chat_id`), NOT the
/// individual speaker (`sender_id`). In a group the legitimate reply target is
/// the group, not the member who spoke; only in a DM (where the chat id
/// coincides with the sender, or none is stamped) does it fall back to
/// `sender_id`.
///
/// This is the SAME canonical target the cross-chat dispatch guard validates
/// against (`expected_chat`), so the two can never disagree: previously the
/// auto-fill resolved `sender_id` (the speaker) while the guard expected
/// `sender_chat_id` (the group). In a group that mismatch made a no-recipient
/// send target the speaker's user id as if it were a room — e.g. a Matrix send
/// to `@user:hs` instead of the room `!room:hs`, which the homeserver rejected
/// with `403 not in room` while the tool still reported success (#6117 guard
/// only scrutinised the explicit path).
fn resolve_send_target<'a>(
    explicit_recipient: Option<&'a str>,
    sender_chat_id: Option<&'a str>,
    sender_id: Option<&'a str>,
) -> Option<&'a str> {
    explicit_recipient
        .or_else(|| sender_chat_id.filter(|s| !s.is_empty()))
        .or(sender_id)
}

/// Resolve the `(channel, chat_id)` pair a `channel_members` read targets.
///
/// Both components default to the conversation the current turn arrived on, so the common "who is in this channel?" call needs no arguments at all.
///
/// An explicit `chat_id` naming a *different* conversation on the channel the turn arrived on is refused.
/// The roster carries the display name of every member the daemon has ever seen speak in a chat, so a free-form chat id would let one group enumerate the membership of every other chat the same bot sits in — the inbound twin of the #6117 cross-chat dispatch leak.
/// The guard is deliberately the same shape as the one in `tool_channel_send`: only an explicit argument is scrutinised, only when the turn's channel is known, only within the same base channel type (so the WhatsApp gateway's `"whatsapp:<jid>"` stamp does not silently disable it), and the chat id is compared case-sensitively while the channel match is not.
/// A cross-channel read stays allowed for the same reason a cross-channel send does — the agent is reaching into a surface the current conversation has no claim on either way, and out-of-band callers (cron, triggers) have no turn context to scope against.
fn resolve_roster_target<'a>(
    explicit_channel: Option<&'a str>,
    explicit_chat_id: Option<&'a str>,
    turn_channel: Option<&'a str>,
    turn_chat_id: Option<&'a str>,
    turn_sender_id: Option<&'a str>,
) -> Result<(&'a str, &'a str), String> {
    let channel = explicit_channel
        .or_else(|| turn_channel.filter(|s| !s.is_empty()))
        .ok_or("Missing 'channel' parameter. It is auto-filled only while handling an inbound channel message — pass it explicitly from a cron job, trigger, or API-driven run.")?;

    // The same canonical conversation id `channel_send` replies to: the chat / room, falling back to the peer for a DM.
    let turn_conversation =
        resolve_send_target(None, turn_chat_id, turn_sender_id).filter(|s| !s.is_empty());

    if let (Some(explicit), Some(turn), Some(expected)) =
        (explicit_chat_id, turn_channel, turn_conversation)
    {
        if !turn.is_empty()
            && channel_base(turn).eq_ignore_ascii_case(channel_base(channel))
            && explicit != expected
        {
            return Err(format!(
                "channel_members chat_id '{explicit}' does not match the current chat '{expected}' on channel '{channel}'. Reading another chat's roster is forbidden — omit chat_id to list the members of the current conversation."
            ));
        }
    }

    let chat_id = explicit_chat_id
        .or(turn_conversation)
        .ok_or("Missing 'chat_id' parameter. It is auto-filled only while handling an inbound channel message — pass it explicitly from a cron job, trigger, or API-driven run.")?;

    Ok((channel, chat_id))
}

/// `channel_members` — enumerate the persisted roster of a group conversation.
///
/// This is the read half of the roster the channel bridge has been writing since the `group_roster` table landed: every group message the daemon observes upserts its sender through `ChannelBridgeHandle::roster_upsert`, and `KernelHandle::roster_members` has been able to read it back the whole time with no caller.
/// Without this tool an agent sitting in a shared Slack or Telegram group cannot answer "who is in this channel?", and has no way to obtain the platform user id it needs to attribute a request to the person who made it (#7086).
///
/// Read-only and non-mutating: it neither sends anything nor teaches the roster about anyone new.
pub(super) fn tool_channel_members(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    sender_channel: Option<&str>,
    sender_chat_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let (channel, chat_id) = resolve_roster_target(
        trim_opt_string(input["channel"].as_str()),
        trim_opt_string(input["chat_id"].as_str()),
        sender_channel,
        sender_chat_id,
        sender_id,
    )?;

    // The roster is keyed on the bare channel *type* (`channel_type_str` in the bridge), so strip the conversation suffix the WhatsApp gateway embeds in its channel string (#5227) before looking it up — otherwise every WhatsApp read misses.
    let roster_channel = channel_base(channel);

    let members = kh
        .roster_members(roster_channel, chat_id)
        .map_err(|e| e.to_string())?;

    let mut out = serde_json::json!({
        "channel": roster_channel,
        "chat_id": chat_id,
        "count": members.len(),
        "members": members,
    });

    // An empty roster is the expected state for a DM, and for a group the daemon has never observed a message in.
    // Say so, rather than letting the model read `[]` as "this channel has no members" or as a broken tool.
    if members.is_empty() {
        out["note"] = serde_json::Value::String(
            "No members recorded for this conversation. The roster is built from group messages the daemon has observed, so a member who has never spoken in it is absent, and a direct message has no roster at all.".to_string(),
        );
    }

    Ok(out.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    sender_id: Option<&str>,
    // #6117: the current turn's inbound channel and platform conversation id
    // (chat_id / group jid). Used to scope the cross-chat dispatch guard. Both
    // `None` for out-of-band callers (cron, triggers, external MCP) — the guard
    // no-ops, preserving the existing unrestricted behaviour for those paths.
    sender_channel: Option<&str>,
    sender_chat_id: Option<&str>,
    // #6443: the bot account / tenant the current turn arrived on. Used to scope
    // the cross-account dispatch guard. `None` for out-of-band callers (cron,
    // triggers, API-driven runs) and single-tenant deployments — the guard
    // no-ops, preserving the existing unrestricted behaviour for those paths.
    sender_account_id: Option<&str>,
    caller_agent_id: Option<&str>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    // Kernel `send_channel_*` lookups are case-sensitive; adapters register with original case (#6078).
    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_string();

    // An explicitly-supplied recipient is the only cross-chat-leak vector — an
    // auto-filled one (reply to the current conversation) targets the inbound
    // chat by construction (see `resolve_send_target`). Keep the two apart so
    // the guard below only scrutinises an explicit `recipient`.
    let explicit_recipient = input["recipient"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Auto-fill replies to the conversation id (`sender_chat_id` — the group /
    // room), falling back to `sender_id` only for DMs. This is the same target
    // the cross-chat guard treats as canonical, so a no-recipient send can
    // never resolve a chat the guard would have rejected.
    let recipient = resolve_send_target(explicit_recipient, sender_chat_id, sender_id)
        .ok_or("Missing 'recipient' parameter. When replying to the original sender, recipient is auto-filled — ensure channel_send is called in response to a message.")?
        .trim();

    if recipient.is_empty() {
        return Err("Recipient cannot be empty".to_string());
    }

    // #6117 cross-chat dispatch guard (audio cross-chat leak 2026-05-19). When
    // the model explicitly targets a recipient on the SAME channel the turn
    // arrived on, it must match the current conversation. The comparison key is
    // the platform conversation id (`sender_chat_id` — a Telegram chat_id,
    // group jid, …), not the individual `sender_id`: in a group the legitimate
    // reply target is the group, not the speaker. DMs (where chat_id coincides
    // with the sender, or no chat_id is stamped) fall back to `sender_id`.
    //
    // A different-channel dispatch (e.g. emailing while replying to a WhatsApp
    // peer) stays allowed — only intra-channel re-targeting is the leak. To
    // legitimately reach a different contact, the agent uses `notify_owner`
    // (kernel-mediated) or waits for that contact's own inbound message.
    if let (Some(explicit), Some(turn_channel)) = (explicit_recipient, sender_channel) {
        // The expected chat is the same canonical reply target a no-recipient
        // send resolves to (`resolve_send_target` with no explicit recipient):
        // the conversation id, falling back to `sender_id` for DMs. The empty
        // filter on `sender_chat_id` is load-bearing — the in-process path
        // stamps the raw metadata value (unlike the `/mcp` bridge, which drops
        // empty headers), so `Some("")` must not mask the `sender_id` fallback
        // and silently disable the guard.
        let expected_chat = resolve_send_target(None, sender_chat_id, sender_id);
        if let Some(expected) = expected_chat {
            // Compare the recipient case-SENSITIVELY: `send_channel_*` lookups
            // are case-sensitive (#6078), so a case-insensitive match here would
            // let `recipient = "OWNER"` pass while the send routes to a distinct
            // case-sensitive chat. The channel match stays case-insensitive so
            // the guard still fires when the model varies the channel's casing.
            if !expected.is_empty()
                && !turn_channel.is_empty()
                && turn_channel.eq_ignore_ascii_case(&channel)
                && explicit != expected
            {
                return Err(format!(
                    "channel_send recipient '{explicit}' does not match the current chat '{expected}' on channel '{channel}'. Cross-chat dispatch is forbidden — to reach a different contact use notify_owner, or wait for that contact's inbound message."
                ));
            }
        }
    }

    let thread_id = trim_opt_string(input["thread_id"].as_str());
    let account_id = trim_opt_string(input["account_id"].as_str());

    // #6443 cross-account (cross-tenant) dispatch guard. `account_id` selects
    // which registered bot instance the send routes through. On a multi-tenant
    // daemon (several bot accounts on one daemon, each with its own
    // `default_agent` serving a different customer) an agent induced via model
    // hallucination or prompt injection could otherwise pass a *different*
    // tenant's `account_id` and silently dispatch content into that customer's
    // chat — the same trigger class as the #6117 live incident, but a wider
    // (cross-tenant) blast radius that the #6117 recipient guard does not cover
    // (it only scrutinises `recipient`, and runs before `account_id` is even
    // parsed). Mirror the #6117 scoping exactly: only an EXPLICIT `account_id`
    // is checked, only when the turn's originating account is known, and only
    // within the SAME channel the turn arrived on (a cross-channel dispatch
    // stays allowed by the #6117 design, and `account_id` is only comparable
    // within one channel's namespace). An auto-filled send carries no
    // `account_id` and already targets the originating account. Out-of-band
    // callers leave `sender_account_id` `None`, so the guard no-ops for them.
    if let (Some(explicit_account), Some(turn_account), Some(turn_channel)) =
        (account_id, sender_account_id, sender_channel)
    {
        // Compare the BASE channel type, not the raw stamped string. Adapters
        // that embed the conversation id in the channel (the WhatsApp gateway
        // stamps `sender_channel = "whatsapp:<jid>"`, see #5227) would otherwise
        // never equal the bare `channel = "whatsapp"` a `channel_send` targets,
        // silently disabling this guard for exactly the multi-account WhatsApp
        // deployments it must protect. Stripping the `:<suffix>` keeps the
        // same-channel-type scoping while surviving the embedding. Account ids
        // are compared case-SENSITIVELY (opaque registration identifiers, like
        // `recipient`); the channel match stays case-insensitive.
        if !turn_account.is_empty()
            && !turn_channel.is_empty()
            && channel_base(turn_channel).eq_ignore_ascii_case(channel_base(&channel))
            && explicit_account != turn_account
        {
            return Err(format!(
                "channel_send account_id '{explicit_account}' does not match the current account '{turn_account}' on channel '{channel}'. Cross-account (cross-tenant) dispatch is forbidden — an agent may only send from the bot account its turn arrived on."
            ));
        }
    }

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
        return mirror_on_success(
            kh,
            caller_agent_id,
            &channel,
            recipient,
            caption.unwrap_or(url),
            kh.send_channel_media(
                &channel, recipient, "image", url, caption, None, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string()),
        )
        .await;
    }

    if let Some(url) = file_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        let filename = input["filename"].as_str();
        if let Some(c) = caption {
            if let Some(violation) = check_taint_outbound_text(c, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }
        return mirror_on_success(
            kh,
            caller_agent_id,
            &channel,
            recipient,
            caption.unwrap_or(url),
            kh.send_channel_media(
                &channel, recipient, "file", url, caption, filename, thread_id, account_id,
            )
            .await
            .map_err(|e| e.to_string()),
        )
        .await;
    }

    if let Some(raw_path) = file_path {
        let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| format!("Failed to stat file '{}': {e}", resolved.display()))?;
        if meta.len() > MAX_FILE_SIZE {
            return Err(format!(
                "File '{}' is too large ({} bytes, max {} bytes)",
                resolved.display(),
                meta.len(),
                MAX_FILE_SIZE
            ));
        }

        let data = tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;

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
            "ogg" | "oga" | "opus" => "audio/ogg",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };

        return mirror_on_success(
            kh,
            caller_agent_id,
            &channel,
            recipient,
            &filename,
            kh.send_channel_file_data(
                &channel,
                recipient,
                bytes::Bytes::from(data),
                &filename,
                mime_type,
                thread_id,
                account_id,
            )
            .await
            .map_err(|e| e.to_string()),
        )
        .await;
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
            .map(|n| {
                u8::try_from(n).map_err(|_| {
                    format!("poll_correct_option {n} exceeds u8 range (must be 0-255)")
                })
            })
            .transpose()?;
        let explanation = input.get("poll_explanation").and_then(|v| v.as_str());
        if let Some(exp) = explanation {
            if let Some(violation) = check_taint_outbound_text(exp, &TaintSink::agent_message()) {
                return Err(violation);
            }
        }

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
            thread_id,
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

    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;

    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    let final_message = if channel == "email" {
        validate_email(recipient)?;
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

    mirror_on_success(
        kh,
        caller_agent_id,
        &channel,
        recipient,
        &final_message,
        kh.send_channel_message(&channel, recipient, &final_message, thread_id, account_id)
            .await
            .map_err(|e| e.to_string()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{resolve_roster_target, resolve_send_target};

    // A group turn: the speaker (`sender_id`) and the room (`sender_chat_id`)
    // differ. A no-recipient send must reply to the room, not the speaker —
    // the bug where a Matrix file send targeted `@user:hs` instead of the room
    // `!room:hs` and the homeserver returned `403 not in room`.
    #[test]
    fn auto_fill_replies_to_group_not_speaker() {
        let target = resolve_send_target(None, Some("!room:hs"), Some("@user:hs"));
        assert_eq!(target, Some("!room:hs"));
    }

    // DM: no chat id stamped (or it coincides with the sender) — fall back to
    // the sender so 1:1 replies keep working.
    #[test]
    fn auto_fill_falls_back_to_sender_for_dm() {
        assert_eq!(
            resolve_send_target(None, None, Some("@user:hs")),
            Some("@user:hs")
        );
        assert_eq!(
            resolve_send_target(None, Some(""), Some("@user:hs")),
            Some("@user:hs")
        );
    }

    // An explicit recipient always wins, even over a present chat id — the
    // cross-chat guard (not this resolver) is what rejects a mismatched one.
    #[test]
    fn explicit_recipient_wins() {
        let target = resolve_send_target(Some("@other:hs"), Some("!room:hs"), Some("@user:hs"));
        assert_eq!(target, Some("@other:hs"));
    }

    // The guard's `expected_chat` is this resolver with no explicit recipient,
    // so the auto-fill target and the guard's expectation are byte-identical —
    // a no-recipient send can never resolve a chat the guard would reject.
    #[test]
    fn auto_fill_matches_guard_expected_chat() {
        let chat = Some("!room:hs");
        let sender = Some("@user:hs");
        let auto_fill = resolve_send_target(None, chat, sender);
        let expected_chat = resolve_send_target(None, chat, sender);
        assert_eq!(auto_fill, expected_chat);
    }

    // No identity at all (out-of-band caller — cron, trigger): nothing to
    // resolve, the caller surfaces the "Missing 'recipient'" error.
    #[test]
    fn no_identity_resolves_none() {
        assert_eq!(resolve_send_target(None, None, None), None);
    }

    // A bare `channel_members` call during a group turn reads that group.
    #[test]
    fn roster_target_defaults_to_the_current_group() {
        let resolved =
            resolve_roster_target(None, None, Some("slack"), Some("C123"), Some("U9")).unwrap();
        assert_eq!(resolved, ("slack", "C123"));
    }

    // A DM stamps no chat id (or the peer's own id), so the peer is the conversation — the same fallback `channel_send` uses.
    #[test]
    fn roster_target_falls_back_to_the_peer_in_a_dm() {
        let resolved =
            resolve_roster_target(None, None, Some("telegram"), None, Some("4242")).unwrap();
        assert_eq!(resolved, ("telegram", "4242"));
        let empty_chat =
            resolve_roster_target(None, None, Some("telegram"), Some(""), Some("4242")).unwrap();
        assert_eq!(empty_chat, ("telegram", "4242"));
    }

    // Restating the current conversation is allowed — only a *different* one is a leak.
    #[test]
    fn roster_target_accepts_an_explicit_current_chat() {
        let resolved = resolve_roster_target(
            Some("slack"),
            Some("C123"),
            Some("slack"),
            Some("C123"),
            None,
        )
        .unwrap();
        assert_eq!(resolved, ("slack", "C123"));
    }

    // The inbound twin of the #6117 leak: one group must not enumerate another group's membership.
    #[test]
    fn roster_target_rejects_another_chat_on_the_same_channel() {
        let err = resolve_roster_target(
            Some("slack"),
            Some("C999"),
            Some("slack"),
            Some("C123"),
            Some("U9"),
        )
        .expect_err("cross-chat roster read must be refused");
        assert!(err.contains("C999"), "{err}");
        assert!(err.contains("C123"), "{err}");
    }

    // The channel match is case-insensitive, so varying the casing cannot slip a foreign chat id past the guard.
    #[test]
    fn roster_target_guard_survives_channel_case_variation() {
        assert!(resolve_roster_target(
            Some("Slack"),
            Some("C999"),
            Some("slack"),
            Some("C123"),
            None
        )
        .is_err());
    }

    // The WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227).
    // Comparing the raw strings would never match the bare `"whatsapp"` a tool call names, silently disabling the guard for exactly the multi-member groups it protects.
    #[test]
    fn roster_target_guard_survives_the_whatsapp_channel_suffix() {
        assert!(resolve_roster_target(
            Some("whatsapp"),
            Some("other@g.us"),
            Some("whatsapp:123@g.us"),
            Some("123@g.us"),
            None
        )
        .is_err());
        let ok = resolve_roster_target(
            Some("whatsapp"),
            Some("123@g.us"),
            Some("whatsapp:123@g.us"),
            Some("123@g.us"),
            None,
        )
        .unwrap();
        assert_eq!(ok, ("whatsapp", "123@g.us"));
    }

    // A different channel than the turn arrived on stays readable, matching the `channel_send` scoping — the guard only covers intra-channel re-targeting.
    #[test]
    fn roster_target_allows_a_different_channel() {
        let resolved = resolve_roster_target(
            Some("telegram"),
            Some("-100"),
            Some("slack"),
            Some("C123"),
            None,
        )
        .unwrap();
        assert_eq!(resolved, ("telegram", "-100"));
    }

    // The tool has to be declared to be reachable: a dispatch arm with no matching `ToolDefinition` is invisible to the model and to `tool_load` / `tool_search`.
    #[test]
    fn channel_members_is_registered_in_builtins() {
        let defs = crate::tool_runner::builtin_tool_definitions();
        let def = defs
            .iter()
            .find(|d| d.name == "channel_members")
            .expect("channel_members must appear in builtin_tool_definitions");
        let props = def.input_schema["properties"]
            .as_object()
            .expect("properties object");
        assert!(props.contains_key("channel"));
        assert!(props.contains_key("chat_id"));
        // Both arguments default to the current conversation, so a bare call during message handling must be valid.
        assert!(
            def.input_schema.get("required").is_none(),
            "channel_members must have no required arguments"
        );
    }

    // #3298: the schema is stringified into every request that declares this tool, so its key order has to be a canonical property of the literal rather than something a future edit can shuffle — a reordering invalidates provider prompt caches on byte-identical content.
    #[test]
    fn channel_members_schema_property_order_is_canonical() {
        let defs = crate::tool_runner::builtin_tool_definitions();
        let def = defs
            .iter()
            .find(|d| d.name == "channel_members")
            .expect("channel_members must appear in builtin_tool_definitions");
        let keys: Vec<&str> = def.input_schema["properties"]
            .as_object()
            .expect("properties object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    // Out-of-band callers (cron, triggers) carry no turn context, so both arguments become required rather than resolving to something arbitrary.
    #[test]
    fn roster_target_requires_explicit_arguments_out_of_band() {
        let no_channel = resolve_roster_target(None, Some("C1"), None, None, None)
            .expect_err("channel must be required without a turn");
        assert!(no_channel.contains("channel"), "{no_channel}");
        let no_chat = resolve_roster_target(Some("slack"), None, None, None, None)
            .expect_err("chat_id must be required without a turn");
        assert!(no_chat.contains("chat_id"), "{no_chat}");
        let both = resolve_roster_target(Some("slack"), Some("C1"), None, None, None).unwrap();
        assert_eq!(both, ("slack", "C1"));
    }
}
