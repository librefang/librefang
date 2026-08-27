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

/// Resolve the conversation a `channel_dm` is authorized against: the base channel type and the platform conversation id of the turn currently being handled.
///
/// `channel_dm` deliberately takes no `channel` / `chat_id` arguments.
/// Its whole safety property is that the recipient must be a member of *this* conversation's roster, and a caller-supplied conversation would let the model choose its own authorization set — which is the #6117 cross-chat leak with extra steps.
/// So the pair is derived from the turn and nothing else, and an out-of-band caller (cron, trigger, API-driven run) has no conversation to be authorized against and is refused rather than silently given a wider one.
///
/// The channel is reduced to its base type for two reasons at once: the roster is keyed on the bare `channel_type_str` the bridge writes, and the WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227), which no registered adapter name matches.
fn resolve_dm_conversation<'a>(
    turn_channel: Option<&'a str>,
    turn_chat_id: Option<&'a str>,
    turn_sender_id: Option<&'a str>,
) -> Result<(&'a str, &'a str), String> {
    let channel = turn_channel
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("channel_dm is only available while handling an inbound channel message — it addresses a member of the conversation the turn arrived on, and there is no such conversation here. Use channel_send with an explicit channel and recipient instead.")?;

    let conversation = resolve_send_target(None, turn_chat_id, turn_sender_id)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("channel_dm could not determine the current conversation: the turn carries no chat id and no sender id.")?;

    Ok((channel_base(channel), conversation))
}

/// `channel_dm` — deliver a message privately to one member of the current group conversation.
///
/// The gap this closes (#7086): in a group the only target `channel_send` accepts is the group itself (#6117), and `notify_owner` — which the guard used to recommend — produces an `owner_notice` in the reply envelope that no sidecar channel adapter routes anywhere.
/// An agent asked to tell one person their task finished therefore had to either broadcast it to everyone or drop it silently.
///
/// The authorization set is the persisted roster of the conversation the turn arrived on: the recipient must be someone the daemon has observed speaking there.
/// That is what keeps #6117 closed while still allowing a private reply — the model cannot name an arbitrary platform id, only someone already in the room with it.
///
/// What "private" means per platform, because it is not the same everywhere:
///
/// * Slack — `chat.postMessage` addressed to a `U…` id posts into the bot↔user IM, which the bot may open with any workspace member.
/// * Telegram — a `chat_id` equal to a user id is that user's private chat, but a bot cannot open one; the user must have started the bot, otherwise the API rejects the send.
/// * WhatsApp — a participant JID addresses that participant's individual chat.
///
/// The failure is surfaced, never papered over: nothing here falls back to posting in the group, because a "private" notice that silently becomes public is worse than an error.
pub(super) async fn tool_channel_dm(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    sender_channel: Option<&str>,
    sender_chat_id: Option<&str>,
    sender_account_id: Option<&str>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let user_id = trim_opt_string(input["user_id"].as_str())
        .ok_or("Missing 'user_id' parameter — the platform user id of the member to reach, as returned by channel_members.")?;

    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (the text to deliver privately).")?;
    if message.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    let (channel, conversation) =
        resolve_dm_conversation(sender_channel, sender_chat_id, sender_id)?;

    // Naming the conversation itself is not a private message, it is the broadcast the caller was trying to avoid.
    // The roster lookup below would reject it anyway (a chat id is not one of its own members), but that error would describe the wrong problem.
    if user_id == conversation {
        return Err(format!(
            "channel_dm user_id '{user_id}' is the current conversation itself, not a member of it. To post where everyone in the conversation can read it, use channel_send."
        ));
    }

    // `roster_observed_members`, never `roster_members` (#7086).
    // The roster holds two kinds of row since bulk enumeration landed: people observed speaking here, and people a platform merely listed as members of the channel.
    // Only the first kind may be privately addressed — enumerating a Slack channel would otherwise hand the agent a DM channel to every member of it, including everyone who has never spoken to it, which is the #6117 leak re-opened through a side door.
    // The narrowing lives in the store's `AND source = 'observed'` predicate; calling the wider read here is exactly the mistake the two-method split exists to make visible.
    //
    // Platform ids are opaque and `send_channel_*` lookups are case-sensitive (#6078), so the membership check is too: a case-insensitive match here would authorize one id and deliver to a different one.
    let members = kh
        .roster_observed_members(channel, conversation)
        .map_err(|e| e.to_string())?;
    let is_member = members
        .iter()
        .any(|m| m.get("user_id").and_then(|v| v.as_str()) == Some(user_id));
    if !is_member {
        return Err(format!(
            "channel_dm user_id '{user_id}' is not someone the daemon has observed speaking in the current conversation on channel '{channel}'. A private message may only be addressed to someone who has spoken here — call channel_members and pick a member whose source is 'observed'. A member listed as 'enumerated' is known to the platform but has never addressed this agent, and cannot be messaged privately."
        ));
    }

    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(violation);
    }

    // No `thread_id`: the turn's thread belongs to the group conversation, and carrying it into a one-to-one chat addresses a thread that does not exist there.
    // No caller-supplied `account_id` either — the send routes through the same bot account the turn arrived on, which makes the #6443 cross-account guard unnecessary rather than merely satisfied.
    let confirmation = mirror_on_success(
        kh,
        caller_agent_id,
        channel,
        user_id,
        message,
        kh.send_channel_message(channel, user_id, message, None, sender_account_id)
            .await
            .map_err(|e| e.to_string()),
    )
    .await?;

    Ok(format!(
        "Delivered privately to {user_id} on {channel}; the rest of the conversation cannot see it. {confirmation}"
    ))
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
/// This is the read half of the roster the channel bridge writes: every group message the daemon observes upserts its sender through `ChannelBridgeHandle::roster_upsert`, and an adapter that supplies a platform member list adds the rest through `roster_upsert_enumerated` (#7086).
/// Without this tool an agent sitting in a shared Slack or Telegram group cannot answer "who is in this channel?", and has no way to obtain the platform user id it needs to attribute a request to the person who made it.
///
/// Every entry carries a `source`, and it is not decoration.
/// `"observed"` means the person has spoken here and may be privately addressed with `channel_dm`; `"enumerated"` means a platform listed them and `channel_dm` will refuse.
/// The counts are surfaced alongside the list so the model can see the boundary without tallying the rows itself, and the note spells out the consequence — a model that assumes every listed member is reachable would otherwise discover the split one refusal at a time.
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

    let observed_count = members
        .iter()
        .filter(|m| m.get("source").and_then(|v| v.as_str()) == Some("observed"))
        .count();
    let enumerated_count = members.len() - observed_count;

    let mut out = serde_json::json!({
        "channel": roster_channel,
        "chat_id": chat_id,
        "count": members.len(),
        "observed_count": observed_count,
        "enumerated_count": enumerated_count,
        "members": members,
    });

    // An empty roster is the expected state for a DM, and for a group the daemon has never observed a message in.
    // Say so, rather than letting the model read `[]` as "this channel has no members" or as a broken tool.
    if members.is_empty() {
        out["note"] = serde_json::Value::String(
            "No members recorded for this conversation. The roster is built from group messages the daemon has observed, plus the platform member list on adapters configured to supply one, so a conversation nobody has spoken in on an adapter that does not enumerate has no roster — and a direct message has none either.".to_string(),
        );
    } else if enumerated_count > 0 {
        out["note"] = serde_json::Value::String(
            "Members with source 'observed' have spoken in this conversation and can be reached with channel_dm. Members with source 'enumerated' come from the platform's member list and have never addressed this agent — they are listed here but channel_dm will refuse them.".to_string(),
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
    // legitimately reach one member of the CURRENT conversation, the agent uses
    // `channel_dm`, which is authorized against that conversation's roster; to
    // reach a contact in a different conversation it waits for that contact's
    // own inbound message. The guard used to recommend `notify_owner` here,
    // which was a dead end: `owner_notice` is surfaced through the API reply
    // envelope and consumed by no sidecar channel adapter, so on Slack the
    // model was refused and then sent to a path that delivered nothing (#7086).
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
                    "channel_send recipient '{explicit}' does not match the current chat '{expected}' on channel '{channel}'. Cross-chat dispatch is forbidden. To reach one member of THIS conversation privately, use channel_dm — it delivers to a member of this chat's roster. To reach someone in a different conversation, wait for their inbound message: notify_owner is not a delivery path on a channel, it only surfaces a notice to the operator out of band."
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
    use super::{resolve_dm_conversation, resolve_roster_target, resolve_send_target};

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

    // A group turn resolves to (base channel type, the group) — the pair whose roster authorizes the recipient.
    #[test]
    fn dm_conversation_is_the_current_group() {
        let resolved = resolve_dm_conversation(Some("slack"), Some("C123"), Some("U9")).unwrap();
        assert_eq!(resolved, ("slack", "C123"));
    }

    // The WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227).
    // The suffix has to come off twice over: the roster is keyed on the bare channel type, and no registered adapter answers to the embedded form.
    #[test]
    fn dm_conversation_strips_the_channel_suffix() {
        let resolved =
            resolve_dm_conversation(Some("whatsapp:123@g.us"), Some("123@g.us"), Some("44@s.wa"))
                .unwrap();
        assert_eq!(resolved, ("whatsapp", "123@g.us"));
    }

    // A DM stamps no chat id (or an empty one), so the peer is the conversation — the same fallback `channel_send` uses.
    // The roster of a one-to-one chat is empty, so the membership check refuses the send; that is the correct outcome, not an error here.
    #[test]
    fn dm_conversation_falls_back_to_the_peer() {
        assert_eq!(
            resolve_dm_conversation(Some("telegram"), None, Some("4242")).unwrap(),
            ("telegram", "4242")
        );
        assert_eq!(
            resolve_dm_conversation(Some("telegram"), Some(""), Some("4242")).unwrap(),
            ("telegram", "4242")
        );
    }

    // Out-of-band callers (cron, triggers, API-driven runs) have no conversation, so no roster can authorize a recipient.
    // Refusing is the point: falling back to a caller-supplied conversation would let the model pick its own authorization set, which is #6117 again.
    #[test]
    fn dm_conversation_is_refused_without_a_turn() {
        let err = resolve_dm_conversation(None, Some("C123"), Some("U9"))
            .expect_err("channel_dm must be refused without an inbound channel turn");
        assert!(err.contains("inbound channel message"), "{err}");
        assert!(resolve_dm_conversation(Some("  "), Some("C123"), None).is_err());
        let no_ids = resolve_dm_conversation(Some("slack"), None, None)
            .expect_err("channel_dm must be refused with no conversation to scope against");
        assert!(no_ids.contains("current conversation"), "{no_ids}");
    }

    // A dispatch arm with no matching `ToolDefinition` is invisible to the model and to `tool_load` / `tool_search`.
    #[test]
    fn channel_dm_is_registered_in_builtins() {
        let defs = crate::tool_runner::builtin_tool_definitions();
        let def = defs
            .iter()
            .find(|d| d.name == "channel_dm")
            .expect("channel_dm must appear in builtin_tool_definitions");
        let props = def.input_schema["properties"]
            .as_object()
            .expect("properties object");
        assert!(props.contains_key("user_id"));
        assert!(props.contains_key("message"));
        // Neither the channel nor the conversation is an argument — both come from the turn, and that is what bounds the recipient to this conversation's roster.
        assert!(!props.contains_key("channel"));
        assert!(!props.contains_key("chat_id"));
        let required: Vec<&str> = def.input_schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(required, vec!["message", "user_id"]);
    }

    // #3298: the schema is stringified into every request that declares this tool, so its key order has to be a canonical property of the literal rather than something a future edit can shuffle — a reordering invalidates provider prompt caches on byte-identical content.
    #[test]
    fn channel_dm_schema_property_order_is_canonical() {
        let defs = crate::tool_runner::builtin_tool_definitions();
        let def = defs
            .iter()
            .find(|d| d.name == "channel_dm")
            .expect("channel_dm must appear in builtin_tool_definitions");
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
