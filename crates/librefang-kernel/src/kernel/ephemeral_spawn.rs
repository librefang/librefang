//! Ephemeral worker spawn engine (refs #6699, #7723).
//!
//! [`LibreFangKernel::spawn_ephemeral_worker`] runs one agent turn that leaves nothing behind: no registry entry, no persisted session, and a workspace that is deleted when the run ends.
//! It is `send_message_ephemeral` (the tool-less `/btw` side question) with a real tool set, a real workspace, and a real owner.
//!
//! ## The advertised set is the executable set
//!
//! The failure this module exists to avoid is subtle and expensive: hand the model a tool *definition* while the loop that would execute it holds `None` where the kernel handle, skill registry, MCP pool, web context, browser, workspace root or process manager should be.
//! Nothing fails at spawn. The model reads the schema, commits a turn to calling the tool, and gets `Unavailable` back — or, worse, hallucinates a plausible result rather than reporting the error.
//!
//! There are two ways to close that, and this module takes the first: **give the worker the same capability bundle a permanent agent gets**, rather than narrowing the advertised list to the handful of builtins that need no capability at all.
//! The narrower option was rejected on the merits, not on effort.
//! `agent_spawn`, `file_write`, `shell_exec`, `web_fetch` and the skill tools are the entire point of a worker — a mission workspace (#7723) exists so a worker has somewhere to write intermediate files, which is meaningless if the file tools cannot run — and the recursion bound this feature is required to carry only means something if the worker can reach `agent_spawn` at all.
//! A worker restricted to `system_time` and `location_get` would be a feature in name only.
//!
//! What makes the equality *provable* rather than merely intended is the choice of ceiling: the worker's advertised set is derived from [`LibreFangKernel::available_tools`] **for the parent agent**, so it is by construction a subset of what the parent itself may call, executed with the parent's identity against the same handles the parent's own turns run against.
//! `ephemeral_spawn_advertises_exactly_what_the_parent_can_execute` asserts that equality, and `ephemeral_spawn_wires_every_capability_the_permanent_path_wires` pins the call site so a future `None` in a capability slot fails the build's tests rather than a user's turn.
//!
//! ## Everything is attributed to the parent
//!
//! The worker's session carries the **parent's** `AgentId`.
//! That one line is what makes the rest true: `caller_id_str` in the agent loop is `session.agent_id`, so every tool call is attributed to and authorized as the parent; the `UsageRecord` written at the end names the parent, so spend lands on a ledger someone owns; and the quota checked before and after the run is the parent's `[resources]`, not the throwaway manifest's always-unlimited defaults.
//! A pre-flight `check_quota` means an already-exhausted parent is refused *before* the LLM call rather than after it.
//!
//! ## Depth
//!
//! A worker that reaches `agent_spawn` can spawn a worker. The bound is the existing `max_agent_call_depth` quota — the same counter `agent_send` and `run_workflow` use — checked here and incremented by wrapping the run in `with_agent_call_depth`.
//! Reusing it rather than adding a second counter is deliberate: two independently-maintained depth counters is how one of them ends up never being incremented.

use super::mission_workspace::MissionWorkspace;
use super::*;
use crate::agent_template::load_agent_template;
use crate::kernel::subsystems::metering::MeteringSubsystemApi;
use librefang_types::ephemeral::{EphemeralSpawnRequest, EphemeralSpawnResult};

/// Longest mission label accepted before it is sanitized into a directory name.
///
/// Matches `MissionWorkspace`'s own cap, so a label that passes here is never silently truncated there.
const MAX_LABEL_LEN: usize = 64;

/// Narrow a tool-definition list to `wanted`, reporting names that were not available rather than dropping them.
///
/// A silently-dropped name makes a typo in a caller's tool list indistinguishable from success: the worker simply never sees the tool and the caller never learns why.
/// `wanted` entries may be globs (`memory_*`), matching the syntax `capabilities.tools` already uses; a glob that matches nothing is still an error, because an operator who wrote it expected it to match something.
///
/// Returns the narrowed definitions on success, or the offending names, sorted, on failure.
fn narrow_tools(
    available: &[ToolDefinition],
    wanted: &[String],
) -> Result<Vec<ToolDefinition>, Vec<String>> {
    let mut unmatched: Vec<String> = wanted
        .iter()
        .filter(|w| !available.iter().any(|t| glob_matches(w, &t.name)))
        .cloned()
        .collect();
    if !unmatched.is_empty() {
        unmatched.sort();
        unmatched.dedup();
        return Err(unmatched);
    }
    Ok(available
        .iter()
        .filter(|t| wanted.iter().any(|w| glob_matches(w, &t.name)))
        .cloned()
        .collect())
}

/// Whether a `capabilities.tools` list means "no restriction".
///
/// Empty and `["*"]` both mean unrestricted, which is the reading `available_tools` already applies to a permanent agent's manifest.
fn is_unrestricted(tools: &[String]) -> bool {
    tools.is_empty() || tools.iter().any(|t| t == "*")
}

/// The iteration budget one worker run may consume.
///
/// A caller may ask for *fewer* turns than the operator allows and never more: `max_iterations` arrives from a tool call, and an unbounded value there is a spend amplifier aimed at the parent's budget.
/// An unset `agent_max_iterations` means "the compiled-in limit applies", not "no limit", so the ceiling falls back to `AutonomousConfig::DEFAULT_MAX_ITERATIONS` rather than to the request.
/// A requested `0` is treated as "unspecified" — the loop reads `0` as "stop before the first iteration", which no caller means and which would make a worker that silently does nothing.
pub(crate) fn clamp_iterations(requested: Option<u32>, configured: Option<u32>) -> u32 {
    let ceiling =
        configured.unwrap_or(librefang_types::agent::AutonomousConfig::DEFAULT_MAX_ITERATIONS);
    requested
        .filter(|n| *n > 0)
        .map_or(ceiling, |n| n.min(ceiling))
}

/// Build the throwaway session an ephemeral worker's turn runs on.
///
/// `parent_session_id` is always `None`: this session is run with
/// `incognito: true` (see `loop_opts` in `spawn_ephemeral_worker`), which
/// suppresses the end-of-turn `save_session` call entirely, so nothing here
/// ever reaches the `sessions` table. Setting a parent pointer on a row that
/// is never written would be a write nobody reads (#7991 review).
fn new_ephemeral_session(agent_id: AgentId, label: String) -> librefang_memory::session::Session {
    librefang_memory::session::Session {
        id: SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: Some(label),
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    }
}

impl LibreFangKernel {
    /// The tool set an ephemeral worker spawned by `parent_id` both advertises and can execute.
    ///
    /// Three filters compose, all narrowing and never widening:
    ///
    /// 1. **The parent's own set** — `available_tools(parent_id)`, the ceiling. A worker can never call something its parent may not, so a restricted parent cannot launder a privilege escalation through a worker.
    /// 2. **The agent type's declared tools**, when the run is from a template that declares any. This is the "Tools: defined by agent type" row of #6699's design table.
    /// 3. **The caller's requested list**, when one is given.
    ///
    /// Filters 2 and 3 reject a name that survives no earlier filter, naming it, instead of dropping it.
    ///
    /// Ordering is inherited from `available_tools`, which is itself deterministic, so the prompt this feeds is byte-stable across processes (#3298).
    pub(crate) fn ephemeral_tool_set(
        &self,
        parent_id: AgentId,
        template_tools: Option<&[String]>,
        requested: Option<&[String]>,
    ) -> KernelResult<Vec<ToolDefinition>> {
        let parent_tools = self.available_tools(parent_id);
        let mut tools: Vec<ToolDefinition> = (*parent_tools).clone();

        if let Some(declared) = template_tools.filter(|t| !is_unrestricted(t)) {
            tools = narrow_tools(&tools, declared).map_err(|missing| {
                KernelError::LibreFang(LibreFangError::CapabilityDenied(format!(
                    "The agent type declares tools its spawning agent may not call: {}. \
                     A worker cannot exceed the tools of the agent that spawned it.",
                    missing.join(", ")
                )))
            })?;
        }

        if let Some(requested) = requested.filter(|t| !is_unrestricted(t)) {
            tools = narrow_tools(&tools, requested).map_err(|missing| {
                KernelError::LibreFang(LibreFangError::InvalidInput(format!(
                    "Unknown or unavailable tools for an ephemeral worker: {}. \
                     A worker may only be given tools its spawning agent can itself call.",
                    missing.join(", ")
                )))
            })?;
        }

        Ok(tools)
    }

    /// Run one ephemeral worker turn on behalf of `request.parent_id`.
    ///
    /// See the module documentation for the three properties this upholds — advertised-equals-executable, attribution to the parent, and a bounded recursion depth — and why each is built the way it is.
    ///
    /// The mission workspace is created before the LLM call and removed when this function returns, on every path: a returned result, a returned error, and a panic unwinding through it.
    /// A hard kill leaves the directory behind; `sweep_orphan_missions` collects that residue at the next boot.
    pub async fn spawn_ephemeral_worker(
        &self,
        request: EphemeralSpawnRequest,
    ) -> KernelResult<EphemeralSpawnResult> {
        let cfg = self.config.load();
        let parent_id = request.parent_id;

        // ── The parent must exist, and must be able to accept work ──────────
        let parent = self.agents.registry.get(parent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(parent_id.to_string()))
        })?;
        if parent.state == AgentState::Suspended {
            return Err(KernelError::LibreFang(LibreFangError::CapabilityDenied(
                format!(
                    "Agent '{}' is suspended and cannot spawn an ephemeral worker",
                    parent.name
                ),
            )));
        }

        // ── Depth ───────────────────────────────────────────────────────────
        // Checked before any work — before the mission directory, before the
        // budget reservation, before the driver — so a runaway chain costs
        // nothing but the check. `CapabilityDenied` (→ 403), not an internal
        // error: this is an operator quota, and a caller that retries a 5xx
        // would hammer the same refusal.
        let max_depth = cfg.max_agent_call_depth;
        let current_depth = librefang_runtime::tool_runner::current_agent_depth();
        if current_depth >= max_depth {
            return Err(KernelError::LibreFang(LibreFangError::CapabilityDenied(
                format!(
                    "Ephemeral worker spawn depth exceeded (max {max_depth}). \
                     A worker that spawns a worker stacks turns on one task; raise \
                     `max_agent_call_depth` or restructure the task through the task queue."
                ),
            )));
        }

        // ── Budget: the parent's, checked before the call ───────────────────
        // #6930's review found the quota check running against the throwaway
        // worker manifest, whose `ResourceQuota::default()` is unlimited — so
        // attribution was right for reporting and absent for enforcement.
        // The parent's quota is the only one that means anything here.
        let parent_resources = parent.manifest.resources.clone();
        self.metering
            .engine
            .check_quota(parent_id, &parent_resources)
            .map_err(KernelError::LibreFang)?;
        self.metering
            .engine
            .check_global_budget(&self.current_budget())
            .map_err(KernelError::LibreFang)?;

        // ── The manifest the worker runs under ──────────────────────────────
        let (mut manifest, template_tools) = match request.agent_type.as_deref() {
            Some(agent_type) => {
                // Every `TemplateLoadError` variant is a caller-fixable input
                // problem — a typo, a path-shaped name, a malformed or
                // misnamed manifest — and each one's `Display` already names
                // the file or the directories searched. `InvalidInput`
                // (→ 400) keeps that message in front of whoever can act on
                // it; `Internal` would present an operator's broken TOML as a
                // daemon fault.
                let (template, _path) =
                    load_agent_template(self.home_dir(), agent_type).map_err(|e| {
                        KernelError::LibreFang(LibreFangError::InvalidInput(e.to_string()))
                    })?;
                // Deliberately no `source_template` stamp here, unlike the
                // step-agent resolver and `POST /api/agents` (#8018): an
                // ephemeral worker registers no agent and persists no manifest,
                // so there is nothing for the provenance to live on. The field
                // is about answering "where did this agent come from" in the
                // catalog, and this run never enters it.
                let declared = template.capabilities.tools.clone();
                (template, Some(declared))
            }
            // No agent type: the worker is a side task of the parent, so it
            // inherits the parent's persona and model unless told otherwise.
            None => (parent.manifest.clone(), None),
        };

        let tools = self.ephemeral_tool_set(
            parent_id,
            template_tools.as_deref(),
            request.tools.as_deref(),
        )?;

        // ── The mission workspace ───────────────────────────────────────────
        // Created before anything that can fail slowly, and owned by a guard
        // that removes it on the way out of this function no matter which way
        // we leave. `label` is caller-supplied text that becomes a directory
        // path and is later handed to `remove_dir_all`, so containment is
        // established inside `MissionWorkspace::create`, not assumed here.
        let label = request.label.trim();
        if label.is_empty() {
            return Err(KernelError::LibreFang(LibreFangError::InvalidInput(
                "An ephemeral worker needs a non-empty label — it names the mission workspace and the worker itself".to_string(),
            )));
        }
        if label.chars().count() > MAX_LABEL_LEN {
            return Err(KernelError::LibreFang(LibreFangError::InvalidInput(
                format!("Ephemeral worker label is longer than {MAX_LABEL_LEN} characters"),
            )));
        }
        let mission = MissionWorkspace::create(self.home_dir(), label)?;
        let mission_name = mission.name().to_string();

        manifest.name = mission_name.clone();
        manifest.workspace = Some(mission.path().to_path_buf());
        // A worker is dispatched once and discarded: it has no background
        // tick, no triggers to register, and no schedule to honour. Clearing
        // these also keeps them out of the system prompt, where `is_autonomous`
        // would otherwise tell the model it runs on a loop it does not have.
        manifest.autonomous = None;
        manifest.triggers.clear();
        // Enforcement reads the parent's quota (above); carrying the same
        // values on the manifest keeps any incidental reader from finding a
        // second, more permissive answer.
        manifest.resources = parent_resources.clone();
        manifest.capabilities.tools = tools.iter().map(|t| t.name.clone()).collect();
        // The MCP and skill allowlists come from the parent for the same reason the quota does: the worker's tool set was computed by `available_tools(parent_id)`, which applies *the parent's* allowlists, so those are the lists that actually describe what the worker holds.
        // Leaving a template's own values here would let `build_mcp_summary` below tell the model about servers whose tools are not in its list, and stay silent about servers whose tools are.
        // An agent type narrows a worker through `capabilities.tools`, which the tool-set computation above does honour; these four fields are prompt-facing only and are never consulted when a tool is dispatched.
        manifest.mcp_servers = parent.manifest.mcp_servers.clone();
        manifest.mcp_disabled = parent.manifest.mcp_disabled;
        manifest.skills = parent.manifest.skills.clone();
        manifest.skills_disabled = parent.manifest.skills_disabled;
        if let Some(prompt) = request.system_prompt.as_deref() {
            manifest.model.system_prompt = prompt.to_string();
        }
        if let Some(over) = request.model.as_ref() {
            apply_model_override(&mut manifest, over);
        }
        // #8112: same resolution `execute_llm_agent` runs — without it, a
        // `top_p` / `frequency_penalty` / `presence_penalty` set as a
        // per-model catalog override never reached an ephemeral worker's
        // turn. Placed after the request-level model override above so it
        // resolves against the model the worker will actually call.
        super::manifest_helpers::apply_resolved_inference_params(
            &self.llm.model_catalog.load(),
            &mut manifest.model,
        );

        // ── System prompt ───────────────────────────────────────────────────
        let (granted_tool_names, granted_tool_hints) =
            librefang_runtime::prompt_builder::collect_granted_tool_names_and_hints(&tools);
        let hook_ctx = librefang_runtime::hooks::HookContext {
            agent_name: &manifest.name,
            // The hook sees the parent's id because that is the identity the
            // worker runs under — a hook that keys state on `agent_id` must
            // land on the agent that owns the run, not on an id that will
            // never be seen again.
            agent_id: &parent_id.0.to_string(),
            event: librefang_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "phase": "build",
                "call_site": "ephemeral_worker",
                "user_message": request.message,
                "is_subagent": true,
                "granted_tools": granted_tool_names,
                "mission_workspace": mission.path().display().to_string(),
            }),
        };
        let dynamic_sections = self.governance.hooks.collect_prompt_sections(&hook_ctx);

        let mcp_tool_count = self.mcp.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
        let prompt_ctx = librefang_runtime::prompt_builder::PromptContext {
            agent_name: manifest.name.clone(),
            agent_description: manifest.description.clone(),
            base_system_prompt: manifest.model.system_prompt.clone(),
            granted_tools: granted_tool_names.clone(),
            granted_tool_hints,
            mcp_summary: if mcp_tool_count > 0 && !manifest.mcp_disabled {
                self.build_mcp_summary(&manifest.mcp_servers)
            } else {
                String::new()
            },
            workspace_path: Some(mission.path().display().to_string()),
            peer_agents: self.agents.registry.peer_agents_summary(),
            // Date only, for the same prompt-cache reason the other call sites
            // give: a per-minute timestamp invalidates the cached prefix every
            // 60 s (#3700).
            current_date: Some(
                chrono::Local::now()
                    .format("%A, %B %d, %Y (%Y-%m-%d %Z)")
                    .to_string(),
            ),
            // A worker is dispatched by another agent, which is exactly what
            // `is_subagent` marks.
            is_subagent: true,
            active_goals: self.active_goals_for_prompt(parent_id),
            dynamic_sections,
            ..Default::default()
        };
        manifest.model.system_prompt =
            librefang_runtime::prompt_builder::build_system_prompt(&prompt_ctx);

        // ── Provider budget gate ────────────────────────────────────────────
        let provider = manifest.model.provider.clone();
        let exhausted = self.flag_provider_budget_if_exhausted(&provider);
        if self.provider_exhausted_blocks_call(exhausted, &manifest, &cfg) {
            return Err(KernelError::LibreFang(LibreFangError::QuotaExceeded(
                format!(
                    "Provider '{provider}' hourly budget exhausted and the worker has no fallback chain"
                ),
            )));
        }

        // ── Driver, model metadata, capability handles ──────────────────────
        let driver = self.resolve_driver_for_owner(&manifest, None)?;
        // `resolve_context_window` now answers with the layer that produced the
        // number so a session report can label it (#7774). An ephemeral worker
        // renders no such report — it only needs the budget — so take the tokens
        // and drop the provenance here rather than threading it somewhere unused.
        let ctx_window = super::manifest_helpers::resolve_context_window(
            &self.llm.model_catalog.load(),
            &manifest.model,
            None,
        )
        .map(|resolved| resolved.tokens);
        if let Some(supports) = Some(self.llm.model_catalog.load()).and_then(|cat| {
            cat.find_model_for_manifest(&manifest.model.provider, &manifest.model.model)
                .map(|m| cat.effective_capabilities(m).supports_tools)
        }) {
            manifest.metadata.insert(
                "model_supports_tools".to_string(),
                serde_json::Value::Bool(supports),
            );
        }

        // Snapshot the skill registry before the await — the read guard is
        // `!Send`. The mission workspace is fresh, so it contributes no
        // workspace-scoped skills; the global set is what the worker gets.
        let skill_snapshot = self
            .skills
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();
        let agent_mcp = self
            .build_agent_mcp_pool(manifest.workspace.as_deref())
            .await;
        let effective_mcp = agent_mcp.as_ref().unwrap_or(&self.mcp.mcp_connections);
        let kernel_handle = self.kernel_handle();

        // ── The session ─────────────────────────────────────────────────────
        // The parent's `AgentId`, deliberately. `caller_id_str` in the agent
        // loop is `session.agent_id`, so this is the single line that makes
        // every tool call the worker issues run as — and be authorized as —
        // the parent. A fresh random id would have no registry entry, so
        // every registry-consulting tool would fail and every row it wrote
        // would be an orphan.
        //
        // The `SessionId` is fresh and never persisted: `incognito` suppresses
        // the end-of-turn save, and nothing here writes the session table.
        //
        // `parent_session_id` is deliberately `None`, not `Some(parent.session_id)`
        // (#7991 review: the only production writer of the field was building it
        // on exactly this session). Stamping a parent on a session nothing ever
        // saves is a write nobody reads — `children_of` can never find it, and
        // the value would only ever surface in a test that calls `save_session`
        // by hand, which is not a path a real deployment takes. If a real
        // sub-agent lineage feature needs this field, it needs a session that
        // is actually persisted (a non-incognito sub-agent session, or the
        // #7904 `ephemeral_runs` row) — not this one.
        let mut session =
            new_ephemeral_session(parent_id, format!("ephemeral mission {mission_name}"));

        let max_iterations = Some(clamp_iterations(
            request.max_iterations,
            cfg.agent_max_iterations,
        ));

        let loop_opts = librefang_runtime::agent_loop::LoopOptions {
            is_fork: false,
            // #7744: an ephemeral spawn writes nothing durable — `incognito`
            // below suppresses the session, and its tool surface is the
            // parent's — so there is nothing to stamp and no principal is
            // resolved for it.
            acting_principal: None,
            // The run must not reach the session-persistence boundary. Without
            // this the worker's whole session — system prompt, messages, tool
            // calls — is written to the substrate under a session nothing will
            // ever read again, contradicting the "no persistence" contract.
            incognito: true,
            allowed_tools: None,
            interrupt: Some(librefang_runtime::interrupt::SessionInterrupt::new()),
            max_iterations,
            max_history_messages: cfg.max_history_messages,
            memory_fact_budget_percent: cfg.memory_fact_budget_percent,
            aux_client: Some(self.llm.aux_client.load_full()),
            parent_session_id: Some(parent.session_id),
            tool_results_config: Some(cfg.tool_results.clone()),
            compaction_config: Some(cfg.compaction.clone()),
            gateway_compression: Some(cfg.gateway_compression.clone()),
            parallel_tools_config: Some(cfg.parallel_tools.clone()),
            canvas_config: Some(cfg.canvas.clone()),
            system_call: false,
        };

        info!(
            parent_id = %parent_id,
            parent = %parent.name,
            mission = %mission_name,
            workspace = %mission.path().display(),
            tool_count = tools.len(),
            depth = current_depth,
            "Spawning ephemeral worker"
        );

        let started_at = chrono::Utc::now();
        let start_time = std::time::Instant::now();
        // Every capability slot below is wired, and that is the point: the
        // worker executes the tools it advertises. `with_agent_call_depth`
        // makes this turn one level deeper for anything the worker itself
        // spawns, which is what gives the check at the top of this function
        // something to count.
        let outcome = librefang_runtime::tool_runner::with_agent_call_depth(run_agent_loop(
            &manifest,
            &request.message,
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
            None, // no phase callback
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
            None, // no user content blocks
            self.memory.proactive_memory.get().cloned(),
            self.context_engine_for_agent(&manifest),
            None, // no mid-turn injection channel — the worker takes one task
            &loop_opts,
        ))
        .await;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // A run that failed is the run an operator most wants to see.
        // The record is written before the error is propagated, so a worker that died on a driver error leaves the same reachable trace as one that answered — otherwise the only runs visible under a parent would be the ones that went fine, which inverts the point of the feature.
        let result = match outcome {
            Ok(r) => r,
            Err(e) => {
                self.record_ephemeral_run(librefang_memory::EphemeralRunRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_agent_id: parent_id.0.to_string(),
                    label: label.to_string(),
                    worker_name: mission_name.clone(),
                    agent_type: request.agent_type.clone(),
                    task: librefang_memory::ephemeral_run_store::truncate_for_record(
                        &request.message,
                    ),
                    response: String::new(),
                    status: "failed".to_string(),
                    error: Some(e.to_string()),
                    provider: manifest.model.provider.clone(),
                    model: manifest.model.model.clone(),
                    iterations: 0,
                    tool_calls: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                    latency_ms: latency_ms as i64,
                    started_at: started_at.to_rfc3339(),
                    finished_at: chrono::Utc::now().to_rfc3339(),
                });
                return Err(KernelError::LibreFang(e));
            }
        };

        // ── Metering, billed to the parent ──────────────────────────────────
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &self.llm.model_catalog.load(),
            &manifest.model.model,
            result.total_usage.input_tokens,
            result.total_usage.output_tokens,
            result.total_usage.cache_read_input_tokens,
            result.total_usage.cache_creation_input_tokens,
        );
        let usage_record = librefang_memory::usage::UsageRecord {
            agent_id: parent_id,
            // An ephemeral worker has no registry entry of its own, so there is no `AgentEntry` for `billed_agent_for` to read a parent from.
            // The parent IS the billed agent by construction — the worker runs on its behalf and its budget — which is the same answer that helper returns for a parented agent (#7714, #6699).
            billed_agent_id: Some(parent_id),
            provider: result
                .actual_provider
                .clone()
                .unwrap_or_else(|| manifest.model.provider.clone()),
            model: result
                .actual_model
                .clone()
                .unwrap_or_else(|| manifest.model.model.clone()),
            input_tokens: result.total_usage.input_tokens,
            output_tokens: result.total_usage.output_tokens,
            cost_usd: cost,
            tool_calls: result.decision_traces.len() as u32,
            latency_ms,
            user_id: None,
            channel: None,
            // The worker's session is throwaway and is never written to the
            // sessions table, so naming it here would produce a usage row
            // pointing at a session that does not exist.
            session_id: None,
        };
        if let Err(e) = self.metering.engine.check_all_and_record(
            &usage_record,
            &parent_resources,
            &self.current_budget(),
        ) {
            // The work is already done and the tokens are already spent —
            // dropping the record to honour a quota breach would hide the
            // spend that caused it.
            warn!(
                parent_id = %parent_id,
                mission = %mission_name,
                error = %e,
                "Post-call quota check failed for an ephemeral worker; recording usage anyway"
            );
            let _ = self.metering.engine.record(&usage_record);
        }

        // ── The run record, filed under the parent ──────────────────────────
        // The counterpart to the usage row above: that one says what the run cost, this one says what the run *was*.
        // Both name `parent_id`, so the ledger and the run list agree on the owner.
        //
        // Written here, by the kernel, rather than by clearing the loop's `incognito` flag — see `loop_opts` above.
        // That flag gates the episodic-memory write, the context-engine advance and the proactive `auto_memorize` pass as well as the session write, and all four key on `session.agent_id`, which for a worker *is* the parent.
        // Clearing it would teach the parent it said things it never said and file the worker's throwaway session among the parent's real conversations.
        self.record_ephemeral_run(librefang_memory::EphemeralRunRow {
            id: uuid::Uuid::new_v4().to_string(),
            parent_agent_id: parent_id.0.to_string(),
            label: label.to_string(),
            worker_name: mission_name.clone(),
            agent_type: request.agent_type.clone(),
            task: librefang_memory::ephemeral_run_store::truncate_for_record(&request.message),
            response: librefang_memory::ephemeral_run_store::truncate_for_record(&result.response),
            status: "completed".to_string(),
            error: None,
            provider: usage_record.provider.clone(),
            model: usage_record.model.clone(),
            iterations: i64::from(result.iterations),
            tool_calls: result.decision_traces.len() as i64,
            input_tokens: i64::try_from(result.total_usage.input_tokens).unwrap_or(i64::MAX),
            output_tokens: i64::try_from(result.total_usage.output_tokens).unwrap_or(i64::MAX),
            cost_usd: cost,
            latency_ms: latency_ms as i64,
            started_at: started_at.to_rfc3339(),
            finished_at: chrono::Utc::now().to_rfc3339(),
        });

        info!(
            parent_id = %parent_id,
            mission = %mission_name,
            iterations = result.iterations,
            latency_ms,
            "Ephemeral worker finished"
        );

        Ok(EphemeralSpawnResult {
            name: mission_name,
            response: result.response,
            iterations: result.iterations,
            cost_usd: (cost > 0.0).then_some(cost),
            tools: granted_tool_names,
        })
        // `mission` drops here — on this path, on every `?` above it, and
        // while unwinding through a panic — taking the directory with it.
    }

    /// Persist one ephemeral run record, best-effort (refs #7752).
    ///
    /// Best-effort for the same reason the post-call usage record is: the work is already done and the tokens are already spent, so failing the call because bookkeeping failed would throw away an answer the caller has paid for.
    /// A warning names the parent and the mission so the gap is traceable rather than silent.
    ///
    /// This records the *run*, not the workspace.
    /// `MissionWorkspace` still deletes the worker's directory on every exit path (#7723) — a run record makes the delegation auditable, which is a different question from whether the worker's scratch files outlive it, and the answer to the second stays "no".
    pub(crate) fn record_ephemeral_run(&self, row: librefang_memory::EphemeralRunRow) {
        let store = librefang_memory::EphemeralRunStore::new(self.memory.substrate.pool());
        if let Err(e) = store.record_run(&row) {
            warn!(
                parent_id = %row.parent_agent_id,
                mission = %row.worker_name,
                error = %e,
                "Failed to persist an ephemeral worker run record"
            );
        }
    }

    /// The ephemeral runs an agent most recently spawned, newest first.
    ///
    /// Bounded by the store's own per-parent retention cap, so this is the retained history rather than all time.
    pub fn ephemeral_runs_for_agent(
        &self,
        parent_id: AgentId,
        limit: usize,
    ) -> KernelResult<Vec<librefang_memory::EphemeralRunRow>> {
        librefang_memory::EphemeralRunStore::new(self.memory.substrate.pool())
            .list_for_parent(&parent_id.0.to_string(), limit)
            .map_err(KernelError::LibreFang)
    }

    /// Aggregate spend and run count across an agent's retained ephemeral runs.
    pub fn ephemeral_run_rollup_for_agent(
        &self,
        parent_id: AgentId,
    ) -> KernelResult<librefang_memory::EphemeralRunRollup> {
        librefang_memory::EphemeralRunStore::new(self.memory.substrate.pool())
            .rollup_for_parent(&parent_id.0.to_string())
            .map_err(KernelError::LibreFang)
    }
}

/// Apply an `EphemeralModelOverride` to the worker's manifest.
///
/// With no `agent_type` the worker manifest is `parent.manifest.clone()`, so
/// every provider- or model-keyed field arrives describing the **parent's**
/// model. None of them travels with an override, and the permanent spawn path
/// never carries them because it builds a fresh manifest from the profile
/// (#7789 review).
///
/// Takes the whole manifest, not just `manifest.model`: `fallback_models` is a
/// sibling of `model` on `AgentManifest` and carries its own credentials, so a
/// function scoped to `ModelConfig` structurally cannot close the inheritance.
fn apply_model_override(
    manifest: &mut librefang_types::agent::AgentManifest,
    over: &librefang_types::ephemeral::EphemeralModelOverride,
) {
    let model = &mut manifest.model;
    let provider_changed = over
        .provider
        .as_deref()
        .is_some_and(|p| !p.eq_ignore_ascii_case(&model.provider));
    let model_changed = over.model.as_deref().is_some_and(|m| m != model.model);

    // `context_window` is written for one model, and `resolve_context_window`
    // ranks the manifest's own value above the catalog — so a parent pinned by
    // `architect` at 1M tokens spawning a `quick` worker budgeted claude-haiku at
    // 1M: compaction never fires and the provider rejects the oversized request.
    // Cleared on a model change and not only a provider change, because that case
    // stays inside `anthropic` and is the one an operator actually hits.
    //
    // `max_tokens` is the same story from the output side: it reaches the wire
    // through `effective_max_tokens()` with no clamp against the catalog, so a
    // parent at 64000 spawning a Haiku worker earns the identical provider
    // rejection. `max_output_tokens` sits in the same conceptual bucket but has
    // no reader on the request path — only the registry, the catalog and the
    // dashboard — so it is deliberately left alone rather than cleared on
    // speculation.
    //
    // `extra_params` is flattened straight into the request body as
    // `extra_body`, and its own doc-comment calls it provider-specific: a parent
    // on Qwen with `enable_memory`, or on OpenAI with `reasoning_effort`, would
    // otherwise post those keys to anthropic.
    if provider_changed || model_changed {
        model.context_window = None;
        model.max_tokens = None;
        model.extra_params.clear();
    }
    // `api_key_env` / `base_url` are keyed to the provider. Left in place they
    // send the new provider's requests to the parent's endpoint with the parent's
    // key — note `check_provider_credentials` has just asserted that the
    // *profile's* provider has a key, which is then not the one used. A
    // model-only override stays inside the provider they were written for, so it
    // keeps them.
    if provider_changed {
        model.api_key_env = None;
        model.base_url = None;
    }

    if let Some(provider) = over.provider.as_deref() {
        model.provider = provider.to_string();
    }
    if let Some(m) = over.model.as_deref() {
        model.model = m.to_string();
    }

    // The last inheritance path, and the one that defeats the clears above on
    // the second request rather than the first: every `FallbackModel` carries
    // its own `api_key_env`, `base_url` and `extra_params`, and
    // `resolve_effective_fallbacks` treats `Some([…])` as the agent's exclusive
    // chain. A parent whose chain falls back to its expensive model under
    // `MY_PROXY_KEY` hands that entry to a `quick` worker; the first rate-limit
    // on haiku then promotes the worker onto the parent's model with the
    // parent's credential, past both `allowed_profiles` and `cost_budget`.
    //
    // `None` rather than `Some(vec![])`: it restores the global
    // `fallback_providers` chain, which is exactly what a manifest freshly built
    // from a profile carries, so the ephemeral and permanent paths agree.
    if provider_changed || model_changed {
        manifest.fallback_models = None;
    }
}

#[cfg(test)]
mod model_override_tests {
    use super::apply_model_override;
    use librefang_types::agent::{AgentManifest, FallbackModel, ModelConfig};
    use librefang_types::ephemeral::EphemeralModelOverride;

    /// A parent pinned to a custom OpenAI-compatible proxy, at a large window,
    /// with an output cap and a provider extension its model understands.
    fn parent_pinned_to_a_proxy() -> AgentManifest {
        manifest_with(ModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: Some("MY_PROXY_KEY".to_string()),
            base_url: Some("https://proxy.internal/v1".to_string()),
            context_window: Some(1_000_000),
            max_tokens: Some(64_000),
            extra_params: [("reasoning_effort".to_string(), serde_json::json!("high"))]
                .into_iter()
                .collect(),
            ..Default::default()
        })
    }

    fn manifest_with(model: ModelConfig) -> AgentManifest {
        AgentManifest {
            name: "parent".to_string(),
            model,
            ..Default::default()
        }
    }

    fn over(provider: Option<&str>, model: Option<&str>) -> EphemeralModelOverride {
        EphemeralModelOverride {
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
        }
    }

    /// The consequence, rather than the mechanism: the sibling tests assert that `fallback_models` is cleared, this one asserts what that clearing buys.
    /// It composes the spawn-time rewrite with `resolve_effective_fallbacks`, the function that actually decides the worker's **second** request — `Some(list)` is treated as the agent's exclusive chain, so anything inherited here is what the worker escalates onto when haiku rate-limits.
    ///
    /// That second-request timing is what makes the defect expensive to find: the clears above make the *first* call go to anthropic on anthropic's credential, so every assertion about it looks correct while the parent's chain rides along untouched underneath.
    /// Against the pre-fix call site — `apply_model_override(&mut manifest.model, over)` — this fails with the parent's `MY_PROXY_KEY` in the resolved chain.
    #[test]
    fn the_workers_second_request_cannot_reach_the_parents_credential() {
        let mut manifest = parent_pinned_to_a_proxy();
        manifest.fallback_models = Some(vec![FallbackModel {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: Some("MY_PROXY_KEY".to_string()),
            base_url: Some("https://proxy.internal/v1".to_string()),
            extra_params: std::collections::BTreeMap::new(),
        }]);

        apply_model_override(
            &mut manifest,
            &over(Some("anthropic"), Some("claude-haiku-4-5")),
        );

        // No global chain configured, so whatever comes back is what the worker inherited.
        let chain =
            crate::kernel::llm_drivers::resolve_effective_fallbacks(&manifest.fallback_models, &[]);

        assert!(
            !chain
                .iter()
                .any(|f| f.api_key_env.as_deref() == Some("MY_PROXY_KEY")),
            "the worker escalates onto the parent's credential on its second request; \
             got chain {chain:?}"
        );
    }

    /// The credential half: a cross-provider override that keeps the parent's
    /// `base_url` / `api_key_env` posts anthropic-shaped requests to the parent's
    /// proxy with the parent's key — while `check_provider_credentials` has just
    /// asserted that *anthropic* has a key, which is not the one that would be used.
    #[test]
    fn a_provider_change_drops_the_parents_endpoint_and_key() {
        let mut manifest = parent_pinned_to_a_proxy();

        apply_model_override(
            &mut manifest,
            &over(Some("anthropic"), Some("claude-haiku-4-5")),
        );

        assert_eq!(manifest.model.provider, "anthropic");
        assert_eq!(manifest.model.model, "claude-haiku-4-5");
        assert_eq!(
            manifest.model.api_key_env, None,
            "the parent's key names a credential for the parent's provider"
        );
        assert_eq!(
            manifest.model.base_url, None,
            "the parent's endpoint speaks the parent's provider's wire format"
        );
    }

    /// The budget half, and the case a provider-only guard would miss: `architect`
    /// and `quick` are both `anthropic`, so the provider never changes and only a
    /// model-keyed clear saves the worker from being budgeted at the parent's window.
    #[test]
    fn a_model_change_within_one_provider_still_drops_the_window() {
        let mut manifest = manifest_with(ModelConfig {
            provider: "anthropic".to_string(),
            model: "claude-opus-4-1".to_string(),
            context_window: Some(1_000_000),
            ..Default::default()
        });

        apply_model_override(&mut manifest, &over(None, Some("claude-haiku-4-5")));

        assert_eq!(manifest.model.model, "claude-haiku-4-5");
        assert_eq!(
            manifest.model.context_window, None,
            "a 1M-token budget on a Haiku worker never compacts and is rejected by the provider"
        );
    }

    /// The clears are scoped to what actually changed: an override naming the
    /// values already in place is not a reason to discard an operator's pinning.
    #[test]
    fn an_override_that_changes_nothing_keeps_the_parents_pinning() {
        let mut manifest = parent_pinned_to_a_proxy();
        manifest.fallback_models = Some(vec![parent_fallback()]);

        apply_model_override(&mut manifest, &over(Some("OpenAI"), Some("gpt-4o")));

        assert_eq!(
            manifest.model.api_key_env.as_deref(),
            Some("MY_PROXY_KEY"),
            "provider compared case-insensitively; nothing moved, nothing to clear"
        );
        assert_eq!(
            manifest.model.base_url.as_deref(),
            Some("https://proxy.internal/v1")
        );
        assert_eq!(manifest.model.context_window, Some(1_000_000));
        assert_eq!(manifest.model.max_tokens, Some(64_000));
        assert!(manifest.model.extra_params.contains_key("reasoning_effort"));
        assert!(
            manifest.fallback_models.is_some(),
            "the worker is still the parent's model, so the parent's chain still describes it"
        );
    }

    /// A model-only override stays inside the provider the endpoint and key were
    /// written for, so those two must survive it.
    #[test]
    fn a_model_only_override_keeps_the_providers_endpoint_and_key() {
        let mut manifest = parent_pinned_to_a_proxy();

        apply_model_override(&mut manifest, &over(None, Some("gpt-4o-mini")));

        assert_eq!(manifest.model.api_key_env.as_deref(), Some("MY_PROXY_KEY"));
        assert_eq!(
            manifest.model.base_url.as_deref(),
            Some("https://proxy.internal/v1")
        );
        assert_eq!(
            manifest.model.context_window, None,
            "the window was written for gpt-4o"
        );
    }

    /// The parent's fallback entry: its own model, its own credential, its own
    /// endpoint — everything `apply_model_override` clears on the primary.
    fn parent_fallback() -> FallbackModel {
        FallbackModel {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: Some("MY_PROXY_KEY".to_string()),
            base_url: Some("https://proxy.internal/v1".to_string()),
            extra_params: Default::default(),
        }
    }

    /// The inheritance path that survives every clear on the primary and only
    /// fires on the *second* request: `resolve_effective_fallbacks` treats
    /// `Some([…])` as the agent's exclusive chain, so the first rate-limit on
    /// the cheap worker promotes it onto the parent's expensive model with the
    /// parent's key — past `allowed_profiles` and `cost_budget` alike.
    #[test]
    fn a_model_change_drops_the_parents_fallback_chain() {
        let mut manifest = parent_pinned_to_a_proxy();
        manifest.fallback_models = Some(vec![parent_fallback()]);

        apply_model_override(
            &mut manifest,
            &over(Some("anthropic"), Some("claude-haiku-4-5")),
        );

        assert!(
            manifest.fallback_models.is_none(),
            "the parent's chain names the parent's model under the parent's credential; \
             None restores the global fallback_providers a profile-built manifest would carry"
        );
    }

    /// `extra_params` is flattened into the request body verbatim and
    /// `max_tokens` reaches the wire through `effective_max_tokens()` with no
    /// clamp — both are provider/model-keyed exactly like `context_window`.
    #[test]
    fn a_model_change_drops_the_parents_output_cap_and_extension_params() {
        let mut manifest = parent_pinned_to_a_proxy();

        apply_model_override(
            &mut manifest,
            &over(Some("anthropic"), Some("claude-haiku-4-5")),
        );

        assert_eq!(
            manifest.model.max_tokens, None,
            "a 64000-token output cap on a Haiku worker is the same provider rejection \
             the window clear exists to avoid"
        );
        assert!(
            manifest.model.extra_params.is_empty(),
            "OpenAI's reasoning_effort posted to anthropic, got: {:?}",
            manifest.model.extra_params
        );
    }
}

#[cfg(test)]
mod ephemeral_session_tests {
    use super::new_ephemeral_session;
    use librefang_types::agent::AgentId;

    /// Regression for #7991 review: the ephemeral worker's session is
    /// `incognito`, so `save_session` is never called on it — nothing
    /// downstream can ever read a `parent_session_id` stamped here. Pins
    /// the value at the point of construction, since no round-trip
    /// through the database can distinguish the two (both leave
    /// `sessions` untouched either way).
    #[test]
    fn ephemeral_session_has_no_parent_session_id() {
        let session = new_ephemeral_session(AgentId::new(), "test mission".to_string());
        assert!(
            session.parent_session_id.is_none(),
            "an ephemeral worker's session is never persisted (incognito=true \
             suppresses save_session), so a parent pointer here is a write \
             nobody reads and falsely implies lineage `children_of` could find"
        );
    }
}
