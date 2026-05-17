//! Cluster pulled out of mod.rs in #4713 phase 3c.
//!
//! Hosts the `send_message*` / `send_message_streaming*` family plus the
//! `resolve_agent_home_channel` helper and `run_forked_agent_streaming`.
//! These methods form the public dispatch surface for agent turns
//! (channel bridges, HTTP routes, inter-agent calls, fork spawns).
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery — the pull-down rule (parent's private items are
//! visible to children) means everything declared in mod.rs is reachable
//! via `Self::*` from here.

use std::sync::Arc;

use librefang_channels::types::SenderContext;
use librefang_runtime::agent_loop::{run_agent_loop, AgentLoopResult};
use librefang_runtime::kernel_handle::prelude::*;
use librefang_types::agent::{AgentId, AgentState, SessionId};
use librefang_types::error::LibreFangError;
use tracing::info;

use crate::error::{KernelError, KernelResult};
use crate::metering::MeteringEngine;
use crate::MeteringSubsystemApi;

use super::*;

impl LibreFangKernel {
    /// Send a message to an agent and get a response.
    ///
    /// Automatically upgrades the kernel handle from `self_handle` so that
    /// agent turns triggered by cron, channels, events, or inter-agent calls
    /// have full access to kernel tools (cron_create, agent_send, etc.).
    pub async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_with_handle(agent_id, message, Some(self.kernel_handle()))
            .await
    }

    /// Send a message honouring a per-call `SessionMode` override.
    ///
    /// Used by workflow step execution (#4834) so a step targeting a registry
    /// agent can opt-in to a fresh session (`SessionMode::New`) or insist on
    /// the agent's persistent session (`SessionMode::Persistent`), overriding
    /// the target agent's manifest default. Resolution precedence is enforced
    /// inside `send_message_full`: per-call > manifest > kernel default.
    pub async fn send_message_with_session_mode(
        &self,
        agent_id: AgentId,
        message: &str,
        session_mode_override: Option<librefang_types::agent::SessionMode>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full(
            agent_id,
            message,
            self.kernel_handle(),
            None,
            None,
            session_mode_override,
            None,
            None,
        )
        .await
    }

    /// Send a multimodal message (text + images) to an agent and get a response.
    ///
    /// Used by channel bridges when a user sends a photo — the image is downloaded,
    /// base64 encoded, and passed as `ContentBlock::Image` alongside any caption text.
    pub async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_with_handle_and_blocks(
            agent_id,
            message,
            Some(self.kernel_handle()),
            Some(blocks),
        )
        .await
    }

    /// Send a message to an agent with sender identity context from a channel.
    ///
    /// The sender context (channel name, user ID, display name) is injected into
    /// the agent's system prompt so it knows who is talking and from which channel.
    pub async fn send_message_with_sender_context(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full(
            agent_id,
            message,
            self.kernel_handle(),
            None,
            Some(sender),
            None,
            None,
            None,
        )
        .await
    }

    /// Send a message with both sender identity context and a per-call
    /// deep-thinking override.
    ///
    /// Used by HTTP / channel paths that already track sender metadata but
    /// also need to honour a per-message thinking flag (e.g. the chat UI's
    /// deep-thinking toggle).
    pub async fn send_message_with_sender_context_and_thinking(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &SenderContext,
        thinking_override: Option<bool>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full(
            agent_id,
            message,
            self.kernel_handle(),
            None,
            Some(sender),
            None,
            thinking_override,
            None,
        )
        .await
    }

    /// Send a multimodal message with sender identity context from a channel.
    pub async fn send_message_with_blocks_and_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
        sender: &SenderContext,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full(
            agent_id,
            message,
            self.kernel_handle(),
            Some(blocks),
            Some(sender),
            None,
            None,
            None,
        )
        .await
    }

    /// Send a message with an optional kernel handle for inter-agent tools.
    ///
    /// `kernel_handle` is `Option` only because some tests pass a stub handle;
    /// production callers always reach this with `Some(...)` (see #3652). When
    /// `None`, the kernel auto-wires its own self-handle.
    pub async fn send_message_with_handle(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
    ) -> KernelResult<AgentLoopResult> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_full(agent_id, message, handle, None, None, None, None, None)
            .await
    }

    /// Send a message to `agent_id` on behalf of `parent_agent_id`. If the
    /// parent currently has an active session interrupt registered (i.e. is
    /// mid-turn), it is threaded as an upstream signal to the child's loop
    /// so a parent `/stop` cascades into the callee. When no parent
    /// interrupt is registered (parent is idle, or caller is system-level),
    /// behaves identically to [`Self::send_message`].
    ///
    /// Added for issue #3044 — previously a parent `agent_send`'ing to a
    /// hand / subagent could not stop the child when the user cancelled,
    /// because every new turn created a fresh, disconnected interrupt.
    pub async fn send_message_as(
        &self,
        agent_id: AgentId,
        message: &str,
        parent_agent_id: AgentId,
    ) -> KernelResult<AgentLoopResult> {
        let upstream = self.any_session_interrupt_for_agent(parent_agent_id);
        self.send_message_full_with_upstream(
            agent_id,
            message,
            self.kernel_handle(),
            None,
            None,
            None,
            None,
            None,
            upstream,
            false,
        )
        .await
    }

    /// Send a message with a per-call deep-thinking override.
    ///
    /// `thinking_override`:
    /// - `Some(true)` — force thinking on (use default budget if manifest has none)
    /// - `Some(false)` — force thinking off (clear any manifest/global setting)
    /// - `None` — use the manifest/global default
    pub async fn send_message_with_thinking_override(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        thinking_override: Option<bool>,
    ) -> KernelResult<AgentLoopResult> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_full(
            agent_id,
            message,
            handle,
            None,
            None,
            None,
            thinking_override,
            None,
        )
        .await
    }

    /// Send a message with an explicit session ID override, optional sender context,
    /// and optional deep-thinking override.
    ///
    /// Used by the HTTP `/message` endpoint when the caller supplies a `session_id`
    /// in the request body (multi-tab / multi-session UIs). Resolution order:
    /// explicit session_id > channel-derived > registry canonical.
    ///
    /// Returns 400 if `session_id_override` belongs to a different agent.
    pub async fn send_message_with_session_override(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<AgentLoopResult> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_full(
            agent_id,
            message,
            handle,
            None,
            sender_context,
            None,
            thinking_override,
            session_id_override,
        )
        .await
    }

    /// Send a message in **incognito mode**: the LLM turn runs normally (memory
    /// reads are full-access) but session messages and proactive-memory writes
    /// are silently suppressed. The conversation leaves no trace in SQLite.
    ///
    /// `incognito` is analogous to `is_fork` at the persistence boundary but
    /// without the fork semantics (no shared parent prefix, no tool allowlist).
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message_with_incognito(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<AgentLoopResult> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_full_with_upstream(
            agent_id,
            message,
            handle,
            None,
            sender_context,
            None,
            thinking_override,
            session_id_override,
            None,
            incognito,
        )
        .await
    }

    /// Send a message with optional content blocks and an optional kernel handle.
    ///
    /// When `content_blocks` is `Some`, the LLM agent loop receives structured
    /// multimodal content (text + images) instead of just a text string. This
    /// enables vision models to process images sent from channels like Telegram.
    ///
    /// Per-agent locking ensures that concurrent messages for the same agent
    /// are serialized (preventing session corruption), while messages for
    /// different agents run in parallel.
    pub async fn send_message_with_handle_and_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        content_blocks: Option<Vec<librefang_types::message::ContentBlock>>,
    ) -> KernelResult<AgentLoopResult> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_full(
            agent_id,
            message,
            handle,
            content_blocks,
            None,
            None,
            None,
            None,
        )
        .await
    }

    /// Resolve the **home channel** for an agent, if any.
    ///
    /// An agent's home channel is the channel instance in `config.toml` whose
    /// `default_agent` field names this agent. It represents the natural
    /// return path for proactive / trigger-fired messages that don't carry
    /// an inbound `SenderContext`.
    ///
    /// Returns a synthetic `SenderContext` populated with:
    /// - `channel` — the channel type (e.g. `"telegram"`, `"discord"`)
    /// - `account_id` — the specific bot instance's `account_id` (when set)
    /// - `use_canonical_session = true` — preserves the trigger's existing
    ///   `session_mode` semantics; without this the kernel would switch to a
    ///   channel-scoped `SessionId::for_channel(agent, channel)` which would
    ///   break the "persistent vs new" contract triggers rely on.
    ///
    /// Returns `None` when no channel's `default_agent` matches this agent —
    /// in that case callers should fall back to sender-context-less dispatch
    /// (the pre-#2872 behavior).
    pub(crate) fn resolve_agent_home_channel(&self, agent_id: AgentId) -> Option<SenderContext> {
        let entry = self.agents.registry.get(agent_id)?;
        let agent_name = entry.name.clone();
        let cfg = self.config.load_full();
        let channels = &cfg.channels;

        // Scan each channel type for the first instance whose default_agent
        // names this agent. The `first` semantics match `channel_overrides`
        // in channel_bridge.rs when multiple instances share a default_agent.
        //
        // `for_each_channel_field!` expands the exhaustive field list shared
        // with `resolve_channel_owner` in channel_sender.rs — one edit point
        // for all 40+ channel types keeps the two functions in sync.
        macro_rules! check {
            ($field:ident, $channel_name:literal) => {{
                if let Some(entry) = channels
                    .$field
                    .iter()
                    .find(|c| c.default_agent.as_deref() == Some(agent_name.as_str()))
                {
                    return Some(SenderContext {
                        channel: $channel_name.to_string(),
                        account_id: entry.account_id.clone(),
                        use_canonical_session: true,
                        ..Default::default()
                    });
                }
            }};
        }

        crate::for_each_channel_field!(check);

        None
    }

    /// Send an ephemeral "side question" to an agent (`/btw` command).
    ///
    /// The message is answered using the agent's system prompt and model, but in a
    /// **fresh temporary session** — no conversation history is loaded and the
    /// exchange is **not persisted** to the real session. This lets users ask quick
    /// throwaway questions without polluting the ongoing conversation context.
    pub async fn send_message_ephemeral(
        &self,
        agent_id: AgentId,
        message: &str,
        sender_context: Option<&SenderContext>,
    ) -> KernelResult<AgentLoopResult> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        if entry.state == AgentState::Suspended {
            tracing::debug!(agent_id = %agent_id, "Skipping ephemeral message to suspended agent");
            return Ok(AgentLoopResult::default());
        }

        // #4807: the pre-dispatch provider-budget gate was removed
        // from this path. Budget exhaustion is now signalled through
        // the shared `ProviderExhaustionStore` (flagged by
        // `MeteringEngine::flag_provider_budget_exhausted` when a
        // per-provider operator cap trips) and consumed by the LLM
        // fallback chain (`FallbackDriver` + `FallbackChain`), so an
        // exhausted primary provider falls over to a healthy slot
        // instead of refusing the whole call. Global `[budget]` caps
        // and per-agent quotas still apply via `reserve_global_budget`
        // / `check_quota_and_reserve` below — only the per-provider
        // gate that #4807 explicitly asked to remove is gone.

        // Ephemeral: no tools — prevents side effects (tool writes to memory/disk)
        let tools: Vec<librefang_types::tool::ToolDefinition> = vec![];
        let mut manifest = entry.manifest.clone();

        // Reuse the prompt-builder to get a proper system prompt
        {
            let mcp_tool_count = self.mcp.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
            let shared_id = shared_memory_agent_id();
            // Mirror the peer-scoping that `memory_store` applies on write
            // (#4923). When the ephemeral call carries a sender (channel
            // bridge, /btw with auth context), we read the same peer-scoped
            // key the agent wrote on a non-ephemeral turn — so the agent
            // doesn't re-ask the user's name on `/btw` queries.
            let peer_id = sender_context
                .map(|s| s.user_id.as_str())
                .filter(|s| !s.is_empty());
            // peer_scoped_key now rejects colon-bearing / empty peer_ids
            // (#5119); on a malformed peer_id we skip the user_name lookup
            // with a WARN so prompt assembly stays best-effort rather than
            // failing the turn.
            let user_name = match peer_scoped_key("user_name", peer_id) {
                Ok(user_name_key) => self
                    .memory
                    .substrate
                    .structured_get(shared_id, &user_name_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_str().map(String::from)),
                Err(e) => {
                    tracing::warn!(
                        peer_id = ?peer_id,
                        error = %e,
                        "skipping user_name lookup: invalid peer_id namespace"
                    );
                    None
                }
            };

            let peer_agents: Vec<(String, String, String)> =
                self.agents.registry.peer_agents_summary();

            let ws_meta = manifest
                .workspace
                .as_ref()
                .map(|w| self.cached_workspace_metadata(w, manifest.autonomous.is_some()));

            let agent_id_str = agent_id.0.to_string();
            // One pass over `tools` produces both the name list (for the
            // hook payload + `PromptContext::granted_tools`) and the
            // description hint map (for `PromptContext::granted_tool_hints`).
            // Avoids three separate walks per send (#4805 review).
            let (granted_tool_names, granted_tool_hints) =
                librefang_runtime::prompt_builder::collect_granted_tool_names_and_hints(&tools);
            let hook_ctx = librefang_runtime::hooks::HookContext {
                agent_name: &manifest.name,
                agent_id: agent_id_str.as_str(),
                event: librefang_types::agent::HookEvent::BeforePromptBuild,
                data: serde_json::json!({
                    "phase": "build",
                    "call_site": "ephemeral",
                    "user_message": message,
                    "is_subagent": false,
                    "granted_tools": granted_tool_names,
                }),
            };
            let dynamic_sections = self.governance.hooks.collect_prompt_sections(&hook_ctx);

            // Re-read context.md per turn by default so external writers
            // (cron jobs, integrations) reach the LLM on the next message.
            // Opt out via `cache_context = true` on the manifest.
            // Pre-loaded off the runtime worker (tokio::fs) so the struct
            // literal below stays sync — see #3579.
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
                granted_tools: granted_tool_names,
                granted_tool_hints,
                recalled_memories: vec![],
                skill_summary: String::new(),
                skill_count: 0,
                skill_prompt_context: String::new(),
                skill_config_section: String::new(),
                mcp_summary: if mcp_tool_count > 0 && !manifest.mcp_disabled {
                    self.build_mcp_summary(&manifest.mcp_servers)
                } else {
                    String::new()
                },
                workspace_path: manifest.workspace.as_ref().map(|p| p.display().to_string()),
                soul_md: ws_meta.as_ref().and_then(|m| m.soul_md.clone()),
                user_md: ws_meta.as_ref().and_then(|m| m.user_md.clone()),
                memory_md: ws_meta.as_ref().and_then(|m| m.memory_md.clone()),
                canonical_context: None,
                user_name,
                channel_type: None,
                sender_display_name: None,
                sender_user_id: None,
                is_subagent: false,
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
                is_group: false,
                was_mentioned: false,
                context_md,
                dynamic_sections,
            };
            manifest.model.system_prompt =
                librefang_runtime::prompt_builder::build_system_prompt(&prompt_ctx);
        }

        let driver = self.resolve_driver(&manifest)?;

        let ctx_window = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model(&manifest.model.model)
                .map(|m| m.context_window as usize)
                .filter(|w| *w > 0)
        });

        // Inject model_supports_tools for auto web search augmentation.
        // Refs #4745: honour user-configured per-model capability overrides
        // here too — when a user has manually flipped `supports_tools` for a
        // model whose catalog metadata was wrong, the auto-augmentation path
        // must respect that override or the runtime behaviour diverges from
        // what the dashboard shows.
        if let Some(supports) = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model(&manifest.model.model)
                .map(|m| cat.effective_capabilities(m).supports_tools)
        }) {
            manifest.metadata.insert(
                "model_supports_tools".to_string(),
                serde_json::Value::Bool(supports),
            );
        }

        // Create a temporary in-memory session (empty — no history loaded)
        let ephemeral_session_id = SessionId::new();
        let mut ephemeral_session = librefang_memory::session::Session {
            id: ephemeral_session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: Some("ephemeral /btw".to_string()),
            model_override: None,
            messages_generation: 0,
            last_repaired_generation: None,
        };

        info!(
            agent = %entry.name,
            agent_id = %agent_id,
            "Ephemeral /btw message — using temporary session (no history, no persistence)"
        );

        let start_time = std::time::Instant::now();
        let result = run_agent_loop(
            &manifest,
            message,
            &mut ephemeral_session,
            &self.memory.substrate,
            driver,
            &tools,
            None, // no kernel handle — keep side questions simple
            None, // no skills
            None, // no MCP
            None, // no web
            None, // no browser
            None, // no embeddings
            manifest.workspace.as_deref(),
            None, // no phase callback
            None, // no media engine
            None, // no media drivers
            None, // no TTS
            None, // no docker
            None, // no hooks
            ctx_window,
            None, // no process manager
            None, // no checkpoint manager (ephemeral /btw — side questions only)
            None, // no process registry
            None, // no content blocks
            None, // no proactive memory
            None, // no context engine
            None, // no pending messages
            &librefang_runtime::agent_loop::LoopOptions {
                is_fork: false,
                incognito: false,
                allowed_tools: None,
                interrupt: Some(librefang_runtime::interrupt::SessionInterrupt::new()),
                max_iterations: self.config.load().agent_max_iterations,
                max_history_messages: self.config.load().max_history_messages,
                aux_client: Some(self.llm.aux_client.load_full()),
                parent_session_id: None,
                // Ephemeral /btw sessions start with empty history so no
                // stale tool results can accumulate — `None` uses compiled
                // defaults, which is fine.  The fold helper's length
                // fast-path exits on short histories.  Layer 2 / Layer 3
                // byte-budget enforcement (defaults: 16 KB per result,
                // 50 KB per turn) still applies — only fold no-ops here.
                tool_results_config: None,
                // #4976: ephemeral /btw uses an empty session — the
                // compressor will never fire here. Leave at None so the
                // loop falls back to compiled defaults.
                compaction_config: None,
                // Ephemeral /btw also starts empty — gateway pass would
                // no-op (under threshold) so we skip it explicitly.
                gateway_compression: None,
            },
        )
        .await
        .map_err(KernelError::LibreFang)?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // NOTE: We intentionally do NOT save the ephemeral session, do NOT
        // update canonical memory, do NOT write JSONL mirror, and do NOT
        // append to the daily memory log. The side question is truly ephemeral.

        // Atomically check quotas and record metering so cost tracking stays
        // accurate (prevents TOCTOU race on concurrent ephemeral requests)
        let model = &manifest.model.model;
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &self.llm.model_catalog.load(),
            model,
            result.total_usage.input_tokens,
            result.total_usage.output_tokens,
            result.total_usage.cache_read_input_tokens,
            result.total_usage.cache_creation_input_tokens,
        );
        // Ephemeral side-questions have no sender context — no user/channel
        // attribution to record. Per-user budget rollup will skip these.
        // session_id is also None: ephemerals run on a throwaway session
        // that is not persisted in the sessions table.
        //
        // #4807 review nit 10: honour `actual_provider` so a chain
        // fail-over bills the slot that did the work.
        let billed_provider = result
            .actual_provider
            .clone()
            .unwrap_or_else(|| manifest.model.provider.clone());
        let usage_record = librefang_memory::usage::UsageRecord {
            agent_id,
            provider: billed_provider,
            model: model.clone(),
            input_tokens: result.total_usage.input_tokens,
            output_tokens: result.total_usage.output_tokens,
            cost_usd: cost,
            tool_calls: result.decision_traces.len() as u32,
            latency_ms,
            user_id: None,
            channel: None,
            session_id: None,
        };
        if let Err(e) = self.metering.engine.check_all_and_record(
            &usage_record,
            &manifest.resources,
            &self.current_budget(),
        ) {
            tracing::warn!(
                agent_id = %agent_id,
                error = %e,
                "Post-call quota check failed (ephemeral); recording usage anyway"
            );
            let _ = self.metering.engine.record(&usage_record);
        }

        // Record experiment metrics if running an experiment (kernel has cost info)
        if let Some(ref ctx) = result.experiment_context {
            let has_content = !result.response.trim().is_empty();
            let no_tool_errors = result.iterations > 0;
            let success = has_content && no_tool_errors;
            let _ = self.record_experiment_request(
                &ctx.experiment_id.to_string(),
                &ctx.variant_id.to_string(),
                latency_ms,
                cost,
                success,
            );
        }

        let mut result = result;
        result.cost_usd = if cost > 0.0 { Some(cost) } else { None };
        result.latency_ms = latency_ms;

        Ok(result)
    }

    /// Internal: send a message with all optional parameters (content blocks + sender context).
    ///
    /// This is the unified entry point for all message dispatch. When `sender_context`
    /// is provided, the agent's system prompt includes the sender's identity (channel,
    /// user ID, display name) so the agent knows who is talking and from where.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn send_message_full(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        content_blocks: Option<Vec<librefang_types::message::ContentBlock>>,
        sender_context: Option<&SenderContext>,
        session_mode_override: Option<librefang_types::agent::SessionMode>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<AgentLoopResult> {
        self.send_message_full_with_upstream(
            agent_id,
            message,
            kernel_handle,
            content_blocks,
            sender_context,
            session_mode_override,
            thinking_override,
            session_id_override,
            None,
            false,
        )
        .await
    }

    /// Same as [`Self::send_message_full`] but threads an optional upstream
    /// [`SessionInterrupt`] so a parent session's `/stop` can cascade into
    /// this subagent's loop (issue #3044). Used by `tool_agent_send` when
    /// the caller agent's own interrupt should gate the callee.
    #[allow(clippy::too_many_arguments)]
    async fn send_message_full_with_upstream(
        &self,
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
        // Briefly acquire the config reload barrier to ensure we observe a
        // fully-applied hot-reload (config swap + side effects are atomic
        // under the writer's guard). We drop the guard immediately after —
        // `self.config` is an `ArcSwap`, so any subsequent `.load()` already
        // returns a consistent snapshot. Holding the read guard across the
        // entire LLM call (multi-minute streams) was a bug (#3564):
        // `tokio::sync::RwLock` is write-preferring, so a single
        // `/api/config/reload` froze every new request behind the queued
        // writer until the slowest in-flight stream completed.
        {
            let _config_guard = self.config_reload_lock.read().await;
        }

        let agent_id = self
            .resolve_assistant_target(agent_id, message, sender_context)
            .await?;

        // When the caller supplies an explicit session_id, scope the lock to that
        // session so concurrent messages to *different* sessions of the same agent
        // are not serialized against each other (multi-tab / multi-session UIs).
        // Without an override, fall back to the per-agent lock to preserve the
        // existing serialization guarantee for single-session agents.
        let lock = if let Some(sid) = session_id_override {
            self.agents
                .session_msg_locks
                .entry(sid)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        } else {
            self.agents
                .agent_msg_locks
                .entry(agent_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Pre-call global budget reservation (#3616). Estimate cost from
        // the model's max output tokens and reserve it on the in-memory
        // ledger so concurrent trigger fires can't all observe the same
        // pre-call total and collectively overshoot the cap. Settled
        // (after success) or released (on failure / suspended target)
        // alongside the existing token reservation below.
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // #4807: the pre-dispatch provider-budget gate was removed
        // from this path. Budget exhaustion is signalled through the
        // shared `ProviderExhaustionStore` and consumed by the LLM
        // fallback chain so an exhausted primary provider fails over
        // to a healthy slot. See the ephemeral-path explanation
        // above for the full rationale.

        let estimated_usd = {
            // Best-effort pre-call estimate: model.max_tokens worth of
            // output, plus a conservative input estimate equal to the
            // same token count. Real cost is settled later via
            // `check_all_and_record`; this only sizes the in-memory hold.
            let max_out = entry.manifest.model.max_tokens as u64;
            let est_in = max_out;
            {
                let catalog = self.llm.model_catalog.load();
                MeteringEngine::estimate_cost_with_catalog(
                    &catalog,
                    &entry.manifest.model.model,
                    est_in,
                    max_out,
                    0,
                    0,
                )
            }
        };
        let usd_reservation = self
            .metering
            .engine
            .reserve_global_budget(&self.current_budget(), estimated_usd)
            .map_err(KernelError::LibreFang)?;

        // Enforce quota on the effective target agent (after routing).
        // Use check_quota_and_reserve so the estimated token budget is
        // pre-charged inside the same DashMap write-lock, closing the TOCTOU
        // race where N concurrent callers all pass the check before any of
        // them calls record_usage (#3736).
        let estimated_tokens = entry.manifest.model.max_tokens as u64;
        let token_reservation = match self
            .agents
            .scheduler
            .check_quota_and_reserve(agent_id, estimated_tokens)
        {
            Ok(r) => r,
            Err(e) => {
                // Roll back the USD reservation — the call never dispatched.
                usd_reservation.release();
                return Err(KernelError::LibreFang(e));
            }
        };

        // Skip suspended agents — cron/triggers should not dispatch to them
        if entry.state == AgentState::Suspended {
            tracing::debug!(agent_id = %agent_id, "Skipping message to suspended agent");
            // No LLM call is made; release reservations without inflating
            // llm_calls or the burst window.
            self.agents
                .scheduler
                .release_reservation(agent_id, token_reservation);
            usd_reservation.release();
            return Ok(AgentLoopResult::default());
        }

        // Resolve the effective session id up front for the LLM path so we
        // can include it in supervisor / failure logs below, then pass it
        // back down as the explicit override so the kernel and the log line
        // agree on the id even when `session_mode = "new"` would otherwise
        // mint a fresh session inside `execute_llm_agent`.
        let resolved_session_id: Option<SessionId> = resolve_dispatch_session_id(
            &entry.manifest.module,
            agent_id,
            entry.session_id,
            entry.manifest.session_mode,
            sender_context,
            session_mode_override,
            session_id_override,
        );

        // Dispatch based on module type
        let result = match entry.manifest.module.as_str() {
            module if module.starts_with("wasm:") => {
                self.execute_wasm_agent(&entry, message, kernel_handle)
                    .await
            }
            module if module.starts_with("python:") => {
                self.execute_python_agent(&entry, agent_id, message).await
            }
            _ => {
                // Default: LLM agent loop (builtin:chat or any unrecognized module)
                self.execute_llm_agent(
                    &entry,
                    agent_id,
                    message,
                    kernel_handle,
                    content_blocks,
                    sender_context,
                    session_mode_override,
                    thinking_override,
                    resolved_session_id.or(session_id_override),
                    upstream_interrupt,
                    incognito,
                )
                .await
            }
        };

        match result {
            Ok(result) => {
                // Settle the pre-charged token reservation with actual
                // usage. The USD reservation is settled here too — actual
                // cost will be recorded by `check_all_and_record` further
                // down the call path; releasing the in-memory hold lets
                // the next reservation pass see a consistent total.
                self.agents.scheduler.settle_reservation(
                    agent_id,
                    token_reservation,
                    &result.total_usage,
                );
                usd_reservation.settle();
                // Record tool calls for rate limiting
                let tool_count = result.decision_traces.len() as u32;
                self.agents
                    .scheduler
                    .record_tool_calls(agent_id, tool_count);

                // Update last active time
                let _ = self
                    .agents
                    .registry
                    .set_state(agent_id, AgentState::Running);

                // Store decision traces for API retrieval
                if !result.decision_traces.is_empty() {
                    self.agents
                        .decision_traces
                        .insert(agent_id, result.decision_traces.clone());
                }

                if result.provider_not_configured {
                    if !self
                        .provider_unconfigured_logged
                        .swap(true, std::sync::atomic::Ordering::Relaxed)
                    {
                        self.metering.audit_log.record(
                            agent_id.to_string(),
                            librefang_runtime::audit::AuditAction::AgentMessage,
                            "agent loop skipped",
                            "No LLM provider configured — configure via dashboard settings",
                        );
                    }
                    return Ok(result);
                }

                // SECURITY: Record successful message in audit trail
                self.metering.audit_log.record(
                    agent_id.to_string(),
                    librefang_runtime::audit::AuditAction::AgentMessage,
                    format!(
                        "tokens_in={}, tokens_out={}",
                        result.total_usage.input_tokens, result.total_usage.output_tokens
                    ),
                    "ok",
                );

                // Push task_completed notification for autonomous (hand) agents
                if let Some(entry) = self.agents.registry.get(agent_id) {
                    let is_autonomous = entry.tags.iter().any(|t| t.starts_with("hand:"))
                        || entry.manifest.autonomous.is_some();
                    if is_autonomous {
                        let name = &entry.name;
                        let msg = format!(
                            "Agent \"{}\" completed task (in={}, out={} tokens)",
                            name, result.total_usage.input_tokens, result.total_usage.output_tokens,
                        );
                        self.push_notification(
                            &agent_id.to_string(),
                            "task_completed",
                            &msg,
                            resolved_session_id.as_ref(),
                        )
                        .await;
                    }
                }

                // Skill evolution: check if any skill_evolve_* tools were used
                // and hot-reload the registry so new/updated skills take effect
                // immediately for subsequent messages.
                let used_evolution_tool = result
                    .decision_traces
                    .iter()
                    .any(|t| t.tool_name.starts_with("skill_evolve_"));
                if used_evolution_tool {
                    tracing::info!(
                        agent_id = %agent_id,
                        "Agent used skill evolution tools — reloading skill registry"
                    );
                    self.reload_skills();
                }

                // Background skill review: when the agent used enough tool calls
                // to suggest a non-trivial workflow, spawn a background LLM call
                // to evaluate whether the approach should be saved as a skill.
                // Runs AFTER the response is delivered so it never competes with
                // the user's task for model attention.
                // Cooldown: per-agent, at most one review every SKILL_REVIEW_COOLDOWN_SECS.
                let now_epoch = chrono::Utc::now().timestamp();
                let agent_id_str = agent_id.to_string();
                // Pre-claim gate 0a: per-agent opt-out. A2A worker agents
                // and any agent where trigger responsiveness matters more
                // than automatic skill distillation can set
                // `auto_evolve = false` in agent.toml to skip the review
                // entirely — no LLM call, no semaphore, no cooldown slot.
                if !entry.manifest.auto_evolve {
                    tracing::debug!(
                        agent_id = %agent_id,
                        "Skipping background skill review — auto_evolve disabled for this agent"
                    );
                }
                // Pre-claim gate 0b: Stable mode / frozen registry. Skip
                // spawning a review task entirely when the operator
                // chose a no-skill-mutations posture — the review would
                // write to disk and the reload_skills() call afterwards
                // would silently no-op, so all we'd accomplish is to
                // bill the default driver for nothing.
                let registry_frozen = self
                    .skills
                    .skill_registry
                    .read()
                    .map(|r| r.is_frozen())
                    .unwrap_or(false);
                // Pre-claim gate 1: eligibility. Only consider claiming
                // the cooldown slot if this loop actually suggested a
                // review AND the agent didn't already evolve a skill
                // AND the registry isn't frozen AND auto_evolve is on.
                let eligible = result.skill_evolution_suggested
                    && !used_evolution_tool
                    && !registry_frozen
                    && entry.manifest.auto_evolve;
                // Pre-claim gate 2: budget. Background reviews are
                // optional work — if the global budget is exhausted we
                // want to skip WITHOUT burning the 5-minute cooldown
                // slot, so that the next message (after any budget top-up
                // or rollover) can re-try immediately. Checking before
                // claim is the whole point here.
                let budget_ok = if eligible {
                    match self
                        .metering
                        .engine
                        .check_global_budget(&self.current_budget())
                    {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::debug!(
                                agent_id = %agent_id,
                                error = %e,
                                "Skipping background skill review — global budget exhausted"
                            );
                            false
                        }
                    }
                } else {
                    false
                };
                // Semaphore-first: if no permit is available, drop the
                // review WITHOUT burning the 5-min cooldown — so the next
                // loop (after congestion clears) can retry. Previously
                // the cooldown was claimed BEFORE the permit check,
                // silently starving agents that happened to finish during
                // a review stampede.
                let permit_opt = if budget_ok {
                    match self
                        .skills
                        .skill_review_concurrency
                        .clone()
                        .try_acquire_owned()
                    {
                        Ok(p) => Some(p),
                        Err(_) => {
                            tracing::info!(
                                agent_id = %agent_id,
                                "Skipping background skill review — global concurrency limit reached"
                            );
                            None
                        }
                    }
                } else {
                    None
                };
                // Atomic cooldown claim: only after we have a permit. The
                // and_modify/or_insert CAS closes the check-then-insert
                // race between concurrent agent loops for the same agent id.
                let claimed = permit_opt.is_some()
                    && self.try_claim_skill_review_slot(&agent_id_str, now_epoch);
                if claimed {
                    let permit = permit_opt.expect("permit was acquired before claim");
                    // Prefer the driver the agent's own turn resolved to.
                    // When an agent is pinned to a provider the global
                    // default isn't configured for (or vice versa), using
                    // `self.llm.default_driver` meant reviews failed with
                    // "unknown provider" while the task itself had
                    // succeeded — so complex workflows from those agents
                    // never got distilled into skills. Fall back to the
                    // default only if manifest resolution fails.
                    let driver = self
                        .resolve_driver(&entry.manifest)
                        .unwrap_or_else(|_| self.llm.default_driver.clone());
                    let skills_dir = self.home_dir_boot.join("skills");
                    let trace_summary = Self::summarize_traces_for_review(&result.decision_traces);
                    let response_summary = result.response.chars().take(2000).collect::<String>();
                    let kernel_weak = self.self_handle.get().cloned();
                    let audit_log = self.metering.audit_log.clone();
                    let agent_id_for_task = agent_id_str.clone();
                    // Cost-attribution model: use the agent's own model
                    // so review spend rolls up under the same line the
                    // main turn did (matches the driver choice above).
                    // Falls back to the global default when the agent
                    // didn't pin a provider/model pair.
                    let default_model = if entry.manifest.model.provider.is_empty()
                        || entry.manifest.model.model.is_empty()
                    {
                        self.default_model()
                    } else {
                        librefang_types::config::DefaultModelConfig {
                            provider: entry.manifest.model.provider.clone(),
                            model: entry.manifest.model.model.clone(),
                            api_key_env: entry
                                .manifest
                                .model
                                .api_key_env
                                .clone()
                                .unwrap_or_default(),
                            base_url: entry.manifest.model.base_url.clone(),
                            ..self.default_model()
                        }
                    };
                    let review_agent_id = agent_id;
                    let audit_log_success = audit_log.clone();
                    let agent_id_for_success = agent_id_str.clone();
                    let review_handle = spawn_logged("auto_memorize", async move {
                        // Move the permit into the task so it's released
                        // on task exit. Binding it to `_permit` keeps
                        // clippy happy (dropped at end of scope).
                        let _permit = permit;
                        // Retry only on LLM-call-boundary (network/timeout/
                        // rate-limit) errors. Post-parse failures (malformed
                        // JSON, missing fields, security_blocked) are
                        // classified Permanent and break out immediately —
                        // a retry would issue a FRESH LLM call with the
                        // same prompt, potentially applying a DIFFERENT
                        // update on each attempt (non-idempotent), which
                        // was the pre-fix behavior.
                        const MAX_ATTEMPTS: u32 = 3;
                        let mut last_err = String::new();
                        let mut attempts_used = 0u32;
                        for attempt in 0..MAX_ATTEMPTS {
                            attempts_used = attempt + 1;
                            if attempt > 0 {
                                tokio::time::sleep(std::time::Duration::from_secs(
                                    2u64.pow(attempt),
                                ))
                                .await;
                            }
                            match Self::background_skill_review(
                                driver.clone(),
                                &skills_dir,
                                &trace_summary,
                                &response_summary,
                                kernel_weak.clone(),
                                review_agent_id,
                                &default_model,
                            )
                            .await
                            {
                                Ok(()) => {
                                    last_err.clear();
                                    audit_log_success.record(
                                        agent_id_for_success.clone(),
                                        librefang_runtime::audit::AuditAction::AgentMessage,
                                        "skill_review",
                                        format!("completed after {attempts_used} attempt(s)"),
                                    );
                                    break;
                                }
                                Err(ReviewError::Transient(e)) => {
                                    tracing::debug!(
                                        attempt = attempts_used,
                                        error = %e,
                                        "Background skill review attempt failed (transient, will retry)"
                                    );
                                    last_err = e;
                                }
                                Err(ReviewError::Permanent(e)) => {
                                    tracing::debug!(
                                        attempt = attempts_used,
                                        error = %e,
                                        "Background skill review attempt failed (permanent, not retrying)"
                                    );
                                    last_err = e;
                                    break;
                                }
                            }
                        }
                        if !last_err.is_empty() {
                            tracing::warn!(
                                agent_id = %agent_id_for_task,
                                attempts = attempts_used,
                                error = %last_err,
                                "Background skill review failed"
                            );
                            audit_log.record(
                                agent_id_for_task,
                                librefang_runtime::audit::AuditAction::AgentMessage,
                                "skill_review",
                                format!("failed after {attempts_used} attempt(s): {last_err}"),
                            );
                        }
                    });
                    // Track the review task so kill_agent can abort it and
                    // release its semaphore permit promptly (#3705).
                    self.register_agent_watcher(agent_id, review_handle);
                }

                Ok(result)
            }
            Err(e) => {
                // Release the pre-charged token + USD reservations — the
                // agent loop failed before completing, no usage to settle.
                self.agents
                    .scheduler
                    .release_reservation(agent_id, token_reservation);
                usd_reservation.release();

                // SECURITY: Record failed message in audit trail
                self.metering.audit_log.record(
                    agent_id.to_string(),
                    librefang_runtime::audit::AuditAction::AgentMessage,
                    "agent loop failed",
                    format!("error: {e}"),
                );

                // Record the failure in supervisor for health reporting
                self.agents.supervisor.record_panic();
                let session_id_for_log = resolved_session_id
                    .map(|s| s.0.to_string())
                    .unwrap_or_else(|| "<none>".to_string());
                warn!(
                    agent_id = %agent_id,
                    session_id = %session_id_for_log,
                    error = %e,
                    "Agent loop failed — recorded in supervisor"
                );

                // Push failure notification to alert_channels
                let agent_name = self
                    .agents
                    .registry
                    .get(agent_id)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| agent_id.to_string());
                // Push notification — use "tool_failure" for the repeated-tool-failure
                // exit path so operators with tool_failure agent_rules get alerted.
                let (event_type, fail_msg) = match &e {
                    KernelError::LibreFang(LibreFangError::RepeatedToolFailures {
                        iterations,
                        error_count,
                    }) => (
                        "tool_failure",
                        format!(
                            "Agent \"{}\" exited after {} consecutive tool failures ({} errors in final iteration)",
                            agent_name, iterations, error_count
                        ),
                    ),
                    // Provider safety / content filter — distinct from generic
                    // task_failed so operators can route refusals separately (#3450).
                    KernelError::LibreFang(LibreFangError::ContentFiltered { message }) => (
                        "content_filtered",
                        format!(
                            "Agent \"{}\" response blocked by provider safety filter: {}",
                            agent_name,
                            message.chars().take(200).collect::<String>()
                        ),
                    ),
                    other => (
                        "task_failed",
                        format!(
                            "Agent \"{}\" loop failed: {}",
                            agent_name,
                            other.to_string().chars().take(200).collect::<String>()
                        ),
                    ),
                };
                self.push_notification(
                    &agent_id.to_string(),
                    event_type,
                    &fail_msg,
                    resolved_session_id.as_ref(),
                )
                .await;

                Err(e)
            }
        }
    }

    /// Send a message with LLM intent routing + streaming.
    ///
    /// When the target is the assistant, first classifies the message via a
    /// lightweight LLM call and routes to the appropriate specialist.
    pub async fn send_message_streaming_with_routing(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_resolved(agent_id, message, handle, None, None, None)
            .await
    }

    /// Streaming variant with an explicit session ID override.
    ///
    /// Used by the HTTP `/message/stream` endpoint when the caller supplies a
    /// `session_id` in the request body (multi-tab / multi-session UIs).
    pub async fn send_message_streaming_with_routing_and_session_override(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_resolved(
            agent_id,
            message,
            handle,
            None,
            None,
            session_id_override,
        )
        .await
    }

    /// Streaming variant of [`Self::send_message_with_incognito`].
    ///
    /// Runs a normal streaming agent turn but with `incognito: true` in
    /// `LoopOptions` so session messages and proactive-memory writes are
    /// suppressed while memory reads remain full-access.
    pub async fn send_message_streaming_with_incognito(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        let effective_id = self
            .resolve_assistant_target(agent_id, message, None)
            .await?;
        let session_interrupt = librefang_runtime::interrupt::SessionInterrupt::new();
        let loop_opts = librefang_runtime::agent_loop::LoopOptions {
            is_fork: false,
            incognito,
            allowed_tools: None,
            interrupt: Some(session_interrupt),
            max_iterations: self.config.load().agent_max_iterations,
            max_history_messages: self.config.load().max_history_messages,
            aux_client: Some(self.llm.aux_client.load_full()),
            parent_session_id: None,
            tool_results_config: Some(self.config.load().tool_results.clone()),
            // #4976: per-agent compaction overrides are resolved inside
            // `send_message_streaming_with_sender_and_opts` once the
            // agent registry has been consulted — leave as None here.
            compaction_config: None,
            gateway_compression: Some(self.config.load().gateway_compression.clone()),
        };
        self.send_message_streaming_with_sender_and_opts(
            effective_id,
            message,
            handle,
            None,
            None,
            session_id_override,
            loop_opts,
        )
    }

    /// Sender-aware streaming entry point for channel bridges.
    pub async fn send_message_streaming_with_sender_context_and_routing(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender: &SenderContext,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_resolved(agent_id, message, handle, Some(sender), None, None)
            .await
    }

    /// Streaming entry point with per-call deep-thinking override.
    ///
    /// Used by the WebUI chat route so users can flip deep thinking on/off
    /// per message from the UI.
    pub async fn send_message_streaming_with_sender_context_routing_and_thinking(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender: &SenderContext,
        thinking_override: Option<bool>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_resolved(
            agent_id,
            message,
            handle,
            Some(sender),
            thinking_override,
            None,
        )
        .await
    }

    /// Streaming entry point that combines a sender context with a per-request
    /// `session_id_override` (multi-tab WebSocket UIs, issue #2959). The
    /// override wins over channel-derived session resolution. When `None`,
    /// behavior is identical to
    /// [`Self::send_message_streaming_with_sender_context_routing_and_thinking`].
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message_streaming_with_sender_context_routing_thinking_and_session(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
        sender: &SenderContext,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_resolved(
            agent_id,
            message,
            handle,
            Some(sender),
            thinking_override,
            session_id_override,
        )
        .await
    }

    /// Send a message to an agent with streaming responses.
    ///
    /// Returns a receiver for incremental `StreamEvent`s and a `JoinHandle`
    /// that resolves to the final `AgentLoopResult`. The caller reads stream
    /// events while the agent loop runs, then awaits the handle for final stats.
    ///
    /// WASM and Python agents don't support true streaming — they execute
    /// synchronously and emit a single `TextDelta` + `ContentComplete` pair.
    pub fn send_message_streaming(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let handle = kernel_handle.unwrap_or_else(|| self.kernel_handle());
        self.send_message_streaming_with_sender(agent_id, message, handle, None, None)
    }

    /// Run a *derivative* (forked) turn for an agent using the canonical
    /// session's messages as a cache-aligned prefix. Used by auto-dream and
    /// any future post-turn consumer that wants to fire an LLM call on top
    /// of the agent's context without persisting into its history.
    ///
    /// Semantics vs. `send_message_streaming`:
    ///
    /// - **Does not persist** messages added by the fork turn. The session
    ///   is shared with canonical at read time but writes stay in memory.
    /// - **Does not trigger AgentLoopEnd consumers that filter on
    ///   `is_fork`** — notably auto-dream's own hook skips fork turns, so
    ///   a dream won't trigger a nested dream (the file lock would also
    ///   prevent it, but this is cheaper).
    /// - **Enforces a runtime tool allowlist** via `allowed_tools`. The
    ///   list is NOT applied to the request schema sent to the provider
    ///   (that would break cache alignment) — it's enforced at tool
    ///   execute time with a synthetic error returned to the model.
    ///
    /// Rejects WASM / Python agents with `Err` — the fork mode only
    /// makes sense for LLM-backed agents.
    pub fn run_forked_agent_streaming(
        self: &Arc<Self>,
        agent_id: AgentId,
        fork_prompt: &str,
        allowed_tools: Option<Vec<String>>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;
        if entry.manifest.module.starts_with("wasm:")
            || entry.manifest.module.starts_with("python:")
        {
            return Err(KernelError::LibreFang(LibreFangError::Internal(
                "run_forked_agent_streaming is only supported for LLM agents".to_string(),
            )));
        }
        // Inherit the parent turn's interrupt when one exists so a caller
        // invoking `stop_agent_run(agent_id)` on the parent also cancels
        // tools that are in-flight inside this fork (#2939). Both handles
        // wrap the same `Arc<AtomicBool>`, so `cancel()` on either one is
        // observed by both. When no parent is running (e.g. auto_memorize
        // fires from an idle agent), fall back to a fresh interrupt so the
        // fork still has a cancellation primitive for its own tools.
        //
        // Post-#3172 the interrupt map is keyed by (agent, session); the
        // fork doesn't yet know which parent session is driving it, so we
        // pick any in-flight one for the same agent. With concurrent
        // loops the choice is best-effort, but cancellation chains via the
        // shared Arc<AtomicBool> still work — `stop_agent_run(agent_id)`
        // fans out across all sessions, so no matter which entry we
        // borrowed from, the cascade reaches this fork.
        //
        // We also snapshot the parent session id from the same lookup so
        // the kernel's session resolver can pin the fork to the parent
        // turn's session for prompt-cache alignment, instead of
        // re-reading `entry.session_id` later (which is mutable by
        // `switch_agent_session`, producing a TOCTOU race — #4291). When
        // no parent loop is in flight, fall back to the registry pointer
        // — the only signal we have, and the fork will create/resume
        // that session on its own.
        let (parent_session_id, interrupt) =
            match self.any_session_interrupt_with_id_for_agent(agent_id) {
                Some((sid, intr)) => (sid, intr),
                None => (
                    entry.session_id,
                    librefang_runtime::interrupt::SessionInterrupt::default(),
                ),
            };
        let loop_opts = librefang_runtime::agent_loop::LoopOptions {
            is_fork: true,
            incognito: false,
            allowed_tools,
            interrupt: Some(interrupt),
            max_iterations: self.config.load().agent_max_iterations,
            max_history_messages: self.config.load().max_history_messages,
            aux_client: Some(self.llm.aux_client.load_full()),
            parent_session_id: Some(parent_session_id),
            tool_results_config: Some(self.config.load().tool_results.clone()),
            // #4976: per-agent compaction overrides are resolved inside
            // `send_message_streaming_with_sender_and_opts` once the
            // agent registry has been consulted — leave as None here.
            // Forks inherit the parent agent's compaction policy (the
            // forked agent_id is identical to the parent's at this
            // layer; allowed_tools is the only fork-specific override).
            compaction_config: None,
            gateway_compression: Some(self.config.load().gateway_compression.clone()),
        };
        // INVARIANT: forks must use the canonical session so the parent turn's
        // prompt-cache prefix is reused. Do NOT pass a `session_id_override`
        // here — it would win over the fork branch in
        // `send_message_streaming_with_sender_and_opts`'s session resolver and
        // break cache alignment (see issue #2959 for the override semantics).
        self.send_message_streaming_with_sender_and_opts(
            agent_id,
            fork_prompt,
            self.kernel_handle(),
            None, // no sender context — fork uses the canonical session
            None, // no thinking override
            None, // forks MUST stay on canonical — see invariant above
            loop_opts,
        )
    }

    fn send_message_streaming_with_sender(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        self.send_message_streaming_with_sender_and_session(
            agent_id,
            message,
            kernel_handle,
            sender_context,
            thinking_override,
            None,
        )
    }

    pub(crate) fn send_message_streaming_with_sender_and_session(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        // TODO(#3044): the streaming entry does not yet accept an upstream
        // interrupt, so any subagent invoked through a streaming path (rather
        // than `tool_agent_send` → `send_message_as`) will not receive parent
        // /stop cascade. All inter-agent dispatch today goes through the
        // non-streaming `send_message_as`, so this is latent — but the next
        // caller that adds streaming subagent dispatch must extend the
        // cascade here.
        // Construct the interrupt here; the registration into
        // `session_interrupts` happens inside
        // `send_message_streaming_with_sender_and_opts` once
        // `effective_session_id` has been resolved (the map is keyed by
        // `(agent, session)` post-#3172 and the session id is not yet known
        // at this layer).
        let session_interrupt = librefang_runtime::interrupt::SessionInterrupt::new();
        let loop_opts = librefang_runtime::agent_loop::LoopOptions {
            is_fork: false,
            incognito: false,
            allowed_tools: None,
            interrupt: Some(session_interrupt),
            max_iterations: self.config.load().agent_max_iterations,
            max_history_messages: self.config.load().max_history_messages,
            aux_client: Some(self.llm.aux_client.load_full()),
            parent_session_id: None,
            tool_results_config: Some(self.config.load().tool_results.clone()),
            // #4976: resolved inside the `_with_opts` callee once the
            // registry has been consulted for this agent's manifest.
            compaction_config: None,
            gateway_compression: Some(self.config.load().gateway_compression.clone()),
        };
        self.send_message_streaming_with_sender_and_opts(
            agent_id,
            message,
            kernel_handle,
            sender_context,
            thinking_override,
            session_id_override,
            loop_opts,
        )
    }

    /// Internal: same as [`Self::send_message_streaming_with_sender`] but
    /// accepts a pre-built [`LoopOptions`]. `run_forked_agent_streaming`
    /// passes `is_fork = true` + an `allowed_tools` filter so the spawned
    /// agent_loop knows to skip session-saving and enforce the runtime
    /// tool allowlist. All public streaming entry points above go through
    /// this with the default `LoopOptions` (a normal main turn).
    #[allow(clippy::too_many_arguments)]
    fn send_message_streaming_with_sender_and_opts(
        self: &Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Arc<dyn KernelHandle>,
        sender_context: Option<&SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
        mut loop_opts: librefang_runtime::agent_loop::LoopOptions,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<StreamEvent>,
        tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    )> {
        // Try to acquire config reload barrier (non-blocking — this is a sync fn).
        // If a reload is in progress we proceed without the guard.
        let _config_guard = self.config_reload_lock.try_read();
        let cfg = self.config.load();

        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // #4976: resolve per-agent [compaction] overrides against the
        // kernel-global config and stash the merged snapshot on
        // `loop_opts` so the in-loop `ContextCompressor` builder picks
        // up the agent's keep_recent / max_summary_tokens /
        // token_threshold_ratio. Callers that already populated this
        // field (forks, future overrides) win.
        if loop_opts.compaction_config.is_none() {
            let merged = match entry.manifest.compaction.as_ref() {
                Some(o) if !o.is_empty() => o.resolve(&cfg.compaction),
                _ => cfg.compaction.clone(),
            };
            loop_opts.compaction_config = Some(merged);
        }

        // #4807: the pre-dispatch provider-budget gate was removed
        // from this path. Budget exhaustion is signalled through the
        // shared `ProviderExhaustionStore` and consumed by the LLM
        // fallback chain so an exhausted primary provider fails over
        // to a healthy slot. See the ephemeral-path explanation
        // above for the full rationale.

        // Pre-charge the estimated token budget atomically to prevent the
        // TOCTOU race (#3736).  The reservation is settled inside the spawned
        // task after the LLM call completes.
        let estimated_tokens = entry.manifest.model.max_tokens as u64;
        let token_reservation = self
            .agents
            .scheduler
            .check_quota_and_reserve(agent_id, estimated_tokens)
            .map_err(KernelError::LibreFang)?;

        let is_wasm = entry.manifest.module.starts_with("wasm:");
        let is_python = entry.manifest.module.starts_with("python:");

        // Non-LLM modules: execute non-streaming and emit results as stream events
        if is_wasm || is_python {
            // Fan out to the session hub so attached clients see the
            // synthesized text delta + complete event for non-LLM agents too.
            let (tx, rx) = crate::session_stream_hub::install_stream_fanout(
                &self.events.session_stream_hub,
                entry.session_id,
            );
            let kernel_clone = Arc::clone(self);
            let message_owned = message.to_string();
            let entry_clone = entry.clone();

            let handle = tokio::spawn(async move {
                let result = if is_wasm {
                    kernel_clone
                        .execute_wasm_agent(&entry_clone, &message_owned, kernel_handle)
                        .await
                } else {
                    kernel_clone
                        .execute_python_agent(&entry_clone, agent_id, &message_owned)
                        .await
                };

                match result {
                    Ok(result) => {
                        // Emit the complete response as a single text delta
                        let _ = tx
                            .send(StreamEvent::TextDelta {
                                text: result.response.clone(),
                            })
                            .await;
                        let _ = tx
                            .send(StreamEvent::ContentComplete {
                                stop_reason: librefang_types::message::StopReason::EndTurn,
                                usage: result.total_usage,
                            })
                            .await;
                        // Settle pre-charged reservation (#3736)
                        kernel_clone.agents.scheduler.settle_reservation(
                            agent_id,
                            token_reservation,
                            &result.total_usage,
                        );
                        let _ = kernel_clone
                            .agents
                            .registry
                            .set_state(agent_id, AgentState::Running);
                        Ok(result)
                    }
                    Err(e) => {
                        // Non-LLM agent (wasm/python) failed — never made an
                        // LLM call, release reservation without inflating
                        // llm_calls.
                        kernel_clone
                            .agents
                            .scheduler
                            .release_reservation(agent_id, token_reservation);
                        kernel_clone.agents.supervisor.record_panic();
                        warn!(agent_id = %agent_id, error = %e, "Non-LLM agent failed");
                        Err(e)
                    }
                }
            });

            return Ok((rx, handle));
        }

        // LLM agent: true streaming via agent loop
        // Session resolution order (highest priority first):
        // 1. Explicit override from the HTTP caller (multi-tab / multi-session UIs).
        //    Safety check: existing session must belong to this agent.
        // 2. Channel-derived deterministic ID: `SessionId::for_channel(agent, scope)`.
        // 3. Fork: always canonical to preserve prompt-cache alignment.
        // 4. Session-mode fallback: Persistent = entry.session_id, New = fresh UUID.
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
                    // Operators previously had no way to tell from logs
                    // why their `session_mode = "new"` declaration was
                    // not producing per-fire isolation for channel /
                    // cron traffic. Demoted to `trace!` when the
                    // manifest is on the default (Persistent) so the
                    // override is observationally a no-op.
                    let requested_mode = entry.manifest.session_mode;
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
                // Fork calls always target the parent turn's session — the
                // whole point of fork mode is to share the parent's
                // context (and therefore its prompt-cache prefix). An agent
                // with `session_mode = "new"` would otherwise land on
                // `SessionId::new()` here, producing a fresh empty session
                // and breaking cache alignment. Force Persistent for forks
                // regardless of manifest.
                //
                // We read the parent session id from `loop_opts`, NOT from
                // `entry.session_id`. The registry pointer is mutable by
                // `switch_agent_session` / `update_session_id` and can flip
                // between parent loop start and fork spawn, sending the
                // fork to the wrong session and polluting that session's
                // history (#4291). The fork-spawn site
                // (`run_forked_agent_streaming`) snapshots the parent
                // session at fork-construction time and threads it through
                // `LoopOptions::parent_session_id`.
                //
                // NOTE: an explicit `session_id_override` (above) wins over
                // this branch — if you ever plumb an override through a fork
                // caller, prompt-cache alignment WILL break. The current
                // `run_forked_agent_streaming` deliberately passes `None` to
                // preserve this invariant.
                _ if loop_opts.is_fork => loop_opts.parent_session_id.ok_or_else(|| {
                    KernelError::LibreFang(LibreFangError::Internal(
                        "fork loop_opts missing parent_session_id (must be set by \
                         run_forked_agent_streaming before reaching the session resolver)"
                            .to_string(),
                    ))
                })?,
                _ => match entry.manifest.session_mode {
                    librefang_types::agent::SessionMode::Persistent => entry.session_id,
                    librefang_types::agent::SessionMode::New => SessionId::new(),
                },
            }
        };

        // Register the SessionInterrupt clone now that `effective_session_id`
        // is known. Forks deliberately skip this — they share the parent's
        // entry by lookup (see `run_forked_agent_streaming`) and must not
        // overwrite it. See #3172 for the rekey rationale.
        if !loop_opts.is_fork {
            if let Some(interrupt) = loop_opts.interrupt.as_ref() {
                self.agents
                    .session_interrupts
                    .insert((agent_id, effective_session_id), interrupt.clone());
            }
        }

        let existing_session = self
            .memory
            .substrate
            .get_session(effective_session_id)
            .map_err(KernelError::LibreFang)?;
        let session_was_new = existing_session.is_none();
        let mut session = existing_session.unwrap_or_else(|| librefang_memory::session::Session {
            id: effective_session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
            model_override: None,
            messages_generation: 0,
            last_repaired_generation: None,
        });

        // Lifecycle: emit SessionCreated only when get_session returned None.
        if session_was_new {
            self.events.session_lifecycle_bus.publish(
                crate::session_lifecycle::SessionLifecycleEvent::SessionCreated {
                    agent_id,
                    session_id: effective_session_id,
                },
            );
        }

        // Snapshot the compaction config so the spawned task can recompute the
        // `needs_compact` flag *after* reloading the session under the lock.
        // Computing it here on the pre-lock snapshot would make it stale: a
        // concurrent turn that committed history while we were waiting for
        // the lock could push us across (or back below) the threshold.
        //
        // #4976: merge per-agent `[compaction]` overrides on top of the
        // global config so a chat agent and an orchestrator can use
        // different thresholds / summary budgets in the same daemon.
        let compaction_config_snapshot = {
            use librefang_runtime::compactor::CompactionConfig;
            CompactionConfig::from_toml_with_overrides(
                &cfg.compaction,
                entry.manifest.compaction.as_ref(),
            )
        };

        let tools = self.available_tools(agent_id);
        let tools = entry.mode.filter_tools((*tools).clone());
        // NOTE: fork-mode tool allowlist is NOT applied at request-build
        // time — doing so would change the `tools` cache-key component
        // and break Anthropic prompt-cache alignment between parent and
        // fork. The allowlist is enforced at execute time via
        // `LoopOptions::allowed_tools` in agent_loop instead. Before the
        // forkedAgent migration this was filtered here by matching on
        // `sender_context.channel == AUTO_DREAM_CHANNEL`.
        let driver = self.resolve_driver(&entry.manifest)?;

        // Look up model's actual context window from the catalog. Filter out
        // 0 so image/audio entries (no context window) fall through to the
        // caller's default rather than poisoning compaction math.
        let ctx_window = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model(&entry.manifest.model.model)
                .map(|m| m.context_window as usize)
                .filter(|w| *w > 0)
        });

        let (tx, rx) = crate::session_stream_hub::install_stream_fanout(
            &self.events.session_stream_hub,
            effective_session_id,
        );
        let mut manifest = entry.manifest.clone();

        // Apply per-session model override (#4898) before any manifest field is
        // read downstream (model catalog lookup, system prompt build, billing).
        // The pre-lock session snapshot already carries model_override; the
        // reload inside the spawn task will produce the same value because the
        // PATCH route updates the persisted session before returning.
        if let Some(override_str) = session.model_override.as_deref() {
            librefang_runtime::agent_loop::apply_session_model_override_to_manifest(
                &mut manifest,
                override_str,
            )
            .unwrap_or_else(|e| {
                tracing::warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "session model override apply failed on streaming path, falling back to manifest default"
                );
            });
        }

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
                warn!(agent_id = %agent_id, "Failed to backfill workspace (streaming): {e}");
            } else {
                migrate_identity_files(&workspace_dir);
                manifest.workspace = Some(workspace_dir);
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
            // Mirror the peer-scoping applied by `memory_store` on write: use
            // the sender's user_id as the peer namespace so we read the same key
            // the agent wrote.  Falls back to the unscoped key for system turns.
            let peer_id = sender_context
                .map(|s| s.user_id.as_str())
                .filter(|s| !s.is_empty());
            // peer_scoped_key now rejects colon-bearing / empty peer_ids
            // (#5119); on a malformed peer_id we skip the user_name lookup
            // with a WARN so prompt assembly stays best-effort rather than
            // failing the turn.
            let user_name = match peer_scoped_key("user_name", peer_id) {
                Ok(user_name_key) => self
                    .memory
                    .substrate
                    .structured_get(shared_id, &user_name_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_str().map(String::from)),
                Err(e) => {
                    tracing::warn!(
                        peer_id = ?peer_id,
                        error = %e,
                        "skipping user_name lookup: invalid peer_id namespace"
                    );
                    None
                }
            };

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
            // One pass over `tools` produces both the name list (for the
            // hook payload + `PromptContext::granted_tools`) and the
            // description hint map (for `PromptContext::granted_tool_hints`).
            // Avoids three separate walks per send (#4805 review).
            let (granted_tool_names, granted_tool_hints) =
                librefang_runtime::prompt_builder::collect_granted_tool_names_and_hints(&tools);
            let hook_ctx = librefang_runtime::hooks::HookContext {
                agent_name: &manifest.name,
                agent_id: agent_id_str.as_str(),
                event: librefang_types::agent::HookEvent::BeforePromptBuild,
                data: serde_json::json!({
                    "phase": "build",
                    "call_site": "streaming",
                    "user_message": message,
                    "session_id": effective_session_id.to_string(),
                    "channel_type": sender_context.map(|s| s.channel.clone()),
                    "is_group": sender_context.map(|s| s.is_group).unwrap_or(false),
                    "is_subagent": is_subagent_flag,
                    "granted_tools": granted_tool_names,
                }),
            };
            let dynamic_sections = self.governance.hooks.collect_prompt_sections(&hook_ctx);

            // Re-read context.md per turn (cache_context=true to opt out).
            // NOTE: this site is inside `send_message_streaming_with_sender_and_opts`,
            // which is intentionally a non-async wrapper returning a JoinHandle, so
            // we cannot use the async variant here. The sync read remains a known
            // blocking site tracked under #3579 — async-ifying it requires lifting
            // the streaming entry path itself to async, which is out of scope for
            // this PR.
            let context_md = manifest.workspace.as_ref().and_then(|w| {
                librefang_runtime::agent_context::load_context_md(w, manifest.cache_context)
            });

            let prompt_ctx = librefang_runtime::prompt_builder::PromptContext {
                agent_name: manifest.name.clone(),
                agent_description: manifest.description.clone(),
                base_system_prompt: manifest.model.system_prompt.clone(),
                granted_tools: granted_tool_names,
                granted_tool_hints,
                recalled_memories: vec![],
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
                sender_user_id: sender_context.map(|s| s.user_id.clone()),
                sender_display_name: sender_context.map(|s| s.display_name.clone()),
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

        // Inject sender context into manifest metadata so the tool runner can
        // use it for per-sender trust and channel-specific authorization rules.
        // Mirrors `kernel/agent_execution.rs::execute_llm_agent` —
        // `sender_display_name` is part of the same triple and must land here
        // too, otherwise `build_sender_prefix` (#4666) falls back to
        // `sender_user_id` and triggers / `agent_send` produce
        // `[<numeric_id>]: ` instead of `[<friendly_name>]: ` for the same
        // user identity.
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
        }

        let memory = Arc::clone(&self.memory.substrate);
        // Build link context from user message (auto-extract URLs for the agent)
        let message_owned = if let Some(link_ctx) =
            librefang_runtime::link_understanding::build_link_context(message, &cfg.links)
        {
            format!("{message}{link_ctx}")
        } else {
            message.to_string()
        };
        let kernel_clone = Arc::clone(self);

        // RBAC M5: snapshot the caller's UserId / channel from the inbound
        // SenderContext before we move into the spawned task. The auth
        // manager maps `(channel, platform_id)` → UserId; if no binding
        // exists we still record the channel so the spend rolls up under
        // an "unknown user" bucket on that channel.
        let attribution_user_id: Option<UserId> =
            sender_context.and_then(|sc| self.security.auth.identify(&sc.channel, &sc.user_id));
        let attribution_channel: Option<String> = sender_context.map(|sc| sc.channel.clone());

        // `loop_opts` is already a local — the spawned async move will
        // capture it. Agent loop reads these at each turn-end / save /
        // tool-exec checkpoint (see `LoopOptions::is_fork` and
        // `LoopOptions::allowed_tools`). Also snapshot `is_fork` here
        // because we need it after the spawn (to gate `running_tasks`
        // insertion) but `loop_opts` itself gets moved into the async
        // block — can't be re-read outside.
        let is_fork = loop_opts.is_fork;

        // All config-derived values have been snapshotted above; release the
        // reload barrier before spawning the async task.
        drop(_config_guard);

        // Acquire the same session/agent lock as the non-streaming path so concurrent
        // turns are serialized. Clone the Arc here (sync fn); lock inside the spawn.
        let session_lock = if session_id_override.is_some() {
            self.agents
                .session_msg_locks
                .entry(effective_session_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        } else {
            self.agents
                .agent_msg_locks
                .entry(agent_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };

        // Lifecycle: emit TurnStarted right before the spawn. Cloning the bus
        // Arc separately keeps it usable inside the async block via `kernel_clone`.
        self.events.session_lifecycle_bus.publish(
            crate::session_lifecycle::SessionLifecycleEvent::TurnStarted {
                agent_id,
                session_id: effective_session_id,
            },
        );

        // Unique id for this turn — used by cleanup-side `remove_if` so a
        // late-finishing predecessor never wipes out a successor's entry
        // (#3445 stale-entry guard).
        let turn_task_id = uuid::Uuid::new_v4();

        // Reload session after acquiring the lock so we never act on a stale
        // snapshot captured before a concurrent turn's writes landed.
        let handle = tokio::spawn(async move {
            // Acquire the session/agent serialization lock for the duration of
            // this streaming turn.  This matches the non-streaming path and
            // prevents concurrent streaming + non-streaming writes from
            // producing last-write-wins data loss on session history.
            let _session_guard = session_lock.lock().await;

            // Reload session under the lock; keep the placeholder on miss.
            match memory.get_session(effective_session_id) {
                Ok(Some(reloaded)) => {
                    session = reloaded;
                }
                Ok(None) => {
                    // Brand-new session — keep the empty placeholder.
                }
                Err(e) => {
                    warn!(
                        agent_id = %agent_id,
                        session_id = %effective_session_id,
                        error = %e,
                        "Failed to reload session under lock; proceeding with pre-lock snapshot (streaming)"
                    );
                }
            }

            // Recompute `needs_compact` against the freshly-reloaded session.
            // Computing it on the pre-lock snapshot was racy: a concurrent
            // turn that wrote history while we were queued on `session_lock`
            // could have pushed us across (or back below) the threshold,
            // causing this turn to either skip a compact that is now due or
            // re-compact a session another turn just compacted.
            let needs_compact = {
                use librefang_runtime::compactor::{
                    estimate_token_count, needs_compaction as check_compact,
                    needs_compaction_by_tokens,
                };
                let by_messages = check_compact(&session, &compaction_config_snapshot);
                let estimated = estimate_token_count(
                    &session.messages,
                    Some(&manifest.model.system_prompt),
                    None,
                );
                let by_tokens = needs_compaction_by_tokens(estimated, &compaction_config_snapshot);
                if by_tokens && !by_messages {
                    info!(
                        agent_id = %agent_id,
                        estimated_tokens = estimated,
                        messages = session.messages.len(),
                        "Token-based compaction triggered (messages below threshold but tokens above)"
                    );
                }
                by_messages || by_tokens
            };

            // Auto-compact if the session is large before running the loop.
            // Pass the in-turn session id so the compactor operates on
            // the SAME session the outer loop just measured. Using the
            // plain `compact_agent_session(agent_id)` re-looked up via
            // `entry.session_id`, which for channel-derived or
            // `session_mode = "new"` sessions points at a *different*
            // session — and the compactor ended up inspecting an empty
            // one and returning "0 messages, threshold 30" while the
            // real session was 57 messages deep and overflowing.
            // Fork turns must not trigger auto-compaction. Compaction mutates
            // the canonical session on disk — so a dream or auto_memorize fork
            // could compact the user's real conversation, breaking the
            // ephemeral-fork guarantee. Main turns are unaffected: they hit
            // the same check and compact as before.
            if needs_compact && !loop_opts.is_fork {
                info!(agent_id = %agent_id, messages = session.messages.len(), "Auto-compacting session");
                match kernel_clone
                    .compact_agent_session_with_id(agent_id, Some(session.id))
                    .await
                {
                    Ok(msg) => {
                        info!(agent_id = %agent_id, "{msg}");
                        // Reload the session after compaction
                        if let Ok(Some(reloaded)) = memory.get_session(session.id) {
                            session = reloaded;
                        }
                    }
                    Err(e) => {
                        warn!(agent_id = %agent_id, "Auto-compaction failed: {e}");
                    }
                }
            }

            let mut skill_snapshot = kernel_clone
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
                        warn!(agent_id = %agent_id, "Failed to load workspace skills (streaming): {e}");
                    }
                }
            }

            // Create a phase callback that emits PhaseChange events to WS/SSE clients
            let phase_tx = tx.clone();
            let phase_cb: librefang_runtime::agent_loop::PhaseCallback =
                std::sync::Arc::new(move |phase| {
                    use librefang_runtime::agent_loop::LoopPhase;
                    let (phase_str, detail) = match &phase {
                        LoopPhase::Thinking => ("thinking".to_string(), None),
                        LoopPhase::ToolUse { tool_name } => {
                            ("tool_use".to_string(), Some(tool_name.clone()))
                        }
                        LoopPhase::Streaming => ("streaming".to_string(), None),
                        LoopPhase::Done => ("done".to_string(), None),
                        LoopPhase::Error => ("error".to_string(), None),
                    };
                    let event = StreamEvent::PhaseChange {
                        phase: phase_str,
                        detail,
                    };
                    let _ = phase_tx.try_send(event);
                });

            // Set up mid-turn injection channel. Fork turns skip — inserting
            // would overwrite the parent turn's channel (forks share the parent's
            // session id for prompt-cache alignment).
            let injection_rx = if loop_opts.is_fork {
                None
            } else {
                Some(kernel_clone.setup_injection_channel(agent_id, effective_session_id))
            };

            let start_time = std::time::Instant::now();
            // Snapshot config for the duration of the agent loop call
            // (load_full returns Arc so the data stays alive across .await).
            let loop_cfg = kernel_clone.config.load_full();

            // Per-agent MCP pool (workspace-scoped roots).
            let agent_mcp = kernel_clone
                .build_agent_mcp_pool(manifest.workspace.as_deref())
                .await;
            let effective_mcp = agent_mcp
                .as_ref()
                .unwrap_or(&kernel_clone.mcp.mcp_connections);

            let result = run_agent_loop_streaming(
                &manifest,
                &message_owned,
                &mut session,
                &memory,
                driver,
                &tools,
                Some(kernel_handle),
                tx,
                Some(&skill_snapshot),
                Some(effective_mcp),
                Some(&kernel_clone.media.web_ctx),
                Some(&kernel_clone.media.browser_ctx),
                kernel_clone.llm.embedding_driver.as_deref(),
                manifest.workspace.as_deref(),
                Some(&phase_cb),
                Some(&kernel_clone.media.media_engine),
                Some(&kernel_clone.media.media_drivers),
                if loop_cfg.tts.enabled {
                    Some(&kernel_clone.media.tts_engine)
                } else {
                    None
                },
                if loop_cfg.docker.enabled {
                    Some(&loop_cfg.docker)
                } else {
                    None
                },
                Some(&kernel_clone.governance.hooks),
                ctx_window,
                Some(&kernel_clone.processes.manager),
                kernel_clone.checkpoint_manager.clone(),
                Some(&kernel_clone.processes.registry),
                None, // content_blocks (streaming path uses text only for now)
                kernel_clone.memory.proactive_memory.get().cloned(),
                kernel_clone.context_engine_for_agent(&manifest),
                injection_rx.as_deref(),
                &loop_opts,
            )
            .await;

            // Tear down injection channel after loop finishes (skipped for
            // forks since they never set one up — tearing down would
            // remove the parent turn's entry under the shared
            // (agent, session) key).
            if !loop_opts.is_fork {
                kernel_clone.teardown_injection_channel(agent_id, effective_session_id);
            }

            let latency_ms = start_time.elapsed().as_millis() as u64;

            match result {
                Ok(result) => {
                    // Fork turns must not leak into on-disk persistence. The
                    // in-loop `save_session_async` is already gated via
                    // `LoopOptions::is_fork`, but the kernel wraps agent_loop
                    // with three more persistence side effects that were
                    // running regardless: `append_canonical` (cross-channel
                    // memory layer), JSONL session mirror in the agent's
                    // workspace, and the daily memory log. Without this gate
                    // a dream / auto_memorize fork's messages would re-enter
                    // future prompt context via any of those surfaces, which
                    // is exactly the "ephemeral" guarantee the fork API
                    // documents that it provides. Metering / usage stays
                    // unchanged below — forks do consume real tokens and
                    // should count against the agent's budget.
                    if !loop_opts.is_fork {
                        // Append new messages to canonical session for cross-channel memory.
                        // Use run_agent_loop_streaming's own start index (post-trim) instead
                        // of one captured here — the loop may trim session history and make
                        // a locally-captured index stale (see #2067). Clamp defensively.
                        let start = result.new_messages_start.min(session.messages.len());
                        if start < session.messages.len() {
                            let new_messages = session.messages[start..].to_vec();
                            if let Err(e) = memory.append_canonical(
                                agent_id,
                                &new_messages,
                                None,
                                Some(effective_session_id),
                            ) {
                                warn!(agent_id = %agent_id, "Failed to update canonical session (streaming): {e}");
                            }
                        }

                        // Write JSONL session mirror to workspace
                        if let Some(ref workspace) = manifest.workspace {
                            if let Err(e) =
                                memory.write_jsonl_mirror(&session, &workspace.join("sessions"))
                            {
                                warn!("Failed to write JSONL session mirror (streaming): {e}");
                            }
                            // Append daily memory log (best-effort)
                            append_daily_memory_log(workspace, &result.response);
                        }
                    }

                    // Settle the pre-charged token reservation with actual usage
                    // (#3736). This replaces record_usage for the token counters
                    // while still correctly accounting for the burst window.
                    kernel_clone.agents.scheduler.settle_reservation(
                        agent_id,
                        token_reservation,
                        &result.total_usage,
                    );
                    // Record tool calls for rate limiting
                    let tool_count = result.decision_traces.len() as u32;
                    kernel_clone
                        .agents
                        .scheduler
                        .record_tool_calls(agent_id, tool_count);

                    // Lifecycle: emit TurnCompleted alongside settle_reservation. Use
                    // post-loop session length for message_count.
                    kernel_clone.events.session_lifecycle_bus.publish(
                        crate::session_lifecycle::SessionLifecycleEvent::TurnCompleted {
                            agent_id,
                            session_id: effective_session_id,
                            message_count: session.messages.len(),
                        },
                    );

                    // Atomically check quotas and persist usage to SQLite
                    // (mirrors non-streaming path — prevents TOCTOU race)
                    let model = &manifest.model.model;
                    let cost = MeteringEngine::estimate_cost_with_catalog(
                        &kernel_clone.llm.model_catalog.load(),
                        model,
                        result.total_usage.input_tokens,
                        result.total_usage.output_tokens,
                        result.total_usage.cache_read_input_tokens,
                        result.total_usage.cache_creation_input_tokens,
                    );
                    // #4807 review nit 10: honour `actual_provider`
                    // so a chain fail-over bills the slot that did the
                    // work, not the manifest-nominated provider.
                    let billed_provider = result
                        .actual_provider
                        .clone()
                        .unwrap_or_else(|| manifest.model.provider.clone());
                    let usage_record = librefang_memory::usage::UsageRecord {
                        agent_id,
                        provider: billed_provider,
                        model: model.clone(),
                        input_tokens: result.total_usage.input_tokens,
                        output_tokens: result.total_usage.output_tokens,
                        cost_usd: cost,
                        tool_calls: result.decision_traces.len() as u32,
                        latency_ms,
                        // RBAC M5: attribution captured from sender_context
                        // before the spawn — moves into this async block.
                        user_id: attribution_user_id,
                        channel: attribution_channel.clone(),
                        session_id: Some(effective_session_id),
                    };
                    if let Err(e) = kernel_clone.metering.engine.check_all_and_record(
                        &usage_record,
                        &manifest.resources,
                        &kernel_clone.current_budget(),
                    ) {
                        tracing::warn!(
                            agent_id = %agent_id,
                            error = %e,
                            "Post-call quota check failed (streaming); recording usage anyway"
                        );
                        // Hash-chain audit: record BudgetExceeded so the
                        // operator can correlate denied calls with spend.
                        kernel_clone.metering.audit_log.record_with_context(
                            agent_id.to_string(),
                            librefang_runtime::audit::AuditAction::BudgetExceeded,
                            format!("{e}"),
                            "denied",
                            attribution_user_id,
                            attribution_channel.clone(),
                        );
                        let _ = kernel_clone.metering.engine.record(&usage_record);
                    } else if let Some(uid) = attribution_user_id {
                        // RBAC M5: per-user budget enforcement, post-call.
                        // `check_all_and_record` already persisted the row,
                        // so `query_user_*` reflects this call. A breach
                        // doesn't roll back the current response (tokens
                        // were already billed) — it trips BudgetExceeded
                        // so the next call from this user gets denied at
                        // the gate.
                        if let Some(user_budget) = kernel_clone.security.auth.budget_for(uid) {
                            if let Err(e) = kernel_clone
                                .metering
                                .engine
                                .check_user_budget(uid, &user_budget)
                            {
                                tracing::warn!(
                                    agent_id = %agent_id,
                                    user = %uid,
                                    error = %e,
                                    "Per-user budget check failed (streaming)"
                                );
                                kernel_clone.metering.audit_log.record_with_context(
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

                    // Record experiment metrics if running an experiment.
                    // Fork turns skip — a dream / auto_memorize fork is not
                    // a user-initiated request and shouldn't distort the
                    // experiment arm's latency / success / cost averages.
                    // Token / cost accounting above still runs for forks
                    // because those tokens were really billed.
                    if !loop_opts.is_fork {
                        if let Some(ref ctx) = result.experiment_context {
                            let has_content = !result.response.trim().is_empty();
                            let no_tool_errors = result.iterations > 0;
                            let success = has_content && no_tool_errors;
                            let _ = kernel_clone.record_experiment_request(
                                &ctx.experiment_id.to_string(),
                                &ctx.variant_id.to_string(),
                                latency_ms,
                                cost,
                                success,
                            );
                        }
                    }

                    let _ = kernel_clone
                        .agents
                        .registry
                        .set_state(agent_id, AgentState::Running);

                    // Post-loop compaction check: if session now exceeds token threshold,
                    // trigger compaction in background for the next call.
                    // Forks skip — compaction rewrites the canonical session
                    // on disk, which would leak fork context into the user's
                    // real conversation history.
                    if !loop_opts.is_fork {
                        use librefang_runtime::compactor::{
                            estimate_token_count, needs_compaction_by_tokens, CompactionConfig,
                        };
                        let compact_cfg = kernel_clone.config.load();
                        // #4976: merge per-agent [compaction] overrides on
                        // top of the global config. The token threshold
                        // ratio is the field that primarily gates this
                        // post-loop check, and it's now per-agent
                        // tunable.
                        let config = CompactionConfig::from_toml_with_overrides(
                            &compact_cfg.compaction,
                            manifest.compaction.as_ref(),
                        );
                        let estimated = estimate_token_count(&session.messages, None, None);
                        if needs_compaction_by_tokens(estimated, &config) {
                            let kc = kernel_clone.clone();
                            let sid = session.id;
                            // #3740: spawn_logged so compaction panics surface in logs.
                            spawn_logged("post_loop_compaction", async move {
                                info!(agent_id = %agent_id, estimated_tokens = estimated, "Post-loop compaction triggered");
                                // Pass the session id explicitly (same
                                // reason as the pre-loop path above).
                                if let Err(e) =
                                    kc.compact_agent_session_with_id(agent_id, Some(sid)).await
                                {
                                    warn!(agent_id = %agent_id, "Post-loop compaction failed: {e}");
                                }
                            });
                        }
                    }

                    // Skill evolution hot-reload: mirror the non-streaming
                    // `send_message_full` path so ChatPage / SSE clients
                    // also pick up evolved skills immediately after a turn.
                    // Without this, `GET /api/skills/{name}` kept serving
                    // stale versions after `skill_evolve_*` tool calls —
                    // the disk had v0.1.8 while the in-memory registry
                    // was still at v0.1.7, requiring an explicit
                    // `POST /api/skills/reload` to converge.
                    if result
                        .decision_traces
                        .iter()
                        .any(|t| t.tool_name.starts_with("skill_evolve_"))
                    {
                        tracing::info!(
                            agent_id = %agent_id,
                            "Agent used skill evolution tools (streaming) — reloading skill registry"
                        );
                        kernel_clone.reload_skills();
                    }

                    // Task is finishing normally — remove the interrupt handle
                    // so the map doesn't grow without bound.
                    //
                    // Forks share the parent's `SessionInterrupt` entry (see
                    // `run_forked_agent_streaming`), so a fork must NOT remove
                    // it on its own completion — that would orphan the parent
                    // from `stop_agent_run` cancellation. Only the original
                    // parent turn cleans up the map.
                    if !loop_opts.is_fork {
                        kernel_clone
                            .agents
                            .session_interrupts
                            .remove(&(agent_id, effective_session_id));
                        // #3445: only remove if THIS turn's entry is still
                        // present — a faster successor turn may have already
                        // swapped it for its own RunningTask.
                        kernel_clone
                            .agents
                            .running_tasks
                            .remove_if(&(agent_id, effective_session_id), |_, v| {
                                v.task_id == turn_task_id
                            });
                    }
                    Ok(result)
                }
                Err(e) => {
                    // Release the pre-charged token reservation — the
                    // streaming loop failed, no usage to settle.
                    kernel_clone
                        .agents
                        .scheduler
                        .release_reservation(agent_id, token_reservation);
                    kernel_clone.agents.supervisor.record_panic();
                    warn!(agent_id = %agent_id, error = %e, "Streaming agent loop failed");
                    // Lifecycle: emit TurnFailed before cleanup so subscribers
                    // see the failure with the live session_id still valid.
                    kernel_clone.events.session_lifecycle_bus.publish(
                        crate::session_lifecycle::SessionLifecycleEvent::TurnFailed {
                            agent_id,
                            session_id: effective_session_id,
                            error: e.to_string(),
                        },
                    );
                    if !loop_opts.is_fork {
                        kernel_clone
                            .agents
                            .session_interrupts
                            .remove(&(agent_id, effective_session_id));
                        // #3445: only remove if THIS turn's entry is still
                        // present — see Ok branch above.
                        kernel_clone
                            .agents
                            .running_tasks
                            .remove_if(&(agent_id, effective_session_id), |_, v| {
                                v.task_id == turn_task_id
                            });
                    }
                    Err(KernelError::LibreFang(e))
                }
            }
        });

        // Store abort handle for cancellation support. Fork turns skip —
        // registering the fork's handle under the parent's `(agent, session)`
        // key would overwrite the parent's entry (forks deliberately reuse
        // the parent's session id for cache alignment), so a caller invoking
        // `stop_agent_run(agent_id)` during the fork window would abort the
        // fork instead of the parent. Forks are driven by their own caller
        // (auto_memorize, dream) which has its own join handle and doesn't
        // need external cancellation via the registry.
        if !is_fork {
            // #3739: atomically swap in the new task and abort the previous
            // one if any.  `DashMap::insert` returns the displaced value
            // under the same shard write-lock, so two concurrent
            // `send_message_full` calls for the same (agent, session)
            // can never both observe an empty slot and lose one of the
            // abort handles.  The earlier `remove(...) → insert(...)`
            // sequence had exactly that race window.
            //
            // #3445: skip insert if the task already finished while we
            // were preparing to register it. The task's own cleanup
            // path uses `remove_if(... task_id matches ...)`, but if it
            // ran before our insert, the cleanup found nothing to
            // remove and our insert here would leave a stale handle
            // forever. `is_finished()` closes that window.
            //
            // Residual race: if the task finishes between is_finished()
            // returning false and the insert below, cleanup already ran
            // and found nothing; insert then leaves a completed entry.
            // The entry is harmless — AbortHandle::abort() on an already-
            // finished task is a no-op, and the next turn for the same
            // (agent, session) will overwrite it with a fresh RunningTask.
            if handle.is_finished() {
                tracing::debug!(
                    agent_id = %agent_id,
                    session_id = %effective_session_id,
                    "spawned task already finished; skipping running_tasks registration"
                );
            } else {
                let new_task = RunningTask {
                    abort: handle.abort_handle(),
                    started_at: chrono::Utc::now(),
                    task_id: turn_task_id,
                };
                if let Some(old_task) = self
                    .agents
                    .running_tasks
                    .insert((agent_id, effective_session_id), new_task)
                {
                    tracing::debug!(
                        agent_id = %agent_id,
                        session_id = %effective_session_id,
                        "aborting previous running task before starting new one"
                    );
                    old_task.abort.abort();
                }
            }
        }

        Ok((rx, handle))
    }
}
