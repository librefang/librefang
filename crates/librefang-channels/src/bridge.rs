//! Channel bridge — connects channel adapters to the LibreFang kernel.
//!
//! Defines `ChannelBridgeHandle` (implemented by librefang-api on the kernel) and
//! `BridgeManager` which owns running adapters and dispatches messages.

use crate::formatter;
use crate::rate_limiter::ChannelRateLimiter;
use crate::router::AgentRouter;
use crate::sanitizer::{InputSanitizer, SanitizeResult};
use crate::types::{
    default_phase_emoji, truncate_utf8, AgentPhase, ChannelAdapter, ChannelContent, ChannelMessage,
    ChannelUser, GroupMember, InteractiveButton, LifecycleReaction, ParticipantRef, SenderContext,
};
use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::agent::AgentId;
use librefang_types::config::{
    AutoRouteStrategy, ChannelOverrides, DmPolicy, GroupPolicy, OutputFormat, PrefixStyle,
};
use librefang_types::message::ContentBlock;
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

/// Two-channel reply envelope returned by the bridge. The `public` field is
/// what should reach the source chat (DM or group). The `owner_notice` field
/// is a structured private message intended for the operator's DM only —
/// e.g. produced by the `notify_owner` LLM tool. Adapters that don't support
/// owner-side delivery should ignore `owner_notice` and forward only `public`.
///
/// Both fields are `Option` so legacy/silent paths can carry "no public reply"
/// (`public = None`) without losing an `owner_notice`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyEnvelope {
    #[serde(default)]
    pub public: Option<String>,
    #[serde(default)]
    pub owner_notice: Option<String>,
}

impl ReplyEnvelope {
    /// Build an envelope carrying only a public reply (no owner notice).
    pub fn from_public(s: impl Into<String>) -> Self {
        Self {
            public: Some(s.into()),
            owner_notice: None,
        }
    }

    /// Build an envelope with no public reply and no owner notice (silent turn).
    pub fn silent() -> Self {
        Self::default()
    }

    /// Convenience: extract the public text or empty string. Used by adapters
    /// that don't yet route the owner_notice channel — they still get the
    /// behaviour of the previous `Result<String, String>` API.
    pub fn public_or_empty(&self) -> String {
        self.public.clone().unwrap_or_default()
    }
}

/// Kernel operations needed by channel adapters.
///
/// Defined here to avoid circular deps (librefang-channels can't depend on librefang-kernel).
/// Implemented in librefang-api on the actual kernel.
#[async_trait]
pub trait ChannelBridgeHandle: Send + Sync {
    /// Send a message to an agent and get the text response.
    async fn send_message(&self, agent_id: AgentId, message: &str) -> Result<String, String>;

    /// Send a message with structured content blocks (text + images) to an agent.
    ///
    /// Default implementation extracts text from blocks and falls back to `send_message()`.
    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        blocks: Vec<ContentBlock>,
    ) -> Result<String, String> {
        // Default: extract text from blocks and send as plain text
        let text: String = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.send_message(agent_id, &text).await
    }

    /// Send a message to an agent with sender identity context.
    ///
    /// The sender context is propagated to the agent's system prompt so it knows
    /// who is talking and from which channel. Default falls back to `send_message()`.
    async fn send_message_with_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> Result<String, String> {
        let _ = sender;
        self.send_message(agent_id, message).await
    }

    /// Send a multimodal message with sender identity context.
    ///
    /// Default falls back to `send_message_with_blocks()`.
    async fn send_message_with_blocks_and_sender(
        &self,
        agent_id: AgentId,
        blocks: Vec<ContentBlock>,
        sender: &SenderContext,
    ) -> Result<String, String> {
        let _ = sender;
        self.send_message_with_blocks(agent_id, blocks).await
    }

    /// Find an agent by name, returning its ID.
    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String>;

    /// List running agents as (id, name) pairs.
    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String>;

    /// Spawn an agent by manifest name, returning its ID.
    async fn spawn_agent_by_name(&self, manifest_name: &str) -> Result<AgentId, String>;

    /// Return uptime info string (e.g., "2h 15m, 5 agents").
    async fn uptime_info(&self) -> String {
        let agents = self.list_agents().await.unwrap_or_default();
        format!("{} agent(s) running", agents.len())
    }

    /// List available models as formatted text for channel display.
    async fn list_models_text(&self) -> String {
        "Model listing not available.".to_string()
    }

    /// List providers and their auth status as formatted text for channel display.
    async fn list_providers_text(&self) -> String {
        "Provider listing not available.".to_string()
    }

    /// Return (provider_id, display_name, auth_ok) for each provider.
    async fn list_providers_interactive(&self) -> Vec<(String, String, bool)> {
        Vec::new()
    }

    /// Return (model_id, display_name) for models belonging to the given provider.
    async fn list_models_by_provider(&self, _provider_id: &str) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Send an ephemeral "side question" (`/btw`) — answered with the agent's system
    /// prompt but without loading or saving session history.
    async fn send_message_ephemeral(
        &self,
        _agent_id: AgentId,
        _message: &str,
    ) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Reset every session for an agent (default + per-channel + cron).
    /// Used by surfaces that mean "wipe this agent" — dashboard / explicit
    /// admin reset. Channel `/new` should call [`Self::reset_channel_session`]
    /// instead so other surfaces are not collateral damage (#4868).
    async fn reset_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Hard-reboot every session for an agent — full context clear without
    /// saving summaries. Channel `/reboot` should call
    /// [`Self::reboot_channel_session`] instead (#4868).
    async fn reboot_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Trigger LLM-based session compaction for an agent's registry-pointer
    /// session. Channel `/compact` should call
    /// [`Self::compact_channel_session`] instead so it operates on the
    /// per-channel session the user is actually chatting in (#4868).
    async fn compact_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Reset only the session derived from `(channel, chat_id)` — the
    /// per-channel session that channel `/new` actually means to clear
    /// (#4868). Sibling sessions (other channels, dashboard) stay intact.
    ///
    /// `chat_id` follows the inbound-message convention: `None` for an
    /// adapter that doesn't disambiguate by chat (the channel name itself
    /// becomes the scope), `Some(<sender.platform_id>)` otherwise — the same
    /// pair the channel resolver uses to derive
    /// [`librefang_types::agent::SessionId::for_channel`] for inbound traffic.
    async fn reset_channel_session(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _chat_id: Option<&str>,
    ) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Hard-reboot only the per-channel session derived from
    /// `(channel, chat_id)` — no summary saved (#4868).
    async fn reboot_channel_session(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _chat_id: Option<&str>,
    ) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Compact only the per-channel session derived from
    /// `(channel, chat_id)` — operates on the session the channel user is
    /// chatting in, not the agent's registry-pointer session (#4868).
    async fn compact_channel_session(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _chat_id: Option<&str>,
    ) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Set an agent's model.
    async fn set_model(&self, _agent_id: AgentId, _model: &str) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Stop an agent's current LLM run.
    async fn stop_run(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Get session token usage and estimated cost.
    async fn session_usage(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Toggle extended thinking mode for an agent.
    async fn set_thinking(&self, _agent_id: AgentId, _on: bool) -> Result<String, String> {
        Ok("Extended thinking preference saved.".to_string())
    }

    /// List installed skills as formatted text for channel display.
    async fn list_skills_text(&self) -> String {
        "Skill listing not available.".to_string()
    }

    /// List hands (marketplace + active) as formatted text for channel display.
    async fn list_hands_text(&self) -> String {
        "Hand listing not available.".to_string()
    }

    /// Authorize a channel user for an action.
    ///
    /// Returns Ok(()) if the user is allowed, Err(reason) if denied.
    /// Default implementation: allow all (RBAC disabled).
    async fn authorize_channel_user(
        &self,
        _channel_type: &str,
        _platform_id: &str,
        _action: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Get per-channel overrides for a given channel type.
    ///
    /// Returns `None` if the channel is not configured or has no overrides.
    async fn channel_overrides(
        &self,
        _channel_type: &str,
        _account_id: Option<&str>,
    ) -> Option<ChannelOverrides> {
        None
    }

    /// When an agent declares `[channel_overrides]` in its `agent.toml`,
    /// those values take priority over the channel-level overrides.
    /// Returns `None` if the agent has no per-agent overrides configured.
    async fn agent_channel_overrides(&self, _agent_id: AgentId) -> Option<ChannelOverrides> {
        None
    }

    /// Already-escaped regex patterns from `channel_overrides.group_trigger_patterns`; callers must not re-escape.
    async fn get_agent_group_trigger_patterns(&self, _agent_id: AgentId) -> Vec<String> {
        Vec::new()
    }

    /// Persist a group roster member to the kernel's persistent storage.
    async fn roster_upsert(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
        _display_name: &str,
        _username: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Lightweight LLM classification: should the bot reply to this group message?
    ///
    /// Returns `true` if the bot should reply, `false` to stay silent.
    /// Default implementation always returns `true` (fail-open).
    async fn classify_reply_intent(
        &self,
        _message_text: &str,
        _sender_name: &str,
        _model: Option<&str>,
        _bot_name: Option<&str>,
        _aliases: Option<&[String]>,
    ) -> bool {
        true
    }

    /// Record a delivery result for tracking (optional — default no-op).
    ///
    /// `thread_id` preserves Telegram forum-topic context so cron/workflow
    /// delivery can target the same topic later.
    async fn record_delivery(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _recipient: &str,
        _success: bool,
        _error: Option<&str>,
        _thread_id: Option<&str>,
    ) {
        // Default: no tracking
    }

    /// Check if auto-reply is enabled and the message should trigger one.
    /// Returns Some(reply_text) if auto-reply fires, None otherwise.
    async fn check_auto_reply(&self, _agent_id: AgentId, _message: &str) -> Option<String> {
        None
    }

    // ── Automation: workflows, triggers, schedules, approvals ──

    /// List all registered workflows as formatted text.
    async fn list_workflows_text(&self) -> String {
        "Workflows not available.".to_string()
    }

    /// Run a workflow by name with the given input text.
    async fn run_workflow_text(&self, _name: &str, _input: &str) -> String {
        "Workflows not available.".to_string()
    }

    /// List all registered triggers as formatted text.
    async fn list_triggers_text(&self) -> String {
        "Triggers not available.".to_string()
    }

    /// Create a trigger for an agent with the given pattern and prompt.
    async fn create_trigger_text(
        &self,
        _agent_name: &str,
        _pattern: &str,
        _prompt: &str,
    ) -> String {
        "Triggers not available.".to_string()
    }

    /// Delete a trigger by UUID prefix.
    async fn delete_trigger_text(&self, _id_prefix: &str) -> String {
        "Triggers not available.".to_string()
    }

    /// List all cron jobs as formatted text.
    async fn list_schedules_text(&self) -> String {
        "Schedules not available.".to_string()
    }

    /// Manage a cron job: add, del, or run.
    async fn manage_schedule_text(&self, _action: &str, _args: &[String]) -> String {
        "Schedules not available.".to_string()
    }

    /// List pending approval requests as formatted text.
    async fn list_approvals_text(&self) -> String {
        "No approvals pending.".to_string()
    }

    /// Approve or reject a pending approval by UUID prefix.
    ///
    /// When `totp_code` is provided, it is used for TOTP second-factor
    /// verification on approve actions. `sender_id` identifies the user for
    /// per-user TOTP failure tracking.
    async fn resolve_approval_text(
        &self,
        _id_prefix: &str,
        _approve: bool,
        _totp_code: Option<&str>,
        _sender_id: &str,
    ) -> String {
        "Approvals not available.".to_string()
    }

    /// Subscribe to system events (including approval requests).
    ///
    /// Returns a broadcast receiver for kernel events. Channel adapters can
    /// listen for `ApprovalRequested` events and send interactive messages.
    /// Default returns None (event subscription not available).
    async fn subscribe_events(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<librefang_types::event::Event>>>
    {
        None
    }

    /// Record that the consumer side dropped `n` events due to broadcast
    /// lag. Called by listeners that receive from [`subscribe_events`] when
    /// they observe `RecvError::Lagged(n)`. The production impl forwards
    /// to `EventBus::record_consumer_lag` so lag drops show up in the
    /// kernel's `dropped_count` metric and trigger a rate-limited
    /// `error!` log (issue #3630).
    ///
    /// No default impl on purpose: a default no-op would let any future
    /// production handle silently inherit the no-op and swallow lag
    /// drops, re-defeating #3630 with no compiler signal. Test mocks
    /// that have no event bus to forward to should write an explicit
    /// `fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {}`
    /// to acknowledge the requirement; that one line is cheaper than
    /// chasing another silent-drop regression.
    fn record_consumer_lag(&self, n: u64, context: &'static str);

    // ── Budget, Network, A2A ──

    /// Show global budget status (limits, spend, % used).
    async fn budget_text(&self) -> String {
        "Budget information not available.".to_string()
    }

    /// Show OFP peer network status.
    async fn peers_text(&self) -> String {
        "Peer network not available.".to_string()
    }

    /// List discovered external A2A agents.
    async fn a2a_agents_text(&self) -> String {
        "A2A agents not available.".to_string()
    }

    /// Send a message to an agent and stream text deltas back.
    ///
    /// Returns a receiver of incremental text chunks. Adapters that support
    /// streaming (e.g. Telegram) can display tokens progressively instead of
    /// waiting for the full response.
    ///
    /// Default implementation falls back to `send_message()` and emits the
    /// complete response as a single chunk.
    async fn send_message_streaming(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> Result<mpsc::Receiver<String>, String> {
        let response = self.send_message(agent_id, message).await?;
        let (tx, rx) = mpsc::channel(1);
        if let Err(e) = tx.send(response).await {
            // Receiver was dropped before we could push the single chunk;
            // caller will see an empty stream. Surface for debugging since
            // this is the default fallback path used when adapters don't
            // implement true streaming.
            warn!(error = %e, "send_message_streaming default fallback: receiver dropped before response delivery");
        }
        Ok(rx)
    }

    /// Send a message with sender identity context and stream text deltas back.
    ///
    /// Default implementation preserves existing streaming behavior and ignores
    /// the sender context for handles that do not support it.
    async fn send_message_streaming_with_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> Result<mpsc::Receiver<String>, String> {
        let _ = sender;
        self.send_message_streaming(agent_id, message).await
    }

    /// Streaming send that *also* reports the kernel's terminal success/error
    /// once the stream completes. Callers that need accurate delivery metrics,
    /// lifecycle reactions, and error suppression should use this variant —
    /// the plain `send_message_streaming_with_sender` collapses everything
    /// into the text channel, which makes it impossible to distinguish a
    /// successful reply from a sanitized error message after the fact.
    ///
    /// The oneshot resolves to `Ok(())` on success and `Err(error_string)` on
    /// failure (panic, kernel error, or LLM error). It is sent only once the
    /// kernel join handle has resolved, so awaiting it after draining the
    /// text receiver is safe.
    ///
    /// Default implementation preserves existing behavior by reporting
    /// fake-success — implementers that can detect kernel failure (e.g. the
    /// real `LibreFangKernel` impl) should override this to surface real
    /// status.
    async fn send_message_streaming_with_sender_status(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> Result<
        (
            mpsc::Receiver<String>,
            tokio::sync::oneshot::Receiver<Result<(), String>>,
        ),
        String,
    > {
        let rx = self
            .send_message_streaming_with_sender(agent_id, message, sender)
            .await?;
        let (status_tx, status_rx) = tokio::sync::oneshot::channel();
        if status_tx.send(Ok(())).is_err() {
            // The receiver half was dropped before we could report status.
            // Default impl reports fake-success, so losing it just means the
            // caller stopped caring — log at debug for visibility.
            debug!("send_message_streaming_with_sender_status: status receiver dropped before fake-success report");
        }
        Ok((rx, status_rx))
    }

    /// Push a proactive outbound message to a channel recipient.
    ///
    /// Used by the REST API push endpoint (`POST /api/agents/:id/push`) to let
    /// external callers send messages through a configured channel adapter without
    /// going through the agent loop. The `thread_id` is optional and adapter-specific.
    async fn send_channel_push(
        &self,
        _channel_type: &str,
        _recipient: &str,
        _message: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        Err("Channel push not available".to_string())
    }

    // ── File download config accessors ──

    /// Return the configured file download directory, if set.
    fn channels_download_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Return the effective file download directory: configured value or
    /// the legacy `<temp>/librefang_uploads` default. Use this everywhere
    /// instead of re-deriving the fallback inline (see issue #4435).
    fn effective_channels_download_dir(&self) -> std::path::PathBuf {
        self.channels_download_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"))
    }

    /// Return the configured max file download size in bytes, if set.
    fn channels_download_max_bytes(&self) -> Option<u64> {
        None
    }
}

struct PendingMessage {
    message: ChannelMessage,
    image_blocks: Option<Vec<ContentBlock>>,
}

struct SenderBuffer {
    messages: Vec<PendingMessage>,
    first_arrived: Instant,
    timer_handle: Option<tokio::task::JoinHandle<()>>,
    max_timer_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Backpressure cap for the debouncer flush channel (#3580). Bridges
/// previously used an unbounded channel here; if the dispatcher stalled
/// (rate-limited Telegram, paused agent, etc.) the queue grew until OOM.
const FLUSH_CHANNEL_CAP: usize = 1024;

struct MessageDebouncer {
    debounce_ms: u64,
    debounce_max_ms: u64,
    max_buffer: usize,
    flush_tx: mpsc::Sender<String>,
}

/// Log a `MessageDebouncer` flush-channel send failure at `warn` level.
///
/// All five flush trigger paths (max-timer, immediate, debounce-timer,
/// typing-triggered, typing-stop) share two failure modes — the
/// dispatcher's receiver has been dropped, or the bounded flush channel
/// is full because the dispatcher is stalled (#3580). In either case the
/// buffered message is dropped. `location` distinguishes the trigger in
/// logs as a structured field; `key` is the debouncer key when the call
/// site has it on hand (the spawn'd timer paths consume it before the
/// send and pass `None`).
fn warn_flush_dropped<E: std::fmt::Display>(
    result: Result<(), E>,
    location: &'static str,
    key: Option<&str>,
) {
    if let Err(e) = result {
        warn!(
            error = %e,
            key = key.unwrap_or(""),
            location,
            "Debouncer flush dropped: dispatch receiver closed or flush channel full",
        );
    }
}

impl MessageDebouncer {
    fn new(
        debounce_ms: u64,
        debounce_max_ms: u64,
        max_buffer: usize,
    ) -> (Self, mpsc::Receiver<String>) {
        // Bounded to bound RSS when downstream dispatcher stalls (#3580).
        // Cap is generous: the queue is keyed by sender (one entry per
        // distinct (channel, chat) pair within a debounce window), so 1024
        // accommodates large fan-out without uncapped growth.
        let (flush_tx, flush_rx) = mpsc::channel(FLUSH_CHANNEL_CAP);
        (
            Self {
                debounce_ms,
                debounce_max_ms,
                max_buffer,
                flush_tx,
            },
            flush_rx,
        )
    }

    fn push(
        &self,
        key: &str,
        pending: PendingMessage,
        buffers: &mut HashMap<String, SenderBuffer>,
    ) {
        use std::time::Duration;
        let debounce_dur = Duration::from_millis(self.debounce_ms);
        let max_dur = Duration::from_millis(self.debounce_max_ms);

        let buf = buffers.entry(key.to_string()).or_insert_with(|| {
            let flush_tx = self.flush_tx.clone();
            let flush_key = key.to_string();
            let max_timer_handle = Some(tokio::spawn(async move {
                tokio::time::sleep(max_dur).await;
                // Dispatcher receiver gone — buffered messages for this
                // sender will be dropped. Usually only happens during
                // shutdown.
                warn_flush_dropped(flush_tx.send(flush_key).await, "max-timer", None);
            }));
            SenderBuffer {
                messages: Vec::new(),
                first_arrived: Instant::now(),
                timer_handle: None,
                max_timer_handle,
            }
        });
        buf.messages.push(pending);

        if let Some(handle) = buf.timer_handle.take() {
            handle.abort();
        }

        let elapsed = buf.first_arrived.elapsed();
        if elapsed >= max_dur || buf.messages.len() >= self.max_buffer {
            if let Some(handle) = buf.max_timer_handle.take() {
                handle.abort();
            }
            // Guard against double-fire (#3742): the max_timer task may have
            // already enqueued its flush message before we could abort() it.
            // The double-fire is suppressed by `drain()` below — once the
            // first flush key is processed, the entry is removed from
            // `buffers`, so the stale key will find nothing and return None.
            warn_flush_dropped(
                self.flush_tx.try_send(key.to_string()),
                "immediate",
                Some(key),
            );
            return;
        }

        let remaining_cap = max_dur.saturating_sub(elapsed);
        let delay = debounce_dur.min(remaining_cap);
        let flush_tx = self.flush_tx.clone();
        let flush_key = key.to_string();
        buf.timer_handle = Some(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            warn_flush_dropped(flush_tx.send(flush_key).await, "debounce-timer", None);
        }));
    }

    fn on_typing(&self, key: &str, is_typing: bool, buffers: &mut HashMap<String, SenderBuffer>) {
        use std::time::Duration;
        let Some(buf) = buffers.get_mut(key) else {
            return;
        };

        let max_dur = Duration::from_millis(self.debounce_max_ms);
        let elapsed = buf.first_arrived.elapsed();
        if elapsed >= max_dur {
            warn_flush_dropped(
                self.flush_tx.try_send(key.to_string()),
                "typing-triggered",
                Some(key),
            );
            return;
        }

        if let Some(handle) = buf.timer_handle.take() {
            handle.abort();
        }

        if !is_typing {
            let remaining_cap = max_dur.saturating_sub(elapsed);
            let delay = Duration::from_millis(self.debounce_ms).min(remaining_cap);
            let flush_tx = self.flush_tx.clone();
            let flush_key = key.to_string();
            buf.timer_handle = Some(tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                warn_flush_dropped(flush_tx.send(flush_key).await, "typing-stop", None);
            }));
        }
    }

    fn drain(
        &self,
        key: &str,
        buffers: &mut HashMap<String, SenderBuffer>,
    ) -> Option<(ChannelMessage, Option<Vec<ContentBlock>>)> {
        // Guard against double-fire (#3742): if the manual-flush path in
        // `push()` and a max_timer task both enqueue the same key, the second
        // drain call will find the entry already gone and return `None` here.
        let buf = buffers.remove(key)?;
        if buf.messages.is_empty() {
            return None;
        }

        if let Some(handle) = buf.max_timer_handle {
            handle.abort();
        }
        if let Some(handle) = buf.timer_handle {
            handle.abort();
        }

        let mut messages = buf.messages;
        if messages.len() == 1 {
            let pm = messages.remove(0);
            return Some((pm.message, pm.image_blocks));
        }

        let first = messages.remove(0);
        let mut merged_msg = first.message;
        let mut all_blocks: Vec<ContentBlock> = Vec::new();

        if let Some(blocks) = first.image_blocks {
            all_blocks.extend(blocks);
        }

        let first_content_type = std::mem::discriminant(&merged_msg.content);
        let mut all_same_type = true;
        let mut all_commands_same_name: Option<String> = None;

        if matches!(merged_msg.content, ChannelContent::Command { .. }) {
            if let ChannelContent::Command { name, .. } = &merged_msg.content {
                all_commands_same_name = Some(name.clone());
            }
        }

        for pm in &messages {
            if std::mem::discriminant(&pm.message.content) != first_content_type {
                all_same_type = false;
                break;
            }
            if let Some(name) = &all_commands_same_name {
                if let ChannelContent::Command { name: n, .. } = &pm.message.content {
                    if n != name {
                        all_commands_same_name = None;
                        break;
                    }
                } else {
                    all_commands_same_name = None;
                    break;
                }
            }
        }

        if all_same_type {
            if let Some(command_name) = all_commands_same_name {
                let mut cmd_args: Vec<String> = Vec::new();
                if let ChannelContent::Command { args, .. } = &merged_msg.content {
                    cmd_args.extend(args.clone());
                }
                for pm in messages {
                    if let ChannelContent::Command { args, .. } = pm.message.content {
                        cmd_args.extend(args);
                    }
                    if let Some(blocks) = pm.image_blocks {
                        all_blocks.extend(blocks);
                    }
                }
                merged_msg.content = ChannelContent::Command {
                    name: command_name,
                    args: cmd_args,
                };
            } else {
                let mut text_parts = vec![content_to_text(&merged_msg.content)];
                for pm in messages {
                    text_parts.push(content_to_text(&pm.message.content));
                    if let Some(blocks) = pm.image_blocks {
                        all_blocks.extend(blocks);
                    }
                }
                merged_msg.content = ChannelContent::Text(text_parts.join("\n"));
            }
        } else {
            let mut text_parts = vec![content_to_text(&merged_msg.content)];
            for pm in messages {
                text_parts.push(content_to_text(&pm.message.content));
                if let Some(blocks) = pm.image_blocks {
                    all_blocks.extend(blocks);
                }
            }
            merged_msg.content = ChannelContent::Text(text_parts.join("\n"));
        }

        let blocks = if all_blocks.is_empty() {
            None
        } else {
            Some(all_blocks)
        };

        Some((merged_msg, blocks))
    }
}

fn content_to_text(content: &ChannelContent) -> String {
    match content {
        ChannelContent::Text(t) => t.clone(),
        ChannelContent::Command { name, args } => {
            if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {}", args.join(" "))
            }
        }
        ChannelContent::Image { url, caption, .. } => match caption {
            Some(c) => format!("[Photo: {url}]\n{c}"),
            None => format!("[Photo: {url}]"),
        },
        ChannelContent::File { url, filename } => format!("[File ({filename}): {url}]"),
        ChannelContent::Voice {
            url,
            duration_seconds,
            caption,
        } => {
            let cap = caption.as_deref().unwrap_or("");
            if cap.is_empty() {
                format!("[Voice message ({duration_seconds}s): {url}]")
            } else {
                format!("[Voice message ({duration_seconds}s): {url}] {cap}")
            }
        }
        ChannelContent::Video {
            url,
            caption,
            duration_seconds,
            ..
        } => match caption {
            Some(c) => format!("[Video ({duration_seconds}s): {url}]\n{c}"),
            None => format!("[Video ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Location { lat, lon } => format!("[Location: {lat}, {lon}]"),
        ChannelContent::FileData { filename, .. } => format!("[File: {filename}]"),
        ChannelContent::Interactive { text, .. } => text.clone(),
        ChannelContent::ButtonCallback { action, .. } => format!("[Button: {action}]"),
        ChannelContent::DeleteMessage { message_id } => {
            format!("[Delete message: {message_id}]")
        }
        ChannelContent::EditInteractive { text, .. } => text.clone(),
        ChannelContent::Audio {
            url,
            caption,
            duration_seconds,
            ..
        } => match caption {
            Some(c) => format!("[Audio ({duration_seconds}s): {url}]\n{c}"),
            None => format!("[Audio ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Animation {
            url,
            caption,
            duration_seconds,
        } => match caption {
            Some(c) => format!("[Animation ({duration_seconds}s): {url}]\n{c}"),
            None => format!("[Animation ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Sticker { file_id } => format!("[Sticker: {file_id}]"),
        ChannelContent::MediaGroup { items } => format!("[Media group: {} items]", items.len()),
        ChannelContent::Poll { question, .. } => format!("[Poll: {question}]"),
        ChannelContent::PollAnswer {
            poll_id,
            option_ids,
        } => {
            format!("[Poll answer: poll={poll_id}, options={option_ids:?}]")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_debounced(
    debouncer: &MessageDebouncer,
    key: &str,
    buffers: &mut HashMap<String, SenderBuffer>,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &Arc<dyn ChannelAdapter>,
    rate_limiter: &ChannelRateLimiter,
    sanitizer: &Arc<InputSanitizer>,
    semaphore: &Arc<tokio::sync::Semaphore>,
    journal: &Option<crate::message_journal::MessageJournal>,
    thread_ownership: &Arc<crate::thread_ownership::ThreadOwnershipRegistry>,
) -> Option<tokio::task::JoinHandle<()>> {
    let (merged_msg, blocks) = debouncer.drain(key, buffers)?;

    let channel_handle = (*handle).clone();
    let router = router.clone();
    let adapter = adapter.clone();
    let rate_limiter = rate_limiter.clone();
    let sanitizer = Arc::clone(sanitizer);
    let journal = journal.clone();
    let sem = semaphore.clone();
    let thread_ownership = Arc::clone(thread_ownership);

    let join_handle = tokio::spawn(async move {
        let _permit = match sem.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        if let Some(mut blocks) = blocks {
            let text = content_to_text(&merged_msg.content);
            if !text.is_empty() {
                blocks.insert(
                    0,
                    ContentBlock::Text {
                        text,
                        provider_metadata: None,
                    },
                );
            }

            let ct_str = channel_type_str(&merged_msg.channel);

            // --- Input sanitization (prompt injection detection) ---
            if !sanitizer.is_off() {
                // Command-type messages are checked by reconstructing their text
                // so that slash-command args cannot carry prompt-injection payloads.
                let text_to_check: Option<String> = match &merged_msg.content {
                    ChannelContent::Text(t) => Some(t.clone()),
                    ChannelContent::Command { name, args } => {
                        if args.is_empty() {
                            Some(format!("/{name}"))
                        } else {
                            Some(format!("/{name} {}", args.join(" ")))
                        }
                    }
                    ChannelContent::Image { caption, .. } => caption.clone(),
                    ChannelContent::Voice { caption, .. } => caption.clone(),
                    ChannelContent::Video { caption, .. } => caption.clone(),
                    _ => None,
                };
                let message_type = match &merged_msg.content {
                    ChannelContent::Command { .. } => "Command",
                    _ => "User",
                };
                if let Some(ref text) = text_to_check {
                    match sanitizer.check(text) {
                        SanitizeResult::Clean => {}
                        SanitizeResult::Warned(reason) => {
                            warn!(
                                channel = ct_str,
                                user = %merged_msg.sender.display_name,
                                message_type = message_type,
                                reason = reason.as_str(),
                                "Suspicious channel input (warn mode, allowing through)"
                            );
                        }
                        SanitizeResult::Blocked(reason) => {
                            warn!(
                                channel = ct_str,
                                source = %merged_msg.sender.display_name,
                                message_type = message_type,
                                reason = reason.as_str(),
                                "Input sanitizer blocked potential prompt injection in {message_type} message from {}"
                                , merged_msg.sender.display_name,
                            );
                            if let Err(e) = adapter
                                .send(
                                    &merged_msg.sender,
                                    ChannelContent::Text(
                                        "Your message could not be processed.".to_string(),
                                    ),
                                )
                                .await
                            {
                                warn!(
                                    channel = ct_str,
                                    recipient = %merged_msg.sender.display_name,
                                    error = %e,
                                    "Failed to deliver sanitizer-block notice to user",
                                );
                            }
                            return;
                        }
                    }
                }
            }

            let overrides = channel_handle
                .channel_overrides(
                    ct_str,
                    merged_msg
                        .metadata
                        .get("account_id")
                        .and_then(|v| v.as_str()),
                )
                .await;
            let channel_default_format = default_output_format_for_channel(ct_str);
            let output_format = overrides
                .as_ref()
                .and_then(|o| o.output_format)
                .unwrap_or(channel_default_format);
            let threading_enabled = overrides.as_ref().map(|o| o.threading).unwrap_or(false);
            let thread_id = if threading_enabled {
                merged_msg.thread_id.as_deref()
            } else {
                None
            };

            dispatch_with_blocks(
                blocks,
                &merged_msg,
                &channel_handle,
                &router,
                adapter.as_ref(),
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal.as_ref(),
                &thread_ownership,
            )
            .await;
        } else {
            dispatch_message(
                &merged_msg,
                &channel_handle,
                &router,
                adapter.as_ref(),
                &rate_limiter,
                &sanitizer,
                journal.as_ref(),
                &thread_ownership,
            )
            .await;
        }
    });
    Some(join_handle)
}

/// Owns all running channel adapters and dispatches messages to agents.
pub struct BridgeManager {
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    rate_limiter: ChannelRateLimiter,
    sanitizer: Arc<InputSanitizer>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    adapters: Vec<Arc<dyn ChannelAdapter>>,
    /// Webhook routes collected from adapters, to be mounted on the shared server.
    webhook_routes: Vec<(String, axum::Router)>,
    /// Optional message journal for crash recovery.
    journal: Option<crate::message_journal::MessageJournal>,
    /// Single-process thread-ownership claims. Suppresses multi-agent
    /// duplicate replies in shared group threads (#3334).
    thread_ownership: Arc<crate::thread_ownership::ThreadOwnershipRegistry>,
}

impl BridgeManager {
    pub fn new(handle: Arc<dyn ChannelBridgeHandle>, router: Arc<AgentRouter>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let sanitize_config = librefang_types::config::SanitizeConfig::default();
        Self {
            handle,
            router,
            rate_limiter: ChannelRateLimiter::default(),
            sanitizer: Arc::new(InputSanitizer::from_config(&sanitize_config)),
            shutdown_tx,
            shutdown_rx,
            tasks: Vec::new(),
            adapters: Vec::new(),
            webhook_routes: Vec::new(),
            journal: None,
            thread_ownership: Arc::new(crate::thread_ownership::ThreadOwnershipRegistry::new()),
        }
    }

    /// Create a `BridgeManager` with an explicit sanitize configuration.
    pub fn with_sanitizer(
        handle: Arc<dyn ChannelBridgeHandle>,
        router: Arc<AgentRouter>,
        sanitize_config: &librefang_types::config::SanitizeConfig,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            handle,
            router,
            rate_limiter: ChannelRateLimiter::default(),
            sanitizer: Arc::new(InputSanitizer::from_config(sanitize_config)),
            shutdown_tx,
            shutdown_rx,
            tasks: Vec::new(),
            adapters: Vec::new(),
            webhook_routes: Vec::new(),
            journal: None,
            thread_ownership: Arc::new(crate::thread_ownership::ThreadOwnershipRegistry::new()),
        }
    }

    /// Attach a message journal for crash recovery.
    pub fn with_journal(mut self, journal: crate::message_journal::MessageJournal) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Get a reference to the journal (if configured).
    pub fn journal(&self) -> Option<&crate::message_journal::MessageJournal> {
        self.journal.as_ref()
    }

    /// Recover messages that were in-flight when the daemon last crashed.
    /// Returns the messages that need re-processing.  The caller is
    /// responsible for re-dispatching them to agents.
    pub async fn recover_pending(&self) -> Vec<crate::message_journal::JournalEntry> {
        match &self.journal {
            Some(j) => {
                let entries = j.pending_entries().await;
                if !entries.is_empty() {
                    info!(
                        count = entries.len(),
                        "Recovering messages from journal that were interrupted by shutdown/crash"
                    );
                }
                entries
            }
            None => Vec::new(),
        }
    }

    /// Compact the journal and flush on shutdown.
    pub async fn compact_journal(&self) {
        if let Some(j) = &self.journal {
            j.compact().await;
        }
    }

    /// Start an adapter: subscribe to its message stream and spawn a dispatch task.
    ///
    /// Each incoming message is dispatched as a concurrent task so that slow LLM
    /// calls (10-30s) don't block subsequent messages. This prevents voice/media
    /// messages sent in quick succession from appearing "lost" — all messages
    /// begin processing immediately. Per-agent serialization (to prevent session
    /// corruption) is handled by the kernel's `agent_msg_locks`.
    ///
    /// A semaphore limits concurrent dispatch tasks to prevent unbounded memory
    /// growth under burst traffic.
    pub async fn start_adapter(
        &mut self,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Sweep stale files (>24h) from the download directory on startup.
        // Use Once so that registering multiple adapters doesn't trigger
        // redundant cleanup sweeps.
        {
            static CLEANUP_ONCE: std::sync::Once = std::sync::Once::new();
            let dir = self.handle.effective_channels_download_dir();
            CLEANUP_ONCE.call_once(|| {
                tokio::spawn(async move { cleanup_old_uploads(&dir).await });
            });
        }

        // 24h retention only fires when something accesses a bucket;
        // groups that go quiet without ever being addressed need an
        // active ticker to free memory. The evictor is owned by the
        // process-wide buffer (see `crate::group_history::install_global`),
        // not by any one BridgeManager — binding its lifetime to a single
        // bridge would orphan the buffer's TTL on hot-reload (the second
        // BridgeManager would skip the spawn, leaving the singleton
        // accumulating entries with no ticker).
        crate::group_history::install_global(|| {
            Arc::new(crate::group_history::GroupHistoryBuffer::with_default_retention())
        });

        // Prefer shared webhook routes over adapter-managed HTTP servers.
        // If the adapter provides webhook routes, collect them for mounting
        // on the main API server and use the returned stream for dispatch.
        let stream = if let Some((routes, stream)) = adapter.create_webhook_routes().await {
            let name = adapter.name().to_string();
            info!(
                "Channel {name} webhook endpoint: /channels/{name}/webhook \
                 (configure this URL on the external platform)"
            );
            self.webhook_routes.push((name, routes));
            stream
        } else {
            warn!(
                "Channel {} did not provide webhook routes, falling back to standalone mode",
                adapter.name()
            );
            adapter.start().await?
        };
        let handle = self.handle.clone();
        let router = self.router.clone();
        let rate_limiter = self.rate_limiter.clone();
        let sanitizer = self.sanitizer.clone();
        let adapter_clone = adapter.clone();
        let journal = self.journal.clone();
        let thread_ownership = Arc::clone(&self.thread_ownership);
        let mut shutdown = self.shutdown_rx.clone();

        let ct_str = channel_type_str(&adapter.channel_type()).to_string();
        let overrides = handle.channel_overrides(&ct_str, None).await;
        let debounce_ms = overrides
            .as_ref()
            .map(|o| o.message_debounce_ms)
            .unwrap_or(0);
        let debounce_max_ms = overrides
            .as_ref()
            .map(|o| o.message_debounce_max_ms)
            .unwrap_or(30000);
        let max_buffer = overrides
            .as_ref()
            .map(|o| o.message_debounce_max_buffer)
            .unwrap_or(64);

        let semaphore = Arc::new(tokio::sync::Semaphore::new(32));
        let upload_dir = handle.effective_channels_download_dir();

        if debounce_ms == 0 {
            // Fast path: no debouncing (current behavior)
            let task = tokio::spawn(async move {
                let mut stream = std::pin::pin!(stream);
                loop {
                    tokio::select! {
                        msg = stream.next() => {
                            match msg {
                                Some(message) => {
                                    let handle = handle.clone();
                                    let router = router.clone();
                                    let adapter = adapter_clone.clone();
                                    let rate_limiter = rate_limiter.clone();
                                    let sanitizer = sanitizer.clone();
                                    let journal = journal.clone();
                                    let sem = semaphore.clone();
                                    let thread_ownership = Arc::clone(&thread_ownership);
                                    tokio::spawn(async move {
                                        let _permit = match sem.acquire().await {
                                            Ok(p) => p,
                                            Err(_) => return,
                                        };
                                        dispatch_message(
                                            &message,
                                            &handle,
                                            &router,
                                            adapter.as_ref(),
                                            &rate_limiter,
                                            &sanitizer,
                                            journal.as_ref(),
                                            &thread_ownership,
                                        ).await;
                                    });
                                }
                                None => {
                                    info!("Channel adapter {} stream ended", adapter_clone.name());
                                    break;
                                }
                            }
                        }
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                info!("Shutting down channel adapter {}", adapter_clone.name());
                                break;
                            }
                        }
                    }
                }
            });
            self.tasks.push(task);
        } else {
            // Debounce path
            let (debouncer, mut flush_rx) =
                MessageDebouncer::new(debounce_ms, debounce_max_ms, max_buffer);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let mut typing_rx = adapter_clone.typing_events();

            let task = tokio::spawn(async move {
                let mut stream = std::pin::pin!(stream);
                loop {
                    tokio::select! {
                        msg = stream.next() => {
                            match msg {
                                Some(message) => {
                                    let sender_key = format!(
                                        "{}:{}",
                                        channel_type_str(&message.channel),
                                        message.sender.platform_id
                                    );

                                    let image_blocks = if let ChannelContent::Image {
                                        ref url, ref caption, ref mime_type
                                    } = message.content {
                                        let extra_headers = adapter_clone.fetch_headers_for(url);
                                        match download_image_to_blocks(url, caption.as_deref(), mime_type.as_deref(), &upload_dir, &extra_headers).await {
                                            blocks if blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. } | ContentBlock::ImageFile { .. })) => Some(blocks),
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    };

                                    let pending = PendingMessage { message, image_blocks };
                                    debouncer.push(&sender_key, pending, &mut buffers);
                                }
                                None => {
                                    let keys: Vec<String> = buffers.keys().cloned().collect();
                                    let mut handles = Vec::new();
                                    for key in keys {
                                        if let Some(handle) = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal, &thread_ownership) {
                                            handles.push(handle);
                                        }
                                    }
                                    for handle in handles {
                                        let _ = handle.await;
                                    }
                                    info!("Channel adapter {} stream ended", adapter_clone.name());
                                    break;
                                }
                            }
                        }
                        Some(event) = async {
                            match typing_rx.as_mut() {
                                Some(rx) => rx.recv().await,
                                None => std::future::pending::<Option<crate::types::TypingEvent>>().await,
                            }
                        } => {
                            let sender_key = format!("{}:{}", channel_type_str(&event.channel), event.sender.platform_id);
                            debouncer.on_typing(&sender_key, event.is_typing, &mut buffers);
                        }
                        Some(key) = flush_rx.recv() => {
                            let _ = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal, &thread_ownership);
                        }
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                let keys: Vec<String> = buffers.keys().cloned().collect();
                                let mut handles = Vec::new();
                                for key in keys {
                                    if let Some(handle) = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal, &thread_ownership) {
                                        handles.push(handle);
                                    }
                                }
                                for handle in handles {
                                    let _ = handle.await;
                                }
                                info!("Shutting down channel adapter {}", adapter_clone.name());
                                break;
                            }
                        }
                    }
                }
            });
            self.tasks.push(task);
        }

        self.adapters.push(adapter);
        Ok(())
    }

    /// Start listening for `ApprovalRequested` kernel events and forward them
    /// to every running channel adapter as a text notification (#4875).
    ///
    /// Per-adapter recipients come from
    /// [`ChannelAdapter::notification_recipients`]. Adapters that return an
    /// empty list (the default) silently skip the broadcast — that is the
    /// correct behaviour for group-only / public-broadcast adapters that
    /// have no stable operator inbox. The current payload is plain text
    /// with the truncated approval ID and `/approve <id>` / `/reject <id>`
    /// instructions; inline-keyboard support per adapter is a follow-on
    /// (re-opens the delivery side of #2029).
    pub async fn start_approval_listener(&mut self) {
        let maybe_rx = self.handle.subscribe_events().await;
        let Some(mut rx) = maybe_rx else {
            debug!("Event subscription not available — approval listener not started");
            return;
        };

        let mut shutdown = self.shutdown_rx.clone();
        let handle = self.handle.clone();
        let adapters = self.adapters.clone();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Bias toward shutdown so a stop() call wins deterministically
                    // over an in-flight ApprovalRequested poll. Without this the
                    // unbiased select can pick the broadcast arm on the same poll
                    // that shutdown_tx fires, then call adapter.send() on an
                    // adapter that stop() has already drained — benign (warn! +
                    // continue) but spurious in shutdown logs.
                    biased;
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            info!("Shutting down approval event listener");
                            break;
                        }
                    }
                    result = rx.recv() => {
                        match result {
                            Ok(event) => {
                                if let librefang_types::event::EventPayload::ApprovalRequested(approval) = &event.payload {
                                    let short_id = &approval.request_id[..8.min(approval.request_id.len())];
                                    let msg = format!(
                                        "Approval required for agent {}\n\
                                         Tool: {}\n\
                                         Risk: {}\n\
                                         {}\n\n\
                                         Reply /approve {short_id} or /reject {short_id}",
                                        approval.agent_id,
                                        approval.tool_name,
                                        approval.risk_level,
                                        approval.description,
                                    );

                                    for adapter in &adapters {
                                        let recipients = adapter.notification_recipients();
                                        if recipients.is_empty() {
                                            debug!(
                                                adapter = adapter.name(),
                                                request_id = %approval.request_id,
                                                "Adapter has no notification recipients — skipping approval broadcast"
                                            );
                                            continue;
                                        }
                                        for user in &recipients {
                                            if let Err(e) = adapter
                                                .send(user, ChannelContent::Text(msg.clone()))
                                                .await
                                            {
                                                warn!(
                                                    adapter = adapter.name(),
                                                    request_id = %approval.request_id,
                                                    recipient = %user.platform_id,
                                                    error = %e,
                                                    "Failed to deliver approval notification"
                                                );
                                            } else {
                                                info!(
                                                    adapter = adapter.name(),
                                                    request_id = %approval.request_id,
                                                    recipient = %user.platform_id,
                                                    "Delivered approval notification"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                // Route through the kernel's lag counter so
                                // approval-event misses contribute to
                                // EventBus::dropped_count and surface as a
                                // rate-limited error! log (#3630). Default
                                // impl is a no-op for test mocks without an
                                // event bus.
                                handle.record_consumer_lag(n, "channel_approval_listener");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                info!("Event bus closed — stopping approval listener");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.tasks.push(task);
    }

    /// Push a proactive outbound message to a channel recipient.
    ///
    /// Routes the message through the kernel's `send_channel_message` which
    /// looks up the adapter by name and delivers via `ChannelAdapter::send()`.
    /// This is the bridge-level entry point used by the REST API push endpoint.
    pub async fn push_message(
        &self,
        channel_type: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        if channel_type.is_empty() {
            return Err("channel_type cannot be empty".to_string());
        }
        if recipient.is_empty() {
            return Err("recipient cannot be empty".to_string());
        }
        if message.is_empty() {
            return Err("message cannot be empty".to_string());
        }

        info!(
            channel = channel_type,
            recipient = recipient,
            "Pushing outbound message via bridge"
        );

        // Delegate to the kernel handle which owns the adapter registry
        self.handle
            .send_channel_push(channel_type, recipient, message, thread_id)
            .await
    }

    /// Stop all adapters and wait for dispatch tasks to finish.
    /// Take the collected webhook routes and merge them into a single Router.
    ///
    /// Each adapter's routes are nested under `/{adapter_name}`. The caller
    /// should mount the returned router under `/channels` on the main API
    /// server, without auth middleware (webhook adapters handle their own
    /// signature verification).
    pub fn take_webhook_router(&mut self) -> axum::Router {
        let mut router = axum::Router::new();
        for (name, routes) in self.webhook_routes.drain(..) {
            router = router.nest(&format!("/{name}"), routes);
        }
        router
    }

    /// Register a background task with the bridge so its lifetime is
    /// tied to the bridge's. `stop()` awaits every tracked handle.
    /// External spawners (e.g. the journal retry ticker in
    /// `librefang-api`) MUST register here or they leak across
    /// hot-reloads — old and new instances would race on the same
    /// journal entries and double-dispatch.
    pub fn track_task(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.tasks.push(handle);
    }

    /// Subscriber to the bridge's shutdown signal. Background tasks
    /// can `select!` on this to exit cleanly when `stop()` fires.
    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown_rx.clone()
    }

    pub async fn stop(&mut self) {
        // Signal the dispatch loops to stop. A send error here only means
        // every receiver was already dropped, which is fine on a duplicate
        // shutdown call but worth surfacing for diagnostics.
        if let Err(e) = self.shutdown_tx.send(true) {
            debug!(error = %e, "Channel bridge shutdown signal had no live receivers");
        }

        // Stop each adapter's internal tasks (WebSocket connections, callback
        // servers, etc.) so they release ports and connections before we
        // potentially restart them during hot-reload.
        for adapter in self.adapters.drain(..) {
            if let Err(e) = adapter.stop().await {
                warn!(adapter = adapter.name(), error = %e, "Error stopping channel adapter");
            }
        }

        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}

/// Resolve channel type to its config string key.
fn channel_type_str(channel: &crate::types::ChannelType) -> &str {
    match channel {
        crate::types::ChannelType::Telegram => "telegram",
        crate::types::ChannelType::Discord => "discord",
        crate::types::ChannelType::Slack => "slack",
        crate::types::ChannelType::WhatsApp => "whatsapp",
        crate::types::ChannelType::Signal => "signal",
        crate::types::ChannelType::Matrix => "matrix",
        crate::types::ChannelType::Email => "email",
        crate::types::ChannelType::Teams => "teams",
        crate::types::ChannelType::Mattermost => "mattermost",
        crate::types::ChannelType::WeChat => "wechat",
        crate::types::ChannelType::WebChat => "webchat",
        crate::types::ChannelType::CLI => "cli",
        crate::types::ChannelType::Custom(s) => s.as_str(),
    }
}

/// Metadata key for the actual sender user ID (distinct from platform_id in DMs).
pub const SENDER_USER_ID_KEY: &str = "sender_user_id";

#[derive(Debug)]
struct CompiledGroupTriggerPatterns {
    regex_set: Option<RegexSet>,
}

static GROUP_TRIGGER_PATTERN_CACHE: OnceLock<
    dashmap::DashMap<String, Arc<CompiledGroupTriggerPatterns>>,
> = OnceLock::new();

fn group_trigger_pattern_cache(
) -> &'static dashmap::DashMap<String, Arc<CompiledGroupTriggerPatterns>> {
    GROUP_TRIGGER_PATTERN_CACHE.get_or_init(dashmap::DashMap::new)
}

fn compile_group_trigger_patterns(patterns: &[String]) -> Arc<CompiledGroupTriggerPatterns> {
    let cache_key = patterns.join("\u{1f}");
    if let Some(existing) = group_trigger_pattern_cache().get(&cache_key) {
        return existing.clone();
    }

    let mut valid_patterns = Vec::new();
    for pattern in patterns {
        match regex::Regex::new(pattern) {
            Ok(_) => valid_patterns.push(pattern.clone()),
            Err(err) => {
                error!(pattern = %pattern, error = %err, "Invalid group trigger regex pattern");
            }
        }
    }

    let compiled = Arc::new(CompiledGroupTriggerPatterns {
        regex_set: if valid_patterns.is_empty() {
            None
        } else {
            match RegexSet::new(&valid_patterns) {
                Ok(regex_set) => Some(regex_set),
                Err(err) => {
                    error!(error = %err, "Failed to compile group trigger regex set");
                    None
                }
            }
        },
    });

    group_trigger_pattern_cache().insert(cache_key, compiled.clone());
    compiled
}

fn text_content(message: &ChannelMessage) -> Option<&str> {
    match &message.content {
        ChannelContent::Text(text) => Some(text.as_str()),
        _ => None,
    }
}

fn matches_group_trigger_pattern(
    ct_str: &str,
    message: &ChannelMessage,
    patterns: &[String],
) -> bool {
    let Some(text) = text_content(message) else {
        return false;
    };
    let compiled = compile_group_trigger_patterns(patterns);
    let Some(regex_set) = compiled.regex_set.as_ref() else {
        return false;
    };
    let matched = regex_set.is_match(text);
    if matched {
        debug!(
            channel = ct_str,
            user = %message.sender.display_name,
            "Group message matched regex trigger pattern"
        );
    }
    matched
}

// ---------------------------------------------------------------------------
// Phase 2 §C — Positional vocative trigger + addressee guard (OB-04, OB-05)
// ---------------------------------------------------------------------------

/// Truncate `text` to `max` chars (UTF-8 safe) for log excerpts.
fn truncate_excerpt(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Returns true when `LIBREFANG_GROUP_ADDRESSEE_GUARD=on`.
///
/// Per D-§C-6 the guard is shipped default-off for a 1-week observation
/// window. While off, the legacy substring matcher remains authoritative
/// and the new positional/addressee functions are bypassed in
/// `should_process_group_message`.
fn addressee_guard_enabled() -> bool {
    std::env::var("LIBREFANG_GROUP_ADDRESSEE_GUARD")
        .ok()
        .as_deref()
        == Some("on")
}

/// Detect a leading-vocative `<Capitalized>[,!]` token in `text`.
///
/// Returns the captured name (without the punctuation) when the turn opens
/// with a vocative form like "Caterina,". The match is anchored at the start
/// of the string after optional whitespace; only ASCII-style capitalized
/// names are recognized (Italian/English vocatives — sufficient for §C).
fn leading_vocative_name(text: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // ^\s* <Capitalized name (1+ letters)> followed by , or !
        Regex::new(r"^\s*([A-ZÀ-Ý][A-Za-zÀ-ÿ]+)[,!]").expect("leading_vocative regex compiles")
    });
    re.captures(text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Strict positional vocative-trigger match for `pattern` in `text`.
///
/// True iff the (whole-word, case-sensitive — pattern is expected to be a
/// proper name like "Signore") `pattern` appears either:
///  * at the start of the turn after optional whitespace, or
///  * immediately after a `[.!?]` punctuation boundary followed by whitespace.
///
/// Additionally REJECTED when another capitalized vocative appears BEFORE
/// the matched pattern — this captures the Beeper-screenshot case
/// `"Caterina, chiedi al Signore..."` where "Signore" is mentioned but the
/// turn is addressed to Caterina.
fn is_vocative_trigger(text: &str, pattern: &str) -> bool {
    if text.is_empty() || pattern.is_empty() {
        return false;
    }
    // Build a per-call regex (patterns vary per-agent and tests cover several).
    // Pattern is a literal proper name; escape to avoid regex-meta surprises.
    let escaped = regex::escape(pattern);
    let combined = format!(r"(?:^|[.!?])\s*({escaped})\b", escaped = escaped);
    let re = match Regex::new(&combined) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let Some(m) = re.find(text) else { return false };

    // Heuristic: reject if any *other* capitalized vocative (`<Name>,`) appears
    // BEFORE the pattern position. We scan only the prefix [0..match_start].
    let prefix = &text[..m.start()];
    static OTHER_VOCATIVE: OnceLock<Regex> = OnceLock::new();
    let other = OTHER_VOCATIVE.get_or_init(|| {
        Regex::new(r"\b([A-ZÀ-Ý][A-Za-zÀ-ÿ]+),\s").expect("other_vocative regex compiles")
    });
    for cap in other.captures_iter(prefix) {
        if let Some(name) = cap.get(1) {
            // If the prefix vocative IS the pattern itself we'd have matched at
            // start; getting here means it's a *different* name → reject.
            if !name.as_str().eq_ignore_ascii_case(pattern) {
                return false;
            }
        }
    }
    true
}

/// True when the turn opens with a vocative addressed to a participant other
/// than the agent (e.g. `"Caterina, chiedi..."` in a group containing
/// Caterina + the Bot).
///
/// Heuristic: extract a leading `<Capitalized>[,!]` token and look it up
/// (case-insensitively) in the participant roster. If found and not equal
/// to `agent_name`, the turn is addressed to someone else.
fn is_addressed_to_other_participant(
    text: &str,
    participants: &[ParticipantRef],
    agent_name: &str,
) -> bool {
    let Some(name) = leading_vocative_name(text) else {
        return false;
    };
    if name.eq_ignore_ascii_case(agent_name) {
        return false;
    }
    participants.iter().any(|p| {
        p.display_name.eq_ignore_ascii_case(&name)
            && !p.display_name.eq_ignore_ascii_case(agent_name)
    })
}

fn is_group_command(message: &ChannelMessage) -> bool {
    matches!(&message.content, ChannelContent::Command { .. })
        || matches!(&message.content, ChannelContent::Text(text) if text.starts_with('/'))
}

/// Check whether a built-in slash command is permitted on this channel.
///
/// Precedence: `disable_commands` > `allowed_commands` (whitelist) >
/// `blocked_commands` (blacklist). When no overrides are configured,
/// everything is allowed (current default behaviour).
///
/// Config entries may be written with or without a leading `/` (both
/// `"agent"` and `"/agent"` match the dispatcher's bare `"agent"` token).
fn is_command_allowed(cmd: &str, overrides: Option<&ChannelOverrides>) -> bool {
    let Some(ov) = overrides else { return true };
    if ov.disable_commands {
        return false;
    }
    // Normalize config entries: strip a single optional leading slash so users
    // can write either "agent" or "/agent" in TOML.
    let matches = |entry: &String| -> bool {
        let name = entry.strip_prefix('/').unwrap_or(entry);
        name == cmd
    };
    if !ov.allowed_commands.is_empty() {
        return ov.allowed_commands.iter().any(matches);
    }
    !ov.blocked_commands.iter().any(matches)
}

/// Reconstruct the raw slash-command text so that blocked commands can be
/// forwarded to the agent as normal user input (e.g. `/agent admin` →
/// `"/agent admin"`). Keeps the slash so the agent can see what the user
/// originally typed.
fn reconstruct_command_text(name: &str, args: &[String]) -> String {
    if args.is_empty() {
        format!("/{name}")
    } else {
        format!("/{} {}", name, args.join(" "))
    }
}

fn should_process_group_message(
    ct_str: &str,
    overrides: &ChannelOverrides,
    message: &ChannelMessage,
) -> bool {
    match overrides.group_policy {
        GroupPolicy::Ignore => {
            debug!("Ignoring group message on {ct_str} (group_policy=ignore)");
            false
        }
        GroupPolicy::CommandsOnly => {
            if !is_group_command(message) {
                debug!(
                    "Ignoring non-command group message on {ct_str} (group_policy=commands_only)"
                );
                return false;
            }
            true
        }
        GroupPolicy::MentionOnly => {
            let was_mentioned = message
                .metadata
                .get("was_mentioned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let is_command = is_group_command(message);
            let text = text_content(message).unwrap_or("");
            let sender_excerpt: &str = &message.sender.display_name;
            let guard_on = addressee_guard_enabled();

            // OB-04/OB-05 — addressee guard. When the turn opens with a vocative
            // matching another participant in the group roster, abstain even if
            // a substring of `group_trigger_patterns` matches mid-turn.
            // (No owner short-circuit here: per OB-06 audit no `is_owner` branch
            // exists in librefang-channels — owner is treated as any participant.)
            if guard_on {
                let participants = extract_group_participants(message);
                let agent_name = extract_agent_name(message);
                if is_addressed_to_other_participant(text, &participants, &agent_name) {
                    info!(
                        event = "group_gating_skip",
                        reason = "addressed_to_other_participant",
                        channel = ct_str,
                        sender = %sender_excerpt,
                        text_excerpt = %truncate_excerpt(text, 80),
                        "OB-04: vocative addressed to other participant"
                    );
                    return false;
                }
            }

            // Trigger-pattern check. Under guard-on we additionally require
            // `is_vocative_trigger` (positional) on top of the substring match,
            // so "Caterina, chiedi al Signore..." with pattern "Signore" no
            // longer triggers (the substring matches but the position is wrong
            // AND another vocative precedes it).
            let regex_triggered = if !was_mentioned && !is_command {
                let mut hit = matches_group_trigger_pattern(
                    ct_str,
                    message,
                    &overrides.group_trigger_patterns,
                );
                if hit && guard_on {
                    let positional_ok = overrides
                        .group_trigger_patterns
                        .iter()
                        .any(|p| is_vocative_trigger(text, p));
                    if !positional_ok {
                        info!(
                            event = "group_gating_skip",
                            reason = "vocative_position_mismatch",
                            channel = ct_str,
                            sender = %sender_excerpt,
                            text_excerpt = %truncate_excerpt(text, 80),
                            "OB-05: substring matched but not at vocative position"
                        );
                        hit = false;
                    }
                }
                hit
            } else {
                false
            };

            if !was_mentioned && !is_command && !regex_triggered {
                info!(
                    event = "group_gating_skip",
                    reason = "mention_only_no_mention",
                    channel = ct_str,
                    sender = %sender_excerpt,
                    text_excerpt = %truncate_excerpt(text, 80),
                    "OB-06: mention_only and bot was not mentioned"
                );
                return false;
            }
            info!(
                event = "group_gating_pass",
                channel = ct_str,
                sender = %sender_excerpt,
                was_mentioned,
                is_command,
                regex_triggered,
                "Group message accepted for processing"
            );
            true
        }
        GroupPolicy::All => true,
    }
}

/// Extract structured `GroupMember` entries from the inbound message metadata.
/// Channels that supply `group_members` (a JSON array of `{user_id, display_name, username?}`)
/// populate this; the bridge persists them to the roster store for later queries.
fn extract_group_members(message: &ChannelMessage) -> Vec<GroupMember> {
    message
        .metadata
        .get("group_members")
        .and_then(|v| serde_json::from_value::<Vec<GroupMember>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Read `group_participants` from the inbound message metadata payload
/// (populated gateway-side by `sock.groupMetadata`). Returns empty when the
/// channel doesn't supply a roster — the addressee guard then becomes a no-op
/// (cannot fire false positives).
fn extract_group_participants(message: &ChannelMessage) -> Vec<ParticipantRef> {
    message
        .metadata
        .get("group_participants")
        .and_then(|v| serde_json::from_value::<Vec<ParticipantRef>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Read the canonical agent display name from message metadata when the
/// caller provides it (gateway/runtime injects so the addressee guard knows
/// "this name == us"). Empty string when absent — `eq_ignore_ascii_case("")`
/// then never matches a real participant name, so the guard simply checks
/// whether the leading vocative belongs to another roster member.
fn extract_agent_name(message: &ChannelMessage) -> String {
    message
        .metadata
        .get("agent_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Build a `SenderContext` from an incoming `ChannelMessage`.
///
/// Per-channel auto-routing fields are populated from `overrides` when provided,
/// and default to `AutoRouteStrategy::Off` / zeros otherwise.
fn build_sender_context(
    message: &ChannelMessage,
    overrides: Option<&ChannelOverrides>,
) -> SenderContext {
    let (
        auto_route,
        auto_route_ttl_minutes,
        auto_route_confidence_threshold,
        auto_route_sticky_bonus,
        auto_route_divergence_count,
    ) = match overrides {
        Some(ov) => (
            ov.auto_route.clone(),
            ov.auto_route_ttl_minutes,
            ov.auto_route_confidence_threshold,
            ov.auto_route_sticky_bonus,
            ov.auto_route_divergence_count,
        ),
        None => (AutoRouteStrategy::Off, 0, 0, 0, 0),
    };
    let chat_id = if message.sender.platform_id.is_empty() {
        None
    } else {
        Some(message.sender.platform_id.clone())
    };
    SenderContext {
        channel: channel_type_str(&message.channel).to_string(),
        user_id: sender_user_id(message).to_string(),
        chat_id,
        display_name: message.sender.display_name.clone(),
        is_group: message.is_group,
        was_mentioned: message
            .metadata
            .get("was_mentioned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        thread_id: message.thread_id.clone(),
        account_id: message
            .metadata
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        auto_route,
        auto_route_ttl_minutes,
        auto_route_confidence_threshold,
        auto_route_sticky_bonus,
        auto_route_divergence_count,
        // §C: forward roster from inbound payload (gateway populates via
        // sock.groupMetadata). Empty for non-WhatsApp channels — addressee
        // guard then becomes a no-op (BC-01).
        group_participants: extract_group_participants(message),
        // Bot identity metadata for group context enrichment.
        bot_username: message
            .metadata
            .get("bot_username")
            .and_then(|v| v.as_str())
            .map(String::from),
        sender_username: message
            .metadata
            .get("sender_username")
            .and_then(|v| v.as_str())
            .map(String::from),
        group_members: extract_group_members(message),
        // Channel bridges land in per-channel sessions (the default); only
        // the dashboard WS opts into canonical storage.
        use_canonical_session: false,
        // Channel-originated traffic is never internal cron — [SILENT] markers
        // coming from real users must be treated as literal message content.
        is_internal_cron: false,
    }
}

/// Extract the sender identity used for RBAC and per-user rate limiting.
fn sender_user_id(message: &ChannelMessage) -> &str {
    message
        .metadata
        .get(SENDER_USER_ID_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or(&message.sender.platform_id)
}

/// Persists the observed group sender; skips DMs and messages without SENDER_USER_ID_KEY to avoid storing the group's own platform_id.
async fn upsert_sender_into_roster(
    handle: &Arc<dyn ChannelBridgeHandle>,
    message: &ChannelMessage,
) {
    if !message.is_group {
        return;
    }
    let Some(user_id) = message
        .metadata
        .get(SENDER_USER_ID_KEY)
        .and_then(|v| v.as_str())
    else {
        return;
    };
    if user_id.is_empty() || message.sender.platform_id.is_empty() {
        return;
    }
    let username = message
        .metadata
        .get("sender_username")
        .and_then(|v| v.as_str());
    let channel_str = channel_type_str(&message.channel);
    if let Err(e) = handle
        .roster_upsert(
            channel_str,
            &message.sender.platform_id,
            user_id,
            &message.sender.display_name,
            username,
        )
        .await
    {
        warn!(
            channel = channel_str,
            chat_id = %message.sender.platform_id,
            user_id = %user_id,
            error = %e,
            "roster_upsert failed; group member will not be remembered for this turn"
        );
    }
}

/// Wrap an outbound message with the responding agent's name according to
/// `style`.
///
/// Applied once at the top of the final response text (never per streaming
/// chunk). If the text already starts with the exact bracketed agent label
/// (e.g. the agent echoed its own name, or an inner agent already prefixed a
/// delegated reply), the wrap is skipped to keep things idempotent.
///
/// # Idempotency caveats
///
/// The "starts-with" check uses the literal `[name]` / `**[name]**` string. If
/// `agent_name` itself contains `[`, `]`, or `*` characters, the detection is
/// best-effort:
///
/// - The function never panics or corrupts UTF-8 — output stays well-formed.
/// - For pathological names like `"a]b"`, repeated invocations may produce
///   nested prefixes like `"[a]b] [a]b] text"` because the outer `[a]b]`
///   isn't recognized as already-prefixed by a naive `starts_with`.
///
/// Worst-case degradation is therefore "extra prefix" rather than data loss
/// or crash. Agents authoring outbound replies should pick names without
/// bracket / asterisk characters; the dashboard's agent rename UI does not
/// enforce this today.
///
/// Per-platform native identity features (Slack `username` override, Discord
/// embed `author`, Telegram `From:` in rich messages) are intentionally not
/// handled here.
pub(crate) fn apply_agent_prefix(style: PrefixStyle, agent_name: &str, text: &str) -> String {
    if matches!(style, PrefixStyle::Off) || agent_name.is_empty() {
        return text.to_string();
    }
    let bracket = format!("[{agent_name}]");
    let bold = format!("**[{agent_name}]**");
    if text.starts_with(&bracket) || text.starts_with(&bold) {
        return text.to_string();
    }
    match style {
        PrefixStyle::Off => text.to_string(),
        PrefixStyle::Bracket => format!("{bracket} {text}"),
        PrefixStyle::BoldBracket => format!("{bold} {text}"),
    }
}

/// Look up an agent's display name by id.
///
/// Returns `None` if the kernel can't list agents or the id is not currently
/// known. Only called when `prefix_agent_name` is enabled, so the extra
/// `list_agents()` round-trip is pay-per-use.
async fn resolve_agent_name(handle: &Arc<dyn ChannelBridgeHandle>, id: AgentId) -> Option<String> {
    handle
        .list_agents()
        .await
        .ok()?
        .into_iter()
        .find_map(|(aid, name)| (aid == id).then_some(name))
}

/// Apply `prefix_agent_name` to an outbound agent response if configured.
///
/// Safe to call on every success path: resolves the agent name lazily and
/// returns the original text unchanged when the style is `Off`.
async fn maybe_prefix_response(
    handle: &Arc<dyn ChannelBridgeHandle>,
    overrides: Option<&ChannelOverrides>,
    agent_id: AgentId,
    text: String,
) -> String {
    let style = overrides
        .map(|o| o.prefix_agent_name)
        .unwrap_or(PrefixStyle::Off);
    if matches!(style, PrefixStyle::Off) {
        return text;
    }
    match resolve_agent_name(handle, agent_id).await {
        Some(name) => apply_agent_prefix(style, &name, &text),
        None => text,
    }
}

/// Resolve the leading prefix chunk (e.g. `"[coder] "`) for streaming output,
/// or `None` if prefixing is disabled / agent name unknown.
///
/// Used by the streaming success path to inject the prefix as the first
/// delta — `apply_agent_prefix` only handles the non-streaming "wrap full
/// text" case.
async fn resolve_prefix_chunk(
    handle: &Arc<dyn ChannelBridgeHandle>,
    overrides: Option<&ChannelOverrides>,
    agent_id: AgentId,
) -> Option<String> {
    let style = overrides.map(|o| o.prefix_agent_name)?;
    if matches!(style, PrefixStyle::Off) {
        return None;
    }
    let name = resolve_agent_name(handle, agent_id).await?;
    if name.is_empty() {
        return None;
    }
    match style {
        PrefixStyle::Off => None,
        PrefixStyle::Bracket => Some(format!("[{name}] ")),
        PrefixStyle::BoldBracket => Some(format!("**[{name}]** ")),
    }
}

/// Send a response, applying output formatting and optional threading.
async fn send_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) {
    tracing::debug!(
        adapter = adapter.name(),
        user = %user.platform_id,
        text_len = text.len(),
        "Sending response to channel"
    );
    let formatted = formatter::format_for_channel(&text, output_format);
    let content = ChannelContent::Text(formatted);

    let result = if let Some(tid) = thread_id {
        adapter.send_in_thread(user, content, tid).await
    } else {
        adapter.send(user, content).await
    };

    if let Err(e) = result {
        error!("Failed to send response: {e}");
    }
}

fn default_output_format_for_channel(channel_type: &str) -> OutputFormat {
    formatter::default_output_format_for_channel(channel_type)
}

/// Extract the tool name from a `\n\n🔧 toolname\n\n` progress marker
/// emitted by `librefang_api::channel_bridge` in response to a kernel
/// `StreamEvent::ToolUseStart` event. Returns `None` for plain text
/// deltas, the trailing-`⚠️`-error marker, the context-warning marker,
/// or anything that doesn't exactly match the prefix+suffix wrapping —
/// the api channel bridge sends each marker as its own dedicated
/// `tx.send(line)` so an exact-match strip is the right shape (we
/// would NOT want to grab a `🔧` that appeared inside model prose).
fn extract_tool_marker_name(delta: &str) -> Option<String> {
    let prefix = "\n\n🔧 ";
    let suffix = "\n\n";
    let inner = delta.strip_prefix(prefix)?.strip_suffix(suffix)?;
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Send a lifecycle reaction (best-effort, non-blocking for supported adapters).
///
/// Errors are logged at WARN — reactions are best-effort UX polish, but a
/// silent failure mode masks real problems. The original `debug!` here hid
/// per-room rate-limit drops on Matrix (`M_LIMIT_EXCEEDED`) where the
/// trailing `✅ Done` reaction was being silently swallowed at default
/// verbosity, and made the lifecycle-reaction feature look broken even
/// when it was working. WARN is the right level: a single failure tells
/// an operator "your homeserver is rate-limiting the bot", which is
/// exactly the actionable diagnosis we want surfaced.
/// For Telegram, the underlying HTTP call is already fire-and-forget
/// (spawned internally), so this await returns almost immediately.
async fn send_lifecycle_reaction(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    message_id: &str,
    phase: &AgentPhase,
) {
    let reaction = LifecycleReaction {
        emoji: default_phase_emoji(phase).to_string(),
        phase: phase.clone(),
        remove_previous: true,
    };
    if let Err(e) = adapter.send_reaction(user, message_id, &reaction).await {
        warn!(
            adapter = adapter.name(),
            message_id = message_id,
            phase = ?phase,
            error = %e,
            "Lifecycle reaction send failed (best-effort, not retried)",
        );
    }
}

/// On stale cached agent IDs, re-resolve the channel default by name and retry once.
async fn try_reresolution(
    error: &str,
    failed_agent_id: AgentId,
    channel_key: &str,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
) -> Option<AgentId> {
    if !error.contains("Agent not found") {
        return None;
    }

    if router.channel_default(channel_key) != Some(failed_agent_id) {
        return None;
    }

    let agent_name = router.channel_default_name(channel_key)?;
    info!(
        channel = channel_key,
        agent_name = %agent_name,
        "Channel default agent ID is stale; re-resolving by name"
    );

    match handle.find_agent_by_name(&agent_name).await {
        Ok(Some(agent_id)) => {
            router.update_channel_default(channel_key, agent_id);
            Some(agent_id)
        }
        Ok(None) => {
            warn!(
                channel = channel_key,
                agent_name = %agent_name,
                "Could not re-resolve default agent by name"
            );
            None
        }
        Err(e) => {
            warn!(channel = channel_key, error = %e, "Failed to re-resolve default agent");
            None
        }
    }
}

/// Handle a failed agent send: attempt re-resolution for stale agent IDs, otherwise
/// report the error to the user.
///
/// This covers the full error path — the caller can simply return after calling this.
#[allow(clippy::too_many_arguments)]
async fn handle_send_error<F, Fut>(
    error: &str,
    agent_id: AgentId,
    channel_key: &str,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    sender: &ChannelUser,
    msg_id: &str,
    ct_str: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
    overrides: Option<&ChannelOverrides>,
    send_fn: F,
) where
    F: FnOnce(AgentId) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    // Try re-resolution for stale agent IDs
    if let Some(new_id) = try_reresolution(error, agent_id, channel_key, handle, router).await {
        send_lifecycle_reaction(adapter, sender, msg_id, &AgentPhase::Thinking).await;

        match send_fn(new_id).await {
            Ok(response) => {
                send_lifecycle_reaction(adapter, sender, msg_id, &AgentPhase::Done).await;
                if !response.is_empty() {
                    let response = maybe_prefix_response(handle, overrides, new_id, response).await;
                    send_response(adapter, sender, response, thread_id, output_format).await;
                }
                handle
                    .record_delivery(new_id, ct_str, &sender.platform_id, true, None, thread_id)
                    .await;
                return;
            }
            Err(e2) => {
                // Re-resolution succeeded but the retry failed — report retry error
                send_lifecycle_reaction(adapter, sender, msg_id, &AgentPhase::Error).await;
                warn!("Agent error for {new_id} (after re-resolution): {e2}");
                let err_msg = format!("Agent error: {e2}");
                if !adapter.suppress_error_responses() {
                    send_response(adapter, sender, err_msg.clone(), thread_id, output_format).await;
                }
                handle
                    .record_delivery(
                        new_id,
                        ct_str,
                        &sender.platform_id,
                        false,
                        Some(&err_msg),
                        thread_id,
                    )
                    .await;
                return;
            }
        }
    }

    // Not a stale-agent error (or re-resolution not applicable) — report original error
    send_lifecycle_reaction(adapter, sender, msg_id, &AgentPhase::Error).await;
    warn!("Agent error for {agent_id}: {error}");
    let err_msg = format!("Agent error: {error}");
    if !adapter.suppress_error_responses() {
        send_response(adapter, sender, err_msg.clone(), thread_id, output_format).await;
    }
    handle
        .record_delivery(
            agent_id,
            ct_str,
            &sender.platform_id,
            false,
            Some(&err_msg),
            thread_id,
        )
        .await;
}

/// Resolve the target agent for an incoming message using thread routing, binding
/// context, and fallback logic. Returns `Some(agent_id)` or `None` if no agents exist.
///
/// Shared by `dispatch_message` and `dispatch_with_blocks` to ensure consistent routing.
async fn resolve_or_fallback(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
) -> Option<AgentId> {
    // Thread-based agent routing: if the adapter tagged this message with a
    // thread_route_agent, resolve that agent name first.
    let thread_route_agent_id = if let Some(agent_name) = message
        .metadata
        .get("thread_route_agent")
        .and_then(|v| v.as_str())
    {
        match handle.find_agent_by_name(agent_name).await {
            Ok(Some(id)) => Some(id),
            Ok(None) => {
                warn!(
                    "Thread route agent '{agent_name}' not found, falling back to default routing"
                );
                None
            }
            Err(e) => {
                warn!("Thread route agent lookup failed for '{agent_name}': {e}");
                None
            }
        }
    } else {
        None
    };

    // Route to agent — use resolve_with_context to support account_id, guild_id, etc.
    let agent_id = if let Some(id) = thread_route_agent_id {
        Some(id)
    } else {
        let ctx = crate::router::BindingContext {
            channel: std::borrow::Cow::Borrowed(crate::router::channel_type_to_str(
                &message.channel,
            )),
            account_id: message
                .metadata
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(std::borrow::Cow::Borrowed),
            peer_id: std::borrow::Cow::Borrowed(&message.sender.platform_id),
            guild_id: message
                .metadata
                .get("guild_id")
                .and_then(|v| v.as_str())
                .map(std::borrow::Cow::Borrowed),
            roles: smallvec::SmallVec::new(),
        };
        router.resolve_with_context(
            &message.channel,
            &message.sender.platform_id,
            message.sender.librefang_user.as_deref(),
            &ctx,
        )
    };

    if let Some(id) = agent_id {
        return Some(id);
    }

    // Fallback: try "assistant" agent, then first available agent
    let fallback = handle.find_agent_by_name("assistant").await.ok().flatten();
    let fallback = match fallback {
        Some(id) => Some(id),
        None => handle
            .list_agents()
            .await
            .ok()
            .and_then(|agents| agents.first().map(|(id, _)| *id)),
    };
    if let Some(id) = fallback {
        // Auto-set this as the user's default so future messages route directly
        router.set_user_default(message.sender.platform_id.clone(), id);
    }
    fallback
}

/// Dispatch a single incoming message — handles bot commands or routes to an agent.
///
/// Applies per-channel policies (DM/group filtering, rate limiting, formatting, threading).
/// Input sanitization runs early — before any command parsing or agent dispatch.
#[allow(clippy::too_many_arguments)]
async fn dispatch_message(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    rate_limiter: &ChannelRateLimiter,
    sanitizer: &InputSanitizer,
    journal: Option<&crate::message_journal::MessageJournal>,
    thread_ownership: &Arc<crate::thread_ownership::ThreadOwnershipRegistry>,
) {
    let ct_str = channel_type_str(&message.channel);

    // --- Webhook direct delivery (deliver_only mode) ---
    // If the incoming message was tagged by a deliver_only webhook route,
    // forward the content straight to the configured delivery channel and
    // return early — no LLM or agent is involved.
    if message
        .metadata
        .get("__deliver_only__")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let target = message
            .metadata
            .get("__deliver_target__")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let text = match &message.content {
            ChannelContent::Text(t) => t.as_str(),
            _ => "",
        };
        let route = message
            .metadata
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or(ct_str);
        info!(
            route = route,
            target = target,
            "webhook: direct delivery for route {}, skipping agent",
            route
        );
        if !target.is_empty() && !text.is_empty() {
            if let Err(e) = handle
                .send_channel_push(target, &message.sender.platform_id, text, None)
                .await
            {
                warn!(target = target, error = %e, "webhook direct delivery failed");
            }
        }
        return;
    }

    // --- Input sanitization (prompt injection detection) ---
    if !sanitizer.is_off() {
        // Command-type messages are checked by reconstructing their text
        // so that slash-command args cannot carry prompt-injection payloads.
        let text_to_check: Option<String> = match &message.content {
            ChannelContent::Text(t) => Some(t.clone()),
            ChannelContent::Command { name, args } => {
                if args.is_empty() {
                    Some(format!("/{name}"))
                } else {
                    Some(format!("/{name} {}", args.join(" ")))
                }
            }
            ChannelContent::Image { caption, .. } => caption.clone(),
            ChannelContent::Voice { caption, .. } => caption.clone(),
            ChannelContent::Video { caption, .. } => caption.clone(),
            _ => None,
        };
        let message_type = match &message.content {
            ChannelContent::Command { .. } => "Command",
            _ => "User",
        };
        if let Some(ref text) = text_to_check {
            match sanitizer.check(text) {
                SanitizeResult::Clean => {}
                SanitizeResult::Warned(reason) => {
                    warn!(
                        channel = ct_str,
                        user = %message.sender.display_name,
                        message_type = message_type,
                        reason = reason.as_str(),
                        "Suspicious channel input (warn mode, allowing through)"
                    );
                }
                SanitizeResult::Blocked(reason) => {
                    warn!(
                        channel = ct_str,
                        source = %message.sender.display_name,
                        message_type = message_type,
                        reason = reason.as_str(),
                        "Input sanitizer blocked potential prompt injection in {message_type} message from {}"
                        , message.sender.display_name,
                    );
                    if let Err(e) = adapter
                        .send(
                            &message.sender,
                            ChannelContent::Text(
                                "Your message could not be processed.".to_string(),
                            ),
                        )
                        .await
                    {
                        warn!(
                            channel = ct_str,
                            recipient = %message.sender.display_name,
                            error = %e,
                            "Failed to deliver sanitizer-block notice to user",
                        );
                    }
                    return;
                }
            }
        }
    }

    // Resolve target agent early so per-agent overrides can take priority
    let early_agent_id = resolve_or_fallback(message, handle, router).await;

    // Fetch overrides: agent-level (from agent.toml) wins, channel-level is fallback.
    let channel_overrides = handle
        .channel_overrides(
            ct_str,
            message.metadata.get("account_id").and_then(|v| v.as_str()),
        )
        .await;
    let overrides = if let Some(aid) = early_agent_id {
        handle
            .agent_channel_overrides(aid)
            .await
            .or(channel_overrides)
    } else {
        channel_overrides
    };
    let channel_default_format = default_output_format_for_channel(ct_str);
    let output_format = overrides
        .as_ref()
        .and_then(|o| o.output_format)
        .unwrap_or(channel_default_format);
    let threading_enabled = overrides.as_ref().map(|o| o.threading).unwrap_or(false);
    let thread_id = if threading_enabled {
        message.thread_id.as_deref()
    } else {
        None
    };

    // --- DM/Group policy check ---
    if let Some(ref ov) = overrides {
        if message.is_group {
            // capture the group_jid before the gating call so
            // both branches (record-on-skip, drain-on-pass) can use the
            // same key without re-deriving it. The bridge keys group
            // messages by `sender.platform_id` (= chat JID for groups).
            let group_id = message.sender.platform_id.clone();

            if !should_process_group_message(ct_str, ov, message) {
                // Record the skipped message into the per-group buffer so
                // the next addressed turn on this group can recover its
                // text. Only plain-text content reaches `dispatch_message`
                // (media goes through `dispatch_with_blocks` which doesn't
                // gate); recording empty text would just bloat the
                // buffer, so we skip when nothing useful is extractable.
                if let Some(buffer) = crate::group_history::global() {
                    if let Some(text) = text_content(message) {
                        if !text.is_empty() {
                            let entry = crate::group_history::HistoryEntry {
                                sender_display_name: message.sender.display_name.clone(),
                                text: text.to_string(),
                                captured_at: std::time::Instant::now(),
                            };
                            buffer
                                .record(&crate::group_history::group_key(ct_str, &group_id), entry)
                                .await;
                        }
                    }
                }
                return;
            }
            // Gating pass: the drain is deferred to the dispatch site
            // (just before the journal record) so per-channel rate-limit,
            // per-user rate-limit, reply-intent precheck, command-policy,
            // thread-ownership, RBAC, and auto-reply early-returns can
            // each take their turn first. Draining here would empty the
            // buffer even when one of those gates suppresses the message,
            // erasing the very context the next addressed turn was meant
            // to recover. See `dispatch_message` near the journal-record
            // call for the actual drain.
            // Reply-intent precheck: lightweight LLM classification for group
            // messages when group_policy is "all" and precheck is enabled.
            // Skipped for mentions and commands (already filtered above).
            if ov.reply_precheck && matches!(ov.group_policy, GroupPolicy::All) {
                let text = text_content(message).unwrap_or("");
                let sender = &message.sender.display_name;
                let model = ov.reply_precheck_model.as_deref();
                let account_id = message.metadata.get("account_id").and_then(|v| v.as_str());
                let channel_key_for_name = match account_id {
                    Some(aid) => format!("{}:{}", ct_str, aid),
                    None => ct_str.to_string(),
                };
                let bot_name = router.channel_default_name(&channel_key_for_name);
                let aliases = if ov.group_trigger_patterns.is_empty() {
                    None
                } else {
                    Some(ov.group_trigger_patterns.as_slice())
                };
                if !handle
                    .classify_reply_intent(text, sender, model, bot_name.as_deref(), aliases)
                    .await
                {
                    debug!(
                        channel = ct_str,
                        sender = %sender,
                        "Reply precheck declined — staying silent"
                    );
                    return;
                }
            }
        } else {
            // DM
            match ov.dm_policy {
                DmPolicy::Ignore => {
                    debug!("Ignoring DM on {ct_str} (dm_policy=ignore)");
                    return;
                }
                DmPolicy::AllowedOnly => {
                    // Rely on RBAC authorize_channel_user below
                }
                DmPolicy::Respond => {}
            }
        }
    }

    // --- Rate limiting ---
    if let Some(ref ov) = overrides {
        // Global per-channel rate limit (all users combined)
        if ov.rate_limit_per_minute > 0 {
            if let Err(msg) = rate_limiter.check(ct_str, "__global__", ov.rate_limit_per_minute) {
                send_response(adapter, &message.sender, msg, thread_id, output_format).await;
                return;
            }
        }
        // Per-user rate limit
        if ov.rate_limit_per_user > 0 {
            if let Err(msg) =
                rate_limiter.check(ct_str, sender_user_id(message), ov.rate_limit_per_user)
            {
                send_response(adapter, &message.sender, msg, thread_id, output_format).await;
                return;
            }
        }
    }

    // Handle commands first (early return) — unless the per-channel command
    // policy blocks this command, in which case we fall through and treat it
    // as normal text forwarded to the agent.
    if let ChannelContent::Command { ref name, ref args } = message.content {
        if is_command_allowed(name, overrides.as_ref()) {
            // Special-case /agents: send an inline keyboard with one button per agent.
            if name == "agents" {
                let agents = handle.list_agents().await.unwrap_or_default();
                if !agents.is_empty() {
                    let buttons: Vec<Vec<InteractiveButton>> = agents
                        .into_iter()
                        .map(|(_, agent_name)| {
                            // Telegram callback_data limit is 64 bytes.
                            // "/agent " is 7 bytes; truncate name to 57 bytes if needed.
                            let action = {
                                let prefix = "/agent ";
                                let safe_name = truncate_utf8(&agent_name, 64 - prefix.len());
                                format!("{prefix}{safe_name}")
                            };
                            vec![InteractiveButton {
                                label: agent_name,
                                action,
                                style: None,
                                url: None,
                            }]
                        })
                        .collect();
                    let content = ChannelContent::Interactive {
                        text: "Select an agent:".to_string(),
                        buttons,
                    };
                    let result = if let Some(tid) = thread_id {
                        adapter.send_in_thread(&message.sender, content, tid).await
                    } else {
                        adapter.send(&message.sender, content).await
                    };
                    if let Err(e) = result {
                        error!("Failed to send /agents interactive message: {e}");
                    }
                    return;
                }
                // Empty agent list — fall through to handle_command for plain text response.
            }
            // Special-case /models: send an inline keyboard with one button per provider.
            if name == "models" {
                let providers = handle.list_providers_interactive().await;
                if !providers.is_empty() {
                    let buttons: Vec<Vec<InteractiveButton>> = providers
                        .into_iter()
                        .map(|(pid, pname, _auth_ok)| {
                            let action = {
                                let prefix = "prov:";
                                let safe_id = truncate_utf8(&pid, 64 - prefix.len());
                                format!("{prefix}{safe_id}")
                            };
                            vec![InteractiveButton {
                                label: pname,
                                action,
                                style: None,
                                url: None,
                            }]
                        })
                        .collect();
                    let content = ChannelContent::Interactive {
                        text: "Select a provider:".to_string(),
                        buttons,
                    };
                    let result = if let Some(tid) = thread_id {
                        adapter.send_in_thread(&message.sender, content, tid).await
                    } else {
                        adapter.send(&message.sender, content).await
                    };
                    if let Err(e) = result {
                        error!("Failed to send /models interactive message: {e}");
                    }
                    return;
                }
                // Empty provider list — fall through to handle_command for plain text response.
            }
            let result = handle_command(
                name,
                args,
                handle,
                router,
                &message.sender,
                &message.channel,
                overrides.as_ref(),
            )
            .await;
            send_response(adapter, &message.sender, result, thread_id, output_format).await;
            return;
        }
        debug!(
            command = name,
            channel = ct_str,
            "Command blocked by channel policy — forwarding to agent as text"
        );
    }

    // For images: download, base64 encode, and send as multimodal content blocks
    if let ChannelContent::Image {
        ref url,
        ref caption,
        ref mime_type,
    } = message.content
    {
        let upload_dir = handle.effective_channels_download_dir();
        let extra_headers = adapter.fetch_headers_for(url);
        let blocks = download_image_to_blocks(
            url,
            caption.as_deref(),
            mime_type.as_deref(),
            &upload_dir,
            &extra_headers,
        )
        .await;
        if blocks.iter().any(|b| {
            matches!(
                b,
                ContentBlock::Image { .. } | ContentBlock::ImageFile { .. }
            )
        }) {
            // We have actual image data — send as structured blocks for vision
            dispatch_with_blocks(
                blocks,
                message,
                handle,
                router,
                adapter,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal,
                thread_ownership,
            )
            .await;
            return;
        }
        // Image download failed — fall through to text description below
    }

    // For files: download to disk and send as content blocks
    if let ChannelContent::File {
        ref url,
        ref filename,
    } = message.content
    {
        let download_dir = handle.effective_channels_download_dir();
        let max_bytes = handle
            .channels_download_max_bytes()
            .unwrap_or(CHANNEL_FILE_DOWNLOAD_MAX_BYTES);
        let extra_headers = adapter.fetch_headers_for(url);
        let blocks =
            download_file_to_blocks(url, filename, max_bytes, &download_dir, &extra_headers).await;
        if has_file_saved_block(&blocks) {
            dispatch_with_blocks(
                blocks,
                message,
                handle,
                router,
                adapter,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal,
                thread_ownership,
            )
            .await;
            return;
        }
        // Download failed — fall through to text description below
    }

    // For voice messages: download to disk and send as content blocks so
    // tools like media_transcribe can read the saved file directly.
    if let ChannelContent::Voice {
        ref url,
        ref caption,
        duration_seconds,
    } = message.content
    {
        let download_dir = handle.effective_channels_download_dir();
        let max_bytes = handle
            .channels_download_max_bytes()
            .unwrap_or(CHANNEL_FILE_DOWNLOAD_MAX_BYTES);
        let filename = filename_from_url(url).unwrap_or_else(|| "voice.ogg".to_string());
        let extra_headers = adapter.fetch_headers_for(url);
        let mut blocks =
            download_file_to_blocks(url, &filename, max_bytes, &download_dir, &extra_headers).await;
        if has_file_saved_block(&blocks) {
            // Prepend a context block carrying duration + caption so the
            // model knows this is voice (not an arbitrary file) and any
            // user-supplied caption survives the save-path replacement.
            let context = match caption {
                Some(c) if !c.is_empty() => {
                    format!("[Voice message ({duration_seconds}s)]\nCaption: {c}")
                }
                _ => format!("[Voice message ({duration_seconds}s)]"),
            };
            blocks.insert(
                0,
                ContentBlock::Text {
                    text: context,
                    provider_metadata: None,
                },
            );
            dispatch_with_blocks(
                blocks,
                message,
                handle,
                router,
                adapter,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal,
                thread_ownership,
            )
            .await;
            return;
        }
        // Download failed — fall through to text description below
    }

    // For audio (music/podcast — distinct from voice memos): same pattern
    // as Voice. Audio carries optional title/performer metadata which we
    // surface in the prepended context block.
    if let ChannelContent::Audio {
        ref url,
        ref caption,
        duration_seconds,
        ref title,
        ref performer,
    } = message.content
    {
        let download_dir = handle.effective_channels_download_dir();
        let max_bytes = handle
            .channels_download_max_bytes()
            .unwrap_or(CHANNEL_FILE_DOWNLOAD_MAX_BYTES);
        let filename = filename_from_url(url).unwrap_or_else(|| "audio.mp3".to_string());
        let extra_headers = adapter.fetch_headers_for(url);
        let mut blocks =
            download_file_to_blocks(url, &filename, max_bytes, &download_dir, &extra_headers).await;
        if has_file_saved_block(&blocks) {
            let mut header = format!("[Audio ({duration_seconds}s)");
            match (title.as_deref(), performer.as_deref()) {
                (Some(t), Some(p)) if !t.is_empty() && !p.is_empty() => {
                    header.push_str(&format!(" — {t} by {p}"));
                }
                (Some(t), _) if !t.is_empty() => header.push_str(&format!(" — {t}")),
                (_, Some(p)) if !p.is_empty() => header.push_str(&format!(" by {p}")),
                _ => {}
            }
            header.push(']');
            let context = match caption {
                Some(c) if !c.is_empty() => format!("{header}\nCaption: {c}"),
                _ => header,
            };
            blocks.insert(
                0,
                ContentBlock::Text {
                    text: context,
                    provider_metadata: None,
                },
            );
            dispatch_with_blocks(
                blocks,
                message,
                handle,
                router,
                adapter,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal,
                thread_ownership,
            )
            .await;
            return;
        }
        // Download failed — fall through to text description below
    }

    // For video messages: same pattern as Voice. Prefer the channel-
    // provided `filename` when present, otherwise derive from URL, then
    // fall back to a stable default so the saved file always has an
    // extension hint for `media_transcribe` / vision tools.
    if let ChannelContent::Video {
        ref url,
        ref caption,
        duration_seconds,
        ref filename,
    } = message.content
    {
        let download_dir = handle.effective_channels_download_dir();
        let max_bytes = handle
            .channels_download_max_bytes()
            .unwrap_or(CHANNEL_FILE_DOWNLOAD_MAX_BYTES);
        let resolved_filename = filename
            .clone()
            .or_else(|| filename_from_url(url))
            .unwrap_or_else(|| "video.mp4".to_string());
        let extra_headers = adapter.fetch_headers_for(url);
        let mut blocks = download_file_to_blocks(
            url,
            &resolved_filename,
            max_bytes,
            &download_dir,
            &extra_headers,
        )
        .await;
        if has_file_saved_block(&blocks) {
            let context = match caption {
                Some(c) if !c.is_empty() => {
                    format!("[Video ({duration_seconds}s)]\nCaption: {c}")
                }
                _ => format!("[Video ({duration_seconds}s)]"),
            };
            blocks.insert(
                0,
                ContentBlock::Text {
                    text: context,
                    provider_metadata: None,
                },
            );
            dispatch_with_blocks(
                blocks,
                message,
                handle,
                router,
                adapter,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                journal,
                thread_ownership,
            )
            .await;
            return;
        }
        // Download failed — fall through to text description below
    }

    // Intercept interactive menu callbacks before forwarding to LLM.
    if let ChannelContent::ButtonCallback { ref action, .. } = message.content {
        if action.starts_with("prov:") || action.starts_with("model:") || action == "back:providers"
        {
            let mid = message
                .metadata
                .get("message_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let Some(message_id) = mid else {
                debug!("ButtonCallback menu: missing message_id in metadata, ignoring");
                return;
            };
            if action.starts_with("prov:") {
                let provider_id = action.strip_prefix("prov:").unwrap_or("");
                let models = handle.list_models_by_provider(provider_id).await;
                let provider_label = provider_id.to_string();
                let mut buttons: Vec<Vec<InteractiveButton>> = models
                    .iter()
                    .map(|(mid_str, mlabel)| {
                        let action_str = {
                            let prefix = "model:";
                            let safe_id = truncate_utf8(mid_str, 64 - prefix.len());
                            format!("{prefix}{safe_id}")
                        };
                        vec![InteractiveButton {
                            label: mlabel.clone(),
                            action: action_str,
                            style: None,
                            url: None,
                        }]
                    })
                    .collect();
                buttons.push(vec![InteractiveButton {
                    label: "\u{2B05} Back".to_string(),
                    action: "back:providers".to_string(),
                    style: None,
                    url: None,
                }]);
                let content = ChannelContent::EditInteractive {
                    message_id,
                    text: format!("{provider_label} \u{2014} select a model:"),
                    buttons,
                };
                let result = if let Some(tid) = thread_id {
                    adapter.send_in_thread(&message.sender, content, tid).await
                } else {
                    adapter.send(&message.sender, content).await
                };
                if let Err(e) = result {
                    error!("Failed to send provider models menu: {e}");
                }
            } else if action == "back:providers" {
                let providers = handle.list_providers_interactive().await;
                let buttons: Vec<Vec<InteractiveButton>> = providers
                    .into_iter()
                    .map(|(pid, pname, _auth_ok)| {
                        let action_str = {
                            let prefix = "prov:";
                            let safe_id = truncate_utf8(&pid, 64 - prefix.len());
                            format!("{prefix}{safe_id}")
                        };
                        vec![InteractiveButton {
                            label: pname,
                            action: action_str,
                            style: None,
                            url: None,
                        }]
                    })
                    .collect();
                let content = ChannelContent::EditInteractive {
                    message_id,
                    text: "Select a provider:".to_string(),
                    buttons,
                };
                let result = if let Some(tid) = thread_id {
                    adapter.send_in_thread(&message.sender, content, tid).await
                } else {
                    adapter.send(&message.sender, content).await
                };
                if let Err(e) = result {
                    error!("Failed to send providers back menu: {e}");
                }
            } else if action.starts_with("model:") {
                let model_id = action.strip_prefix("model:").unwrap_or("");
                let agent_id = router.resolve(
                    &message.channel,
                    &message.sender.platform_id,
                    message.sender.librefang_user.as_deref(),
                );
                let label = {
                    // Best-effort: look up display name from all providers
                    // (we don't know which provider this model belongs to here)
                    model_id.to_string()
                };
                let confirmation = if let Some(aid) = agent_id {
                    match handle.set_model(aid, model_id).await {
                        Ok(_) => format!("\u{2705} Active model: {label}"),
                        Err(e) => format!("\u{274C} Could not set model: {e}"),
                    }
                } else {
                    format!("\u{2705} Active model: {label}\n(No agent selected \u{2014} use /agent to choose one)")
                };
                let content = ChannelContent::EditInteractive {
                    message_id,
                    text: confirmation,
                    buttons: vec![],
                };
                let result = if let Some(tid) = thread_id {
                    adapter.send_in_thread(&message.sender, content, tid).await
                } else {
                    adapter.send(&message.sender, content).await
                };
                if let Err(e) = result {
                    error!("Failed to send model confirmation: {e}");
                }
            }
            return;
        }
    }

    let text = match &message.content {
        ChannelContent::Text(t) => t.clone(),
        ChannelContent::Command { name, args } => reconstruct_command_text(name, args),
        ChannelContent::Image {
            ref url,
            ref caption,
            ..
        } => {
            // Fallback when image download failed
            match caption {
                Some(c) => format!("[User sent a photo: {url}]\nCaption: {c}"),
                None => format!("[User sent a photo: {url}]"),
            }
        }
        ChannelContent::File {
            ref url,
            ref filename,
        } => {
            format!("[User sent a file ({filename}): {url}]")
        }
        ChannelContent::Voice {
            ref url,
            ref caption,
            duration_seconds,
        } => match caption {
            Some(c) => {
                format!("[User sent a voice message ({duration_seconds}s): {url}]\nCaption: {c}")
            }
            None => format!("[User sent a voice message ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Video {
            ref url,
            ref caption,
            duration_seconds,
            ..
        } => match caption {
            Some(c) => {
                format!("[User sent a video ({duration_seconds}s): {url}]\nCaption: {c}")
            }
            None => format!("[User sent a video ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Location { lat, lon } => {
            format!("[User shared location: {lat}, {lon}]")
        }
        ChannelContent::FileData { ref filename, .. } => {
            format!("[User sent a local file: {filename}]")
        }
        ChannelContent::Interactive { ref text, .. } => {
            // Interactive messages are outbound-only; if one arrives as inbound
            // treat the text portion as the user message.
            text.clone()
        }
        ChannelContent::ButtonCallback {
            ref action,
            ref message_text,
        } => {
            // If action starts with '/', treat it as a slash command directly.
            // This allows interactive buttons (e.g. Approve/Reject on approval
            // notifications) to trigger commands like /approve or /reject.
            if action.starts_with('/') {
                action.clone()
            } else {
                match message_text {
                    Some(mt) => format!("[Button clicked: {action}] (on message: {mt})"),
                    None => format!("[Button clicked: {action}]"),
                }
            }
        }
        ChannelContent::DeleteMessage { ref message_id } => {
            format!("[Delete message: {message_id}]")
        }
        ChannelContent::EditInteractive { ref text, .. } => text.clone(),
        ChannelContent::Audio {
            ref url,
            ref caption,
            duration_seconds,
            ..
        } => match caption {
            Some(c) => format!("[User sent audio ({duration_seconds}s): {url}]\nCaption: {c}"),
            None => format!("[User sent audio ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Animation {
            ref url,
            ref caption,
            duration_seconds,
        } => match caption {
            Some(c) => {
                format!("[User sent animation ({duration_seconds}s): {url}]\nCaption: {c}")
            }
            None => format!("[User sent animation ({duration_seconds}s): {url}]"),
        },
        ChannelContent::Sticker { ref file_id } => format!("[User sent sticker: {file_id}]"),
        ChannelContent::MediaGroup { ref items } => {
            format!("[User sent media group: {} items]", items.len())
        }
        ChannelContent::Poll { ref question, .. } => format!("[Poll: {question}]"),
        ChannelContent::PollAnswer {
            ref poll_id,
            ref option_ids,
        } => {
            let question = message
                .metadata
                .get("poll_question")
                .and_then(|v| v.as_str())
                .unwrap_or(poll_id);
            let options: Vec<String> = message
                .metadata
                .get("poll_options")
                .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
                .unwrap_or_default();
            if options.is_empty() {
                format!("[User answered poll {poll_id}: options {option_ids:?}]")
            } else {
                let selected: Vec<&str> = option_ids
                    .iter()
                    .filter_map(|&i| options.get(i as usize).map(|s| s.as_str()))
                    .collect();
                format!("[User answered poll \"{question}\": selected {selected:?}]")
            }
        }
    };

    // Check if it's a slash command embedded in text (e.g. "/agents")
    if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = &parts[0][1..]; // strip leading '/'
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };

        if crate::commands::is_channel_command(cmd) {
            if is_command_allowed(cmd, overrides.as_ref()) {
                // Special-case /agents: send an inline keyboard with one button per agent.
                if cmd == "agents" {
                    let agents = handle.list_agents().await.unwrap_or_default();
                    if !agents.is_empty() {
                        let buttons: Vec<Vec<InteractiveButton>> = agents
                            .into_iter()
                            .map(|(_, name)| {
                                // Telegram callback_data limit is 64 bytes.
                                // "/agent " is 7 bytes; truncate name to 57 bytes if needed.
                                let action = {
                                    let prefix = "/agent ";
                                    let safe_name = truncate_utf8(&name, 64 - prefix.len());
                                    format!("{prefix}{safe_name}")
                                };
                                vec![InteractiveButton {
                                    label: name,
                                    action,
                                    style: None,
                                    url: None,
                                }]
                            })
                            .collect();
                        let content = ChannelContent::Interactive {
                            text: "Select an agent:".to_string(),
                            buttons,
                        };
                        let result = if let Some(tid) = thread_id {
                            adapter.send_in_thread(&message.sender, content, tid).await
                        } else {
                            adapter.send(&message.sender, content).await
                        };
                        if let Err(e) = result {
                            error!("Failed to send /agents interactive message: {e}");
                        }
                        return;
                    }
                    // Empty agent list — fall through to handle_command for plain text response.
                }
                // Special-case /models: send an inline keyboard with one button per provider.
                if cmd == "models" {
                    let providers = handle.list_providers_interactive().await;
                    if !providers.is_empty() {
                        let buttons: Vec<Vec<InteractiveButton>> = providers
                            .into_iter()
                            .map(|(pid, pname, _auth_ok)| {
                                let action = {
                                    let prefix = "prov:";
                                    let safe_id = truncate_utf8(&pid, 64 - prefix.len());
                                    format!("{prefix}{safe_id}")
                                };
                                vec![InteractiveButton {
                                    label: pname,
                                    action,
                                    style: None,
                                    url: None,
                                }]
                            })
                            .collect();
                        let content = ChannelContent::Interactive {
                            text: "Select a provider:".to_string(),
                            buttons,
                        };
                        let result = if let Some(tid) = thread_id {
                            adapter.send_in_thread(&message.sender, content, tid).await
                        } else {
                            adapter.send(&message.sender, content).await
                        };
                        if let Err(e) = result {
                            error!("Failed to send /models interactive message: {e}");
                        }
                        return;
                    }
                    // Empty provider list — fall through to handle_command for plain text response.
                }
                let result = handle_command(
                    cmd,
                    &args,
                    handle,
                    router,
                    &message.sender,
                    &message.channel,
                    overrides.as_ref(),
                )
                .await;
                send_response(adapter, &message.sender, result, thread_id, output_format).await;
                return;
            }
            debug!(
                command = cmd,
                channel = ct_str,
                "Command blocked by channel policy — forwarding to agent as text"
            );
        }
        // Other slash commands (and blocked ones) pass through to the agent
    }

    // Check broadcast routing first
    if router.has_broadcast(&message.sender.platform_id) {
        let targets = router.resolve_broadcast(&message.sender.platform_id);
        if !targets.is_empty() {
            // RBAC check applies to broadcast too
            if let Err(denied) = handle
                .authorize_channel_user(ct_str, sender_user_id(message), "chat")
                .await
            {
                send_response(
                    adapter,
                    &message.sender,
                    format!("Access denied: {denied}"),
                    thread_id,
                    output_format,
                )
                .await;
                return;
            }
            if let Err(e) = adapter.send_typing(&message.sender).await {
                debug!(adapter = adapter.name(), error = %e, "send_typing failed (best-effort)");
            }

            let strategy = router.broadcast_strategy();
            let mut responses = Vec::new();

            match strategy {
                librefang_types::config::BroadcastStrategy::Parallel => {
                    let mut handles_vec = Vec::new();
                    for (name, maybe_id) in &targets {
                        if let Some(aid) = maybe_id {
                            let h = handle.clone();
                            let t = text.clone();
                            let aid = *aid;
                            let name = name.clone();
                            handles_vec.push(tokio::spawn(async move {
                                let result = h.send_message(aid, &t).await;
                                (name, aid, result)
                            }));
                        }
                    }
                    for jh in handles_vec {
                        if let Ok((name, _aid, result)) = jh.await {
                            match result {
                                Ok(r) if !r.is_empty() => responses.push(format!("[{name}]: {r}")),
                                Ok(_) => {} // silent response — skip
                                Err(e) => {
                                    if !adapter.suppress_error_responses() {
                                        responses.push(format!("[{name}]: Error: {e}"));
                                    }
                                }
                            }
                        }
                    }
                }
                librefang_types::config::BroadcastStrategy::Sequential => {
                    for (name, maybe_id) in &targets {
                        if let Some(aid) = maybe_id {
                            match handle.send_message(*aid, &text).await {
                                Ok(r) if !r.is_empty() => responses.push(format!("[{name}]: {r}")),
                                Ok(_) => {} // silent response — skip
                                Err(e) => {
                                    if !adapter.suppress_error_responses() {
                                        responses.push(format!("[{name}]: Error: {e}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let combined = responses.join("\n\n");
            if !combined.is_empty() {
                send_response(adapter, &message.sender, combined, thread_id, output_format).await;
            }
            return;
        }
    }

    let agent_id = match early_agent_id {
        Some(id) => id,
        None => {
            send_response(
                adapter,
                &message.sender,
                "No agents available. Start the dashboard at http://127.0.0.1:4545 to create one."
                    .to_string(),
                thread_id,
                output_format,
            )
            .await;
            return;
        }
    };

    // Thread-ownership gate (#3334). Only meaningful for group threads with
    // a platform thread id; DMs and untreaded channels bypass entirely.
    // An explicit @-mention re-claims the thread for the new agent.
    if message.is_group
        && overrides
            .as_ref()
            .map(|o| o.thread_ownership_enabled)
            .unwrap_or(true)
    {
        if let Some(thread_str) = message.thread_id.as_deref() {
            if let Some(key) = crate::thread_ownership::ThreadKey::new(ct_str, thread_str) {
                let was_mentioned = message
                    .metadata
                    .get("was_mentioned")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match thread_ownership.decide(key, agent_id, was_mentioned) {
                    crate::thread_ownership::DispatchDecision::Allow { .. } => {}
                    crate::thread_ownership::DispatchDecision::Suppress { holder } => {
                        debug!(
                            channel = ct_str,
                            thread_id = thread_str,
                            candidate = %agent_id,
                            holder = %holder,
                            "thread_ownership: suppressing dispatch — another agent owns this thread"
                        );
                        return;
                    }
                }
            }
        }
    }

    let channel_key = channel_type_str(&message.channel).to_string();

    // RBAC: authorize the user before forwarding to agent
    if let Err(denied) = handle
        .authorize_channel_user(ct_str, sender_user_id(message), "chat")
        .await
    {
        send_response(
            adapter,
            &message.sender,
            format!("Access denied: {denied}"),
            thread_id,
            output_format,
        )
        .await;
        return;
    }

    // Auto-reply check — if enabled, the engine decides whether to process this message.
    // If auto-reply is enabled but suppressed for this message, skip agent call entirely.
    if let Some(reply) = handle.check_auto_reply(agent_id, &text).await {
        let reply = maybe_prefix_response(handle, overrides.as_ref(), agent_id, reply).await;
        send_response(adapter, &message.sender, reply, thread_id, output_format).await;
        handle
            .record_delivery(
                agent_id,
                ct_str,
                &message.sender.platform_id,
                true,
                None,
                thread_id,
            )
            .await;
        return;
    }

    // --- Group-history drain (gating pass survived all early-return gates) ---
    //
    // Done here, after rate-limit / reply-intent / command-policy /
    // thread-ownership / RBAC / auto-reply have all let the message
    // through — earlier in the gating block we'd erase the buffer even
    // when one of these suppressed the dispatch, costing the very
    // context the next addressed turn was meant to recover. The drained
    // count is log-only in v1; the kernel-side prompt enrichment that
    // consumes `drained` is the follow-up PR.
    if message.is_group {
        if let Some(buffer) = crate::group_history::global() {
            let key = crate::group_history::group_key(ct_str, &message.sender.platform_id);
            if let Some(drained) = buffer.drain(&key).await {
                info!(
                    event = "group_history_drained",
                    channel = ct_str,
                    group = %message.sender.platform_id,
                    entries = drained.len(),
                    "drained prior group entries on gating pass",
                );
            }
        }
    }

    // --- Message journal: record before dispatch for crash recovery ---
    if let Some(j) = journal {
        let entry = crate::message_journal::JournalEntry {
            message_id: message.platform_message_id.clone(),
            channel: ct_str.to_string(),
            sender_id: message.sender.platform_id.clone(),
            sender_name: message.sender.display_name.clone(),
            content: text.clone(),
            agent_name: None, // resolved at re-dispatch if needed
            received_at: message.timestamp,
            status: crate::message_journal::JournalStatus::Processing,
            attempts: 0,
            last_error: None,
            updated_at: chrono::Utc::now(),
            is_group: message.is_group,
            thread_id: thread_id.map(|s| s.to_string()),
            metadata: std::collections::HashMap::new(),
            next_retry_after: None,
        };
        j.record(entry).await;
    }

    // Send typing indicator (best-effort)
    if let Err(e) = adapter.send_typing(&message.sender).await {
        debug!(adapter = adapter.name(), error = %e, "send_typing failed (best-effort)");
    }

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Thinking).await;

    upsert_sender_into_roster(handle, message).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message, overrides.as_ref());

    // Streaming path: if the adapter supports progressive output, pipe text
    // deltas directly to it instead of waiting for the full response.
    //
    // We use the `_status` variant of the streaming kernel call so we can
    // distinguish four outcomes once both `send_streaming` and the kernel
    // have settled:
    //   1. send_streaming Ok + kernel Ok  → real success
    //   2. send_streaming Ok + kernel Err → adapter delivered partial text
    //      but the agent loop ultimately failed; emit Error reaction and
    //      record_delivery(false) so metrics reflect reality
    //   3. send_streaming Err + kernel Ok → adapter HTTP failed mid-stream
    //      but the agent loop produced a clean response; fall back to
    //      send_response(buffered_text) and emit Done
    //   4. send_streaming Err + kernel Err → both failed; honor
    //      suppress_error_responses when delivering the buffered error
    //      text via the fallback path
    if adapter.supports_streaming() {
        match handle
            .send_message_streaming_with_sender_status(agent_id, &text, &sender_ctx)
            .await
        {
            Ok((mut delta_rx, status_rx)) => {
                send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Streaming)
                    .await;

                // Resolve the agent-name prefix once up-front so it can be
                // injected as the very first delta — without this, streaming
                // adapters (e.g. Telegram) would never show the prefix on the
                // success path. `None` when prefix is disabled, agent unknown,
                // or the agent has no display name.
                let prefix_chunk = resolve_prefix_chunk(handle, overrides.as_ref(), agent_id).await;

                // Tee: forward deltas to the adapter while buffering a copy.
                // If send_streaming fails, the buffer lets us fall back to send().
                //
                // Drain runs as a sibling future via `tokio::join!` (not a
                // detached `tokio::spawn`) so it shares the dispatch task's
                // borrow of `adapter`. That lets us call
                // `send_lifecycle_reaction(adapter, ...)` from inside the
                // drain when we observe the api/channel_bridge's
                // `\n\n🔧 toolname\n\n` text marker — a turn that runs a
                // tool now flips the trigger-message reaction to ⚙️ for the
                // duration of the call, instead of staying stuck on ✍️.
                let (adapter_tx, adapter_rx) = mpsc::channel::<String>(64);
                let prefix_chunk_owned = prefix_chunk.clone();
                let drain_fut = async {
                    let mut buffered = String::new();
                    // Inject the prefix as the first delta so it becomes
                    // part of the streamed message. Mirror it into the
                    // buffer so the stream-fail fallback path's
                    // idempotency check (`apply_agent_prefix`) sees an
                    // already-prefixed buffer and skips re-prefixing.
                    if let Some(ref p) = prefix_chunk_owned {
                        buffered.push_str(p);
                        if adapter_tx.send(p.clone()).await.is_err() {
                            return buffered;
                        }
                    }
                    while let Some(delta) = delta_rx.recv().await {
                        buffered.push_str(&delta);
                        if let Some(name) = extract_tool_marker_name(&delta) {
                            send_lifecycle_reaction(
                                adapter,
                                &message.sender,
                                msg_id,
                                &AgentPhase::tool_use(&name),
                            )
                            .await;
                        }
                        // Best-effort forward — if adapter dropped rx, stop.
                        if adapter_tx.send(delta).await.is_err() {
                            break;
                        }
                    }
                    drop(adapter_tx);
                    buffered
                };

                let (stream_result, buffered_text) = tokio::join!(
                    adapter.send_streaming(&message.sender, adapter_rx, thread_id),
                    drain_fut
                );

                // Status is sent after the text channel fully drains, so
                // awaiting here will not block longer than the stream itself.
                let kernel_status = status_rx.await.unwrap_or(Ok(()));
                let kernel_ok = kernel_status.is_ok();
                let kernel_err_str = kernel_status.as_ref().err().cloned();

                match &stream_result {
                    Ok(()) => {
                        // Adapter delivered. Final state depends on whether
                        // the agent loop itself succeeded.
                        let phase = if kernel_ok {
                            AgentPhase::Done
                        } else {
                            AgentPhase::Error
                        };
                        send_lifecycle_reaction(adapter, &message.sender, msg_id, &phase).await;
                        handle
                            .record_delivery(
                                agent_id,
                                ct_str,
                                &message.sender.platform_id,
                                kernel_ok,
                                kernel_err_str.as_deref(),
                                thread_id,
                            )
                            .await;
                        if let Some(j) = journal {
                            j.record_outcome(
                                &message.platform_message_id,
                                kernel_ok,
                                kernel_err_str.clone(),
                            )
                            .await;
                        }
                        return;
                    }
                    Err(e) => {
                        warn!("Streaming send failed, falling back to non-streaming: {e}");
                        // Fall back: re-send the full accumulated text via
                        // send_response so the user still gets a response.
                        // Honor suppress_error_responses when the kernel
                        // failed — the buffered text will contain a
                        // sanitized error string we should not leak to
                        // public-feed adapters.
                        if !buffered_text.is_empty()
                            && (kernel_ok || !adapter.suppress_error_responses())
                        {
                            let buffered_text = if kernel_ok {
                                maybe_prefix_response(
                                    handle,
                                    overrides.as_ref(),
                                    agent_id,
                                    buffered_text,
                                )
                                .await
                            } else {
                                buffered_text
                            };
                            send_response(
                                adapter,
                                &message.sender,
                                buffered_text,
                                thread_id,
                                output_format,
                            )
                            .await;
                            let phase = if kernel_ok {
                                AgentPhase::Done
                            } else {
                                AgentPhase::Error
                            };
                            send_lifecycle_reaction(adapter, &message.sender, msg_id, &phase).await;
                            // Pair the err field with the success flag — when
                            // kernel succeeded, the fallback send_response
                            // delivered the real reply, so the transport-side
                            // stream error is irrelevant to delivery accounting
                            // (record_delivery=true with err=Some is a
                            // contradictory signal). When kernel failed, keep
                            // the kernel error string so metrics know why.
                            // (`e`, the stream transport error, was already
                            // logged via warn! above.)
                            let err_str = if kernel_ok {
                                None
                            } else {
                                kernel_err_str.clone()
                            };
                            handle
                                .record_delivery(
                                    agent_id,
                                    ct_str,
                                    &message.sender.platform_id,
                                    kernel_ok,
                                    err_str.as_deref(),
                                    thread_id,
                                )
                                .await;
                            if let Some(j) = journal {
                                j.record_outcome(&message.platform_message_id, kernel_ok, err_str)
                                    .await;
                            }
                            return;
                        }
                        // Buffer was empty OR kernel errored on a
                        // suppress_error_responses adapter — give up cleanly.
                        send_lifecycle_reaction(
                            adapter,
                            &message.sender,
                            msg_id,
                            &AgentPhase::Error,
                        )
                        .await;
                        let err_str = kernel_err_str.unwrap_or_else(|| e.to_string());
                        handle
                            .record_delivery(
                                agent_id,
                                ct_str,
                                &message.sender.platform_id,
                                false,
                                Some(&err_str),
                                thread_id,
                            )
                            .await;
                        if let Some(j) = journal {
                            j.record_outcome(&message.platform_message_id, false, Some(err_str))
                                .await;
                        }
                        return;
                    }
                }
            }
            Err(e) => {
                // Streaming not available for this request — fall through to
                // non-streaming path below.
                debug!("Streaming unavailable, falling back to non-streaming: {e}");
            }
        }
    }

    // Non-streaming-adapter path. We route through the kernel's streaming
    // API (via `_status` variant) so progress events (tool invocations,
    // errors) get surfaced into the accumulated text — the channel bridge
    // injects "🔧 tool_name" and "⚠️ tool failed" lines for streaming
    // consumers, and we want non-streaming adapters (Discord/Slack/Matrix/...)
    // to show those too. We accumulate deltas and send once via send_response
    // so output_format and thread_id are still honored.
    //
    // The `_status` variant returns a oneshot that resolves to the kernel's
    // terminal Result. We use it to drive the correct lifecycle reaction
    // (Done vs Error), accurate `record_delivery` success metric, journal
    // status, and to honor `suppress_error_responses` on public-feed adapters
    // (Mastodon) — accumulated text contains a sanitized error string when
    // the agent loop fails, which we must not leak to a public timeline.
    //
    // If the streaming kernel call is unavailable up-front we fall through
    // to the non-streaming kernel call — preserves the pre-existing
    // `handle_send_error` retry / re-resolution path.
    if let Ok((mut delta_rx, status_rx)) = handle
        .send_message_streaming_with_sender_status(agent_id, &text, &sender_ctx)
        .await
    {
        let mut accumulated = String::new();
        while let Some(delta) = delta_rx.recv().await {
            accumulated.push_str(&delta);
        }
        // Status is sent after the text channel fully drains, so awaiting
        // here will not block longer than the stream itself.
        let kernel_status = status_rx.await.unwrap_or(Ok(()));
        let success = kernel_status.is_ok();
        let phase = if success {
            AgentPhase::Done
        } else {
            AgentPhase::Error
        };
        send_lifecycle_reaction(adapter, &message.sender, msg_id, &phase).await;
        if !accumulated.is_empty() && (success || !adapter.suppress_error_responses()) {
            let accumulated = if success {
                maybe_prefix_response(handle, overrides.as_ref(), agent_id, accumulated).await
            } else {
                accumulated
            };
            send_response(
                adapter,
                &message.sender,
                accumulated,
                thread_id,
                output_format,
            )
            .await;
        }
        let err_str = kernel_status.as_ref().err().cloned();
        handle
            .record_delivery(
                agent_id,
                ct_str,
                &message.sender.platform_id,
                success,
                err_str.as_deref(),
                thread_id,
            )
            .await;
        if let Some(j) = journal {
            j.record_outcome(&message.platform_message_id, success, err_str)
                .await;
        }
        return;
    }

    // Fallback: streaming kernel call unavailable for this request.
    match handle
        .send_message_with_sender(agent_id, &text, &sender_ctx)
        .await
    {
        Ok(response) => {
            send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Done).await;
            if !response.is_empty() {
                let response =
                    maybe_prefix_response(handle, overrides.as_ref(), agent_id, response).await;
                send_response(adapter, &message.sender, response, thread_id, output_format).await;
            }
            handle
                .record_delivery(
                    agent_id,
                    ct_str,
                    &message.sender.platform_id,
                    true,
                    None,
                    thread_id,
                )
                .await;
            if let Some(j) = journal {
                j.record_outcome(&message.platform_message_id, true, None)
                    .await;
            }
        }
        Err(e) => {
            let sender_ctx_retry = sender_ctx.clone();
            handle_send_error(
                &e,
                agent_id,
                &channel_key,
                handle,
                router,
                adapter,
                &message.sender,
                msg_id,
                ct_str,
                thread_id,
                output_format,
                overrides.as_ref(),
                |new_id| {
                    let h = handle.clone();
                    let t = text.clone();
                    async move {
                        h.send_message_with_sender(new_id, &t, &sender_ctx_retry)
                            .await
                    }
                },
            )
            .await;
            if let Some(j) = journal {
                j.record_outcome(&message.platform_message_id, false, Some(e.to_string()))
                    .await;
            }
        }
    }
}

/// Detect image format from the first few magic bytes.
///
/// Returns `Some("image/...")` for JPEG, PNG, GIF, and WebP.
fn detect_image_magic(bytes: &[u8]) -> Option<String> {
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg".to_string());
    }
    if bytes.len() >= 4 && bytes[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return Some("image/png".to_string());
    }
    if bytes.len() >= 4 && bytes[..4] == [0x47, 0x49, 0x46, 0x38] {
        return Some("image/gif".to_string());
    }
    if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return Some("image/webp".to_string());
    }
    None
}

/// Detect audio format from the first few magic bytes.
///
/// Returns `Some("audio/...")` for OGG, MP3, WAV, FLAC, M4A, and WebM/Matroska.
/// Used to recover a correct MIME type when the HTTP Content-Type header is
/// the uninformative `application/octet-stream` (common with Telegram CDN).
fn detect_audio_magic(bytes: &[u8]) -> Option<&'static str> {
    // OGG container — covers Opus (.oga/.opus), Vorbis, etc.
    if bytes.len() >= 4 && bytes[..4] == [0x4F, 0x67, 0x67, 0x53] {
        return Some("audio/ogg");
    }
    // MP3: ID3 tag header
    if bytes.len() >= 3 && bytes[..3] == [0x49, 0x44, 0x33] {
        return Some("audio/mpeg");
    }
    // MP3: sync word (0xFF 0xEx or 0xFF 0xFx) with valid MPEG version/layer bits.
    // Byte 1 encodes: sync(3 bits) | version(2) | layer(2) | crc(1).
    // Reject version=00 (reserved) and layer=00 (reserved) to reduce false positives.
    // Valid second bytes: 0xF2/0xF3 (MPEG-2), 0xFA/0xFB/0xF2/0xF3/0xE2/0xE3 (various).
    // Simplified: require byte[0]==0xFF, upper nibble of byte[1] is 0xF or 0xE,
    // version bits != 01 (reserved), layer bits != 00 (reserved).
    if bytes.len() >= 2 && bytes[0] == 0xFF {
        let b1 = bytes[1];
        // Upper nibble must be 0xE or 0xF (sync continuation)
        if b1 & 0xE0 == 0xE0 {
            let version = (b1 >> 3) & 0x03; // bits 4-3
            let layer = (b1 >> 1) & 0x03; // bits 2-1
            if version != 0x01 && layer != 0x00 {
                return Some("audio/mpeg");
            }
        }
    }
    // WAV: RIFF....WAVE
    if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x41, 0x56, 0x45]
    {
        return Some("audio/wav");
    }
    // FLAC
    if bytes.len() >= 4 && bytes[..4] == [0x66, 0x4C, 0x61, 0x43] {
        return Some("audio/flac");
    }
    // M4A / MP4 audio: ftyp box at offset 4 with a known audio-only brand.
    // Brands: "M4A " (iTunes), "M4B " (audiobook), "mp42", "mp41", "isom", "dash".
    if bytes.len() >= 12 && bytes[4..8] == [0x66, 0x74, 0x79, 0x70] {
        let brand = &bytes[8..12];
        if brand == b"M4A "
            || brand == b"M4B "
            || brand == b"mp42"
            || brand == b"mp41"
            || brand == b"isom"
            || brand == b"dash"
        {
            return Some("audio/mp4");
        }
    }
    // WebM / Matroska: EBML magic — but this also matches video/webm.
    // Return None here and let filename-based detection resolve .weba → audio/webm.
    // (Returning audio/webm unconditionally would misclassify video files.)
    if bytes.len() >= 4 && bytes[..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return None;
    }
    None
}

/// Infer an audio MIME type from a filename extension.
///
/// Returns `Some("audio/...")` for known audio extensions, `None` otherwise.
/// Used as a fallback when magic-byte detection is inconclusive.
fn audio_mime_from_filename(filename: &str) -> Option<&'static str> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".ogg") || lower.ends_with(".oga") || lower.ends_with(".opus") {
        Some("audio/ogg")
    } else if lower.ends_with(".mp3") {
        Some("audio/mpeg")
    } else if lower.ends_with(".wav") {
        Some("audio/wav")
    } else if lower.ends_with(".flac") {
        Some("audio/flac")
    } else if lower.ends_with(".m4a") {
        Some("audio/mp4")
    } else if lower.ends_with(".webm") {
        Some("audio/webm")
    } else {
        None
    }
}

/// Guess image media type from the URL file extension.
fn media_type_from_url(url: &str) -> String {
    if url.contains(".png") {
        "image/png".to_string()
    } else if url.contains(".gif") {
        "image/gif".to_string()
    } else if url.contains(".webp") {
        "image/webp".to_string()
    } else {
        // JPEG is the most common image format — safe default
        "image/jpeg".to_string()
    }
}

/// Default max bytes for file downloads when the bridge has no config (50 MB).
/// Keep in sync with `default_file_download_max_bytes` in `librefang-types`.
const CHANNEL_FILE_DOWNLOAD_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Prefix string for a successfully saved non-image file block.
/// Used both by `download_file_to_blocks` to produce the text and by
/// `dispatch_message` to detect success vs failure.
const FILE_SAVED_BLOCK_PREFIX: &str = "[File: ";

/// Returns `true` when [`download_file_to_blocks`] produced a block that
/// represents a successfully saved download — either an inline `ImageFile`
/// (when the response was image-typed) or a `Text` block whose content
/// starts with [`FILE_SAVED_BLOCK_PREFIX`] (the canonical save-success
/// marker).
///
/// All four media-download arms in `dispatch_message` (File, Voice, Audio,
/// Video) use this single check so any future change to the success
/// representation lands in one place. The check is intentionally broad:
/// even when a non-image arm (Voice/Audio/Video) receives an image-typed
/// response, the bytes are already on disk and the agent should still
/// receive the dispatched block — falling through to the text fallback
/// here would orphan the saved file.
fn has_file_saved_block(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|b| match b {
        ContentBlock::ImageFile { .. } => true,
        ContentBlock::Text { text, .. } => text.starts_with(FILE_SAVED_BLOCK_PREFIX),
        _ => false,
    })
}

/// Extract a basename-style filename from the path component of a URL.
///
/// Returns `None` when the URL is unparseable, has no path basename, or the
/// basename collapses to empty after trimming. Query/fragment portions are
/// dropped. Used by the voice/file dispatch path to derive a stable filename
/// for the on-disk saved copy when the channel didn't provide one.
fn filename_from_url(url: &str) -> Option<String> {
    let parsed = ::url::Url::parse(url).ok()?;
    let last = parsed.path_segments()?.next_back()?;
    let trimmed = last.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Sanitize a file extension to alphanumeric characters only.
///
/// Strips everything that isn't ASCII alphanumeric. Returns `"bin"` when the
/// result would be empty.
fn sanitize_extension(ext: &str) -> String {
    let cleaned: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if cleaned.is_empty() {
        "bin".to_string()
    } else {
        cleaned.to_lowercase()
    }
}

/// Validate that a URL is safe for the daemon to fetch on behalf of an
/// inbound channel message (#3442).
///
/// Delegates to [`crate::http_client::validate_url_for_fetch`], which
/// enforces:
/// * `http`/`https` scheme only — rejects `file://`, `ftp://`,
///   `javascript:`, `data:`, etc.
/// * No IPv4/IPv6 literal in any private, loopback, link-local,
///   unique-local, multicast, reserved, or cloud-metadata range —
///   including the IPv4-mapped (`::ffff:x.x.x.x`) and NAT64
///   (`64:ff9b::x.x.x.x`) wire-equivalent forms.
/// * No internal hostname (`localhost`, `*.local`,
///   `metadata.google.internal`, `169.254.169.254`).
///
/// Without this guard, a forged inbound message containing
/// `attachment.url = "http://169.254.169.254/latest/meta-data/..."`
/// or `"http://127.0.0.1:4545/api/agents"` would have its body fetched
/// and base64'd into the agent's LLM context.
fn validate_url_scheme(url: &str) -> Result<(), String> {
    crate::http_client::validate_url_for_fetch(url)
}

/// Download a file from a URL to disk with streaming and size cap.
///
/// Returns `ContentBlock::ImageFile` on success (reuses the variant for all
/// downloaded files) or a text block describing the failure.
async fn download_file_to_blocks(
    url: &str,
    filename: &str,
    max_bytes: u64,
    download_dir: &std::path::Path,
    extra_headers: &[(String, String)],
) -> Vec<ContentBlock> {
    // Validate URL scheme
    if let Err(reason) = validate_url_scheme(url) {
        warn!("{reason}");
        return vec![ContentBlock::Text {
            text: format!("[File download rejected: {reason}]"),
            provider_metadata: None,
        }];
    }

    let client = crate::http_client::new_client();
    let mut req = client.get(url).timeout(std::time::Duration::from_secs(60));
    for (name, value) in extra_headers {
        req = req.header(name.as_str(), value.as_str());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to download file from channel: {e}");
            return vec![ContentBlock::Text {
                text: format!("[File download failed: {e}]"),
                provider_metadata: None,
            }];
        }
    };

    // Fail closed on non-2xx. Without this the body of a 4xx/5xx (e.g.
    // Synapse's `M_NOT_FOUND` JSON, ~45 bytes) streams straight into
    // `<uuid>.<ext>` and the agent then sees a "PDF" that's actually an
    // error envelope.
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let preview: String = body.chars().take(200).collect();
        warn!(
            status = %status,
            body_preview = %preview,
            url = %url,
            "File download returned non-success status"
        );
        return vec![ContentBlock::Text {
            text: format!("[File download failed: HTTP {status} ({filename})]"),
            provider_metadata: None,
        }];
    }

    // Fast-reject via Content-Length header when available.
    if let Some(cl) = resp.content_length() {
        if cl > max_bytes {
            warn!(
                content_length = cl,
                max_bytes, "File exceeds size cap (Content-Length), skipping download"
            );
            return vec![ContentBlock::Text {
                text: format!(
                    "[File too large: {cl} bytes exceeds {max_bytes} byte limit ({filename})]"
                ),
                provider_metadata: None,
            }];
        }
    }

    // Detect media type from Content-Type header.
    let media_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Extract and sanitize extension from the original filename.
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(sanitize_extension)
        .unwrap_or_else(|| "bin".to_string());

    let dest_filename = format!("{}.{}", uuid::Uuid::new_v4(), ext);
    let file_path = download_dir.join(&dest_filename);

    // Ensure upload directory exists.
    if let Err(e) = tokio::fs::create_dir_all(download_dir).await {
        warn!(
            "Failed to create download dir {}: {e}",
            download_dir.display()
        );
        return vec![ContentBlock::Text {
            text: format!("[File download failed: cannot create directory: {e}]"),
            provider_metadata: None,
        }];
    }

    // Stream body to disk chunk by chunk, enforcing size cap.
    let mut stream = resp.bytes_stream();
    let mut file = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to create file {}: {e}", file_path.display());
            return vec![ContentBlock::Text {
                text: format!("[File download failed: {e}]"),
                provider_metadata: None,
            }];
        }
    };

    let mut total: u64 = 0;
    // Retain the first 12 bytes of the response body so we can sniff the audio
    // MIME type without a second read syscall (avoids sync IO in async context).
    let mut magic_buf = [0u8; 12];
    let mut magic_filled: usize = 0;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                total += chunk.len() as u64;
                if total > max_bytes {
                    warn!(
                        total_bytes = total,
                        max_bytes, "File download exceeded size cap, aborting"
                    );
                    drop(file);
                    let _ = tokio::fs::remove_file(&file_path).await;
                    return vec![ContentBlock::Text {
                        text: format!(
                            "[File too large: exceeded {max_bytes} byte limit ({filename})]"
                        ),
                        provider_metadata: None,
                    }];
                }
                // Fill magic buffer from the very first bytes of the stream.
                if magic_filled < magic_buf.len() {
                    let need = magic_buf.len() - magic_filled;
                    let take = need.min(chunk.len());
                    magic_buf[magic_filled..magic_filled + take].copy_from_slice(&chunk[..take]);
                    magic_filled += take;
                }
                if let Err(e) = file.write_all(&chunk).await {
                    warn!("Failed to write chunk to {}: {e}", file_path.display());
                    drop(file);
                    let _ = tokio::fs::remove_file(&file_path).await;
                    return vec![ContentBlock::Text {
                        text: format!("[File download failed: write error: {e}]"),
                        provider_metadata: None,
                    }];
                }
            }
            Err(e) => {
                warn!("Stream error downloading file: {e}");
                drop(file);
                let _ = tokio::fs::remove_file(&file_path).await;
                return vec![ContentBlock::Text {
                    text: format!("[File download failed: {e}]"),
                    provider_metadata: None,
                }];
            }
        }
    }

    if let Err(e) = file.flush().await {
        warn!("Failed to flush file {}: {e}", file_path.display());
    }

    // When the Content-Type header was uninformative (application/octet-stream
    // or absent — common with Telegram and S3 CDNs), attempt to recover the
    // real MIME type so the kernel STT pipeline fires correctly:
    //   1. Magic-byte sniff from the bytes already buffered during streaming
    //      (no extra read syscall — avoids blocking sync IO in async context).
    //   2. Fall back to filename extension.
    //   3. Keep application/octet-stream only when both are inconclusive.
    let media_type = if media_type == "application/octet-stream" {
        let sniffed_magic = detect_audio_magic(&magic_buf[..magic_filled]).map(str::to_string);
        let sniffed_name = audio_mime_from_filename(filename).map(str::to_string);

        // Log when magic and filename hint disagree so operators can debug
        // files that land with the wrong MIME.
        if let (Some(ref magic_mime), Some(ref name_mime)) = (&sniffed_magic, &sniffed_name) {
            if magic_mime != name_mime {
                debug!(
                    sniffed_mime = %magic_mime,
                    filename_mime = %name_mime,
                    filename = %filename,
                    "audio MIME source disagreement: magic-bytes and filename extension differ; \
                     using magic-bytes result"
                );
            }
        }

        // Magic bytes take precedence; filename is the fallback.
        sniffed_magic.or(sniffed_name).unwrap_or(media_type)
    } else {
        media_type
    };

    // Probabilistic cleanup — avoids unbounded disk growth between restarts.
    // Triggers on ~1/256 downloads without a rand dependency.
    if std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        .is_multiple_of(256)
    {
        let sweep_dir = download_dir.to_path_buf();
        tokio::spawn(async move { cleanup_old_uploads(&sweep_dir).await });
    }

    info!(
        path = %file_path.display(),
        size_bytes = total,
        media_type = %media_type,
        original_filename = %filename,
        "Downloaded channel file to disk"
    );

    let path_str = file_path.to_string_lossy().into_owned();
    if media_type.starts_with("image/") {
        vec![ContentBlock::ImageFile {
            media_type,
            path: path_str,
        }]
    } else {
        // Content-aware enrichment (#4448): when the file is a PDF or a
        // text-like format, surface its actual content to the LLM in
        // addition to the saved-path block. The path block is preserved
        // so tools that legitimately want raw bytes (media_transcribe,
        // custom file readers) still work.
        let mut blocks =
            crate::attachment_enrich::enrich_saved_file(&file_path, &media_type, filename);
        blocks.push(ContentBlock::Text {
            text: format!("{FILE_SAVED_BLOCK_PREFIX}{filename}] saved to {path_str}"),
            provider_metadata: None,
        });
        blocks
    }
}

/// Remove files older than 24 hours from the upload/download directory.
///
/// Called on bridge startup to prevent unbounded disk growth.
async fn cleanup_old_uploads(dir: &std::path::Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 60 * 60);
    let mut removed = 0u64;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff && tokio::fs::remove_file(entry.path()).await.is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        info!(removed, dir = %dir.display(), "Cleaned up old upload files");
    }
}

/// Download an image from a URL and build content blocks for multimodal LLM input.
///
/// Returns a `Vec<ContentBlock>` containing an image block (base64-encoded) and
/// optionally a text block for the caption. If the download fails, returns a
/// text-only block describing the failure.
///
/// `mime_type_hint` is an optional MIME type pre-detected by the channel adapter
/// (e.g. from a Telegram file path). When present it takes priority over the
/// HTTP Content-Type header because many APIs return `application/octet-stream`.
async fn download_image_to_blocks(
    url: &str,
    caption: Option<&str>,
    mime_type_hint: Option<&str>,
    upload_dir: &std::path::Path,
    extra_headers: &[(String, String)],
) -> Vec<ContentBlock> {
    use base64::Engine;

    // 5 MB limit to prevent memory abuse from oversized images
    const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

    // SSRF guard (#3442) + size cap (5 MiB, in-memory) + Content-Type
    // capture, all behind one helper. The helper rejects non-http(s)
    // schemes and any host literally in a private/loopback/metadata
    // range BEFORE opening a socket — so a forged
    // `http://169.254.169.254/...` never produces an "image" block in
    // the agent's LLM context. The size cap enforces both a
    // Content-Length pre-check and a streaming-accumulator mid-fetch
    // bound, so a chunked-transfer "lying" length cannot bypass it.
    //
    // `extra_headers` is threaded through to attach auth (MSC3916
    // Bearer for Matrix's authenticated media path); the adapter has
    // already gated the URL host before producing the headers — see
    // `ChannelAdapter::fetch_headers_for` for the credential-leak
    // contract.
    let (buf, response_content_type) =
        match crate::http_client::fetch_url_bytes(url, MAX_IMAGE_BYTES, extra_headers).await {
            Ok(t) => t,
            Err(crate::http_client::FetchError::Rejected(reason)) => {
                warn!("Rejecting image download: {reason}");
                return vec![ContentBlock::Text {
                    text: format!("[Image download rejected: {reason}]"),
                    provider_metadata: None,
                }];
            }
            Err(crate::http_client::FetchError::TooLarge { actual, limit }) => {
                let reported_kb = actual
                    .map(|a| a / 1024)
                    .unwrap_or_else(|| (limit as u64) / 1024);
                match actual {
                    Some(len) => warn!(
                    "Image Content-Length ({len} bytes) exceeds limit, rejecting before download"
                ),
                    None => warn!("Image stream exceeded {limit} byte limit, aborting download"),
                }
                let desc = match caption {
                    Some(c) => {
                        format!("[Image too large for vision ({reported_kb} KB)]\nCaption: {c}")
                    }
                    None => format!("[Image too large for vision ({reported_kb} KB)]"),
                };
                return vec![ContentBlock::Text {
                    text: desc,
                    provider_metadata: None,
                }];
            }
            Err(crate::http_client::FetchError::Failed(reason)) => {
                warn!("Image download failed: {reason}");
                return vec![ContentBlock::Text {
                    text: format!("[Image download failed: {reason}]"),
                    provider_metadata: None,
                }];
            }
        };

    // Detect media type from Content-Type header — but only trust it if it's
    // actually an image/* type. Many APIs (Telegram, S3 pre-signed URLs) return
    // `application/octet-stream` for all files, which breaks vision.
    let header_type = response_content_type
        .as_deref()
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string())
        .filter(|ct| ct.starts_with("image/"));

    let bytes = bytes::Bytes::from(buf);

    // Four-tier media type detection:
    // 1. Adapter-provided hint (e.g. Telegram file path extension) — most
    //    reliable because many APIs return application/octet-stream in headers
    // 2. Trusted Content-Type header (only if image/*)
    // 3. Magic byte sniffing (most reliable for binary data)
    // 4. URL extension fallback
    let media_type = mime_type_hint
        .map(|s| s.to_string())
        .or(header_type)
        .unwrap_or_else(|| detect_image_magic(&bytes).unwrap_or_else(|| media_type_from_url(url)));

    // Downscale large images so batches of many photos fit within the LLM
    // context window.  Max dimension 1024px keeps enough detail for analysis
    // while reducing a 3 MB photo to ~80-150 KB of JPEG.
    const MAX_DIMENSION: u32 = 1024;
    const DOWNSCALE_THRESHOLD: usize = 200 * 1024; // only resize if > 200 KB
    let final_bytes: Vec<u8>;
    let final_media_type: String;
    if bytes.len() > DOWNSCALE_THRESHOLD {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let resized = img.resize(
                    MAX_DIMENSION,
                    MAX_DIMENSION,
                    image::imageops::FilterType::Triangle,
                );
                let mut buf = std::io::Cursor::new(Vec::new());
                if resized.write_to(&mut buf, image::ImageFormat::Jpeg).is_ok() {
                    final_bytes = buf.into_inner();
                    final_media_type = "image/jpeg".to_string();
                    tracing::debug!(
                        original_kb = bytes.len() / 1024,
                        resized_kb = final_bytes.len() / 1024,
                        "Downscaled image for LLM context budget"
                    );
                } else {
                    final_bytes = bytes.to_vec();
                    final_media_type = media_type;
                }
            }
            Err(_) => {
                // Can't decode (e.g. exotic format) — send as-is
                final_bytes = bytes.to_vec();
                final_media_type = media_type;
            }
        }
    } else {
        final_bytes = bytes.to_vec();
        final_media_type = media_type;
    }

    let mut blocks = Vec::new();

    // Caption as text block first (gives the LLM context about the image)
    if let Some(cap) = caption {
        if !cap.is_empty() {
            blocks.push(ContentBlock::Text {
                text: cap.to_string(),
                provider_metadata: None,
            });
        }
    }

    // Save image to disk instead of base64-encoding into the session.
    // A 3 MB photo becomes ~100 KB on disk with only a short path in the session.
    let ext = match final_media_type.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    };

    // Ensure upload directory exists (BRDG-04)
    if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
        warn!("Failed to create upload dir {}: {e}", upload_dir.display());
        // Fallback to base64 inline encoding
        let data = base64::engine::general_purpose::STANDARD.encode(&final_bytes);
        blocks.push(ContentBlock::Image {
            media_type: final_media_type,
            data,
        });
        return blocks;
    }
    // Restrict directory permissions to owner-only on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            tokio::fs::set_permissions(&upload_dir, std::fs::Permissions::from_mode(0o700)).await
        {
            warn!("Failed to set permissions on {}: {e}", upload_dir.display());
        }
    }

    let filename = format!("{}.{}", uuid::Uuid::new_v4(), ext);
    let file_path = upload_dir.join(&filename);

    // Save image to disk (BRDG-01)
    match tokio::fs::write(&file_path, &final_bytes).await {
        Ok(()) => {
            tracing::debug!(
                path = %file_path.display(),
                size_kb = final_bytes.len() / 1024,
                "Saved channel image to disk"
            );
            // Return ImageFile with absolute path (BRDG-02)
            blocks.push(ContentBlock::ImageFile {
                media_type: final_media_type,
                path: file_path.to_string_lossy().into_owned(),
            });
        }
        Err(e) => {
            warn!(
                "Failed to write image to {}: {e} — falling back to base64",
                file_path.display()
            );
            let data = base64::engine::general_purpose::STANDARD.encode(&final_bytes);
            blocks.push(ContentBlock::Image {
                media_type: final_media_type,
                data,
            });
        }
    }

    blocks
}

/// Dispatch a multimodal message (content blocks) to an agent, handling routing
/// and RBAC the same way as the text path.
#[allow(clippy::too_many_arguments)]
async fn dispatch_with_blocks(
    blocks: Vec<ContentBlock>,
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    ct_str: &str,
    thread_id: Option<&str>,
    output_format: OutputFormat,
    overrides: Option<&ChannelOverrides>,
    journal: Option<&crate::message_journal::MessageJournal>,
    thread_ownership: &Arc<crate::thread_ownership::ThreadOwnershipRegistry>,
) {
    let agent_id = match resolve_or_fallback(message, handle, router).await {
        Some(id) => id,
        None => {
            send_response(
                adapter,
                &message.sender,
                "No agents available. Start the dashboard at http://127.0.0.1:4545 to create one."
                    .to_string(),
                thread_id,
                output_format,
            )
            .await;
            return;
        }
    };

    // Thread-ownership gate (#3334). Mirrors the text-path check in
    // `dispatch_message`. Multimodal messages may not include a
    // platform-level @-mention marker; treat absence as "no override".
    if message.is_group
        && overrides
            .map(|o| o.thread_ownership_enabled)
            .unwrap_or(true)
    {
        if let Some(thread_str) = message.thread_id.as_deref() {
            if let Some(key) = crate::thread_ownership::ThreadKey::new(ct_str, thread_str) {
                let was_mentioned = message
                    .metadata
                    .get("was_mentioned")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match thread_ownership.decide(key, agent_id, was_mentioned) {
                    crate::thread_ownership::DispatchDecision::Allow { .. } => {}
                    crate::thread_ownership::DispatchDecision::Suppress { holder } => {
                        debug!(
                            channel = ct_str,
                            thread_id = thread_str,
                            candidate = %agent_id,
                            holder = %holder,
                            "thread_ownership: suppressing block dispatch — another agent owns this thread"
                        );
                        return;
                    }
                }
            }
        }
    }

    let channel_key = channel_type_str(&message.channel).to_string();

    // RBAC check
    if let Err(denied) = handle
        .authorize_channel_user(ct_str, &message.sender.platform_id, "chat")
        .await
    {
        send_response(
            adapter,
            &message.sender,
            format!("Access denied: {denied}"),
            thread_id,
            output_format,
        )
        .await;
        return;
    }

    // --- Message journal: record before dispatch for crash recovery ---
    if let Some(j) = journal {
        let text = content_to_text(&message.content);
        let entry = crate::message_journal::JournalEntry {
            message_id: message.platform_message_id.clone(),
            channel: ct_str.to_string(),
            sender_id: message.sender.platform_id.clone(),
            sender_name: message.sender.display_name.clone(),
            content: text,
            agent_name: None,
            received_at: message.timestamp,
            status: crate::message_journal::JournalStatus::Processing,
            attempts: 0,
            last_error: None,
            updated_at: chrono::Utc::now(),
            is_group: message.is_group,
            thread_id: thread_id.map(|s| s.to_string()),
            metadata: std::collections::HashMap::new(),
            next_retry_after: None,
        };
        j.record(entry).await;
    }

    if let Err(e) = adapter.send_typing(&message.sender).await {
        debug!(adapter = adapter.name(), error = %e, "send_typing failed (best-effort)");
    }

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Thinking).await;

    upsert_sender_into_roster(handle, message).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message, overrides);

    match handle
        .send_message_with_blocks_and_sender(agent_id, blocks.clone(), &sender_ctx)
        .await
    {
        Ok(response) => {
            send_lifecycle_reaction(adapter, &message.sender, msg_id, &AgentPhase::Done).await;
            if !response.is_empty() {
                let response = maybe_prefix_response(handle, overrides, agent_id, response).await;
                send_response(adapter, &message.sender, response, thread_id, output_format).await;
            }
            if let Some(j) = journal {
                j.record_outcome(&message.platform_message_id, true, None)
                    .await;
            }
            handle
                .record_delivery(
                    agent_id,
                    ct_str,
                    &message.sender.platform_id,
                    true,
                    None,
                    thread_id,
                )
                .await;
        }
        Err(e) => {
            let sender_ctx_retry = sender_ctx.clone();
            handle_send_error(
                &e,
                agent_id,
                &channel_key,
                handle,
                router,
                adapter,
                &message.sender,
                msg_id,
                ct_str,
                thread_id,
                output_format,
                overrides,
                |new_id| {
                    let h = handle.clone();
                    async move {
                        h.send_message_with_blocks_and_sender(new_id, blocks, &sender_ctx_retry)
                            .await
                    }
                },
            )
            .await;
            if let Some(j) = journal {
                j.record_outcome(&message.platform_message_id, false, Some(e.to_string()))
                    .await;
            }
        }
    }
}

/// Handle a bot command (returns the response text).
///
/// `overrides` reflects the merged agent + channel policy for the calling
/// context. It currently affects `/help` rendering (so disabled/blocked
/// commands don't appear in the help text); other branches treat it as
/// advisory.
async fn handle_command(
    name: &str,
    args: &[String],
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    sender: &ChannelUser,
    channel_type: &crate::types::ChannelType,
    overrides: Option<&ChannelOverrides>,
) -> String {
    match name {
        "start" => {
            let agents = handle.list_agents().await.unwrap_or_default();
            let mut msg =
                "Welcome to LibreFang! I connect you to AI agents.\n\nAvailable agents:\n"
                    .to_string();
            if agents.is_empty() {
                msg.push_str("  (none running)\n");
            } else {
                for (_, name) in &agents {
                    msg.push_str(&format!("  - {name}\n"));
                }
            }
            msg.push_str("\nCommands:\n/agents - list agents\n/agent <name> - select an agent\n/help - show this help");
            msg
        }
        "help" => crate::commands::channel_help_text(overrides),
        "status" => handle.uptime_info().await,
        "agents" => {
            let agents = handle.list_agents().await.unwrap_or_default();
            if agents.is_empty() {
                "No agents running.".to_string()
            } else {
                let mut msg = "Running agents:\n".to_string();
                for (_, name) in &agents {
                    msg.push_str(&format!("  - {name}\n"));
                }
                msg
            }
        }
        "agent" => {
            if args.is_empty() {
                return "Usage: /agent <name>".to_string();
            }
            let agent_name = &args[0];
            match handle.find_agent_by_name(agent_name).await {
                Ok(Some(agent_id)) => {
                    router.set_user_default(sender.platform_id.clone(), agent_id);
                    format!("Now talking to agent: {agent_name}")
                }
                Ok(None) => {
                    // Try to spawn it
                    match handle.spawn_agent_by_name(agent_name).await {
                        Ok(agent_id) => {
                            router.set_user_default(sender.platform_id.clone(), agent_id);
                            format!("Spawned and connected to agent: {agent_name}")
                        }
                        Err(e) => {
                            format!("Agent '{agent_name}' not found and could not spawn: {e}")
                        }
                    }
                }
                Err(e) => format!("Error finding agent: {e}"),
            }
        }
        "btw" => {
            if args.is_empty() {
                return "Usage: /btw <question> — ask a side question without affecting session history".to_string();
            }
            let question = args.join(" ");
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => handle
                    .send_message_ephemeral(aid, &question)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "new" => {
            // Resolve the user's current agent and the channel-derived sid
            // so /new only resets THIS chat (#4868). The (channel, chat_id)
            // pair must match `build_sender_context` exactly so the sid we
            // delete here equals the sid the next inbound message will
            // resolve via `SessionId::for_channel`.
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => {
                    let ch = channel_type_str(channel_type);
                    let chat = if sender.platform_id.is_empty() {
                        None
                    } else {
                        Some(sender.platform_id.as_str())
                    };
                    handle
                        .reset_channel_session(aid, ch, chat)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}"))
                }
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "reboot" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => {
                    let ch = channel_type_str(channel_type);
                    let chat = if sender.platform_id.is_empty() {
                        None
                    } else {
                        Some(sender.platform_id.as_str())
                    };
                    handle
                        .reboot_channel_session(aid, ch, chat)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}"))
                }
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "compact" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => {
                    let ch = channel_type_str(channel_type);
                    let chat = if sender.platform_id.is_empty() {
                        None
                    } else {
                        Some(sender.platform_id.as_str())
                    };
                    handle
                        .compact_channel_session(aid, ch, chat)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}"))
                }
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "model" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => {
                    if args.is_empty() {
                        // Show current model
                        handle
                            .set_model(aid, "")
                            .await
                            .unwrap_or_else(|e| format!("Error: {e}"))
                    } else {
                        handle
                            .set_model(aid, &args[0])
                            .await
                            .unwrap_or_else(|e| format!("Error: {e}"))
                    }
                }
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "stop" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => handle
                    .stop_run(aid)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "usage" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => handle
                    .session_usage(aid)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "think" => {
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => {
                    let on = args.first().map(|a| a == "on").unwrap_or(true);
                    handle
                        .set_thinking(aid, on)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}"))
                }
                None => "No agent selected. Use /agent <name> first.".to_string(),
            }
        }
        "models" => handle.list_models_text().await,
        "providers" => handle.list_providers_text().await,
        "skills" => handle.list_skills_text().await,
        "hands" => handle.list_hands_text().await,

        // ── Automation: workflows, triggers, schedules, approvals ──
        "workflows" => handle.list_workflows_text().await,
        "workflow" => {
            if args.len() >= 2 && args[0] == "run" {
                let wf_name = &args[1];
                let input = if args.len() > 2 {
                    args[2..].join(" ")
                } else {
                    String::new()
                };
                handle.run_workflow_text(wf_name, &input).await
            } else {
                "Usage: /workflow run <name> [input]".to_string()
            }
        }
        "triggers" => handle.list_triggers_text().await,
        "trigger" => {
            if args.len() >= 4 && args[0] == "add" {
                let agent_name = &args[1];
                let pattern = &args[2];
                let prompt = args[3..].join(" ");
                handle
                    .create_trigger_text(agent_name, pattern, &prompt)
                    .await
            } else if args.len() >= 2 && args[0] == "del" {
                handle.delete_trigger_text(&args[1]).await
            } else {
                "Usage:\n  /trigger add <agent> <pattern> <prompt>\n  /trigger del <id-prefix>"
                    .to_string()
            }
        }
        "schedules" => handle.list_schedules_text().await,
        "schedule" => {
            if args.is_empty() {
                return "Usage:\n  /schedule add <agent> <cron-5-fields> <message>\n  /schedule del <id-prefix>\n  /schedule run <id-prefix>".to_string();
            }
            let action = args[0].as_str();
            match action {
                "add" | "del" | "run" => {
                    handle.manage_schedule_text(action, &args[1..]).await
                }
                _ => "Usage:\n  /schedule add <agent> <cron-5-fields> <message>\n  /schedule del <id-prefix>\n  /schedule run <id-prefix>".to_string(),
            }
        }
        "approvals" => handle.list_approvals_text().await,
        "approve" => {
            if args.is_empty() {
                "Usage: /approve <id-prefix> [totp-code]".to_string()
            } else {
                let totp_code = args.get(1).map(|s| s.as_str());
                handle
                    .resolve_approval_text(&args[0], true, totp_code, &sender.platform_id)
                    .await
            }
        }
        "reject" => {
            if args.is_empty() {
                "Usage: /reject <id-prefix>".to_string()
            } else {
                handle
                    .resolve_approval_text(&args[0], false, None, &sender.platform_id)
                    .await
            }
        }

        // ── Budget, Network, A2A ──
        "budget" => handle.budget_text().await,
        "peers" => handle.peers_text().await,
        "a2a" => handle.a2a_agents_text().await,

        _ => format!("Unknown command: /{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChannelType;
    use std::sync::Mutex;

    /// Serialize every test in this module that reads OR writes
    /// `LIBREFANG_GROUP_ADDRESSEE_GUARD`. The nested
    /// `should_process_group_message_v2` module has its own copy of this
    /// pattern for its tests; without serialization at this level too,
    /// `test_mention_only_*` tests that live in the outer module flake
    /// under parallel execution — they read the env var through
    /// `addressee_guard_enabled()` while v2 tests concurrently mutate
    /// it, and occasionally see `guard=on` when they expect the default.
    pub(super) static ADDRESSEE_GUARD_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the env lock and clear the guard var for the duration of
    /// the test so reads return `false` deterministically. Intended for
    /// tests that assume the default (guard-off) behavior.
    pub(super) fn with_guard_off_locked<F: FnOnce()>(f: F) {
        let _g = ADDRESSEE_GUARD_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("LIBREFANG_GROUP_ADDRESSEE_GUARD");
        f();
    }

    #[test]
    fn test_is_command_allowed_default_allows_everything() {
        // No overrides configured — all commands allowed (current behaviour).
        assert!(is_command_allowed("agent", None));
        assert!(is_command_allowed("new", None));

        // Explicit default overrides also allow everything.
        let ov = ChannelOverrides::default();
        assert!(is_command_allowed("agent", Some(&ov)));
        assert!(is_command_allowed("reboot", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_disable_commands_blocks_all() {
        let ov = ChannelOverrides {
            disable_commands: true,
            ..Default::default()
        };
        assert!(!is_command_allowed("start", Some(&ov)));
        assert!(!is_command_allowed("help", Some(&ov)));
        assert!(!is_command_allowed("agent", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_whitelist() {
        let ov = ChannelOverrides {
            allowed_commands: vec!["start".into(), "help".into()],
            ..Default::default()
        };
        assert!(is_command_allowed("start", Some(&ov)));
        assert!(is_command_allowed("help", Some(&ov)));
        assert!(!is_command_allowed("agent", Some(&ov)));
        assert!(!is_command_allowed("new", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_blacklist() {
        let ov = ChannelOverrides {
            blocked_commands: vec!["agent".into(), "new".into(), "reboot".into()],
            ..Default::default()
        };
        assert!(!is_command_allowed("agent", Some(&ov)));
        assert!(!is_command_allowed("new", Some(&ov)));
        assert!(!is_command_allowed("reboot", Some(&ov)));
        assert!(is_command_allowed("help", Some(&ov)));
        assert!(is_command_allowed("start", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_precedence_disable_over_allow() {
        // disable_commands trumps a whitelist.
        let ov = ChannelOverrides {
            disable_commands: true,
            allowed_commands: vec!["start".into()],
            ..Default::default()
        };
        assert!(!is_command_allowed("start", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_precedence_allow_over_block() {
        // Whitelist takes precedence over blacklist when both set.
        let ov = ChannelOverrides {
            allowed_commands: vec!["agent".into()],
            blocked_commands: vec!["agent".into(), "help".into()],
            ..Default::default()
        };
        assert!(is_command_allowed("agent", Some(&ov)));
        // `help` is not in the whitelist — blocked even though not via blocklist.
        assert!(!is_command_allowed("help", Some(&ov)));
    }

    #[test]
    fn test_is_command_allowed_tolerates_leading_slash_in_config() {
        // Users may write either "agent" or "/agent" in TOML — both should work.
        let ov = ChannelOverrides {
            allowed_commands: vec!["/start".into(), "help".into()],
            ..Default::default()
        };
        assert!(is_command_allowed("start", Some(&ov)));
        assert!(is_command_allowed("help", Some(&ov)));
        assert!(!is_command_allowed("agent", Some(&ov)));

        let ov = ChannelOverrides {
            blocked_commands: vec!["/agent".into(), "new".into()],
            ..Default::default()
        };
        assert!(!is_command_allowed("agent", Some(&ov)));
        assert!(!is_command_allowed("new", Some(&ov)));
        assert!(is_command_allowed("help", Some(&ov)));
    }

    #[test]
    fn test_reconstruct_command_text() {
        assert_eq!(reconstruct_command_text("help", &[]), "/help");
        assert_eq!(
            reconstruct_command_text("agent", &["admin".into()]),
            "/agent admin"
        );
        assert_eq!(
            reconstruct_command_text(
                "workflow",
                &["run".into(), "pipeline-1".into(), "hello".into()]
            ),
            "/workflow run pipeline-1 hello"
        );
    }

    /// Mock kernel handle for testing.
    struct MockHandle {
        agents: Mutex<Vec<(AgentId, String)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockHandle {
        async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
            Ok(format!("Echo: {message}"))
        }
        async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
            let agents = self.agents.lock().unwrap();
            Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
        }
        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(self.agents.lock().unwrap().clone())
        }
        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn not implemented in mock".to_string())
        }
        fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
            // Test mock: no event bus to forward to.
        }
    }

    /// Helper: replicate the metadata read + key build the bridge does, then
    /// ask the registry. Exercises the same logic `dispatch_message` runs
    /// without standing up the full channel handle / adapter mocks.
    fn bridge_thread_ownership_decision(
        registry: &crate::thread_ownership::ThreadOwnershipRegistry,
        message: &ChannelMessage,
        ct_str: &str,
        candidate: AgentId,
        thread_ownership_enabled: bool,
    ) -> Option<crate::thread_ownership::DispatchDecision> {
        if !message.is_group || !thread_ownership_enabled {
            return None;
        }
        let thread_str = message.thread_id.as_deref()?;
        let key = crate::thread_ownership::ThreadKey::new(ct_str, thread_str)?;
        let was_mentioned = message
            .metadata
            .get("was_mentioned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Some(registry.decide(key, candidate, was_mentioned))
    }

    fn group_thread_message(thread: &str, was_mentioned: bool) -> ChannelMessage {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "was_mentioned".to_string(),
            serde_json::json!(was_mentioned),
        );
        ChannelMessage {
            channel: ChannelType::Slack,
            platform_message_id: "1".into(),
            sender: ChannelUser {
                platform_id: "u1".into(),
                display_name: "user".into(),
                librefang_user: None,
            },
            content: ChannelContent::Text("hi".into()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: true,
            thread_id: Some(thread.into()),
            metadata,
        }
    }

    #[test]
    fn dm_messages_bypass_thread_ownership_check() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let mut msg = group_thread_message("T1", false);
        msg.is_group = false; // DM
        let alice = AgentId::new();
        assert!(
            bridge_thread_ownership_decision(&registry, &msg, "slack", alice, true).is_none(),
            "DM messages must skip the ownership check entirely"
        );
    }

    #[test]
    fn group_message_without_thread_id_bypasses_check() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let mut msg = group_thread_message("T1", false);
        msg.thread_id = None; // group but untreaded
        let alice = AgentId::new();
        assert!(
            bridge_thread_ownership_decision(&registry, &msg, "slack", alice, true).is_none(),
            "Untreaded group messages must skip the registry"
        );
    }

    #[test]
    fn group_thread_first_dispatch_allows_and_claims() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let msg = group_thread_message("T1", false);
        let alice = AgentId::new();
        let decision =
            bridge_thread_ownership_decision(&registry, &msg, "slack", alice, true).unwrap();
        match decision {
            crate::thread_ownership::DispatchDecision::Allow { agent_id } => {
                assert_eq!(agent_id, alice);
            }
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn group_thread_second_agent_no_mention_is_suppressed() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let msg = group_thread_message("T1", false);
        let alice = AgentId::new();
        let bob = AgentId::new();
        let _ = bridge_thread_ownership_decision(&registry, &msg, "slack", alice, true);
        let decision =
            bridge_thread_ownership_decision(&registry, &msg, "slack", bob, true).unwrap();
        assert!(matches!(
            decision,
            crate::thread_ownership::DispatchDecision::Suppress { .. }
        ));
    }

    #[test]
    fn group_thread_at_mention_lets_second_agent_take_over() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let alice = AgentId::new();
        let bob = AgentId::new();
        let _ = bridge_thread_ownership_decision(
            &registry,
            &group_thread_message("T1", false),
            "slack",
            alice,
            true,
        );
        let mention_msg = group_thread_message("T1", true);
        let decision =
            bridge_thread_ownership_decision(&registry, &mention_msg, "slack", bob, true).unwrap();
        match decision {
            crate::thread_ownership::DispatchDecision::Allow { agent_id } => {
                assert_eq!(agent_id, bob, "@-mention must re-claim for the new agent");
            }
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn channel_override_thread_ownership_disabled_bypasses_check() {
        let registry = crate::thread_ownership::ThreadOwnershipRegistry::new();
        let alice = AgentId::new();
        let bob = AgentId::new();
        // First call with the feature enabled claims for alice.
        let _ = bridge_thread_ownership_decision(
            &registry,
            &group_thread_message("T1", false),
            "slack",
            alice,
            true,
        );
        // Now bob arrives with the per-channel feature disabled — the bridge
        // skips the registry entirely.
        let decision = bridge_thread_ownership_decision(
            &registry,
            &group_thread_message("T1", false),
            "slack",
            bob,
            false,
        );
        assert!(
            decision.is_none(),
            "thread_ownership_enabled = false must bypass the registry"
        );
    }

    #[test]
    fn test_command_parsing() {
        // Verify slash commands are parsed correctly from text
        let text = "/agent hello-world";
        assert!(text.starts_with('/'));
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = &parts[0][1..];
        assert_eq!(cmd, "agent");
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        assert_eq!(args, vec!["hello-world"]);
    }

    #[tokio::test]
    async fn test_dispatch_routes_to_correct_agent() {
        let agent_id = AgentId::new();
        let mock = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "test-agent".to_string())]),
        });

        let handle: Arc<dyn ChannelBridgeHandle> = mock;

        // Verify find_agent_by_name works
        let found = handle.find_agent_by_name("test-agent").await.unwrap();
        assert_eq!(found, Some(agent_id));

        let not_found = handle.find_agent_by_name("nonexistent").await.unwrap();
        assert_eq!(not_found, None);

        // Verify send_message echoes
        let response = handle.send_message(agent_id, "hello").await.unwrap();
        assert_eq!(response, "Echo: hello");
    }

    #[tokio::test]
    async fn test_handle_command_agents() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            librefang_user: None,
        };

        let result = handle_command(
            "agents",
            &[],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("coder"));

        let result = handle_command(
            "help",
            &[],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("/agents"));
    }

    #[tokio::test]
    async fn test_handle_command_agent_select() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            librefang_user: None,
        };

        // Select existing agent
        let result = handle_command(
            "agent",
            &["coder".to_string()],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("Now talking to agent: coder"));

        // Verify router was updated
        let resolved = router.resolve(&ChannelType::Telegram, "user1", None);
        assert_eq!(resolved, Some(agent_id));
    }

    #[test]
    fn test_rate_limiter_allows_within_limit() {
        let limiter = ChannelRateLimiter::default();
        assert!(limiter.check("telegram", "user1", 5).is_ok());
        assert!(limiter.check("telegram", "user1", 5).is_ok());
        assert!(limiter.check("telegram", "user1", 5).is_ok());
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }
        // 4th should be blocked
        let result = limiter.check("telegram", "user1", 3);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Rate limit exceeded"));
    }

    #[test]
    fn test_rate_limiter_zero_means_unlimited() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..100 {
            assert!(limiter.check("telegram", "user1", 0).is_ok());
        }
    }

    #[test]
    fn test_rate_limiter_separate_users() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }
        // user1 is blocked
        assert!(limiter.check("telegram", "user1", 3).is_err());
        // user2 should still be ok
        assert!(limiter.check("telegram", "user2", 3).is_ok());
    }

    #[test]
    fn test_dm_policy_filtering() {
        // Test that DmPolicy::Ignore would be checked
        assert_eq!(DmPolicy::default(), DmPolicy::Respond);
        assert_eq!(GroupPolicy::default(), GroupPolicy::MentionOnly);
    }

    fn group_text_message(text: &str) -> ChannelMessage {
        ChannelMessage {
            channel: ChannelType::WhatsApp,
            platform_message_id: "m-1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Alice".to_string(),
                librefang_user: None,
            },
            content: ChannelContent::Text(text.to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: true,
            thread_id: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_mention_only_allows_regex_trigger_pattern() {
        with_guard_off_locked(|| {
            let message = group_text_message("hello MyAgent");
            let overrides = ChannelOverrides {
                group_trigger_patterns: vec!["(?i)\\bmyagent\\b".to_string()],
                ..Default::default()
            };
            assert!(should_process_group_message(
                "whatsapp", &overrides, &message
            ));
        });
    }

    #[test]
    fn test_mention_only_rejects_partial_regex_match() {
        with_guard_off_locked(|| {
            let message = group_text_message("hello myagenttt");
            let overrides = ChannelOverrides {
                group_trigger_patterns: vec!["(?i)\\bmyagent\\b".to_string()],
                ..Default::default()
            };
            assert!(!should_process_group_message(
                "whatsapp", &overrides, &message
            ));
        });
    }

    #[test]
    fn test_mention_only_skips_invalid_regex_patterns() {
        with_guard_off_locked(|| {
            let message = group_text_message("bot please reply");
            let overrides = ChannelOverrides {
                group_trigger_patterns: vec!["(".to_string(), "(?i)\\bbot\\b".to_string()],
                ..Default::default()
            };
            assert!(should_process_group_message(
                "telegram", &overrides, &message
            ));
        });
    }

    #[test]
    fn test_mention_only_keeps_existing_mention_behavior() {
        with_guard_off_locked(|| {
            let mut message = group_text_message("hello there");
            message
                .metadata
                .insert("was_mentioned".to_string(), serde_json::Value::Bool(true));
            let overrides = ChannelOverrides::default();
            assert!(should_process_group_message(
                "telegram", &overrides, &message
            ));
        });
    }

    #[test]
    fn test_channel_type_str() {
        assert_eq!(channel_type_str(&ChannelType::Telegram), "telegram");
        assert_eq!(channel_type_str(&ChannelType::Matrix), "matrix");
        assert_eq!(channel_type_str(&ChannelType::Email), "email");
        assert_eq!(
            channel_type_str(&ChannelType::Custom("irc".to_string())),
            "irc"
        );
    }

    #[test]
    fn test_sender_user_id_from_metadata() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            SENDER_USER_ID_KEY.to_string(),
            serde_json::Value::String("U456".to_string()),
        );
        let msg = ChannelMessage {
            channel: ChannelType::Slack,
            platform_message_id: "ts".to_string(),
            sender: ChannelUser {
                platform_id: "C789".to_string(),
                display_name: "U456".to_string(),
                librefang_user: None,
            },
            content: ChannelContent::Text("hi".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: true,
            thread_id: None,
            metadata,
        };
        assert_eq!(sender_user_id(&msg), "U456");
    }

    #[test]
    fn test_sender_user_id_fallback_to_platform_id() {
        let msg = ChannelMessage {
            channel: ChannelType::Telegram,
            platform_message_id: "123".to_string(),
            sender: ChannelUser {
                platform_id: "chat123".to_string(),
                display_name: "Alice".to_string(),
                librefang_user: None,
            },
            content: ChannelContent::Text("hi".to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: true,
            thread_id: None,
            metadata: std::collections::HashMap::new(),
        };
        assert_eq!(sender_user_id(&msg), "chat123");
    }

    #[test]
    fn test_default_output_format_for_channel() {
        assert_eq!(
            default_output_format_for_channel("telegram"),
            OutputFormat::TelegramHtml
        );
        assert_eq!(
            default_output_format_for_channel("slack"),
            OutputFormat::SlackMrkdwn
        );
        assert_eq!(
            default_output_format_for_channel("wecom"),
            OutputFormat::Markdown
        );
        assert_eq!(
            default_output_format_for_channel("discord"),
            OutputFormat::Markdown
        );
        assert_eq!(
            default_output_format_for_channel("signal"),
            OutputFormat::PlainText
        );
    }

    #[test]
    fn test_apply_agent_prefix_off_is_identity() {
        let text = "hello world";
        let out = apply_agent_prefix(PrefixStyle::Off, "coder", text);
        assert_eq!(out, text);
        assert_eq!(out.as_bytes(), text.as_bytes());
    }

    #[test]
    fn test_apply_agent_prefix_bracket() {
        let out = apply_agent_prefix(
            PrefixStyle::Bracket,
            "platform-architect",
            "Here's my take.",
        );
        assert_eq!(out, "[platform-architect] Here's my take.");
    }

    #[test]
    fn test_apply_agent_prefix_bold_bracket() {
        let out = apply_agent_prefix(PrefixStyle::BoldBracket, "coder", "All green.");
        assert_eq!(out, "**[coder]** All green.");
    }

    #[test]
    fn test_apply_agent_prefix_idempotent_bracket() {
        let already = "[coder] already prefixed";
        let out = apply_agent_prefix(PrefixStyle::Bracket, "coder", already);
        assert_eq!(out, already);
    }

    #[test]
    fn test_apply_agent_prefix_idempotent_bold_bracket() {
        let already = "**[coder]** already bold";
        let out = apply_agent_prefix(PrefixStyle::BoldBracket, "coder", already);
        assert_eq!(out, already);
        let out2 = apply_agent_prefix(PrefixStyle::Bracket, "coder", already);
        assert_eq!(out2, already);
    }

    #[test]
    fn test_apply_agent_prefix_empty_name_is_noop() {
        let text = "no author";
        let out = apply_agent_prefix(PrefixStyle::Bracket, "", text);
        assert_eq!(out, text);
    }

    /// Names containing `]` / `[` / `*` are pathological because our naive
    /// `starts_with("[name]")` idempotency check can misfire.
    ///
    /// Required behaviors verified here (per the doc-comment caveat):
    ///   1. Function MUST NOT panic on bracket / asterisk in the name.
    ///   2. Output MUST stay well-formed UTF-8.
    ///   3. Worst-case degradation is "extra/duplicated prefix", never data
    ///      loss or corruption of the body text.
    #[test]
    fn test_apply_agent_prefix_bracket_in_name_does_not_panic() {
        // `]` inside the name. First call produces `[a]b] hello`.
        let out = apply_agent_prefix(PrefixStyle::Bracket, "a]b", "hello");
        assert_eq!(out, "[a]b] hello");
        assert!(out.is_char_boundary(out.len()));

        // Second call: starts_with("[a]b]") matches because the literal is
        // `[a]b]` and the text begins with that — this is the "lucky" case
        // where the caveat doesn't bite. Idempotent here.
        let out2 = apply_agent_prefix(PrefixStyle::Bracket, "a]b", &out);
        assert_eq!(out2, "[a]b] hello");

        // `[` inside the name — the documented worst case. Repeated calls
        // legitimately stack a fresh prefix because `starts_with("[a[b]")`
        // does NOT match `[a[b] [a[b] hello`. Body ("hello") is preserved.
        let stacked = apply_agent_prefix(
            PrefixStyle::Bracket,
            "a[b",
            &apply_agent_prefix(PrefixStyle::Bracket, "a[b", "hello"),
        );
        assert!(
            stacked.ends_with("hello"),
            "body must be preserved: {stacked}"
        );
        assert!(stacked.is_char_boundary(stacked.len()));

        // `*` inside the name — bold style relies on `**[name]**`; an
        // asterisk in the name produces `**[a*b]**` which still passes the
        // `starts_with` check on a second invocation.
        let bold = apply_agent_prefix(PrefixStyle::BoldBracket, "a*b", "hi");
        assert_eq!(bold, "**[a*b]** hi");
        let bold2 = apply_agent_prefix(PrefixStyle::BoldBracket, "a*b", &bold);
        assert_eq!(bold2, bold);
    }

    #[tokio::test]
    async fn test_maybe_prefix_response_off_is_byte_identical() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides::default();
        let input = "Hello from the agent.".to_string();
        let original_bytes = input.clone();
        let out = maybe_prefix_response(&handle, Some(&overrides), agent_id, input).await;
        assert_eq!(out.as_bytes(), original_bytes.as_bytes());
    }

    #[tokio::test]
    async fn test_maybe_prefix_response_bracket_wraps() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::Bracket,
            ..Default::default()
        };
        let out =
            maybe_prefix_response(&handle, Some(&overrides), agent_id, "Hi".to_string()).await;
        assert_eq!(out, "[coder] Hi");
    }

    #[tokio::test]
    async fn test_maybe_prefix_response_bold_bracket_wraps() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::BoldBracket,
            ..Default::default()
        };
        let out =
            maybe_prefix_response(&handle, Some(&overrides), agent_id, "Hi".to_string()).await;
        assert_eq!(out, "**[coder]** Hi");
    }

    #[tokio::test]
    async fn test_maybe_prefix_response_unknown_agent_falls_back() {
        let known = AgentId::new();
        let unknown = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(known, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::Bracket,
            ..Default::default()
        };
        let out = maybe_prefix_response(&handle, Some(&overrides), unknown, "Hi".to_string()).await;
        assert_eq!(out, "Hi");
    }

    #[test]
    fn test_prefix_style_default_is_off_and_serde_snake_case() {
        assert_eq!(PrefixStyle::default(), PrefixStyle::Off);
        let v: PrefixStyle = serde_json::from_str("\"bracket\"").unwrap();
        assert_eq!(v, PrefixStyle::Bracket);
        let v: PrefixStyle = serde_json::from_str("\"bold_bracket\"").unwrap();
        assert_eq!(v, PrefixStyle::BoldBracket);
        let v: PrefixStyle = serde_json::from_str("\"off\"").unwrap();
        assert_eq!(v, PrefixStyle::Off);
    }

    #[test]
    fn test_channel_overrides_default_prefix_off() {
        let o = ChannelOverrides::default();
        assert_eq!(o.prefix_agent_name, PrefixStyle::Off);
    }

    #[tokio::test]
    async fn test_resolve_prefix_chunk_off_returns_none() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides::default();
        let out = resolve_prefix_chunk(&handle, Some(&overrides), agent_id).await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_resolve_prefix_chunk_bracket() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::Bracket,
            ..Default::default()
        };
        let out = resolve_prefix_chunk(&handle, Some(&overrides), agent_id).await;
        assert_eq!(out.as_deref(), Some("[coder] "));
    }

    #[tokio::test]
    async fn test_resolve_prefix_chunk_bold_bracket() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::BoldBracket,
            ..Default::default()
        };
        let out = resolve_prefix_chunk(&handle, Some(&overrides), agent_id).await;
        assert_eq!(out.as_deref(), Some("**[coder]** "));
    }

    #[tokio::test]
    async fn test_resolve_prefix_chunk_unknown_agent() {
        let known = AgentId::new();
        let unknown = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(known, "coder".to_string())]),
        });
        let overrides = ChannelOverrides {
            prefix_agent_name: PrefixStyle::Bracket,
            ..Default::default()
        };
        let out = resolve_prefix_chunk(&handle, Some(&overrides), unknown).await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn test_send_message_with_blocks_default_fallback() {
        // The default implementation of send_message_with_blocks extracts text
        // from blocks and calls send_message
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "vision-agent".to_string())]),
        });

        let blocks = vec![
            ContentBlock::Text {
                text: "What is in this photo?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "base64data".to_string(),
            },
        ];

        // Default impl should extract text and call send_message
        let result = handle
            .send_message_with_blocks(agent_id, blocks)
            .await
            .unwrap();
        assert_eq!(result, "Echo: What is in this photo?");
    }

    #[tokio::test]
    async fn test_send_message_with_blocks_image_only() {
        // When there's no text block, the default should still work
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "vision-agent".to_string())]),
        });

        let blocks = vec![ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        }];

        // Default impl sends empty text when no text blocks
        let result = handle
            .send_message_with_blocks(agent_id, blocks)
            .await
            .unwrap();
        assert_eq!(result, "Echo: ");
    }

    #[test]
    fn test_detect_image_magic_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_image_magic(&bytes), Some("image/jpeg".to_string()));
    }

    #[test]
    fn test_detect_image_magic_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_magic(&bytes), Some("image/png".to_string()));
    }

    #[test]
    fn test_detect_image_magic_gif() {
        let bytes = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_image_magic(&bytes), Some("image/gif".to_string()));
    }

    #[test]
    fn test_detect_image_magic_webp() {
        let bytes = [
            0x52, 0x49, 0x46, 0x46, // RIFF
            0x00, 0x00, 0x00, 0x00, // size (don't care)
            0x57, 0x45, 0x42, 0x50, // WEBP
        ];
        assert_eq!(detect_image_magic(&bytes), Some("image/webp".to_string()));
    }

    #[test]
    fn test_detect_image_magic_unknown() {
        let bytes = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_image_magic(&bytes), None);
    }

    #[test]
    fn test_detect_image_magic_empty() {
        assert_eq!(detect_image_magic(&[]), None);
    }

    #[test]
    fn test_detect_audio_magic_ogg() {
        // OggS magic
        let bytes = [0x4F, 0x67, 0x67, 0x53, 0x00, 0x02];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/ogg"));
    }

    #[test]
    fn test_detect_audio_magic_mp3_id3() {
        // ID3 tag
        let bytes = [0x49, 0x44, 0x33, 0x03, 0x00, 0x00];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mpeg"));
    }

    #[test]
    fn test_detect_audio_magic_mp3_sync_fb() {
        let bytes = [0xFF, 0xFB, 0x90, 0x00];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mpeg"));
    }

    #[test]
    fn test_detect_audio_magic_mp3_sync_f3() {
        let bytes = [0xFF, 0xF3, 0x90, 0x00];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mpeg"));
    }

    #[test]
    fn test_detect_audio_magic_mp3_sync_f2() {
        let bytes = [0xFF, 0xF2, 0x90, 0x00];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mpeg"));
    }

    #[test]
    fn test_detect_audio_magic_wav() {
        // RIFF....WAVE
        let bytes = [
            0x52, 0x49, 0x46, 0x46, // RIFF
            0x24, 0x00, 0x00, 0x00, // size
            0x57, 0x41, 0x56, 0x45, // WAVE
        ];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/wav"));
    }

    #[test]
    fn test_detect_audio_magic_flac() {
        // fLaC
        let bytes = [0x66, 0x4C, 0x61, 0x43, 0x00, 0x00];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/flac"));
    }

    #[test]
    fn test_detect_audio_magic_m4a() {
        // ....ftypM4A
        let bytes = [
            0x00, 0x00, 0x00, 0x20, // box size
            0x66, 0x74, 0x79, 0x70, // ftyp
            0x4D, 0x34, 0x41, 0x20, // M4A
        ];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mp4"));
    }

    #[test]
    fn test_detect_audio_magic_m4b() {
        // ....ftypM4B  (audiobook brand)
        let bytes = [
            0x00, 0x00, 0x00, 0x20, // box size
            0x66, 0x74, 0x79, 0x70, // ftyp
            0x4D, 0x34, 0x42, 0x20, // M4B
        ];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mp4"));
    }

    #[test]
    fn test_detect_audio_magic_isom() {
        // ....ftypisom
        let bytes = [
            0x00, 0x00, 0x00, 0x1C, // box size
            0x66, 0x74, 0x79, 0x70, // ftyp
            0x69, 0x73, 0x6F, 0x6D, // isom
        ];
        assert_eq!(detect_audio_magic(&bytes), Some("audio/mp4"));
    }

    #[test]
    fn test_detect_audio_magic_webm_ebml_returns_none() {
        // EBML magic also matches video/webm, so magic alone returns None;
        // filename-based detection (.weba) is the fallback for audio/webm.
        let bytes = [0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00];
        assert_eq!(detect_audio_magic(&bytes), None);
    }

    #[test]
    fn test_detect_audio_magic_unknown() {
        // Random bytes — must stay None
        let bytes = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
        assert_eq!(detect_audio_magic(&bytes), None);
    }

    #[test]
    fn test_detect_audio_magic_empty() {
        assert_eq!(detect_audio_magic(&[]), None);
    }

    #[test]
    fn test_audio_mime_from_filename_oga() {
        assert_eq!(audio_mime_from_filename("file_136.oga"), Some("audio/ogg"));
    }

    #[test]
    fn test_audio_mime_from_filename_ogg() {
        assert_eq!(audio_mime_from_filename("track.OGG"), Some("audio/ogg"));
    }

    #[test]
    fn test_audio_mime_from_filename_opus() {
        assert_eq!(audio_mime_from_filename("voice.opus"), Some("audio/ogg"));
    }

    #[test]
    fn test_audio_mime_from_filename_mp3() {
        assert_eq!(audio_mime_from_filename("song.mp3"), Some("audio/mpeg"));
    }

    #[test]
    fn test_audio_mime_from_filename_wav() {
        assert_eq!(audio_mime_from_filename("clip.wav"), Some("audio/wav"));
    }

    #[test]
    fn test_audio_mime_from_filename_flac() {
        assert_eq!(audio_mime_from_filename("album.flac"), Some("audio/flac"));
    }

    #[test]
    fn test_audio_mime_from_filename_m4a() {
        assert_eq!(audio_mime_from_filename("audio.m4a"), Some("audio/mp4"));
    }

    #[test]
    fn test_audio_mime_from_filename_webm() {
        assert_eq!(audio_mime_from_filename("clip.webm"), Some("audio/webm"));
    }

    #[test]
    fn test_audio_mime_from_filename_unknown() {
        // No audio extension — must return None
        assert_eq!(audio_mime_from_filename("photo.jpg"), None);
        assert_eq!(audio_mime_from_filename("document.pdf"), None);
        assert_eq!(audio_mime_from_filename("noextension"), None);
    }

    #[tokio::test]
    async fn test_handle_command_btw_no_args() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            librefang_user: None,
        };

        let result = handle_command(
            "btw",
            &[],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("Usage:"));
    }

    #[tokio::test]
    async fn test_handle_command_btw_no_agent_selected() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            librefang_user: None,
        };

        // No agent selected for this user
        let result = handle_command(
            "btw",
            &["what is rust?".to_string()],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("No agent selected"));
    }

    #[tokio::test]
    async fn test_help_includes_btw_command() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            librefang_user: None,
        };

        let result = handle_command(
            "help",
            &[],
            &handle,
            &router,
            &sender,
            &ChannelType::CLI,
            None,
        )
        .await;
        assert!(result.contains("/btw"));
    }

    #[test]
    fn test_media_type_from_url() {
        assert_eq!(
            media_type_from_url("https://example.com/photo.png"),
            "image/png"
        );
        assert_eq!(
            media_type_from_url("https://example.com/anim.gif"),
            "image/gif"
        );
        assert_eq!(
            media_type_from_url("https://example.com/img.webp"),
            "image/webp"
        );
        assert_eq!(
            media_type_from_url("https://example.com/photo.jpg"),
            "image/jpeg"
        );
        // No extension — defaults to JPEG
        assert_eq!(
            media_type_from_url("https://api.telegram.org/file/bot123/photos/file_42"),
            "image/jpeg"
        );
    }

    #[test]
    fn test_content_to_text_command() {
        let cmd = ChannelContent::Command {
            name: "help".to_string(),
            args: vec!["list".to_string()],
        };
        assert_eq!(content_to_text(&cmd), "/help list");
    }

    #[test]
    fn test_content_to_text_command_no_args() {
        let cmd = ChannelContent::Command {
            name: "status".to_string(),
            args: vec![],
        };
        assert_eq!(content_to_text(&cmd), "/status");
    }

    #[test]
    fn test_content_to_text_text() {
        let text = ChannelContent::Text("hello world".to_string());
        assert_eq!(content_to_text(&text), "hello world");
    }

    #[test]
    fn test_content_to_text_image() {
        let img = ChannelContent::Image {
            url: "https://example.com/photo.jpg".to_string(),
            caption: Some("A cat".to_string()),
            mime_type: None,
        };
        assert_eq!(
            content_to_text(&img),
            "[Photo: https://example.com/photo.jpg]\nA cat"
        );
    }

    #[test]
    fn test_content_to_text_image_no_caption() {
        let img = ChannelContent::Image {
            url: "https://example.com/photo.jpg".to_string(),
            caption: None,
            mime_type: None,
        };
        assert_eq!(
            content_to_text(&img),
            "[Photo: https://example.com/photo.jpg]"
        );
    }

    #[test]
    fn test_content_to_text_file() {
        let file = ChannelContent::File {
            url: "https://example.com/doc.pdf".to_string(),
            filename: "document.pdf".to_string(),
        };
        assert_eq!(
            content_to_text(&file),
            "[File (document.pdf): https://example.com/doc.pdf]"
        );
    }

    #[test]
    fn test_content_to_text_voice() {
        let voice = ChannelContent::Voice {
            url: "https://example.com/voice.ogg".to_string(),
            duration_seconds: 30,
            caption: None,
        };
        assert_eq!(
            content_to_text(&voice),
            "[Voice message (30s): https://example.com/voice.ogg]"
        );
    }

    #[test]
    fn test_filename_from_url_basic() {
        assert_eq!(
            filename_from_url("https://example.com/path/voice_42.oga").as_deref(),
            Some("voice_42.oga")
        );
    }

    #[test]
    fn test_filename_from_url_strips_query_and_fragment() {
        assert_eq!(
            filename_from_url("https://example.com/x/file.ogg?token=abc#t=1").as_deref(),
            Some("file.ogg")
        );
    }

    #[test]
    fn test_filename_from_url_no_basename() {
        assert!(filename_from_url("https://example.com/").is_none());
        assert!(filename_from_url("not a url").is_none());
    }

    #[test]
    fn test_content_to_text_button_callback() {
        let cb = ChannelContent::ButtonCallback {
            action: "approve".to_string(),
            message_text: Some("Approved".to_string()),
        };
        assert_eq!(content_to_text(&cb), "[Button: approve]");
    }

    #[test]
    fn test_content_to_text_audio() {
        let content = ChannelContent::Audio {
            url: "https://example.com/song.mp3".to_string(),
            caption: Some("My song".to_string()),
            duration_seconds: 180,
            title: Some("Song Title".to_string()),
            performer: Some("Artist".to_string()),
        };
        let text = content_to_text(&content);
        assert!(
            text.contains("song.mp3") || text.contains("Song Title") || text.contains("Audio"),
            "Audio content_to_text should contain meaningful info, got: {text}"
        );
    }

    #[test]
    fn test_content_to_text_audio_no_caption() {
        let content = ChannelContent::Audio {
            url: "https://example.com/track.mp3".to_string(),
            caption: None,
            duration_seconds: 60,
            title: None,
            performer: None,
        };
        let text = content_to_text(&content);
        assert!(
            !text.is_empty(),
            "Audio without caption should still produce text"
        );
    }

    #[test]
    fn test_content_to_text_animation() {
        let content = ChannelContent::Animation {
            url: "https://example.com/funny.gif".to_string(),
            caption: Some("LOL".to_string()),
            duration_seconds: 5,
        };
        let text = content_to_text(&content);
        assert!(
            text.contains("LOL") || text.contains("Animation") || text.contains("funny.gif"),
            "Animation content_to_text should contain meaningful info, got: {text}"
        );
    }

    #[test]
    fn test_content_to_text_sticker() {
        let content = ChannelContent::Sticker {
            file_id: "CAACAgIAAxkBAAI".to_string(),
        };
        let text = content_to_text(&content);
        assert!(!text.is_empty(), "Sticker should produce non-empty text");
    }

    #[test]
    fn test_content_to_text_media_group() {
        let content = ChannelContent::MediaGroup {
            items: vec![
                crate::types::MediaGroupItem::Photo {
                    url: "https://example.com/1.jpg".to_string(),
                    caption: Some("First".to_string()),
                },
                crate::types::MediaGroupItem::Video {
                    url: "https://example.com/2.mp4".to_string(),
                    caption: None,
                    duration_seconds: 30,
                },
            ],
        };
        let text = content_to_text(&content);
        assert!(
            text.contains("2") || text.contains("album") || text.contains("media"),
            "MediaGroup should mention item count or type, got: {text}"
        );
    }

    #[test]
    fn test_content_to_text_poll() {
        let content = ChannelContent::Poll {
            question: "What is 2+2?".to_string(),
            options: vec!["3".to_string(), "4".to_string(), "5".to_string()],
            is_quiz: true,
            correct_option_id: Some(1),
            explanation: Some("Basic math".to_string()),
        };
        let text = content_to_text(&content);
        assert!(
            text.contains("2+2") || text.contains("Poll") || text.contains("quiz"),
            "Poll should contain the question or type, got: {text}"
        );
    }

    #[test]
    fn test_content_to_text_poll_answer() {
        let content = ChannelContent::PollAnswer {
            poll_id: "poll_123".to_string(),
            option_ids: vec![0, 2],
        };
        let text = content_to_text(&content);
        assert!(!text.is_empty(), "PollAnswer should produce non-empty text");
    }

    #[test]
    fn test_content_to_text_delete_message() {
        let content = ChannelContent::DeleteMessage {
            message_id: "42".to_string(),
        };
        let text = content_to_text(&content);
        assert!(
            text.contains("42") || text.contains("delete") || text.contains("Delete"),
            "DeleteMessage should mention message_id or action, got: {text}"
        );
    }

    mod message_debouncer {
        use super::*;
        use std::collections::HashMap;

        fn make_test_message(text: &str) -> ChannelMessage {
            ChannelMessage {
                channel: ChannelType::Discord,
                platform_message_id: "msg1".to_string(),
                sender: ChannelUser {
                    platform_id: "user123".to_string(),
                    display_name: "TestUser".to_string(),
                    librefang_user: None,
                },
                content: ChannelContent::Text(text.to_string()),
                target_agent: None,
                timestamp: chrono::Utc::now(),
                is_group: false,
                thread_id: None,
                metadata: HashMap::new(),
            }
        }

        fn make_test_command(name: &str, args: Vec<String>) -> ChannelMessage {
            ChannelMessage {
                channel: ChannelType::Discord,
                platform_message_id: "msg1".to_string(),
                sender: ChannelUser {
                    platform_id: "user123".to_string(),
                    display_name: "TestUser".to_string(),
                    librefang_user: None,
                },
                content: ChannelContent::Command {
                    name: name.to_string(),
                    args,
                },
                target_agent: None,
                timestamp: chrono::Utc::now(),
                is_group: false,
                thread_id: None,
                metadata: HashMap::new(),
            }
        }

        fn assert_content_eq(actual: &ChannelContent, expected: &str) {
            let actual_text = content_to_text(actual);
            assert_eq!(actual_text, expected);
        }

        #[tokio::test]
        async fn test_debouncer_single_message() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let msg = make_test_message("hello");
            let pending = PendingMessage {
                message: msg.clone(),
                image_blocks: None,
            };

            debouncer.push("discord:user123", pending, &mut buffers);

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_some());
            let (drained_msg, blocks) = result.unwrap();
            assert_content_eq(&drained_msg.content, "hello");
            assert!(blocks.is_none());
        }

        #[tokio::test]
        async fn test_debouncer_multiple_texts_merge() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let msg1 = make_test_message("hello");
            let msg2 = make_test_message("world");

            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: msg1,
                    image_blocks: None,
                },
                &mut buffers,
            );
            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: msg2,
                    image_blocks: None,
                },
                &mut buffers,
            );

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_some());
            let (drained_msg, _) = result.unwrap();
            assert_content_eq(&drained_msg.content, "hello\nworld");
        }

        #[tokio::test]
        async fn test_debouncer_commands_same_name_merge() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let cmd1 = make_test_command("help", vec!["list".to_string()]);
            let cmd2 = make_test_command("help", vec!["status".to_string()]);

            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: cmd1,
                    image_blocks: None,
                },
                &mut buffers,
            );
            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: cmd2,
                    image_blocks: None,
                },
                &mut buffers,
            );

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_some());
            let (drained_msg, _) = result.unwrap();
            match drained_msg.content {
                ChannelContent::Command { name, args } => {
                    assert_eq!(name, "help");
                    assert_eq!(args, vec!["list", "status"]);
                }
                _ => panic!("Expected Command content"),
            }
        }

        #[tokio::test]
        async fn test_debouncer_different_commands_no_merge() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let cmd1 = make_test_command("help", vec![]);
            let cmd2 = make_test_command("status", vec![]);

            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: cmd1,
                    image_blocks: None,
                },
                &mut buffers,
            );
            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: cmd2,
                    image_blocks: None,
                },
                &mut buffers,
            );

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_some());
            let (drained_msg, _) = result.unwrap();
            assert_content_eq(&drained_msg.content, "/help\n/status");
        }

        #[tokio::test]
        async fn test_debouncer_empty_buffer_returns_none() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_debouncer_different_senders_separate() {
            let (debouncer, _rx) = MessageDebouncer::new(100, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let msg1 = make_test_message("hello from user1");
            let msg2 = make_test_message("hello from user2");

            debouncer.push(
                "discord:user1",
                PendingMessage {
                    message: msg1,
                    image_blocks: None,
                },
                &mut buffers,
            );
            debouncer.push(
                "discord:user2",
                PendingMessage {
                    message: msg2,
                    image_blocks: None,
                },
                &mut buffers,
            );

            let result1 = debouncer.drain("discord:user1", &mut buffers);
            let result2 = debouncer.drain("discord:user2", &mut buffers);

            assert!(result1.is_some());
            assert!(result2.is_some());
            assert_content_eq(&result1.unwrap().0.content, "hello from user1");
            assert_content_eq(&result2.unwrap().0.content, "hello from user2");
        }

        #[tokio::test]
        async fn test_debouncer_max_buffer_triggers_flush() {
            let (debouncer, _rx) = MessageDebouncer::new(1000, 5000, 2);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            let msg1 = make_test_message("1");
            let msg2 = make_test_message("2");

            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: msg1,
                    image_blocks: None,
                },
                &mut buffers,
            );
            debouncer.push(
                "discord:user123",
                PendingMessage {
                    message: msg2,
                    image_blocks: None,
                },
                &mut buffers,
            );

            let result = debouncer.drain("discord:user123", &mut buffers);
            assert!(result.is_some());
            let (drained_msg, _) = result.unwrap();
            assert_content_eq(&drained_msg.content, "1\n2");
        }

        // Regression test for #3742: simulates the race where the manual
        // max-buffer flush path AND the max_timer task BOTH enqueue the same
        // key on flush_tx. The receiver loop calls drain() once per dequeued
        // key, so the second call must be a noop — i.e. drain() relies on
        // `buffers.remove(key)` as the atomic single-take guard. If anything
        // ever regresses to e.g. `buffers.get(key)` + side effects, this test
        // catches the resulting double-send.
        #[tokio::test]
        async fn test_debouncer_double_drain_is_idempotent() {
            let (debouncer, _rx) = MessageDebouncer::new(1000, 5000, 10);
            let mut buffers: HashMap<String, SenderBuffer> = HashMap::new();

            debouncer.push(
                "discord:userX",
                PendingMessage {
                    message: make_test_message("only"),
                    image_blocks: None,
                },
                &mut buffers,
            );

            // First drain takes the buffer atomically.
            let first = debouncer.drain("discord:userX", &mut buffers);
            assert!(first.is_some());
            // Second drain on the same key must observe an empty entry and noop.
            let second = debouncer.drain("discord:userX", &mut buffers);
            assert!(
                second.is_none(),
                "double-flush race must not duplicate-send (#3742)"
            );
            assert!(
                !buffers.contains_key("discord:userX"),
                "drain must remove the buffer entry"
            );
        }

        // Regression for #3580: the flush channel is bounded so that a
        // stalled / dropped dispatcher cannot let RSS grow unbounded.
        // We drop the receiver and push more sender keys than the cap;
        // every send beyond the first must surface as an Err (and be
        // logged + dropped via warn_flush_dropped) rather than silently
        // accumulating in an unbounded queue.
        #[tokio::test]
        async fn test_debouncer_flush_channel_is_bounded() {
            let (debouncer, rx) = MessageDebouncer::new(1000, 5000, 10);
            // Drop receiver to force every try_send to error — this models
            // the worst-case "dispatcher gone" path; the cap-limited path
            // is exercised inherently by `mpsc::channel(FLUSH_CHANNEL_CAP)`.
            drop(rx);

            let mut errs = 0usize;
            // Push 2x cap distinct keys; each immediate-flush path hits
            // try_send. With a bounded channel + dropped rx, every call
            // returns Err. With the old unbounded channel, the queue
            // would grow without bound and never error.
            for i in 0..(FLUSH_CHANNEL_CAP * 2) {
                let key = format!("k{i}");
                if debouncer.flush_tx.try_send(key).is_err() {
                    errs += 1;
                }
            }
            assert_eq!(
                errs,
                FLUSH_CHANNEL_CAP * 2,
                "bounded flush channel must reject sends when receiver is gone"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Phase 2 §C — Vocative trigger + addressee guard tests (OB-04, OB-05)
    // ---------------------------------------------------------------------

    mod vocative_tests {
        use super::super::is_vocative_trigger;

        #[test]
        fn matches_at_start_of_turn_with_comma() {
            assert!(is_vocative_trigger("Signore, dimmi", "Signore"));
        }

        #[test]
        fn matches_at_start_of_turn_with_space() {
            assert!(is_vocative_trigger("Signore chiedi al bot", "Signore"));
        }

        #[test]
        fn matches_after_strong_punctuation() {
            assert!(is_vocative_trigger("ciao. Signore, come va?", "Signore"));
        }

        #[test]
        fn matches_with_leading_whitespace() {
            assert!(is_vocative_trigger("  Signore, ...", "Signore"));
        }

        #[test]
        fn rejects_other_capitalized_vocative_before_pattern() {
            // The Beeper-screenshot case (user directive).
            assert!(!is_vocative_trigger(
                "Caterina, chiedi al Signore il pagamento",
                "Signore"
            ));
        }

        #[test]
        fn rejects_when_not_at_vocative_position() {
            assert!(!is_vocative_trigger(
                "Ieri il Signore ha detto di...",
                "Signore"
            ));
        }

        #[test]
        fn rejects_lowercase_substring() {
            // Pattern is "Signore" (proper-name); lowercase should not match.
            assert!(!is_vocative_trigger("il signore è arrivato", "Signore"));
        }

        #[test]
        fn rejects_with_alessandro_then_signore() {
            assert!(!is_vocative_trigger(
                "Alessandro, dopo chiama il Signore",
                "Signore"
            ));
        }

        #[test]
        fn word_boundary_signori_not_signore() {
            assert!(!is_vocative_trigger("Signori, ascoltate", "Signore"));
        }

        #[test]
        fn empty_text_returns_false() {
            assert!(!is_vocative_trigger("", "Signore"));
        }

        #[test]
        fn dammi_il_signore_rejected() {
            assert!(!is_vocative_trigger("dammi il Signore", "Signore"));
        }
    }

    mod addressee_tests {
        use super::super::is_addressed_to_other_participant;
        use crate::types::ParticipantRef;

        fn roster(names: &[&str]) -> Vec<ParticipantRef> {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| ParticipantRef {
                    jid: format!("{}@s.whatsapp.net", i),
                    display_name: (*n).to_string(),
                })
                .collect()
        }

        #[test]
        fn caterina_with_caterina_in_roster_returns_true() {
            let r = roster(&["Caterina", "Ambrogio"]);
            assert!(is_addressed_to_other_participant(
                "Caterina, chiedi...",
                &r,
                "Ambrogio"
            ));
        }

        #[test]
        fn agent_addressed_returns_false() {
            let r = roster(&["Caterina", "Ambrogio"]);
            assert!(!is_addressed_to_other_participant(
                "Ambrogio, vieni qui",
                &r,
                "Ambrogio"
            ));
        }

        #[test]
        fn no_vocative_returns_false() {
            let r = roster(&["Caterina", "Ambrogio"]);
            assert!(!is_addressed_to_other_participant(
                "stamattina è bello",
                &r,
                "Ambrogio"
            ));
        }

        #[test]
        fn exclamation_vocative_recognized() {
            let r = roster(&["Caterina", "Bot"]);
            assert!(is_addressed_to_other_participant("Caterina!", &r, "Bot"));
        }

        #[test]
        fn beeper_screenshot_full_turn() {
            let r = roster(&["Caterina", "Bot"]);
            assert!(is_addressed_to_other_participant(
                "Caterina, chiedi al Signore il pagamento",
                &r,
                "Bot"
            ));
        }

        #[test]
        fn name_not_in_roster_returns_false() {
            // "Marco," is a vocative but Marco isn't a participant — guard
            // does not fire (avoids false positives on names that happen to
            // start a sentence but aren't in the group).
            let r = roster(&["Caterina", "Bot"]);
            assert!(!is_addressed_to_other_participant(
                "Marco, dove sei?",
                &r,
                "Bot"
            ));
        }

        #[test]
        fn case_insensitive_match() {
            let r = roster(&["caterina", "Bot"]);
            assert!(is_addressed_to_other_participant(
                "Caterina, vieni qui",
                &r,
                "Bot"
            ));
        }
    }

    // ---------------------------------------------------------------------
    // §C wiring tests — should_process_group_message + guard flag behavior
    // ---------------------------------------------------------------------

    mod should_process_group_message_v2 {
        use super::super::{should_process_group_message, ParticipantRef};
        use super::group_text_message;
        use librefang_types::config::{ChannelOverrides, GroupPolicy};
        use serde_json::json;

        // Reuse the outer module's env lock so tests across BOTH modules
        // serialize their reads/writes of LIBREFANG_GROUP_ADDRESSEE_GUARD.
        // Two independent Mutexes meant v2 tests could mutate the env var
        // while outer-module `test_mention_only_*` tests read it via
        // `addressee_guard_enabled()`, causing flakes under `cargo test`
        // parallel execution.
        use super::ADDRESSEE_GUARD_ENV_LOCK as ENV_LOCK;

        fn with_guard_on<F: FnOnce()>(f: F) {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: guarded by ENV_LOCK mutex; no concurrent thread reads/writes
            // LIBREFANG_GROUP_ADDRESSEE_GUARD while the lock is held.
            unsafe {
                std::env::set_var("LIBREFANG_GROUP_ADDRESSEE_GUARD", "on");
            }
            f();
            unsafe { std::env::remove_var("LIBREFANG_GROUP_ADDRESSEE_GUARD") };
        }

        fn with_guard_off<F: FnOnce()>(f: F) {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: guarded by ENV_LOCK mutex.
            unsafe { std::env::remove_var("LIBREFANG_GROUP_ADDRESSEE_GUARD") };
            f();
        }

        fn inject_roster(msg: &mut crate::types::ChannelMessage, names: &[&str], agent: &str) {
            let participants: Vec<ParticipantRef> = names
                .iter()
                .enumerate()
                .map(|(i, n)| ParticipantRef {
                    jid: format!("{i}@s.whatsapp.net"),
                    display_name: (*n).to_string(),
                })
                .collect();
            msg.metadata.insert(
                "group_participants".to_string(),
                serde_json::to_value(&participants).unwrap(),
            );
            msg.metadata.insert("agent_name".to_string(), json!(agent));
        }

        #[test]
        fn caterina_chiedi_al_signore_rejected_under_guard() {
            with_guard_on(|| {
                let mut msg = group_text_message("Caterina, chiedi al Signore il pagamento");
                inject_roster(&mut msg, &["Caterina", "Ambrogio"], "Ambrogio");
                let overrides = ChannelOverrides {
                    group_policy: GroupPolicy::MentionOnly,
                    group_trigger_patterns: vec!["Signore".to_string()],
                    ..Default::default()
                };
                assert!(!should_process_group_message("whatsapp", &overrides, &msg));
            });
        }

        #[test]
        fn signore_at_start_passes_under_guard() {
            with_guard_on(|| {
                let mut msg = group_text_message("Signore, conferma il prossimo appuntamento");
                inject_roster(&mut msg, &["Caterina", "Ambrogio"], "Ambrogio");
                let overrides = ChannelOverrides {
                    group_policy: GroupPolicy::MentionOnly,
                    group_trigger_patterns: vec!["Signore".to_string()],
                    ..Default::default()
                };
                assert!(should_process_group_message("whatsapp", &overrides, &msg));
            });
        }

        #[test]
        fn owner_no_mention_no_pattern_rejected() {
            // OB-06: "owner-in-group" doesn't bypass mention_only — there's
            // no owner short-circuit in librefang-channels (audit confirms).
            // A plain "ciao a tutti" with no mention is rejected.
            with_guard_on(|| {
                let mut msg = group_text_message("ciao a tutti, come va?");
                inject_roster(&mut msg, &["Caterina", "Ambrogio"], "Ambrogio");
                let overrides = ChannelOverrides {
                    group_policy: GroupPolicy::MentionOnly,
                    group_trigger_patterns: vec!["Signore".to_string()],
                    ..Default::default()
                };
                assert!(!should_process_group_message("whatsapp", &overrides, &msg));
            });
        }

        #[test]
        fn owner_explicit_mention_passes() {
            with_guard_on(|| {
                let mut msg = group_text_message("@Bot rispondimi");
                inject_roster(&mut msg, &["Caterina", "Ambrogio"], "Ambrogio");
                msg.metadata
                    .insert("was_mentioned".to_string(), json!(true));
                let overrides = ChannelOverrides {
                    group_policy: GroupPolicy::MentionOnly,
                    ..Default::default()
                };
                assert!(should_process_group_message("whatsapp", &overrides, &msg));
            });
        }

        #[test]
        fn legacy_substring_still_works_with_guard_off() {
            // Backward compat: with the flag default-off (rollback path)
            // the pre-Phase-2 substring matcher remains authoritative.
            with_guard_off(|| {
                let msg = group_text_message("Caterina, chiedi al Signore il pagamento");
                let overrides = ChannelOverrides {
                    group_policy: GroupPolicy::MentionOnly,
                    group_trigger_patterns: vec!["(?i)\\bSignore\\b".to_string()],
                    ..Default::default()
                };
                // Legacy behavior: substring matches → returns true.
                assert!(should_process_group_message("whatsapp", &overrides, &msg));
            });
        }
    }

    // ---------------------------------------------------------------------
    // BC-02 — SenderContext serde-default for group_participants
    // ---------------------------------------------------------------------

    mod bc02_tests {
        use crate::types::SenderContext;

        #[test]
        fn old_blob_without_group_participants_parses() {
            // Stored canonical blob from before Phase 2 §C — no
            // `group_participants` key. Must deserialize cleanly.
            let json = r#"{
                "channel": "whatsapp",
                "user_id": "u1",
                "display_name": "Alice",
                "is_group": false,
                "was_mentioned": false,
                "thread_id": null,
                "account_id": null,
                "auto_route": "off",
                "auto_route_ttl_minutes": 0,
                "auto_route_confidence_threshold": 0,
                "auto_route_sticky_bonus": 0,
                "auto_route_divergence_count": 0
            }"#;
            let ctx: SenderContext = serde_json::from_str(json).expect("BC-02 parse");
            assert!(ctx.group_participants.is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // ReplyEnvelope (§A — owner-notify channel)
    // -----------------------------------------------------------------------

    #[test]
    fn reply_envelope_default_has_no_fields() {
        let env = ReplyEnvelope::default();
        assert!(env.public.is_none());
        assert!(env.owner_notice.is_none());
    }

    #[test]
    fn reply_envelope_from_public_sets_only_public() {
        let env = ReplyEnvelope::from_public("hi");
        assert_eq!(env.public.as_deref(), Some("hi"));
        assert!(env.owner_notice.is_none());
    }

    #[test]
    fn reply_envelope_silent_is_default() {
        let env = ReplyEnvelope::silent();
        assert_eq!(env, ReplyEnvelope::default());
    }

    #[test]
    fn reply_envelope_serde_roundtrip_full() {
        let env = ReplyEnvelope {
            public: Some("yes Sir".into()),
            owner_notice: Some("Caterina asked something".into()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let decoded: ReplyEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, env);
    }

    #[test]
    fn reply_envelope_deserializes_legacy_missing_fields() {
        // BC-02: stored blobs may not contain these fields yet.
        let decoded: ReplyEnvelope = serde_json::from_str("{}").unwrap();
        assert!(decoded.public.is_none());
        assert!(decoded.owner_notice.is_none());

        let decoded2: ReplyEnvelope = serde_json::from_str(r#"{"public":"x"}"#).unwrap();
        assert_eq!(decoded2.public.as_deref(), Some("x"));
        assert!(decoded2.owner_notice.is_none());
    }

    #[test]
    fn reply_envelope_public_or_empty_helper() {
        assert_eq!(ReplyEnvelope::default().public_or_empty(), "");
        assert_eq!(
            ReplyEnvelope::from_public("hello").public_or_empty(),
            "hello"
        );
    }

    mod classify_reply_intent_tests {
        use super::super::*;
        use std::sync::{Arc, Mutex};

        struct CapturingHandle {
            captured_bot_name: Arc<Mutex<Option<Option<String>>>>,
        }

        impl CapturingHandle {
            fn new() -> (Self, Arc<Mutex<Option<Option<String>>>>) {
                let slot = Arc::new(Mutex::new(None));
                (
                    Self {
                        captured_bot_name: Arc::clone(&slot),
                    },
                    slot,
                )
            }
        }

        #[async_trait::async_trait]
        impl ChannelBridgeHandle for CapturingHandle {
            async fn send_message(&self, _: AgentId, _: &str) -> Result<String, String> {
                Err("not used in test".into())
            }
            async fn find_agent_by_name(&self, _: &str) -> Result<Option<AgentId>, String> {
                Err("not used in test".into())
            }
            async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
                Err("not used in test".into())
            }
            async fn spawn_agent_by_name(&self, _: &str) -> Result<AgentId, String> {
                Err("not used in test".into())
            }
            async fn classify_reply_intent(
                &self,
                _message_text: &str,
                _sender_name: &str,
                _model: Option<&str>,
                bot_name: Option<&str>,
                _aliases: Option<&[String]>,
            ) -> bool {
                *self.captured_bot_name.lock().unwrap() = Some(bot_name.map(|s| s.to_string()));
                true
            }
            fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
                // Test mock: no event bus to forward to.
            }
        }

        #[tokio::test]
        async fn default_impl_returns_true_with_bot_name() {
            struct AlwaysTrue;
            #[async_trait::async_trait]
            impl ChannelBridgeHandle for AlwaysTrue {
                async fn send_message(&self, _: AgentId, _: &str) -> Result<String, String> {
                    Err("not used in test".into())
                }
                async fn find_agent_by_name(&self, _: &str) -> Result<Option<AgentId>, String> {
                    Err("not used in test".into())
                }
                async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
                    Err("not used in test".into())
                }
                async fn spawn_agent_by_name(&self, _: &str) -> Result<AgentId, String> {
                    Err("not used in test".into())
                }
                fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
                    // Test mock: no event bus to forward to.
                }
            }

            let h = AlwaysTrue;
            assert!(
                h.classify_reply_intent("hello", "user", None, Some("rodelo"), None)
                    .await
            );
            assert!(
                h.classify_reply_intent("hello", "user", None, None, None)
                    .await
            );
        }

        #[tokio::test]
        async fn bot_name_is_forwarded_to_implementation() {
            let (handle, slot) = CapturingHandle::new();
            handle
                .classify_reply_intent("rodelo qué hora es?", "Alice", None, Some("rodelo"), None)
                .await;
            assert_eq!(
                *slot.lock().unwrap(),
                Some(Some("rodelo".to_string())),
                "bot_name must be forwarded to the classify_reply_intent implementation"
            );
        }

        #[tokio::test]
        async fn none_bot_name_is_forwarded() {
            let (handle, slot) = CapturingHandle::new();
            handle
                .classify_reply_intent("hey there", "Bob", None, None, None)
                .await;
            assert_eq!(
                *slot.lock().unwrap(),
                Some(None),
                "None bot_name must be forwarded as None"
            );
        }
    }

    // ---------------------------------------------------------------------
    // File download helpers
    // ---------------------------------------------------------------------

    mod file_download_tests {
        use super::*;

        #[test]
        fn test_sanitize_extension_normal() {
            assert_eq!(sanitize_extension("pdf"), "pdf");
            assert_eq!(sanitize_extension("PNG"), "png");
            assert_eq!(sanitize_extension("tar"), "tar");
            assert_eq!(sanitize_extension("jpg"), "jpg");
        }

        #[test]
        fn test_sanitize_extension_strips_non_alnum() {
            // tar.gz via Path::extension gives "gz", but test the sanitizer directly
            assert_eq!(sanitize_extension("g.z"), "gz");
            assert_eq!(sanitize_extension("../etc/passwd"), "etcpasswd");
            assert_eq!(sanitize_extension("exe;rm -rf"), "exermrf");
        }

        #[test]
        fn test_sanitize_extension_empty_and_special() {
            assert_eq!(sanitize_extension(""), "bin");
            assert_eq!(sanitize_extension("..."), "bin");
            assert_eq!(sanitize_extension("///"), "bin");
        }

        #[test]
        fn test_sanitize_extension_unicode() {
            // Non-ASCII chars are stripped
            assert_eq!(sanitize_extension("pdfé"), "pdf");
            assert_eq!(sanitize_extension("日本語"), "bin");
        }

        #[test]
        fn test_validate_url_scheme_http() {
            assert!(validate_url_scheme("https://example.com/file.pdf").is_ok());
            assert!(validate_url_scheme("http://example.com/file.pdf").is_ok());
        }

        #[test]
        fn test_validate_url_scheme_rejected() {
            assert!(validate_url_scheme("file:///etc/passwd").is_err());
            assert!(validate_url_scheme("ftp://example.com/file.pdf").is_err());
            assert!(validate_url_scheme("javascript:alert(1)").is_err());
            assert!(validate_url_scheme("data:text/plain,hello").is_err());
            assert!(validate_url_scheme("/local/path").is_err());
        }

        /// #3442: an inbound channel message may not smuggle a loopback,
        /// private, link-local, or cloud-metadata URL through the
        /// attachment-download path.  Pre-fix this checked scheme only.
        #[test]
        fn test_validate_url_scheme_blocks_ssrf_targets() {
            for url in [
                "http://127.0.0.1/admin",
                "http://localhost/admin",
                "http://169.254.169.254/latest/meta-data/",
                "http://10.0.0.1/internal",
                "http://192.168.1.1/router",
                "http://[::1]/admin",
                "http://[::ffff:169.254.169.254]/imds",
                "http://metadata.google.internal/v1/instance",
            ] {
                assert!(
                    validate_url_scheme(url).is_err(),
                    "expected SSRF reject for {url}"
                );
            }
        }

        #[tokio::test]
        async fn test_file_download_rejects_bad_scheme() {
            let dir = std::env::temp_dir().join("librefang_test_download");
            let blocks = download_file_to_blocks(
                "ftp://evil.com/malware.exe",
                "malware.exe",
                1024,
                &dir,
                &[],
            )
            .await;
            assert_eq!(blocks.len(), 1);
            match &blocks[0] {
                ContentBlock::Text { text, .. } => {
                    assert!(
                        text.contains("rejected"),
                        "Expected rejection message, got: {text}"
                    );
                }
                other => panic!("Expected Text block, got: {other:?}"),
            }
        }
    }

    mod tool_marker_extraction {
        use super::super::extract_tool_marker_name;

        #[test]
        fn extracts_simple_tool_name() {
            assert_eq!(
                extract_tool_marker_name("\n\n🔧 system_time\n\n"),
                Some("system_time".to_string())
            );
        }

        #[test]
        fn extracts_pretty_tool_name_with_spaces() {
            // `prettify_tool_name` upstream may render `web_fetch` as
            // `Web Fetch` — the marker passes through whatever upstream
            // emits. Trim whitespace but preserve the inner text.
            assert_eq!(
                extract_tool_marker_name("\n\n🔧 Web Fetch\n\n"),
                Some("Web Fetch".to_string())
            );
        }

        #[test]
        fn rejects_plain_text_delta() {
            assert_eq!(extract_tool_marker_name("Hello world"), None);
            assert_eq!(extract_tool_marker_name(""), None);
        }

        #[test]
        fn rejects_error_marker() {
            // `\n\n⚠️ tool failed\n\n` is a different signal — it MUST
            // not be misread as a ToolUse start (would fire ⚙️ on a
            // tool that just errored).
            assert_eq!(
                extract_tool_marker_name("\n\n⚠️ system_time failed\n\n"),
                None
            );
        }

        #[test]
        fn rejects_marker_inside_prose() {
            // We rely on the api channel bridge sending each marker as
            // its own `tx.send(line)` — a `🔧` literal that appears
            // inside model-authored prose must NOT be treated as a
            // marker, since we'd extract the wrong "tool name" and
            // fire a phantom reaction.
            assert_eq!(
                extract_tool_marker_name("Sure, I'll use 🔧 to fix it."),
                None
            );
        }

        #[test]
        fn rejects_marker_missing_suffix() {
            // The api bridge always emits `\n\n…\n\n`. If the suffix
            // is missing the delta is malformed; fail closed rather
            // than guess.
            assert_eq!(extract_tool_marker_name("\n\n🔧 system_time"), None);
        }

        #[test]
        fn rejects_empty_tool_name() {
            assert_eq!(extract_tool_marker_name("\n\n🔧 \n\n"), None);
            assert_eq!(extract_tool_marker_name("\n\n🔧    \n\n"), None);
        }
    }

    // -----------------------------------------------------------------------
    // MIME sniff integration — verifies the detection pipeline end-to-end:
    // bytes served as application/octet-stream over HTTP → detect_audio_magic
    // returns the correct type.  Uses fetch_url_bytes_unchecked (which skips
    // the SSRF guard) so wiremock's 127.0.0.1 binding is reachable from tests.
    // -----------------------------------------------------------------------

    mod audio_mime_sniff {
        use super::super::{audio_mime_from_filename, detect_audio_magic};
        use crate::http_client::fetch_url_bytes_unchecked;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// OGG bytes served with Content-Type: application/octet-stream.
        /// Asserts detect_audio_magic correctly identifies audio/ogg from the
        /// magic bytes, and that audio_mime_from_filename agrees via extension.
        /// This locks BLOCKER-1: the sniff logic that runs before enrich must
        /// produce "audio/ogg", not "application/octet-stream".
        #[tokio::test]
        async fn ogg_served_as_octet_stream_is_detected_as_audio_ogg() {
            // Minimal OGG bytes: OggS magic + padding to fill 12-byte buffer.
            let mut ogg_bytes = vec![
                0x4F, 0x67, 0x67, 0x53, // OggS
                0x00, 0x02, 0x00, 0x00, // version + header type
                0x00, 0x00, 0x00, 0x00, // granule position (low)
            ];
            ogg_bytes.extend_from_slice(&[0u8; 64]);

            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/file/bot123/voice/file_136.oga"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(ogg_bytes),
                )
                .mount(&server)
                .await;

            let url = format!("{}/file/bot123/voice/file_136.oga", server.uri());
            let client = crate::http_client::new_client();
            let (body, content_type) = fetch_url_bytes_unchecked(&client, &url, 1024 * 1024, &[])
                .await
                .expect("fetch succeeded");

            // Server sends application/octet-stream — same as Telegram CDN.
            assert_eq!(
                content_type.as_deref(),
                Some("application/octet-stream"),
                "expected server to return application/octet-stream"
            );

            // Magic-byte sniff must recover audio/ogg from the first 12 bytes.
            let magic = detect_audio_magic(&body[..body.len().min(12)]);
            assert_eq!(
                magic,
                Some("audio/ogg"),
                "detect_audio_magic should identify OGG bytes as audio/ogg"
            );

            // Filename fallback also agrees (extension .oga).
            let fname_mime = audio_mime_from_filename("file_136.oga");
            assert_eq!(fname_mime, Some("audio/ogg"));
        }
    }
}
