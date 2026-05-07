//! [`kernel_handle::ChannelSender`] — send text / media / file / poll content
//! to a registered channel adapter, plus roster CRUD. Adapter lookup keys
//! by `"<channel>:<account_id>"` first then falls back to `<channel>` so
//! multi-account installs don't collide.

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

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
        let cfg = self.config.load_full();
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
        let formatted = if channel == "wecom" {
            let output_format = cfg
                .channels
                .wecom
                .as_ref()
                .and_then(|c| c.overrides.output_format)
                .unwrap_or(default_format);
            librefang_channels::formatter::format_for_wecom(message, output_format)
        } else {
            librefang_channels::formatter::format_for_channel(message, default_format)
        };

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

        adapter
            .send(&user, content)
            .await
            .map_err(|e| format!("Channel poll send failed: {e}"))?;

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
}
