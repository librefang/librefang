//! Channel bridge — connects channel adapters to the LibreFang kernel.
//!
//! Defines `ChannelBridgeHandle` (implemented by librefang-api on the kernel) and
//! `BridgeManager` which owns running adapters and dispatches messages.

use crate::formatter;
use crate::rate_limiter::ChannelRateLimiter;
use crate::router::AgentRouter;
use crate::sanitizer::{InputSanitizer, SanitizeResult};
use crate::types::{
    default_phase_emoji, AgentPhase, ChannelAdapter, ChannelContent, ChannelMessage, ChannelUser,
    LifecycleReaction, SenderContext,
};
use async_trait::async_trait;
use futures::StreamExt;
use librefang_types::agent::AgentId;
use librefang_types::config::{ChannelOverrides, DmPolicy, GroupPolicy, OutputFormat};
use librefang_types::message::ContentBlock;
use regex::RegexSet;
use std::sync::{Arc, OnceLock};
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
    async fn resolve_approval_text(&self, _id_prefix: &str, _approve: bool) -> String {
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
        let stream = adapter.start().await?;
        let handle = self.handle.clone();
        let router = self.router.clone();
        let rate_limiter = self.rate_limiter.clone();
        let sanitizer = self.sanitizer.clone();
        let adapter_clone = adapter.clone();
        let mut shutdown = self.shutdown_rx.clone();

        // Limit concurrent dispatch tasks to prevent unbounded growth.
        // 32 is generous — most setups have 1-5 concurrent users.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(32));

        let task = tokio::spawn(async move {
            let mut stream = std::pin::pin!(stream);
            loop {
                tokio::select! {
                    msg = stream.next() => {
                        match msg {
                            Some(message) => {
                                // Spawn each dispatch as a concurrent task so the stream
                                // loop is never blocked by slow LLM calls. The kernel's
                                // per-agent lock ensures session integrity.
                                let handle = handle.clone();
                                let router = router.clone();
                                let adapter = adapter_clone.clone();
                                let rate_limiter = rate_limiter.clone();
                                let sanitizer = sanitizer.clone();
                                let sem = semaphore.clone();
                                tokio::spawn(async move {
                                    // Acquire semaphore permit (blocks if 32 tasks are in flight).
                                    let _permit = match sem.acquire().await {
                                        Ok(p) => p,
                                        Err(_) => return, // semaphore closed — shutting down
                                    };
                                    dispatch_message(
                                        &message,
                                        &handle,
                                        &router,
                                        adapter.as_ref(),
                                        &rate_limiter,
                                        &sanitizer,
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
    pub async fn stop(&mut self) {
        let _ = self.shutdown_tx.send(true);
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

fn is_group_command(message: &ChannelMessage) -> bool {
    matches!(&message.content, ChannelContent::Command { .. })
        || matches!(&message.content, ChannelContent::Text(text) if text.starts_with('/'))
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
            let regex_triggered = !was_mentioned
                && !is_command
                && matches_group_trigger_pattern(
                    ct_str,
                    message,
                    &overrides.group_trigger_patterns,
                );
            if !was_mentioned && !is_command && !regex_triggered {
                debug!(
                    "Ignoring group message on {ct_str} (group_policy=mention_only, not mentioned)"
                );
                return false;
            }
            true
        }
        GroupPolicy::All => true,
    }
}

/// Build a `SenderContext` from an incoming `ChannelMessage`.
fn build_sender_context(message: &ChannelMessage) -> SenderContext {
    SenderContext {
        channel: channel_type_str(&message.channel).to_string(),
        user_id: sender_user_id(message).to_string(),
        display_name: message.sender.display_name.clone(),
        is_group: message.is_group,
        thread_id: message.thread_id.clone(),
        account_id: message
            .metadata
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(String::from),
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
    let formatted = if adapter.name() == "wecom" {
        formatter::format_for_wecom(&text, output_format)
    } else {
        formatter::format_for_channel(&text, output_format)
    };
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
    match channel_type {
        "telegram" => OutputFormat::TelegramHtml,
        "slack" => OutputFormat::SlackMrkdwn,
        "wecom" => OutputFormat::PlainText,
        _ => OutputFormat::Markdown,
    }
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
            if !should_process_group_message(ct_str, ov, message) {
                return;
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

    // Handle commands first (early return)
    if let ChannelContent::Command { ref name, ref args } = message.content {
        let result = handle_command(name, args, handle, router, &message.sender).await;
        send_response(adapter, &message.sender, result, thread_id, output_format).await;
        return;
    }

    // For images: download, base64 encode, and send as multimodal content blocks
    if let ChannelContent::Image {
        ref url,
        ref caption,
        ref mime_type,
    } = message.content
    {
        let blocks = download_image_to_blocks(url, caption.as_deref(), mime_type.as_deref()).await;
        if blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. }))
        {
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
            )
            .await;
            return;
        }
        // Image download failed — fall through to text description below
    }

    let text = match &message.content {
        ChannelContent::Text(t) => t.clone(),
        ChannelContent::Command { .. } => unreachable!(), // handled above
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
            // A user clicked an interactive button — pass the callback action
            // as the message text so the agent can handle it.
            match message_text {
                Some(mt) => format!("[Button clicked: {action}] (on message: {mt})"),
                None => format!("[Button clicked: {action}]"),
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
            let result = handle_command(cmd, &args, handle, router, &message.sender).await;
            send_response(adapter, &message.sender, result, thread_id, output_format).await;
            return;
        }
        // Other slash commands pass through to the agent
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

    // Thread-based agent routing: if the adapter tagged this message with a
    // thread_route_agent, resolve that agent name before falling through to
    // the standard router. This allows Telegram forum threads (and similar)
    // to route to different agents based on config.
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

    // Route to agent (standard path) — use resolve_with_context to support account_id
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
    let channel_key = format!("{:?}", message.channel);

    let agent_id = match agent_id {
        Some(id) => id,
        None => {
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
            match fallback {
                Some(id) => {
                    // Auto-set this as the user's default so future messages route directly
                    router.set_user_default(message.sender.platform_id.clone(), id);
                    id
                }
                None => {
                    send_response(
                        adapter,
                        &message.sender,
                        "No agents available. Start the dashboard at http://127.0.0.1:4545 to create one.".to_string(),
                        thread_id,
                        output_format,
                    ).await;
                    return;
                }
            }
        }
    };

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

    // Send typing indicator (best-effort)
    let _ = adapter.send_typing(&message.sender).await;

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Thinking).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message);

    // Streaming path: if the adapter supports progressive output, pipe text
    // deltas directly to it instead of waiting for the full response.
    if adapter.supports_streaming() {
        match handle
            .send_message_streaming_with_sender(agent_id, &text, &sender_ctx)
            .await
        {
            Ok(mut delta_rx) => {
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

                match &stream_result {
                    Ok(()) => {
                        send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Done)
                            .await;
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
                    Err(e) => {
                        warn!("Streaming send failed, falling back to non-streaming: {e}");
                        // Fall back: re-send the full accumulated text via the
                        // non-streaming path so the user still gets a response.
                        if !buffered_text.is_empty() {
                            send_response(
                                adapter,
                                &message.sender,
                                buffered_text,
                                thread_id,
                                output_format,
                            )
                            .await;
                            send_lifecycle_reaction(
                                adapter,
                                &message.sender,
                                msg_id,
                                AgentPhase::Done,
                            )
                            .await;
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
                        // Buffer was empty — fall through to non-streaming path.
                        send_lifecycle_reaction(
                            adapter,
                            &message.sender,
                            msg_id,
                            AgentPhase::Error,
                        )
                        .await;
                        handle
                            .record_delivery(
                                agent_id,
                                ct_str,
                                &message.sender.platform_id,
                                false,
                                Some(&e.to_string()),
                                thread_id,
                            )
                            .await;
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

    // Non-streaming path: send to agent and relay response (with sender identity).
    match handle
        .send_message_with_sender(agent_id, &text, &sender_ctx)
        .await
    {
        Ok(response) => {
            send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Done).await;
            // Empty response means the agent intentionally chose to stay silent
            // (NO_REPLY / [[silent]]) — do not leak a message to the channel.
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

    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

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

    blocks.push(ContentBlock::Image { media_type, data });

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
) {
    // Thread-based agent routing (same as text path)
    let thread_route_agent_id = if let Some(agent_name) = message
        .metadata
        .get("thread_route_agent")
        .and_then(|v| v.as_str())
    {
        match handle.find_agent_by_name(agent_name).await {
            Ok(Some(id)) => Some(id),
            _ => None,
        }
    } else {
        None
    };

    // Route to agent (same logic as text path) — use resolve_with_context for account_id
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
    let channel_key = format!("{:?}", message.channel);

    let agent_id = match agent_id {
        Some(id) => id,
        None => {
            let fallback = handle.find_agent_by_name("assistant").await.ok().flatten();
            let fallback = match fallback {
                Some(id) => Some(id),
                None => handle
                    .list_agents()
                    .await
                    .ok()
                    .and_then(|agents| agents.first().map(|(id, _)| *id)),
            };
            match fallback {
                Some(id) => {
                    router.set_user_default(message.sender.platform_id.clone(), id);
                    id
                }
                None => {
                    send_response(
                        adapter,
                        &message.sender,
                        "No agents available. Start the dashboard at http://127.0.0.1:4545 to create one.".to_string(),
                        thread_id,
                        output_format,
                    ).await;
                    return;
                }
            }
        }
    };

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

    let _ = adapter.send_typing(&message.sender).await;

    // Lifecycle reaction: ⏳ Queued → 🤔 Thinking → ✅ Done / ❌ Error
    let msg_id = &message.platform_message_id;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Queued).await;
    send_lifecycle_reaction(adapter, &message.sender, msg_id, AgentPhase::Thinking).await;

    // Build sender context to propagate identity to the agent
    let sender_ctx = build_sender_context(message);

    match handle
        .send_message_with_blocks_and_sender(agent_id, blocks.clone(), &sender_ctx)
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                &crate::types::ChannelType::CLI,
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
                "Usage: /approve <id-prefix>".to_string()
            } else {
                handle.resolve_approval_text(&args[0], true).await
            }
        }
        "reject" => {
            if args.is_empty() {
                "Usage: /reject <id-prefix>".to_string()
            } else {
                handle.resolve_approval_text(&args[0], false).await
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

        let result = handle_command("agents", &[], &handle, &router, &sender).await;
        assert!(result.contains("coder"));

        let result = handle_command("help", &[], &handle, &router, &sender).await;
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
        let result =
            handle_command("agent", &["coder".to_string()], &handle, &router, &sender).await;
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
        let message = group_text_message("hello MyAgent");
        let overrides = ChannelOverrides {
            group_trigger_patterns: vec!["(?i)\\bmyagent\\b".to_string()],
            ..Default::default()
        };
        assert!(should_process_group_message(
            "whatsapp", &overrides, &message
        ));
    }

    #[test]
    fn test_mention_only_rejects_partial_regex_match() {
        let message = group_text_message("hello myagenttt");
        let overrides = ChannelOverrides {
            group_trigger_patterns: vec!["(?i)\\bmyagent\\b".to_string()],
            ..Default::default()
        };
        assert!(!should_process_group_message(
            "whatsapp", &overrides, &message
        ));
    }

    #[test]
    fn test_mention_only_skips_invalid_regex_patterns() {
        let message = group_text_message("bot please reply");
        let overrides = ChannelOverrides {
            group_trigger_patterns: vec!["(".to_string(), "(?i)\\bbot\\b".to_string()],
            ..Default::default()
        };
        assert!(should_process_group_message(
            "telegram", &overrides, &message
        ));
    }

    #[test]
    fn test_mention_only_keeps_existing_mention_behavior() {
        let mut message = group_text_message("hello there");
        message
            .metadata
            .insert("was_mentioned".to_string(), serde_json::Value::Bool(true));
        let overrides = ChannelOverrides::default();
        assert!(should_process_group_message(
            "telegram", &overrides, &message
        ));
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
            OutputFormat::PlainText
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

        let result = handle_command("btw", &[], &handle, &router, &sender).await;
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

        let result = handle_command("help", &[], &handle, &router, &sender).await;
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
}
