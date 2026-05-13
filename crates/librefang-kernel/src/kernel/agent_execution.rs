//! Cluster pulled out of mod.rs in #4713 phase 3d.
//!
//! Hosts the per-turn execution dispatch surface: `execute_wasm_agent`,
//! `execute_python_agent`, and the giant `execute_llm_agent` core that
//! resolves model routing, builds prompt context, runs the agent loop,
//! and posts metering / session updates. Plus two small helpers used by
//! `execute_llm_agent`: `should_reuse_cached_route` and
//! `is_brief_acknowledgement`.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;
use crate::MeteringSubsystemApi;

impl LibreFangKernel {
    // -----------------------------------------------------------------------
    // Module dispatch: WASM / Python / LLM
    // -----------------------------------------------------------------------

    /// Execute a WASM module agent.
    ///
    /// Loads the `.wasm` or `.wat` file, maps manifest capabilities into
    /// `SandboxConfig`, and runs through the `WasmSandbox` engine.
    pub(crate) async fn execute_wasm_agent(
        &self,
        entry: &AgentEntry,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
    ) -> KernelResult<AgentLoopResult> {
        let module_path = entry.manifest.module.strip_prefix("wasm:").unwrap_or("");
        let wasm_path = self.resolve_module_path(module_path);

        info!(agent = %entry.name, path = %wasm_path.display(), "Executing WASM agent");

        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Failed to read WASM module '{}': {e}",
                wasm_path.display()
            )))
        })?;

        // Map manifest capabilities to sandbox capabilities
        let caps = manifest_to_capabilities(&entry.manifest);
        let sandbox_config = SandboxConfig {
            fuel_limit: entry.manifest.resources.max_cpu_time_ms * 100_000,
            max_memory_bytes: entry.manifest.resources.max_memory_bytes as usize,
            capabilities: caps,
            timeout_secs: Some(30),
        };

        let input = serde_json::json!({
            "message": message,
            "agent_id": entry.id.to_string(),
            "agent_name": entry.name,
        });

        let result = self
            .wasm_sandbox
            .execute(
                &wasm_bytes,
                input,
                sandbox_config,
                Some(kernel_handle),
                &entry.id.to_string(),
            )
            .await
            // #3711 (2-of-21): propagate the typed `SandboxError` instead
            // of collapsing it to `LibreFangError::Internal(String)`.
            // Display output ("WASM execution failed: …") is preserved
            // byte-for-byte by the format on `KernelError::WasmSandbox`,
            // so existing log/UI strings remain identical while upstream
            // callers gain the ability to match on typed variants
            // (e.g., `FuelExhausted` → CPU-budget quota error).
            .map_err(KernelError::from)?;

        // Extract response text from WASM output JSON
        let response = result
            .output
            .get("response")
            .and_then(|v| v.as_str())
            .or_else(|| result.output.get("text").and_then(|v| v.as_str()))
            .or_else(|| result.output.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::to_string(&result.output).unwrap_or_default());

        info!(
            agent = %entry.name,
            fuel_consumed = result.fuel_consumed,
            "WASM agent execution complete"
        );

        Ok(AgentLoopResult {
            response,
            total_usage: librefang_types::message::TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                ..Default::default()
            },
            iterations: 1,
            cost_usd: None,
            silent: false,
            directives: Default::default(),
            decision_traces: Vec::new(),
            memories_saved: Vec::new(),
            memories_used: Vec::new(),
            memory_conflicts: Vec::new(),
            provider_not_configured: false,
            experiment_context: None,
            latency_ms: 0,
            // WASM agents don't mutate the session; N/A.
            new_messages_start: 0,
            skill_evolution_suggested: false,
            owner_notice: None,
        })
    }

    /// Execute a Python script agent.
    ///
    /// Delegates to `python_runtime::run_python_agent()` via subprocess.
    pub(crate) async fn execute_python_agent(
        &self,
        entry: &AgentEntry,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<AgentLoopResult> {
        let script_path = entry.manifest.module.strip_prefix("python:").unwrap_or("");
        let resolved_path = self.resolve_module_path(script_path);

        info!(agent = %entry.name, path = %resolved_path.display(), "Executing Python agent");

        let config = PythonConfig {
            timeout_secs: (entry.manifest.resources.max_cpu_time_ms / 1000).max(30),
            working_dir: Some(
                resolved_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .to_string_lossy()
                    .to_string(),
            ),
            ..PythonConfig::default()
        };

        let context = serde_json::json!({
            "agent_name": entry.name,
            "system_prompt": entry.manifest.model.system_prompt,
        });

        let result = python_runtime::run_python_agent(
            &resolved_path.to_string_lossy(),
            &agent_id.to_string(),
            message,
            &context,
            &config,
        )
        .await
        // #3711 (4-of-21): propagate the typed `PythonError` instead of
        // collapsing it to `LibreFangError::Internal(String)`. Display
        // output ("Python execution failed: …") is preserved byte-for-byte
        // by the format on `KernelError::Python`, so existing log/UI
        // strings remain identical while upstream callers gain the ability
        // to match on typed variants (e.g., `Timeout` → 408, `ScriptError`
        // → 422).
        .map_err(KernelError::from)?;

        info!(agent = %entry.name, "Python agent execution complete");

        Ok(AgentLoopResult {
            response: result.response,
            total_usage: librefang_types::message::TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                ..Default::default()
            },
            cost_usd: None,
            iterations: 1,
            silent: false,
            directives: Default::default(),
            decision_traces: Vec::new(),
            memories_saved: Vec::new(),
            memories_used: Vec::new(),
            memory_conflicts: Vec::new(),
            provider_not_configured: false,
            experiment_context: None,
            latency_ms: 0,
            // Python agents don't mutate the session; N/A.
            new_messages_start: 0,
            skill_evolution_suggested: false,
            owner_notice: None,
        })
    }

    pub(crate) fn should_reuse_cached_route(message: &str) -> bool {
        Self::should_skip_intent_classification(message) && !Self::is_brief_acknowledgement(message)
    }

    fn is_brief_acknowledgement(message: &str) -> bool {
        let trimmed = message.trim();
        let lower = trimmed.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "ok" | "okay"
                | "thanks"
                | "thank you"
                | "thx"
                | "cool"
                | "great"
                | "nice"
                | "got it"
                | "sounds good"
        ) || matches!(
            trimmed,
            "好的" | "谢谢" | "谢了" | "收到" | "了解" | "行" | "好" | "多谢"
        )
    }

    /// Execute the default LLM-based agent loop.
    #[allow(clippy::too_many_arguments)]
    #[instrument(
        skip_all,
        fields(
            agent.id = %agent_id,
            agent.name = %entry.manifest.name,
            message.len = message.len(),
            channel = sender_context.map(|c| c.channel.as_str()).unwrap_or("direct"),
        ),
    )]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_llm_agent(
        &self,
        entry: &AgentEntry,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        content_blocks: Option<Vec<librefang_types::message::ContentBlock>>,
        sender_context: Option<&SenderContext>,
        session_mode_override: Option<librefang_types::agent::SessionMode>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
        upstream_interrupt: Option<librefang_runtime::interrupt::SessionInterrupt>,
        incognito: bool,
    ) -> KernelResult<AgentLoopResult> {
        let cfg = self.config.load_full();
        // Check metering quota before starting
        self.metering
            .engine
            .check_quota(agent_id, &entry.manifest.resources)
            .map_err(KernelError::LibreFang)?;

        // Sticky-flip: this is the single chokepoint for "agent processed a
        // real message" — any inbound message, channel event, autonomous
        // tick, cron fire, or fork that produces an LLM call routes here.
        // The heartbeat monitor uses this flag (not a time window) to
        // decide whether an idle agent should be flagged unresponsive.
        // Idempotent: subsequent calls only refresh `last_active`.
        self.agents.registry.mark_processed_message(agent_id);

        // Derive session ID. Resolution order (highest priority first):
        //
        // 1. Explicit override from the HTTP caller (multi-tab / multi-session UIs).
        //    Safety check: if the session exists and belongs to a different agent,
        //    reject with an error so sessions can never bleed across agents.
        // 2. Channel-derived deterministic ID: `SessionId::for_channel(agent, scope)`
        //    where scope = "<channel>:<chat_id>" (or just "<channel>"). Prevents
        //    context bleed between group and DM on the same (agent, channel).
        // 3. Session-mode fallback: per-trigger override > agent manifest default.
        //    `use_canonical_session` forces Persistent so the dashboard WS always
        //    persists to `entry.session_id`.
        let effective_session_id = if let Some(sid) = session_id_override {
            if let Some(existing) = self
                .memory
                .substrate
                .get_session(sid)
                .map_err(KernelError::LibreFang)?
            {
                if existing.agent_id != agent_id {
                    return Err(KernelError::LibreFang(LibreFangError::InvalidInput(
                        format!("session {} belongs to a different agent", sid),
                    )));
                }
            }
            sid
        } else {
            match sender_context {
                Some(ctx) if !ctx.channel.is_empty() && !ctx.use_canonical_session => {
                    let derived =
                        SessionId::for_sender_scope(agent_id, &ctx.channel, ctx.chat_id.as_deref());
                    // #3692: surface when the channel branch silently
                    // overrides a non-default manifest `session_mode`.
                    // The `execute_llm_agent` path is reached by
                    // channel bridges (always) and by the cron
                    // dispatcher (synthetic `SenderContext{channel:
                    // "cron"}`), so this is the canonical place where
                    // the manifest declaration gets dropped on the
                    // floor. Logged at `debug!` when the manifest /
                    // per-trigger override actually disagrees with the
                    // channel-derived id; `trace!` otherwise.
                    let requested_mode =
                        session_mode_override.unwrap_or(entry.manifest.session_mode);
                    if matches!(requested_mode, librefang_types::agent::SessionMode::New) {
                        debug!(
                            agent_id = %agent_id,
                            effective_session_id = %derived,
                            resolution_source = "channel-derived",
                            requested_session_mode = ?requested_mode,
                            channel = %ctx.channel,
                            chat_id = ctx.chat_id.as_deref().unwrap_or(""),
                            "session_mode override ignored: channel branch derives a deterministic SessionId::for_channel(agent, channel:chat)"
                        );
                    } else {
                        tracing::trace!(
                            agent_id = %agent_id,
                            effective_session_id = %derived,
                            resolution_source = "channel-derived",
                            requested_session_mode = ?requested_mode,
                            channel = %ctx.channel,
                            "session resolved via channel branch"
                        );
                    }
                    derived
                }
                _ => {
                    let mode = session_mode_override.unwrap_or(entry.manifest.session_mode);
                    match mode {
                        librefang_types::agent::SessionMode::Persistent => entry.session_id,
                        librefang_types::agent::SessionMode::New => SessionId::new(),
                    }
                }
            }
        };

        let mut session = self
            .memory
            .substrate
            .get_session(effective_session_id)
            .map_err(KernelError::LibreFang)?
            .unwrap_or_else(|| librefang_memory::session::Session {
                id: effective_session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                model_override: None,
                messages_generation: 0,
                last_repaired_generation: None,
            });
        // Evaluate the global session reset policy against this agent's
        // last_active timestamp.  The `force_session_wipe` flag on the entry
        // acts as an operator-forced hard-wipe signal that always wins
        // regardless of the configured mode.
        //
        // When a reset is required:
        //   - `session.messages` is cleared so the LLM starts a fresh context.
        //   - The registry entry's `force_session_wipe` / `resume_pending`
        //     flags and `reset_reason` are updated in-place.
        //
        // `mode = "off"` (the default) is a no-op — fully backward compatible.
        //
        // Skip entirely for `session_mode = "new"`: every invocation already
        // gets a fresh ephemeral session_id, so there is nothing to reset and
        // we must not touch the `force_session_wipe` / `resume_pending` flags
        // that belong to the persistent session path.
        {
            use crate::session_policy::SessionResetPolicyExt;
            let effective_mode = session_mode_override.unwrap_or(entry.manifest.session_mode);
            // `New` mode creates a fresh ephemeral session_id on every call;
            // there is nothing persistent to reset, and mutating
            // `force_session_wipe`/`resume_pending` flags would corrupt state
            // for future persistent-mode invocations.
            let skip_reset = matches!(effective_mode, librefang_types::agent::SessionMode::New);
            if !skip_reset {
                let policy = cfg.session.reset.clone();
                let last_active: std::time::SystemTime = entry.last_active.into();
                if let Some(reason) = policy.should_reset(last_active, entry.force_session_wipe) {
                    tracing::info!(
                        agent_id = %agent_id,
                        agent = %entry.name,
                        reason = %reason,
                        event = "session_reset",
                        "Auto-resetting session per policy"
                    );
                    if !session.messages.is_empty() {
                        session.messages.clear();
                        session.mark_messages_mutated();
                    }
                    // Persist the cleared session immediately so the next
                    // invocation loads an empty transcript from storage rather
                    // than re-loading the stale pre-reset messages.  Without
                    // this the downstream "persist if anything was injected"
                    // guard (which is skipped when there are no injections)
                    // would leave the storage copy untouched and the reset
                    // would be invisible to subsequent calls.
                    if let Err(e) = self.memory.substrate.save_session_async(&session).await {
                        tracing::warn!(
                            agent_id = %agent_id,
                            error = %e,
                            "Failed to persist session after auto-reset"
                        );
                    }
                    let _ = self
                        .agents
                        .registry
                        .update_session_reset_state(agent_id, reason);
                    // Persist the updated entry so the reset state survives a crash.
                    // Other registry updates (update_skills, update_mcp_servers, etc.)
                    // follow the same pattern: update + save_agent.
                    if let Some(updated) = self.agents.registry.get(agent_id) {
                        if let Err(e) = self.memory.substrate.save_agent_async(&updated).await {
                            tracing::warn!(
                                agent_id = %agent_id,
                                error = %e,
                                "Failed to persist agent entry after auto-reset"
                            );
                        }
                    }
                }
            }
        }
        // ───────────────────────────────────────────────────────────────────

        let tools = self.available_tools(agent_id);
        let tools = entry.mode.filter_tools((*tools).clone());

        info!(
            agent = %entry.name,
            agent_id = %agent_id,
            tool_count = tools.len(),
            tool_names = ?tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            "Tools selected for LLM request"
        );

        // Apply model routing if configured (disabled in Stable mode)
        let mut manifest = entry.manifest.clone();

        // Resolve "default" provider/model to the current effective default.
        // This covers three cases:
        // 1. New agents stored as "default"/"default" (post-fix spawn behavior)
        // 2. The auto-spawned "assistant" agent that may have a stale concrete
        //    provider/model in DB from before a provider switch
        // 3. TOML agents with provider="default" that got a concrete value baked in
        {
            let is_default_provider =
                manifest.model.provider.is_empty() || manifest.model.provider == "default";
            let is_default_model =
                manifest.model.model.is_empty() || manifest.model.model == "default";
            let is_auto_spawned = entry.name == "assistant"
                && manifest
                    .description
                    .starts_with("General-purpose assistant");
            if (is_default_provider && is_default_model) || is_auto_spawned {
                let override_guard = self
                    .llm
                    .default_model_override
                    .read()
                    .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
                let dm = override_guard.as_ref().unwrap_or(&cfg.default_model);
                if !dm.provider.is_empty() {
                    manifest.model.provider = dm.provider.clone();
                }
                if !dm.model.is_empty() {
                    manifest.model.model = dm.model.clone();
                }
                if !dm.api_key_env.is_empty() && manifest.model.api_key_env.is_none() {
                    manifest.model.api_key_env = Some(dm.api_key_env.clone());
                }
                if dm.base_url.is_some() && manifest.model.base_url.is_none() {
                    manifest.model.base_url.clone_from(&dm.base_url);
                }
            }
        }

        // Apply per-session model override (#4898). Runs after default-model
        // resolution so it takes precedence over both the manifest default and
        // the global default override. Running here (in the kernel dispatcher)
        // means every downstream consumer — billing, router, metering — sees
        // the effective model, not just the agent loop.
        if let Some(override_str) = session.model_override.as_deref() {
            librefang_runtime::agent_loop::apply_session_model_override_to_manifest(
                &mut manifest,
                override_str,
            )
            .map_err(KernelError::LibreFang)?;
        }

        // Backfill thinking config from global config if per-agent is not set
        if manifest.thinking.is_none() {
            manifest.thinking = cfg.thinking.clone();
        }

        // Apply per-call thinking override (from API request).
        apply_thinking_override(&mut manifest, thinking_override);

        // Lazy backfill: create workspace for existing agents spawned before workspaces
        if manifest.workspace.is_none() {
            let workspace_dir =
                backfill_workspace_dir(&cfg, &manifest.tags, &manifest.name, agent_id)?;
            if let Err(e) = ensure_workspace(&workspace_dir) {
                warn!(agent_id = %agent_id, "Failed to backfill workspace: {e}");
            } else {
                migrate_identity_files(&workspace_dir);
                manifest.workspace = Some(workspace_dir);
                // Persist updated workspace in registry
                let _ = self
                    .agents
                    .registry
                    .update_workspace(agent_id, manifest.workspace.clone());
            }
        }

        // Build the structured system prompt via prompt_builder.
        // Workspace metadata and skill summaries are cached to avoid redundant
        // filesystem I/O and skill registry iteration on every message.
        {
            let mcp_tool_count = self.mcp.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
            let shared_id = shared_memory_agent_id();
            let stable_prefix_mode = cfg.stable_prefix_mode;
            // Apply the same peer-scoping that `memory_store` uses on write so
            // the key we read actually matches what the agent stored.  When
            // sender_context carries a non-empty user_id (e.g. the WebUI client
            // IP or a channel user identifier) the key is `peer:{id}:user_name`;
            // for system / autonomous / cron turns (no sender) we fall back to
            // the unscoped `"user_name"` key — same as the write side.
            let peer_id = sender_context
                .map(|s| s.user_id.as_str())
                .filter(|s| !s.is_empty());
            let user_name_key = peer_scoped_key("user_name", peer_id);
            let user_name = self
                .memory
                .substrate
                .structured_get(shared_id, &user_name_key)
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(String::from));

            let peer_agents: Vec<(String, String, String)> =
                self.agents.registry.peer_agents_summary();

            // Use cached workspace metadata (identity files + workspace context)
            let ws_meta = manifest
                .workspace
                .as_ref()
                .map(|w| self.cached_workspace_metadata(w, manifest.autonomous.is_some()));

            // Use cached skill metadata (summary + prompt context)
            let skill_meta = if manifest.skills_disabled {
                None
            } else {
                Some(self.cached_skill_metadata(&manifest.skills))
            };

            let is_subagent_flag = manifest
                .metadata
                .get("is_subagent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let agent_id_str = agent_id.0.to_string();
            let hook_ctx = librefang_runtime::hooks::HookContext {
                agent_name: &manifest.name,
                agent_id: agent_id_str.as_str(),
                event: librefang_types::agent::HookEvent::BeforePromptBuild,
                data: serde_json::json!({
                    "phase": "build",
                    "call_site": "execute_llm",
                    "user_message": message,
                    "session_id": effective_session_id.to_string(),
                    "channel_type": sender_context.map(|s| s.channel.clone()),
                    "is_group": sender_context.map(|s| s.is_group).unwrap_or(false),
                    "is_subagent": is_subagent_flag,
                    "granted_tools": tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
                }),
            };
            let dynamic_sections = self.governance.hooks.collect_prompt_sections(&hook_ctx);

            // Re-read context.md per turn (cache_context=true to opt out).
            // Pre-loaded off the runtime worker via tokio::fs — see #3579.
            let context_md = match manifest.workspace.as_ref() {
                Some(w) => {
                    librefang_runtime::agent_context::load_context_md_async(
                        w,
                        manifest.cache_context,
                    )
                    .await
                }
                None => None,
            };

            let prompt_ctx = librefang_runtime::prompt_builder::PromptContext {
                agent_name: manifest.name.clone(),
                agent_description: manifest.description.clone(),
                base_system_prompt: manifest.model.system_prompt.clone(),
                granted_tools: tools.iter().map(|t| t.name.clone()).collect(),
                recalled_memories: vec![], // Recalled in agent_loop, not here
                skill_summary: skill_meta
                    .as_ref()
                    .map(|s| s.skill_summary.clone())
                    .unwrap_or_default(),
                skill_count: skill_meta.as_ref().map(|s| s.skill_count).unwrap_or(0),
                skill_prompt_context: skill_meta
                    .as_ref()
                    .map(|s| s.skill_prompt_context.clone())
                    .unwrap_or_default(),
                skill_config_section: skill_meta
                    .as_ref()
                    .map(|s| s.skill_config_section.clone())
                    .unwrap_or_default(),
                mcp_summary: if mcp_tool_count > 0 && !manifest.mcp_disabled {
                    self.build_mcp_summary(&manifest.mcp_servers)
                } else {
                    String::new()
                },
                workspace_path: manifest.workspace.as_ref().map(|p| p.display().to_string()),
                soul_md: ws_meta.as_ref().and_then(|m| m.soul_md.clone()),
                user_md: ws_meta.as_ref().and_then(|m| m.user_md.clone()),
                memory_md: ws_meta.as_ref().and_then(|m| m.memory_md.clone()),
                canonical_context: if stable_prefix_mode {
                    None
                } else {
                    self.memory
                        .substrate
                        .canonical_context(agent_id, Some(effective_session_id), None)
                        .ok()
                        .and_then(|(s, _)| s)
                },
                user_name,
                channel_type: sender_context.map(|s| s.channel.clone()),
                sender_display_name: sender_context.map(|s| s.display_name.clone()),
                sender_user_id: sender_context.map(|s| s.user_id.clone()),
                is_group: sender_context.map(|s| s.is_group).unwrap_or(false),
                was_mentioned: sender_context.map(|s| s.was_mentioned).unwrap_or(false),
                is_subagent: is_subagent_flag,
                is_autonomous: manifest.autonomous.is_some(),
                agents_md: ws_meta.as_ref().and_then(|m| m.agents_md.clone()),
                bootstrap_md: ws_meta.as_ref().and_then(|m| m.bootstrap_md.clone()),
                workspace_context: ws_meta.as_ref().and_then(|m| m.workspace_context.clone()),
                identity_md: ws_meta.as_ref().and_then(|m| m.identity_md.clone()),
                heartbeat_md: ws_meta.as_ref().and_then(|m| m.heartbeat_md.clone()),
                tools_md: ws_meta.as_ref().and_then(|m| m.tools_md.clone()),
                peer_agents,
                current_date: Some(
                    // Date only — omitting the clock time keeps the system prompt
                    // stable across the ~1 440 turns in a day so LLM providers
                    // (Anthropic, OpenAI) can cache it.  A per-minute timestamp
                    // invalidates the prompt cache every 60 s, doubling effective
                    // token cost (issue #3700).
                    chrono::Local::now()
                        .format("%A, %B %d, %Y (%Y-%m-%d %Z)")
                        .to_string(),
                ),
                active_goals: self.active_goals_for_prompt(Some(agent_id)),
                context_md,
                dynamic_sections,
            };
            manifest.model.system_prompt =
                librefang_runtime::prompt_builder::build_system_prompt(&prompt_ctx);
            // Pass stable_prefix_mode flag to the agent loop via metadata
            manifest.metadata.insert(
                STABLE_PREFIX_MODE_METADATA_KEY.to_string(),
                serde_json::json!(stable_prefix_mode),
            );
            // Store canonical context separately for injection as user message
            // (keeps system prompt stable across turns for provider prompt caching)
            if let Some(cc_msg) =
                librefang_runtime::prompt_builder::build_canonical_context_message(&prompt_ctx)
            {
                manifest.metadata.insert(
                    "canonical_context_msg".to_string(),
                    serde_json::Value::String(cc_msg),
                );
            }

            // Pass prompt_caching config to the agent loop via metadata.
            manifest.metadata.insert(
                "prompt_caching".to_string(),
                serde_json::Value::Bool(cfg.prompt_caching),
            );
            // Pass the prompt-cache strategy (#4970) as a string —
            // the agent loop parses it back into a `PromptCacheStrategy`.
            manifest.metadata.insert(
                "prompt_cache_strategy".to_string(),
                serde_json::Value::String(cfg.prompt_cache.strategy.to_string()),
            );
            manifest.metadata.insert(
                "prompt_cache_ttl_hint_secs".to_string(),
                serde_json::Value::from(cfg.prompt_cache.cache_ttl_hint_secs),
            );

            // Pass privacy config to the agent loop via metadata.
            if let Ok(privacy_json) = serde_json::to_value(&cfg.privacy) {
                manifest
                    .metadata
                    .insert("privacy".to_string(), privacy_json);
            }
        }

        let is_stable = cfg.mode == librefang_types::config::KernelMode::Stable;

        if is_stable {
            // In Stable mode: use pinned_model if set, otherwise default model
            if let Some(ref pinned) = manifest.pinned_model {
                info!(
                    agent = %manifest.name,
                    pinned_model = %pinned,
                    "Stable mode: using pinned model"
                );
                manifest.model.model = pinned.clone();
            }
        } else if let Some(routing_config) =
            manifest.routing.as_ref().or(cfg.default_routing.as_ref())
        {
            let mut router = ModelRouter::new(routing_config.clone());
            // Resolve aliases (e.g. "sonnet" -> "claude-sonnet-4-20250514") before scoring
            router.resolve_aliases(&self.llm.model_catalog.load());
            // Build a probe request to score complexity
            let probe_model =
                strip_provider_prefix(&manifest.model.model, &manifest.model.provider);
            let echo_policy = self.lookup_reasoning_echo_policy(&probe_model);
            let probe = CompletionRequest {
                model: probe_model,
                messages: std::sync::Arc::new(vec![librefang_types::message::Message::user(
                    message,
                )]),
                tools: std::sync::Arc::new(tools.clone()),
                max_tokens: manifest.model.max_tokens,
                temperature: manifest.model.temperature,
                system: Some(manifest.model.system_prompt.clone()),
                thinking: None,
                prompt_caching: false,
                cache_ttl: None,
                prompt_cache_strategy: None,
                response_format: None,
                timeout_secs: None,
                extra_body: None,
                agent_id: None,
                session_id: None,
                step_id: None,
                reasoning_echo_policy: echo_policy,
            };
            let (complexity, routed_model) = router.select_model(&probe);
            // Check if the routed model's provider has a valid API key.
            // If not, keep the current (default) provider instead of switching
            // to one the user hasn't configured.
            let mut use_routed = true;
            let cat = self.llm.model_catalog.load();
            {
                if let Some(entry) = cat.find_model(&routed_model) {
                    if entry.provider != manifest.model.provider {
                        let key_env = cfg.resolve_api_key_env(&entry.provider);
                        if std::env::var(&key_env).is_err() {
                            warn!(
                                agent = %manifest.name,
                                routed_model = %routed_model,
                                provider = %entry.provider,
                                "Model routing skipped — provider API key not configured, using default"
                            );
                            use_routed = false;
                        }
                    }
                }
            }
            if use_routed {
                info!(
                    agent = %manifest.name,
                    complexity = %complexity,
                    routed_model = %routed_model,
                    "Model routing applied"
                );
                manifest.model.model = routed_model.clone();
                let cat = self.llm.model_catalog.load();
                {
                    if let Some(entry) = cat.find_model(&routed_model) {
                        if entry.provider != manifest.model.provider {
                            manifest.model.provider = entry.provider.clone();
                        }
                    }
                }
            }
        }

        // Apply per-model inference parameter overrides from the catalog.
        // Placed AFTER model routing so overrides match the final model, not
        // the pre-routing one (e.g. routing may switch sonnet → haiku).
        // Priority: model overrides > agent manifest > system defaults.
        {
            let override_key = format!("{}:{}", manifest.model.provider, manifest.model.model);
            let catalog = self.llm.model_catalog.load();
            if let Some(mo) = catalog.get_overrides(&override_key) {
                if let Some(t) = mo.temperature {
                    manifest.model.temperature = t;
                }
                if let Some(mt) = mo.max_tokens {
                    manifest.model.max_tokens = mt;
                }
                let ep = &mut manifest.model.extra_params;
                if let Some(tp) = mo.top_p {
                    ep.insert("top_p".to_string(), serde_json::json!(tp));
                }
                if let Some(fp) = mo.frequency_penalty {
                    ep.insert("frequency_penalty".to_string(), serde_json::json!(fp));
                }
                if let Some(pp) = mo.presence_penalty {
                    ep.insert("presence_penalty".to_string(), serde_json::json!(pp));
                }
                if let Some(ref re) = mo.reasoning_effort {
                    ep.insert("reasoning_effort".to_string(), serde_json::json!(re));
                }
                if mo.use_max_completion_tokens == Some(true) {
                    ep.insert(
                        "use_max_completion_tokens".to_string(),
                        serde_json::json!(true),
                    );
                }
                if mo.force_max_tokens == Some(true) {
                    ep.insert("force_max_tokens".to_string(), serde_json::json!(true));
                }
            }
        }

        let driver = self.resolve_driver(&manifest)?;

        // Look up model's actual context window from the catalog. Filter out
        // 0 so image/audio entries (no context window) fall through to the
        // caller's default rather than poisoning compaction math.
        let ctx_window = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model(&manifest.model.model)
                .map(|m| m.context_window as usize)
                .filter(|w| *w > 0)
        });

        // Inject model_supports_tools for auto web search augmentation.
        // Refs #4745: honour user capability overrides via effective_capabilities.
        if let Some(supports) = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model(&manifest.model.model)
                .map(|m| cat.effective_capabilities(m).supports_tools)
        }) {
            manifest.metadata.insert(
                "model_supports_tools".to_string(),
                serde_json::Value::Bool(supports),
            );
        }

        // Snapshot skill registry before async call (RwLockReadGuard is !Send)
        let mut skill_snapshot = self
            .skills
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        // Load workspace-scoped skills (override global skills with same name)
        if let Some(ref workspace) = manifest.workspace {
            let ws_skills = workspace.join("skills");
            if ws_skills.exists() {
                if let Err(e) = skill_snapshot.load_workspace_skills(&ws_skills) {
                    warn!(agent_id = %agent_id, "Failed to load workspace skills: {e}");
                }
            }
        }

        // Strip the [SILENT] marker before the message reaches the LLM. The
        // marker is a system-level signal for the kernel; the LLM should never
        // see it in the conversation. Stripping must happen before link-context
        // expansion so the expanded string is also clean.
        // Only active for internal cron calls (is_internal_cron flag) — external
        // callers cannot trigger this path, so legitimate user messages containing
        // "[SILENT]" (e.g. "add a `[SILENT]` comment") are preserved.
        let is_internal_cron = sender_context.is_some_and(|ctx| ctx.is_internal_cron);
        let message_for_llm = if is_internal_cron && message.contains("[SILENT]") {
            let stripped = message.replace("[SILENT]", "").trim().to_string();
            if stripped.is_empty() {
                message.trim().to_string()
            } else {
                stripped
            }
        } else {
            message.trim().to_string()
        };

        // Build link context from user message (auto-extract URLs for the agent)
        let message_with_links = if let Some(link_ctx) =
            librefang_runtime::link_understanding::build_link_context(&message_for_llm, &cfg.links)
        {
            format!("{message_for_llm}{link_ctx}")
        } else {
            message_for_llm
        };

        // Inject sender context into manifest metadata so the tool runner can
        // use it for per-sender trust and channel-specific authorization rules.
        if let Some(ctx) = sender_context {
            if !ctx.user_id.is_empty() {
                manifest.metadata.insert(
                    "sender_user_id".to_string(),
                    serde_json::Value::String(ctx.user_id.clone()),
                );
            }
            if !ctx.channel.is_empty() {
                manifest.metadata.insert(
                    "sender_channel".to_string(),
                    serde_json::Value::String(ctx.channel.clone()),
                );
            }
            if !ctx.display_name.is_empty() {
                manifest.metadata.insert(
                    "sender_display_name".to_string(),
                    serde_json::Value::String(ctx.display_name.clone()),
                );
            }
            if ctx.is_group {
                manifest
                    .metadata
                    .insert("is_group".to_string(), serde_json::Value::Bool(true));
            }
        }

        let proactive_memory = self.memory.proactive_memory.get().cloned();

        // Set up mid-turn injection channel.
        let injection_rx = self.setup_injection_channel(agent_id, effective_session_id);

        // Session-scoped interrupt for tool-level cancellation.  Cloned into
        // each ToolExecutionContext so that cancelling the session (via
        // interrupt.cancel()) aborts in-flight tools without affecting other
        // concurrent sessions. When this child turn was invoked on behalf of
        // a parent session (e.g. via `agent_send` during a parent's tool
        // batch), `upstream_interrupt` carries the parent's handle so a
        // parent /stop cascades down to this subagent. See issue #3044.
        let session_interrupt = match upstream_interrupt.as_ref() {
            Some(up) => librefang_runtime::interrupt::SessionInterrupt::new_with_upstream(up),
            None => librefang_runtime::interrupt::SessionInterrupt::new(),
        };
        // Register in session_interrupts so stop_agent_run / stop_session_run
        // can call cancel() even when the caller uses the non-streaming
        // send_message() path. Map keyed by (agent, session) post-#3172 so
        // concurrent sessions for one agent don't overwrite each other.
        self.agents
            .session_interrupts
            .insert((agent_id, effective_session_id), session_interrupt.clone());
        let loop_opts = librefang_runtime::agent_loop::LoopOptions {
            is_fork: false,
            incognito,
            allowed_tools: None,
            interrupt: Some(session_interrupt),
            max_iterations: cfg.agent_max_iterations,
            max_history_messages: cfg.max_history_messages,
            aux_client: Some(self.llm.aux_client.load_full()),
            parent_session_id: None,
            tool_results_config: Some(cfg.tool_results.clone()),
        };

        // Build a per-execution MCP pool that includes the agent workspace as
        // a root. Falls back to the global pool if the workspace adds nothing
        // new or if all connections fail.
        let agent_mcp = self
            .build_agent_mcp_pool(manifest.workspace.as_deref())
            .await;
        let effective_mcp = agent_mcp.as_ref().unwrap_or(&self.mcp.mcp_connections);

        // Fire external agent:start hook (fire-and-forget, never blocks execution).
        {
            let preview: String = message.chars().take(200).collect();
            self.governance.external_hooks.fire(
                crate::hooks::ExternalHookEvent::AgentStart,
                serde_json::json!({
                    "agent_id": agent_id.to_string(),
                    "agent_name": entry.name,
                    "session_id": effective_session_id.0.to_string(),
                    "message_preview": preview,
                }),
            );
        }

        let start_time = std::time::Instant::now();
        let result = run_agent_loop(
            &manifest,
            &message_with_links,
            &mut session,
            &self.memory.substrate,
            driver,
            &tools,
            Some(kernel_handle),
            Some(&skill_snapshot),
            Some(effective_mcp),
            Some(&self.media.web_ctx),
            Some(&self.media.browser_ctx),
            self.llm.embedding_driver.as_deref(),
            manifest.workspace.as_deref(),
            None, // on_phase callback
            Some(&self.media.media_engine),
            Some(&self.media.media_drivers),
            if cfg.tts.enabled {
                Some(&self.media.tts_engine)
            } else {
                None
            },
            if cfg.docker.enabled {
                Some(&cfg.docker)
            } else {
                None
            },
            Some(&self.governance.hooks),
            ctx_window,
            Some(&self.processes.manager),
            self.checkpoint_manager.clone(),
            Some(&self.processes.registry),
            content_blocks,
            proactive_memory,
            self.context_engine_for_agent(&manifest),
            Some(&injection_rx),
            &loop_opts,
        )
        .await;

        // Tear down injection channel after loop finishes.
        self.teardown_injection_channel(agent_id, effective_session_id);

        // Clean up the interrupt handle regardless of outcome — the map must
        // not retain stale entries that would suppress cancellation on the
        // next run for the same (agent, session) pair.
        self.agents
            .session_interrupts
            .remove(&(agent_id, effective_session_id));

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // Fire external agent:end hook (fire-and-forget) before checking result.
        // This ensures the hook fires even when the agent loop returns an error,
        // matching the principle that "agent:end" fires on loop completion.
        let hook_payload = if let Ok(ref r) = result {
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "agent_name": entry.name,
                "session_id": effective_session_id.0.to_string(),
                "latency_ms": latency_ms,
                "success": true,
                "input_tokens": r.total_usage.input_tokens,
                "output_tokens": r.total_usage.output_tokens,
            })
        } else {
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "agent_name": entry.name,
                "session_id": effective_session_id.0.to_string(),
                "latency_ms": latency_ms,
                "success": false,
            })
        };
        self.governance
            .external_hooks
            .fire(crate::hooks::ExternalHookEvent::AgentEnd, hook_payload);

        let result = result.map_err(KernelError::LibreFang)?;

        // Cron [SILENT] marker: if the cron prompt contains "[SILENT]", the
        // agent intends this job to be maintenance-only. Strip the assistant
        // response from session history so it does not pollute the conversation
        // context for future turns. The prompt is checked on the original
        // `message` parameter (before any link-context additions) so the
        // marker placement is unambiguous to the job author.
        //
        // Gated to internal cron callers only (is_internal_cron flag) so
        // that a regular user sending "[SILENT]" in chat does not accidentally
        // suppress their own session history. The channel field cannot be
        // trusted because external callers can set it via the API.
        //
        // Session write: we still save the session — we just remove the
        // assistant turn from it first so the next cron fire does not see the
        // suppressed response in its context window.
        // Canonical append: skipped entirely for silent cron turns.
        let skip_canonical_append = if is_internal_cron && message.contains("[SILENT]") {
            // Remove the last assistant message from the in-memory session so
            // it is not included in the re-saved version.
            let removed = session
                .messages
                .iter()
                .rposition(|msg| msg.role == librefang_types::message::Role::Assistant)
                .map(|idx| {
                    session.messages.remove(idx);
                    session.mark_messages_mutated();
                    true
                })
                .unwrap_or(false);

            if removed {
                // Persist the stripped session. agent_loop already called
                // save_session internally; this second save overwrites that
                // with the version that has the assistant turn removed.
                if let Err(e) = self.memory.substrate.save_session_async(&session).await {
                    warn!("cron [SILENT]: failed to persist stripped session: {e}");
                }
            }
            tracing::info!(
                event = "cron_silent_job_completed",
                agent = %entry.name,
                agent_id = %agent_id,
                stripped = removed,
                "[SILENT] cron job completed — assistant response suppressed from session history"
            );
            true
        } else {
            false
        };

        // Append new messages to canonical session for cross-channel memory.
        // Use run_agent_loop's own start index (post-trim) instead of one
        // captured here — the loop may trim session history and make a
        // locally-captured index stale (see #2067). Clamp defensively.
        // Skipped for [SILENT] cron turns — we stripped the assistant message
        // from the session above and do not want it in canonical context.
        if !skip_canonical_append {
            let start = result.new_messages_start.min(session.messages.len());
            if start < session.messages.len() {
                let new_messages = session.messages[start..].to_vec();
                if let Err(e) = self
                    .memory
                    .substrate
                    .append_canonical_async(
                        agent_id,
                        &new_messages,
                        None,
                        Some(effective_session_id),
                    )
                    .await
                {
                    warn!("Failed to update canonical session: {e}");
                }
            }
        }

        // Write JSONL session mirror to workspace
        if let Some(ref workspace) = manifest.workspace {
            if let Err(e) = self
                .memory
                .substrate
                .write_jsonl_mirror(&session, &workspace.join("sessions"))
            {
                warn!("Failed to write JSONL session mirror: {e}");
            }
            // Append daily memory log (best-effort)
            append_daily_memory_log(workspace, &result.response);
        }

        // Atomically check quotas and record usage in a single SQLite
        // transaction to prevent the TOCTOU race where concurrent requests
        // both pass the pre-check before either records its spend.
        let model = &manifest.model.model;
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &self.llm.model_catalog.load(),
            model,
            result.total_usage.input_tokens,
            result.total_usage.output_tokens,
            result.total_usage.cache_read_input_tokens,
            result.total_usage.cache_creation_input_tokens,
        );
        // RBAC M5: derive user/channel attribution from the inbound sender
        // so per-user budgets and audit events can roll up per call.
        let attribution_user_id: Option<UserId> =
            sender_context.and_then(|sc| self.security.auth.identify(&sc.channel, &sc.user_id));
        let attribution_channel: Option<String> = sender_context.map(|sc| sc.channel.clone());
        let usage_record = librefang_memory::usage::UsageRecord {
            agent_id,
            provider: manifest.model.provider.clone(),
            model: model.clone(),
            input_tokens: result.total_usage.input_tokens,
            output_tokens: result.total_usage.output_tokens,
            cost_usd: cost,
            tool_calls: result.decision_traces.len() as u32,
            latency_ms,
            user_id: attribution_user_id,
            channel: attribution_channel.clone(),
            session_id: Some(effective_session_id),
        };
        if let Err(e) = self.metering.engine.check_all_and_record(
            &usage_record,
            &manifest.resources,
            &self.current_budget(),
        ) {
            // Quota exceeded after the LLM call — log but still return the
            // result (the tokens were already consumed by the provider).
            tracing::warn!(
                agent_id = %agent_id,
                error = %e,
                "Post-call quota check failed; usage recorded anyway to keep accounting accurate"
            );
            // Hash-chain audit: BudgetExceeded surfaces in `/api/audit/query`
            // so an operator can correlate the denial with the user / channel.
            self.metering.audit_log.record_with_context(
                agent_id.to_string(),
                librefang_runtime::audit::AuditAction::BudgetExceeded,
                format!("{e}"),
                "denied",
                attribution_user_id,
                attribution_channel.clone(),
            );
            // Fall back to plain record so the cost is not lost from tracking
            let _ = self.metering.engine.record(&usage_record);
        } else if let Some(uid) = attribution_user_id {
            // RBAC M5: per-user budget enforcement, post-call (matches the
            // global / per-agent / per-provider semantics — the row was
            // already persisted above so `query_user_*` includes this
            // call). A breach trips BudgetExceeded for downstream gating
            // and dashboard visibility; the current response is returned
            // unchanged because the tokens are already billed.
            if let Some(user_budget) = self.security.auth.budget_for(uid) {
                if let Err(e) = self.metering.engine.check_user_budget(uid, &user_budget) {
                    tracing::warn!(
                        agent_id = %agent_id,
                        user = %uid,
                        error = %e,
                        "Per-user budget check failed"
                    );
                    self.metering.audit_log.record_with_context(
                        agent_id.to_string(),
                        librefang_runtime::audit::AuditAction::BudgetExceeded,
                        format!("{e}"),
                        "denied",
                        Some(uid),
                        attribution_channel.clone(),
                    );
                }
            }
        }

        // Populate cost on the result based on usage_footer mode
        let mut result = result;
        result.latency_ms = latency_ms;
        match cfg.usage_footer {
            librefang_types::config::UsageFooterMode::Off => {
                result.cost_usd = None;
            }
            librefang_types::config::UsageFooterMode::Cost
            | librefang_types::config::UsageFooterMode::Full => {
                result.cost_usd = if cost > 0.0 { Some(cost) } else { None };
            }
            librefang_types::config::UsageFooterMode::Tokens => {
                // Tokens are already in result.total_usage, omit cost
                result.cost_usd = None;
            }
        }

        // Fire-and-forget: ask the auxiliary cheap-tier model to generate a
        // short title for this session if it doesn't have one yet.  Spawned
        // AFTER the response is delivered so it never competes with the
        // user's turn for model attention; failures / timeouts are silent.
        self.spawn_session_label_generation(agent_id, effective_session_id);

        Ok(result)
    }
}
