//! [`kernel_handle::SessionWriter`] — splice content blocks into an agent's
//! current session. Used by the API attachment-upload path to inject
//! image / file blocks ahead of the next user turn so the agent can refer
//! to them by name. Falls back to a fresh session on miss (rare; only when
//! the registered session id has been pruned).

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::SessionWriter for LibreFangKernel {
    /// Serializes with the inbound-router and any concurrent mirror call on
    /// the same agent by acquiring `agent_msg_locks[agent_id]`.
    ///
    /// Using the agent-scoped lock (same key space as `send_message_full`'s
    /// no-override path) ensures that a concurrent `channel_send` mirror and
    /// the live inbound-routing session write cannot race the same JSONL
    /// append.  `session_msg_locks[session_id]` is a *different* key space
    /// and would not exclude the inbound-router writer.
    ///
    /// `block_in_place` is used instead of `blocking_lock()` so this method
    /// is safe to call from both async worker threads and `spawn_blocking`
    /// threads without panicking.  The SQLite write underneath is still
    /// synchronous; `block_in_place` parks the async worker while it runs.
    fn append_to_session(
        &self,
        session_id: librefang_types::agent::SessionId,
        agent_id: librefang_types::agent::AgentId,
        message: librefang_types::message::Message,
    ) {
        // Acquire the per-agent lock using block_in_place so this is safe
        // from both async contexts and spawn_blocking threads.
        let lock = self
            .agents
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = tokio::task::block_in_place(|| lock.blocking_lock());

        // Load existing session or create a fresh one for this (agent, session) pair.
        let mut session = match self.memory.substrate.get_session(session_id) {
            Ok(Some(s)) => s,
            _ => librefang_memory::session::Session {
                id: session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                model_override: None,
                messages_generation: 0,
                last_repaired_generation: None,
            },
        };

        session.push_message(message);
        let total_messages = session.messages.len();

        if let Err(e) = self.memory.substrate.save_session(&session) {
            tracing::warn!(
                agent_id = ?agent_id,
                session_id = ?session_id,
                total_messages,
                error = %e,
                "append_to_session: failed to save session"
            );
        } else {
            tracing::debug!(
                agent_id = ?agent_id,
                session_id = ?session_id,
                total_messages,
                "append_to_session: mirrored channel_send into session"
            );
        }
    }

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

        // Serialize with any concurrent write to the same agent's session.
        // block_in_place is safe from both async worker threads and
        // spawn_blocking threads (unlike blocking_lock which panics in async).
        let lock = self
            .agents
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = tokio::task::block_in_place(|| lock.blocking_lock());

        let mut session = match self.memory.substrate.get_session(entry.session_id) {
            Ok(Some(s)) => s,
            _ => librefang_memory::session::Session {
                id: entry.session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                model_override: None,
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
