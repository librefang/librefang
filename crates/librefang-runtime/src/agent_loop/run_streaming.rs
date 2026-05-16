//! `run_agent_loop_streaming` — the streaming variant of the main loop.
//!
//! Like `run_agent_loop`, but sends `StreamEvent`s to the provided channel
//! as tokens arrive from the LLM. Tool execution happens between LLM calls
//! and is not streamed. Extracted into its own file to keep `mod.rs` under
//! the #3710 2,000 LOC cap.

use super::retry::stream_with_retry;
use super::*;

/// Run the agent execution loop with streaming support.
///
/// Like `run_agent_loop`, but sends `StreamEvent`s to the provided channel
/// as tokens arrive from the LLM. Tool execution happens between LLM calls
/// and is not streamed.
#[allow(clippy::too_many_arguments)]
// `level = "warn"` to survive the daemon's `librefang_runtime=warn` baseline
// filter — see the comment on `run_agent_loop` above. Also fold in
// `session.id` so streaming events get the same correlation surface.
#[instrument(level = "warn", skip_all, fields(agent.name = %manifest.name, agent.id = %session.agent_id, session.id = %session.id))]
pub async fn run_agent_loop_streaming(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    stream_tx: mpsc::Sender<StreamEvent>,
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
    info!(agent = %manifest.name, "Starting streaming agent loop");

    // Start index of new messages added during this turn. See the matching
    // comment in run_agent_loop for details. Initialized to the current
    // session length, updated post-trim to len-1. Fixes #2067.
    let mut new_messages_start = session.messages.len();

    // Skip streaming agent loop if no LLM provider is configured.
    if !driver.is_configured() {
        info!(agent = %manifest.name, "Skipping streaming agent loop — no LLM provider configured");
        return Ok(AgentLoopResult {
            silent: true,
            provider_not_configured: true,
            experiment_context: None,
            new_messages_start,
            ..Default::default()
        });
    }

    // Gateway-level safety-net compression (#4972). See the matching block
    // in `run_agent_loop` for rationale. Same pure-function entry point.
    if let (Some(cfg), Some(ctx_window)) = (
        opts.gateway_compression.as_ref(),
        context_window_tokens.filter(|w| *w > 0),
    ) {
        let ctx_window_u32: u32 = ctx_window.try_into().unwrap_or(u32::MAX);
        let report =
            crate::gateway_compression::apply_if_needed(&mut session.messages, ctx_window_u32, cfg);
        if report.mutated() {
            // See the matching block in `run_agent_loop` for why we don't
            // touch `new_messages_start` here — `prepare_messages`
            // recomputes it unconditionally after `safe_trim_messages`.
            session.mark_messages_mutated();
            info!(
                agent = %manifest.name,
                session_id = %session.id,
                tokens_before = report.tokens_before,
                tokens_after = report.tokens_after,
                tool_results_stubbed = report.tool_results_stubbed,
                tool_result_bytes_elided = report.tool_result_bytes_elided,
                messages_dropped = report.messages_dropped,
                "Gateway compression pruned session before streaming loop (#4972)"
            );
        } else if report.fired {
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
    } = select_running_experiment(manifest, session, kernel.as_ref(), true);

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
        streaming: true,
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
        streaming: true,
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
    // Accumulated text from intermediate tool_use iterations — see the
    // matching declaration in run_agent_loop for full rationale.
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

    // Pre-clone the system prompt for the streaming loop.  Identical rationale
    // to the non-streaming path: constant across iterations, cloned per-LLM call
    // because CompletionRequest takes ownership, so clone once up-front.
    let system_prompt_snapshot = system_prompt.clone();

    // Resolve tool list once before the loop and reuse via Arc on every
    // iteration.  See `ResolvedToolsCache` for rationale (#3586).
    let mut tools_cache =
        ResolvedToolsCache::new(available_tools, &session_loaded_tools, lazy_tools);

    for iteration in 0..max_iterations {
        debug!(iteration, "Streaming agent loop iteration");

        // Check for session-scoped interrupt at each iteration boundary.
        if opts.interrupt.as_ref().is_some_and(|i| i.is_cancelled()) {
            debug!(
                iteration,
                "Streaming agent loop interrupted by session cancel signal"
            );
            return Ok(AgentLoopResult {
                silent: true,
                new_messages_start,
                ..Default::default()
            });
        }

        // Pluggable context engine: threshold-gated compaction (same as the
        // non-streaming loop). `last_prompt_tokens` carries only the previous
        // turn's prompt cost — never the cumulative total.  `total_usage`
        // (accumulated) is never read or written here.
        if let Some(engine) = context_engine {
            if engine.should_compress(last_prompt_tokens, ctx_window) {
                debug!(
                    iteration,
                    last_prompt_tokens,
                    ctx_window,
                    "Context engine requested compaction (streaming path)"
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
                            "Context engine compaction complete (streaming)"
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
                        // `last_prompt_tokens` is NOT reset — see non-streaming
                        // comment for rationale.
                    }
                    Err(e) => {
                        warn!("Context engine compaction failed (continuing, streaming): {e}");
                    }
                }
            }
        }

        // History fold (#3347 3/N): rewrite stale tool-result blocks in place
        // before context assembly — streaming path mirrors non-streaming via
        // `maybe_fold_stale_tool_results` (#4 review-followup DRY).
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
            true,
            fold_echo_policy,
        )
        .await;

        // Context assembly — use context engine if available, else inline logic
        let recovery = if let Some(engine) = context_engine {
            let result = engine
                .assemble(
                    session.agent_id,
                    &mut messages,
                    &system_prompt,
                    available_tools,
                    ctx_window,
                )
                .await?;
            result.recovery
        } else {
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

            let remaining_tokens = crate::compactor::estimate_token_count(
                &messages,
                Some(&system_prompt),
                Some(available_tools),
            );
            let hard_trim_threshold = (ctx_window as f64 * 0.70) as usize;
            let recovery = if remaining_tokens > hard_trim_threshold {
                let r = recover_from_overflow(
                    &mut messages,
                    &system_prompt,
                    available_tools,
                    ctx_window,
                );
                hard_trimmed = r != RecoveryStage::None;
                r
            } else {
                RecoveryStage::None
            };

            if hard_trimmed {
                messages = crate::session_repair::validate_and_repair(&messages);
                messages = crate::session_repair::ensure_starts_with_user(messages);
            }
            if had_soft_compression {
                session.set_messages(messages.clone());
            }
            apply_context_guard(&mut messages, &context_budget, available_tools);
            recovery
        };
        match &recovery {
            RecoveryStage::None => {}
            RecoveryStage::FinalError => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Context overflow unrecoverable. Use /reset or /compact.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context overflow warning");
                }
            }
            _ => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Older messages trimmed to stay within context limits. Use /compact for smarter summarization.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context trim warning");
                }
            }
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

        // Per-request timeout: manifest metadata takes priority, then browser
        // heuristic, then driver default (None = use driver's configured value).
        let timeout_override = manifest
            .metadata
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                // Auto-extend for agents with browser tools
                if available_tools
                    .iter()
                    .any(|t| t.name.starts_with("browser_") || t.name.starts_with("playwright_"))
                {
                    Some(600) // 10 minutes for browser tasks
                } else {
                    None
                }
            });

        // Catalog-driven reasoning_echo_policy lookup (#4842), same as the
        // non-streaming path above.
        let reasoning_echo_policy = kernel
            .as_ref()
            .map(|k| k.reasoning_echo_policy_for(&api_model))
            .unwrap_or_default();

        // Same Arc-wrap as the non-streaming hot path (#3766).
        let request = CompletionRequest {
            model: api_model,
            messages: std::sync::Arc::new(messages.clone()),
            tools: tools_cache.get(available_tools, &session_loaded_tools),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            // Clone from pre-built snapshot (same rationale as non-streaming loop).
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

        // Notify phase: on first iteration emit Streaming; on subsequent
        // iterations (after tool execution) emit Thinking so the UI shows
        // "Thinking..." instead of overwriting streamed text with "streaming".
        if let Some(cb) = on_phase {
            if iteration == 0 {
                cb(LoopPhase::Streaming);
            } else {
                cb(LoopPhase::Thinking);
            }
        }

        // Stamp last_active before LLM call to prevent heartbeat false-positives
        // during long-running completions.
        if let Some(ref k) = kernel {
            k.touch_heartbeat(&agent_id_str);
        }

        // Stream LLM call with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let stream_result = match stream_with_retry(
            &*driver,
            request,
            stream_tx.clone(),
            Some(provider_name),
            None,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("timed out") {
                    // Extract last_activity from error if present (format: "last: <activity>")
                    let activity = err_str
                        .find("last: ")
                        .map(|i| {
                            let start = i + 6;
                            let end = err_str[start..]
                                .find(')')
                                .map_or(err_str.len(), |j| start + j);
                            &err_str[start..end]
                        })
                        .unwrap_or("unknown");
                    let note = format!(
                        "[System: your previous task timed out while doing: {activity}. \
                         The user's request could not be completed. \
                         Any partial output was already sent to the user.]"
                    );
                    session.push_message(Message::assistant(note));
                    repair_session_before_save(session, agent_id_str.as_str(), "streaming_timeout");
                    if !opts.is_fork && !opts.incognito {
                        if let Err(save_err) = memory.save_session_async(session).await {
                            warn!(
                                "Failed to persist timeout note to session: {save_err}. \
                                 The timeout marker will not appear on next session load."
                            );
                        }
                    }
                }
                return Err(e);
            }
        };

        // Incremental cascade-leak guard fired mid-stream: the forward task
        // already stopped emitting TextDelta. Treat the turn as a silent
        // drop unconditionally — regardless of stop_reason (including
        // ToolUse). This prevents an attacker from leaking the system prompt
        // and then forcing tool execution by emitting a tool_use block after
        // the leak trigger.
        if stream_result.cascade_leak_aborted {
            warn!(
                event = "silent_response_detected",
                agent = %manifest.name,
                reason = ?crate::silent_response::SilentReason::PromptRegurgitated,
                source = "agent_loop.streaming.incremental",
                "Incremental cascade-leak guard fired mid-stream — aborting turn, delivering [no reply needed]"
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
                Default::default(),
                decision_traces,
                memories_used.clone(),
                experiment_context.clone(),
                new_messages_start,
            ));
        }

        let mut response = stream_result.response;

        accumulate_token_usage(&mut total_usage, &response.usage);

        // Snapshot prompt tokens for the next iteration's should_compress check.
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

        // Recover tool calls output as text (streaming path)
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
                    "Recovered text-based tool calls (streaming) → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                tools_recovered_from_text = true;
                response.content = tool_use_blocks_from_calls(&response.tool_calls);
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                let text = response.text();

                // Parse reply directives from the streaming response text
                let (cleaned_text_s, parsed_directives_s) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text_s;

                // NO_REPLY: agent intentionally chose not to reply
                if is_no_reply(&text) || parsed_directives_s.silent {
                    let reason = if parsed_directives_s.silent {
                        crate::silent_response::SilentReason::PolicyBlock
                    } else {
                        crate::silent_response::SilentReason::NoReply
                    };
                    info!(
                        event = "silent_response_detected",
                        agent = %manifest.name,
                        reason = ?reason,
                        source = "agent_loop.streaming",
                        "Agent chose silent completion"
                    );
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent (streaming) — silent completion");
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
                        parsed_directives_s,
                        decision_traces,
                        memories_used.clone(),
                        experiment_context.clone(),
                        new_messages_start,
                    ));
                }

                // Cascade scaffolding-leak guard (streaming path) — see
                // non-stream mirror above. Drops text-only EndTurn replies
                // that contain 2+ structural prompt/memory markers.
                if response.tool_calls.is_empty()
                    && !tools_recovered_from_text
                    && is_cascade_leak(&text)
                {
                    warn!(
                        agent = %manifest.name,
                        text_excerpt = %text.chars().take(120).collect::<String>(),
                        "Cascade scaffolding leak detected (streaming, 2+ structural markers in text-only EndTurn) — dropping as silent"
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
                        parsed_directives_s,
                        decision_traces,
                        memories_used.clone(),
                        experiment_context.clone(),
                        new_messages_start,
                    ));
                }

                // Progress-text-leak guard (streaming path) — see non-stream
                // mirror above. Drops ellipsis-terminated short preambles
                // that arrive without the promised tool_use.
                if response.tool_calls.is_empty()
                    && !tools_recovered_from_text
                    && is_progress_text_leak(&text)
                {
                    warn!(
                        agent = %manifest.name,
                        text_excerpt = %text.chars().take(80).collect::<String>(),
                        "Progress-text leak detected (streaming, ellipsis-terminated short reply without tool_use) — dropping as silent"
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
                        parsed_directives_s,
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
                            "Empty response (streaming), retrying once"
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
                        info!(
                            agent = %manifest.name,
                            iteration,
                            "Detected hallucinated action (streaming) — agent claimed action without tool calls, retrying"
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
                            "User requested action but LLM responded without tool calls (streaming) — nudging retry"
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
                    "Empty response from LLM (streaming) — guard activated",
                    &accumulated_text,
                );
                final_response = text.clone();

                signal_response_complete(&stream_tx).await;

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
                        streaming: true,
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
                        experiment_context,
                        directives: reply_directives_from_parsed(parsed_directives_s),
                        new_messages_start,
                        owner_notice: pending_owner_notice.take(),
                    },
                )
                .await;
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                // Capture text from this tool_use turn (streaming path) — the
                // streaming sink already forwards the deltas to the channel,
                // but the in-memory accumulator is what feeds the empty-text
                // fallback in finalize_end_turn_text. Mirrors the sync path.
                //
                // IMPORTANT (streaming-already-emitted semantics): every byte
                // pushed into `accumulated_text` here has *already been
                // delivered to the client* via the streaming sink. The
                // accumulator is a **post-stream** fallback, not a re-emit:
                //   * On final EndTurn with non-empty text the live deltas
                //     drove the UI, and `final_response` is only used for
                //     session persistence + memory extraction.
                //   * On final EndTurn with empty text, finalize_end_turn_text
                //     returns `accumulated_text` as `final_response`, but the
                //     stream has already drained — no re-push to `stream_tx`
                //     happens (see signal_response_complete is fire-only).
                //   * The bridge.rs streaming success path
                //     (channel_bridge.rs ~3032 `Ok(())` arm) calls only
                //     `record_delivery` + lifecycle reaction; it never invokes
                //     `send_response` with the buffered text. Fallback to
                //     `send_response(buffered_text)` only fires on the
                //     `Err(stream_error)` adapter-failure arm — that is the
                //     intended recovery path, not a duplicate display.
                //
                // So the surface-level concern of "double display" does not
                // manifest with the current bridge wiring. Any future
                // refactor that has the streaming success arm also
                // re-emit `final_response` MUST either drop the
                // accumulated_text fallback in finalize_end_turn_text or
                // gate it on a `streaming_already_emitted: bool` flag.
                //
                // Buffer is capped at ACCUMULATED_TEXT_MAX_BYTES — see
                // push_accumulated_text.
                let intermediate_text = response.text();
                if !intermediate_text.trim().is_empty() {
                    push_accumulated_text(&mut accumulated_text, intermediate_text.trim());
                }

                // See non-streaming branch above for the full rationale
                // — this is the streaming twin of the #2381 staged-commit
                // fix.
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
                        streaming: true,
                        agent_id_str: agent_id_str.as_str(),
                        opts,
                        interrupt: opts.interrupt.clone(),
                        dangerous_command_checker: Some(&session_checker),
                    };
                    let executed = execute_single_tool_call(&mut tool_exec_ctx, tool_call).await?;

                    // §A — capture owner_notice from notify_owner tool and
                    // surface it on the live SSE stream so the gateway can
                    // route it to OWNER_JID without waiting for turn end.
                    if let Some(ref notice) = executed.result.owner_notice {
                        pending_owner_notice = Some(match pending_owner_notice.take() {
                            Some(prev) => format!("{prev}\n\n{notice}"),
                            None => notice.clone(),
                        });
                        if stream_tx
                            .send(StreamEvent::OwnerNotice {
                                text: notice.clone(),
                            })
                            .await
                            .is_err()
                        {
                            warn!(agent = %manifest.name, "Stream consumer disconnected during owner_notice emit");
                        }
                    }

                    // Capture lazy-load side-channel (issue #3044) — the
                    // streaming path, same rationale as the non-streaming
                    // version above.
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

                    // Notify client of tool execution result (detect dead consumer)
                    let preview: String = budgeted_content.chars().take(300).collect();
                    if stream_tx
                        .send(StreamEvent::ToolExecutionResult {
                            name: tool_call.name.clone(),
                            result_preview: preview,
                            is_error: executed.result.is_error,
                        })
                        .await
                        .is_err()
                    {
                        warn!(agent = %manifest.name, "Stream consumer disconnected — continuing tool loop but will not stream further");
                    }

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
                    // Issue #2381: stub the remaining tool_calls so every tool_call_id
                    // has a matching tool_result. See the non-streaming branch above for
                    // the full explanation of why this matters.
                    let is_soft_error = executed.result.status.is_soft_error()
                        || is_soft_error_content(&executed.result.content);
                    if executed.result.is_error && !is_soft_error {
                        warn!(
                            tool = %tool_call.name,
                            "Tool execution failed — skipping remaining tool calls (streaming)"
                        );
                        append_skipped_tool_results(
                            &mut staged.tool_result_blocks,
                            &response.tool_calls[call_idx + 1..],
                            "previous tool call in the same batch failed with a hard error",
                        );
                        break;
                    }

                    // Mid-turn message injection (#956): check for
                    // pending user messages between tool calls (streaming
                    // variant).
                    if let Some(flushed_outcomes) = handle_mid_turn_signal(
                        pending_messages,
                        &manifest.name,
                        session,
                        &mut messages,
                        &mut staged,
                    ) {
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

                if !opts.is_fork && !opts.incognito {
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to interim-save session: {e}");
                    }
                }
                // Track consecutive all-failed iterations to cap wasted retries.
                // (soft errors — approval denials, sandbox rejections, truncation —
                //  do NOT count; the LLM is expected to recover from those cheaply.)
                // NOTE: keep in sync with run_agent_loop (non-streaming).
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
                        "Tool failures in {MAX_CONSECUTIVE_ALL_FAILED} consecutive iterations — exiting streaming loop"
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
                // See non-streaming branch above — same logic for #2286.
                let pure_text_overflow = response.tool_calls.is_empty();
                if pure_text_overflow || consecutive_max_tokens >= MAX_CONTINUATIONS {
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
                            "Max tokens hit on pure-text response (streaming) — returning partial (no tool calls to continue)"
                        );
                    } else {
                        warn!(
                            iteration,
                            consecutive_max_tokens,
                            "Max continuations reached (streaming), returning partial response"
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
                    signal_response_complete(&stream_tx).await;
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
                    });
                }
                let text = response.text();
                session.push_message(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.push_message(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit (streaming), continuing");
            }
            StopReason::ContentFiltered => {
                // Streaming twin of the non-streaming refusal handler (#3450).
                let text = response.text();
                let partial = if text.trim().is_empty() {
                    "[content filtered by provider]".to_string()
                } else {
                    text
                };
                warn!(
                    agent = %manifest.name,
                    iteration,
                    "LLM response blocked by provider safety / content filter (streaming)"
                );
                session.push_message(Message::assistant(&partial));
                if !opts.is_fork && !opts.incognito {
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to save session on content filter: {e}");
                    }
                }
                signal_response_complete(&stream_tx).await;
                return Err(LibreFangError::ContentFiltered { message: partial });
            }
        }
    }

    repair_session_before_save(session, agent_id_str.as_str(), "streaming_max_iterations");
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
