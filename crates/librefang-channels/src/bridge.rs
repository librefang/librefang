//! Channel bridge — connects channel adapters to the LibreFang kernel.
//!
//! Defines `ChannelBridgeHandle` (implemented by librefang-api on the kernel) and
//! `BridgeManager` which owns running adapters and dispatches messages.

use crate::formatter;
use crate::rate_limiter::ChannelRateLimiter;
use crate::roster::GroupRosterStore;
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
    AutoRouteStrategy, ChannelOverrides, DmPolicy, GroupPolicy, OutputFormat,
};
use librefang_types::message::ContentBlock;
use regex::{Regex, RegexSet};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

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

    /// Return the uploads directory for an agent's workspace.
    ///
    /// Channel files (images, documents) are saved here so agent tools can
    /// access them inside the workspace sandbox.  Returns `None` if the agent
    /// has no workspace configured.
    async fn agent_upload_dir(&self, _agent_id: AgentId) -> Option<std::path::PathBuf> {
        None
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

    /// Reset an agent's session (clear messages, fresh session ID).
    async fn reset_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Hard-reboot an agent's session — full context clear without saving summary.
    async fn reboot_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Trigger LLM-based session compaction for an agent.
    async fn compact_session(&self, _agent_id: AgentId) -> Result<String, String> {
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
    ) -> Option<tokio::sync::broadcast::Receiver<librefang_types::event::Event>> {
        None
    }

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
        let _ = tx.send(response).await;
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
        let _ = status_tx.send(Ok(()));
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

    /// Lightweight LLM precheck: should the agent reply to this group message?
    ///
    /// `context` contains recent conversation lines (e.g. the message being
    /// replied to) so the classifier can detect continuations.
    ///
    /// Returns `Ok(true)` → reply, `Ok(false)` → stay silent.
    /// On error the caller fails open (replies anyway).
    /// Default: always reply (opt-in via `reply_precheck = true` in channel config).
    async fn classify_reply_intent(
        &self,
        _agent_id: AgentId,
        _message: &str,
        _context: Option<&str>,
    ) -> Result<bool, String> {
        Ok(true)
    }

    /// Read the custom precheck prompt from the agent's workspace
    /// (`PRECHECK.md`). Returns `None` if the file doesn't exist.
    async fn get_precheck_prompt(&self, _agent_id: AgentId) -> Option<String> {
        None
    }

    /// Get the routing aliases for an agent (from `[metadata.routing].aliases`).
    /// Default: empty (no aliases).
    async fn get_agent_aliases(&self, _agent_id: AgentId) -> Vec<String> {
        Vec::new()
    }

    /// Upsert a member into the persistent group roster (SQLite-backed).
    async fn roster_upsert(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
        _display_name: &str,
        _username: Option<&str>,
    ) {
        // default no-op
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

struct MessageDebouncer {
    debounce_ms: u64,
    debounce_max_ms: u64,
    max_buffer: usize,
    flush_tx: mpsc::UnboundedSender<String>,
}

impl MessageDebouncer {
    fn new(
        debounce_ms: u64,
        debounce_max_ms: u64,
        max_buffer: usize,
    ) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (flush_tx, flush_rx) = mpsc::unbounded_channel();
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
                let _ = flush_tx.send(flush_key);
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
            let _ = self.flush_tx.send(key.to_string());
            return;
        }

        let remaining_cap = max_dur.saturating_sub(elapsed);
        let delay = debounce_dur.min(remaining_cap);
        let flush_tx = self.flush_tx.clone();
        let flush_key = key.to_string();
        buf.timer_handle = Some(tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = flush_tx.send(flush_key);
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
            let _ = self.flush_tx.send(key.to_string());
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
                let _ = flush_tx.send(flush_key);
            }));
        }
    }

    fn drain(
        &self,
        key: &str,
        buffers: &mut HashMap<String, SenderBuffer>,
    ) -> Option<(ChannelMessage, Option<Vec<ContentBlock>>)> {
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
        ChannelContent::File { url, filename } => {
            // After download_file_to_local, url is a local path
            if url.starts_with('/') || url.starts_with("./") {
                format!("[File: {filename} — saved at {url}]")
            } else {
                format!("[File ({filename}): {url}]")
            }
        }
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
) -> Option<tokio::task::JoinHandle<()>> {
    let (merged_msg, blocks) = debouncer.drain(key, buffers)?;

    let channel_handle = (*handle).clone();
    let router = router.clone();
    let adapter = adapter.clone();
    let rate_limiter = rate_limiter.clone();
    let sanitizer = Arc::clone(sanitizer);
    let journal = journal.clone();
    let sem = semaphore.clone();

    let join_handle = tokio::spawn(async move {
        let _permit = match sem.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        if let Some(mut blocks) = blocks {
            // When we already have image content blocks, only prepend the
            // caption text — NOT the `[Photo: URL]` string that
            // `content_to_text` would produce, because the raw Telegram URL
            // confuses vision models into thinking they must fetch it.
            let text = match &merged_msg.content {
                ChannelContent::Image {
                    caption: Some(c), ..
                } => c.clone(),
                ChannelContent::Image { caption: None, .. } => String::new(),
                other => content_to_text(other),
            };
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
                let text_to_check: Option<&str> = match &merged_msg.content {
                    ChannelContent::Text(t) => Some(t.as_str()),
                    ChannelContent::Image { caption, .. } => caption.as_deref(),
                    ChannelContent::Voice { caption, .. } => caption.as_deref(),
                    ChannelContent::Video { caption, .. } => caption.as_deref(),
                    _ => None,
                };
                if let Some(text) = text_to_check {
                    match sanitizer.check(text) {
                        SanitizeResult::Clean => {}
                        SanitizeResult::Warned(reason) => {
                            warn!(
                                channel = ct_str,
                                user = %merged_msg.sender.display_name,
                                reason = reason.as_str(),
                                "Suspicious channel input (warn mode, allowing through)"
                            );
                        }
                        SanitizeResult::Blocked(reason) => {
                            warn!(
                                channel = ct_str,
                                user = %merged_msg.sender.display_name,
                                reason = reason.as_str(),
                                "Blocked channel input (prompt injection detected)"
                            );
                            let _ = adapter
                                .send(
                                    &merged_msg.sender,
                                    ChannelContent::Text(
                                        "Your message could not be processed.".to_string(),
                                    ),
                                )
                                .await;
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

                                    // Resolve workspace uploads dir for the channel's
                                    // default agent so downloaded files land inside the
                                    // sandbox where agent tools can read them.
                                    // Router keys use Debug format ("Telegram"), not
                                    // the lowercase config key ("telegram").
                                    let ct_key = format!("{:?}", message.channel);
                                    let ws_upload_dir = match router.channel_default(&ct_key) {
                                        Some(aid) => handle.agent_upload_dir(aid).await,
                                        None => None,
                                    };

                                    // Download remote files to local storage so
                                    // agent tools (file_read, media_describe) can
                                    // access them after the remote URL expires.
                                    let mut message = message;
                                    if let ChannelContent::File { ref url, ref filename } = message.content {
                                        if let Some(local_path) = download_file_to_local(url, filename, ws_upload_dir.as_deref()).await {
                                            message.content = ChannelContent::File {
                                                url: local_path,
                                                filename: filename.clone(),
                                            };
                                        }
                                    }

                                    let image_blocks = if let ChannelContent::Image {
                                        ref url, ref caption, ref mime_type
                                    } = message.content {
                                        match download_image_to_blocks(url, caption.as_deref(), mime_type.as_deref(), ws_upload_dir.as_deref()).await {
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
                                        if let Some(handle) = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal) {
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
                            let _ = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal);
                        }
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                let keys: Vec<String> = buffers.keys().cloned().collect();
                                let mut handles = Vec::new();
                                for key in keys {
                                    if let Some(handle) = flush_debounced(&debouncer, &key, &mut buffers, &handle, &router, &adapter_clone, &rate_limiter, &sanitizer, &semaphore, &journal) {
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
    /// to all running channel adapters as interactive approval messages.
    ///
    /// Each adapter receives a text notification about the pending approval.
    /// Adapters that support inline keyboards (e.g. Telegram) can later be
    /// extended to send interactive buttons; for now we send a text prompt
    /// with the approval ID so users can `/approve <id>` or `/reject <id>`.
    pub async fn start_approval_listener(&mut self, adapters: Vec<Arc<dyn ChannelAdapter>>) {
        let maybe_rx = self.handle.subscribe_events().await;
        let Some(mut rx) = maybe_rx else {
            debug!("Event subscription not available — approval listener not started");
            return;
        };

        let mut shutdown = self.shutdown_rx.clone();
        let handle = self.handle.clone();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = rx.recv() => {
                        match result {
                            Ok(event) => {
                                if let librefang_types::event::EventPayload::ApprovalRequested(ref approval) = event.payload {
                                    let msg = format!(
                                        "Approval required for agent {}\n\
                                         Tool: {}\n\
                                         Risk: {}\n\
                                         {}\n\n\
                                         Reply /approve {} or /reject {}",
                                        approval.agent_id,
                                        approval.tool_name,
                                        approval.risk_level,
                                        approval.description,
                                        &approval.request_id[..8.min(approval.request_id.len())],
                                        &approval.request_id[..8.min(approval.request_id.len())],
                                    );

                                    // Send to all adapters (best-effort). Each adapter
                                    // gets the notification so the user sees it on
                                    // whichever channel they are active on.
                                    for adapter in &adapters {
                                        // We don't have a specific user to send to, so
                                        // this is a broadcast-style notification. Adapters
                                        // that don't support broadcast will simply skip.
                                        // For now, log the notification — concrete delivery
                                        // requires per-adapter user tracking which is a
                                        // follow-up feature.
                                        info!(
                                            adapter = adapter.name(),
                                            request_id = %approval.request_id,
                                            "Approval notification ready for channel adapter"
                                        );
                                    }

                                    let _ = &msg; // Suppress unused variable warning
                                    let _ = &handle;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                warn!("Approval event listener lagged by {n} events");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                info!("Event bus closed — stopping approval listener");
                                break;
                            }
                        }
                    }
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            info!("Shutting down approval event listener");
                            break;
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

    pub async fn stop(&mut self) {
        // Signal the dispatch loops to stop
        let _ = self.shutdown_tx.send(true);

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

/// Convert plain alias strings into case-insensitive word-boundary regex patterns
/// suitable for use in `group_trigger_patterns`.
///
/// This lets operators avoid manually translating agent aliases into regex syntax:
/// `aliases_to_trigger_patterns(&["fandango", "oye fandango"])` produces
/// `["(?i)\\bfandango\\b", "(?i)\\boye fandango\\b"]`.
pub fn aliases_to_trigger_patterns(aliases: &[String]) -> Vec<String> {
    aliases
        .iter()
        .map(|alias| {
            let escaped = regex::escape(alias);
            format!("(?i)\\b{escaped}\\b")
        })
        .collect()
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
pub fn is_vocative_trigger(text: &str, pattern: &str) -> bool {
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
pub fn is_addressed_to_other_participant(
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
    agent_aliases: &[String],
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

            let alias_triggered = !was_mentioned
                && !is_command
                && !regex_triggered
                && text_content(message).is_some_and(|text| {
                    let lower = text.to_lowercase();
                    agent_aliases
                        .iter()
                        .any(|alias| lower.contains(&alias.to_lowercase()))
                });
            if alias_triggered {
                debug!(
                    channel = ct_str,
                    user = %message.sender.display_name,
                    "Group message matched agent alias trigger"
                );
            }

            if !was_mentioned && !is_command && !regex_triggered && !alias_triggered {
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
/// Singleton in-memory group roster shared across all channel adapters.
///
/// Populated on every incoming group message from `build_sender_context`. The
/// accumulated roster is then handed to the agent's system prompt so the LLM
/// can distinguish the current message sender from other group members it has
/// seen before (e.g. when a user writes `@pepe` meaning another human, not an
/// agent in the system).
fn group_roster() -> &'static GroupRosterStore {
    static ROSTER: OnceLock<GroupRosterStore> = OnceLock::new();
    ROSTER.get_or_init(GroupRosterStore::new)
}

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

    let channel = channel_type_str(&message.channel).to_string();
    let chat_id = message
        .metadata
        .get("chat_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            if message.sender.platform_id.is_empty() {
                None
            } else {
                Some(message.sender.platform_id.clone())
            }
        });
    let bot_username = message
        .metadata
        .get("bot_username")
        .and_then(|v| v.as_str())
        .map(String::from);
    let sender_username = message
        .metadata
        .get("sender_username")
        .and_then(|v| v.as_str())
        .map(String::from);

    // For group messages, upsert the current sender into the roster and then
    // read back all known members. For DMs the roster stays empty — the LLM
    // only needs the current sender and doesn't gain anything from a roster
    // with a single member.
    let group_members: Vec<GroupMember> = if message.is_group {
        if let Some(ref cid) = chat_id {
            let current = GroupMember {
                user_id: sender_user_id(message).to_string(),
                display_name: message.sender.display_name.clone(),
                username: sender_username.clone(),
            };
            let store = group_roster();
            store.upsert(&channel, cid, current);
            Vec::new() // Roster is now tool-based, not injected into prompt
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    SenderContext {
        channel,
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
        bot_username,
        sender_username,
        group_members,
        // §C: forward roster from inbound payload (gateway populates via
        // sock.groupMetadata). Empty for non-WhatsApp channels — addressee
        // guard then becomes a no-op (BC-01).
        group_participants: extract_group_participants(message),
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

/// Send a lifecycle reaction (best-effort, non-blocking for supported adapters).
///
/// Silently ignores errors — reactions are non-critical UX polish.
/// For Telegram, the underlying HTTP call is already fire-and-forget (spawned internally),
/// so this await returns almost immediately.
async fn send_lifecycle_reaction(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    message_id: &str,
    phase: AgentPhase,
) {
    let reaction = LifecycleReaction {
        emoji: default_phase_emoji(&phase).to_string(),
        phase,
        remove_previous: true,
    };
    let _ = adapter.send_reaction(user, message_id, &reaction).await;
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
    send_fn: F,
) where
    F: FnOnce(AgentId) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    // Try re-resolution for stale agent IDs
    if let Some(new_id) = try_reresolution(error, agent_id, channel_key, handle, router).await {
        send_lifecycle_reaction(adapter, sender, msg_id, AgentPhase::Thinking).await;

        match send_fn(new_id).await {
            Ok(response) => {
                send_lifecycle_reaction(adapter, sender, msg_id, AgentPhase::Done).await;
                if !response.is_empty() {
                    send_response(adapter, sender, response, thread_id, output_format).await;
                }
                handle
                    .record_delivery(new_id, ct_str, &sender.platform_id, true, None, thread_id)
                    .await;
                return;
            }
            Err(e2) => {
                // Re-resolution succeeded but the retry failed — report retry error
                send_lifecycle_reaction(adapter, sender, msg_id, AgentPhase::Error).await;
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
    send_lifecycle_reaction(adapter, sender, msg_id, AgentPhase::Error).await;
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
async fn dispatch_message(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    rate_limiter: &ChannelRateLimiter,
    sanitizer: &InputSanitizer,
    journal: Option<&crate::message_journal::MessageJournal>,
) {
    let ct_str = channel_type_str(&message.channel);

    // --- Input sanitization (prompt injection detection) ---
    if !sanitizer.is_off() {
        let text_to_check: Option<&str> = match &message.content {
            ChannelContent::Text(t) => Some(t.as_str()),
            ChannelContent::Image { caption, .. } => caption.as_deref(),
            ChannelContent::Voice { caption, .. } => caption.as_deref(),
            ChannelContent::Video { caption, .. } => caption.as_deref(),
            _ => None,
        };
        if let Some(text) = text_to_check {
            match sanitizer.check(text) {
                SanitizeResult::Clean => {}
                SanitizeResult::Warned(reason) => {
                    warn!(
                        channel = ct_str,
                        user = %message.sender.display_name,
                        reason = reason.as_str(),
                        "Suspicious channel input (warn mode, allowing through)"
                    );
                }
                SanitizeResult::Blocked(reason) => {
                    warn!(
                        channel = ct_str,
                        user = %message.sender.display_name,
                        reason = reason.as_str(),
                        "Blocked channel input (prompt injection detected)"
                    );
                    let _ = adapter
                        .send(
                            &message.sender,
                            ChannelContent::Text(
                                "Your message could not be processed.".to_string(),
                            ),
                        )
                        .await;
                    return;
                }
            }
        }
    }

    // Fetch per-channel overrides (if configured)
    let overrides = handle
        .channel_overrides(
            ct_str,
            message.metadata.get("account_id").and_then(|v| v.as_str()),
        )
        .await;
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
            if !should_process_group_message(ct_str, ov, message, &[]) {
                return;
            }
            // Reply-intent precheck runs post-resolution (after agent_id is
            // known) so it can use per-agent PRECHECK.md prompts and reply
            // context.  See the "Post-resolution group filters" block below.
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

    // Resolve workspace uploads dir so downloaded files land inside the sandbox.
    // Router keys use Debug format ("Telegram"), not the lowercase config key.
    let ct_key = format!("{:?}", message.channel);
    let ws_upload_dir = match router.channel_default(&ct_key) {
        Some(aid) => handle.agent_upload_dir(aid).await,
        None => None,
    };

    // For files: download to local storage so agent tools can access them
    // after the remote URL expires (e.g. Telegram file links are short-lived).
    // We create a modified copy since `message` is borrowed immutably.
    let message = if let ChannelContent::File {
        ref url,
        ref filename,
    } = message.content
    {
        if let Some(local_path) =
            download_file_to_local(url, filename, ws_upload_dir.as_deref()).await
        {
            let mut m = message.clone();
            m.content = ChannelContent::File {
                url: local_path,
                filename: filename.clone(),
            };
            std::borrow::Cow::Owned(m)
        } else {
            std::borrow::Cow::Borrowed(message)
        }
    } else {
        std::borrow::Cow::Borrowed(message)
    };
    let message = message.as_ref();

    // For images: download, base64 encode, and send as multimodal content blocks
    if let ChannelContent::Image {
        ref url,
        ref caption,
        ref mime_type,
    } = message.content
    {
        let blocks = download_image_to_blocks(
            url,
            caption.as_deref(),
            mime_type.as_deref(),
            ws_upload_dir.as_deref(),
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
            )
            .await;
            return;
        }
        // Image download failed — fall through to text description below
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

        if matches!(
            cmd,
            "start"
                | "help"
                | "agents"
                | "agent"
                | "status"
                | "models"
                | "providers"
                | "new"
                | "reboot"
                | "compact"
                | "model"
                | "stop"
                | "usage"
                | "think"
                | "skills"
                | "hands"
                | "btw"
                | "workflows"
                | "workflow"
                | "triggers"
                | "trigger"
                | "schedules"
                | "schedule"
                | "approvals"
                | "approve"
                | "reject"
                | "budget"
                | "peers"
                | "a2a"
        ) {
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
            let _ = adapter.send_typing(&message.sender).await;

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

    // --- Post-resolution group filters (alias awareness + reply-intent precheck) ---
    if message.is_group {
        let was_mentioned = message
            .metadata
            .get("was_mentioned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Reply-intent precheck: lightweight LLM call to decide if the agent
        // should reply. Only for non-mentioned group messages when enabled.
        // Skipped for @mentions, commands, and DMs (those always get a reply).
        if !was_mentioned && !matches!(message.content, ChannelContent::Command { .. }) {
            // Check agent aliases first — if a message contains an alias like
            // "oye fandango", treat it as an implicit mention (skip precheck).
            let aliases = handle.get_agent_aliases(agent_id).await;
            let alias_hit = !aliases.is_empty()
                && text_content(message).is_some_and(|text| {
                    let lower = text.to_lowercase();
                    aliases.iter().any(|a| lower.contains(&a.to_lowercase()))
                });

            if alias_hit {
                debug!(
                    channel = ct_str,
                    "Group message matched agent alias — treating as mention"
                );
            } else if let Some(ref ov) = overrides {
                if ov.reply_precheck {
                    // No mention, no alias — run LLM precheck.
                    // Include reply-to context so the classifier can detect
                    // conversation continuations (e.g. user replying to the bot).
                    let reply_context: Option<String> =
                        message.metadata.get("reply_to").and_then(|rt| {
                            let sender = rt
                                .get("sender")
                                .and_then(|s| s.as_str())
                                .unwrap_or("someone");
                            let text = rt.get("text").and_then(|t| t.as_str())?;
                            Some(format!("[Replying to {sender}: \"{text}\"]"))
                        });
                    if let Some(text) = text_content(message) {
                        match handle
                            .classify_reply_intent(agent_id, text, reply_context.as_deref())
                            .await
                        {
                            Ok(true) => {
                                debug!(channel = ct_str, "Reply-intent precheck: REPLY");
                            }
                            Ok(false) => {
                                debug!(
                                    channel = ct_str,
                                    "Reply-intent precheck: NO_REPLY — staying silent"
                                );
                                return;
                            }
                            Err(e) => {
                                warn!(channel = ct_str, error = %e, "Reply-intent precheck failed — proceeding with reply");
                            }
                        }
                    }
                }
            }
        }
    }

    let channel_key = format!("{:?}", message.channel);

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
        };
        j.record(entry).await;
    }

    // Send typing indicator (best-effort)
    let _ = adapter.send_typing(&message.sender).await;

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Thinking).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message, overrides.as_ref());

    // Persist roster member to SQLite
    if message.is_group {
        if let Some(ref cid) = sender_ctx.chat_id {
            handle
                .roster_upsert(
                    ct_str,
                    cid,
                    &sender_ctx.user_id,
                    &sender_ctx.display_name,
                    sender_ctx.sender_username.as_deref(),
                )
                .await;
        }
    }

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
                send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Streaming)
                    .await;

                // Tee: forward deltas to the adapter while buffering a copy.
                // If send_streaming fails, the buffer lets us fall back to send().
                let (adapter_tx, adapter_rx) = mpsc::channel::<String>(64);
                let mut buffered_text = String::new();
                let buffer_handle = tokio::spawn({
                    let mut buffered = String::new();
                    async move {
                        while let Some(delta) = delta_rx.recv().await {
                            buffered.push_str(&delta);
                            // Best-effort forward — if adapter dropped rx, stop.
                            if adapter_tx.send(delta).await.is_err() {
                                break;
                            }
                        }
                        buffered
                    }
                });

                let stream_result = adapter
                    .send_streaming(&message.sender, adapter_rx, thread_id)
                    .await;

                // Collect the buffered text (always succeeds unless the task panicked).
                if let Ok(text) = buffer_handle.await {
                    buffered_text = text;
                }

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
                        send_lifecycle_reaction(adapter, &message.sender, msg_id, phase).await;
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
                            let jstatus = if kernel_ok {
                                crate::message_journal::JournalStatus::Completed
                            } else {
                                crate::message_journal::JournalStatus::Failed
                            };
                            j.update_status(
                                &message.platform_message_id,
                                jstatus,
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
                            send_lifecycle_reaction(adapter, &message.sender, msg_id, phase).await;
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
                                let jstatus = if kernel_ok {
                                    crate::message_journal::JournalStatus::Completed
                                } else {
                                    crate::message_journal::JournalStatus::Failed
                                };
                                j.update_status(&message.platform_message_id, jstatus, err_str)
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
                            AgentPhase::Error,
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
                            j.update_status(
                                &message.platform_message_id,
                                crate::message_journal::JournalStatus::Failed,
                                Some(err_str),
                            )
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
        send_lifecycle_reaction(adapter, &message.sender, msg_id, phase).await;
        if !accumulated.is_empty() && (success || !adapter.suppress_error_responses()) {
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
            let jstatus = if success {
                crate::message_journal::JournalStatus::Completed
            } else {
                crate::message_journal::JournalStatus::Failed
            };
            j.update_status(&message.platform_message_id, jstatus, err_str)
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
            send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Done).await;
            if !response.is_empty() {
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
                j.update_status(
                    &message.platform_message_id,
                    crate::message_journal::JournalStatus::Completed,
                    None,
                )
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
                j.update_status(
                    &message.platform_message_id,
                    crate::message_journal::JournalStatus::Failed,
                    Some(e.to_string()),
                )
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

/// Download a file from a URL and save it locally so agent tools can access it
/// after the remote URL expires (e.g. Telegram file links are short-lived).
///
/// Returns the local path on success, or `None` if the download fails.
/// Compatible with all channels — any `ChannelContent::File` gets persisted.
async fn download_file_to_local(
    url: &str,
    filename: &str,
    upload_dir: Option<&std::path::Path>,
) -> Option<String> {
    const MAX_FILE_BYTES: usize = 50 * 1024 * 1024; // 50 MB limit

    let client = crate::http_client::new_client();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to download file from channel: {e}");
            return None;
        }
    };

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read file bytes: {e}");
            return None;
        }
    };

    if bytes.len() > MAX_FILE_BYTES {
        warn!(
            size = bytes.len(),
            limit = MAX_FILE_BYTES,
            "File too large, skipping download"
        );
        return None;
    }

    // Save to workspace uploads/ dir (inside sandbox) or /tmp/ as fallback
    let fallback = std::env::temp_dir().join("librefang_uploads");
    let upload_dir = upload_dir.unwrap_or(&fallback);
    if let Err(e) = tokio::fs::create_dir_all(upload_dir).await {
        warn!("Failed to create upload dir: {e}");
        return None;
    }

    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let local_name = format!("{}_{}.{}", uuid::Uuid::new_v4(), filename, ext);
    let local_path = upload_dir.join(&local_name);

    if let Err(e) = tokio::fs::write(&local_path, &bytes).await {
        warn!("Failed to write file to {}: {e}", local_path.display());
        return None;
    }

    debug!(
        filename = filename,
        path = %local_path.display(),
        size = bytes.len(),
        "Downloaded channel file to local storage"
    );

    Some(local_path.to_string_lossy().to_string())
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
    upload_dir: Option<&std::path::Path>,
) -> Vec<ContentBlock> {
    use base64::Engine;

    // 5 MB limit to prevent memory abuse from oversized images
    const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

    let client = crate::http_client::new_client();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to download image from channel: {e}");
            return vec![ContentBlock::Text {
                text: format!("[Image download failed: {e}]"),
                provider_metadata: None,
            }];
        }
    };

    // Detect media type from Content-Type header — but only trust it if it's
    // actually an image/* type. Many APIs (Telegram, S3 pre-signed URLs) return
    // `application/octet-stream` for all files, which breaks vision.
    let header_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim().to_string())
        .filter(|ct| ct.starts_with("image/"));

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read image bytes: {e}");
            return vec![ContentBlock::Text {
                text: format!("[Image read failed: {e}]"),
                provider_metadata: None,
            }];
        }
    };

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

    if bytes.len() > MAX_IMAGE_BYTES {
        warn!(
            "Image too large ({} bytes), skipping vision — sending as text",
            bytes.len()
        );
        let desc = match caption {
            Some(c) => format!(
                "[Image too large for vision ({} KB)]\nCaption: {c}",
                bytes.len() / 1024
            ),
            None => format!("[Image too large for vision ({} KB)]", bytes.len() / 1024),
        };
        return vec![ContentBlock::Text {
            text: desc,
            provider_metadata: None,
        }];
    }

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
    // Use workspace uploads/ dir (inside sandbox) or /tmp/ as fallback.
    let fallback = std::env::temp_dir().join("librefang_uploads");
    let upload_dir = upload_dir.unwrap_or(&fallback).to_path_buf();

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
    let channel_key = format!("{:?}", message.channel);

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
        };
        j.record(entry).await;
    }

    let _ = adapter.send_typing(&message.sender).await;

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Thinking).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message, overrides);

    // Persist roster member to SQLite
    if message.is_group {
        if let Some(ref cid) = sender_ctx.chat_id {
            handle
                .roster_upsert(
                    ct_str,
                    cid,
                    &sender_ctx.user_id,
                    &sender_ctx.display_name,
                    sender_ctx.sender_username.as_deref(),
                )
                .await;
        }
    }

    match handle
        .send_message_with_blocks_and_sender(agent_id, blocks.clone(), &sender_ctx)
        .await
    {
        Ok(response) => {
            send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Done).await;
            if !response.is_empty() {
                send_response(adapter, &message.sender, response, thread_id, output_format).await;
            }
            if let Some(j) = journal {
                j.update_status(
                    &message.platform_message_id,
                    crate::message_journal::JournalStatus::Completed,
                    None,
                )
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
                j.update_status(
                    &message.platform_message_id,
                    crate::message_journal::JournalStatus::Failed,
                    Some(e.to_string()),
                )
                .await;
            }
        }
    }
}

/// Handle a bot command (returns the response text).
async fn handle_command(
    name: &str,
    args: &[String],
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    sender: &ChannelUser,
    channel_type: &crate::types::ChannelType,
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
        "help" => "LibreFang Bot Commands:\n\
             \n\
             Session:\n\
             /agents - list running agents\n\
             /agent <name> - select which agent to talk to\n\
             /new - reset session (clear messages)\n\
             /reboot - hard reset session (full context clear, no summary)\n\
             /compact - trigger LLM session compaction\n\
             /model [name] - show or switch agent model\n\
             /stop - cancel current agent run\n\
             /usage - show session token usage and cost\n\
             /think [on|off] - toggle extended thinking\n\
             \n\
             Info:\n\
             /models - list available AI models\n\
             /providers - show configured providers\n\
             /skills - list installed skills\n\
             /hands - list available and active hands\n\
             /status - show system status\n\
             \n\
             Automation:\n\
             /workflows - list workflows\n\
             /workflow run <name> [input] - run a workflow\n\
             /triggers - list event triggers\n\
             /trigger add <agent> <pattern> <prompt> - create trigger\n\
             /trigger del <id> - remove trigger\n\
             /schedules - list cron jobs\n\
             /schedule add <agent> <cron-5-fields> <message> - create job\n\
             /schedule del <id> - remove job\n\
             /schedule run <id> - run job now\n\
             /approvals - list pending approvals\n\
             /approve <id> - approve a request\n\
             /reject <id> - reject a request\n\
             \n\
             Monitoring:\n\
             /budget - show spending limits and current costs\n\
             /peers - show OFP peer network status\n\
             /a2a - list discovered external A2A agents\n\
             \n\
             /btw <question> - ask a side question (ephemeral, not saved to session)\n\
             \n\
             /start - show welcome message\n\
             /help - show this help"
            .to_string(),
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
            // Need to resolve the user's current agent
            let agent_id = router.resolve(
                channel_type,
                &sender.platform_id,
                sender.librefang_user.as_deref(),
            );
            match agent_id {
                Some(aid) => handle
                    .reset_session(aid)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
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
                Some(aid) => handle
                    .reboot_session(aid)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
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
                Some(aid) => handle
                    .compact_session(aid)
                    .await
                    .unwrap_or_else(|e| format!("Error: {e}")),
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

        let result =
            handle_command("agents", &[], &handle, &router, &sender, &ChannelType::CLI).await;
        assert!(result.contains("coder"));

        let result =
            handle_command("help", &[], &handle, &router, &sender, &ChannelType::CLI).await;
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
                "whatsapp",
                &overrides,
                &message,
                &[]
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
                "whatsapp",
                &overrides,
                &message,
                &[]
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
                "telegram",
                &overrides,
                &message,
                &[]
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
                "telegram",
                &overrides,
                &message,
                &[]
            ));
        });
    }

    #[test]
    fn test_mention_only_triggers_on_agent_alias() {
        let message = group_text_message("hey fandango what do you think?");
        let overrides = ChannelOverrides::default();
        let aliases = vec!["fandango".to_string()];
        assert!(should_process_group_message(
            "telegram", &overrides, &message, &aliases
        ));
    }

    #[test]
    fn test_mention_only_alias_is_case_insensitive() {
        let message = group_text_message("FANDANGO help me");
        let overrides = ChannelOverrides::default();
        let aliases = vec!["Fandango".to_string()];
        assert!(should_process_group_message(
            "telegram", &overrides, &message, &aliases
        ));
    }

    #[test]
    fn test_mention_only_rejects_no_alias_match() {
        let message = group_text_message("hello there, anyone?");
        let overrides = ChannelOverrides::default();
        let aliases = vec!["fandango".to_string(), "rodelo".to_string()];
        assert!(!should_process_group_message(
            "telegram", &overrides, &message, &aliases
        ));
    }

    #[test]
    fn test_mention_only_alias_does_not_override_regex_trigger() {
        // regex trigger and alias can independently activate — alias fires here
        let message = group_text_message("oye fandango");
        let overrides = ChannelOverrides::default();
        let aliases = vec!["oye fandango".to_string()];
        assert!(should_process_group_message(
            "telegram", &overrides, &message, &aliases
        ));
    }

    #[test]
    fn test_aliases_to_trigger_patterns_produces_word_boundary_regex() {
        let aliases = vec!["fandango".to_string(), "oye fandango".to_string()];
        let patterns = aliases_to_trigger_patterns(&aliases);
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0], r"(?i)\bfandango\b");
        assert_eq!(patterns[1], r"(?i)\boye fandango\b");
    }

    #[test]
    fn test_aliases_to_trigger_patterns_escapes_special_chars() {
        let aliases = vec!["bot.v2".to_string()];
        let patterns = aliases_to_trigger_patterns(&aliases);
        assert_eq!(patterns[0], r"(?i)\bbot\.v2\b");
    }

    #[test]
    fn test_aliases_to_trigger_patterns_empty() {
        let patterns = aliases_to_trigger_patterns(&[]);
        assert!(patterns.is_empty());
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

        let result = handle_command("btw", &[], &handle, &router, &sender, &ChannelType::CLI).await;
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

        let result =
            handle_command("help", &[], &handle, &router, &sender, &ChannelType::CLI).await;
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
            std::env::set_var("LIBREFANG_GROUP_ADDRESSEE_GUARD", "on");
            f();
            std::env::remove_var("LIBREFANG_GROUP_ADDRESSEE_GUARD");
        }

        fn with_guard_off<F: FnOnce()>(f: F) {
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            std::env::remove_var("LIBREFANG_GROUP_ADDRESSEE_GUARD");
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
}
