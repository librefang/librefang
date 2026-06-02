// ============================================================================
// 16. SessionWriter — pre-inject content blocks into an agent session before
//     an LLM turn, used by the HTTP attachment upload path (#3744).
//
//     Abstracts over `agent_registry()` + `memory_substrate()` so callers
//     in librefang-api do not need to import the concrete kernel type.
// ============================================================================

pub trait SessionWriter: Send + Sync {
    /// Pre-insert `blocks` as a User-role message into the **specific**
    /// session identified by `session_id` so the LLM sees the content in
    /// the next turn.  No-op (with a `warn!`) when the agent is not found;
    /// best-effort on save failure.
    ///
    /// **Session isolation invariant (2026-05-20 incident).** Callers MUST
    /// derive `session_id` with the *same* resolver used by the matching
    /// `send_message_*` call for this turn (see
    /// `kernel::messaging::send_message_streaming_with_incognito` /
    /// `send_message_with_incognito`): explicit override wins, otherwise
    /// `SessionId::for_sender_scope(agent, channel, chat_id)` for
    /// channel-scoped turns, otherwise the agent's persistent
    /// `entry.session_id`. Passing the agent's default registry session
    /// when the text part of the same request will land on a
    /// channel-derived session causes a cross-chat leak — the bug fixed
    /// alongside this signature change. The implementation MUST write into
    /// the SPECIFIC `session_id` and must NOT silently fall back to
    /// `entry.session_id` on its own.
    ///
    /// **Blocking I/O notice.**  The current production implementation
    /// (`LibreFangKernel`) calls `MemorySubstrate::save_session` synchronously,
    /// which blocks on a SQLite write.  Callers running inside an async
    /// runtime should wrap the call in `tokio::task::spawn_blocking` to
    /// avoid stalling worker threads under contention. (#3579 will move the
    /// substrate to `tokio::fs`-aware async; once that lands, the trait
    /// itself can become `async fn` and this caveat goes away.)
    fn inject_attachment_blocks(
        &self,
        agent_id: librefang_types::agent::AgentId,
        session_id: librefang_types::agent::SessionId,
        blocks: Vec<librefang_types::message::ContentBlock>,
    );

    /// Append a single message to an existing session identified by
    /// `session_id`.  Used by `tool_channel_send` to mirror outbound
    /// messages into the channel-owner agent's inbound-routing session.
    ///
    /// Best-effort: implementations should log a `warn!` on failure rather
    /// than propagating the error — the platform send already succeeded and
    /// the caller must not fail the tool call because of a persistence blip.
    ///
    /// **Blocking I/O notice** — same caveat as `inject_attachment_blocks`.
    fn append_to_session(
        &self,
        session_id: librefang_types::agent::SessionId,
        agent_id: librefang_types::agent::AgentId,
        message: librefang_types::message::Message,
    ) {
        let _ = (session_id, agent_id, message);
    }
}
