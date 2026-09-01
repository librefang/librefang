//! [`kernel_handle::ChannelSender`] — send text / media / file / poll content to a registered channel adapter, plus roster CRUD.
//! Every outbound send resolves its adapter through the single [`resolve_channel_adapter`] helper, whose precedence rules are the security-relevant part of this module.
//! [`resolve_channel_adapter`] is `pub(in crate::kernel)` because two callers outside this trait impl must resolve identically or they silently diverge from the send they guard: the interactive approval notification in `kernel::mod` and the owner-notification gate in `kernel::assistant_routing`.
//!
//! Every channel runs out-of-process as a sidecar; the per-channel
//! `default_agent` lookup is therefore single-pass over
//! `cfg.sidecar_channels` via [`sidecar_default_agent`].

use std::sync::Arc;

use dashmap::DashMap;
use librefang_channels::types::ChannelAdapter;
use librefang_runtime::kernel_handle;
use tracing::debug;

use super::super::LibreFangKernel;

/// Resolve the channel adapter an outbound send must route through, given the `(channel, account_id)` pair the originating turn was stamped with.
///
/// Resolution order, most specific first:
///
/// 1. `"<channel>:<account_id>"`, when an `account_id` is known — the account-qualified registration key.
/// 2. `"<channel>"`, **only** when no `account_id` is known — the bare key, which the bridge fills with an adapter's instance name.
/// 3. A scan for the one registered adapter whose `channel_type()` equals `channel`, disambiguated by `account_id`.
///
/// Step 3 is what makes a channel instance whose **name differs from its adapter type** reachable at all (#8055).
/// The bridge registers every adapter under `adapter.name()` — the `[[sidecar_channels]] name`, e.g. `"slack-hr"` — plus `"<name>:<account_id>"`, and it sources that `account_id` from the same `name`.
/// Inbound turns, meanwhile, are stamped with the *channel type* (`"slack"`, via `channel_type_str(adapter.channel_type())` in `librefang_channels::bridge`).
/// Under the common `<adapter>-<team>` naming convention the two never meet, so steps 1 and 2 both missed and the post-approval reply path, `channel_dm`, and every auto-filled `channel_send` failed with `Channel 'slack' with account_id 'slack-hr' not found. Available: ["slack-hr", "slack-hr:slack-hr"]` — the agent's reply was produced, persisted, and never delivered.
///
/// Two adapters of one channel type are two different tenants, so ambiguity is an error rather than an arbitrary pick:
///
/// * With an `account_id`, a candidate must actually carry it — as its own `account_id()`, or as its registration `name`, which is where the bridge reads the value from (`channel_bridge.rs`: `Some(sidecar_config.name.clone())`) and which is populated even before a sidecar's `ready` event has filled the `account_id()` `OnceLock`.
/// * Without one, the scan resolves only when exactly one adapter of that channel type is registered.
///   A bare `"slack"` on a two-workspace daemon keeps erroring instead of choosing a tenant at random.
///
/// A known-but-unmatched `account_id` never falls back to the bare key.
/// That fallback is the leak the approval listener in `librefang_channels::bridge` documents at length: in a mixed config (one single-bot adapter and one multi-bot adapter on the same channel type) it would point a qualified miss at the *other* tenant's adapter and deliver into that tenant's chat.
///
/// This widens no authorization.
/// The cross-chat (#6117) and cross-account (#6443) dispatch guards live upstream in `librefang_runtime::tool_runner::channel` and run before any of this; every pair that resolves here is one that already names a specific registered instance.
pub(in crate::kernel) fn resolve_channel_adapter(
    adapters: &DashMap<String, Arc<dyn ChannelAdapter>>,
    channel: &str,
    account_id: Option<&str>,
) -> Result<Arc<dyn ChannelAdapter>, String> {
    // An empty `account_id` is "unknown", not "an account named the empty string" — every caller filtered it this way before the helper existed.
    let account_id = account_id.filter(|s| !s.is_empty());

    match account_id {
        Some(aid) => {
            if let Some(hit) = adapters.get(&format!("{channel}:{aid}")) {
                return Ok(hit.clone());
            }
        }
        None => {
            if let Some(hit) = adapters.get(channel) {
                return Ok(hit.clone());
            }
        }
    }

    let mut candidates: Vec<Arc<dyn ChannelAdapter>> = Vec::new();
    for entry in adapters.iter() {
        let adapter = entry.value();
        if librefang_channels::router::channel_type_to_str(&adapter.channel_type()) != channel {
            continue;
        }
        if let Some(aid) = account_id {
            if adapter.account_id() != Some(aid) && adapter.name() != aid {
                continue;
            }
        }
        // One adapter is registered under both its bare and its qualified key, so identity — not name — is what distinguishes "seen twice" from "two instances that happen to share a name".
        if candidates.iter().any(|c| Arc::ptr_eq(c, adapter)) {
            continue;
        }
        candidates.push(Arc::clone(adapter));
    }

    if candidates.len() == 1 {
        return Ok(candidates.swap_remove(0));
    }
    Err(adapter_lookup_error(
        adapters,
        channel,
        account_id,
        candidates.len(),
    ))
}

/// Render the miss from [`resolve_channel_adapter`] as an operator-readable message.
///
/// The registered keys are **sorted** before rendering.
/// `DashMap` iteration order varies per process, and this list is the only place an operator sees how the daemon actually keyed its adapters — an unstable order made the #8055 reports hard to compare against each other.
fn adapter_lookup_error(
    adapters: &DashMap<String, Arc<dyn ChannelAdapter>>,
    channel: &str,
    account_id: Option<&str>,
    ambiguous: usize,
) -> String {
    let mut available: Vec<String> = adapters.iter().map(|e| e.key().clone()).collect();
    available.sort();
    if ambiguous > 1 {
        // Naming the account is already the caller's most specific option, so telling it to pass one is only useful when it did not.
        return match account_id {
            Some(aid) => format!(
                "Channel '{channel}' with account_id '{aid}' is ambiguous: {ambiguous} registered adapters of that channel type answer to that account, so routing could reach the wrong one. Address the instance by its registered name instead. Available: {available:?}"
            ),
            None => format!(
                "Channel '{channel}' is ambiguous: {ambiguous} registered adapters share that channel type, so routing by type alone could reach the wrong account. Name the instance directly, or pass the account_id of the instance to send through. Available: {available:?}"
            ),
        };
    }
    match account_id {
        Some(aid) => format!(
            "Channel '{channel}' with account_id '{aid}' not found. Available: {available:?}"
        ),
        None => format!("Channel '{channel}' not found. Available channels: {available:?}"),
    }
}

/// Resolve the `default_agent` name for a sidecar channel matching `channel`.
///
/// A sidecar entry's effective channel name is its `channel_type` (falling
/// back to `name`), mirroring how `channel_bridge` derives the
/// `ChannelType`. The first matching entry that carries a non-empty
/// `default_agent` wins — deterministic because `sidecar_channels` is an
/// ordered `Vec`. The `channel_send` mirror introduced in #4824 routes
/// through this lookup post-sidecar-migration.
fn sidecar_default_agent<'a>(
    sidecar_channels: &'a [librefang_types::config::SidecarChannelConfig],
    channel: &str,
) -> Option<&'a str> {
    sidecar_channels.iter().find_map(|entry| {
        let entry_channel = entry.channel_type.as_deref().unwrap_or(entry.name.as_str());
        if entry_channel == channel {
            entry.agent.as_deref().filter(|s| !s.is_empty())
        } else {
            None
        }
    })
}

/// Wire shape of one roster row, as both roster reads hand it to the tool layer.
///
/// `source` rides along on every entry rather than only on the full listing: `channel_members` renders it so the model can see which people it may privately address, and a row that lost its classification in transit would read as addressable.
fn roster_member_json(member: librefang_memory::roster_store::RosterMember) -> serde_json::Value {
    serde_json::json!({
        "user_id": member.user_id,
        "display_name": member.display_name,
        "username": member.username,
        "source": member.source.as_str(),
    })
}

#[async_trait::async_trait]
impl kernel_handle::ChannelSender for LibreFangKernel {
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        // `self.config.load_full()` was previously read here for the
        // wecom-specific output-format override; removed in the
        // wecom-sidecar migration (the sidecar handles its own
        // formatting via `msgtype: "markdown"` frames).
        let adapter = resolve_channel_adapter(&self.mesh.channel_adapters, channel, account_id)?;

        let user = librefang_channels::types::ChannelUser {
            platform_id: recipient.to_string(),
            display_name: recipient.to_string(),
            librefang_user: None,
        };

        // #6445: honour a per-channel `output_format` override on the outbound path too, not only on inbound replies.
        // The sidecar adapter projects its `[[sidecar_channels]] output_format` into `ChannelOverrides`, so an agent-initiated `channel_send` / delegation forward formats the same way a normal reply would.
        // Falls back to the channel default when the adapter has no override (every non-sidecar adapter, and any sidecar that did not set the knob).
        let format = adapter
            .channel_overrides()
            .and_then(|ov| ov.output_format)
            .unwrap_or_else(|| {
                // wecom migrated to a sidecar; its formatting now happens inside
                // the Python adapter (`librefang.sidecar.adapters.wecom`) which
                // wraps every outbound chunk as `msgtype: "markdown"`. The
                // generic `format_for_channel` path with the Markdown default
                // (see `default_output_format_for_channel("wecom")`) gives the
                // sidecar exactly that.
                librefang_channels::formatter::default_output_format_for_channel(channel)
            });
        let formatted = librefang_channels::formatter::format_for_channel(message, format);

        let content = librefang_channels::types::ChannelContent::Text(formatted);

        if let Some(tid) = thread_id {
            adapter
                .send_in_thread(&user, content, tid)
                .await
                .map_err(|e| format!("Channel send failed: {e}"))?;
        } else {
            adapter
                .send(&user, content)
                .await
                .map_err(|e| format!("Channel send failed: {e}"))?;
        }

        Ok(format!("Message sent to {} via {}", recipient, channel))
    }

    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let adapter = resolve_channel_adapter(&self.mesh.channel_adapters, channel, account_id)?;

        let user = librefang_channels::types::ChannelUser {
            platform_id: recipient.to_string(),
            display_name: recipient.to_string(),
            librefang_user: None,
        };

        let content = match media_type {
            "image" => librefang_channels::types::ChannelContent::Image {
                url: media_url.to_string(),
                caption: caption.map(|s| s.to_string()),
                mime_type: None,
            },
            "file" => librefang_channels::types::ChannelContent::File {
                url: media_url.to_string(),
                filename: filename.unwrap_or("file").to_string(),
            },
            _ => {
                return Err(kernel_handle::KernelOpError::InvalidInput(format!(
                    "media_type: Unsupported media type: '{media_type}'. Use 'image' or 'file'."
                )));
            }
        };

        if let Some(tid) = thread_id {
            adapter
                .send_in_thread(&user, content, tid)
                .await
                .map_err(|e| format!("Channel media send failed: {e}"))?;
        } else {
            adapter
                .send(&user, content)
                .await
                .map_err(|e| format!("Channel media send failed: {e}"))?;
        }

        Ok(format!(
            "{} sent to {} via {}",
            media_type, recipient, channel
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: bytes::Bytes,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let adapter = resolve_channel_adapter(&self.mesh.channel_adapters, channel, account_id)?;

        let user = librefang_channels::types::ChannelUser {
            platform_id: recipient.to_string(),
            display_name: recipient.to_string(),
            librefang_user: None,
        };

        // `ChannelContent::FileData` still carries `Vec<u8>` (changing it
        // is out of scope for #3553 — that's a follow-up that touches
        // every channel adapter). `Vec::from(Bytes)` is O(1) when the
        // Bytes uniquely owns its allocation, which is the common case
        // here (caller built it via `Bytes::from(vec)` straight from
        // `tokio::fs::read`).
        let content = librefang_channels::types::ChannelContent::FileData {
            data: Vec::from(data),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
        };

        if let Some(tid) = thread_id {
            adapter
                .send_in_thread(&user, content, tid)
                .await
                .map_err(|e| format!("Channel file send failed: {e}"))?;
        } else {
            adapter
                .send(&user, content)
                .await
                .map_err(|e| format!("Channel file send failed: {e}"))?;
        }

        Ok(format!(
            "File '{}' sent to {} via {}",
            filename, recipient, channel
        ))
    }

    async fn send_channel_poll(
        &self,
        channel: &str,
        recipient: &str,
        question: &str,
        options: &[String],
        is_quiz: bool,
        correct_option_id: Option<u8>,
        explanation: Option<&str>,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let adapter = resolve_channel_adapter(&self.mesh.channel_adapters, channel, account_id)?;

        let user = librefang_channels::types::ChannelUser {
            platform_id: recipient.to_string(),
            display_name: recipient.to_string(),
            librefang_user: None,
        };

        let content = librefang_channels::types::ChannelContent::Poll {
            question: question.to_string(),
            options: options.to_vec(),
            is_quiz,
            correct_option_id,
            explanation: explanation.map(|s| s.to_string()),
        };

        if let Some(tid) = thread_id {
            adapter
                .send_in_thread(&user, content, tid)
                .await
                .map_err(|e| format!("Channel poll send failed: {e}"))?;
        } else {
            adapter
                .send(&user, content)
                .await
                .map_err(|e| format!("Channel poll send failed: {e}"))?;
        }

        Ok(())
    }

    fn roster_upsert(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        display_name: &str,
        username: Option<&str>,
    ) -> Result<(), kernel_handle::KernelOpError> {
        self.memory
            .substrate
            .roster()
            .upsert(channel, chat_id, user_id, display_name, username)
    }

    fn roster_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        let members = self.memory.substrate.roster().members(channel, chat_id)?;
        Ok(members.into_iter().map(roster_member_json).collect())
    }

    fn roster_observed_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        let members = self
            .memory
            .substrate
            .roster()
            .observed_members(channel, chat_id)?;
        Ok(members.into_iter().map(roster_member_json).collect())
    }

    fn roster_remove_member(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
    ) -> Result<(), kernel_handle::KernelOpError> {
        self.memory
            .substrate
            .roster()
            .remove_member(channel, chat_id, user_id)
    }

    fn resolve_channel_owner(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Option<librefang_types::agent::AgentId> {
        // Binding-first owner resolution (#4824 / #6022).
        //
        // The outbound `channel_send` mirror must land on the SAME agent that
        // owns the inbound conversation. When a `[[bindings]]` `peer_id` rule
        // routes a specific `(channel, chat_id)` to a non-default agent, the
        // inbound reply is delivered to the bound agent — so the mirror has to
        // resolve through the bindings too, not just the channel
        // `default_agent`. Matching purely on `default_agent` (the pre-fix
        // behaviour) wrote the mirror to the wrong session and the bound agent
        // never saw its own outbound context — the exact gap #4824 set out to
        // close.
        //
        // We reuse `BindingMatchRule::matches` — the same matcher the inbound
        // `MessageRouter` uses — so the two paths cannot drift. For the
        // outbound direction we only have `(channel, chat_id)`; `account_id`,
        // `guild_id`, and `roles` are unknown, matching the
        // inbound-for-outbound semantics already used by
        // `bound_recipients_for_agent`.
        //
        // We must NOT rely on the kernel binding store being
        // specificity-sorted: only the runtime `add_binding` path sorts
        // (`bindings_and_handle.rs`), while the config-boot store
        // (`MeshSubsystem::new`, populated from `config.bindings` in file
        // order) is left unsorted. The inbound `MessageRouter` keeps its own
        // separately-sorted copy (`router.rs::load_bindings`), so taking the
        // *first* match here would resolve the outbound mirror to a broad
        // binding declared earlier in config while the inbound reply routes to
        // a more-specific one — re-opening the exact cross-agent leak #6022 set
        // out to close. Select the highest-specificity match explicitly so the
        // result is independent of store order.
        //
        // Tie-break to mirror the inbound router exactly: `load_bindings`
        // (`router.rs`) does a *stable* descending sort by specificity and
        // takes the first match, so among bindings of equal specificity the
        // one declared earliest in config wins. We therefore fold with a
        // strict `>` (replace only on strictly-greater specificity), keeping
        // the first equal-specificity match — `max_by_key` would keep the
        // *last*, drifting from inbound on a tie.
        let bound_agent = {
            let bindings = super::super::bindings_and_handle::lock_bindings(&self.mesh.bindings);
            let mut best: Option<&librefang_types::config::AgentBinding> = None;
            for b in bindings
                .iter()
                .filter(|b| b.match_rule.matches(channel, None, chat_id, None, &[]))
            {
                if best.is_none_or(|cur| b.match_rule.specificity() > cur.match_rule.specificity())
                {
                    best = Some(b);
                }
            }
            best.map(|b| b.agent.clone())
        };
        if let Some(agent_name) = bound_agent {
            if let Some(entry) = self.agents.registry.find_by_name(&agent_name) {
                debug!(
                    channel,
                    chat_id,
                    agent = %agent_name,
                    agent_id = %entry.id,
                    "channel_send mirror: resolved owner via binding"
                );
                return Some(entry.id);
            }
            // A binding named an agent that is not currently registered; fall
            // through to the channel default rather than dropping the mirror.
            debug!(
                channel,
                chat_id,
                agent = %agent_name,
                "channel_send mirror: binding matched but agent not registered, falling back to channel default_agent"
            );
        }

        // Fallback: every channel runs as a sidecar; the `default_agent`
        // lookup is a single pass over `cfg.sidecar_channels`.
        let cfg = self.config.load_full();
        match sidecar_default_agent(&cfg.sidecar_channels, channel) {
            Some(agent_name) => {
                debug!(
                    channel,
                    chat_id,
                    agent_name = %agent_name,
                    "channel_send mirror: no binding matched, using channel default_agent"
                );
                self.agents.registry.find_by_name(agent_name).map(|e| e.id)
            }
            None => {
                debug!(
                    channel,
                    chat_id,
                    "channel_send mirror: no binding and no default_agent, owner unresolved"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sidecar_default_agent;
    use librefang_types::config::SidecarChannelConfig;

    /// Build a `SidecarChannelConfig` from a minimal JSON shape — `name` and
    /// `command` are required; everything else (incl. the restart knobs) fills
    /// from serde defaults. `SidecarChannelConfig` derives no `Default`.
    fn sc(json: serde_json::Value) -> SidecarChannelConfig {
        serde_json::from_value(json).expect("valid SidecarChannelConfig")
    }

    #[test]
    fn sidecar_default_agent_matches_by_channel_type_then_name() {
        // `channel_type` is the effective channel key when present.
        let chans = vec![sc(serde_json::json!({
            "name": "my-slack",
            "command": "python3",
            "channel_type": "slack",
            "default_agent": "ops",
        }))];
        assert_eq!(sidecar_default_agent(&chans, "slack"), Some("ops"));
        // No entry for "discord" → None.
        assert_eq!(sidecar_default_agent(&chans, "discord"), None);

        // Falls back to `name` when `channel_type` is absent.
        let chans = vec![sc(serde_json::json!({
            "name": "telegram",
            "command": "python3",
            "default_agent": "tg-bot",
        }))];
        assert_eq!(sidecar_default_agent(&chans, "telegram"), Some("tg-bot"));
    }

    #[test]
    fn sidecar_default_agent_skips_entries_without_agent_and_is_first_match() {
        let chans = vec![
            // Matches the channel but carries no default_agent → skipped.
            sc(serde_json::json!({
                "name": "slack-a", "command": "python3", "channel_type": "slack",
            })),
            // First matching entry WITH an agent wins.
            sc(serde_json::json!({
                "name": "slack-b", "command": "python3", "channel_type": "slack",
                "default_agent": "first",
            })),
            sc(serde_json::json!({
                "name": "slack-c", "command": "python3", "channel_type": "slack",
                "default_agent": "second",
            })),
        ];
        assert_eq!(sidecar_default_agent(&chans, "slack"), Some("first"));

        // An empty default_agent string is treated as unset.
        let chans = vec![sc(serde_json::json!({
            "name": "slack", "command": "python3", "channel_type": "slack",
            "default_agent": "",
        }))];
        assert_eq!(sidecar_default_agent(&chans, "slack"), None);
    }
}
