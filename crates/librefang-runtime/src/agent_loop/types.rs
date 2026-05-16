//! Public-facing types of the agent loop: phase callbacks, invocation options,
//! and the loop's result struct.
//!
//! Kept separate from the loop itself so callers (kernel, channel bridges,
//! integration tests) can refer to the result type without pulling in the
//! whole 13k-LOC loop body.

use librefang_types::message::TokenUsage;
use librefang_types::tool::DecisionTrace;
use std::sync::Arc;

/// Agent lifecycle phase within the execution loop.
/// Used for UX indicators (typing, reactions) without coupling to channel types.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopPhase {
    /// Agent is calling the LLM.
    Thinking,
    /// Agent is executing a tool.
    ToolUse { tool_name: String },
    /// Agent is streaming tokens.
    Streaming,
    /// Agent finished successfully.
    Done,
    /// Agent encountered an error.
    Error,
}

/// Callback for agent lifecycle phase changes.
/// Implementations should be non-blocking (fire-and-forget) to avoid slowing the loop.
pub type PhaseCallback = Arc<dyn Fn(LoopPhase) + Send + Sync>;

/// Options that modify how `run_agent_loop` / `run_agent_loop_streaming`
/// behave for non-standard invocations.
///
/// Passed by reference because callers typically hold one `LoopOptions` per
/// kernel streaming entry; a clone-per-call would allocate unnecessarily.
///
/// `Default::default()` corresponds to a normal user-initiated main turn:
/// session gets saved, AgentLoopEnd hooks fire with `is_fork: false`, and
/// there's no extra runtime tool allowlist beyond the manifest's.
#[derive(Debug, Clone, Default)]
pub struct LoopOptions {
    /// When true, this invocation is a *derivative* turn (a "fork") rather
    /// than a user-initiated main turn. Semantically:
    ///
    /// - The working session's new messages are **not** persisted to disk.
    ///   Derivative calls are ephemeral by design; they must not pollute
    ///   the agent's canonical conversation history.
    /// - The `AgentLoopEnd` hook context's `data` payload gets `is_fork:
    ///   true`, so subscribers that should only react to real turns (like
    ///   the auto-dream trigger) can filter themselves out and avoid
    ///   recursion.
    ///
    /// Cache alignment: fork turns share the parent's session messages
    /// as a prefix (caller prepares the session), so the Anthropic
    /// prompt cache can hit. Setting `is_fork = true` does *not* change
    /// what gets sent to the provider — only what happens at the
    /// post-response persistence boundary.
    pub is_fork: bool,
    /// When true, this turn runs in **incognito mode**: session messages and
    /// proactive-memory writes are suppressed while memory *reads* remain
    /// fully operational. Incognito turns are useful for brainstorming,
    /// sensitive queries, and debugging agent configuration without polluting
    /// the persistent conversation history.
    ///
    /// Mechanically identical to `is_fork` for the persistence boundary
    /// (all `save_session_async` calls are skipped), but without the other
    /// fork semantics (shared parent-session prefix, tool allowlist, etc.).
    pub incognito: bool,
    /// Runtime tool allowlist. When `Some`, any `tool_use` the model
    /// emits that names a tool outside this list is denied at execute
    /// time (a synthetic error result is returned to the model so it can
    /// adapt). When `None`, no extra filter beyond the agent manifest's
    /// built-in `capabilities.tools` is applied.
    ///
    /// This is enforced at *execute* time rather than by stripping tools
    /// from the request schema, so the request body stays byte-identical
    /// to the parent turn and the Anthropic prompt cache keeps hitting.
    /// The model may "try" a disallowed tool (wasting a few tokens on
    /// the `tool_use` block) but cannot actually invoke it — same
    /// defense-in-depth as libre-code's `createAutoMemCanUseTool`.
    pub allowed_tools: Option<Vec<String>>,
    /// Per-session interrupt handle.  When `Some`, long-running tools
    /// (shell_exec, sub-process tools) poll this flag and abort promptly
    /// when it is set.  When `None`, no interrupt checking is performed.
    ///
    /// The handle is created once per session/turn and cloned into each
    /// `ToolExecutionContext` so that cancelling the parent session
    /// interrupts all its in-flight tools without affecting other sessions.
    pub interrupt: Option<crate::interrupt::SessionInterrupt>,
    /// Operator-level override for the agent-loop iteration cap. Resolution
    /// order when the loop starts:
    /// 1. `manifest.autonomous.max_iterations` (per-agent)
    /// 2. `opts.max_iterations` (operator / kernel config)
    /// 3. `AutonomousConfig::DEFAULT_MAX_ITERATIONS` (library fallback)
    ///
    /// Kernel populates this from `KernelConfig.agent_max_iterations` so
    /// operators can lower the default without recompiling or editing every
    /// manifest. None → use the library fallback.
    pub max_iterations: Option<u32>,
    /// Operator-level override for the message-history trim cap.
    /// Resolution order when the loop starts:
    /// 1. `manifest.max_history_messages` (per-agent)
    /// 2. `opts.max_history_messages` (operator / kernel config)
    /// 3. `DEFAULT_MAX_HISTORY_MESSAGES` (library fallback)
    ///
    /// Kernel populates this from `KernelConfig.max_history_messages` so
    /// operators can lower the default without recompiling or editing every
    /// manifest. `None` → use the library fallback. Values below
    /// `MIN_HISTORY_MESSAGES` are clamped up at resolution time.
    pub max_history_messages: Option<usize>,
    /// Auxiliary LLM client used for cheap-tier side tasks
    /// (context compression, title generation, search summarisation,
    /// vision captioning). When `None`, side tasks fall back to the
    /// primary `driver` — preserving pre-issue-#3314 behaviour.
    ///
    /// Kernel populates this from the boot-time-built [`AuxClient`].
    /// Tests typically leave it as `None`.
    pub aux_client: Option<std::sync::Arc<crate::aux_client::AuxClient>>,
    /// When `is_fork = true`, the session id the *parent* turn was actually
    /// invoked on (i.e. the parent's resolved `effective_session_id`, NOT
    /// the registry's mutable `entry.session_id` pointer). The kernel's
    /// session resolver consumes this to land the fork on the parent's
    /// session for prompt-cache alignment, regardless of whether the
    /// agent registry pointer has since been re-pointed by
    /// `switch_agent_session` / `update_session_id`.
    ///
    /// MUST be `Some(parent_session)` whenever `is_fork = true`. The
    /// kernel surfaces a hard error if `is_fork && parent_session_id ==
    /// None`, because reading `entry.session_id` at fork-spawn time is a
    /// TOCTOU race against `switch_agent_session` (#4291). For
    /// non-fork loops this field is ignored and should be left `None`.
    pub parent_session_id: Option<librefang_types::agent::SessionId>,
    /// Tool-result budget configuration (#3347 2/N and 3/N).
    ///
    /// When `Some`, the per-result spill threshold, per-turn cumulative cap,
    /// and history-fold turn count are taken from this config.  When `None`
    /// (the default), all three fall back to [`ToolResultsConfig::default()`].
    ///
    /// Kernel populates this from `KernelConfig.runtime.tool_results`.
    pub tool_results_config: Option<librefang_types::config::ToolResultsConfig>,
    /// Compaction config snapshot (#4976).
    ///
    /// When `Some`, the agent loop builds its
    /// [`crate::context_compressor::ContextCompressor`] from this config,
    /// honouring `keep_recent` / `max_summary_tokens` /
    /// `token_threshold_ratio` that may have been overridden by the
    /// agent's `[compaction]` block in `agent.toml`. When `None` (the
    /// default and the legacy test path), the compressor falls back to
    /// `CompressionConfig::default()`.
    ///
    /// Kernel populates this by merging
    /// [`AgentManifest::compaction`] on top of
    /// `KernelConfig.compaction` via
    /// [`librefang_types::agent::CompactionOverrides::resolve`] before
    /// constructing the loop. Pre-merging in the kernel keeps the
    /// agent-loop blind to override semantics.
    pub compaction_config: Option<librefang_types::config::CompactionTomlConfig>,
    /// Gateway compression configuration (#4972). When `Some`, the
    /// runtime runs a cheap safety-net compression pass at the top of the
    /// loop, before the first LLM call, when the session has grown past
    /// the configured threshold (default 85 % of context window). When
    /// `None`, the gateway pass is skipped entirely — kept `None` in tests
    /// and other call-paths that don't need it. Kernel populates this
    /// from `KernelConfig.gateway_compression`.
    pub gateway_compression: Option<librefang_types::config::GatewayCompressionConfig>,
}

/// Result of an agent loop execution.
#[derive(Debug, Default)]
pub struct AgentLoopResult {
    /// The final text response from the agent.
    pub response: String,
    /// Total token usage across all LLM calls.
    pub total_usage: TokenUsage,
    /// Number of iterations the loop ran.
    pub iterations: u32,
    /// Estimated cost in USD (populated by the kernel after the loop returns).
    pub cost_usd: Option<f64>,
    /// True when the agent intentionally chose not to reply (NO_REPLY token or [[silent]]).
    pub silent: bool,
    /// Reply directives extracted from the agent's response.
    pub directives: librefang_types::message::ReplyDirectives,
    /// Structured decision traces for each tool call made during the loop.
    /// Captures reasoning, inputs, timing, and outcomes for debugging and auditing.
    pub decision_traces: Vec<DecisionTrace>,
    /// Summaries of memories that were saved during this turn (from auto_memorize).
    /// Empty when no new memories were extracted.
    pub memories_saved: Vec<String>,
    /// Summaries of memories that were recalled and injected as context (from auto_retrieve).
    /// Empty when no relevant memories were found.
    pub memories_used: Vec<String>,
    /// Detected memory conflicts where new info contradicts existing memories.
    /// Empty when no conflicts were detected.
    pub memory_conflicts: Vec<librefang_types::memory::MemoryConflict>,
    /// True when the agent loop was skipped because no LLM provider is configured.
    /// Distinct from `silent` (agent chose not to reply) — this means the system
    /// couldn't run the agent at all.
    pub provider_not_configured: bool,
    /// Experiment tracking: when an A/B experiment is running, this holds the variant used.
    pub experiment_context: Option<ExperimentContext>,
    /// Latency in milliseconds for this request.
    pub latency_ms: u64,
    /// Index in `session.messages` where messages appended during this turn
    /// begin. Callers use this to slice out the turn's new messages (e.g. for
    /// writing to a canonical cross-channel session) without tracking their
    /// own index — which would go stale if the loop trims session history.
    /// Always in range [0, session.messages.len()] after the loop returns.
    pub new_messages_start: usize,
    /// True when the agent used enough tool calls that skill evolution review
    /// is recommended. The kernel checks this to trigger background skill
    /// creation/improvement suggestions. Threshold: 5+ tool calls.
    pub skill_evolution_suggested: bool,
    /// Optional private message destined for the agent's owner (operator DM),
    /// produced when the LLM invokes the `notify_owner` tool during the turn.
    /// `None` means the model did not request an owner-side notification.
    /// Multiple notify_owner calls in the same turn are concatenated with
    /// "\n\n" by the tool handler before being placed here.
    pub owner_notice: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExperimentContext {
    pub experiment_id: uuid::Uuid,
    pub variant_id: uuid::Uuid,
    pub variant_name: String,
    pub request_start: std::time::Instant,
}

impl Default for ExperimentContext {
    fn default() -> Self {
        Self {
            experiment_id: uuid::Uuid::default(),
            variant_id: uuid::Uuid::default(),
            variant_name: String::new(),
            request_start: std::time::Instant::now(),
        }
    }
}
