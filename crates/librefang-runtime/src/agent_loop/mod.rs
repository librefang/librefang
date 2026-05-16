//! Core agent execution loop.
//!
//! The agent loop handles receiving a user message, recalling relevant memories,
//! calling the LLM, executing tool calls, and saving the conversation.

use crate::checkpoint_manager::CheckpointManager;
use crate::context_budget::{apply_context_guard, ContextBudget};
use crate::context_engine::ContextEngine;
use crate::context_overflow::{recover_from_overflow, RecoveryStage};
use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::prelude::*;
use crate::llm_driver::{CompletionRequest, LlmDriver, StreamEvent, PHASE_RESPONSE_COMPLETE};
use crate::loop_guard::{LoopGuard, LoopGuardConfig, LoopGuardVerdict};
use crate::mcp::McpConnection;
use crate::tool_budget::{ToolBudgetEnforcer, ToolResultEntry};
use crate::tool_runner;
use crate::web_search::WebToolsContext;
use librefang_memory::session::Session;
use librefang_memory::{MemorySubstrate, ProactiveMemoryHooks};
use librefang_skills::registry::SkillRegistry;
use librefang_types::agent::AgentManifest;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{Memory, MemoryFilter, MemorySource};
use librefang_types::memory::{MemoryFragment, MemoryId};
use librefang_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use librefang_types::tool::{AgentLoopSignal, DecisionTrace, ToolCall, ToolDefinition};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, instrument, warn};

mod end_turn;
mod history;
mod message;
mod model;
mod prompt;
mod retry;
mod run_streaming;
mod text_recovery;
mod tool_call;
mod tool_resolution;
mod types;
mod web_augment;

pub use self::history::DEFAULT_MAX_HISTORY_MESSAGES;
pub use self::model::{apply_session_model_override_to_manifest, strip_provider_prefix};
pub use self::run_streaming::run_agent_loop_streaming;
pub use self::types::{AgentLoopResult, ExperimentContext, LoopOptions, LoopPhase, PhaseCallback};

use self::end_turn::{
    build_silent_agent_loop_result, classify_end_turn_retry, finalize_end_turn_text,
    finalize_successful_end_turn, gated_proactive_memory_for_memorize,
    gated_proactive_memory_for_retrieve, maybe_fold_stale_tool_results, EndTurnRetry,
    EndTurnRetryContext, FinalizeEndTurnContext, FinalizeEndTurnResultData,
};
use self::history::resolve_max_history;
use self::message::{
    accumulate_token_usage, is_cascade_leak, is_no_reply, is_parameter_error_content,
    is_progress_text_leak, is_soft_error_content, push_accumulated_text, safe_trim_messages,
    sanitize_sender_label, sanitize_tool_result_content, strip_prior_image_data,
    strip_processed_image_data,
};
use self::model::{stable_prefix_mode_enabled, UNKNOWN_MODEL_CONTEXT_WINDOW};
use self::prompt::{
    build_prompt_setup, log_repair_stats, prepare_llm_messages, push_filtered_user_message,
    reply_directives_from_parsed, select_running_experiment, setup_recalled_memories,
    PreparedMessages, PromptExperimentSelection, PromptSetup, PromptSetupContext, RecallSetup,
    RecallSetupContext,
};
use self::retry::call_with_retry;
use self::text_recovery::recover_text_tool_calls;
use self::tool_call::{
    append_skipped_tool_results, execute_single_tool_call, handle_mid_turn_signal,
    stage_tool_use_turn, tool_use_blocks_from_calls, update_consecutive_hard_failures,
    ToolExecutionContext, ToolResultOutcomeSummary,
};
use self::tool_resolution::ResolvedToolsCache;
use self::web_augment::web_search_augment;

/// Maximum iterations in the agent loop before giving up.
///
/// Single source of truth is `AutonomousConfig::DEFAULT_MAX_ITERATIONS` in
/// `librefang-types` — kept as a local alias here so the hot-path branches
/// in this file read as a plain constant instead of a fully-qualified
/// path. Changing the policy value in one place propagates to both the
/// runtime fallback and the manifest default.
const MAX_ITERATIONS: u32 = librefang_types::agent::AutonomousConfig::DEFAULT_MAX_ITERATIONS;

/// Timeout for individual tool executions (seconds).
/// Raised from 120s to 600s for agent_send/agent_spawn and long-running builds.
const TOOL_TIMEOUT_SECS: u64 = 600;

/// Maximum consecutive MaxTokens continuations before returning partial response.
/// Raised from 3 to 5 to allow longer-form generation.
const MAX_CONTINUATIONS: u32 = 5;

/// Run session repair on `session.messages` before persisting on failure paths.
///
/// When the agent loop exits via circuit breaker, max iterations, or timeout,
/// the session history may contain orphaned `ToolUse` blocks with no matching
/// `ToolResult`.  Providers that enforce strict pairing (Moonshot, OpenAI)
/// then return 400 on next load, making the session permanently broken.
///
/// This helper replaces `session.messages` with the repaired copy so the
/// persisted history is always well-formed.
fn repair_session_before_save(session: &mut Session, agent_id: &str, reason: &str) {
    let (repaired, stats) =
        crate::session_repair::validate_and_repair_with_stats(&session.messages);
    if stats != crate::session_repair::RepairStats::default() {
        tracing::warn!(
            agent_id,
            reason,
            orphaned_results_removed = stats.orphaned_results_removed,
            empty_messages_removed = stats.empty_messages_removed,
            messages_merged = stats.messages_merged,
            results_reordered = stats.results_reordered,
            synthetic_results_inserted = stats.synthetic_results_inserted,
            duplicates_removed = stats.duplicates_removed,
            misplaced_results_rescued = stats.misplaced_results_rescued,
            positional_synthetic_inserted = stats.positional_synthetic_inserted,
            "Session repair applied before save"
        );
    }
    session.set_messages(repaired);
    session.last_repaired_generation = Some(session.messages_generation);
}

/// Maximum consecutive iterations where every executed tool failed before
/// the loop exits with `RepeatedToolFailures`. Catches expensive wheel-spinning
/// when the LLM cannot fix a tool call (bad auth, permanent 404, etc.).
const MAX_CONSECUTIVE_ALL_FAILED: u32 = 3;

/// Marker included in timeout error messages when partial output was delivered.
/// Used by channel_bridge to detect this case without fragile string matching.
pub const TIMEOUT_PARTIAL_OUTPUT_MARKER: &str = "[partial_output_delivered]";

/// Notify the stream consumer that the LLM has finished producing text for
/// this turn so the UI can unblock input before the agent loop's remaining
/// post-processing (session persistence, proactive memory extraction) lands
/// the final `response` event. Fire-and-forget: send failures are ignored
/// because a disconnected consumer is not fatal to the turn.
async fn signal_response_complete(tx: &mpsc::Sender<StreamEvent>) {
    let _ = tx
        .send(StreamEvent::PhaseChange {
            phase: PHASE_RESPONSE_COMPLETE.to_string(),
            detail: None,
        })
        .await;
}

fn max_tokens_response_text(response: &crate::llm_driver::CompletionResponse) -> String {
    let text = response.text();
    if text.trim().is_empty() {
        "[Partial response — token limit reached with no text output.]".to_string()
    } else {
        text
    }
}

fn fire_hook_best_effort(
    hook_reg: Option<&crate::hooks::HookRegistry>,
    ctx: &crate::hooks::HookContext<'_>,
) {
    if let Some(hook_reg) = hook_reg {
        if let Err(err) = hook_reg.fire(ctx) {
            warn!(
                event = ?ctx.event,
                agent = ctx.agent_name,
                error = %err,
                "Hook failed in best-effort path"
            );
        }
    }
}

fn recall_or_default<T, E>(result: Result<T, E>, warning: &str) -> T
where
    T: Default,
    E: std::fmt::Display,
{
    match result {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "{}", warning);
            T::default()
        }
    }
}

/// Distinguishes system-fired turns from real human ones in the user
/// message itself. The kernel synthesises a `SenderContext` with
/// `channel = "cron"` or `"autonomous"` for scheduled / loop fires, but
/// the user message that reaches the LLM is otherwise indistinguishable
/// from a real human turn. The model has been observed answering a
/// scheduled trigger as if a person had asked, then conflating that
/// response with the next real human request that arrives.
///
/// Returns a typed marker prepended to the user message so the LLM can
/// distinguish "this came from a cron job" from "this came from a person".
/// The string is stable so few-shot examples and persona rules can
/// reference it explicitly. Returns `None` for human-driven channels so
/// 1:1 chats and API calls keep their existing un-prefixed message shape.
///
/// **Prompt cache safety**: the marker is byte-stable per channel (no
/// clock or PID interpolation) and only mutates the *current* user
/// message tail — historical session bytes are not rewritten. This is the
/// distinction the `build_sender_prefix` carve-out below cares about:
/// dynamic per-turn names like a Web-UI display would invalidate the
/// cache; a fixed channel-keyed marker does not.
fn build_automation_marker_prefix(sender_channel: Option<&str>) -> Option<&'static str> {
    match sender_channel {
        Some("cron") => Some("[Scheduled trigger]\n"),
        Some("autonomous") => Some("[Autonomous trigger]\n"),
        _ => None,
    }
}

/// Build the `[sender]: message` prefix for a user turn.
///
/// Emits a sanitized prefix when a real human sender identity is available
/// (group chat, channel DM with display_name / user_id). Returns `None` when:
/// - No identity available (no `sender_display_name`, no `sender_user_id`), OR
/// - Channel is a kernel-internal / dashboard surface where the synthesized
///   `display_name` is a placeholder, not a real user identity (`webui`,
///   `cron`, `autonomous`). Adding `[Web UI]: ` / `[cron]: ` to every
///   message there would be noise and would invalidate the provider prompt
///   cache by mutating the user-message body each turn.
///
/// The prefix is applied AFTER PII filtering to prevent display names that look like emails
/// or phone numbers from being redacted into the message content.
fn build_sender_prefix(manifest: &AgentManifest, sender_user_id: Option<&str>) -> Option<String> {
    let channel = manifest
        .metadata
        .get("sender_channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Keep these literals in sync with the kernel-side synthetic channel
    // sentinels: `librefang_kernel::SYSTEM_CHANNEL_{CRON,AUTONOMOUS,WEBUI}`.
    // Runtime can't import the constants directly (circular dep — runtime
    // is below kernel), so a grep-pointer is the best we can do; api / cli
    // / kernel sites reference the kernel constants by name and stay in
    // lock-step.
    if matches!(channel, "webui" | "cron" | "autonomous") {
        return None;
    }
    let raw = manifest
        .metadata
        .get("sender_display_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or(sender_user_id)?;
    Some(format!("[{}]: ", sanitize_sender_label(raw)))
}

/// Run the agent execution loop for a single user message.
///
/// This is the core of LibreFang: it loads session context, recalls memories,
/// runs the LLM in a tool-use loop, and saves the updated session.
#[allow(clippy::too_many_arguments)]
// `level = "warn"` (not the default `info`) so the daemon's baseline filter
// (`librefang_runtime=warn` in `init_tracing_stderr`) keeps this span alive.
// At INFO the span gets filtered out before it's ever created, and every
// WARN/ERROR event inside the loop loses its parent context — including the
// `agent.id` / `session.id` fields, which is the whole point of instrumenting.
// The span itself does not emit a log line; the level only gates creation.
#[instrument(level = "warn", skip_all, fields(agent.name = %manifest.name, agent.id = %session.agent_id, session.id = %session.id))]
pub async fn run_agent_loop(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&librefang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
    process_registry: Option<&crate::process_registry::ProcessRegistry>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    proactive_memory: Option<Arc<librefang_memory::ProactiveMemoryStore>>,
    context_engine: Option<&dyn ContextEngine>,
    pending_messages: Option<&tokio::sync::Mutex<mpsc::Receiver<AgentLoopSignal>>>,
    opts: &LoopOptions,
) -> LibreFangResult<AgentLoopResult> {
    info!(agent = %manifest.name, "Starting agent loop");

    // Start index of new messages added during this turn. Initialized to
    // current session length so early returns (before the user message is
    // pushed) expose an empty slice to callers. Updated after
    // safe_trim_messages to point at the post-trim position of the just-
    // pushed user message (len-1) so slicing stays in-bounds even when the
    // trim drains deeper than (len - DEFAULT_MAX_HISTORY_MESSAGES). Fixes #2067.
    let mut new_messages_start = session.messages.len();

    // Early return if driver is not configured
    if !driver.is_configured() {
        return Ok(AgentLoopResult {
            silent: true,
            provider_not_configured: true,
            new_messages_start,
            ..Default::default()
        });
    }

    // Gateway-level safety-net compression (#4972). Runs before any prompt
    // build / first LLM call: catches sessions that grew between turns
    // (overnight channel backlog, cron output piling up) and have already
    // exceeded the context window before the agent-level compactor would
    // ever get a chance to fire. No-op when the session is under threshold,
    // when ctx_window is unknown, or when the kernel did not enable it.
    if let (Some(cfg), Some(ctx_window)) = (
        opts.gateway_compression.as_ref(),
        context_window_tokens.filter(|w| *w > 0),
    ) {
        let ctx_window_u32: u32 = ctx_window.try_into().unwrap_or(u32::MAX);
        let report =
            crate::gateway_compression::apply_if_needed(&mut session.messages, ctx_window_u32, cfg);
        if report.mutated() {
            // `new_messages_start` is recomputed unconditionally in
            // `prepare_messages` (set to `session.messages.len() - 1`
            // after `safe_trim_messages` runs), so no fixup is needed
            // here — the gateway prune only narrows the prior history
            // and the just-pushed user message hasn't been added yet.
            session.mark_messages_mutated();
            info!(
                agent = %manifest.name,
                session_id = %session.id,
                tokens_before = report.tokens_before,
                tokens_after = report.tokens_after,
                tool_results_stubbed = report.tool_results_stubbed,
                tool_result_bytes_elided = report.tool_result_bytes_elided,
                messages_dropped = report.messages_dropped,
                "Gateway compression pruned session before agent loop (#4972)"
            );
        } else if report.fired {
            // Fired but nothing pruned (e.g. entirely pinned history). The
            // LLM compactor's summarisation is the only remedy.
            tracing::warn!(
                agent = %manifest.name,
                session_id = %session.id,
                tokens_before = report.tokens_before,
                "Gateway compression fired but could not prune (entirely pinned?)"
            );
        }
    }

    let PromptExperimentSelection {
        experiment_context,
        running_experiment,
    } = select_running_experiment(manifest, session, kernel.as_ref(), false);

    // Extract hand-allowed env vars from manifest metadata (set by kernel for hand settings)
    let hand_allowed_env: Vec<String> = manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Lazy tool loading (issue #3044). Default ON — the LLM gets a trimmed
    // "always native" toolset plus `tool_load` / `tool_search`, and pays a
    // per-turn round-trip to pull in any other schema it wants. Set
    // `lazy_tools = false` in manifest.metadata to restore eager mode.
    let lazy_tools = manifest
        .metadata
        .get("lazy_tools")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut session_loaded_tools: Vec<ToolDefinition> = Vec::new();

    // Extract sender context from manifest metadata (set by kernel for per-sender
    // trust and channel-specific tool authorization).
    let sender_user_id: Option<String> = manifest
        .metadata
        .get("sender_user_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let sender_channel: Option<String> = manifest
        .metadata
        .get("sender_channel")
        .and_then(|v| v.as_str())
        .map(String::from);

    let stable_prefix_mode = stable_prefix_mode_enabled(manifest);

    let RecallSetup {
        memories,
        memories_used,
    } = setup_recalled_memories(RecallSetupContext {
        session,
        user_message,
        memory,
        embedding_driver,
        proactive_memory: gated_proactive_memory_for_retrieve(manifest, proactive_memory.as_ref()),
        context_engine,
        sender_user_id: sender_user_id.as_deref(),
        sender_channel: sender_channel.as_deref(),
        kernel: kernel.as_ref(),
        stable_prefix_mode,
        streaming: false,
        opts,
    })
    .await;

    // Fire BeforePromptBuild hook
    let agent_id_str = session.agent_id.0.to_string();
    let ctx = crate::hooks::HookContext {
        agent_name: &manifest.name,
        agent_id: agent_id_str.as_str(),
        event: librefang_types::agent::HookEvent::BeforePromptBuild,
        data: serde_json::json!({
            "system_prompt": &manifest.model.system_prompt,
            "user_message": user_message,
        }),
    };
    fire_hook_best_effort(hooks, &ctx);

    let PromptSetup {
        system_prompt,
        memory_context_msg,
    } = build_prompt_setup(PromptSetupContext {
        manifest,
        session,
        kernel: kernel.as_ref(),
        experiment_context: experiment_context.as_ref(),
        running_experiment: running_experiment.as_ref(),
        memories: &memories,
        stable_prefix_mode,
        streaming: false,
    });

    // Mutable collector for memories saved during this turn (populated by auto_memorize).
    let memories_saved: Vec<String> = Vec::new();
    // Mutable collector for memory conflicts detected during this turn.
    let memory_conflicts: Vec<librefang_types::memory::MemoryConflict> = Vec::new();

    // PII privacy filtering: extract config from manifest metadata.
    let privacy_config: librefang_types::config::PrivacyConfig = manifest
        .metadata
        .get("privacy")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let pii_filter = crate::pii_filter::PiiFilter::new(&privacy_config.redact_patterns);

    // Compute a sanitized `[sender]: ` prefix so the LLM can distinguish who said
    // what across multiple turns. Emitted for groups (#2262) and for channel DMs
    // with a real sender identity (#4666); skipped for dashboard / cron /
    // autonomous fires where `display_name` is a placeholder. The prefix is
    // applied AFTER PII filtering (see push_filtered_user_message) so display
    // names that look like emails/phones do not get redacted into the stored
    // content.
    let sender_prefix = build_sender_prefix(manifest, sender_user_id.as_deref());
    // Automation marker for cron / autonomous-loop fires so the model can
    // tell a scheduled trigger from a real human turn. Orthogonal to
    // `sender_prefix`: cron / autonomous channels are in the carve-out
    // above, so `sender_prefix` is None there and the combined form is
    // just the marker. The marker is byte-stable per channel and so does
    // not invalidate the provider prompt cache.
    let automation_marker = build_automation_marker_prefix(sender_channel.as_deref());
    let combined_prefix: Option<String> = match (automation_marker, sender_prefix.as_deref()) {
        (Some(m), Some(p)) => Some(format!("{m}{p}")),
        (Some(m), None) => Some(m.to_string()),
        (None, Some(p)) => Some(p.to_string()),
        (None, None) => None,
    };
    let effective_user_message = match &combined_prefix {
        Some(p) => format!("{p}{user_message}"),
        None => user_message.to_string(),
    };

    // Prompt injection guard: scan user message for injection attempts before
    // it reaches the LLM. Threats are logged and a warning prefix is prepended
    // to the message — the message itself is never silently dropped so users
    // are not confused by missing replies.
    let injection_prefix_storage;
    let (guarded_user_message, guarded_user_content_blocks) =
        if let Some(warning) = crate::injection_guard::scan_message(user_message) {
            warn!(
                event = "injection_guard",
                threats = ?warning.threat_ids,
                summary = %warning.summary,
                agent = %manifest.name,
                "Prompt injection indicators detected in user message"
            );
            let prefix = crate::injection_guard::warning_prefix(&warning);
            injection_prefix_storage = format!("{prefix}{user_message}");
            // For multimodal messages prepend the warning as an extra text block.
            let prefixed_blocks = user_content_blocks.map(|blocks| {
                let mut out = blocks;
                out.insert(
                    0,
                    ContentBlock::Text {
                        text: prefix,
                        provider_metadata: None,
                    },
                );
                out
            });
            (injection_prefix_storage.as_str(), prefixed_blocks)
        } else {
            // No injection detected — pass the original message through; the
            // storage binding is left unused on this branch.
            (user_message, user_content_blocks)
        };

    // Add the user message to session history.
    // When content blocks are provided (e.g. text + image from a channel),
    // use multimodal message format so the LLM receives the image for vision.
    push_filtered_user_message(
        session,
        guarded_user_message,
        guarded_user_content_blocks,
        &pii_filter,
        &privacy_config,
        combined_prefix.as_deref(),
    );

    let max_history = resolve_max_history(manifest, opts);
    let PreparedMessages {
        mut messages,
        new_messages_start: prepared_new_messages_start,
        repair_stats,
    } = prepare_llm_messages(
        manifest,
        session,
        &effective_user_message,
        memory_context_msg,
        max_history,
    );
    log_repair_stats(manifest, session, &repair_stats);

    // Web search augmentation: generate search queries via LLM, search the web,
    // and inject results into context for models without tool/function calling.
    let web_search_echo_policy = kernel
        .as_ref()
        .map(|k| k.reasoning_echo_policy_for(&manifest.model.model))
        .unwrap_or_default();
    if let Some(search_results) = web_search_augment(
        manifest,
        user_message,
        web_ctx,
        driver.as_ref(),
        &session.messages,
        web_search_echo_policy,
    )
    .await
    {
        messages.insert(
            0,
            Message::user(format!(
                "[Web search results — use these to inform your response]\n{search_results}"
            )),
        );
    }

    let mut total_usage = TokenUsage::default();
    let final_response;
    // Track the slot that actually served the most recent LLM call —
    // populated by `FallbackDriver` / `FallbackChain` on chain failover
    // (#4807 review nit 10). The kernel reads this off `AgentLoopResult`
    // and stamps the matching `UsageRecord.provider` so billing rolls
    // up against the slot that did the work.
    let mut last_actual_provider: Option<String> = None;
    // Accumulate text content from intermediate tool_use iterations. A turn
    // that yields a tool_use response may also carry user-facing text (e.g.
    // "Looking that up for you..." before a memory_store call). Without this
    // buffer that text is lost when the final EndTurn iteration returns an
    // empty body and the empty-response guard takes over. See #fix-3074.
    let mut accumulated_text = String::new();

    new_messages_start = prepared_new_messages_start;

    // Resolution order: per-agent manifest > operator LoopOptions > library default.
    let max_iterations = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .or(opts.max_iterations)
        .unwrap_or(MAX_ITERATIONS);

    // Initialize loop guard — scale circuit breaker for autonomous agents
    let loop_guard_config = {
        let mut cfg = LoopGuardConfig::default();
        if max_iterations > cfg.global_circuit_breaker {
            cfg.global_circuit_breaker = max_iterations * 3;
        }
        cfg
    };
    let mut loop_guard = LoopGuard::new(loop_guard_config);
    let mut consecutive_max_tokens: u32 = 0;

    // Per-session dangerous command checker — shared across all tool executions
    // in this loop so that session allowlist entries are honored throughout.
    let session_checker = Arc::new(tokio::sync::RwLock::new(
        crate::dangerous_command::DangerousCommandChecker::default(),
    ));

    // Build context budget from model's actual context window. If the model
    // wasn't in the catalog (`None`), pick a conservative 8K fallback — a
    // 200K assumption silently bills the user for prompts the provider then
    // rejects with HTTP 400 (#3349).
    let ctx_window = context_window_tokens.unwrap_or_else(|| {
        tracing::warn!(
            model = %manifest.model.model,
            fallback = UNKNOWN_MODEL_CONTEXT_WINDOW,
            "Model not in catalog — falling back to conservative context window. \
             Set `model.context_window` in agent.toml to silence this warning."
        );
        UNKNOWN_MODEL_CONTEXT_WINDOW
    });
    let context_budget = ContextBudget::new(ctx_window);
    // Resolve tool-results budget config from opts (falls back to compiled defaults).
    let tool_results_cfg = opts.tool_results_config.clone().unwrap_or_default();
    let tr_per_result = tool_results_cfg.spill_threshold_bytes as usize;
    let tr_per_turn = tool_results_cfg.max_bytes_per_turn as usize;
    let tr_max_artifact_bytes = tool_results_cfg.max_artifact_bytes;
    let tr_fold_after_turns = tool_results_cfg.history_fold_after_turns;
    let tr_fold_min_batch_size = tool_results_cfg.fold_min_batch_size;
    // Context compressor — triggers LLM-based summarisation when token usage
    // exceeds 80% of the context window, before falling back to brute-force trim.
    // #4976: when LoopOptions carries a pre-merged compaction snapshot
    // (per-agent overrides resolved against global config in the kernel),
    // honour its keep_recent / max_summary_tokens / token_threshold_ratio.
    let context_compressor = match opts.compaction_config.as_ref() {
        Some(toml) => crate::context_compressor::ContextCompressor::new(
            crate::context_compressor::CompressionConfig::from_compaction_toml(toml),
        ),
        None => crate::context_compressor::ContextCompressor::with_defaults(),
    };
    let mut any_tools_executed = false;
    let mut decision_traces: Vec<DecisionTrace> = Vec::new();
    // §A — accumulated owner_notice payloads from notify_owner tool calls.
    // Multiple invocations in the same turn are joined with "\n\n".
    let mut pending_owner_notice: Option<String> = None;
    let mut hallucination_retried = false;
    let mut action_nudge_retried = false;
    let mut consecutive_all_failed: u32 = 0;
    // Seed with a pre-loop estimate so that should_compress fires on the very
    // first iteration even for single-turn conversations.  Without this, the
    // check is always `0 < threshold`, which is always false.
    let mut last_prompt_tokens: usize = crate::compactor::estimate_token_count(
        &messages,
        Some(&system_prompt),
        Some(available_tools),
    );

    // Inform the context engine of the active model and context window before
    // the loop starts so threshold calculations use the correct parameters.
    if let Some(engine) = context_engine {
        let initial_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
        engine.update_model(&initial_model, ctx_window);
    }

    // The system prompt is constant across all iterations but `CompletionRequest`
    // takes it by value, so we clone it per-iteration.  Keep a single pre-cloned
    // copy so each per-iteration clone is always from the same allocation rather
    // than potentially going through Arc/mutex indirection on the original source.
    // This is a minor but measurable improvement for long autonomous runs.
    let system_prompt_snapshot = system_prompt.clone();

    // Resolve tool list once before the loop and reuse via Arc on every
    // iteration.  See `ResolvedToolsCache` for rationale (#3586).
    let mut tools_cache =
        ResolvedToolsCache::new(available_tools, &session_loaded_tools, lazy_tools);

    for iteration in 0..max_iterations {
        debug!(iteration, "Agent loop iteration");

        // Check for session-scoped interrupt at each iteration boundary.
        // This allows a /stop signal to abort the loop between LLM calls
        // without affecting other concurrent sessions.
        if opts.interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
            debug!(iteration, "Agent loop interrupted by session cancel signal");
            return Ok(AgentLoopResult {
                silent: true,
                new_messages_start,
                ..Default::default()
            });
        }

        // Fire agent:step external hook (fire-and-forget).
        if let Some(ref k) = kernel {
            k.fire_agent_step(&agent_id_str, iteration);
        }

        // Pluggable context engine: threshold-gated compaction. When the
        // engine signals that the current token count has crossed its
        // compression threshold, run a compaction pass *before* assemble so
        // the assembled context is already trimmed.
        //
        // `last_prompt_tokens` carries the prompt-token count from the
        // previous LLM call — never a running sum.  This correctly gates
        // `should_compress` on each turn's own input cost.  On the first
        // iteration `last_prompt_tokens` is 0, so compaction can only fire
        // when the model's context window itself (via `ctx_window`) is
        // below threshold.  `total_usage` (accumulated across iterations) is
        // never read here, so it remains a clean snapshot for the kernel
        // budget tracker and is never mutated by the compaction path.
        if let Some(engine) = context_engine {
            if engine.should_compress(last_prompt_tokens, ctx_window) {
                debug!(
                    iteration,
                    last_prompt_tokens, ctx_window, "Context engine requested compaction"
                );
                // Normalize the model ID before passing to the engine — raw
                // manifest values may carry a provider prefix (e.g.
                // "openrouter/google/gemini-2.5-flash") that drivers don't
                // understand when used as a summarisation model.
                let compact_model =
                    strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
                match engine
                    .compact(
                        session.agent_id,
                        &messages,
                        driver.clone(),
                        &compact_model,
                        ctx_window,
                    )
                    .await
                {
                    Ok(result) => {
                        debug!(
                            kept = result.kept_messages.len(),
                            "Context engine compaction complete"
                        );
                        // Inject the LLM-generated summary as a synthetic user message
                        // so the agent retains context about what was compacted.
                        // Without this, the summary is silently discarded and the agent
                        // loses all knowledge of earlier turns.
                        let mut compacted = Vec::with_capacity(result.kept_messages.len() + 1);
                        if !result.summary.is_empty() {
                            compacted.push(Message {
                                role: Role::User,
                                content: MessageContent::Text(format!(
                                    "[Context compaction summary] Earlier conversation turns \
                                     were summarised to preserve context space. Summary of \
                                     removed messages: {}",
                                    result.summary
                                )),
                                pinned: false,
                                timestamp: None,
                            });
                        }
                        compacted.extend(result.kept_messages);
                        messages = compacted;
                        // `last_prompt_tokens` is intentionally NOT reset here.
                        // A second compaction should only fire after the next
                        // LLM call raises it above threshold again.  Resetting
                        // to 0 would cause premature re-trigger.
                    }
                    Err(e) => {
                        warn!("Context engine compaction failed (continuing): {e}");
                    }
                }
            }
        }

        // History fold (#3347 3/N): rewrite stale tool-result blocks in place
        // before context assembly so the LLM never sees raw bulk payloads
        // from old turns.  Runs fold first, then the compressor, mirroring
        // the ordering rationale in `history_fold`'s module-level doc.
        let fold_echo_policy = kernel
            .as_ref()
            .map(|k| k.reasoning_echo_policy_for(&manifest.model.model))
            .unwrap_or_default();
        messages = maybe_fold_stale_tool_results(
            messages,
            session,
            crate::history_fold::FoldConfig {
                fold_after_turns: tr_fold_after_turns,
                min_batch_size: tr_fold_min_batch_size,
            },
            &manifest.model.model,
            opts.aux_client.as_deref(),
            driver.clone(),
            false,
            fold_echo_policy,
        )
        .await;

        // Context assembly — use context engine if available, else inline logic
        if let Some(engine) = context_engine {
            let result = engine
                .assemble(
                    session.agent_id,
                    &mut messages,
                    &system_prompt,
                    available_tools,
                    ctx_window,
                )
                .await?;
            if result.recovery == RecoveryStage::FinalError {
                warn!("Context overflow unrecoverable — suggest /reset or /compact");
            }
        } else {
            // Inline fallback: LLM-based context compression (soft), then
            // overflow recovery (hard trim), then context guard.
            //
            // When the kernel wired an [`AuxClient`] through `opts.aux_client`,
            // summarisation routes to the cheap-tier auxiliary chain
            // (issue #3314); otherwise the primary `driver` is used —
            // preserving baseline behaviour.
            let (compressed, compression_events) = context_compressor
                .compress_if_needed_with_aux(
                    messages.clone(),
                    &system_prompt,
                    available_tools,
                    ctx_window,
                    &manifest.model.model,
                    driver.clone(),
                    opts.aux_client.as_deref(),
                )
                .await;

            let had_soft_compression = !compression_events.is_empty();
            let mut hard_trimmed = false;

            if had_soft_compression {
                messages = compressed;
                messages = crate::session_repair::validate_and_repair(&messages);
                messages = crate::session_repair::ensure_starts_with_user(messages);
                // #4971: drop file_read dedup state — the bodies its stubs
                // referenced have been summarised away.
                crate::context_compressor::reset_post_compression_side_state(session.id);
            }

            // Hard-trim only if still above threshold after soft compression
            // and repair. Keep the pre-existing ordering so token estimation
            // and recovery boundaries are computed on provider-valid history.
            let remaining_tokens = crate::compactor::estimate_token_count(
                &messages,
                Some(&system_prompt),
                Some(available_tools),
            );
            let hard_trim_threshold = (ctx_window as f64 * 0.70) as usize;
            if remaining_tokens > hard_trim_threshold {
                let recovery = recover_from_overflow(
                    &mut messages,
                    &system_prompt,
                    available_tools,
                    ctx_window,
                );
                if recovery == RecoveryStage::FinalError {
                    warn!("Context overflow unrecoverable — suggest /reset or /compact");
                }
                hard_trimmed = recovery != RecoveryStage::None;
            }

            // Repair again only if hard trim ran; trimming can cut across a
            // tool-call boundary even when the pre-trim history was valid.
            if hard_trimmed {
                messages = crate::session_repair::validate_and_repair(&messages);
                messages = crate::session_repair::ensure_starts_with_user(messages);
            }
            if had_soft_compression {
                session.set_messages(messages.clone());
            }
            apply_context_guard(&mut messages, &context_budget, available_tools);
        }

        // Strip provider prefix: "openrouter/google/gemini-2.5-flash" → "google/gemini-2.5-flash"
        let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);

        let prompt_caching = manifest
            .metadata
            .get("prompt_caching")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        // Resolve the prompt-cache strategy (#4970). Kernel forwards
        // it as a string ("disabled" / "system_only" / "system_and_N");
        // a parse failure falls back to the driver's built-in default
        // (Anthropic: `system_and_3`) with a warning. We do NOT crash
        // the request on a bad value — the worst case is a turn that
        // is cached less aggressively than the operator intended.
        let prompt_cache_strategy = manifest
            .metadata
            .get("prompt_cache_strategy")
            .and_then(|v| v.as_str())
            .and_then(|s| match s.parse::<librefang_types::config::PromptCacheStrategy>() {
                Ok(strategy) => Some(strategy),
                Err(e) => {
                    tracing::warn!(error = %e, "ignoring invalid prompt_cache_strategy metadata");
                    None
                }
            });
        // Map `prompt_cache.cache_ttl_hint_secs` to Anthropic's two
        // discrete cache windows: ≥ 1800 s selects the 1h beta cache,
        // everything else stays on the default 5m ephemeral cache. The
        // CompletionRequest API only accepts `&'static str` here.
        let cache_ttl = manifest
            .metadata
            .get("prompt_cache_ttl_hint_secs")
            .and_then(|v| v.as_u64())
            .and_then(|secs| if secs >= 1800 { Some("1h") } else { None });

        let timeout_override = manifest
            .metadata
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                if available_tools
                    .iter()
                    .any(|t| t.name.starts_with("browser_") || t.name.starts_with("playwright_"))
                {
                    Some(600)
                } else {
                    None
                }
            });

        // Catalog-driven reasoning_echo_policy lookup (#4842). Falls back
        // to `None` when no kernel handle is wired or the model isn't in
        // the catalog; the OpenAI driver then resolves the policy via its
        // own substring fallback for backwards compatibility.
        let reasoning_echo_policy = kernel
            .as_ref()
            .map(|k| k.reasoning_echo_policy_for(&api_model))
            .unwrap_or_default();

        // Wrap messages once per turn — call_with_retry's `request.clone()`
        // becomes a refcount bump instead of a deep clone of the history (#3766).
        let request = CompletionRequest {
            model: api_model,
            messages: std::sync::Arc::new(messages.clone()),
            tools: tools_cache.get(available_tools, &session_loaded_tools),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            // Clone from the pre-built snapshot rather than the original to
            // avoid redundant Arc-deref / string traversal on every iteration.
            system: Some(system_prompt_snapshot.clone()),
            thinking: manifest.thinking.clone(),
            prompt_caching,
            cache_ttl,
            prompt_cache_strategy,
            response_format: manifest.response_format.clone(),
            timeout_secs: timeout_override,
            extra_body: if manifest.model.extra_params.is_empty() {
                None
            } else {
                Some(manifest.model.extra_params.clone())
            },
            agent_id: Some(agent_id_str.clone()),
            session_id: Some(session.id.to_string()),
            step_id: Some(iteration.to_string()),
            reasoning_echo_policy,
        };

        // Notify phase: Thinking
        if let Some(cb) = on_phase {
            cb(LoopPhase::Thinking);
        }

        // Stamp last_active before LLM call to prevent heartbeat false-positives
        // during long-running completions.
        if let Some(ref k) = kernel {
            k.touch_heartbeat(&agent_id_str);
        }

        // Call LLM with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let mut response = call_with_retry(&*driver, request, Some(provider_name), None).await?;

        accumulate_token_usage(&mut total_usage, &response.usage);
        // Track the actual-serving slot for billing attribution (#4807
        // review nit 10). FallbackDriver / FallbackChain stamp this on
        // chain failover; otherwise it stays None and billing falls
        // back to the manifest-nominated provider.
        if let Some(ref p) = response.actual_provider {
            last_actual_provider = Some(p.clone());
        }

        // Snapshot prompt tokens for the next iteration's should_compress check.
        // This is the per-turn input cost, NOT a running sum — we deliberately
        // do NOT accumulate into last_prompt_tokens.
        //
        // Some drivers (gemini_cli, codex_cli) return input_tokens = 0.  Fall
        // back to a local estimate so should_compress is not permanently
        // suppressed for those providers.
        last_prompt_tokens = if response.usage.input_tokens > 0 {
            response.usage.input_tokens as usize
        } else {
            crate::compactor::estimate_token_count(
                &messages,
                Some(&system_prompt),
                Some(available_tools),
            )
        };

        // Strip image base64 from earlier messages (LLM already processed them)
        let _ = strip_processed_image_data(&mut messages);
        if strip_processed_image_data(&mut session.messages) {
            session.mark_messages_mutated();
        }

        // Recover tool calls output as text by models that don't use the tool_calls API field
        // (e.g. Groq/Llama, DeepSeek emit `<function=name>{json}</function>` in text)
        let mut tools_recovered_from_text = false;
        if matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        ) && response.tool_calls.is_empty()
        {
            let recovered = recover_text_tool_calls(&response.text(), available_tools);
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "Recovered text-based tool calls → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                tools_recovered_from_text = true;
                response.content = tool_use_blocks_from_calls(&response.tool_calls);
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                // LLM is done — extract text and save
                let text = response.text();

                // Parse reply directives from the response text
                let (cleaned_text, parsed_directives) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text;

                // NO_REPLY: agent intentionally chose not to reply
                if is_no_reply(&text) || parsed_directives.silent {
                    let reason = if parsed_directives.silent {
                        crate::silent_response::SilentReason::PolicyBlock
                    } else {
                        crate::silent_response::SilentReason::NoReply
                    };
                    info!(
                        event = "silent_response_detected",
                        agent = %manifest.name,
                        reason = ?reason,
                        source = "agent_loop.non_streaming",
                        "Agent chose silent completion"
                    );
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent — silent completion");
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    if !opts.is_fork && !opts.incognito {
                        memory
                            .save_session_async(session)
                            .await
                            .map_err(LibreFangError::memory)?;
                    }
                    return Ok(build_silent_agent_loop_result(
                        total_usage,
                        iteration + 1,
                        parsed_directives,
                        decision_traces,
                        memories_used.clone(),
                        experiment_context.clone(),
                        new_messages_start,
                    ));
                }

                // Cascade scaffolding-leak guard: model dumped two or more
                // structural markers (memory frames, prompt section headers,
                // gateway envelopes) into the reply text. This is almost
                // always recall-regurgitation rather than user-facing content
                // (see is_cascade_leak doc-comment). Drop as silent.
                if response.tool_calls.is_empty()
                    && !tools_recovered_from_text
                    && is_cascade_leak(&text)
                {
                    warn!(
                        agent = %manifest.name,
                        text_excerpt = %text.chars().take(120).collect::<String>(),
                        "Cascade scaffolding leak detected (2+ structural markers in text-only EndTurn) — dropping as silent"
                    );
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    if !opts.is_fork && !opts.incognito {
                        memory
                            .save_session_async(session)
                            .await
                            .map_err(LibreFangError::memory)?;
                    }
                    return Ok(build_silent_agent_loop_result(
                        total_usage,
                        iteration + 1,
                        parsed_directives,
                        decision_traces,
                        memories_used.clone(),
                        experiment_context.clone(),
                        new_messages_start,
                    ));
                }

                // Progress-text-leak guard: model emitted a short ellipsis-
                // terminated acknowledgment ("Waiting for the script to
                // complete...") but the turn ended without producing the
                // tool call that preamble was introducing. Surfacing this
                // to the channel reads as nonsense; drop as silent and let
                // the operator retrigger.
                if response.tool_calls.is_empty()
                    && !tools_recovered_from_text
                    && is_progress_text_leak(&text)
                {
                    warn!(
                        agent = %manifest.name,
                        text_excerpt = %text.chars().take(80).collect::<String>(),
                        "Progress-text leak detected (ellipsis-terminated short reply without tool_use) — dropping as silent"
                    );
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    if !opts.is_fork && !opts.incognito {
                        memory
                            .save_session_async(session)
                            .await
                            .map_err(LibreFangError::memory)?;
                    }
                    return Ok(build_silent_agent_loop_result(
                        total_usage,
                        iteration + 1,
                        parsed_directives,
                        decision_traces,
                        memories_used.clone(),
                        experiment_context.clone(),
                        new_messages_start,
                    ));
                }

                match classify_end_turn_retry(EndTurnRetryContext {
                    text: &text,
                    response: &response,
                    iteration,
                    available_tools,
                    any_tools_executed,
                    hallucination_retried,
                    action_nudge_retried,
                    user_message,
                }) {
                    Some(EndTurnRetry::EmptyResponse { is_silent_failure }) => {
                        warn!(
                            agent = %manifest.name,
                            iteration,
                            input_tokens = response.usage.input_tokens,
                            output_tokens = response.usage.output_tokens,
                            silent_failure = is_silent_failure,
                            "Empty response, retrying once"
                        );
                        if is_silent_failure {
                            messages = crate::session_repair::validate_and_repair(&messages);
                        }
                        messages.push(Message::assistant("[no response]".to_string()));
                        messages.push(Message::user("Please provide your response.".to_string()));
                        continue;
                    }
                    Some(EndTurnRetry::HallucinatedAction) => {
                        hallucination_retried = true;
                        // One-shot corrective retry — expected in mixed-capability
                        // model fleets and not an error condition. Keep as info
                        // so operators can still see how often it fires.
                        info!(
                            agent = %manifest.name,
                            iteration,
                            "Detected hallucinated action — agent claimed action without tool calls, retrying"
                        );
                        messages.push(Message::assistant(&text));
                        messages.push(Message::user(
                            "[System: You described performing an action but did not actually call any tools. \
                             Please use the provided tools to carry out the action rather than just describing it.]"
                        ));
                        continue;
                    }
                    Some(EndTurnRetry::ActionIntent) => {
                        action_nudge_retried = true;
                        info!(
                            agent = %manifest.name,
                            iteration,
                            "User requested action but LLM responded without tool calls — nudging retry"
                        );
                        messages.push(Message::assistant(&text));
                        messages.push(Message::user(
                            "[System: You described actions but didn't execute them. \
                             Please use the available tools to complete the requested actions.]",
                        ));
                        continue;
                    }
                    None => {}
                }

                let text = finalize_end_turn_text(
                    text,
                    any_tools_executed,
                    &manifest.name,
                    iteration,
                    &total_usage,
                    messages.len(),
                    "Empty response from LLM — guard activated",
                    &accumulated_text,
                );
                final_response = text.clone();

                return finalize_successful_end_turn(
                    FinalizeEndTurnContext {
                        manifest,
                        session,
                        memory,
                        embedding_driver,
                        context_engine,
                        on_phase,
                        proactive_memory: gated_proactive_memory_for_memorize(
                            manifest,
                            proactive_memory.as_ref(),
                        ),
                        hooks,
                        agent_id_str: agent_id_str.as_str(),
                        user_message,
                        messages: &messages,
                        sender_user_id: sender_user_id.as_deref(),
                        streaming: false,
                        opts,
                    },
                    FinalizeEndTurnResultData {
                        final_response,
                        iteration,
                        total_usage,
                        decision_traces,
                        memories_saved,
                        memories_used,
                        memory_conflicts,
                        experiment_context: experiment_context.clone(),
                        directives: reply_directives_from_parsed(parsed_directives),
                        new_messages_start,
                        owner_notice: pending_owner_notice.take(),
                        actual_provider: last_actual_provider.clone(),
                    },
                )
                .await;
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                // Capture any text content from this tool_use turn — the LLM
                // may emit text alongside tool calls (e.g. a chat reply
                // before a memory_store invocation). Without this the text
                // is lost if the next iteration returns EndTurn with empty
                // text.
                //
                // Buffer is capped at ACCUMULATED_TEXT_MAX_BYTES — see
                // push_accumulated_text.
                let intermediate_text = response.text();
                if !intermediate_text.trim().is_empty() {
                    push_accumulated_text(&mut accumulated_text, intermediate_text.trim());
                }

                // Stage the turn locally — session.messages is NOT
                // mutated until `staged.commit(...)` runs below (or the
                // mid-turn signal handler commits on our behalf). If
                // execute_single_tool_call propagates `?` before commit,
                // the staged turn drops silently and session.messages is
                // unchanged — by construction, no orphan ToolUse can
                // reach the persistence layer. See #2381.
                let mut staged = stage_tool_use_turn(
                    &response,
                    session,
                    available_tools,
                    tr_per_result,
                    tr_per_turn,
                    tr_max_artifact_bytes,
                );

                // Execute each tool call with loop guard, timeout, and truncation.
                let mut iteration_outcomes = ToolResultOutcomeSummary::default();
                let mut committed_by_signal = false;
                let total_tool_calls = response.tool_calls.len();
                for (call_idx, tool_call) in response.tool_calls.iter().enumerate() {
                    let mut tool_exec_ctx = ToolExecutionContext {
                        manifest,
                        loop_guard: &mut loop_guard,
                        memory,
                        session,
                        kernel: kernel.as_ref(),
                        available_tool_names: &staged.allowed_tool_names,
                        available_tools,
                        caller_id_str: &staged.caller_id_str,
                        skill_registry,
                        allowed_skills: &manifest.skills,
                        mcp_connections,
                        web_ctx,
                        browser_ctx,
                        hand_allowed_env: &hand_allowed_env,
                        workspace_root,
                        media_engine,
                        media_drivers,
                        tts_engine,
                        docker_config,
                        hooks,
                        process_manager,
                        process_registry,
                        sender_user_id: sender_user_id.as_deref(),
                        sender_channel: sender_channel.as_deref(),
                        checkpoint_manager: checkpoint_manager.as_ref(),
                        context_budget: &context_budget,
                        context_engine,
                        context_window_tokens: ctx_window,
                        on_phase,
                        decision_traces: &mut decision_traces,
                        rationale_text: &staged.rationale_text,
                        tools_recovered_from_text,
                        iteration,
                        streaming: false,
                        agent_id_str: agent_id_str.as_str(),
                        opts,
                        interrupt: opts.interrupt.clone(),
                        dangerous_command_checker: Some(&session_checker),
                    };
                    let executed = execute_single_tool_call(&mut tool_exec_ctx, tool_call).await?;

                    // §A — capture owner_notice side-channel from notify_owner tool.
                    if let Some(ref notice) = executed.result.owner_notice {
                        pending_owner_notice = Some(match pending_owner_notice.take() {
                            Some(prev) => format!("{prev}\n\n{notice}"),
                            None => notice.clone(),
                        });
                    }

                    // Capture lazy-load side-channel from the tool_load meta-tool
                    // (issue #3044). Tools registered this way become callable on
                    // subsequent iterations of this loop.
                    if let Some(def) = executed.result.loaded_tool.clone() {
                        if !session_loaded_tools.iter().any(|t| t.name == def.name) {
                            session_loaded_tools.push(def);
                        }
                    }

                    // Layer 2: per-result budget — spill oversized outputs to
                    // the artifact store (#3347 2/N + #2 review-followup).
                    let budgeted_content =
                        ToolBudgetEnforcer::new(tr_per_result, tr_per_turn, tr_max_artifact_bytes)
                            .maybe_persist_result(
                                &executed.final_content,
                                &executed.result.tool_use_id,
                            );
                    staged.append_result(ContentBlock::ToolResult {
                        tool_use_id: executed.result.tool_use_id.clone(),
                        tool_name: tool_call.name.clone(),
                        content: budgeted_content,
                        is_error: executed.result.is_error,
                        status: executed.result.status,
                        approval_request_id: executed.result.approval_request_id.clone(),
                    });

                    // Stop executing remaining tool calls on failure (#948)
                    // but not for approval denials or sandbox security rejections —
                    // those should let the LLM recover and retry with a valid path (#1861)
                    // Issue #2381: emit stub tool_results for the remaining unexecuted
                    // calls so OpenAI / Anthropic see a response for every tool_call_id.
                    // Without this the next API request returns 400 with
                    // "tool_call_ids ... did not have response messages" and the agent
                    // gets bricked.
                    let is_soft_error = executed.result.status.is_soft_error()
                        || is_soft_error_content(&executed.result.content);
                    if executed.result.is_error && !is_soft_error {
                        warn!(
                            tool = %tool_call.name,
                            "Tool execution failed — skipping remaining tool calls"
                        );
                        append_skipped_tool_results(
                            &mut staged.tool_result_blocks,
                            &response.tool_calls[call_idx + 1..],
                            "previous tool call in the same batch failed with a hard error",
                        );
                        break;
                    }

                    // Mid-turn message injection (#956): check for
                    // pending user messages between tool calls. The
                    // handler pads missing results and commits the
                    // staged turn BEFORE injecting the user message, so
                    // the session never has orphan tool_use_ids.
                    if let Some(flushed_outcomes) = handle_mid_turn_signal(
                        pending_messages,
                        &manifest.name,
                        session,
                        &mut messages,
                        &mut staged,
                    ) {
                        // Same #2381 invariant: even when the batch is
                        // interrupted by a mid-turn signal, every tool_call
                        // must end up with a tool_result. handle_mid_turn_signal
                        // already called pad_missing_results before committing,
                        // so remaining ids are covered. This stub call is a
                        // belt-and-suspenders guard for any ids not yet in staged.
                        if call_idx + 1 < total_tool_calls {
                            append_skipped_tool_results(
                                &mut staged.tool_result_blocks,
                                &response.tool_calls[call_idx + 1..],
                                "tool batch interrupted by a mid-turn user message",
                            );
                        }
                        iteration_outcomes.accumulate(flushed_outcomes);
                        committed_by_signal = true;
                        break;
                    }
                }

                if !committed_by_signal {
                    staged.pad_missing_results();
                    iteration_outcomes.accumulate(staged.commit(session, &mut messages));
                }

                // Interim save after tool execution to prevent data loss on crash.
                // Skipped for fork and incognito turns — both are ephemeral and
                // must not pollute the canonical session even on mid-turn crashes.
                if !opts.is_fork && !opts.incognito {
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to interim-save session: {e}");
                    }
                }
                // Track consecutive all-failed iterations to cap wasted retries.
                // (soft errors — approval denials, sandbox rejections, truncation —
                //  do NOT count; the LLM is expected to recover from those cheaply.)
                // NOTE: keep in sync with run_agent_loop_streaming.
                let hard_error_count = update_consecutive_hard_failures(
                    &mut consecutive_all_failed,
                    iteration_outcomes,
                );
                if consecutive_all_failed > 0
                    && hard_error_count > 0
                    && consecutive_all_failed >= MAX_CONSECUTIVE_ALL_FAILED
                {
                    warn!(
                        agent = %manifest.name,
                        consecutive_all_failed,
                        hard_error_count,
                        "Tool failures in {MAX_CONSECUTIVE_ALL_FAILED} consecutive iterations — exiting loop"
                    );
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: librefang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "reason": "tool_failure",
                            "error_count": hard_error_count,
                            "consecutive_all_failed": consecutive_all_failed,
                            "is_fork": opts.is_fork,
                        }),
                    };
                    fire_hook_best_effort(hooks, &ctx);
                    return Err(LibreFangError::RepeatedToolFailures {
                        iterations: consecutive_all_failed,
                        error_count: hard_error_count,
                    });
                }
            }
            StopReason::MaxTokens => {
                consecutive_max_tokens += 1;
                // If the LLM hit the token cap without emitting any tool
                // calls, this is a pure-text overflow — continuing would
                // only make the response longer without ever completing
                // an action, and downstream channels (Telegram: 4096 char
                // cap) will keep rejecting it. Return the partial text
                // immediately instead of burning more tokens (#2286).
                let pure_text_overflow = response.tool_calls.is_empty();
                if pure_text_overflow || consecutive_max_tokens >= MAX_CONTINUATIONS {
                    // Return partial response instead of continuing forever
                    let text = max_tokens_response_text(&response);
                    let (cleaned_text, parsed_directives) =
                        crate::reply_directives::parse_directives(&text);
                    let text = cleaned_text;
                    session.push_message(Message::assistant(&text));
                    if !opts.is_fork && !opts.incognito {
                        if let Err(e) = memory.save_session_async(session).await {
                            warn!("Failed to save session on max continuations: {e}");
                        }
                    }
                    if pure_text_overflow {
                        warn!(
                            iteration,
                            consecutive_max_tokens,
                            text_len = text.len(),
                            "Max tokens hit on pure-text response — returning partial (no tool calls to continue)"
                        );
                    } else {
                        warn!(
                            iteration,
                            consecutive_max_tokens,
                            "Max continuations reached, returning partial response"
                        );
                    }
                    // Fire AgentLoopEnd hook
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: librefang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "reason": "max_continuations",
                            "is_fork": opts.is_fork,
                        }),
                    };
                    fire_hook_best_effort(hooks, &ctx);
                    return Ok(AgentLoopResult {
                        response: text,
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: false,
                        directives: reply_directives_from_parsed(parsed_directives),
                        skill_evolution_suggested: decision_traces.len() >= 5,
                        decision_traces,
                        memories_saved,
                        memories_used,
                        memory_conflicts,
                        provider_not_configured: false,
                        experiment_context: experiment_context.clone(),
                        latency_ms: 0,
                        new_messages_start,
                        owner_notice: None,
                        actual_provider: None,
                    });
                }
                // Model hit token limit — add partial response and continue
                let text = response.text();
                session.push_message(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.push_message(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit, continuing");
            }
            StopReason::ContentFiltered => {
                // Provider refused / safety-filtered the response (#3450).
                // Persist any partial text and surface as a structured error
                // — never fall through into the EndTurn success path.
                let text = response.text();
                let partial = if text.trim().is_empty() {
                    "[content filtered by provider]".to_string()
                } else {
                    text
                };
                warn!(
                    agent = %manifest.name,
                    iteration,
                    "LLM response blocked by provider safety / content filter"
                );
                session.push_message(Message::assistant(&partial));
                if !opts.is_fork && !opts.incognito {
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to save session on content filter: {e}");
                    }
                }
                return Err(LibreFangError::ContentFiltered { message: partial });
            }
        }
    }

    // Save session before failing so conversation history is preserved.
    // Fork and incognito turns skip — both are ephemeral and must not
    // pollute canonical session history even when the loop bailed out.
    repair_session_before_save(session, agent_id_str.as_str(), "max_iterations");
    if !opts.is_fork && !opts.incognito {
        if let Err(e) = memory.save_session_async(session).await {
            warn!("Failed to save session on max iterations: {e}");
        }
    }

    // Fire AgentLoopEnd hook on max iterations exceeded
    let ctx = crate::hooks::HookContext {
        agent_name: &manifest.name,
        agent_id: agent_id_str.as_str(),
        event: librefang_types::agent::HookEvent::AgentLoopEnd,
        data: serde_json::json!({
            "reason": "max_iterations_exceeded",
            "iterations": max_iterations,
            "is_fork": opts.is_fork,
        }),
    };
    fire_hook_best_effort(hooks, &ctx);

    Err(LibreFangError::MaxIterationsExceeded(max_iterations))
}

#[cfg(test)]
mod tests;
