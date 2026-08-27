//! End-of-turn finalization: serialize the new history slice for memory
//! persistence, classify "should we retry this turn?" against the retry
//! triggers, route the success path through proactive-memory writes, and
//! periodically fold stale tool results to keep the context window viable.

use super::message::{budget_interaction_halves, sanitize_for_memory};
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
    /// Chat-qualified `(channel, chat)` scope of the inbound that
    /// triggered this turn (e.g. `"telegram:<chatId>"`,
    /// `"whatsapp:<jid>"`). Threaded down so `auto_memorize` can stamp
    /// extracted memories with their originating chat — preventing them
    /// from being recalled into a DIFFERENT chat with the same
    /// (agent, peer) on a later turn (#5227). `None` when the turn was
    /// kicked off without channel context (direct API, dashboard); in
    /// that case memories remain chat-agnostic, matching legacy
    /// behaviour. Composed at the kernel inject site via
    /// `librefang_types::agent::compose_sender_scope` so the formula
    /// matches `SessionId::for_sender_scope`'s scope-string composition.
    pub(super) sender_chat_scope: Option<&'a str>,
    /// Identity of the session this turn belongs to, when session-scoped recall is in effect for the agent (#7605).
    /// Stamped onto every memory `auto_memorize` extracts, so a later turn in a different session will not recall it.
    /// `None` stores session-agnostic memories, which is the pre-#7605 behaviour and what an operator gets back by setting `session_scoped_recall = false`.
    /// Produced by [`session_recall_scope`].
    pub(super) session_scope: Option<&'a str>,
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
    /// Provider slot that actually served the LLM request (#4807 nit
    /// 10). Carried through to [`AgentLoopResult::actual_provider`].
    pub(super) actual_provider: Option<String>,
    /// Model the last LLM call actually ran (#6134). Carried through to
    /// [`AgentLoopResult::actual_model`].
    pub(super) actual_model: Option<String>,
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
        actual_provider: None,
        actual_model: None,
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

    let preconditions_met = !ctx.text.trim().is_empty()
        && ctx.response.tool_calls.is_empty()
        && !ctx.available_tools.is_empty()
        && !ctx.any_tools_executed;

    if preconditions_met && !ctx.hallucination_retried && looks_like_hallucinated_action(ctx.text) {
        return Some(EndTurnRetry::HallucinatedAction);
    }

    if preconditions_met
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
    let pruned_before_new_messages = crate::session_repair::prune_heartbeat_turns_tracking_boundary(
        &mut ctx.session.messages,
        keep_recent,
        end_turn.new_messages_start,
    );
    if ctx.session.messages.len() != before_prune_len {
        ctx.session.mark_messages_mutated();
    }
    end_turn.new_messages_start = end_turn
        .new_messages_start
        .saturating_sub(pruned_before_new_messages)
        .min(ctx.session.messages.len());

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
            // Bound the row before it is composed, so the cap applies to what gets embedded as well as to what gets stored (#7911).
            // A channel adapter that renders an attachment into the user message used to land here verbatim — the issue reports a single 201 765-character row that had been retrieved 63 times.
            let (user_clean, resp_clean) = budget_interaction_halves(
                &user_clean,
                &resp_clean,
                ctx.memory.max_episodic_chars(),
            );
            let interaction_text =
                format!("[Past exchange]\nThem: {user_clean}\nYou: {resp_clean}");
            // `sender_user_id` (SenderContext.user_id) is the platform user
            // identity — e.g. a Telegram user ID.  This differs from
            // `sessions.peer_id` in the session-store layer (PR #5286),
            // which uses `chat_id` for session-scope isolation.  The
            // divergence is intentional: memory recall filters on
            // `(agent_id, peer_id)` with `peer_id = sender_user_id` so that
            // each user's episodic memories are isolated within a shared
            // agent (per-user recall).  Session isolation (chat-scoped) is a
            // separate concern and uses chat_id.  If both were collapsed to
            // the same value a group-chat user's memory recall would be
            // scoped to the chat rather than to the individual.
            remember_interaction_best_effort(
                ctx.memory,
                ctx.embedding_driver,
                ctx.session.agent_id,
                &interaction_text,
                ctx.streaming,
                ctx.sender_user_id,
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

    if !ctx.opts.is_fork && !ctx.opts.incognito {
        if let Some(pm_store) = ctx.proactive_memory {
            let user_id = ctx.session.agent_id.0.to_string();
            let new_messages = &ctx.session.messages[end_turn.new_messages_start..];
            let messages_json = serialize_session_messages(new_messages);
            match pm_store
                .auto_memorize(
                    &user_id,
                    &messages_json,
                    ctx.sender_user_id,
                    ctx.sender_chat_scope,
                    ctx.session_scope,
                )
                .await
            {
                Ok(result) if result.has_content => {
                    debug!(
                        memories = result.memories.len(),
                        relations = result.relations.len(),
                        "Proactive memory{}: stored {} memories, {} relations",
                        if ctx.streaming { " (streaming)" } else { "" },
                        result.memories.len(),
                        result.relations.len(),
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
        actual_provider: end_turn.actual_provider,
        actual_model: end_turn.actual_model,
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

/// Gate the proactive-memory store for the *retrieve* side (#4870, #7605).
///
/// Two independent reasons to skip retrieval, checked in this order:
///
/// 1. `capabilities.memory_read` is declared and grants no scope covering the agent's own memory (#7605).
///    An operator who writes `memory_read = []` into `agent.toml` is asking for exactly that, and until this gate existed they still got a populated `memories_used` on every turn.
///    An absent `memory_read` key stays permissive, matching how every other capability list reads when it is not there.
/// 2. The per-agent `[proactive_memory]` override disables `auto_retrieve` (#4870).
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
    if !manifest.capabilities.allows_own_memory_read() {
        tracing::debug!(
            agent = %manifest.name,
            "capabilities.memory_read grants no scope over this agent's own memory; skipping proactive memory retrieval"
        );
        return None;
    }
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

/// Gate the proactive-memory store for the *memorize* side (#4870, #7605).
/// `capabilities.memory_write` gates it the way `memory_read` gates the retrieve half; see [`gated_proactive_memory_for_retrieve`] for the rationale and for why a declared-empty list is not the same as an absent one.
pub(super) fn gated_proactive_memory_for_memorize<'a>(
    manifest: &AgentManifest,
    pm: Option<&'a Arc<librefang_memory::ProactiveMemoryStore>>,
) -> Option<&'a Arc<librefang_memory::ProactiveMemoryStore>> {
    let store = pm?;
    if !manifest.capabilities.allows_own_memory_write() {
        tracing::debug!(
            agent = %manifest.name,
            "capabilities.memory_write grants no scope over this agent's own memory; skipping proactive memory extraction"
        );
        return None;
    }
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

/// Resolve the session identity that memory reads and writes are scoped to for this turn (#7605).
///
/// Returns `Some(session_id_string)` when session-scoped recall is in effect, which is the default, and `None` when the operator has turned it off — globally via `[proactive_memory] session_scoped_recall = false` in `config.toml`, or for one agent via the same key in the `[proactive_memory]` block of `agent.toml`.
///
/// The identity is the session the loop is already reading and writing, not a new one derived here: whatever the resolution ladder in `docs/architecture/session-mode-resolution.md` settled on for this invocation — the canonical session for a bare `librefang message`, the caller's `session_id` for a REST turn or `librefang message --session-id`, `SessionId::for_channel` for a channel message, the parent's session for a fork.
///
/// A consequence worth naming: an agent with `session_mode = "new"` gets a fresh session per invocation, so with this on it no longer auto-recalls what a previous invocation memorized.
/// That is the intended reading of "new" — an isolated turn — but an agent that relied on the old agent-wide pool for continuity wants `session_scoped_recall = false` in its `agent.toml`.
pub(super) fn session_recall_scope(
    manifest: &AgentManifest,
    session: &librefang_memory::session::Session,
    pm: Option<&Arc<librefang_memory::ProactiveMemoryStore>>,
) -> Option<String> {
    let global = pm?.config();
    if !manifest
        .proactive_memory
        .resolve_session_scoped_recall(&global)
    {
        return None;
    }
    Some(session.id.0.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::{MemorySubstrate, ProactiveMemoryStore};
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::memory::ProactiveMemoryConfig;

    fn store(cfg: ProactiveMemoryConfig) -> Arc<ProactiveMemoryStore> {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        Arc::new(ProactiveMemoryStore::new(substrate, cfg))
    }

    fn manifest_from(capabilities_toml: &str) -> AgentManifest {
        let toml_str = format!(
            r#"
name = "gate-test"
version = "0.1.0"
description = "d"
author = "t"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "s"

{capabilities_toml}
"#
        );
        toml::from_str(&toml_str).expect("manifest parses")
    }

    fn session_for(agent_id: AgentId) -> librefang_memory::session::Session {
        librefang_memory::session::Session {
            id: SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
            model_override: None,
            messages_generation: 0,
            last_repaired_generation: None,
            peer_id: None,
        }
    }

    /// #7605 — the reporter set `memory_read = []` / `memory_write = []`,
    /// reloaded the agent, and still got a populated `memories_used` on every
    /// turn plus a growing store. Both gates must fail closed on a declared
    /// empty list.
    #[test]
    fn declared_empty_memory_scopes_close_both_automatic_memory_gates() {
        let pm = store(ProactiveMemoryConfig::default());
        let manifest =
            manifest_from("[capabilities]\ntools = []\nmemory_read = []\nmemory_write = []");

        assert!(
            gated_proactive_memory_for_retrieve(&manifest, Some(&pm)).is_none(),
            "memory_read = [] must stop memories being injected into the prompt"
        );
        assert!(
            gated_proactive_memory_for_memorize(&manifest, Some(&pm)).is_none(),
            "memory_write = [] must stop the turn being auto-memorized"
        );
    }

    /// The two halves are independent: an agent may read what it was given
    /// without recording anything new, or the reverse.
    #[test]
    fn memory_read_and_write_scopes_gate_their_own_half_only() {
        let pm = store(ProactiveMemoryConfig::default());

        let read_only = manifest_from("[capabilities]\nmemory_read = [\"*\"]\nmemory_write = []");
        assert!(gated_proactive_memory_for_retrieve(&read_only, Some(&pm)).is_some());
        assert!(gated_proactive_memory_for_memorize(&read_only, Some(&pm)).is_none());

        let write_only =
            manifest_from("[capabilities]\nmemory_read = []\nmemory_write = [\"self.*\"]");
        assert!(gated_proactive_memory_for_retrieve(&write_only, Some(&pm)).is_none());
        assert!(gated_proactive_memory_for_memorize(&write_only, Some(&pm)).is_some());
    }

    /// An agent that never declared memory scopes keeps the behaviour it had
    /// before #7605 — most manifests in the wild have no `[capabilities]`
    /// block at all, and failing closed on them would silently switch memory
    /// off across an upgrade.
    #[test]
    fn undeclared_memory_scopes_leave_both_gates_open() {
        let pm = store(ProactiveMemoryConfig::default());
        let manifest = manifest_from("");
        assert!(gated_proactive_memory_for_retrieve(&manifest, Some(&pm)).is_some());
        assert!(gated_proactive_memory_for_memorize(&manifest, Some(&pm)).is_some());
    }

    /// The #4870 per-agent override still gates each half on its own, and is
    /// checked independently of the capability gate added by #7605.
    #[test]
    fn per_agent_proactive_memory_override_still_gates_each_half() {
        let pm = store(ProactiveMemoryConfig::default());

        let no_memorize = manifest_from("[proactive_memory]\nauto_memorize = false");
        assert!(gated_proactive_memory_for_retrieve(&no_memorize, Some(&pm)).is_some());
        assert!(gated_proactive_memory_for_memorize(&no_memorize, Some(&pm)).is_none());

        let no_retrieve = manifest_from("[proactive_memory]\nauto_retrieve = false");
        assert!(gated_proactive_memory_for_retrieve(&no_retrieve, Some(&pm)).is_none());
        assert!(gated_proactive_memory_for_memorize(&no_retrieve, Some(&pm)).is_some());

        let off = manifest_from("[proactive_memory]\nenabled = false");
        assert!(gated_proactive_memory_for_retrieve(&off, Some(&pm)).is_none());
        assert!(gated_proactive_memory_for_memorize(&off, Some(&pm)).is_none());
    }

    /// #7605 — the scope handed to the store is the session the turn belongs
    /// to, and it is on by default.
    #[test]
    fn session_recall_scope_defaults_to_the_turns_own_session() {
        let pm = store(ProactiveMemoryConfig::default());
        let manifest = manifest_from("");
        let session = session_for(AgentId::new());

        assert_eq!(
            session_recall_scope(&manifest, &session, Some(&pm)),
            Some(session.id.0.to_string()),
            "with nothing configured, memories are scoped to the session that produced them"
        );

        // Two turns of the same session agree; a different session does not.
        let other = session_for(session.agent_id);
        assert_ne!(
            session_recall_scope(&manifest, &other, Some(&pm)),
            session_recall_scope(&manifest, &session, Some(&pm))
        );
    }

    #[test]
    fn session_recall_scope_can_be_turned_off_per_agent_and_globally() {
        let pm = store(ProactiveMemoryConfig::default());
        let session = session_for(AgentId::new());

        let opted_out = manifest_from("[proactive_memory]\nsession_scoped_recall = false");
        assert_eq!(
            session_recall_scope(&opted_out, &session, Some(&pm)),
            None,
            "a single-user agent must be able to keep one memory pool across its sessions"
        );

        let global_off = store(ProactiveMemoryConfig {
            session_scoped_recall: false,
            ..Default::default()
        });
        assert_eq!(
            session_recall_scope(&manifest_from(""), &session, Some(&global_off)),
            None
        );
        let opted_in = manifest_from("[proactive_memory]\nsession_scoped_recall = true");
        assert_eq!(
            session_recall_scope(&opted_in, &session, Some(&global_off)),
            Some(session.id.0.to_string()),
            "one agent must be able to isolate itself when the deployment default is off"
        );
    }

    /// No store, no scope — and no panic on the `?` in `session_recall_scope`.
    #[test]
    fn session_recall_scope_is_none_without_a_proactive_store() {
        let session = session_for(AgentId::new());
        assert_eq!(
            session_recall_scope(&manifest_from(""), &session, None),
            None
        );
    }
}
