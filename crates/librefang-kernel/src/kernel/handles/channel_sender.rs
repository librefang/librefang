//! [`kernel_handle::ChannelSender`] — send text / media / file / poll content
//! to a registered channel adapter, plus roster CRUD. Adapter lookup keys
//! by `"<channel>:<account_id>"` first then falls back to `<channel>` so
//! multi-account installs don't collide.
//!
//! Every channel runs out-of-process as a sidecar; the per-channel
//! `default_agent` lookup is therefore single-pass over
//! `cfg.sidecar_channels` via [`sidecar_default_agent`].

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

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
            entry.default_agent.as_deref().filter(|s| !s.is_empty())
        } else {
            None
        }
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
        let lookup_key = account_id
            .filter(|s| !s.is_empty())
            .map(|aid| format!("{channel}:{aid}"))
            .unwrap_or_else(|| channel.to_string());
        let adapter = self
            .mesh
            .channel_adapters
            .get(&lookup_key)
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .mesh
                    .channel_adapters
                    .iter()
                    .map(|e| e.key().clone())
                    .collect();
                match account_id.filter(|s| !s.is_empty()) {
                    Some(aid) => format!(
                        "Channel '{}' with account_id '{}' not found. Available: {:?}",
                        channel, aid, available
                    ),
                    None => format!(
                        "Channel '{}' not found. Available channels: {:?}",
                        channel, available
                    ),
                }
            })?
            .clone();

        let user = librefang_channels::types::ChannelUser {
            platform_id: recipient.to_string(),
            display_name: recipient.to_string(),
            librefang_user: None,
        };

        let default_format =
            librefang_channels::formatter::default_output_format_for_channel(channel);
        // wecom migrated to a sidecar; its formatting now happens inside
        // the Python adapter (`librefang.sidecar.adapters.wecom`) which
        // wraps every outbound chunk as `msgtype: "markdown"`. The
        // generic `format_for_channel` path with the Markdown default
        // (see `default_output_format_for_channel("wecom")`) gives the
        // sidecar exactly that.
        let formatted = librefang_channels::formatter::format_for_channel(message, default_format);

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
        let lookup_key = account_id
            .filter(|s| !s.is_empty())
            .map(|aid| format!("{channel}:{aid}"))
            .unwrap_or_else(|| channel.to_string());
        let adapter = self
            .mesh
            .channel_adapters
            .get(&lookup_key)
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .mesh
                    .channel_adapters
                    .iter()
                    .map(|e| e.key().clone())
                    .collect();
                match account_id.filter(|s| !s.is_empty()) {
                    Some(aid) => format!(
                        "Channel '{}' with account_id '{}' not found. Available: {:?}",
                        channel, aid, available
                    ),
                    None => format!(
                        "Channel '{}' not found. Available channels: {:?}",
                        channel, available
                    ),
                }
            })?
            .clone();

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
        let lookup_key = account_id
            .filter(|s| !s.is_empty())
            .map(|aid| format!("{channel}:{aid}"))
            .unwrap_or_else(|| channel.to_string());
        let adapter = self
            .mesh
            .channel_adapters
            .get(&lookup_key)
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .mesh
                    .channel_adapters
                    .iter()
                    .map(|e| e.key().clone())
                    .collect();
                match account_id.filter(|s| !s.is_empty()) {
                    Some(aid) => format!(
                        "Channel '{}' with account_id '{}' not found. Available: {:?}",
                        channel, aid, available
                    ),
                    None => format!(
                        "Channel '{}' not found. Available channels: {:?}",
                        channel, available
                    ),
                }
            })?
            .clone();

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
        let lookup_key = account_id
            .filter(|s| !s.is_empty())
            .map(|aid| format!("{channel}:{aid}"))
            .unwrap_or_else(|| channel.to_string());
        let adapter = self
            .mesh
            .channel_adapters
            .get(&lookup_key)
            .ok_or_else(|| match account_id.filter(|s| !s.is_empty()) {
                Some(aid) => {
                    format!("Channel adapter '{channel}' with account_id '{aid}' not found")
                }
                None => format!("Channel adapter '{channel}' not found"),
            })?
            .clone();

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
            .upsert(channel, chat_id, user_id, display_name, username);
        Ok(())
    }

    fn roster_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        let members = self.memory.substrate.roster().members(channel, chat_id);
        Ok(members
            .into_iter()
            .map(|(user_id, display_name, username)| {
                serde_json::json!({
                    "user_id": user_id,
                    "display_name": display_name,
                    "username": username,
                })
            })
            .collect())
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
            .remove_member(channel, chat_id, user_id);
        Ok(())
    }

    fn resolve_channel_owner(
        &self,
        channel: &str,
        _chat_id: &str,
    ) -> Option<librefang_types::agent::AgentId> {
        // Every channel runs as a sidecar; the `default_agent` lookup is
        // a single pass over `cfg.sidecar_channels`.
        let cfg = self.config.load_full();
        let agent_name = sidecar_default_agent(&cfg.sidecar_channels, channel)?;
        self.agents.registry.find_by_name(agent_name).map(|e| e.id)
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
