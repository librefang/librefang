//! [`kernel_handle::SessionWriter`] — splice content blocks into an agent's
//! current session. Used by the API attachment-upload path to inject
//! image / file blocks ahead of the next user turn so the agent can refer
//! to them by name. Falls back to a fresh session on miss (rare; only when
//! the registered session id has been pruned).

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::SessionWriter for LibreFangKernel {
    fn inject_attachment_blocks(
        &self,
        agent_id: librefang_types::agent::AgentId,
        blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
        use librefang_types::message::{Message, MessageContent, Role};

        let entry = match self.agents.registry.get(agent_id) {
            Some(e) => e,
            None => {
                tracing::warn!(
                    agent_id = ?agent_id,
                    "inject_attachment_blocks: agent not found in registry"
                );
                return;
            }
        };

        let mut session = match self.memory.substrate.get_session(entry.session_id) {
            Ok(Some(s)) => s,
            _ => librefang_memory::session::Session {
                id: entry.session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                messages_generation: 0,
                last_repaired_generation: None,
            },
        };

        let block_count = blocks.len();
        let block_kinds: Vec<&'static str> = blocks
            .iter()
            .map(|b| match b {
                librefang_types::message::ContentBlock::Image { .. } => "image",
                librefang_types::message::ContentBlock::Text { .. } => "text",
                librefang_types::message::ContentBlock::ImageFile { .. } => "image_file",
                librefang_types::message::ContentBlock::ToolUse { .. } => "tool_use",
                librefang_types::message::ContentBlock::ToolResult { .. } => "tool_result",
                librefang_types::message::ContentBlock::Thinking { .. } => "thinking",
                librefang_types::message::ContentBlock::Unknown => "unknown",
            })
            .collect();

        session.push_message(Message {
            role: Role::User,
            content: MessageContent::Blocks(blocks),
            pinned: false,
            timestamp: Some(chrono::Utc::now()),
        });

        let total_messages_after = session.messages.len();

        if let Err(e) = self.memory.substrate.save_session(&session) {
            tracing::warn!(
                agent_id = ?agent_id,
                session_id = ?entry.session_id,
                block_count,
                error = %e,
                "inject_attachment_blocks: failed to save session"
            );
        } else {
            tracing::info!(
                agent_id = ?agent_id,
                session_id = ?entry.session_id,
                block_count,
                block_kinds = ?block_kinds,
                total_messages_after,
                "inject_attachment_blocks: injected content blocks into session"
            );
        }
    }
}
