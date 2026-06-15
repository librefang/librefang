use async_trait::async_trait;

use super::*;

// ============================================================================
// 10. ChannelSender — outbound channel adapters (text / media / file / poll)
// ============================================================================

#[async_trait]
pub trait ChannelSender: Send + Sync {
    /// Send a message to a user on a named channel adapter (e.g., "email", "telegram").
    /// When `thread_id` is provided, the message is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
    /// Returns a confirmation string on success.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, KernelOpError> {
        let _ = (channel, recipient, message, thread_id, account_id);
        Err(KernelOpError::unavailable("Channel send"))
    }

    /// Send media content (image/file) to a user on a named channel adapter.
    /// `media_type` is "image" or "file", `media_url` is the URL, `caption` is optional text.
    /// When `thread_id` is provided, the media is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
    #[allow(clippy::too_many_arguments)]
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
    ) -> Result<String, KernelOpError> {
        let _ = (
            channel, recipient, media_type, media_url, caption, filename, thread_id, account_id,
        );
        Err(KernelOpError::unavailable("Channel media send"))
    }

    /// Send a local file (raw bytes) to a user on a named channel adapter.
    /// Used by the `channel_send` tool when `file_path` is provided.
    /// When `thread_id` is provided, the file is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
    ///
    /// `data` is a `bytes::Bytes` so wrapping layers (metering, retry,
    /// fan-out to multiple adapters) can `clone()` it for free instead
    /// of cloning the underlying buffer. With the 10 MiB upload bump
    /// (#3514) this avoids per-send buffer copies in every wrapping
    /// layer. See issue #3553.
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
    ) -> Result<String, KernelOpError> {
        let _ = (
            channel, recipient, data, filename, mime_type, thread_id, account_id,
        );
        Err(KernelOpError::unavailable("Channel file data send"))
    }

    #[allow(clippy::too_many_arguments)]
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
    ) -> Result<(), KernelOpError> {
        let _ = (
            channel,
            recipient,
            question,
            options,
            is_quiz,
            correct_option_id,
            explanation,
            thread_id,
            account_id,
        );
        Err(KernelOpError::unavailable("Channel poll send"))
    }

    /// Upsert a group roster member (channel bridge → persistent storage).
    fn roster_upsert(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
        _display_name: &str,
        _username: Option<&str>,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }

    /// List group roster members for a (channel, chat_id) pair.
    fn roster_members(
        &self,
        _channel: &str,
        _chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Remove a member from the group roster.
    fn roster_remove_member(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }

    /// Resolve the agent that owns a given `(channel, chat_id)` pair.
    ///
    /// Returns the `AgentId` of the agent whose channel config has
    /// `default_agent` pointing at the named channel instance.  Used by
    /// `tool_channel_send` to mirror outbound messages into the inbound-
    /// routing session so the channel-owning agent has context for the
    /// user's reply.
    ///
    /// Returns `None` when no agent is bound to that channel (e.g. in test
    /// stubs or when the channel has no `default_agent` configured).
    fn resolve_channel_owner(
        &self,
        _channel: &str,
        _chat_id: &str,
    ) -> Option<librefang_types::agent::AgentId> {
        None
    }
}
