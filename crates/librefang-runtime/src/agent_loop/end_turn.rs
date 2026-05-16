//! End-of-turn finalization: serialize the new history slice for memory
//! persistence, classify "should we retry this turn?" against the retry
//! triggers, route the success path through proactive-memory writes, and
//! periodically fold stale tool results to keep the context window viable.

use super::message::sanitize_for_memory;
use super::prompt::{remember_interaction_best_effort, reply_directives_from_parsed};
use super::text_recovery::{looks_like_hallucinated_action, user_message_has_action_intent};
use super::*;

pub(super) struct FinalizeEndTurnContext<'a> {
    pub(super) manifest: &'a AgentManifest,
    pub(super) session: &'a mut Session,
    pub(super) memory: &'a MemorySubstrate,
    pub(super) embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    pub(super) context_engine: Option<&'a dyn ContextEngine>,
    pub(super) on_phase: Option<&'a PhaseCallback>,
    pub(super) proactive_memory: Option<&'a Arc<librefang_memory::ProactiveMemoryStore>>,
    pub(super) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(super) agent_id_str: &'a str,
    pub(super) user_message: &'a str,
    pub(super) messages: &'a [Message],
    pub(super) sender_user_id: Option<&'a str>,
    pub(super) streaming: bool,
    pub(super) opts: &'a LoopOptions,
}

pub(super) struct FinalizeEndTurnResultData {
    pub(super) final_response: String,
    pub(super) iteration: u32,
    pub(super) total_usage: TokenUsage,
    pub(super) decision_traces: Vec<DecisionTrace>,
    pub(super) memories_saved: Vec<String>,
    pub(super) memories_used: Vec<String>,
    pub(super) memory_conflicts: Vec<librefang_types::memory::MemoryConflict>,
    pub(super) experiment_context: Option<ExperimentContext>,
    pub(super) directives: librefang_types::message::ReplyDirectives,
    pub(super) new_messages_start: usize,
    /// Accumulated owner notices captured during this turn via the
    /// `notify_owner` tool. Multiple invocations join with "\n\n".
    pub(super) owner_notice: Option<String>,
}

pub(super) struct EndTurnRetryContext<'a> {
    pub(super) text: &'a str,
    pub(super) response: &'a crate::llm_driver::CompletionResponse,
    pub(super) iteration: u32,
    pub(super) available_tools: &'a [ToolDefinition],
    pub(super) any_tools_executed: bool,
    pub(super) hallucination_retried: bool,
    pub(super) action_nudge_retried: bool,
    pub(super) user_message: &'a str,
}

/// Serialize session messages into a JSON array for auto_memorize.
fn serialize_session_messages(
    messages: &[librefang_types::message::Message],
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let content_str = m.content.text_content();
            let role = match m.role {
                librefang_types::message::Role::System => "system",
                librefang_types::message::Role::User => "user",
                librefang_types::message::Role::Assistant => "assistant",
            };
            serde_json::json!({
                "role": role,
                "content": content_str
            })
        })
        .collect()
}

pub(super) fn build_silent_agent_loop_result(
    total_usage: TokenUsage,
    iterations: u32,
    parsed_directives: crate::reply_directives::DirectiveSet,
    decision_traces: Vec<DecisionTrace>,
    memories_used: Vec<String>,
    experiment_context: Option<ExperimentContext>,
    new_messages_start: usize,
) -> AgentLoopResult {
    AgentLoopResult {
        response: String::new(),
        total_usage,
        iterations,
        cost_usd: None,
        silent: true,
        directives: reply_directives_from_parsed(parsed_directives),
        decision_traces,
        memories_saved: Vec::new(),
        memories_used,
        memory_conflicts: Vec::new(),
        provider_not_configured: false,
        experiment_context,
        latency_ms: 0,
        new_messages_start,
        skill_evolution_suggested: false,
        owner_notice: None,
    }
}

pub(super) enum EndTurnRetry {
    EmptyResponse { is_silent_failure: bool },
    HallucinatedAction,
    ActionIntent,
}

pub(super) fn classify_end_turn_retry(ctx: EndTurnRetryContext<'_>) -> Option<EndTurnRetry> {
    if ctx.text.trim().is_empty() && ctx.response.tool_calls.is_empty() {
        let is_silent_failure =
            ctx.response.usage.input_tokens == 0 && ctx.response.usage.output_tokens == 0;
        if ctx.iteration == 0 || is_silent_failure {
            return Some(EndTurnRetry::EmptyResponse { is_silent_failure });
        }
    }

    if !ctx.text.trim().is_empty()
        && ctx.response.tool_calls.is_empty()
        && !ctx.available_tools.is_empty()
        && !ctx.any_tools_executed
        && !ctx.hallucination_retried
        && looks_like_hallucinated_action(ctx.text)
    {
        return Some(EndTurnRetry::HallucinatedAction);
    }

    if !ctx.text.trim().is_empty()
        && ctx.response.tool_calls.is_empty()
        && !ctx.available_tools.is_empty()
        && !ctx.any_tools_executed
        && !ctx.action_nudge_retried
        && !ctx.hallucination_retried
        && user_message_has_action_intent(ctx.user_message)
    {
        return Some(EndTurnRetry::ActionIntent);
    }

    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_end_turn_text(
    text: String,
    any_tools_executed: bool,
    manifest_name: &str,
    iteration: u32,
    total_usage: &TokenUsage,
    messages_count: usize,
    empty_response_log_message: &str,
    accumulated_text: &str,
) -> String {
    if text.trim().is_empty() {
        // Fallback to text accumulated from intermediate tool_use iterations.
        // Agents commonly emit a chat reply alongside tool_use blocks (e.g. a
        // user-facing message followed by memory_store calls); without this
        // fallback the final EndTurn iteration's empty text would mask that
        // earlier output and the empty-response guard would replace it with
        // a generic completion notice.
        if !accumulated_text.trim().is_empty() {
            debug!(
                agent = %manifest_name,
                accumulated_len = accumulated_text.len(),
                "Using accumulated text from intermediate tool_use iterations"
            );
            return accumulated_text.to_string();
        }
        warn!(
            agent = %manifest_name,
            iteration,
            input_tokens = total_usage.input_tokens,
            output_tokens = total_usage.output_tokens,
            messages_count,
            "{}",
            empty_response_log_message
        );
        if any_tools_executed {
            "[Task completed — the agent executed tools but did not produce a text summary.]"
                .to_string()
        } else {
            "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]".to_string()
        }
    } else {
        text
    }
}

pub(super) async fn finalize_successful_end_turn(
    ctx: FinalizeEndTurnContext<'_>,
    mut end_turn: FinalizeEndTurnResultData,
) -> LibreFangResult<AgentLoopResult> {
    ctx.session
        .push_message(Message::assistant(end_turn.final_response.clone()));

    let keep_recent = ctx
        .manifest
        .autonomous
        .as_ref()
        .and_then(|a| a.heartbeat_keep_recent)
        .unwrap_or(10);
    let before_prune_len = ctx.session.messages.len();
    crate::session_repair::prune_heartbeat_turns(&mut ctx.session.messages, keep_recent);
    if ctx.session.messages.len() != before_prune_len {
        ctx.session.mark_messages_mutated();
    }

    // Fork and incognito turns are ephemeral — skip the persist so the
    // parent agent's canonical session history isn't polluted by
    // derivative calls like auto-dream consolidation or incognito chats.
    // The LLM already ran and we have its response in memory; we just
    // don't write messages back to disk.
    if !ctx.opts.is_fork && !ctx.opts.incognito {
        ctx.memory
            .save_session_async(ctx.session)
            .await
            .map_err(LibreFangError::memory)?;
    }

    // Post-turn memory writes and context-engine updates are skipped for
    // fork and incognito turns. Three reasons for fork (unchanged): (1)
    // ephemeral conversation must not leak into long-term memory; (2)
    // context_engine state shouldn't advance; (3) auto_memorize recursion
    // guard. For incognito: memory reads remain full-access (the agent
    // already recalled memories before this point), but writes are
    // silently dropped so the private conversation leaves no trace.
    if !ctx.opts.is_fork && !ctx.opts.incognito {
        // Past-exchange shape (not `User asked:/I responded:`) and stripped of
        // channel envelopes so recall does not feed prompt-scaffolding-looking
        // bullets back into the LLM context (see sanitize_for_memory /
        // is_cascade_leak doc-comments). Skip persistence entirely when
        // either side is empty after sanitise — a half-empty memory row
        // would itself trip the leak guard on recall.
        if let (Some(user_clean), Some(resp_clean)) = (
            sanitize_for_memory(ctx.user_message),
            sanitize_for_memory(&end_turn.final_response),
        ) {
            let interaction_text =
                format!("[Past exchange]\nThem: {user_clean}\nYou: {resp_clean}");
            remember_interaction_best_effort(
                ctx.memory,
                ctx.embedding_driver,
                ctx.session.agent_id,
                &interaction_text,
                ctx.streaming,
            )
            .await;
        }

        if let Some(engine) = ctx.context_engine {
            if let Err(e) = engine.after_turn(ctx.session.agent_id, ctx.messages).await {
                warn!("Context engine after_turn failed: {e}");
            }
        }
    }

    if let Some(cb) = ctx.on_phase {
        cb(LoopPhase::Done);
    }

    info!(
        agent = %ctx.manifest.name,
        iterations = end_turn.iteration + 1,
        tokens = end_turn.total_usage.total(),
        is_fork = ctx.opts.is_fork,
        "{}",
        if ctx.streaming {
            "Streaming agent loop completed"
        } else {
            "Agent loop completed"
        }
    );

    // Prompt-cache observability (M2): emit a single-line metric so log
    // pipelines can compute hit-rate trends per agent without parsing the
    // surrounding loop summary. `None` (no caching activity) is folded to
    // 0.0 for the log field; readers wanting to distinguish "no caching"
    // from "0% hit" should look at the `creation` + `read` totals.
    tracing::info!(
        target: "librefang::cache",
        agent = ctx.agent_id_str,
        hit_ratio = end_turn.total_usage.cache_hit_ratio().unwrap_or(0.0),
        creation = end_turn.total_usage.cache_creation_input_tokens,
        read = end_turn.total_usage.cache_read_input_tokens,
        "prompt cache metrics for turn"
    );

    if !ctx.opts.is_fork {
        if let Some(pm_store) = ctx.proactive_memory {
            let user_id = ctx.session.agent_id.0.to_string();
            let new_messages = &ctx.session.messages[end_turn.new_messages_start..];
            let messages_json = serialize_session_messages(new_messages);
            match pm_store
                .auto_memorize(&user_id, &messages_json, ctx.sender_user_id)
                .await
            {
                Ok(result) if result.has_content => {
                    debug!(
                        memories = result.memories.len(),
                        relations = result.relations.len(),
                        "{}",
                        if ctx.streaming {
                            "Proactive memory (streaming): stored {} memories, {} relations"
                        } else {
                            "Proactive memory: stored {} memories, {} relations"
                        }
                    );
                    end_turn
                        .memories_saved
                        .extend(result.memories.iter().map(|m| m.content.clone()));
                    end_turn.memory_conflicts.extend(result.conflicts);
                }
                Ok(_) => {}
                Err(e) => {
                    if ctx.streaming {
                        warn!("Proactive memory auto_memorize failed (streaming): {e}");
                    } else {
                        warn!("Proactive memory auto_memorize failed: {e}");
                    }
                }
            }
        }
    }

    let hook_ctx = crate::hooks::HookContext {
        agent_name: &ctx.manifest.name,
        agent_id: ctx.agent_id_str,
        event: librefang_types::agent::HookEvent::AgentLoopEnd,
        data: serde_json::json!({
            "iterations": end_turn.iteration + 1,
            "response_length": end_turn.final_response.len(),
            "is_fork": ctx.opts.is_fork,
        }),
    };
    fire_hook_best_effort(ctx.hooks, &hook_ctx);

    let tool_call_count = end_turn.decision_traces.len();
    Ok(AgentLoopResult {
        response: end_turn.final_response,
        total_usage: end_turn.total_usage,
        iterations: end_turn.iteration + 1,
        cost_usd: None,
        silent: false,
        directives: end_turn.directives,
        decision_traces: end_turn.decision_traces,
        memories_saved: end_turn.memories_saved,
        memories_used: end_turn.memories_used,
        memory_conflicts: end_turn.memory_conflicts,
        provider_not_configured: false,
        experiment_context: end_turn.experiment_context,
        latency_ms: 0,
        new_messages_start: end_turn.new_messages_start,
        skill_evolution_suggested: tool_call_count >= 5,
        owner_notice: end_turn.owner_notice.clone(),
    })
}

/// Shared fold pass for the streaming and non-streaming agent loops
/// (#3347 3/N + #4 review-followup DRY).  Both loops previously inlined
/// the same fast-path / call / debug-log block; collapsing them into one
/// helper prevents the two paths from drifting (e.g. one branch picking
/// up a new `min_batch_size` knob and the other lagging).
///
/// Returns the (possibly modified) working-copy message list — same
/// semantics as [`crate::history_fold::fold_stale_tool_results`] but with
/// the fast-path, call-site logging, and durable-session replay baked in.
/// The durable replay (issue #4866 axis 2) walks `session.messages` and
/// rewrites every matching `ToolResult.content` by `tool_use_id`, then
/// calls `session.mark_messages_mutated()`.  Without that step the fold
/// runs from scratch every turn — see `history_fold.rs` module doc.
/// `streaming` flips the log target so operators can grep `"streaming"`
/// vs `"non-streaming"` in production.
///
/// **Ordering note** — when soft-compression later in this same loop
/// iteration fires `session.set_messages(messages.clone())`, it writes
/// the (already-folded) working copy back over `session.messages`, so
/// the explicit replay above is redundant on that path.  The replay is
/// load-bearing only on the no-compression path; keeping it on both
/// paths keeps session-mutation semantics uniform and removes a foot-gun
/// for future refactors that move the fold call out from under the
/// compression step.
// Eight args because every parameter is genuinely distinct (working
// messages, durable session, knobs, model, aux+primary driver chain,
// streaming flag, reasoning policy) and bundling them into a context
// struct would just be a positional alias.  FoldConfig already pulls the
// two knobs out; further bundling would obscure the call sites.
#[allow(clippy::too_many_arguments)]
pub(super) async fn maybe_fold_stale_tool_results(
    messages: Vec<Message>,
    session: &mut Session,
    fold_cfg: crate::history_fold::FoldConfig,
    model: &str,
    aux_client: Option<&crate::aux_client::AuxClient>,
    driver: Arc<dyn LlmDriver>,
    streaming: bool,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Vec<Message> {
    // Fast-path: a fold pass needs at least `fold_after_turns` *recent*
    // assistant turns plus one stale turn — i.e. more than
    // `fold_after_turns * 2` messages — before any tool-result can be
    // classified stale.  Skipping the call avoids even the index walk
    // inside `collect_stale_indices` on every short-session iteration.
    if fold_cfg.fold_after_turns == 0
        || messages.len() <= (fold_cfg.fold_after_turns as usize).saturating_mul(2)
    {
        return messages;
    }
    let (folded, fold_result) = crate::history_fold::fold_stale_tool_results(
        messages,
        fold_cfg,
        model,
        aux_client,
        driver,
        reasoning_echo_policy,
    )
    .await;
    // Replay the fold onto `session.messages` so the rewrite is persisted
    // on the next `save_session_async` and subsequent turns short-circuit
    // via `is_already_folded` instead of re-summarising from scratch.
    // Matching by `tool_use_id` lets the working copy and durable list
    // drift in length/ordering without breaking the projection.
    if !fold_result.rewrites.is_empty() {
        let durable_changed =
            crate::history_fold::apply_fold_rewrites(&mut session.messages, &fold_result.rewrites);
        if durable_changed {
            session.mark_messages_mutated();
        }
    }
    if fold_result.groups_folded > 0 {
        let label = if streaming {
            "streaming"
        } else {
            "non-streaming"
        };
        debug!(
            groups = fold_result.groups_folded,
            replaced = fold_result.messages_replaced,
            groups_used_fallback = fold_result.groups_used_fallback,
            durable_rewrites = fold_result.rewrites.len(),
            "history_fold: fold pass complete ({label})"
        );
    }
    folded
}

/// Gate the proactive-memory store for the *retrieve* side based on the
/// per-agent override in `manifest.proactive_memory` (#4870).
///
/// Returns `Some(store)` when both the kernel-global config and the
/// per-agent override allow `auto_retrieve`, else `None`. The store's
/// internal gate also reads the global config, so the only case the
/// outer gate covers that the inner doesn't is **per-agent opt-out**
/// (the issue's primary use case: cron sub-agents that should skip
/// retrieval entirely).
pub(super) fn gated_proactive_memory_for_retrieve<'a>(
    manifest: &AgentManifest,
    pm: Option<&'a Arc<librefang_memory::ProactiveMemoryStore>>,
) -> Option<&'a Arc<librefang_memory::ProactiveMemoryStore>> {
    let store = pm?;
    if manifest.proactive_memory.is_empty() {
        return Some(store);
    }
    let global = store.config();
    if manifest.proactive_memory.resolve_auto_retrieve(&global) {
        Some(store)
    } else {
        tracing::debug!(
            agent = %manifest.name,
            "Per-agent override disables auto_retrieve; skipping proactive memory retrieval"
        );
        None
    }
}

/// Gate the proactive-memory store for the *memorize* side based on the
/// per-agent override in `manifest.proactive_memory` (#4870). See
/// [`gated_proactive_memory_for_retrieve`] for the rationale.
pub(super) fn gated_proactive_memory_for_memorize<'a>(
    manifest: &AgentManifest,
    pm: Option<&'a Arc<librefang_memory::ProactiveMemoryStore>>,
) -> Option<&'a Arc<librefang_memory::ProactiveMemoryStore>> {
    let store = pm?;
    if manifest.proactive_memory.is_empty() {
        return Some(store);
    }
    let global = store.config();
    if manifest.proactive_memory.resolve_auto_memorize(&global) {
        Some(store)
    } else {
        tracing::debug!(
            agent = %manifest.name,
            "Per-agent override disables auto_memorize; skipping proactive memory extraction"
        );
        None
    }
}
