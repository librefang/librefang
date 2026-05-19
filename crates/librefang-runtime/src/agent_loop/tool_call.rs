//! Tool-call execution path: staging the LLM's `tool_use` blocks,
//! per-tool dispatch with timeout / interrupt / approval handling,
//! consecutive-failure tracking, and the post-execution `ToolResult`
//! synthesis (including the post-approval re-resolution signal).

use super::*;

pub(super) fn tool_use_blocks_from_calls(tool_calls: &[ToolCall]) -> Vec<ContentBlock> {
    tool_calls
        .iter()
        .map(|tc| ContentBlock::ToolUse {
            id: tc.id.clone(),
            name: tc.name.clone(),
            input: tc.input.clone(),
            provider_metadata: None,
        })
        .collect()
}

/// Sanitize a tool name into a bounded, low-cardinality metric label.
///
/// Strips control chars and caps the length so an LLM that hallucinates
/// a wild tool name can't blow up the metric registry. The set of real
/// tool names is bounded (builtins + skill tools + MCP tools), so this
/// label dimension stays tractable in steady state.
pub(super) fn sanitize_tool_label(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// Record a tool-call outcome for observability (#3495). `outcome` is
/// one of `"success"` / `"failure"`; we never push raw error text into
/// metric labels.
pub(super) fn record_tool_call_metric(tool_name: &str, is_error: bool) {
    let outcome = if is_error { "failure" } else { "success" };
    metrics::counter!(
        "librefang_tool_call_total",
        "tool" => sanitize_tool_label(tool_name),
        "outcome" => outcome,
    )
    .increment(1);
}

pub(super) fn append_tool_result_guidance_blocks(tool_result_blocks: &mut Vec<ContentBlock>) {
    let denial_count = tool_result_blocks
        .iter()
        .filter(|b| {
            matches!(b, ContentBlock::ToolResult { status, .. }
            if *status == librefang_types::tool::ToolExecutionStatus::Denied)
        })
        .count();
    if denial_count > 0 {
        tool_result_blocks.push(ContentBlock::Text {
            text: format!(
                "[System: {} tool call(s) were denied by approval policy. \
                 Do NOT retry denied tools. Explain to the user what you \
                 wanted to do and that it requires their approval.]",
                denial_count
            ),
            provider_metadata: None,
        });
    }

    let modify_count = tool_result_blocks
        .iter()
        .filter(|b| {
            matches!(b, ContentBlock::ToolResult { status, .. }
            if *status == librefang_types::tool::ToolExecutionStatus::ModifyAndRetry)
        })
        .count();
    if modify_count > 0 {
        tool_result_blocks.push(ContentBlock::Text {
            text: format!(
                "[System: {} tool call(s) received human feedback requesting modification. \
                 Read the feedback carefully, revise your approach, and retry with a \
                 different strategy. Do NOT repeat the exact same tool call.]",
                modify_count
            ),
            provider_metadata: None,
        });
    }

    let error_count = tool_result_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
        .count();
    let non_denial_errors = error_count.saturating_sub(denial_count);
    // Separate parameter errors (LLM can self-correct by retrying with valid args)
    // from execution errors (network/IO/permission failures the LLM cannot fix).
    let param_error_count = tool_result_blocks
        .iter()
        .filter(|b| match b {
            ContentBlock::ToolResult {
                is_error: true,
                content,
                ..
            } => is_parameter_error_content(content),
            _ => false,
        })
        .count();
    let non_param_errors = non_denial_errors.saturating_sub(param_error_count);
    if param_error_count > 0 {
        tool_result_blocks.push(ContentBlock::Text {
            text: format!(
                "[System: {} tool call(s) failed due to missing or invalid parameters. \
                 Read the error message, correct your tool call arguments, and retry \
                 immediately. Do NOT ask the user for help — fix the parameters yourself.]",
                param_error_count
            ),
            provider_metadata: None,
        });
    }
    if non_param_errors > 0 {
        tool_result_blocks.push(ContentBlock::Text {
            text: format!(
                "[System: {} tool(s) returned errors. Report the error honestly \
                 to the user. Do NOT fabricate results or pretend the tool succeeded. \
                 If a search or fetch failed, tell the user it failed and suggest \
                 alternatives instead of making up data.]",
                non_param_errors
            ),
            provider_metadata: None,
        });
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ToolResultOutcomeSummary {
    pub(super) hard_error_count: u32,
    pub(super) success_count: u32,
}

impl ToolResultOutcomeSummary {
    pub(super) fn from_blocks(tool_result_blocks: &[ContentBlock]) -> Self {
        let mut summary = Self::default();
        for block in tool_result_blocks {
            match block {
                ContentBlock::ToolResult {
                    status,
                    content,
                    is_error: true,
                    ..
                } if !status.is_soft_error() && !is_soft_error_content(content) => {
                    summary.hard_error_count += 1;
                }
                ContentBlock::ToolResult {
                    is_error: false, ..
                } => {
                    summary.success_count += 1;
                }
                _ => {}
            }
        }

        summary
    }

    pub(super) fn accumulate(&mut self, other: Self) {
        self.hard_error_count += other.hard_error_count;
        self.success_count += other.success_count;
    }
}

pub(super) fn update_consecutive_hard_failures(
    consecutive_all_failed: &mut u32,
    outcome_summary: ToolResultOutcomeSummary,
) -> u32 {
    let hard_error_count = outcome_summary.hard_error_count;
    let success_count = outcome_summary.success_count;

    if success_count == 0 && hard_error_count > 0 {
        *consecutive_all_failed += 1;
    } else {
        *consecutive_all_failed = 0;
    }

    hard_error_count
}

/// Accumulates an in-flight tool-use turn without touching `session.messages`
/// or the LLM working-copy `messages` vec until the turn is ready to commit.
///
/// This is the structural fix for upstream #2381: the previous
/// `begin_tool_use_turn` helper eagerly pushed the assistant `tool_use`
/// message to `session.messages` BEFORE any tool executed, and relied on
/// a later `finalize_tool_use_results` call to add the paired user
/// `tool_result` message. Any control-flow exit between the two (a hard
/// error `break`, a mid-turn signal `break`, or a `?` propagation from
/// `execute_single_tool_call`) left `session.messages` in a
/// half-committed state: the provider then rejected the next request
/// with "tool_call_ids did not have response messages" (HTTP 400).
///
/// With `StagedToolUseTurn` the assistant message AND all tool-result
/// blocks are buffered locally. Only `commit` touches the persisted
/// vectors, and it does so atomically (assistant message + user
/// {tool_results} pushed in a single operation). If the staged turn is
/// dropped without commit — which is exactly what `?` propagation does —
/// `session.messages` is untouched. By construction, no orphan `ToolUse`
/// can ever reach the persistence layer.
pub(super) struct StagedToolUseTurn {
    /// The assistant message carrying `ContentBlock::ToolUse` blocks.
    /// Cloned into both `session.messages` and the LLM `messages`
    /// working copy at commit time.
    pub(super) assistant_msg: Message,
    /// `(tool_use_id, tool_name)` for every tool_use block the LLM
    /// emitted. Used by `pad_missing_results` to fabricate synthetic
    /// "not executed" results for any tool_use_id that never received
    /// an `append_result` (e.g. because a mid-turn signal interrupted
    /// the per-tool loop).
    pub(super) tool_call_ids: Vec<(String, String)>,
    /// Accumulated `ContentBlock::ToolResult` blocks. Committed as the
    /// body of a single user message once the turn is ready.
    pub(super) tool_result_blocks: Vec<ContentBlock>,
    /// Cached assistant rationale text (if any) — preserved here so
    /// the tool-execution loop can pass it to `execute_single_tool_call`
    /// for decision trace recording.
    pub(super) rationale_text: Option<String>,
    /// Names of tools the agent is allowed to invoke on this turn.
    pub(super) allowed_tool_names: Vec<String>,
    /// Caller id (agent id as string) used for hook context and policy.
    pub(super) caller_id_str: String,
    /// Once `commit` runs this flips to true so a second commit call
    /// (or a drop-after-commit) is a no-op.
    pub(super) committed: bool,
    /// Layer 2 per-result spill threshold (bytes). Taken from
    /// `LoopOptions::tool_results_config` at construction time.
    pub(super) per_result_threshold: usize,
    /// Layer 3 per-turn aggregate budget (bytes). Taken from
    /// `LoopOptions::tool_results_config` at construction time.
    pub(super) per_turn_budget: usize,
    /// Per-artifact write cap forwarded into `ToolBudgetEnforcer` so its
    /// underlying `artifact_store::maybe_spill` rejects writes above this
    /// (and the enforcer falls back to inline truncation).  Taken from
    /// `LoopOptions::tool_results_config.max_artifact_bytes`.
    pub(super) max_artifact_bytes: u64,
}

impl StagedToolUseTurn {
    /// Append a tool result block to the staged turn. Called once per
    /// `execute_single_tool_call` completion — including for hard
    /// errors, which are honest information the LLM must see on the
    /// next iteration.
    pub(super) fn append_result(&mut self, block: ContentBlock) {
        self.tool_result_blocks.push(block);
    }

    /// Pad any `tool_use_id` that never had `append_result` called for
    /// it with a synthetic "tool not executed" result block. No-op on
    /// the happy path where every tool executed (and therefore appended
    /// a result — including a real error result).
    ///
    /// This is ONLY for ids that have no result at all. If a tool
    /// returned `is_error=true` via `append_result`, that real error
    /// content is preserved — padding must NOT paper over honest error
    /// information.
    pub(super) fn pad_missing_results(&mut self) {
        for (id, name) in &self.tool_call_ids {
            let already_present = self.tool_result_blocks.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id
                )
            });
            if already_present {
                continue;
            }
            self.tool_result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                tool_name: name.clone(),
                content: "[tool interrupted: turn aborted before this call could execute]"
                    .to_string(),
                is_error: true,
                status: librefang_types::tool::ToolExecutionStatus::Error,
                approval_request_id: None,
            });
        }
    }

    /// Atomically commit the staged assistant message and user
    /// tool-result message to both `session.messages` and the LLM
    /// working copy `messages`. Returns the outcome summary computed
    /// from the accumulated tool-result blocks (for consecutive-failure
    /// tracking).
    ///
    /// Callers should always run `pad_missing_results` before `commit`
    /// if any control-flow exit (mid-turn signal, etc.) interrupted the
    /// per-tool loop — otherwise the wire format will carry orphan
    /// `tool_use_id`s the provider will reject.
    pub(super) fn commit(
        &mut self,
        session: &mut Session,
        messages: &mut Vec<Message>,
    ) -> ToolResultOutcomeSummary {
        if self.committed {
            return ToolResultOutcomeSummary::default();
        }
        self.committed = true;

        // Step 1: push the assistant message carrying the tool_use blocks.
        session.push_message(self.assistant_msg.clone());
        messages.push(self.assistant_msg.clone());

        // Step 2: degenerate-case short-circuit — if no result blocks
        // were accumulated (LLM emitted no tool_calls, or every id was
        // padded away) we skip the paired user message so we don't emit
        // an empty `Blocks(vec![])` message.
        if self.tool_result_blocks.is_empty() {
            return ToolResultOutcomeSummary::default();
        }

        // Step 3: delegate the user{tool_result} push to the existing
        // `finalize_tool_use_results` helper so guidance-block append
        // behaviour stays centralized.
        finalize_tool_use_results(
            session,
            messages,
            &mut self.tool_result_blocks,
            self.per_result_threshold,
            self.per_turn_budget,
            self.max_artifact_bytes,
        )
    }
}

/// Build a `StagedToolUseTurn` for an assistant response whose stop
/// reason is `ToolUse`. Does NOT mutate `session.messages` or the LLM
/// working copy — see `StagedToolUseTurn` docs for why.
pub(super) fn stage_tool_use_turn(
    response: &crate::llm_driver::CompletionResponse,
    session: &Session,
    available_tools: &[ToolDefinition],
    per_result_threshold: usize,
    per_turn_budget: usize,
    max_artifact_bytes: u64,
) -> StagedToolUseTurn {
    let rationale_text = {
        let text = response.text();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    };

    let assistant_msg = Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(response.content.clone()),
        pinned: false,
        timestamp: Some(chrono::Utc::now()),
    };

    let tool_call_ids: Vec<(String, String)> = response
        .tool_calls
        .iter()
        .map(|tc| (tc.id.clone(), tc.name.clone()))
        .collect();

    StagedToolUseTurn {
        assistant_msg,
        tool_call_ids,
        tool_result_blocks: Vec::new(),
        rationale_text,
        allowed_tool_names: available_tools.iter().map(|t| t.name.clone()).collect(),
        caller_id_str: session.agent_id.to_string(),
        committed: false,
        per_result_threshold,
        per_turn_budget,
        max_artifact_bytes,
    }
}

pub(super) struct ExecutedToolCall {
    pub(super) result: librefang_types::tool::ToolResult,
    pub(super) final_content: String,
}

pub(super) struct ToolExecutionContext<'a> {
    pub(super) manifest: &'a AgentManifest,
    pub(super) loop_guard: &'a mut LoopGuard,
    pub(super) memory: &'a MemorySubstrate,
    pub(super) session: &'a mut Session,
    pub(super) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(super) available_tool_names: &'a [String],
    /// Full `ToolDefinition` list for the agent's granted tools — needed so
    /// the lazy-load meta-tools (`tool_load`, `tool_search`) can resolve
    /// non-builtin entries (MCP, skills) against the agent's actual pool
    /// rather than only the builtin catalog (issue #3044 follow-up).
    pub(super) available_tools: &'a [ToolDefinition],
    pub(super) caller_id_str: &'a str,
    pub(super) skill_registry: Option<&'a SkillRegistry>,
    pub(super) allowed_skills: &'a [String],
    pub(super) mcp_connections: Option<&'a tokio::sync::Mutex<Vec<McpConnection>>>,
    pub(super) web_ctx: Option<&'a WebToolsContext>,
    pub(super) browser_ctx: Option<&'a crate::browser::BrowserManager>,
    pub(super) hand_allowed_env: &'a [String],
    pub(super) workspace_root: Option<&'a Path>,
    pub(super) media_engine: Option<&'a crate::media_understanding::MediaEngine>,
    pub(super) media_drivers: Option<&'a crate::media::MediaDriverCache>,
    pub(super) tts_engine: Option<&'a crate::tts::TtsEngine>,
    pub(super) docker_config: Option<&'a librefang_types::config::DockerSandboxConfig>,
    pub(super) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(super) process_manager: Option<&'a crate::process_manager::ProcessManager>,
    pub(super) process_registry: Option<&'a crate::process_registry::ProcessRegistry>,
    pub(super) sender_user_id: Option<&'a str>,
    pub(super) sender_channel: Option<&'a str>,
    pub(super) checkpoint_manager: Option<&'a Arc<CheckpointManager>>,
    pub(super) context_budget: &'a ContextBudget,
    pub(super) context_engine: Option<&'a dyn ContextEngine>,
    pub(super) context_window_tokens: usize,
    pub(super) on_phase: Option<&'a PhaseCallback>,
    pub(super) decision_traces: &'a mut Vec<DecisionTrace>,
    pub(super) rationale_text: &'a Option<String>,
    pub(super) tools_recovered_from_text: bool,
    pub(super) iteration: u32,
    pub(super) streaming: bool,
    pub(super) agent_id_str: &'a str,
    pub(super) opts: &'a LoopOptions,
    /// Per-session interrupt handle propagated into tool execution so that
    /// long-running tools (shell_exec, agent_send, …) can observe a /stop
    /// signal without polling a global flag.
    pub(super) interrupt: Option<crate::interrupt::SessionInterrupt>,
    pub(super) dangerous_command_checker:
        Option<&'a Arc<tokio::sync::RwLock<crate::dangerous_command::DangerousCommandChecker>>>,
}

#[instrument(
    skip_all,
    fields(
        tool.name = %tool_call.name,
        tool.id = %tool_call.id,
    ),
)]
/// Thin wrapper around `execute_single_tool_call_inner` that guarantees
/// `record_tool_call_metric` is called on **every** return path — both `Ok`
/// (success or is_error tool result) and `Err` (e.g. circuit-break).
pub(super) async fn execute_single_tool_call(
    ctx: &mut ToolExecutionContext<'_>,
    tool_call: &ToolCall,
) -> Result<ExecutedToolCall, LibreFangError> {
    let result = execute_single_tool_call_inner(ctx, tool_call).await;
    match &result {
        Ok(executed) => record_tool_call_metric(&tool_call.name, executed.result.is_error),
        Err(_) => record_tool_call_metric(&tool_call.name, true),
    }
    result
}

pub(super) async fn execute_single_tool_call_inner(
    ctx: &mut ToolExecutionContext<'_>,
    tool_call: &ToolCall,
) -> Result<ExecutedToolCall, LibreFangError> {
    let verdict = ctx.loop_guard.check(&tool_call.name, &tool_call.input);
    match &verdict {
        LoopGuardVerdict::CircuitBreak(msg) => {
            if ctx.streaming {
                warn!(tool = %tool_call.name, "Circuit breaker triggered (streaming)");
            } else {
                warn!(tool = %tool_call.name, "Circuit breaker triggered");
            }
            repair_session_before_save(ctx.session, ctx.agent_id_str, "circuit_breaker");
            if !ctx.opts.is_fork && !ctx.opts.incognito {
                if let Err(e) = ctx.memory.save_session_async(ctx.session).await {
                    warn!("Failed to save session on circuit break: {e}");
                }
            }
            let hook_ctx = crate::hooks::HookContext {
                agent_name: &ctx.manifest.name,
                agent_id: ctx.agent_id_str,
                event: librefang_types::agent::HookEvent::AgentLoopEnd,
                data: serde_json::json!({
                    "reason": "circuit_break",
                    "error": msg.as_str(),
                    "is_fork": ctx.opts.is_fork,
                }),
            };
            fire_hook_best_effort(ctx.hooks, &hook_ctx);
            return Err(LibreFangError::Internal(msg.clone()));
        }
        LoopGuardVerdict::Block(msg) => {
            if ctx.streaming {
                warn!(tool = %tool_call.name, "Tool call blocked by loop guard (streaming)");
            } else {
                warn!(tool = %tool_call.name, "Tool call blocked by loop guard");
            }
            return Ok(ExecutedToolCall {
                result: librefang_types::tool::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    content: msg.clone(),
                    is_error: true,
                    status: librefang_types::tool::ToolExecutionStatus::Error,
                    ..Default::default()
                },
                final_content: msg.clone(),
            });
        }
        _ => {}
    }

    // Fork-mode runtime tool allowlist (from LoopOptions::allowed_tools).
    // The request schema wasn't filtered — that would break Anthropic prompt
    // cache alignment — so the model may try any tool in its manifest. We
    // reject non-allowed calls here with a synthetic error result so the
    // model can see the rejection and adapt. Defense-in-depth for derivative
    // calls like auto-dream, which only want `memory_*` but share the full
    // tool prefix with the parent turn for cache alignment.
    if let Some(allow) = ctx.opts.allowed_tools.as_ref() {
        if !allow.iter().any(|t| t == &tool_call.name) {
            let msg = format!(
                "Tool `{}` is not permitted in this fork invocation. Allowed: {}",
                tool_call.name,
                allow.join(", ")
            );
            warn!(
                tool = %tool_call.name,
                is_fork = ctx.opts.is_fork,
                "Tool call outside fork allowlist — denied"
            );
            return Ok(ExecutedToolCall {
                result: librefang_types::tool::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    content: msg.clone(),
                    is_error: true,
                    status: librefang_types::tool::ToolExecutionStatus::Error,
                    ..Default::default()
                },
                final_content: msg,
            });
        }
    }

    // Incognito mode: silently drop memory_store calls so the LLM's perception
    // of a successful write is preserved (it gets an ok response) but nothing
    // is committed to the proactive memory store. Memory reads remain
    // full-access per #4073 spec.
    if ctx.opts.incognito && tool_call.name == "memory_store" {
        tracing::debug!(target: "incognito", tool = "memory_store", "memory_store call dropped during incognito turn");
        return Ok(ExecutedToolCall {
            result: librefang_types::tool::ToolResult {
                tool_use_id: tool_call.id.clone(),
                content: "ok".to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                ..Default::default()
            },
            final_content: "ok".to_string(),
        });
    }

    if ctx.streaming {
        debug!(tool = %tool_call.name, id = %tool_call.id, "Executing tool (streaming)");
    } else {
        debug!(tool = %tool_call.name, id = %tool_call.id, "Executing tool");
    }

    if let Some(cb) = ctx.on_phase {
        let sanitized: String = tool_call
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        cb(LoopPhase::ToolUse {
            tool_name: sanitized,
        });
    }

    if let Some(hook_reg) = ctx.hooks {
        let hook_ctx = crate::hooks::HookContext {
            agent_name: &ctx.manifest.name,
            agent_id: ctx.caller_id_str,
            event: librefang_types::agent::HookEvent::BeforeToolCall,
            data: serde_json::json!({
                "tool_name": &tool_call.name,
                "input": &tool_call.input,
            }),
        };
        if let Err(reason) = hook_reg.fire(&hook_ctx) {
            let content = format!("Hook blocked tool '{}': {}", tool_call.name, reason);
            return Ok(ExecutedToolCall {
                result: librefang_types::tool::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    content: content.clone(),
                    is_error: true,
                    status: librefang_types::tool::ToolExecutionStatus::Error,
                    ..Default::default()
                },
                final_content: content,
            });
        }
    }

    let effective_exec_policy = ctx.manifest.exec_policy.as_ref();
    let tool_timeout = ctx.kernel.as_ref().map_or(TOOL_TIMEOUT_SECS, |k| {
        k.tool_timeout_secs_for(&tool_call.name)
    });
    let trace_start = Instant::now();
    let trace_timestamp = chrono::Utc::now();
    let result = match tokio::time::timeout(
        Duration::from_secs(tool_timeout),
        tool_runner::execute_tool(
            &tool_call.id,
            &tool_call.name,
            &tool_call.input,
            ctx.kernel,
            Some(ctx.available_tool_names),
            Some(ctx.caller_id_str),
            ctx.skill_registry,
            Some(ctx.allowed_skills),
            ctx.mcp_connections,
            ctx.web_ctx,
            ctx.browser_ctx,
            if ctx.hand_allowed_env.is_empty() {
                None
            } else {
                Some(ctx.hand_allowed_env)
            },
            ctx.workspace_root,
            ctx.media_engine,
            ctx.media_drivers,
            effective_exec_policy,
            ctx.tts_engine,
            ctx.docker_config,
            ctx.process_manager,
            ctx.process_registry,
            ctx.sender_user_id,
            ctx.sender_channel,
            ctx.checkpoint_manager,
            ctx.interrupt.clone(),
            Some(ctx.session.id.to_string()).as_deref(),
            ctx.dangerous_command_checker,
            Some(ctx.available_tools),
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            if ctx.streaming {
                warn!(tool = %tool_call.name, "Tool execution timed out after {}s (streaming)", tool_timeout);
            } else {
                warn!(tool = %tool_call.name, "Tool execution timed out after {}s", tool_timeout);
            }
            librefang_types::tool::ToolResult {
                tool_use_id: tool_call.id.clone(),
                content: format!(
                    "Tool '{}' timed out after {}s.",
                    tool_call.name, tool_timeout
                ),
                is_error: true,
                status: librefang_types::tool::ToolExecutionStatus::Expired,
                ..Default::default()
            }
        }
    };
    let execution_ms = trace_start.elapsed().as_millis() as u64;

    let output_summary = librefang_types::truncate_str(&result.content, 200).to_string();
    ctx.decision_traces.push(DecisionTrace {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        input: tool_call.input.clone(),
        rationale: ctx.rationale_text.clone(),
        recovered_from_text: ctx.tools_recovered_from_text,
        execution_ms,
        is_error: result.is_error,
        output_summary,
        iteration: ctx.iteration,
        timestamp: trace_timestamp,
    });

    let hook_ctx = crate::hooks::HookContext {
        agent_name: &ctx.manifest.name,
        agent_id: ctx.caller_id_str,
        event: librefang_types::agent::HookEvent::AfterToolCall,
        data: serde_json::json!({
            "tool_name": &tool_call.name,
            "result": &result.content,
            "is_error": result.is_error,
        }),
    };
    fire_hook_best_effort(ctx.hooks, &hook_ctx);

    // Allow plugins to rewrite the tool result before it enters the conversation context.
    let result_content = if let Some(hook_reg) = ctx.hooks {
        let transform_ctx = crate::hooks::HookContext {
            agent_name: &ctx.manifest.name,
            agent_id: ctx.caller_id_str,
            event: librefang_types::agent::HookEvent::TransformToolResult,
            data: serde_json::json!({
                "tool_name": &tool_call.name,
                "args": &tool_call.input,
                "result": &result.content,
                "is_error": result.is_error,
            }),
        };
        hook_reg
            .fire_transform(&transform_ctx)
            .unwrap_or_else(|| result.content.clone())
    } else {
        result.content.clone()
    };

    // Spill the full raw result to the artifact store BEFORE
    // `sanitize_tool_result_content` truncates it. Web tools spill at
    // execution time, but MCP (and any other) results arrive un-spilled, so
    // without this they would be destructively truncated and the original
    // bytes lost — the LLM would get clipped text with no `read_artifact`
    // reference. A web stub is already < threshold, so this is a no-op pass
    // through for it (no double-spill).
    let result_content = {
        let cfg = ctx.opts.tool_results_config.clone().unwrap_or_default();
        let (threshold, max_artifact) = crate::tool_runner::resolve_spill_config(
            cfg.spill_threshold_bytes,
            cfg.max_artifact_bytes,
        );
        match crate::artifact_store::maybe_spill(
            &tool_call.name,
            result_content.as_bytes(),
            threshold,
            max_artifact,
            &crate::artifact_store::default_artifact_storage_dir(),
        ) {
            Some(stub) => stub,
            None => result_content,
        }
    };

    let content = sanitize_tool_result_content(
        &result_content,
        ctx.context_budget,
        ctx.context_engine,
        ctx.context_window_tokens,
    );
    let final_content = if let LoopGuardVerdict::Warn(ref warn_msg) = verdict {
        format!("{content}\n\n[LOOP GUARD] {warn_msg}")
    } else {
        content
    };

    Ok(ExecutedToolCall {
        result,
        final_content,
    })
}

/// Emit stub `ToolResult` blocks for any tool calls in `remaining` that
/// were not actually executed (e.g. because we hit a hard error and broke
/// out of the per-call loop). OpenAI/Anthropic both require **every**
/// `tool_call_id` in an assistant message to be answered by a matching
/// tool_result on the next turn — without these stubs the next API call
/// fails with `tool_call_ids ... did not have response messages` and
/// the agent gets bricked. Issue #2381.
pub(super) fn append_skipped_tool_results(
    tool_result_blocks: &mut Vec<ContentBlock>,
    remaining: &[ToolCall],
    reason: &str,
) {
    for tc in remaining {
        tool_result_blocks.push(ContentBlock::ToolResult {
            tool_use_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            content: format!("Skipped: {reason}"),
            is_error: true,
            status: librefang_types::tool::ToolExecutionStatus::Skipped,
            approval_request_id: None,
        });
    }
}
pub(super) fn handle_mid_turn_signal(
    pending_messages: Option<&tokio::sync::Mutex<mpsc::Receiver<AgentLoopSignal>>>,
    manifest_name: &str,
    session: &mut Session,
    messages: &mut Vec<Message>,
    staged: &mut StagedToolUseTurn,
) -> Option<ToolResultOutcomeSummary> {
    let pending_rx = pending_messages?;
    let Ok(mut rx) = pending_rx.try_lock() else {
        return None;
    };
    let Ok(signal) = rx.try_recv() else {
        return None;
    };

    // For approval-resolution signals, decide ownership BEFORE touching
    // the staged turn. The kernel's `notify_agent_of_resolution` fans
    // every resolved approval to all live sessions of the agent (because
    // `DeferredToolExecution` does not carry a session id). If we run
    // `pad_missing_results` + `commit` first and only check ownership
    // after, every unrelated session gets its in-progress `tool_use`
    // padded to `is_error=true` and persisted — the staged-pollution
    // bug acknowledged in PR #4091's follow-up commit but not fixed
    // there.
    //
    // Strategy: peek the signal's `tool_use_id` against this session's
    // pending approvals (in `staged.tool_result_blocks`, in
    // `session.messages`, and in the in-flight `messages` slice). If
    // none of them carry the id with `WaitingApproval` status, the
    // signal is for a sibling session — drop it silently without
    // committing or padding. Drop is correct: sibling sessions consume
    // their own copies of the same broadcast.
    if let AgentLoopSignal::ApprovalResolved { tool_use_id, .. } = &signal {
        if !session_owns_pending_approval(session, messages, staged, tool_use_id) {
            debug!(
                agent = %manifest_name,
                tool_use_id = %tool_use_id,
                "Ignoring broadcast approval resolution for tool_use_id not owned by this session"
            );
            return None;
        }
    }

    // Pad any tool_use_id that never produced a result, then commit the
    // staged assistant message + user{tool_results} atomically. After
    // this call, session.messages is guaranteed to have paired
    // ToolUse+ToolResult blocks — no orphan tool_use_id can leak onto
    // the wire (#2381).
    staged.pad_missing_results();
    let flushed_outcomes = staged.commit(session, messages);

    info!(
        agent = %manifest_name,
        "Mid-turn signal injected — interrupting tool execution"
    );
    let injected_text = match signal {
        AgentLoopSignal::Message { content } => Some(content),
        AgentLoopSignal::ApprovalResolved {
            tool_use_id,
            tool_name,
            decision,
            result_content,
            result_is_error,
            result_status,
        } => {
            // Ownership was verified above. `apply_approval_resolution_signal`
            // is guaranteed to find the matching WaitingApproval block
            // (either in committed history or in just-committed staged
            // results) and patch it in place. If for any reason it
            // doesn't (race between fork/reset and resolution arrival),
            // suppress the `[System]` text — same behaviour as the
            // upstream PR #4091 fix.
            let matched = apply_approval_resolution_signal(
                session,
                messages.as_mut_slice(),
                &tool_use_id,
                &result_content,
                result_is_error,
                result_status,
            );
            if matched {
                let result_preview = librefang_types::truncate_str(&result_content, 300);
                Some(format!(
                    "[System] Tool '{}' approval resolved ({}). Result: {}",
                    tool_name, decision, result_preview
                ))
            } else {
                // Ownership was verified before pad/commit; reaching
                // here means the WaitingApproval block disappeared
                // between the peek and the patch (fork/reset race).
                debug!(
                    agent = %manifest_name,
                    tool_use_id = %tool_use_id,
                    "Approval resolution arrived after WaitingApproval block disappeared (fork/reset race?)"
                );
                None
            }
        }
        AgentLoopSignal::TaskCompleted { event } => {
            // Async task tracker (#4983). Step 2 wires the kernel-side
            // registration and injection; the runtime currently surfaces the
            // completion as a `[System] [ASYNC_RESULT] …` message that the
            // LLM can read on the same turn (mid-turn) or on the next turn
            // (idle). Step 3 adds typed handling, `[async_tasks]` config,
            // and the wake-idle path.
            Some(super::format_task_completion_text(&event))
        }
    };
    if let Some(text) = injected_text {
        let inject_msg = Message::user(&text);
        session.push_message(inject_msg.clone());
        messages.push(inject_msg);
    }
    Some(flushed_outcomes)
}

/// Returns `true` if `tool_use_id` has a `WaitingApproval` `ToolResult`
/// block in any of: the staged-but-not-yet-committed turn, the session's
/// committed history, or the in-flight `messages` slice the LLM driver
/// will see next.
///
/// Used by `handle_mid_turn_signal` to decide whether an
/// `ApprovalResolved` broadcast is meant for THIS session before
/// touching staged state. Without this check, a broadcast intended for a
/// sibling session would force `pad_missing_results` + `commit` here,
/// poisoning every unrelated session's in-progress `tool_use` with
/// `is_error=true` — the residual injection_senders pollution
/// acknowledged in PR #4091's follow-up commit (591ad4ec) and fixed
/// here.
pub(super) fn session_owns_pending_approval(
    session: &Session,
    messages: &[Message],
    staged: &StagedToolUseTurn,
    tool_use_id: &str,
) -> bool {
    fn block_is_waiting_for(block: &ContentBlock, target: &str) -> bool {
        matches!(
            block,
            ContentBlock::ToolResult { tool_use_id: id, status, .. }
                if id == target
                    && *status == librefang_types::tool::ToolExecutionStatus::WaitingApproval
        )
    }

    fn message_carries_waiting(msg: &Message, target: &str) -> bool {
        match &msg.content {
            MessageContent::Blocks(blocks) => {
                blocks.iter().any(|b| block_is_waiting_for(b, target))
            }
            _ => false,
        }
    }

    if staged
        .tool_result_blocks
        .iter()
        .any(|b| block_is_waiting_for(b, tool_use_id))
    {
        return true;
    }
    if session
        .messages
        .iter()
        .any(|m| message_carries_waiting(m, tool_use_id))
    {
        return true;
    }
    if messages
        .iter()
        .any(|m| message_carries_waiting(m, tool_use_id))
    {
        return true;
    }
    false
}

pub(super) fn finalize_tool_use_results(
    session: &mut Session,
    messages: &mut Vec<Message>,
    tool_result_blocks: &mut Vec<ContentBlock>,
    per_result_threshold: usize,
    per_turn_budget: usize,
    max_artifact_bytes: u64,
) -> ToolResultOutcomeSummary {
    if tool_result_blocks.is_empty() {
        return ToolResultOutcomeSummary::default();
    }

    // Compute outcome_summary from the original (pre-budget) content so that
    // is_soft_error_content checks match the actual tool error text, not the
    // [tool_result: ... | sha256:...] replacement that Layer 3 may substitute.
    // This must happen before Layer 3 mutates the blocks.
    let outcome_summary = ToolResultOutcomeSummary::from_blocks(tool_result_blocks);

    // Layer 3: per-turn aggregate budget enforcement (#3347 2/N).
    // Convert ToolResult blocks into ToolResultEntry values, run the enforcer
    // with the configured thresholds, then write back any content that was
    // modified (persisted or truncated).
    {
        let enforcer =
            ToolBudgetEnforcer::new(per_result_threshold, per_turn_budget, max_artifact_bytes);
        let mut entries: Vec<ToolResultEntry> = tool_result_blocks
            .iter()
            .filter_map(|b| {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = b
                {
                    Some(ToolResultEntry {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        enforcer.enforce_turn_budget(&mut entries);

        // Write back potentially-modified content using the same index order
        // (only ToolResult blocks participate; Text guidance blocks are skipped).
        let mut entry_iter = entries.into_iter();
        for block in tool_result_blocks.iter_mut() {
            if let ContentBlock::ToolResult { content, .. } = block {
                if let Some(entry) = entry_iter.next() {
                    *content = entry.content;
                }
            }
        }
    }
    append_tool_result_guidance_blocks(tool_result_blocks);

    // Pin messages containing agent_send results so they survive history trim.
    // Delegation results are authoritative work product that the LLM needs to
    // see to avoid redoing delegated tasks. Cap: only pin if ≤ MAX_PINNED_DELEGATION
    // pinned messages already exist in the session to prevent unbounded growth.
    let has_delegation_result = tool_result_blocks.iter().any(
        |b| matches!(b, ContentBlock::ToolResult { tool_name, .. } if tool_name == "agent_send"),
    );
    const MAX_PINNED_DELEGATION: usize = 10;
    let existing_pinned = session.messages.iter().filter(|m| m.pinned).count();
    // Trust boundary: only internal `agent_send` delegation results are pinned.
    // MCP / external tool output must never be pinned so it cannot be injected
    // as persistent context.  `has_delegation_result` gates on the tool name
    // "agent_send" which is an internal kernel-controlled tool, so external
    // content cannot satisfy the predicate.
    let mut pin_this = has_delegation_result && existing_pinned < MAX_PINNED_DELEGATION;
    // Trust-boundary guard (#6 review-followup hardening): the upstream
    // predicate `has_delegation_result` is itself derived from a
    // `tool_name == "agent_send"` scan at the call site, so under normal
    // control flow we expect the two checks to agree.  If they disagree,
    // a real bug has occurred — `has_delegation_result` was computed
    // against a different `tool_result_blocks` view than the one we hold
    // here.  Crash dev/CI builds via `debug_assert!` so the regression
    // shows up in tests; in release builds we additionally clear
    // `pin_this` and emit `error!` so external content never reaches the
    // pinned set even if the bug shipped.
    let blocks_have_agent_send = tool_result_blocks.iter().any(
        |b| matches!(b, ContentBlock::ToolResult { tool_name, .. } if tool_name == "agent_send"),
    );
    debug_assert!(
        !pin_this || blocks_have_agent_send,
        "pin_this/blocks divergence: has_delegation_result implied agent_send but \
         tool_result_blocks contains none — fix the upstream predicate at the call site"
    );
    if pin_this && !blocks_have_agent_send {
        tracing::error!(
            target: "trust_boundary",
            "refusing to pin tool-result message that contains no agent_send block; \
             resetting pin_this to false to prevent external content injection"
        );
        pin_this = false;
    }

    let tool_results_msg = Message {
        role: Role::User,
        content: MessageContent::Blocks(tool_result_blocks.clone()),
        pinned: pin_this,
        timestamp: Some(chrono::Utc::now()),
    };
    session.push_message(tool_results_msg.clone());
    messages.push(tool_results_msg);

    outcome_summary
}

pub(super) fn apply_approval_resolution_signal(
    session: &mut Session,
    messages: &mut [Message],
    tool_use_id: &str,
    result_content: &str,
    result_is_error: bool,
    result_status: librefang_types::tool::ToolExecutionStatus,
) -> bool {
    fn patch_message_blocks(
        msg: &mut Message,
        tool_use_id: &str,
        result_content: &str,
        result_is_error: bool,
        result_status: librefang_types::tool::ToolExecutionStatus,
    ) -> bool {
        let MessageContent::Blocks(blocks) = &mut msg.content else {
            return false;
        };
        for block in blocks.iter_mut() {
            if let ContentBlock::ToolResult {
                tool_use_id: id,
                content,
                is_error,
                status,
                approval_request_id,
                ..
            } = block
            {
                if id == tool_use_id
                    && *status == librefang_types::tool::ToolExecutionStatus::WaitingApproval
                {
                    *content = result_content.to_string();
                    *is_error = result_is_error;
                    *status = result_status;
                    *approval_request_id = None;
                    return true;
                }
            }
        }
        false
    }

    let mut session_matched = false;
    for msg in session.messages.iter_mut().rev() {
        if patch_message_blocks(
            msg,
            tool_use_id,
            result_content,
            result_is_error,
            result_status,
        ) {
            session_matched = true;
            break;
        }
    }
    if session_matched {
        session.mark_messages_mutated();
    }
    let mut matched = session_matched;
    for msg in messages.iter_mut().rev() {
        if patch_message_blocks(
            msg,
            tool_use_id,
            result_content,
            result_is_error,
            result_status,
        ) {
            matched = true;
            break;
        }
    }
    matched
}
