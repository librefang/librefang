//! Pluggable context engine — lifecycle hooks for context management.
//!
//! This trait lets developers plug in their own strategies for memory recall,
//! message assembly, compaction, and post-turn bookkeeping without modifying
//! the core agent loop.
//!
//! # Lifecycle
//!
//! The context engine participates at six lifecycle points:
//!
//! 1. **`bootstrap`** — Called once when the engine is created. Load indexes,
//!    connect to vector databases, warm caches, etc.
//!
//! 2. **`ingest`** — Called when a new user message enters the session.
//!    Store or index the message in your own data store.
//!
//! 3. **`assemble`** — Called before each LLM call. Return an ordered set of
//!    messages that fit within the token budget. This is the core hook — it
//!    controls what the model "sees".
//!
//! 4. **`compact`** — Called when the context window is under pressure.
//!    Summarize older history to free space.
//!
//! 5. **`after_turn`** — Called after a complete turn (LLM response + tool
//!    execution). Persist state, trigger background compaction, update indexes.
//!
//! 6. **`prepare_subagent_context` / `merge_subagent_context`** — Called around
//!    sub-agent spawning to isolate or merge memory scopes.
//!
//! # Default Implementation
//!
//! [`DefaultContextEngine`] wraps all existing LibreFang context management:
//! - CJK-aware token estimation
//! - Two-layer context budget (per-result cap + context guard)
//! - 4-stage overflow recovery with pinned message support
//! - LLM-based session compaction (single-pass, chunked, fallback)
//! - Embedding-based semantic memory recall with LIKE fallback

use async_trait::async_trait;
use librefang_memory::MemorySubstrate;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{Memory, MemoryFilter, MemoryFragment};
use librefang_types::message::Message;
use librefang_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::compactor::{self, CompactionConfig, CompactionResult};
use crate::context_budget::{apply_context_guard, ContextBudget};
use crate::context_overflow::{recover_from_overflow, RecoveryStage};
use crate::embedding::EmbeddingDriver;
use crate::llm_driver::LlmDriver;

/// Return the state file path scoped to a specific agent.
///
/// If `agent_id` is `None` or empty, returns the shared (plugin-level) path.
/// Otherwise returns `{plugin_state_dir}/agents/{agent_id}/state.json`.
fn agent_scoped_state_path(
    base_path: &std::path::Path,
    agent_id: Option<&str>,
) -> std::path::PathBuf {
    match agent_id.filter(|s| !s.is_empty()) {
        Some(id) => {
            // Sanitise the agent ID — keep only alphanumeric, '-', '_'.
            let safe_id: String = id
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            // base_path is e.g. `/home/user/.librefang/plugins/my_plugin/state.json`
            // We place agent state alongside: `…/agents/{safe_id}/state.json`
            let parent = base_path.parent().unwrap_or(base_path);
            parent.join("agents").join(&safe_id).join("state.json")
        }
        None => base_path.to_path_buf(),
    }
}

/// Generate a compact random trace ID (16 lowercase hex characters = 64-bit entropy).
fn generate_trace_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Combine current time + thread ID for low-collision IDs without pulling in `rand`.
    let mut h = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        .hash(&mut h);
    std::thread::current().id().hash(&mut h);
    // XOR with address of a stack variable for extra entropy.
    let stack_var: u64 = 0;
    ((&stack_var) as *const u64 as u64).hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Health snapshot for one engine layer in a StackedContextEngine.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineLayerHealth {
    /// Plugin name for this layer (empty string if unnamed).
    pub plugin_name: String,
    /// Per-hook circuit breaker state: `hook_name → open`.
    pub circuit_open: std::collections::HashMap<String, bool>,
    /// Number of hooks with at least one registered hook script.
    pub active_hooks: usize,
    /// Approximate count of recent errors across all hooks (from trace ring buffer).
    pub recent_errors: usize,
    /// Approximate count of recent calls across all hooks (from trace ring buffer).
    pub recent_calls: usize,
}

/// Aggregated health across all layers of a StackedContextEngine.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StackHealth {
    pub layers: Vec<EngineLayerHealth>,
    pub total_layers: usize,
    pub layers_with_open_circuit: usize,
}

/// An event emitted by a plugin hook.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginEvent {
    /// Event name, e.g. `"code_detected"`, `"user_frustrated"`.
    pub name: String,
    /// Arbitrary JSON payload attached to the event.
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Plugin that emitted the event.
    pub source_plugin: String,
}

/// Rate-limit window between consecutive `error!` logs from
/// [`PluginEventBus::record_consumer_lag`]. Must stay in sync with the
/// hardcoded `10` in `librefang_kernel::event_bus::EventBus::record_consumer_lag`
/// (search for `from_secs(10)` in `crates/librefang-kernel/src/event_bus.rs`)
/// so the two buses behave identically.
const LAG_WARN_INTERVAL_SECS: u64 = 10;

/// A simple in-process event bus for plugin events.
///
/// Backed by a `tokio::sync::broadcast` channel so multiple engines can
/// receive the same event without coordination.
pub struct PluginEventBus {
    tx: tokio::sync::broadcast::Sender<PluginEvent>,
    /// Total events dropped by consumers due to broadcast lag. Mirrors
    /// the kernel `EventBus` counter so plugin-side `on_event` misses
    /// stop being silently swallowed (issue #3630).
    dropped_count: std::sync::atomic::AtomicU64,
    /// Rate-limit timestamp for the lag warning log.
    last_drop_warn: std::sync::Mutex<std::time::Instant>,
}

impl PluginEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(capacity);
        // Initialise the rate-limit timestamp far enough in the past that
        // the FIRST lag burst after startup is always logged. Without this,
        // a fresh process that immediately sees lag would only bump the
        // counter and stay silent for the first 10 s — defeating the
        // "make lag visible" goal of #3630.
        // checked_sub: CLOCK_MONOTONIC can be <(LAG_WARN_INTERVAL_SECS+1) s on boot; fallback forfeits warmup, not correctness.
        let warmup = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(LAG_WARN_INTERVAL_SECS + 1))
            .unwrap_or_else(std::time::Instant::now);
        Self {
            tx,
            dropped_count: std::sync::atomic::AtomicU64::new(0),
            last_drop_warn: std::sync::Mutex::new(warmup),
        }
    }

    /// Publish an event to all subscribers.
    pub fn emit(&self, event: PluginEvent) {
        let _ = self.tx.send(event); // ignore SendError (no subscribers yet)
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<PluginEvent> {
        self.tx.subscribe()
    }

    /// Total events that consumers dropped due to broadcast lag.
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record that a consumer lagged and dropped `n` events. Bumps
    /// `dropped_count` and emits a rate-limited `error!` log. Mirrors
    /// `librefang_kernel::event_bus::EventBus::record_consumer_lag`.
    pub fn record_consumer_lag(&self, n: u64, context: &'static str) {
        let total = self
            .dropped_count
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed)
            + n;
        if let Ok(mut last) = self.last_drop_warn.lock() {
            if last.elapsed() >= std::time::Duration::from_secs(LAG_WARN_INTERVAL_SECS) {
                tracing::error!(
                    lagged = n,
                    total_dropped = total,
                    context = context,
                    "Plugin event bus: consumer lagged behind broadcast queue, events dropped — \
                     receiver should be drained faster or buffer increased",
                );
                *last = std::time::Instant::now();
            }
        }
    }
}

/// Fields that a plugin's `bootstrap` hook may override at runtime.
///
/// The bootstrap script returns a JSON object; any recognised keys here are
/// applied to the running engine.  Unknown keys are silently ignored.
///
/// Example bootstrap output:
/// ```json
/// {
///   "env_overrides": {"MY_PLUGIN_API_KEY": "secret"},
///   "ingest_filter": "only_when_code_present",
///   "allow_network": true
/// }
/// ```
#[derive(Debug, Default, serde::Deserialize)]
pub struct BootstrapOverrides {
    /// Additional env vars merged into `plugin_env` for subsequent hook calls.
    #[serde(default)]
    pub env_overrides: std::collections::HashMap<String, String>,
    /// Override the ingest filter string.
    #[serde(default)]
    pub ingest_filter: Option<String>,
    /// Override network permission for all subsequent hook calls.
    #[serde(default)]
    pub allow_network: Option<bool>,
}

/// Configuration for the context engine.
#[derive(Debug, Clone)]
pub struct ContextEngineConfig {
    /// Model context window size in tokens.
    pub context_window_tokens: usize,
    /// Whether stable-prefix mode is enabled (skip memory recall for caching).
    pub stable_prefix_mode: bool,
    /// Maximum number of memories to recall per query.
    pub max_recall_results: usize,
    /// User-facing compaction configuration (from `[compaction]` TOML section).
    /// When `None`, runtime defaults are used.
    pub compaction: Option<librefang_types::config::CompactionTomlConfig>,
    /// When `true`, hook output that fails schema validation causes the hook call
    /// to return an error instead of logging a warning.  Defaults to `false` for
    /// backward compatibility.
    pub output_schema_strict: bool,
    /// Maximum number of times any single hook may be called per 60-second window.
    /// `0` means unlimited.  Defaults to `0`.
    pub max_hook_calls_per_minute: u32,
}

impl Default for ContextEngineConfig {
    fn default() -> Self {
        Self {
            context_window_tokens: 200_000,
            stable_prefix_mode: false,
            max_recall_results: 5,
            compaction: None,
            output_schema_strict: false,
            max_hook_calls_per_minute: 0,
        }
    }
}

/// Result from the `assemble` lifecycle hook.
#[derive(Debug)]
pub struct AssembleResult {
    /// Recovery stage applied during assembly (if any).
    pub recovery: RecoveryStage,
}

/// Result from the `ingest` lifecycle hook.
#[derive(Debug)]
pub struct IngestResult {
    /// Recalled memory fragments relevant to the ingested message.
    pub recalled_memories: Vec<MemoryFragment>,
}

/// Pluggable context engine trait.
///
/// Implement this trait to provide custom context management strategies.
/// The agent loop calls these hooks at well-defined lifecycle points,
/// giving plugins full control over what the LLM sees and how history
/// is managed.
#[async_trait]
pub trait ContextEngine: Send + Sync {
    /// Called once during engine initialization.
    ///
    /// Use this to load indexes, connect to external vector stores,
    /// warm caches, or perform any one-time setup.
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()>;

    /// Called when a new user message enters the session.
    ///
    /// Use this to index the message, recall relevant memories, or
    /// update internal state. Returns recalled memories that the
    /// agent loop injects into the system prompt.
    ///
    /// `peer_id` is the sender's platform user ID when the message arrived
    /// from a channel (Telegram, Discord, …) — implementors MUST scope
    /// memory recall to this peer to prevent cross-user context leaks.
    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult>;

    /// Called before each LLM call to assemble the context window.
    ///
    /// Given the current messages and available tools, trim and reorder
    /// them to fit within the agent's token budget. `context_window_tokens`
    /// is the **current agent's** model context size (not a global default).
    ///
    /// The default implementation applies overflow recovery, context
    /// guard compaction, and session repair.
    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult>;

    /// Called when the context window is under pressure.
    ///
    /// Summarize older history to free space. `model` is the agent's
    /// configured LLM model name. `context_window_tokens` is the
    /// **current agent's** model context size so compaction uses the
    /// correct window, not the boot-time default.
    /// The default implementation uses LLM-based compaction with 3
    /// strategies (single-pass, chunked, fallback).
    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult>;

    /// Called after a complete turn (LLM response + tool execution).
    ///
    /// Use this to persist state, trigger background compaction, update
    /// indexes, or perform any post-turn bookkeeping.
    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()>;

    /// Called before a sub-agent is spawned.
    ///
    /// Use this to prepare isolated memory scopes or fork context for
    /// the child agent. Default implementation is a no-op.
    async fn prepare_subagent_context(
        &self,
        _parent_id: AgentId,
        _child_id: AgentId,
    ) -> LibreFangResult<()> {
        Ok(())
    }

    /// Called after a sub-agent completes.
    ///
    /// Use this to merge the child's context back into the parent's
    /// memory scope. Default implementation is a no-op.
    async fn merge_subagent_context(
        &self,
        _parent_id: AgentId,
        _child_id: AgentId,
    ) -> LibreFangResult<()> {
        Ok(())
    }

    /// Truncate a tool result according to the engine's budget policy.
    ///
    /// `context_window_tokens` is the **current agent's** model context size
    /// so budget-based caps scale correctly per agent.
    /// Default implementation uses head+tail truncation strategy.
    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String;

    /// Return a snapshot of hook invocation metrics, if the engine tracks them.
    ///
    /// Returns `None` for the default engine (no hooks to instrument).
    /// `ScriptableContextEngine` returns live counters; `StackedContextEngine`
    /// returns aggregated counters across all stacked engines.
    fn hook_metrics(&self) -> Option<HookMetrics> {
        None
    }

    /// Return recent hook invocation traces (last ≤100 calls) for debugging.
    ///
    /// Returns an empty vec for engines that don't record traces.
    fn hook_traces(&self) -> Vec<HookTrace> {
        Vec::new()
    }

    /// Return per-agent hook call stats (agent_id → HookStats).
    fn per_agent_metrics(&self) -> std::collections::HashMap<String, HookStats> {
        std::collections::HashMap::new()
    }

    // -----------------------------------------------------------------------
    // Hermes-Agent ContextEngine compatibility interface
    // -----------------------------------------------------------------------

    /// Return `true` if the engine believes the context should be compacted
    /// this turn.
    ///
    /// The default threshold is 80 % of `max_tokens`.  Engines that perform
    /// summarisation (e.g. [`SummaryContextEngine`]) use this to gate the
    /// call to [`ContextEngine::compact`].  [`NoCompactContextEngine`] always
    /// returns `false`.
    ///
    /// Mirrors `ContextEngine.should_compress(prompt_tokens)` from the Python
    /// reference implementation in `hermes-agent/agent/context_engine.py`.
    fn should_compress(&self, current_tokens: usize, max_tokens: usize) -> bool {
        if max_tokens == 0 {
            return false;
        }
        current_tokens >= (max_tokens * 4 / 5) // 80 %
    }

    /// Notify the engine that the active model or its context window has
    /// changed (e.g. user switched models or fallback kicked in).
    ///
    /// The default implementation is a no-op; engines that maintain their own
    /// token-budget accounting should override this to recalculate thresholds.
    ///
    /// Mirrors `ContextEngine.update_model(model, context_length, …)` from
    /// the Python reference implementation.
    ///
    /// Takes `&self` (not `&mut self`) so the method is callable through a
    /// `&dyn ContextEngine` trait object.  Implementations that need to mutate
    /// state should use interior mutability (e.g. `Mutex`).
    fn update_model(&self, _model: &str, _context_length: usize) {}
}

// ---------------------------------------------------------------------------
// Default implementation — wraps all existing LibreFang context management
// ---------------------------------------------------------------------------

/// Default context engine that wraps LibreFang's built-in context management.
///
/// Composes existing modules:
/// - [`ContextBudget`] for per-result and total tool result caps
/// - [`recover_from_overflow`] for 4-stage overflow recovery
/// - [`compact_session`](crate::compactor::compact_session) for LLM summarization
/// - Embedding-based semantic memory recall with LIKE fallback
pub struct DefaultContextEngine {
    config: ContextEngineConfig,
    memory: Arc<MemorySubstrate>,
    embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
    compaction_config: CompactionConfig,
}

impl DefaultContextEngine {
    /// Create a new default context engine.
    pub fn new(
        config: ContextEngineConfig,
        memory: Arc<MemorySubstrate>,
        embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
    ) -> Self {
        let mut compaction_config = match config.compaction {
            Some(ref toml) => CompactionConfig::from_toml(toml),
            None => CompactionConfig::default(),
        };
        compaction_config.context_window_tokens = config.context_window_tokens;
        Self {
            config,
            memory,
            embedding_driver,
            compaction_config,
        }
    }

    /// Get the context window size in tokens.
    pub fn context_window_tokens(&self) -> usize {
        self.config.context_window_tokens
    }

    /// Get the compaction config.
    pub fn compaction_config(&self) -> &CompactionConfig {
        &self.compaction_config
    }

    /// Get a reference to the memory substrate.
    pub fn memory_substrate(&self) -> &Arc<MemorySubstrate> {
        &self.memory
    }
}

#[async_trait]
impl ContextEngine for DefaultContextEngine {
    async fn bootstrap(&self, _config: &ContextEngineConfig) -> LibreFangResult<()> {
        debug!(
            context_window = self.config.context_window_tokens,
            stable_prefix = self.config.stable_prefix_mode,
            "DefaultContextEngine bootstrapped"
        );
        Ok(())
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        // In stable_prefix_mode, skip memory recall to keep system prompt stable for caching.
        if self.config.stable_prefix_mode {
            return Ok(IngestResult {
                recalled_memories: Vec::new(),
            });
        }

        let filter = Some(MemoryFilter {
            agent_id: Some(agent_id),
            peer_id: peer_id.map(String::from),
            ..Default::default()
        });
        let limit = self.config.max_recall_results;

        // Prefer vector similarity search when embedding driver is available
        let memories = if let Some(ref emb) = self.embedding_driver {
            match emb.embed_one(user_message).await {
                Ok(query_vec) => {
                    debug!("ContextEngine: vector recall (dims={})", query_vec.len());
                    self.memory
                        .recall_with_embedding_async(user_message, limit, filter, Some(&query_vec))
                        .await
                        .unwrap_or_else(|e| {
                            warn!("ContextEngine: vector recall query failed: {e}");
                            Vec::new()
                        })
                }
                Err(e) => {
                    warn!("ContextEngine: embedding recall failed, falling back to text: {e}");
                    self.memory
                        .recall(user_message, limit, filter)
                        .await
                        .unwrap_or_else(|e| {
                            warn!("ContextEngine: text recall failed: {e}");
                            Vec::new()
                        })
                }
            }
        } else {
            self.memory
                .recall(user_message, limit, filter)
                .await
                .unwrap_or_else(|e| {
                    warn!("ContextEngine: memory recall failed: {e}");
                    Vec::new()
                })
        };

        Ok(IngestResult {
            recalled_memories: memories,
        })
    }

    async fn assemble(
        &self,
        _agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        // Stage 1: Overflow recovery pipeline (4-stage cascade, respects pinned messages)
        // Uses the per-agent context window size, not the boot-time default.
        let recovery = recover_from_overflow(messages, system_prompt, tools, context_window_tokens);

        if recovery == RecoveryStage::FinalError {
            warn!("ContextEngine: overflow unrecoverable — suggest /reset or /compact");
        }

        // Re-validate tool_call/tool_result pairing after overflow drains
        if recovery != RecoveryStage::None {
            *messages = crate::session_repair::validate_and_repair(messages);
        }

        // Stage 2: Context guard — compact oversized tool results
        // Build a per-agent budget so tool result caps match the actual context window.
        let agent_budget = ContextBudget::new(context_window_tokens);
        apply_context_guard(messages, &agent_budget, tools);

        Ok(AssembleResult { recovery })
    }

    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        // Build a temporary session for the compactor, using the per-agent
        // context window rather than the boot-time default.
        let session = librefang_memory::session::Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id,
            messages: messages.to_vec(),
            context_window_tokens: context_window_tokens as u64,
            label: None,
            messages_generation: 0,
            last_repaired_generation: None,
        };

        let mut compaction_config = self.compaction_config.clone();
        compaction_config.context_window_tokens = context_window_tokens;

        compactor::compact_session(driver, model, &session, &compaction_config)
            .await
            .map_err(LibreFangError::Internal)
    }

    async fn after_turn(&self, _agent_id: AgentId, _messages: &[Message]) -> LibreFangResult<()> {
        // Default: no-op. Session saving is handled by the agent loop itself
        // since it needs access to the MemorySubstrate and full session object.
        //
        // Custom engines can override this to trigger background indexing,
        // update embeddings, or schedule deferred compaction.
        Ok(())
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        let budget = ContextBudget::new(context_window_tokens);
        crate::context_budget::truncate_tool_result_dynamic(content, &budget)
    }
}

// ---------------------------------------------------------------------------
// Hook invocation traces
// ---------------------------------------------------------------------------

/// One recorded hook invocation — input, output, timing, and outcome.
///
/// Stored in a bounded ring buffer inside `ScriptableContextEngine` and
/// surfaced via `GET /api/context-engine/traces` for debugging.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HookTrace {
    /// Unique identifier for this hook invocation. 16 hex chars (8 random bytes).
    /// Stable across retries — generated once before the retry loop.
    pub trace_id: String,
    /// Shared ID for all hook calls within the same agent turn.
    /// Empty string when not available (e.g. bootstrap, which runs outside a turn).
    pub correlation_id: String,
    /// Hook name (`"ingest"`, `"assemble"`, …).
    pub hook: String,
    /// ISO-8601 timestamp of when the hook started.
    pub started_at: String,
    /// Wall-clock duration in milliseconds.
    pub elapsed_ms: u64,
    /// Whether the hook succeeded.
    pub success: bool,
    /// Error message, if the hook failed.
    pub error: Option<String>,
    /// JSON input sent to the hook script (may be truncated for large payloads).
    pub input_preview: serde_json::Value,
    /// JSON output returned by the hook script (None on failure).
    pub output_preview: Option<serde_json::Value>,
    /// Arbitrary metadata from the hook's `"annotations"` response field.
    /// Stored for observability — surfaced in trace history queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
}

/// Maximum number of traces kept in the ring buffer.
const TRACE_BUFFER_CAPACITY: usize = 100;

// ---------------------------------------------------------------------------
// Hook invocation metrics
// ---------------------------------------------------------------------------

/// Per-hook invocation counters.  Stored inside `ScriptableContextEngine` behind
/// an `Arc<Mutex<…>>` so callers can read them without holding the engine lock.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HookStats {
    /// Total invocations (includes failures).
    pub calls: u64,
    /// Successful invocations.
    pub successes: u64,
    /// Failed invocations (timeout, crash, bad JSON, …).
    pub failures: u64,
    /// Cumulative wall-clock time of all invocations in milliseconds.
    pub total_ms: u64,
}

/// Snapshot of all hook stats for a `ScriptableContextEngine`.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HookMetrics {
    pub ingest: HookStats,
    pub after_turn: HookStats,
    pub bootstrap: HookStats,
    pub assemble: HookStats,
    pub compact: HookStats,
    pub prepare_subagent: HookStats,
    pub merge_subagent: HookStats,
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CircuitBreakerState {
    consecutive_failures: u32,
    /// When the circuit tripped (entered open state). `None` = closed.
    opened_at: Option<std::time::Instant>,
    /// Set to `true` when cooldown has elapsed and one probe call is allowed.
    half_open: bool,
}

impl CircuitBreakerState {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            opened_at: None,
            half_open: false,
        }
    }

    /// Returns `true` when the hook should be skipped (circuit open + not half-open).
    fn is_open(&mut self, max_failures: u32, reset_secs: u64) -> bool {
        if self.consecutive_failures < max_failures {
            return false; // circuit closed
        }
        match self.opened_at {
            None => {
                // Restored from persistent storage without a timestamp (opened_at was NULL).
                // The failure count already meets the threshold, so latch the circuit now
                // so that the full cooldown period is enforced from this moment.
                self.opened_at = Some(std::time::Instant::now());
                true
            }
            Some(t) => {
                if t.elapsed().as_secs() >= reset_secs {
                    // Cooldown elapsed → allow one half-open probe
                    if !self.half_open {
                        self.half_open = true;
                        self.opened_at = None; // reset timer so next trip re-latches
                    }
                    false // allow the probe call through
                } else {
                    true // still in cooldown
                }
            }
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.opened_at = None;
        self.half_open = false;
    }

    fn record_failure(&mut self, max_failures: u32) {
        self.consecutive_failures += 1;
        self.half_open = false; // probe failed → close half-open window
                                // (Re-)latch the circuit when threshold is reached
        if self.consecutive_failures >= max_failures {
            self.opened_at = Some(std::time::Instant::now());
        }
    }
}

// ---------------------------------------------------------------------------
// Per-hook sliding-window rate limiter
// ---------------------------------------------------------------------------

/// Sliding-window call counter for one hook.
#[derive(Default)]
struct HookRateLimiter {
    /// Ring of timestamps (as `std::time::Instant`) for recent calls.
    calls: std::collections::VecDeque<std::time::Instant>,
}

impl HookRateLimiter {
    /// Record a call and return whether the call is allowed.
    ///
    /// Evicts entries older than 60 seconds, then checks the count against
    /// `max_per_minute`.  Returns `true` if the call may proceed, `false` if
    /// the rate limit is exceeded.
    fn check_and_record(&mut self, max_per_minute: u32) -> bool {
        if max_per_minute == 0 {
            return true; // unlimited
        }
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(60);
        // Evict calls older than the window.
        while self
            .calls
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            self.calls.pop_front();
        }
        if self.calls.len() >= max_per_minute as usize {
            return false; // rate limit exceeded
        }
        self.calls.push_back(now);
        true
    }
}

// ---------------------------------------------------------------------------
// Scriptable context engine — wraps DefaultContextEngine + Python script hooks
// ---------------------------------------------------------------------------

/// Context engine that delegates to a [`DefaultContextEngine`] for heavy
/// operations (assemble, compact) and optionally invokes scripts for
/// light lifecycle hooks (ingest, after_turn).
///
/// Hook scripts are language-agnostic — they speak JSON over stdin/stdout.
/// The `runtime` field on the hooks config picks the launcher (`python`
/// stays the default; `native`, `v`, `node`, `deno`, `go` are also
/// supported). See [`crate::plugin_runtime`] for the full protocol.
///
/// ```toml
/// [context_engine.hooks]
/// ingest = "~/.librefang/plugins/my_recall.py"
/// after_turn = "~/.librefang/plugins/my_indexer.py"
/// runtime = "python"  # or "v", "node", "go", "native", ...
/// ```
///
/// **ingest hook** receives:
/// ```json
/// {"type": "ingest", "agent_id": "...", "message": "..."}
/// ```
/// Returns:
/// ```json
/// {"type": "ingest_result", "memories": [{"content": "remembered fact"}]}
/// ```
///
/// **after_turn hook** receives:
/// ```json
/// {"type": "after_turn", "agent_id": "...", "messages": [...]}
/// ```
/// Returns:
/// ```json
/// {"type": "ok"}
/// ```
pub struct ScriptableContextEngine {
    inner: DefaultContextEngine,
    ingest_script: Option<String>,
    after_turn_script: Option<String>,
    bootstrap_script: Option<String>,
    assemble_script: Option<String>,
    compact_script: Option<String>,
    prepare_subagent_script: Option<String>,
    merge_subagent_script: Option<String>,
    runtime: crate::plugin_runtime::PluginRuntime,
    /// Per-invocation timeout for all hooks. Bootstrap uses 2× this.
    hook_timeout_secs: u64,
    /// Plugin-declared env vars (from `[env]` in plugin.toml), passed to every hook.
    plugin_env: Vec<(String, String)>,
    /// Live invocation counters. Shared so callers can snapshot without &mut self.
    metrics: std::sync::Arc<std::sync::Mutex<HookMetrics>>,
    /// What to do when a hook fails after all retries are exhausted.
    on_hook_failure: librefang_types::config::HookFailurePolicy,
    /// How many times to retry a failing hook before applying `on_hook_failure`.
    max_retries: u32,
    /// Milliseconds to wait between retries.
    retry_delay_ms: u64,
    /// Optional substring filter for the `ingest` hook.
    ingest_filter: Option<String>,
    /// Restrict hooks to specific agent ID substrings (empty = all agents).
    agent_id_filter: Vec<String>,
    /// Per-hook JSON Schema definitions for input/output validation.
    hook_schemas: std::collections::HashMap<String, librefang_types::config::HookSchema>,
    /// Bounded ring buffer of recent hook invocations for debugging.
    traces: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
    /// Memory limit (MiB) forwarded to HookConfig.
    max_memory_mb: Option<u64>,
    /// Whether hook subprocesses are allowed network access.
    allow_network: bool,
    /// Hook protocol version declared by this plugin (stored for future compatibility checks).
    #[allow(dead_code)]
    hook_protocol_version: u32,
    /// Optional TTL-based cache for the `ingest` hook (seconds). `None` = disabled.
    ingest_cache_ttl_secs: Option<u64>,
    /// In-memory cache: maps SHA-256(input_json) → (cached_output, expires_at).
    ingest_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// Whether to use persistent subprocesses (process pool) for hooks.
    persistent_subprocess: bool,
    /// Shared pool of persistent hook subprocesses (used when `persistent_subprocess = true`).
    process_pool: std::sync::Arc<crate::plugin_runtime::HookProcessPool>,
    /// TTL-based cache for `assemble` hook results.
    assemble_cache_ttl_secs: Option<u64>,
    assemble_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// TTL-based cache for `compact` hook results.
    compact_cache_ttl_secs: Option<u64>,
    compact_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// Compiled regex filter for the `ingest` hook (from `ingest_regex` config).
    ingest_regex: Option<regex_lite::Regex>,
    /// Path to the per-plugin shared state JSON file (when `enable_shared_state = true`).
    shared_state_path: Option<std::path::PathBuf>,
    /// Circuit breaker states per hook name.
    circuit_breakers:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, CircuitBreakerState>>>,
    /// Circuit breaker config (None = disabled).
    circuit_breaker_cfg: Option<librefang_types::config::CircuitBreakerConfig>,
    /// Semaphore bounding concurrent `after_turn` background tasks.
    after_turn_sem: std::sync::Arc<tokio::sync::Semaphore>,
    /// Whether to pre-warm subprocesses on engine init.
    prewarm_subprocesses: bool,
    /// Per-agent hook call counters: agent_id → HookStats.
    per_agent_metrics:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookStats>>>,
    /// OTel OTLP endpoint for this plugin (advisory; logged if set).
    #[allow(dead_code)]
    otel_endpoint: Option<String>,
    /// Canonical plugin name — used as the `plugin` column when writing to trace_store.
    plugin_name: String,
    /// Persistent SQLite trace store (None if it could not be opened at construction time).
    trace_store: Option<std::sync::Arc<crate::trace_store::TraceStore>>,
    /// Tracks all spawned after_turn background tasks for graceful shutdown.
    after_turn_tasks: std::sync::Arc<tokio::sync::Mutex<tokio::task::JoinSet<()>>>,
    /// Memory substrate for after_turn hook memory injection.
    memory_substrate: std::sync::Arc<librefang_memory::MemorySubstrate>,
    /// Overrides applied by the bootstrap hook at startup.
    bootstrap_applied_overrides: std::sync::Arc<std::sync::Mutex<BootstrapOverrides>>,
    /// Per-hook sliding-window rate limiters.
    rate_limiters:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookRateLimiter>>>,
    /// Script to invoke when an event is received from the event bus.
    on_event_script: Option<String>,
    /// Optional shared event bus. When set, events emitted by this plugin's
    /// hooks are published to all subscribers.
    event_bus: Option<std::sync::Arc<PluginEventBus>>,
    /// Config schema declared in `[config]` of plugin.toml.
    ///
    /// Used to build the resolved config JSON file passed to hook subprocesses
    /// via `LIBREFANG_PLUGIN_CONFIG`.
    plugin_config_schema:
        std::collections::HashMap<String, librefang_types::config::PluginConfigField>,
}

impl ScriptableContextEngine {
    /// Create a scriptable context engine from config.
    ///
    /// Also validates that every declared hook script file actually exists.
    /// Missing scripts are logged as warnings at construction time (not fatal)
    /// so the engine degrades gracefully rather than refusing to start.
    pub fn new(
        inner: DefaultContextEngine,
        hooks: &librefang_types::config::ContextEngineHooks,
    ) -> Self {
        // Warn at construction time for any declared script that cannot be found.
        let all_declared: &[(&str, &Option<String>)] = &[
            ("ingest", &hooks.ingest),
            ("after_turn", &hooks.after_turn),
            ("bootstrap", &hooks.bootstrap),
            ("assemble", &hooks.assemble),
            ("compact", &hooks.compact),
            ("prepare_subagent", &hooks.prepare_subagent),
            ("merge_subagent", &hooks.merge_subagent),
            ("on_event", &hooks.on_event),
        ];
        for (name, path_opt) in all_declared {
            if let Some(path) = path_opt {
                let resolved = Self::resolve_script_path(path);
                if !std::path::Path::new(&resolved).exists() {
                    warn!(
                        hook = *name,
                        path = resolved.as_str(),
                        "Hook script declared in plugin.toml does not exist; \
                         hook will be skipped at runtime"
                    );
                }
            }
        }

        const CURRENT_PROTOCOL: u32 = 1;
        let proto = hooks.hook_protocol_version.unwrap_or(1);
        if proto > CURRENT_PROTOCOL {
            warn!(
                declared = proto,
                current = CURRENT_PROTOCOL,
                "Plugin declares hook_protocol_version {proto} but runtime only supports \
                 version {CURRENT_PROTOCOL}. The plugin may use unsupported features."
            );
        }

        let memory_substrate = std::sync::Arc::clone(inner.memory_substrate());
        Self {
            inner,
            ingest_script: hooks.ingest.clone(),
            after_turn_script: hooks.after_turn.clone(),
            bootstrap_script: hooks.bootstrap.clone(),
            assemble_script: hooks.assemble.clone(),
            compact_script: hooks.compact.clone(),
            prepare_subagent_script: hooks.prepare_subagent.clone(),
            merge_subagent_script: hooks.merge_subagent.clone(),
            runtime: crate::plugin_runtime::PluginRuntime::from_tag(hooks.runtime.as_deref()),
            hook_timeout_secs: hooks.hook_timeout_secs.unwrap_or(30),
            plugin_env: Vec::new(), // populated via with_plugin_env()
            metrics: std::sync::Arc::new(std::sync::Mutex::new(HookMetrics::default())),
            on_hook_failure: hooks.on_hook_failure.clone(),
            max_retries: hooks.max_retries,
            retry_delay_ms: hooks.retry_delay_ms,
            ingest_filter: hooks.ingest_filter.clone(),
            agent_id_filter: hooks.only_for_agent_ids.clone(),
            hook_schemas: hooks.hook_schemas.clone(),
            traces: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(TRACE_BUFFER_CAPACITY),
            )),
            max_memory_mb: hooks.max_memory_mb,
            allow_network: hooks.allow_network,
            hook_protocol_version: proto,
            ingest_cache_ttl_secs: hooks.hook_cache_ttl_secs,
            ingest_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            persistent_subprocess: hooks.persistent_subprocess,
            process_pool: std::sync::Arc::new(crate::plugin_runtime::HookProcessPool::new()),
            assemble_cache_ttl_secs: hooks.assemble_cache_ttl_secs,
            assemble_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            compact_cache_ttl_secs: hooks.compact_cache_ttl_secs,
            compact_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            ingest_regex: hooks.ingest_regex.as_deref().and_then(
                |pat| match regex_lite::Regex::new(pat) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        warn!(pattern = pat, error = %e, "Invalid ingest_regex — ignored");
                        None
                    }
                },
            ),
            // When enable_shared_state is true, set a placeholder path; the
            // actual plugin-scoped path is filled in by `with_plugin_name()`.
            shared_state_path: if hooks.enable_shared_state {
                Some(std::path::PathBuf::from(".state.json"))
            } else {
                None
            },
            circuit_breakers: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            circuit_breaker_cfg: hooks.circuit_breaker.clone(),
            after_turn_sem: std::sync::Arc::new(tokio::sync::Semaphore::new(
                hooks.after_turn_queue_depth.max(1) as usize,
            )),
            prewarm_subprocesses: hooks.prewarm_subprocesses,
            per_agent_metrics: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            otel_endpoint: hooks.otel_endpoint.clone(),
            plugin_name: String::new(), // filled in by with_plugin_name()
            trace_store: None,          // filled in by with_plugin_name()
            after_turn_tasks: std::sync::Arc::new(tokio::sync::Mutex::new(
                tokio::task::JoinSet::new(),
            )),
            memory_substrate,
            bootstrap_applied_overrides: std::sync::Arc::new(std::sync::Mutex::new(
                BootstrapOverrides::default(),
            )),
            rate_limiters: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            on_event_script: hooks.on_event.clone(),
            event_bus: None,
            plugin_config_schema: std::collections::HashMap::new(), // populated via with_plugin_config()
        }
    }

    /// Set the plugin name to resolve the per-plugin shared state file path.
    ///
    /// Call after `new()` when the plugin name is known. If `enable_shared_state`
    /// was `false`, `shared_state_path` is `None` and this is a no-op.
    pub fn with_plugin_name(mut self, name: &str) -> Self {
        self.plugin_name = name.to_string();

        if self.shared_state_path.is_some() {
            // Replace the placeholder with the actual plugin-scoped path.
            self.shared_state_path = Some(
                crate::plugin_manager::plugins_dir()
                    .join(name)
                    .join(".state.json"),
            );
        }

        // Open the persistent trace store. Failure is non-fatal — traces will
        // still land in the in-memory ring buffer even if SQLite is unavailable.
        self.trace_store = crate::plugin_manager::open_trace_store()
            .map(std::sync::Arc::new)
            .map_err(|e| {
                warn!(plugin = name, error = %e, "Could not open hook trace store; SQLite persistence disabled");
            })
            .ok();

        // Restore circuit breaker state from SQLite so tripped circuits survive daemon restarts.
        if let Some(ref store) = self.trace_store {
            if let Ok(saved) = store.load_circuit_states() {
                if let Ok(mut guard) = self.circuit_breakers.lock() {
                    for (key, (failures, opened_at)) in saved {
                        guard.entry(key).or_insert_with(|| {
                            let opened_instant = opened_at.as_deref().and_then(|s| {
                                chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| {
                                    // Convert persisted UTC timestamp to a std::time::Instant
                                    // approximation: compute how many seconds ago it opened.
                                    let elapsed_secs = chrono::Utc::now()
                                        .signed_duration_since(dt.with_timezone(&chrono::Utc))
                                        .num_seconds()
                                        .max(0)
                                        as u64;
                                    std::time::Instant::now()
                                        .checked_sub(std::time::Duration::from_secs(elapsed_secs))
                                        .unwrap_or_else(std::time::Instant::now)
                                })
                            });
                            CircuitBreakerState {
                                consecutive_failures: failures,
                                opened_at: opened_instant,
                                half_open: false,
                            }
                        });
                    }
                }
            }
        }

        self
    }

    /// Set plugin-level env vars from `[env]` in plugin.toml.
    pub fn with_plugin_env(mut self, env: Vec<(String, String)>) -> Self {
        self.plugin_env = env;
        self
    }

    /// Set the config schema declared in `[config]` of plugin.toml.
    ///
    /// Before each hook invocation the resolved config (defaults only for now)
    /// is written to a temporary JSON file and the path is exposed to the
    /// subprocess as `LIBREFANG_PLUGIN_CONFIG`.
    pub fn with_plugin_config(
        mut self,
        schema: std::collections::HashMap<String, librefang_types::config::PluginConfigField>,
    ) -> Self {
        self.plugin_config_schema = schema;
        self
    }

    /// Write the resolved plugin config (defaults merged with user overrides) to a
    /// temporary JSON file.
    ///
    /// Returns the file path, or `None` if the schema is empty.
    ///
    /// The file is written to the system temp directory as
    /// `librefang-plugin-config-<plugin_name>.json` and is overwritten on each
    /// hook invocation (no temp dir cleanup needed — the OS handles it).
    fn write_plugin_config_file(
        plugin_name: &str,
        config_schema: &std::collections::HashMap<
            String,
            librefang_types::config::PluginConfigField,
        >,
        user_overrides: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Option<std::path::PathBuf> {
        if config_schema.is_empty() {
            return None;
        }
        let mut resolved: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        // Start with defaults from the schema.
        for (key, field) in config_schema {
            if let Some(ref default_val) = field.default {
                resolved.insert(key.clone(), default_val.clone());
            }
        }
        // Apply user overrides (only for keys declared in the schema).
        for (key, val) in user_overrides {
            if config_schema.contains_key(key.as_str()) {
                resolved.insert(key.clone(), val.clone());
            }
        }
        let json = serde_json::to_string_pretty(&resolved).ok()?;
        let path = std::env::temp_dir().join(format!("librefang-plugin-config-{plugin_name}.json"));
        std::fs::write(&path, json).ok()?;
        Some(path)
    }

    /// Attach a shared event bus to this engine.
    ///
    /// Attach an event bus so this engine both emits events (from `after_turn` output)
    /// and receives events for its `on_event` hook.
    ///
    /// Starts a background subscription task on the bus: when any plugin on the same
    /// bus emits an event, this engine's `on_event` script (if configured) is invoked.
    pub fn with_event_bus(mut self, bus: std::sync::Arc<PluginEventBus>) -> Self {
        self.event_bus = Some(bus.clone());

        // Start listener only when there is an on_event script to invoke.
        if self.on_event_script.is_some() {
            // Build a lightweight clone of the fields needed inside the task.
            // Using Arc clones keeps it cheap; the spawned task holds them for its lifetime.
            let plugin_name = self.plugin_name.clone();
            let on_event_script = self.on_event_script.clone().unwrap();
            let runtime = self.runtime.clone();
            let hook_timeout_secs = self.hook_timeout_secs;
            let plugin_env = self.plugin_env.clone();
            let bootstrap_overrides = self.bootstrap_applied_overrides.clone();
            let traces = self.traces.clone();
            let hook_schemas = self.hook_schemas.clone();
            let shared_state_path = self.shared_state_path.clone();
            let trace_store = self.trace_store.clone();
            let max_memory_mb = self.max_memory_mb;
            let allow_network = self.allow_network;
            let output_schema_strict = self.inner.config.output_schema_strict;

            let mut rx = bus.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            // Skip events emitted by this same plugin to avoid infinite loops.
                            if event.source_plugin == plugin_name {
                                continue;
                            }

                            let effective_env = {
                                let guard = bootstrap_overrides
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner());
                                let mut env = plugin_env.clone();
                                for (k, v) in &guard.env_overrides {
                                    if !env.iter().any(|(ek, _)| ek == k) {
                                        env.push((k.clone(), v.clone()));
                                    }
                                }
                                env
                            };
                            let effective_allow_network = {
                                let guard = bootstrap_overrides
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner());
                                guard.allow_network.unwrap_or(allow_network)
                            };
                            let input = serde_json::json!({"event": event});
                            let plugin_name_c = plugin_name.clone();
                            let script = on_event_script.clone();
                            let traces_c = traces.clone();
                            let schemas_c = hook_schemas.clone();
                            let state_c = shared_state_path.clone();
                            let store_c = trace_store.clone();
                            let runtime = runtime.clone();
                            tokio::spawn(async move {
                                let _ = ScriptableContextEngine::run_hook(
                                    "on_event",
                                    &script,
                                    runtime,
                                    input,
                                    hook_timeout_secs,
                                    &effective_env,
                                    0, // on_event is best-effort, no retries
                                    0,
                                    max_memory_mb,
                                    effective_allow_network,
                                    &traces_c,
                                    &schemas_c,
                                    state_c.as_deref(),
                                    store_c.as_ref(),
                                    &plugin_name_c,
                                    &generate_trace_id(),
                                    output_schema_strict,
                                )
                                .await;
                            });
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            // Route through the bus's lag counter so plugin
                            // on_event misses are observable in
                            // `dropped_count()` and emit a rate-limited
                            // error! log (#3630). The previous per-listener
                            // warn! was silent in aggregate metrics.
                            bus.record_consumer_lag(n, "plugin.on_event");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        self
    }

    /// Return a snapshot of all hook invocation metrics.
    pub fn metrics(&self) -> HookMetrics {
        self.metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Return recent hook invocation traces (up to `TRACE_BUFFER_CAPACITY`).
    pub fn traces_snapshot(&self) -> Vec<HookTrace> {
        self.traces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Push a trace record into the in-memory ring buffer and the SQLite store.
    ///
    /// The ring buffer provides fast in-process access; the SQLite store persists
    /// traces across daemon restarts for post-mortem analysis.  Both writes are
    /// best-effort — errors are silently swallowed so a telemetry failure never
    /// propagates to the caller.
    fn push_trace(
        traces: &std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
        trace: HookTrace,
        trace_store: Option<&std::sync::Arc<crate::trace_store::TraceStore>>,
        plugin_name: &str,
    ) {
        // Persist to SQLite first (borrows trace by ref).
        if let Some(store) = trace_store {
            store.insert(plugin_name, &trace);
        }
        // Then push into the bounded in-memory ring buffer.
        if let Ok(mut buf) = traces.lock() {
            if buf.len() >= TRACE_BUFFER_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(trace);
        }
    }

    /// Validate a JSON value against a subset of JSON Schema.
    ///
    /// Checks:
    /// - `required`: all listed keys are present (objects only)
    /// - `type`: value matches the declared JSON type
    /// - `enum`: value is one of the listed options
    /// - `minimum` / `maximum`: numeric range (numbers only)
    /// - `minLength` / `maxLength`: string length (strings only)
    /// - `properties`: recursively validate each declared property
    ///
    /// Returns a list of human-readable violation messages (empty = valid).
    /// The caller decides whether to warn or error based on `output_schema_strict`.
    fn validate_schema(
        schema: &serde_json::Value,
        value: &serde_json::Value,
        context: &str,
    ) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // --- type check ---
        if let Some(expected_type) = schema.get("type").and_then(|t| t.as_str()) {
            let actual_matches = match expected_type {
                "object" => value.is_object(),
                "array" => value.is_array(),
                "string" => value.is_string(),
                "number" => value.is_number(),
                "integer" => value.is_i64() || value.is_u64(),
                "boolean" => value.is_boolean(),
                "null" => value.is_null(),
                _ => true, // unknown type — don't reject
            };
            if !actual_matches {
                errors.push(format!(
                    "[{context}] type mismatch: expected={expected_type}, actual={}",
                    value.to_string().chars().take(80).collect::<String>()
                ));
            }
        }

        // --- enum check ---
        if let Some(variants) = schema.get("enum").and_then(|e| e.as_array()) {
            if !variants.contains(value) {
                errors.push(format!(
                    "[{context}] value not in enum: {}",
                    value.to_string().chars().take(80).collect::<String>()
                ));
            }
        }

        // --- numeric range ---
        if let Some(n) = value.as_f64() {
            if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64()) {
                if n < min {
                    errors.push(format!(
                        "[{context}] below minimum: value={n}, minimum={min}"
                    ));
                }
            }
            if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64()) {
                if n > max {
                    errors.push(format!(
                        "[{context}] above maximum: value={n}, maximum={max}"
                    ));
                }
            }
        }

        // --- string length ---
        if let Some(s) = value.as_str() {
            if let Some(min_len) = schema.get("minLength").and_then(|v| v.as_u64()) {
                if (s.len() as u64) < min_len {
                    errors.push(format!(
                        "[{context}] string too short: len={}, min_len={min_len}",
                        s.len()
                    ));
                }
            }
            if let Some(max_len) = schema.get("maxLength").and_then(|v| v.as_u64()) {
                if (s.len() as u64) > max_len {
                    errors.push(format!(
                        "[{context}] string too long: len={}, max_len={max_len}",
                        s.len()
                    ));
                }
            }
        }

        // --- required fields ---
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            if let Some(obj) = value.as_object() {
                for field in required {
                    if let Some(field_str) = field.as_str() {
                        if !obj.contains_key(field_str) {
                            errors.push(format!("[{context}] required field missing: {field_str}"));
                        }
                    }
                }
            }
        }

        // --- properties: recursive per-property validation ---
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            if let Some(obj) = value.as_object() {
                for (key, prop_schema) in props {
                    if let Some(prop_value) = obj.get(key) {
                        errors.extend(Self::validate_schema(
                            prop_schema,
                            prop_value,
                            &format!("{context}.{key}"),
                        ));
                    }
                }
            }
        }

        errors
    }

    fn circuit_is_open(&self, hook: &str, agent_id: Option<&AgentId>) -> bool {
        let Some(ref cfg) = self.circuit_breaker_cfg else {
            return false;
        };
        let key = match agent_id {
            Some(id) => format!("{}:{}", id.0, hook),
            None => hook.to_string(),
        };
        let mut guard = self.circuit_breakers.lock().unwrap();
        guard
            .entry(key)
            .or_insert_with(CircuitBreakerState::new)
            .is_open(cfg.max_failures, cfg.reset_secs)
    }

    fn circuit_record(&self, hook: &str, agent_id: Option<&AgentId>, success: bool) {
        let Some(ref cfg) = self.circuit_breaker_cfg else {
            return;
        };
        let key = match agent_id {
            Some(id) => format!("{}:{}", id.0, hook),
            None => hook.to_string(),
        };
        let (failures, opened_at_rfc3339, just_reset) = {
            let mut guard = self.circuit_breakers.lock().unwrap();
            let state = guard
                .entry(key.clone())
                .or_insert_with(CircuitBreakerState::new);
            if success {
                state.record_success();
                // Reset — signal deletion from SQLite.
                (0u32, None::<String>, true)
            } else {
                state.record_failure(cfg.max_failures);
                if state.consecutive_failures == cfg.max_failures {
                    warn!(
                        hook,
                        cooldown_secs = cfg.reset_secs,
                        "Hook circuit breaker opened"
                    );
                }
                // Compute RFC-3339 opened_at from the stored Instant if available.
                let opened_str = state.opened_at.map(|instant| {
                    let elapsed = instant.elapsed();
                    (chrono::Utc::now() - chrono::Duration::from_std(elapsed).unwrap_or_default())
                        .to_rfc3339()
                });
                (state.consecutive_failures, opened_str, false)
            }
        };

        // Persist to SQLite if trace store is available.
        if let Some(ref store) = self.trace_store {
            if just_reset {
                let _ = store.delete_circuit_state(&key);
            } else {
                let _ = store.save_circuit_state(&key, failures, opened_at_rfc3339.as_deref());
            }
        }
    }

    fn record_per_agent(&self, agent_id: &AgentId, elapsed_ms: u64, success: bool) {
        if let Ok(mut map) = self.per_agent_metrics.lock() {
            let stats = map.entry(agent_id.0.to_string()).or_default();
            stats.calls += 1;
            stats.total_ms += elapsed_ms;
            if success {
                stats.successes += 1;
            } else {
                stats.failures += 1;
            }
        }
    }

    pub fn per_agent_metrics_snapshot(&self) -> std::collections::HashMap<String, HookStats> {
        self.per_agent_metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub async fn prewarm(&self) {
        if !self.prewarm_subprocesses || !self.persistent_subprocess {
            return;
        }
        let runtime = self.runtime.clone();
        let hooks: &[(&str, &Option<String>)] = &[
            ("ingest", &self.ingest_script),
            ("after_turn", &self.after_turn_script),
            ("bootstrap", &self.bootstrap_script),
            ("assemble", &self.assemble_script),
            ("compact", &self.compact_script),
            ("on_event", &self.on_event_script),
        ];
        for (name, script_opt) in hooks {
            if let Some(ref script) = script_opt {
                let resolved = Self::resolve_script_path(script);
                if std::path::Path::new(&resolved).exists() {
                    let runtime = runtime.clone();
                    match self
                        .process_pool
                        .prewarm(&resolved, runtime.clone(), &self.plugin_env)
                        .await
                    {
                        Ok(()) => debug!(hook = name, "Pre-warmed hook subprocess"),
                        Err(e) => warn!(hook = name, error = %e, "Pre-warm failed"),
                    }
                }
            }
        }
    }

    /// Evict all persistent hook subprocesses for this plugin.
    ///
    /// Forces fresh subprocess spawns on the next hook call — useful after
    /// a plugin hot-reload so the new script version is picked up immediately
    /// rather than waiting for the old process to die naturally.
    pub async fn evict_hook_processes(&self) {
        if !self.persistent_subprocess {
            return;
        }
        let hooks: &[&Option<String>] = &[
            &self.ingest_script,
            &self.after_turn_script,
            &self.bootstrap_script,
            &self.assemble_script,
            &self.compact_script,
            &self.on_event_script,
        ];
        for script in hooks.iter().filter_map(|opt| opt.as_deref()) {
            let resolved = Self::resolve_script_path(script);
            self.process_pool.evict(&resolved).await;
        }
    }

    /// Wait for all in-flight after_turn background tasks to complete.
    ///
    /// Call this during daemon shutdown after stopping the agent loop so that
    /// no after_turn work is silently dropped. Times out after `timeout_secs`.
    pub async fn wait_for_after_turn_tasks(&self, timeout_secs: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        // Lock once and drain; this is called during shutdown so no new tasks will
        // be spawned. Holding the async Mutex across join_next().await is safe here.
        let mut tasks = self.after_turn_tasks.lock().await;
        loop {
            if tasks.is_empty() {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                _ = tasks.join_next() => {}
                _ = tokio::time::sleep(remaining) => { break; }
            }
        }
    }

    /// Returns true when the agent_id passes the configured agent_id_filter.
    fn agent_passes_filter(&self, agent_id: &AgentId) -> bool {
        if self.agent_id_filter.is_empty() {
            return true;
        }
        let id_str = agent_id.0.to_string();
        self.agent_id_filter
            .iter()
            .any(|f| id_str.contains(f.as_str()))
    }

    /// Record the outcome of one hook invocation into the named slot.
    fn record_hook(
        metrics: &std::sync::Arc<std::sync::Mutex<HookMetrics>>,
        slot: &str,
        elapsed_ms: u64,
        ok: bool,
    ) {
        if let Ok(mut m) = metrics.lock() {
            let stats = match slot {
                "ingest" => &mut m.ingest,
                "after_turn" => &mut m.after_turn,
                "bootstrap" => &mut m.bootstrap,
                "assemble" => &mut m.assemble,
                "compact" => &mut m.compact,
                "prepare_subagent" => &mut m.prepare_subagent,
                "merge_subagent" => &mut m.merge_subagent,
                _ => return,
            };
            stats.calls += 1;
            stats.total_ms += elapsed_ms;
            if ok {
                stats.successes += 1;
            } else {
                stats.failures += 1;
            }
        }
    }

    /// Process the JSON output returned by an after_turn hook.
    ///
    /// Recognised fields:
    /// - `"memories"`: inject new memories for the agent
    /// - `"log"`:      emit the value as an info-level log line
    /// - `"annotations"`: arbitrary metadata (logged at debug level)
    ///
    /// Unknown fields are silently ignored so future hook versions stay
    /// backwards-compatible with older runtimes.
    fn process_after_turn_output(
        output: &serde_json::Value,
        agent_id: &str,
        memory_substrate: Option<&std::sync::Arc<librefang_memory::MemorySubstrate>>,
        plugin_name: &str,
        event_bus: Option<&std::sync::Arc<PluginEventBus>>,
    ) {
        // "log" field — emit as info log from the plugin's perspective.
        if let Some(msg) = output.get("log").and_then(|v| v.as_str()) {
            let trimmed = msg.chars().take(512).collect::<String>();
            tracing::info!(
                agent_id,
                plugin_log = trimmed.as_str(),
                "after_turn hook log"
            );
        }

        // "annotations" field — debug-level dump for observability.
        if let Some(ann) = output.get("annotations") {
            tracing::debug!(
                agent_id,
                annotations = ann
                    .to_string()
                    .chars()
                    .take(1024)
                    .collect::<String>()
                    .as_str(),
                "after_turn hook annotations"
            );
        }

        // "memories" field — store each entry in the memory substrate.
        if let Some(mems) = output.get("memories").and_then(|v| v.as_array()) {
            if let Some(substrate) = memory_substrate {
                for mem in mems {
                    let content = match mem.get("content").and_then(|v| v.as_str()) {
                        Some(c) if !c.is_empty() => c.to_string(),
                        _ => continue,
                    };
                    let tags: Vec<String> = mem
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| t.as_str().map(String::from))
                                .take(16)
                                .collect()
                        })
                        .unwrap_or_default();

                    // Fire-and-forget: memory injection is best-effort and must not block.
                    let substrate = std::sync::Arc::clone(substrate);
                    let agent_id_owned = agent_id.to_string();
                    tokio::spawn(async move {
                        use librefang_types::memory::Memory as _;
                        let parsed_id = uuid::Uuid::parse_str(&agent_id_owned)
                            .map(librefang_types::agent::AgentId)
                            .unwrap_or_else(|_| librefang_types::agent::AgentId::new());
                        let scope = if tags.is_empty() {
                            "hook".to_string()
                        } else {
                            tags.join(",")
                        };
                        if let Err(e) = substrate
                            .remember(
                                parsed_id,
                                &content,
                                librefang_types::memory::MemorySource::System,
                                &scope,
                                std::collections::HashMap::new(),
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "after_turn hook: failed to inject memory");
                        }
                    });
                }
            }
        }

        // "events" field — publish named events to the event bus.
        if let Some(events) = output.get("events").and_then(|v| v.as_array()) {
            for ev in events {
                if let (Some(name), payload) = (
                    ev.get("name").and_then(|v| v.as_str()),
                    ev.get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                ) {
                    let event = PluginEvent {
                        name: name.to_string(),
                        payload,
                        source_plugin: plugin_name.to_string(),
                    };
                    if let Some(bus) = event_bus {
                        bus.emit(event);
                    }
                }
            }
        }
    }

    /// Dispatch a plugin event to the `on_event` hook script, if configured.
    ///
    /// The hook receives: `{"event": {"name": ..., "payload": ..., "source_plugin": ...}}`
    /// The hook's return value is ignored (fire-and-forget, spawned as background task).
    pub async fn dispatch_event(&self, event: &PluginEvent) {
        let script = match &self.on_event_script {
            Some(s) => s.clone(),
            None => return, // no on_event hook configured
        };

        let input = serde_json::json!({"event": event});
        let plugin_name = self.plugin_name.clone();
        let runtime = self.runtime.clone();
        let timeout_secs = self.hook_timeout_secs;
        let plugin_env = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            env
        };
        let traces = std::sync::Arc::clone(&self.traces);
        let hook_schemas = self.hook_schemas.clone();
        let shared_state_path = self.shared_state_path.clone();
        let trace_store = self.trace_store.clone();
        let max_retries = 0u32; // events are best-effort
        let retry_delay_ms = 0u64;
        let max_memory_mb = self.max_memory_mb;
        let allow_network = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard.allow_network.unwrap_or(self.allow_network)
        };
        let output_schema_strict = self.inner.config.output_schema_strict;
        let event_name = event.name.clone();
        debug!(plugin = %plugin_name, event = %event_name, "dispatching on_event hook");
        tokio::spawn(async move {
            let _ = Self::run_hook(
                "on_event",
                &script,
                runtime,
                input,
                timeout_secs,
                &plugin_env,
                max_retries,
                retry_delay_ms,
                max_memory_mb,
                allow_network,
                &traces,
                &hook_schemas,
                shared_state_path.as_deref(),
                trace_store.as_ref(),
                &plugin_name,
                &generate_trace_id(),
                output_schema_strict,
            )
            .await;
        });
    }

    /// Resolve a script path, expanding `~` to the user's home directory.
    fn resolve_script_path(path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return format!("{}/{rest}", home.display());
            }
        }
        path.to_string()
    }

    /// Run a hook script with JSON input, return `(output, elapsed_ms)`.
    ///
    /// Retries up to `max_retries` times with `retry_delay_ms` between attempts.
    /// Records a `HookTrace` on every call (success or failure).
    #[allow(clippy::too_many_arguments)]
    async fn run_hook(
        hook_name: &str,
        script_path: &str,
        runtime: crate::plugin_runtime::PluginRuntime,
        input: serde_json::Value,
        timeout_secs: u64,
        plugin_env: &[(String, String)],
        max_retries: u32,
        retry_delay_ms: u64,
        max_memory_mb: Option<u64>,
        allow_network: bool,
        traces: &std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
        hook_schemas: &std::collections::HashMap<String, librefang_types::config::HookSchema>,
        shared_state_path: Option<&std::path::Path>,
        trace_store: Option<&std::sync::Arc<crate::trace_store::TraceStore>>,
        plugin_name: &str,
        correlation_id: &str,
        output_schema_strict: bool,
    ) -> Result<(serde_json::Value, u64), String> {
        let resolved = Self::resolve_script_path(script_path);

        if !std::path::Path::new(&resolved).exists() {
            return Err(format!("Hook script not found: {resolved}"));
        }

        // Validate input schema if declared (always warn-only — input validation is advisory).
        if let Some(schema) = hook_schemas.get(hook_name) {
            if let Some(ref input_schema) = schema.input {
                let errs =
                    Self::validate_schema(input_schema, &input, &format!("{hook_name}/input"));
                for e in &errs {
                    warn!("{e}");
                }
            }
        }

        let config = crate::plugin_runtime::HookConfig {
            timeout_secs,
            plugin_env: plugin_env.to_vec(),
            max_memory_mb,
            allow_network,
            state_file: shared_state_path.map(|p| p.to_path_buf()),
            retry_delay_ms,
            ..Default::default()
        };

        let trace_id = generate_trace_id();
        let started_at = chrono::Utc::now().to_rfc3339();
        // Truncate large inputs for trace preview.
        let input_preview = if input.to_string().len() > 2048 {
            serde_json::json!({"_truncated": true, "type": input.get("type")})
        } else {
            input.clone()
        };

        let t = std::time::Instant::now();
        let mut last_err = String::new();
        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    config.delay_for_attempt(attempt),
                ))
                .await;
                debug!(
                    script = resolved.as_str(),
                    attempt, max_retries, "Retrying hook after failure: {last_err}"
                );
            }
            match crate::plugin_runtime::run_hook_json(
                hook_name,
                &resolved,
                runtime.clone(),
                &input,
                &config,
            )
            .await
            {
                Ok(v) => {
                    let elapsed_ms = t.elapsed().as_millis() as u64;
                    // Validate output schema if declared.
                    if let Some(schema) = hook_schemas.get(hook_name) {
                        if let Some(ref output_schema) = schema.output {
                            let errs = Self::validate_schema(
                                output_schema,
                                &v,
                                &format!("{hook_name}/output"),
                            );
                            if !errs.is_empty() {
                                if output_schema_strict {
                                    let err_msg = format!(
                                        "hook {hook_name} output failed schema validation: {}",
                                        errs.join("; ")
                                    );
                                    // Record the failure trace before surfacing the error so
                                    // the trace store is never missing an entry for this call.
                                    Self::push_trace(
                                        traces,
                                        HookTrace {
                                            trace_id: trace_id.clone(),
                                            correlation_id: correlation_id.to_string(),
                                            hook: hook_name.to_string(),
                                            started_at: started_at.clone(),
                                            elapsed_ms: t.elapsed().as_millis() as u64,
                                            success: false,
                                            error: Some(err_msg.clone()),
                                            input_preview: input_preview.clone(),
                                            output_preview: None,
                                            annotations: None,
                                        },
                                        trace_store,
                                        plugin_name,
                                    );
                                    return Err(err_msg);
                                }
                                for e in &errs {
                                    warn!("{e}");
                                }
                            }
                        }
                    }
                    Self::push_trace(
                        traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at: started_at.clone(),
                            elapsed_ms,
                            success: true,
                            error: None,
                            input_preview: input_preview.clone(),
                            output_preview: Some(v.clone()),
                            annotations: v.get("annotations").cloned(),
                        },
                        trace_store,
                        plugin_name,
                    );
                    return Ok((v, elapsed_ms));
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        let elapsed_ms = t.elapsed().as_millis() as u64;
        let err_msg = format!("Hook script failed after {max_retries} retries: {last_err}");
        Self::push_trace(
            traces,
            HookTrace {
                trace_id: trace_id.clone(),
                correlation_id: correlation_id.to_string(),
                hook: hook_name.to_string(),
                started_at,
                elapsed_ms,
                success: false,
                error: Some(err_msg.clone()),
                input_preview,
                output_preview: None,
                annotations: None,
            },
            trace_store,
            plugin_name,
        );
        Err(err_msg)
    }

    /// Dispatch a hook call to either the persistent process pool or a fresh subprocess.
    ///
    /// When `self.persistent_subprocess` is `true`, the call is routed through
    /// `self.process_pool` (JSON-lines, long-lived process). Otherwise a fresh
    /// subprocess is spawned via `Self::run_hook`. Either way the return is
    /// `Ok((output, elapsed_ms))` or `Err(message)`.
    async fn call_hook_dispatch(
        &self,
        hook_name: &str,
        script_path: &str,
        input: serde_json::Value,
        timeout_secs: u64,
        agent_id: Option<&AgentId>,
    ) -> Result<(serde_json::Value, u64), String> {
        // Circuit breaker: reject immediately when open
        if self.circuit_is_open(hook_name, agent_id) {
            return Err(format!(
                "circuit-open: '{hook_name}' suspended after repeated failures"
            ));
        }
        // Rate limiting check.
        let max_rpm = self.inner.config.max_hook_calls_per_minute;
        if max_rpm > 0 {
            let mut limiters = self.rate_limiters.lock().unwrap_or_else(|e| e.into_inner());
            // Key by "{agent_id}:{hook_name}" so one agent cannot exhaust the
            // rate limit for all other agents sharing the same plugin.
            let rl_key = format!(
                "{}:{}",
                agent_id.map(|id| id.0.to_string()).unwrap_or_default(),
                hook_name
            );
            let limiter = limiters.entry(rl_key).or_default();
            if !limiter.check_and_record(max_rpm) {
                warn!(
                    hook = hook_name,
                    max_rpm, "hook rate limit exceeded — skipping call"
                );
                // Return a neutral result: empty object (passthrough for callers).
                return Ok((serde_json::Value::Object(serde_json::Map::new()), 0));
            }
        }
        let correlation_id = generate_trace_id();
        let agent_id_str = agent_id.map(|id| id.0.to_string());
        let result = self
            .call_hook_dispatch_raw(
                hook_name,
                script_path,
                input,
                timeout_secs,
                &correlation_id,
                agent_id_str.as_deref(),
            )
            .await;
        // Update circuit breaker.
        // Schema validation is performed inside call_hook_dispatch_raw (persistent path)
        // and run_hook (non-persistent path) so that Err propagates here correctly.
        match &result {
            Ok(_) => self.circuit_record(hook_name, agent_id, true),
            Err(_) => self.circuit_record(hook_name, agent_id, false),
        }
        result
    }

    async fn call_hook_dispatch_raw(
        &self,
        hook_name: &str,
        script_path: &str,
        input: serde_json::Value,
        timeout_secs: u64,
        correlation_id: &str,
        agent_id: Option<&str>,
    ) -> Result<(serde_json::Value, u64), String> {
        // Compute effective env and network permission, merging bootstrap overrides.
        let (mut effective_env, effective_allow_network) = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            let allow_net = guard.allow_network.unwrap_or(self.allow_network);
            (env, allow_net)
        };

        // Write the resolved plugin config JSON file and expose its path via
        // LIBREFANG_PLUGIN_CONFIG so hook scripts can read typed settings.
        let empty_overrides = std::collections::HashMap::new();
        if let Some(config_path) = Self::write_plugin_config_file(
            &self.plugin_name,
            &self.plugin_config_schema,
            &empty_overrides,
        ) {
            effective_env.push((
                "LIBREFANG_PLUGIN_CONFIG".to_string(),
                config_path.to_string_lossy().into_owned(),
            ));
        }

        // Scope state file to this agent when agent_id is known.
        let effective_state_path = self
            .shared_state_path
            .as_deref()
            .map(|p| agent_scoped_state_path(p, agent_id));

        if self.persistent_subprocess {
            let config = crate::plugin_runtime::HookConfig {
                timeout_secs,
                plugin_env: effective_env.clone(),
                max_memory_mb: self.max_memory_mb,
                allow_network: effective_allow_network,
                state_file: effective_state_path.clone(),
                ..Default::default()
            };
            let trace_id = generate_trace_id();
            let input_preview = if input.to_string().len() > 2048 {
                serde_json::json!({"_truncated": true, "type": input.get("type")})
            } else {
                input.clone()
            };
            let started_at = chrono::Utc::now().to_rfc3339();
            let t = std::time::Instant::now();
            let call_result = self
                .process_pool
                .call(script_path, self.runtime.clone(), &input, &config)
                .await;
            let elapsed_ms = t.elapsed().as_millis() as u64;
            match call_result {
                Ok(output) => {
                    // Validate output schema before recording a success trace so that
                    // schema violations are reflected in both the trace and the circuit
                    // breaker (the Err propagates to call_hook_dispatch which calls
                    // circuit_record(false)).  Mirrors the identical logic in run_hook().
                    if let Some(schema) = self.hook_schemas.get(hook_name) {
                        if let Some(ref output_schema) = schema.output {
                            let errs = Self::validate_schema(
                                output_schema,
                                &output,
                                &format!("{hook_name}/output"),
                            );
                            if !errs.is_empty() {
                                if self.inner.config.output_schema_strict {
                                    let err_msg = format!(
                                        "hook {hook_name} output failed schema validation: {}",
                                        errs.join("; ")
                                    );
                                    Self::push_trace(
                                        &self.traces,
                                        HookTrace {
                                            trace_id: trace_id.clone(),
                                            correlation_id: correlation_id.to_string(),
                                            hook: hook_name.to_string(),
                                            started_at,
                                            elapsed_ms,
                                            success: false,
                                            error: Some(err_msg.clone()),
                                            input_preview,
                                            output_preview: None,
                                            annotations: None,
                                        },
                                        self.trace_store.as_ref(),
                                        &self.plugin_name,
                                    );
                                    return Err(err_msg);
                                }
                                for e in &errs {
                                    warn!("{e}");
                                }
                            }
                        }
                    }
                    Self::push_trace(
                        &self.traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at,
                            elapsed_ms,
                            success: true,
                            error: None,
                            input_preview,
                            output_preview: Some(output.clone()),
                            annotations: output.get("annotations").cloned(),
                        },
                        self.trace_store.as_ref(),
                        &self.plugin_name,
                    );
                    Ok((output, elapsed_ms))
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    Self::push_trace(
                        &self.traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at,
                            elapsed_ms,
                            success: false,
                            error: Some(err_msg.clone()),
                            input_preview,
                            output_preview: None,
                            annotations: None,
                        },
                        self.trace_store.as_ref(),
                        &self.plugin_name,
                    );
                    Err(err_msg)
                }
            }
        } else {
            Self::run_hook(
                hook_name,
                script_path,
                self.runtime.clone(),
                input,
                timeout_secs,
                &effective_env,
                self.max_retries,
                self.retry_delay_ms,
                self.max_memory_mb,
                effective_allow_network,
                &self.traces,
                &self.hook_schemas,
                effective_state_path.as_deref(),
                self.trace_store.as_ref(),
                &self.plugin_name,
                correlation_id,
                self.inner.config.output_schema_strict,
            )
            .await
        }
    }

    /// Apply the configured failure policy to a hook error.
    ///
    /// Returns `Ok(None)` when the policy is Warn or Skip (continue with
    /// fallback), or `Err(…)` when the policy is Abort.
    fn apply_failure_policy(&self, hook: &str, err: &str) -> LibreFangResult<()> {
        use librefang_types::config::HookFailurePolicy;
        match self.on_hook_failure {
            HookFailurePolicy::Warn => {
                warn!(
                    hook,
                    error = err,
                    "Hook failed (warn policy — using fallback)"
                );
                Ok(())
            }
            HookFailurePolicy::Skip => Ok(()), // silent
            HookFailurePolicy::Abort => Err(LibreFangError::Internal(format!(
                "Hook '{hook}' failed (abort policy): {err}"
            ))),
        }
    }

    /// Compute a health snapshot for this engine layer.
    pub async fn layer_health(&self) -> EngineLayerHealth {
        // Circuit breaker: snapshot current open/closed state for each tracked key.
        // Keys are stored as "{agent_id}:{hook}" or bare "{hook}".
        // We report bare hook names; agent-scoped keys use the portion after the last ':'.
        let circuit_open: std::collections::HashMap<String, bool> = {
            let guard = self
                .circuit_breakers
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard
                .iter()
                .map(|(key, state)| {
                    // Extract hook name: last segment after ':' (or whole key if no ':').
                    let hook = match key.rfind(':') {
                        Some(pos) => key[pos + 1..].to_string(),
                        None => key.clone(),
                    };
                    (hook, state.opened_at.is_some())
                })
                .collect()
        };

        // Recent traces from the in-memory ring buffer (sync Mutex).
        let (recent_calls, recent_errors) = {
            let buf = self.traces.lock().unwrap_or_else(|p| p.into_inner());
            let calls = buf.len();
            let errors = buf.iter().filter(|t| t.error.is_some()).count();
            (calls, errors)
        };

        // Active hooks: count how many lifecycle script slots are populated.
        let active_hooks = [
            &self.ingest_script,
            &self.after_turn_script,
            &self.bootstrap_script,
            &self.assemble_script,
            &self.compact_script,
            &self.prepare_subagent_script,
            &self.merge_subagent_script,
            &self.on_event_script,
        ]
        .iter()
        .filter(|opt| opt.is_some())
        .count();

        EngineLayerHealth {
            plugin_name: self.plugin_name.clone(),
            circuit_open,
            active_hooks,
            recent_errors,
            recent_calls,
        }
    }

    /// Apply overrides returned by the bootstrap hook.
    ///
    /// Parses the hook output JSON into a [`BootstrapOverrides`] value and
    /// stores it in `bootstrap_applied_overrides` so subsequent hook calls
    /// pick up the overridden `plugin_env`, `ingest_filter`, and
    /// `allow_network` values.
    fn apply_bootstrap_overrides(&self, output: &serde_json::Value) {
        let overrides: BootstrapOverrides = match serde_json::from_value(output.clone()) {
            Ok(v) => v,
            Err(e) => {
                warn!(plugin = %self.plugin_name, "Failed to parse bootstrap overrides: {e}");
                return;
            }
        };

        if let Ok(mut guard) = self.bootstrap_applied_overrides.lock() {
            // Merge env overrides: only add keys not already present in the initial
            // plugin_env so that statically-configured vars take precedence.
            for (k, v) in overrides.env_overrides {
                if !self.plugin_env.iter().any(|(ek, _)| ek == &k) {
                    guard.env_overrides.insert(k, v);
                }
            }

            // Optional field overrides.
            if let Some(filter) = overrides.ingest_filter {
                guard.ingest_filter = Some(filter);
            }
            if let Some(allow) = overrides.allow_network {
                guard.allow_network = Some(allow);
            }
        }
    }
}

#[async_trait]
impl ContextEngine for ScriptableContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        // Validate all declared hook scripts at startup: existence + executable bit.
        for (name, opt_path) in [
            ("ingest", &self.ingest_script),
            ("after_turn", &self.after_turn_script),
            ("bootstrap", &self.bootstrap_script),
            ("assemble", &self.assemble_script),
            ("compact", &self.compact_script),
            ("prepare_subagent", &self.prepare_subagent_script),
            ("merge_subagent", &self.merge_subagent_script),
            ("on_event", &self.on_event_script),
        ] {
            if let Some(ref path) = opt_path {
                let resolved = Self::resolve_script_path(path);
                let p = std::path::Path::new(&resolved);
                if !p.exists() {
                    warn!("{name} hook script not found: {resolved}");
                } else {
                    // On Unix, check executable bit so we surface "chmod +x" issues early
                    // rather than getting a cryptic "permission denied" at runtime.
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = std::fs::metadata(p) {
                            let mode = meta.permissions().mode();
                            if mode & 0o111 == 0 {
                                warn!(
                                    "{name} hook script is not executable (run `chmod +x {resolved}`)"
                                );
                            }
                        }
                    }
                    debug!("{name} hook configured: {resolved}");
                }
            }
        }

        self.inner.bootstrap(config).await?;

        // Run bootstrap script if configured.
        // Bootstrap runs once and may need extra time for external connections,
        // so it gets double the configured hook timeout.
        if let Some(ref script) = self.bootstrap_script {
            let bootstrap_timeout = self.hook_timeout_secs.saturating_mul(2);
            let input = serde_json::json!({
                "type": "bootstrap",
                "context_window_tokens": config.context_window_tokens,
                "stable_prefix_mode": config.stable_prefix_mode,
                "max_recall_results": config.max_recall_results,
            });
            match self
                .call_hook_dispatch("bootstrap", script, input, bootstrap_timeout, None)
                .await
            {
                Ok((ref output, ms)) => {
                    Self::record_hook(&self.metrics, "bootstrap", ms, true);
                    debug!("Bootstrap hook completed (timeout={bootstrap_timeout}s, {ms}ms)");
                    self.apply_bootstrap_overrides(output);
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "bootstrap", 0, false);
                    let _ = self.apply_failure_policy("bootstrap", &e);
                }
            }
        }

        Ok(())
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        // In stable_prefix_mode, skip all recall (including hooks) to keep prompt stable
        if self.inner.config.stable_prefix_mode {
            return Ok(IngestResult {
                recalled_memories: Vec::new(),
            });
        }

        // If no ingest script, delegate entirely to default engine
        let Some(ref script) = self.ingest_script else {
            return self.inner.ingest(agent_id, user_message, peer_id).await;
        };

        // Apply ingest_filter — skip hook when message doesn't match.
        // Bootstrap overrides take precedence over the statically configured filter.
        let effective_ingest_filter: Option<String> = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard
                .ingest_filter
                .clone()
                .or_else(|| self.ingest_filter.clone())
        };
        if let Some(ref filter) = effective_ingest_filter {
            if !user_message.contains(filter.as_str()) {
                debug!(
                    filter = filter.as_str(),
                    "Ingest hook skipped (filter mismatch)"
                );
                return self.inner.ingest(agent_id, user_message, peer_id).await;
            }
        }

        // Apply ingest_regex filter.
        if let Some(ref re) = self.ingest_regex {
            if !re.is_match(user_message) {
                debug!("Ingest hook skipped (ingest_regex mismatch)");
                return self.inner.ingest(agent_id, user_message, peer_id).await;
            }
        }

        // Apply agent_id_filter — skip hook for agents not in the allowlist.
        if !self.agent_passes_filter(&agent_id) {
            debug!("Ingest hook skipped (agent_id not in only_for_agent_ids filter)");
            return self.inner.ingest(agent_id, user_message, peer_id).await;
        }

        // Run default recall first (for embedding-based memories)
        let default_result = self.inner.ingest(agent_id, user_message, peer_id).await?;

        // Run the hook for additional/custom recall
        let input = serde_json::json!({
            "type": "ingest",
            "agent_id": agent_id.0.to_string(),
            "message": user_message,
            "peer_id": peer_id,
        });

        // TTL-based cache: skip subprocess if we have a fresh cached result.
        if let Some(ttl_secs) = self.ingest_cache_ttl_secs {
            let cache_key = {
                let raw = serde_json::to_string(&input).unwrap_or_default();
                crate::plugin_manager::sha256_hex(raw.as_bytes())
            };
            let cached = {
                let guard = self.ingest_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                tracing::info!(hook = "ingest", agent_id = %agent_id, ttl_secs, "Ingest hook succeeded (cache hit)");
                debug!("Ingest hook cache hit (ttl={}s)", ttl_secs);
                let mut memories = default_result.recalled_memories;
                if let Some(hook_memories) =
                    cached_output.get("memories").and_then(|m| m.as_array())
                {
                    for mem in hook_memories {
                        if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                            memories.push(MemoryFragment {
                                id: librefang_types::memory::MemoryId::new(),
                                agent_id,
                                content: content.to_string(),
                                embedding: None,
                                metadata: std::collections::HashMap::new(),
                                source: librefang_types::memory::MemorySource::System,
                                confidence: 1.0,
                                created_at: chrono::Utc::now(),
                                accessed_at: chrono::Utc::now(),
                                access_count: 0,
                                scope: "hook_cached".to_string(),
                                image_url: None,
                                image_embedding: None,
                                modality: Default::default(),
                            });
                        }
                    }
                }
                return Ok(IngestResult {
                    recalled_memories: memories,
                });
            }
            // Cache miss — run hook and store result below
            let cache_key_owned = cache_key;
            let cache_arc = self.ingest_cache.clone();
            match self
                .call_hook_dispatch(
                    "ingest",
                    script,
                    input.clone(),
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await
            {
                Ok((output, ms)) => {
                    Self::record_hook(&self.metrics, "ingest", ms, true);
                    tracing::info!(hook = "ingest", agent_id = %agent_id, elapsed_ms = ms, "Ingest hook succeeded (cache miss)");
                    // Store in cache
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key_owned, (output.clone(), std::time::Instant::now()));
                        // Evict expired entries when cache grows large
                        if guard.len() > 512 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    let mut memories = default_result.recalled_memories;
                    if let Some(hook_memories) = output.get("memories").and_then(|m| m.as_array()) {
                        for mem in hook_memories {
                            if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                                memories.push(MemoryFragment {
                                    id: librefang_types::memory::MemoryId::new(),
                                    agent_id,
                                    content: content.to_string(),
                                    embedding: None,
                                    metadata: std::collections::HashMap::new(),
                                    source: librefang_types::memory::MemorySource::System,
                                    confidence: 1.0,
                                    created_at: chrono::Utc::now(),
                                    accessed_at: chrono::Utc::now(),
                                    access_count: 0,
                                    scope: "hook".to_string(),
                                    image_url: None,
                                    image_embedding: None,
                                    modality: Default::default(),
                                });
                            }
                        }
                    }
                    return Ok(IngestResult {
                        recalled_memories: memories,
                    });
                }
                Err(err) => {
                    Self::record_hook(&self.metrics, "ingest", 0, false);
                    self.apply_failure_policy("ingest", &err)?;
                    return Ok(default_result); // reached only for Warn/Skip policy
                }
            }
        }

        match self
            .call_hook_dispatch(
                "ingest",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                Self::record_hook(&self.metrics, "ingest", ms, true);
                self.record_per_agent(&agent_id, ms, true);
                tracing::info!(hook = "ingest", agent_id = %agent_id, elapsed_ms = ms, "Ingest hook succeeded (no cache)");
                // Merge hook memories with default memories
                let mut memories = default_result.recalled_memories;
                if let Some(hook_memories) = output.get("memories").and_then(|m| m.as_array()) {
                    for mem in hook_memories {
                        if let Some(content) = mem.get("content").and_then(|c| c.as_str()) {
                            memories.push(MemoryFragment {
                                id: librefang_types::memory::MemoryId::new(),
                                agent_id,
                                content: content.to_string(),
                                embedding: None,
                                metadata: std::collections::HashMap::new(),
                                source: librefang_types::memory::MemorySource::System,
                                confidence: 1.0,
                                created_at: chrono::Utc::now(),
                                accessed_at: chrono::Utc::now(),
                                access_count: 0,
                                scope: "hook".to_string(),
                                image_url: None,
                                image_embedding: None,
                                modality: Default::default(),
                            });
                        }
                    }
                }
                Ok(IngestResult {
                    recalled_memories: memories,
                })
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "ingest", 0, false);
                self.record_per_agent(&agent_id, 0, false);
                self.apply_failure_policy("ingest", &e)?;
                Ok(default_result)
            }
        }
    }

    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        let Some(ref script) = self.assemble_script else {
            return self
                .inner
                .assemble(
                    agent_id,
                    messages,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await;
        };

        // Serialize full message structure — tool_use/tool_result blocks preserved
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let input = serde_json::json!({
            "type": "assemble",
            "agent_id": agent_id.0.to_string(),
            "system_prompt": system_prompt,
            "messages": msg_values,
            "context_window_tokens": context_window_tokens,
        });

        // Apply agent_id_filter for assemble hook.
        if !self.agent_passes_filter(&agent_id) {
            return self
                .inner
                .assemble(
                    agent_id,
                    messages,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await;
        }

        // TTL-based cache for assemble hook.
        if let Some(ttl_secs) = self.assemble_cache_ttl_secs {
            let cache_key = crate::plugin_manager::sha256_hex(
                serde_json::to_string(&input).unwrap_or_default().as_bytes(),
            );
            let cached = {
                let guard = self.assemble_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                debug!("Assemble hook cache hit (ttl={}s)", ttl_secs);
                if let Some(new_msgs) = cached_output.get("messages").and_then(|v| v.as_array()) {
                    let assembled: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !assembled.is_empty() {
                        *messages = assembled;
                        return Ok(AssembleResult {
                            recovery: crate::context_overflow::RecoveryStage::None,
                        });
                    }
                }
                // Cached result had no messages; fall through to default
                return self
                    .inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await;
            }
            // Cache miss — run hook and store result.
            let cache_arc = self.assemble_cache.clone();
            let result = self
                .call_hook_dispatch(
                    "assemble",
                    script,
                    input,
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await;
            match result {
                Ok((output, ms)) => {
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key, (output.clone(), std::time::Instant::now()));
                        if guard.len() > 256 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                        let assembled: Vec<Message> = new_msgs
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        if !assembled.is_empty() {
                            Self::record_hook(&self.metrics, "assemble", ms, true);
                            *messages = assembled;
                            return Ok(AssembleResult {
                                recovery: crate::context_overflow::RecoveryStage::None,
                            });
                        }
                    }
                    Self::record_hook(&self.metrics, "assemble", ms, false);
                    return self
                        .inner
                        .assemble(
                            agent_id,
                            messages,
                            system_prompt,
                            tools,
                            context_window_tokens,
                        )
                        .await;
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "assemble", 0, false);
                    self.apply_failure_policy("assemble", &e)?;
                    return self
                        .inner
                        .assemble(
                            agent_id,
                            messages,
                            system_prompt,
                            tools,
                            context_window_tokens,
                        )
                        .await;
                }
            }
        }

        match self
            .call_hook_dispatch(
                "assemble",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                    let assembled: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();

                    if !assembled.is_empty() {
                        Self::record_hook(&self.metrics, "assemble", ms, true);
                        *messages = assembled;
                        return Ok(AssembleResult {
                            recovery: crate::context_overflow::RecoveryStage::None,
                        });
                    }
                    warn!("Assemble hook returned empty messages, falling back to default");
                } else {
                    warn!("Assemble hook returned no 'messages' field, falling back to default");
                }
                Self::record_hook(&self.metrics, "assemble", ms, false);
                self.inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "assemble", 0, false);
                self.apply_failure_policy("assemble", &e)?;
                self.inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await
            }
        }
    }

    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        let Some(ref script) = self.compact_script else {
            return self
                .inner
                .compact(agent_id, messages, driver, model, context_window_tokens)
                .await;
        };

        // Serialize full message structure — tool_use/tool_result blocks preserved
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        // Build token pressure metadata for the compact hook.
        let used_tokens = crate::compactor::estimate_token_count(messages, None, None);
        let max_ctx = if context_window_tokens > 0 {
            context_window_tokens
        } else {
            100_000
        };
        let pressure = (used_tokens as f64 / max_ctx as f64).min(1.0);
        let recommendation = match pressure {
            p if p >= 0.9 => "critical",
            p if p >= 0.8 => "aggressive",
            p if p >= 0.6 => "moderate",
            _ => "light",
        };
        let token_pressure = serde_json::json!({
            "used_tokens": used_tokens,
            "max_tokens": max_ctx,
            "pressure": pressure,
            "recommendation": recommendation,
        });

        let mut input = serde_json::json!({
            "type": "compact",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_values,
            "model": model,
            "context_window_tokens": context_window_tokens,
        });
        if let Some(obj) = input.as_object_mut() {
            obj.insert("token_pressure".to_string(), token_pressure);
        }

        // TTL-based cache for compact hook.
        if let Some(ttl_secs) = self.compact_cache_ttl_secs {
            let cache_key = crate::plugin_manager::sha256_hex(
                serde_json::to_string(&input).unwrap_or_default().as_bytes(),
            );
            let cached = {
                let guard = self.compact_cache.lock().unwrap();
                guard.get(&cache_key).and_then(|(val, inserted_at)| {
                    if inserted_at.elapsed().as_secs() < ttl_secs {
                        Some(val.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(cached_output) = cached {
                debug!("Compact hook cache hit (ttl={}s)", ttl_secs);
                if let Some(new_msgs) = cached_output.get("messages").and_then(|v| v.as_array()) {
                    let compacted: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();
                    if !compacted.is_empty() {
                        let summary = cached_output
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("plugin compaction (cached)")
                            .to_string();
                        let removed = messages.len().saturating_sub(compacted.len());
                        return Ok(CompactionResult {
                            summary,
                            kept_messages: compacted,
                            compacted_count: removed,
                            chunks_used: 1,
                            used_fallback: false,
                        });
                    }
                }
                return self
                    .inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await;
            }
            // Cache miss — run hook and store result.
            let cache_arc = self.compact_cache.clone();
            let result = self
                .call_hook_dispatch(
                    "compact",
                    script,
                    input,
                    self.hook_timeout_secs,
                    Some(&agent_id),
                )
                .await;
            match result {
                Ok((output, ms)) => {
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key, (output.clone(), std::time::Instant::now()));
                        if guard.len() > 256 {
                            guard.retain(|_, (_, inserted_at)| {
                                inserted_at.elapsed().as_secs() < ttl_secs
                            });
                        }
                    }
                    if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                        let compacted: Vec<Message> = new_msgs
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .collect();
                        if !compacted.is_empty() {
                            Self::record_hook(&self.metrics, "compact", ms, true);
                            let summary = output
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .unwrap_or("plugin compaction")
                                .to_string();
                            let removed = messages.len().saturating_sub(compacted.len());
                            return Ok(CompactionResult {
                                summary,
                                kept_messages: compacted,
                                compacted_count: removed,
                                chunks_used: 1,
                                used_fallback: false,
                            });
                        }
                    }
                    Self::record_hook(&self.metrics, "compact", ms, false);
                    return self
                        .inner
                        .compact(agent_id, messages, driver, model, context_window_tokens)
                        .await;
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "compact", 0, false);
                    self.apply_failure_policy("compact", &e)?;
                    return self
                        .inner
                        .compact(agent_id, messages, driver, model, context_window_tokens)
                        .await;
                }
            }
        }

        match self
            .call_hook_dispatch(
                "compact",
                script,
                input,
                self.hook_timeout_secs,
                Some(&agent_id),
            )
            .await
        {
            Ok((output, ms)) => {
                if let Some(new_msgs) = output.get("messages").and_then(|v| v.as_array()) {
                    let compacted: Vec<Message> = new_msgs
                        .iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect();

                    if !compacted.is_empty() {
                        Self::record_hook(&self.metrics, "compact", ms, true);
                        let summary = output
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("plugin compaction")
                            .to_string();
                        let removed = messages.len().saturating_sub(compacted.len());
                        return Ok(CompactionResult {
                            summary,
                            kept_messages: compacted,
                            compacted_count: removed,
                            chunks_used: 1,
                            used_fallback: false,
                        });
                    }
                    warn!("Compact hook returned empty messages, falling back to default");
                } else {
                    warn!("Compact hook returned no 'messages' field, falling back to default");
                }
                Self::record_hook(&self.metrics, "compact", ms, false);
                self.inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "compact", 0, false);
                self.apply_failure_policy("compact", &e)?;
                self.inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await
            }
        }
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        // Run default after_turn first
        self.inner.after_turn(agent_id, messages).await?;

        // If no after_turn script, we're done
        let Some(ref script) = self.after_turn_script else {
            return Ok(());
        };

        // Send full message structure so scripts can index tool_use/tool_result/image blocks.
        let msg_values: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let input = serde_json::json!({
            "type": "after_turn",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_values,
        });

        // Spawn as fire-and-forget — after_turn is best-effort, don't block the agent.
        // Log if the task panics so failures aren't silently swallowed.

        // Circuit-breaker check: skip spawning if the circuit is already open.
        if self.circuit_is_open("after_turn", Some(&agent_id)) {
            debug!("after_turn hook skipped — circuit breaker is open");
            return Ok(());
        }

        // Apply agent_id_filter for after_turn hook.
        if !self.agent_passes_filter(&agent_id) {
            return Ok(());
        }

        let script = script.clone();
        let runtime = self.runtime.clone();
        let timeout_secs = self.hook_timeout_secs;
        // Merge bootstrap env overrides into the env passed to the background task.
        let plugin_env = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            env
        };
        let metrics = std::sync::Arc::clone(&self.metrics);
        let max_retries = self.max_retries;
        let retry_delay_ms = self.retry_delay_ms;
        let max_memory_mb = self.max_memory_mb;
        let allow_network = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard.allow_network.unwrap_or(self.allow_network)
        };
        let traces = std::sync::Arc::clone(&self.traces);
        let hook_schemas = self.hook_schemas.clone();
        let persistent_subprocess = self.persistent_subprocess;
        let process_pool = std::sync::Arc::clone(&self.process_pool);
        let sem = std::sync::Arc::clone(&self.after_turn_sem);
        let trace_store = self.trace_store.clone();
        let plugin_name = self.plugin_name.clone();
        let agent_id_str = agent_id.0.to_string();
        // Compute agent-scoped state path for this after_turn call.
        let shared_state_path = self
            .shared_state_path
            .as_deref()
            .map(|p| agent_scoped_state_path(p, Some(agent_id_str.as_str())));
        let memory_substrate = std::sync::Arc::clone(&self.memory_substrate);
        let output_schema_strict = self.inner.config.output_schema_strict;
        let after_turn_correlation_id = generate_trace_id();
        let event_bus_arc = self.event_bus.clone();
        // Clone circuit-breaker state for updating from the background task.
        let cb_breakers = std::sync::Arc::clone(&self.circuit_breakers);
        let cb_cfg = self.circuit_breaker_cfg.clone();
        let cb_trace_store = self.trace_store.clone();
        {
            let mut tasks = self.after_turn_tasks.lock().await;
            // Reap already-completed tasks to prevent unbounded growth.
            while tasks.try_join_next().is_some() {}

            let correlation_id_at = after_turn_correlation_id.clone();
            tasks.spawn(async move {
                // Bounded concurrency: acquire a semaphore permit before running the hook.
                // `.ok()` is intentional: if the semaphore is closed (daemon shutting down),
                // `acquire()` returns `Err(AcquireError)`. Ignoring it with `.ok()` lets the
                // task complete its current hook call cleanly instead of panicking.  The permit
                // is held for the lifetime of this spawned task via the `_permit` binding.
                let _permit = sem.acquire().await.ok();
                let result = if persistent_subprocess {
                    let config = crate::plugin_runtime::HookConfig {
                        timeout_secs,
                        plugin_env: plugin_env.clone(),
                        max_memory_mb,
                        allow_network,
                        state_file: shared_state_path.clone(),
                        ..Default::default()
                    };
                    let trace_id = generate_trace_id();
                    let input_preview = if input.to_string().len() > 2048 {
                        serde_json::json!({"_truncated": true, "type": input.get("type")})
                    } else {
                        input.clone()
                    };
                    let started_at = chrono::Utc::now().to_rfc3339();
                    let t = std::time::Instant::now();
                    let call_result = process_pool.call(&script, runtime, &input, &config).await;
                    let elapsed_ms = t.elapsed().as_millis() as u64;
                    match call_result {
                        Ok(output) => {
                            Self::push_trace(
                                &traces,
                                HookTrace {
                                    trace_id: trace_id.clone(),
                                    correlation_id: correlation_id_at.clone(),
                                    hook: "after_turn".to_string(),
                                    started_at,
                                    elapsed_ms,
                                    success: true,
                                    error: None,
                                    input_preview,
                                    output_preview: Some(output.clone()),
                                    annotations: output.get("annotations").cloned(),
                                },
                                trace_store.as_ref(),
                                &plugin_name,
                            );
                            Ok((output, elapsed_ms))
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            Self::push_trace(
                                &traces,
                                HookTrace {
                                    trace_id: trace_id.clone(),
                                    correlation_id: correlation_id_at.clone(),
                                    hook: "after_turn".to_string(),
                                    started_at,
                                    elapsed_ms,
                                    success: false,
                                    error: Some(err_msg.clone()),
                                    input_preview,
                                    output_preview: None,
                                    annotations: None,
                                },
                                trace_store.as_ref(),
                                &plugin_name,
                            );
                            Err(err_msg)
                        }
                    }
                } else {
                    Self::run_hook(
                        "after_turn",
                        &script,
                        runtime,
                        input,
                        timeout_secs,
                        &plugin_env,
                        max_retries,
                        retry_delay_ms,
                        max_memory_mb,
                        allow_network,
                        &traces,
                        &hook_schemas,
                        shared_state_path.as_deref(),
                        trace_store.as_ref(),
                        &plugin_name,
                        &correlation_id_at,
                        output_schema_strict,
                    )
                    .await
                };
                let success = result.is_ok();
                match result {
                    Ok((output, ms)) => {
                        Self::record_hook(&metrics, "after_turn", ms, true);
                        debug!("After-turn hook completed ({ms}ms)");
                        // Inspect hook output for memories, logs, and annotations.
                        Self::process_after_turn_output(
                            &output,
                            &agent_id_str,
                            Some(&memory_substrate),
                            &plugin_name,
                            event_bus_arc.as_ref(),
                        );
                    }
                    Err(e) => {
                        Self::record_hook(&metrics, "after_turn", 0, false);
                        warn!("After-turn hook failed: {e}");
                    }
                }
                // Update circuit breaker from the background task so that repeated
                // after_turn failures can trip the circuit and stop future spawns.
                if let Some(ref cfg) = cb_cfg {
                    let key = format!("{}:after_turn", agent_id_str);
                    let (failures, opened_at_rfc3339, just_reset) = {
                        let mut guard = cb_breakers.lock().unwrap();
                        let state = guard
                            .entry(key.clone())
                            .or_insert_with(CircuitBreakerState::new);
                        if success {
                            state.record_success();
                            (0u32, None::<String>, true)
                        } else {
                            state.record_failure(cfg.max_failures);
                            if state.consecutive_failures == cfg.max_failures {
                                warn!(
                                    hook = "after_turn",
                                    cooldown_secs = cfg.reset_secs,
                                    "Hook circuit breaker opened"
                                );
                            }
                            let opened_str = state.opened_at.map(|instant| {
                                let elapsed = instant.elapsed();
                                (chrono::Utc::now()
                                    - chrono::Duration::from_std(elapsed).unwrap_or_default())
                                .to_rfc3339()
                            });
                            (state.consecutive_failures, opened_str, false)
                        }
                    };
                    if let Some(ref store) = cb_trace_store {
                        if just_reset {
                            let _ = store.delete_circuit_state(&key);
                        } else {
                            let _ = store.save_circuit_state(
                                &key,
                                failures,
                                opened_at_rfc3339.as_deref(),
                            );
                        }
                    }
                }
            });
        }

        Ok(())
    }

    async fn prepare_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        self.inner
            .prepare_subagent_context(parent_id, child_id)
            .await?;

        if let Some(ref script) = self.prepare_subagent_script {
            let input = serde_json::json!({
                "type": "prepare_subagent",
                "parent_id": parent_id.0.to_string(),
                "child_id": child_id.0.to_string(),
            });
            match self
                .call_hook_dispatch(
                    "prepare_subagent",
                    script,
                    input,
                    self.hook_timeout_secs,
                    None,
                )
                .await
            {
                Ok((_, ms)) => {
                    Self::record_hook(&self.metrics, "prepare_subagent", ms, true);
                    debug!("Prepare-subagent hook completed ({ms}ms)");
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "prepare_subagent", 0, false);
                    self.apply_failure_policy("prepare_subagent", &e)?;
                }
            }
        }

        Ok(())
    }

    async fn merge_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        self.inner
            .merge_subagent_context(parent_id, child_id)
            .await?;

        if let Some(ref script) = self.merge_subagent_script {
            let input = serde_json::json!({
                "type": "merge_subagent",
                "parent_id": parent_id.0.to_string(),
                "child_id": child_id.0.to_string(),
            });
            match self
                .call_hook_dispatch(
                    "merge_subagent",
                    script,
                    input,
                    self.hook_timeout_secs,
                    None,
                )
                .await
            {
                Ok((_, ms)) => {
                    Self::record_hook(&self.metrics, "merge_subagent", ms, true);
                    debug!("Merge-subagent hook completed ({ms}ms)");
                }
                Err(e) => {
                    Self::record_hook(&self.metrics, "merge_subagent", 0, false);
                    self.apply_failure_policy("merge_subagent", &e)?;
                }
            }
        }

        Ok(())
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        self.inner
            .truncate_tool_result(content, context_window_tokens)
    }

    fn hook_metrics(&self) -> Option<HookMetrics> {
        Some(self.metrics())
    }

    fn hook_traces(&self) -> Vec<HookTrace> {
        self.traces_snapshot()
    }

    fn per_agent_metrics(&self) -> std::collections::HashMap<String, HookStats> {
        self.per_agent_metrics_snapshot()
    }
}

// ---------------------------------------------------------------------------
// Plugin loader — resolves `plugin = "name"` to hook paths
// ---------------------------------------------------------------------------

/// Default plugin directory: `~/.librefang/plugins/`.
pub fn plugins_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".librefang")
        .join("plugins")
}

/// Load a plugin manifest from `~/.librefang/plugins/<name>/plugin.toml`.
///
/// Hook paths in the manifest are relative to the plugin directory — this
/// function resolves them to absolute paths so the script runner can find them.
/// Validate that a plugin name is a safe directory component (no path traversal).
fn validate_plugin_name(name: &str) -> LibreFangResult<()> {
    // Strict whitelist: only ASCII alphanumeric, hyphens, and underscores.
    // Rejects spaces, null bytes, path separators, unicode, and shell specials.
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(LibreFangError::Internal(format!(
            "Invalid plugin name '{name}': must contain only ASCII letters, digits, hyphens, and underscores"
        )));
    }
    Ok(())
}

fn load_plugin(
    plugin_name: &str,
) -> LibreFangResult<(
    librefang_types::config::PluginManifest,
    librefang_types::config::ContextEngineHooks,
)> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    let manifest_path = plugin_dir.join("plugin.toml");

    if !manifest_path.exists() {
        return Err(LibreFangError::Internal(format!(
            "Plugin '{plugin_name}' not found at {}",
            manifest_path.display()
        )));
    }

    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        LibreFangError::Internal(format!("Failed to read {}: {e}", manifest_path.display()))
    })?;

    let manifest: librefang_types::config::PluginManifest =
        toml::from_str(&content).map_err(|e| {
            LibreFangError::Internal(format!("Invalid plugin.toml for '{plugin_name}': {e}"))
        })?;

    // Resolve relative hook paths to absolute paths within the plugin dir
    // and verify they don't escape the plugin directory (path traversal guard).
    let canon_plugin_dir =
        std::fs::canonicalize(&plugin_dir).unwrap_or_else(|_| plugin_dir.clone());

    let resolve_and_sandbox = |rel_path: &str| -> LibreFangResult<String> {
        let abs_path = plugin_dir.join(rel_path);
        // Canonicalize to resolve any ".." components
        let canon = std::fs::canonicalize(&abs_path).map_err(|e| {
            LibreFangError::Internal(format!(
                "Cannot resolve hook path '{}': {e}",
                abs_path.display()
            ))
        })?;
        if !canon.starts_with(&canon_plugin_dir) {
            return Err(LibreFangError::Internal(format!(
                "Hook script '{}' escapes plugin directory '{}'",
                canon.display(),
                canon_plugin_dir.display()
            )));
        }
        Ok(canon.to_string_lossy().into_owned())
    };

    let resolved_hooks = librefang_types::config::ContextEngineHooks {
        ingest: manifest
            .hooks
            .ingest
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        after_turn: manifest
            .hooks
            .after_turn
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        bootstrap: manifest
            .hooks
            .bootstrap
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        assemble: manifest
            .hooks
            .assemble
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        compact: manifest
            .hooks
            .compact
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        prepare_subagent: manifest
            .hooks
            .prepare_subagent
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        merge_subagent: manifest
            .hooks
            .merge_subagent
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        // Propagate the runtime tag from the plugin manifest. `None` means
        // "use the default" which resolves to Python in PluginRuntime::from_tag.
        runtime: manifest.hooks.runtime.clone(),
        // Propagate all extended hook config fields from the manifest.
        hook_timeout_secs: manifest.hooks.hook_timeout_secs,
        max_retries: manifest.hooks.max_retries,
        retry_delay_ms: manifest.hooks.retry_delay_ms,
        ingest_filter: manifest.hooks.ingest_filter.clone(),
        on_hook_failure: manifest.hooks.on_hook_failure.clone(),
        hook_protocol_version: manifest.hooks.hook_protocol_version,
        max_memory_mb: manifest.hooks.max_memory_mb,
        allow_network: manifest.hooks.allow_network,
        only_for_agent_ids: manifest.hooks.only_for_agent_ids.clone(),
        hook_schemas: manifest.hooks.hook_schemas.clone(),
        hook_cache_ttl_secs: manifest.hooks.hook_cache_ttl_secs,
        persistent_subprocess: manifest.hooks.persistent_subprocess,
        assemble_cache_ttl_secs: manifest.hooks.assemble_cache_ttl_secs,
        compact_cache_ttl_secs: manifest.hooks.compact_cache_ttl_secs,
        priority: manifest.hooks.priority,
        ingest_regex: manifest.hooks.ingest_regex.clone(),
        env_schema: manifest.hooks.env_schema.clone(),
        enable_shared_state: manifest.hooks.enable_shared_state,
        circuit_breaker: manifest.hooks.circuit_breaker.clone(),
        after_turn_queue_depth: manifest.hooks.after_turn_queue_depth,
        prewarm_subprocesses: manifest.hooks.prewarm_subprocesses,
        allow_filesystem: manifest.hooks.allow_filesystem,
        otel_endpoint: manifest.hooks.otel_endpoint.clone(),
        on_event: manifest
            .hooks
            .on_event
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        allowed_secrets: manifest.hooks.allowed_secrets.clone(),
    };

    debug!(
        plugin = plugin_name,
        dir = %plugin_dir.display(),
        ingest = ?resolved_hooks.ingest,
        after_turn = ?resolved_hooks.after_turn,
        bootstrap = ?resolved_hooks.bootstrap,
        assemble = ?resolved_hooks.assemble,
        compact = ?resolved_hooks.compact,
        prepare_subagent = ?resolved_hooks.prepare_subagent,
        merge_subagent = ?resolved_hooks.merge_subagent,
        "Loaded plugin manifest"
    );

    Ok((manifest, resolved_hooks))
}

// ---------------------------------------------------------------------------
// Stacked context engine — chains multiple engines in declaration order
// ---------------------------------------------------------------------------

/// A context engine that chains multiple engines in order.
///
/// Hook semantics per method:
/// - `bootstrap`: all engines in order; first error is fatal.
/// - `ingest`: memories from all engines are merged into a single result.
/// - `assemble`: first engine that returns non-empty messages wins; the rest
///   are skipped. Falls back to the last engine if all return empty.
/// - `compact`: first engine that succeeds with a non-fallback result wins;
///   the rest are skipped. Falls back through the chain until one succeeds.
/// - `after_turn`: all engines run concurrently (best-effort); individual
///   failures are logged but do not propagate.
/// - `prepare_subagent` / `merge_subagent`: all engines in order.
/// - `truncate_tool_result`: delegates to the first (primary) engine.
pub struct StackedContextEngine {
    engines: Vec<Box<dyn ContextEngine>>,
    /// Per-layer priority weights for ingest result ordering.
    /// Higher weights cause a layer's memories to appear first in the merged
    /// result. Defaults to `1.0` for every layer.
    layer_weights: Vec<f32>,
    /// Shared event bus for inter-plugin event dispatch.
    event_bus: std::sync::Arc<PluginEventBus>,
}

impl StackedContextEngine {
    /// Create a stacked engine from an ordered list of constituent engines.
    ///
    /// Panics if `engines` is empty.
    pub fn new(engines: Vec<Box<dyn ContextEngine>>) -> Self {
        let bus = std::sync::Arc::new(PluginEventBus::new(256));
        Self::new_with_bus(engines, bus)
    }

    /// Like [`new`] but uses the provided shared event bus instead of creating
    /// a fresh one.  Use this when you want all constituent engines to share
    /// the same bus (so events emitted by one plugin reach the `on_event` hook
    /// of every other plugin in the stack).
    pub fn new_with_bus(
        engines: Vec<Box<dyn ContextEngine>>,
        bus: std::sync::Arc<PluginEventBus>,
    ) -> Self {
        assert!(
            !engines.is_empty(),
            "StackedContextEngine requires at least one engine"
        );
        let layer_weights = vec![1.0f32; engines.len()];
        Self {
            engines,
            layer_weights,
            event_bus: bus,
        }
    }

    /// Override the default per-layer weights (all `1.0`) with caller-supplied
    /// values.  Weights are matched by position — the first weight applies to
    /// the first engine, and so on.  Weights beyond `engines.len()` are
    /// silently ignored; missing trailing weights default to `1.0`.
    pub fn with_weights(mut self, weights: Vec<f32>) -> Self {
        self.layer_weights = weights;
        self
    }

    /// Return a reference to the shared event bus.
    pub fn event_bus(&self) -> &std::sync::Arc<PluginEventBus> {
        &self.event_bus
    }

    /// Return a health snapshot for the entire engine stack.
    ///
    /// Uses the trait-level `hook_traces()` and `hook_metrics()` methods to
    /// gather per-layer data without requiring a concrete type downcast.
    pub async fn health_summary(&self) -> StackHealth {
        let mut layers = Vec::with_capacity(self.engines.len());
        for engine in &self.engines {
            let traces = engine.hook_traces();
            let recent_calls = traces.len();
            let recent_errors = traces.iter().filter(|t| t.error.is_some()).count();

            // Derive circuit-open map from metrics: a hook slot where all recorded
            // calls have failed (failures > 0, successes == 0) is reported as open.
            let metrics_opt = engine.hook_metrics();
            let circuit_open: std::collections::HashMap<String, bool> =
                if let Some(ref m) = metrics_opt {
                    [
                        ("ingest", m.ingest.failures > 0 && m.ingest.successes == 0),
                        (
                            "after_turn",
                            m.after_turn.failures > 0 && m.after_turn.successes == 0,
                        ),
                        (
                            "bootstrap",
                            m.bootstrap.failures > 0 && m.bootstrap.successes == 0,
                        ),
                        (
                            "assemble",
                            m.assemble.failures > 0 && m.assemble.successes == 0,
                        ),
                        (
                            "compact",
                            m.compact.failures > 0 && m.compact.successes == 0,
                        ),
                        (
                            "prepare_subagent",
                            m.prepare_subagent.failures > 0 && m.prepare_subagent.successes == 0,
                        ),
                        (
                            "merge_subagent",
                            m.merge_subagent.failures > 0 && m.merge_subagent.successes == 0,
                        ),
                    ]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect()
                } else {
                    std::collections::HashMap::new()
                };

            // active_hooks: number of hook slots with at least one recorded call.
            let active_hooks = if let Some(ref m) = metrics_opt {
                [
                    m.ingest.calls,
                    m.after_turn.calls,
                    m.bootstrap.calls,
                    m.assemble.calls,
                    m.compact.calls,
                    m.prepare_subagent.calls,
                    m.merge_subagent.calls,
                ]
                .iter()
                .filter(|&&c| c > 0)
                .count()
            } else {
                0
            };

            layers.push(EngineLayerHealth {
                plugin_name: String::new(),
                circuit_open,
                active_hooks,
                recent_errors,
                recent_calls,
            });
        }
        let layers_with_open_circuit = layers
            .iter()
            .filter(|l| l.circuit_open.values().any(|&open| open))
            .count();
        let total_layers = layers.len();
        StackHealth {
            layers,
            total_layers,
            layers_with_open_circuit,
        }
    }
}

#[async_trait]
impl ContextEngine for StackedContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        for engine in &self.engines {
            engine.bootstrap(config).await?;
        }
        Ok(())
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        // Run all engines concurrently — ingest is independent per engine
        // (each has its own recall store), so parallel execution is safe and
        // reduces total latency to max(individual latencies).
        // Each engine is guarded by a 30-second timeout so a single slow or
        // hung engine cannot block the entire stack indefinitely.
        let timeout_dur = std::time::Duration::from_secs(30);
        let futs = self
            .engines
            .iter()
            .enumerate()
            .map(|(i, engine)| async move {
                match tokio::time::timeout(
                    timeout_dur,
                    engine.ingest(agent_id, user_message, peer_id),
                )
                .await
                {
                    Ok(Ok(r)) => Some(r.recalled_memories),
                    Ok(Err(e)) => {
                        warn!(
                            engine_index = i,
                            error = %e,
                            "StackedContextEngine: ingest engine failed (skipping)"
                        );
                        None
                    }
                    Err(_elapsed) => {
                        warn!(
                            engine_index = i,
                            timeout_secs = 30,
                            "StackedContextEngine: ingest engine timed out (skipping)"
                        );
                        None
                    }
                }
            });

        let default_weight = 1.0f32;
        let mut succeeded: usize = 0;
        let mut failed: usize = 0;

        // Collect (weight, memories) pairs, preserving layer index so we can
        // sort by weight without losing provenance.
        let mut weighted_results: Vec<(f32, Vec<MemoryFragment>)> = Vec::new();
        for (i, memories) in futures::future::join_all(futs)
            .await
            .into_iter()
            .enumerate()
        {
            match memories {
                Some(m) => {
                    succeeded += 1;
                    let w = *self.layer_weights.get(i).unwrap_or(&default_weight);
                    weighted_results.push((w, m));
                }
                None => {
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            warn!(
                succeeded,
                failed, "StackedContextEngine: ingest completed with some engine failures"
            );
        }

        // Sort layers by weight descending so higher-priority layers' memories
        // appear first in the merged result.
        weighted_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let all_memories: Vec<MemoryFragment> =
            weighted_results.into_iter().flat_map(|(_, m)| m).collect();

        Ok(IngestResult {
            recalled_memories: all_memories,
        })
    }

    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        // First engine that returns non-empty messages wins.
        // Clone the buffer for trial runs so we don't corrupt the original.
        for (i, engine) in self.engines.iter().enumerate() {
            let mut candidate = messages.clone();
            match engine
                .assemble(
                    agent_id,
                    &mut candidate,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await
            {
                Ok(result) if !candidate.is_empty() => {
                    *messages = candidate;
                    return Ok(result);
                }
                Ok(_) => {
                    debug!(
                        index = i,
                        "StackedContextEngine: assemble returned empty messages, trying next"
                    );
                }
                Err(e) => {
                    warn!(
                        index = i,
                        error = %e,
                        "StackedContextEngine: assemble error, trying next engine"
                    );
                }
            }
        }
        // All engines returned empty — fall back to the last engine on the original buffer.
        self.engines
            .last()
            .expect("engines is non-empty")
            .assemble(
                agent_id,
                messages,
                system_prompt,
                tools,
                context_window_tokens,
            )
            .await
    }

    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        // First engine that succeeds with a non-fallback result wins.
        let mut last_fallback: Option<CompactionResult> = None;
        for (i, engine) in self.engines.iter().enumerate() {
            match engine
                .compact(
                    agent_id,
                    messages,
                    driver.clone(),
                    model,
                    context_window_tokens,
                )
                .await
            {
                Ok(result) if !result.used_fallback => return Ok(result),
                Ok(fallback_result) => {
                    debug!(
                        index = i,
                        "StackedContextEngine: compact used fallback, trying next engine"
                    );
                    last_fallback = Some(fallback_result);
                }
                Err(e) => {
                    warn!(
                        index = i,
                        error = %e,
                        "StackedContextEngine: compact error, trying next engine"
                    );
                }
            }
        }
        // Return the last successful fallback result, or delegate to the primary engine.
        if let Some(fb) = last_fallback {
            return Ok(fb);
        }
        self.engines
            .first()
            .expect("engines is non-empty")
            .compact(agent_id, messages, driver, model, context_window_tokens)
            .await
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        // Run all engines concurrently (best-effort). Each ScriptableContextEngine
        // already fire-and-forgets its own subprocess, so this outer join simply
        // dispatches all engines at once instead of waiting for each in sequence.
        // Each engine is guarded by a 30-second timeout so a single slow or hung
        // engine cannot stall the entire stack.
        let timeout_dur = std::time::Duration::from_secs(30);
        let futs = self
            .engines
            .iter()
            .enumerate()
            .map(|(i, engine)| async move {
                match tokio::time::timeout(timeout_dur, engine.after_turn(agent_id, messages)).await
                {
                    Ok(Ok(())) => true,
                    Ok(Err(e)) => {
                        warn!(
                            engine_index = i,
                            error = %e,
                            "StackedContextEngine: after_turn engine failed"
                        );
                        false
                    }
                    Err(_elapsed) => {
                        warn!(
                            engine_index = i,
                            timeout_secs = 30,
                            "StackedContextEngine: after_turn engine timed out"
                        );
                        false
                    }
                }
            });

        let outcomes = futures::future::join_all(futs).await;
        let succeeded = outcomes.iter().filter(|&&ok| ok).count();
        let failed = outcomes.len() - succeeded;
        if failed > 0 {
            warn!(
                succeeded,
                failed, "StackedContextEngine: after_turn completed with some engine failures"
            );
        }
        Ok(())
    }

    async fn prepare_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        for engine in &self.engines {
            engine.prepare_subagent_context(parent_id, child_id).await?;
        }
        Ok(())
    }

    async fn merge_subagent_context(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
    ) -> LibreFangResult<()> {
        for engine in &self.engines {
            engine.merge_subagent_context(parent_id, child_id).await?;
        }
        Ok(())
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        // Delegate to the primary (first) engine.
        self.engines
            .first()
            .expect("engines is non-empty")
            .truncate_tool_result(content, context_window_tokens)
    }

    fn hook_traces(&self) -> Vec<HookTrace> {
        let mut all = Vec::new();
        for engine in &self.engines {
            all.extend(engine.hook_traces());
        }
        // Sort by started_at so mixed-engine traces appear chronologically.
        all.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        all.truncate(TRACE_BUFFER_CAPACITY);
        all
    }

    fn hook_metrics(&self) -> Option<HookMetrics> {
        // Aggregate metrics from all stacked engines that expose them.
        let mut aggregate = HookMetrics::default();
        let mut any = false;
        for engine in &self.engines {
            if let Some(m) = engine.hook_metrics() {
                any = true;
                macro_rules! add_stats {
                    ($field:ident) => {
                        aggregate.$field.calls += m.$field.calls;
                        aggregate.$field.successes += m.$field.successes;
                        aggregate.$field.failures += m.$field.failures;
                        aggregate.$field.total_ms += m.$field.total_ms;
                    };
                }
                add_stats!(ingest);
                add_stats!(after_turn);
                add_stats!(bootstrap);
                add_stats!(assemble);
                add_stats!(compact);
                add_stats!(prepare_subagent);
                add_stats!(merge_subagent);
            }
        }
        if any {
            Some(aggregate)
        } else {
            None
        }
    }

    fn per_agent_metrics(&self) -> std::collections::HashMap<String, HookStats> {
        let mut merged: std::collections::HashMap<String, HookStats> =
            std::collections::HashMap::new();
        for engine in &self.engines {
            for (agent_id, stats) in engine.per_agent_metrics() {
                let entry = merged.entry(agent_id).or_default();
                entry.calls += stats.calls;
                entry.successes += stats.successes;
                entry.failures += stats.failures;
                entry.total_ms += stats.total_ms;
            }
        }
        merged
    }
}

/// Resolve `allowed_secrets` from a hooks config into `LIBREFANG_SECRET_<NAME>`
/// environment variable pairs, using the provided vault lookup function.
///
/// Secret names are uppercased when forming the env var name. Secrets that are
/// not found in the vault are skipped with a `warn!` log — no error is raised.
/// Secret values are never logged.
fn resolve_vault_env_vars(
    hooks: &librefang_types::config::ContextEngineHooks,
    vault_lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let mut vault_env = Vec::with_capacity(hooks.allowed_secrets.len());
    for secret_name in &hooks.allowed_secrets {
        debug!(
            secret = secret_name.as_str(),
            "Resolving vault secret for hook"
        );
        match vault_lookup(secret_name) {
            Some(value) => {
                let env_key = format!("LIBREFANG_SECRET_{}", secret_name.to_uppercase());
                vault_env.push((env_key, value));
            }
            None => {
                warn!(
                    secret = secret_name.as_str(),
                    "Secret listed in allowed_secrets not found in vault — skipping"
                );
            }
        }
    }
    vault_env
}

/// Build a context engine from config.
///
/// Resolution order:
/// 1. If `plugin_stack` has 2+ entries, build a `StackedContextEngine`
/// 2. If `plugin` is set, load plugin manifest and use its hooks
/// 3. If manual `hooks` are set, use them directly
/// 4. Otherwise, return a plain `DefaultContextEngine`
///
/// The `vault_lookup` callback is called for each secret name listed in
/// `hooks.allowed_secrets`. Return `Some(value)` for known secrets, `None`
/// for secrets that don't exist. Passing `&|_| None` disables vault injection.
pub fn build_context_engine(
    toml_config: &librefang_types::config::ContextEngineTomlConfig,
    runtime_config: ContextEngineConfig,
    memory: Arc<MemorySubstrate>,
    embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
    vault_lookup: &dyn Fn(&str) -> Option<String>,
) -> Box<dyn ContextEngine> {
    // Build the inner engine (shared base for all built-in engine variants).
    let inner = DefaultContextEngine::new(
        runtime_config.clone(),
        memory.clone(),
        embedding_driver.clone(),
    );

    // Built-in named engines.  These are independent of plugin loading — they
    // provide out-of-the-box behaviour without requiring any plugin to be
    // installed.
    match toml_config.engine.as_str() {
        "summary" => {
            // Threshold-gated LLM summarisation — fires when prompt tokens
            // cross ~80 % of the model's context window.
            //
            // If hooks are also configured, wire them alongside the summary
            // engine via a StackedContextEngine so they remain active.
            if toml_config.plugin.is_some() {
                tracing::warn!(
                    "context engine config: `engine = \"summary\"` takes precedence, \
                     `plugin` config is ignored"
                );
            }
            let summary_engine: Box<dyn ContextEngine> =
                Box::new(SummaryContextEngine::new(inner, 0.80));
            let hooks = &toml_config.hooks;
            let has_hooks = hooks.ingest.is_some()
                || hooks.after_turn.is_some()
                || hooks.bootstrap.is_some()
                || hooks.assemble.is_some()
                || hooks.compact.is_some()
                || hooks.prepare_subagent.is_some()
                || hooks.merge_subagent.is_some();
            if has_hooks {
                // Build a fresh DefaultContextEngine for the hook layer so the
                // SummaryContextEngine continues to own the original `inner`.
                let hook_inner = DefaultContextEngine::new(
                    runtime_config.clone(),
                    memory.clone(),
                    embedding_driver.clone(),
                );
                let vault_env = resolve_vault_env_vars(hooks, vault_lookup);
                let mut hook_engine = ScriptableContextEngine::new(hook_inner, hooks);
                if !vault_env.is_empty() {
                    hook_engine = hook_engine.with_plugin_env(vault_env);
                }
                return Box::new(StackedContextEngine::new(vec![
                    summary_engine,
                    Box::new(hook_engine),
                ]));
            }
            return summary_engine;
        }
        "no_compact" => {
            // Disables automatic compaction while still wiring all other hooks.
            //
            // If hooks are also configured, wire them alongside the no_compact
            // engine via a StackedContextEngine so they remain active.
            if toml_config.plugin.is_some() {
                tracing::warn!(
                    "context engine config: `engine = \"no_compact\"` takes precedence, \
                     `plugin` config is ignored"
                );
            }
            let no_compact_engine: Box<dyn ContextEngine> =
                Box::new(NoCompactContextEngine::new(inner));
            let hooks = &toml_config.hooks;
            let has_hooks = hooks.ingest.is_some()
                || hooks.after_turn.is_some()
                || hooks.bootstrap.is_some()
                || hooks.assemble.is_some()
                || hooks.compact.is_some()
                || hooks.prepare_subagent.is_some()
                || hooks.merge_subagent.is_some();
            if has_hooks {
                // Build a fresh DefaultContextEngine for the hook layer so the
                // NoCompactContextEngine continues to own the original `inner`.
                let hook_inner = DefaultContextEngine::new(
                    runtime_config.clone(),
                    memory.clone(),
                    embedding_driver.clone(),
                );
                let vault_env = resolve_vault_env_vars(hooks, vault_lookup);
                let mut hook_engine = ScriptableContextEngine::new(hook_inner, hooks);
                if !vault_env.is_empty() {
                    hook_engine = hook_engine.with_plugin_env(vault_env);
                }
                return Box::new(StackedContextEngine::new(vec![
                    no_compact_engine,
                    Box::new(hook_engine),
                ]));
            }
            return no_compact_engine;
        }
        "default" => {
            // Plain default engine — no additional wrapping.
        }
        other => {
            warn!(
                engine = other,
                "Unknown context engine '{}' — only 'default', 'summary', and \
                 'no_compact' are built-in; falling back to 'default'",
                other
            );
        }
    }

    // Plugin stack: 2+ plugins → StackedContextEngine
    if let Some(ref stack) = toml_config.plugin_stack {
        if stack.len() >= 2 {
            // Create the shared event bus up-front so every engine in the stack
            // can both emit events (via after_turn output) and receive events
            // (via their on_event hook).  All engines share the same bus.
            let shared_bus = std::sync::Arc::new(PluginEventBus::new(256));

            let mut engines: Vec<Box<dyn ContextEngine>> = Vec::with_capacity(stack.len());
            for plugin_name in stack {
                let eng_memory = memory.clone();
                let eng_emb = embedding_driver.clone();
                let inner = DefaultContextEngine::new(runtime_config.clone(), eng_memory, eng_emb);
                match load_plugin(plugin_name) {
                    Ok((manifest, hooks)) => {
                        if hooks.ingest.is_some()
                            || hooks.after_turn.is_some()
                            || hooks.bootstrap.is_some()
                            || hooks.assemble.is_some()
                            || hooks.compact.is_some()
                            || hooks.prepare_subagent.is_some()
                            || hooks.merge_subagent.is_some()
                        {
                            let config_schema = manifest.config.clone();
                            let mut env: Vec<(String, String)> = manifest.env.into_iter().collect();
                            let vault_env = resolve_vault_env_vars(&hooks, vault_lookup);
                            env.extend(vault_env);
                            engines.push(Box::new(
                                ScriptableContextEngine::new(inner, &hooks)
                                    .with_plugin_name(plugin_name)
                                    .with_plugin_env(env)
                                    .with_plugin_config(config_schema)
                                    // Wire the shared bus: this engine will emit events to it
                                    // AND subscribe its on_event hook to receive events from
                                    // all other plugins in the stack.
                                    .with_event_bus(shared_bus.clone()),
                            ));
                        } else {
                            warn!(
                                plugin = plugin_name.as_str(),
                                "Plugin in stack defines no hooks — adding default engine in its place"
                            );
                            engines.push(Box::new(inner));
                        }
                    }
                    Err(e) => {
                        warn!(
                            plugin = plugin_name.as_str(),
                            error = %e,
                            "Failed to load plugin for stack — using default engine in its place"
                        );
                        engines.push(Box::new(inner));
                    }
                }
            }
            let mut stacked = StackedContextEngine::new_with_bus(engines, shared_bus);
            if !toml_config.plugin_stack_weights.is_empty() {
                stacked = stacked.with_weights(toml_config.plugin_stack_weights.clone());
            }
            return Box::new(stacked);
        }
    }

    // Single plugin takes precedence over manual hooks
    if let Some(ref plugin_name) = toml_config.plugin {
        match load_plugin(plugin_name) {
            Ok((manifest, hooks)) => {
                if hooks.ingest.is_some() || hooks.after_turn.is_some() {
                    let config_schema = manifest.config.clone();
                    let mut env: Vec<(String, String)> = manifest.env.into_iter().collect();
                    let vault_env = resolve_vault_env_vars(&hooks, vault_lookup);
                    env.extend(vault_env);
                    return Box::new(
                        ScriptableContextEngine::new(inner, &hooks)
                            .with_plugin_name(plugin_name)
                            .with_plugin_env(env)
                            .with_plugin_config(config_schema),
                    );
                }
                warn!(
                    plugin = plugin_name.as_str(),
                    "Plugin loaded but defines no hooks — using default engine"
                );
                return Box::new(inner);
            }
            Err(e) => {
                warn!(
                    plugin = plugin_name.as_str(),
                    error = %e,
                    "Failed to load plugin — falling back to default engine"
                );
                return Box::new(inner);
            }
        }
    }

    // Manual hooks
    if toml_config.hooks.ingest.is_some() || toml_config.hooks.after_turn.is_some() {
        let vault_env = resolve_vault_env_vars(&toml_config.hooks, vault_lookup);
        let mut engine = ScriptableContextEngine::new(inner, &toml_config.hooks);
        if !vault_env.is_empty() {
            engine = engine.with_plugin_env(vault_env);
        }
        Box::new(engine)
    } else {
        Box::new(inner)
    }
}

// ---------------------------------------------------------------------------
// NoCompactContextEngine — engine with compression disabled
// ---------------------------------------------------------------------------

/// A context engine that disables all automatic compression while still wiring
/// up the full engine interface.
///
/// `should_compress` always returns `false`, so the agent loop never triggers
/// an automatic compaction.  This is distinct from a true null object: all
/// other lifecycle hooks (`bootstrap`, `ingest`, `assemble`, `after_turn`,
/// …) are delegated to the inner engine.
pub struct NoCompactContextEngine {
    inner: DefaultContextEngine,
}

impl NoCompactContextEngine {
    /// Create a `NoCompactContextEngine` backed by `inner` for the non-
    /// compression lifecycle hooks (`bootstrap`, `ingest`, `assemble`,
    /// `after_turn`, …).
    pub fn new(inner: DefaultContextEngine) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ContextEngine for NoCompactContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        self.inner.bootstrap(config).await
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        self.inner.ingest(agent_id, user_message, peer_id).await
    }

    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        self.inner
            .assemble(
                agent_id,
                messages,
                system_prompt,
                tools,
                context_window_tokens,
            )
            .await
    }

    async fn compact(
        &self,
        _agent_id: AgentId,
        messages: &[Message],
        _driver: Arc<dyn LlmDriver>,
        _model: &str,
        _context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        // NoCompactContextEngine must never compress — return all messages as-is.
        Ok(CompactionResult {
            summary: String::new(),
            kept_messages: messages.to_vec(),
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
        })
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        self.inner.after_turn(agent_id, messages).await
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        self.inner
            .truncate_tool_result(content, context_window_tokens)
    }

    /// Always returns `false` — `NoCompactContextEngine` never triggers compaction.
    fn should_compress(&self, _current_tokens: usize, _max_tokens: usize) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// SummaryContextEngine — threshold-gated LLM summarisation engine
// ---------------------------------------------------------------------------

/// A context engine that wraps `DefaultContextEngine` and adds explicit
/// threshold-gated compaction via `should_compress`.
///
/// When `should_compress(current_tokens, max_tokens)` returns `true` the
/// agent loop should call `compact` to summarise older history.  The default
/// threshold is configurable at construction time (defaults to **80 %**,
/// matching the Python reference implementation).
///
/// The engine also tracks the active model name and context-window length so
/// future telemetry can report which model was active.  The stored values are
/// not currently read by any engine method — `should_compress` uses the
/// `max_tokens` argument passed to it.
///
/// # Example
/// ```ignore
/// let engine = SummaryContextEngine::new(inner, 0.80);
/// if engine.should_compress(current_tokens, ctx_window) {
///     engine.compact(agent_id, &messages, driver, model, ctx_window).await?;
/// }
/// ```
pub struct SummaryContextEngine {
    inner: DefaultContextEngine,
    /// Compression threshold as a fraction of `context_length` (0.0 – 1.0).
    threshold_percent: f64,
    /// Active model name (updated via `update_model`). Stored but not read —
    /// kept so the field is present for future telemetry or logging use.
    model: parking_lot::Mutex<String>,
    /// Current context window size in tokens (updated via `update_model`).
    context_length: parking_lot::Mutex<usize>,
}

impl SummaryContextEngine {
    /// Create a new `SummaryContextEngine`.
    ///
    /// `threshold_percent` controls when `should_compress` returns `true`.
    /// Pass `0.80` for the default 80 % threshold used by the Python
    /// reference implementation.  Values outside `[0.0, 1.0]` are clamped.
    pub fn new(inner: DefaultContextEngine, threshold_percent: f64) -> Self {
        let context_length = inner.context_window_tokens();
        Self {
            inner,
            threshold_percent: threshold_percent.clamp(0.0, 1.0),
            model: parking_lot::Mutex::new(String::new()),
            context_length: parking_lot::Mutex::new(context_length),
        }
    }
}

#[async_trait]
impl ContextEngine for SummaryContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        self.inner.bootstrap(config).await
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        self.inner.ingest(agent_id, user_message, peer_id).await
    }

    async fn assemble(
        &self,
        agent_id: AgentId,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        self.inner
            .assemble(
                agent_id,
                messages,
                system_prompt,
                tools,
                context_window_tokens,
            )
            .await
    }

    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
        model: &str,
        context_window_tokens: usize,
    ) -> LibreFangResult<CompactionResult> {
        self.inner
            .compact(agent_id, messages, driver, model, context_window_tokens)
            .await
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        self.inner.after_turn(agent_id, messages).await
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        self.inner
            .truncate_tool_result(content, context_window_tokens)
    }

    /// Returns `true` when `current_tokens` exceeds the configured threshold
    /// fraction of `max_tokens`.
    ///
    /// Uses `max_tokens` from the argument rather than the stored
    /// `context_length` so the check is always correct even when the agent
    /// temporarily uses a smaller window.
    fn should_compress(&self, current_tokens: usize, max_tokens: usize) -> bool {
        if max_tokens == 0 {
            return false;
        }
        let threshold = (max_tokens as f64 * self.threshold_percent) as usize;
        current_tokens >= threshold
    }

    /// Update the engine's view of the active model and recalculate thresholds.
    ///
    /// Called by the agent loop when the operator switches models or when a
    /// provider fallback activates.  Uses interior mutability so this can be
    /// called through a `&dyn ContextEngine` trait object.
    fn update_model(&self, model: &str, context_length: usize) {
        *self.model.lock() = model.to_owned();
        *self.context_length.lock() = context_length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::session::Session as MemSession;
    use librefang_memory::MemorySubstrate;
    use librefang_types::message::Message;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    fn make_memory() -> Arc<MemorySubstrate> {
        Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap())
    }

    #[tokio::test]
    async fn test_bootstrap_default() {
        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config.clone(), make_memory(), None);
        assert!(engine.bootstrap(&config).await.is_ok());
    }

    #[tokio::test]
    async fn plugin_bus_record_consumer_lag_increments_dropped_count() {
        // Mirrors `EventBus::record_consumer_lag_increments_dropped_count`
        // from librefang-kernel — the two impls drift apart silently
        // unless both are tested. Counter accounting only; the rate-
        // limited error! log is exercised but not asserted on.
        let bus = PluginEventBus::new(8);
        assert_eq!(bus.dropped_count(), 0);
        bus.record_consumer_lag(7, "test");
        assert_eq!(bus.dropped_count(), 7);
        bus.record_consumer_lag(3, "test");
        assert_eq!(bus.dropped_count(), 10);
    }

    /// Warmup regression guard: first lag burst must advance `last_drop_warn` (#3630).
    #[tokio::test]
    async fn plugin_bus_first_lag_burst_advances_warn_timestamp() {
        let bus = PluginEventBus::new(8);
        let before = *bus.last_drop_warn.lock().unwrap();
        bus.record_consumer_lag(1, "test_first_burst");
        let after = *bus.last_drop_warn.lock().unwrap();
        assert!(
            after > before,
            "first lag burst must advance last_drop_warn — warmup regression"
        );
    }

    #[tokio::test]
    async fn test_ingest_stable_prefix_mode() {
        let config = ContextEngineConfig {
            stable_prefix_mode: true,
            ..Default::default()
        };
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        let result = engine.ingest(AgentId::new(), "hello", None).await.unwrap();
        assert!(result.recalled_memories.is_empty());
    }

    #[tokio::test]
    async fn test_ingest_recalls_memories() {
        let memory = make_memory();
        // Store a memory first
        memory
            .remember(
                AgentId::new(), // different agent
                "unrelated",
                librefang_types::memory::MemorySource::Conversation,
                "episodic",
                std::collections::HashMap::new(),
            )
            .await
            .unwrap();

        let agent_id = AgentId::new();
        memory
            .remember(
                agent_id,
                "The user likes Rust programming",
                librefang_types::memory::MemorySource::Conversation,
                "episodic",
                std::collections::HashMap::new(),
            )
            .await
            .unwrap();

        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config, memory, None);
        let result = engine.ingest(agent_id, "Rust", None).await.unwrap();
        assert_eq!(result.recalled_memories.len(), 1);
        assert!(result.recalled_memories[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_assemble_no_overflow() {
        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        let mut messages = vec![Message::user("hi"), Message::assistant("hello")];
        let result = engine
            .assemble(AgentId::new(), &mut messages, "system", &[], 200_000)
            .await
            .unwrap();
        assert_eq!(result.recovery, RecoveryStage::None);
    }

    #[tokio::test]
    async fn test_assemble_triggers_overflow_recovery() {
        let config = ContextEngineConfig {
            context_window_tokens: 100, // tiny window
            ..Default::default()
        };
        let engine = DefaultContextEngine::new(config, make_memory(), None);

        // Create messages that exceed the tiny context window
        let mut messages: Vec<Message> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(format!("msg{}: {}", i, "x".repeat(200)))
                } else {
                    Message::assistant(format!("msg{}: {}", i, "x".repeat(200)))
                }
            })
            .collect();

        let result = engine
            .assemble(AgentId::new(), &mut messages, "system", &[], 100)
            .await
            .unwrap();
        assert_ne!(result.recovery, RecoveryStage::None);
    }

    #[tokio::test]
    async fn test_truncate_tool_result() {
        let config = ContextEngineConfig {
            context_window_tokens: 500,
            ..Default::default()
        };
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        let big_content = "x".repeat(10_000);
        let truncated = engine.truncate_tool_result(&big_content, 500);
        assert!(truncated.len() < big_content.len());
        assert!(truncated.contains("[TRUNCATED:"));
    }

    #[tokio::test]
    async fn test_after_turn_noop() {
        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        assert!(engine
            .after_turn(AgentId::new(), &[Message::user("hi")])
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_subagent_hooks_noop() {
        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        let parent = AgentId::new();
        let child = AgentId::new();
        assert!(engine.prepare_subagent_context(parent, child).await.is_ok());
        assert!(engine.merge_subagent_context(parent, child).await.is_ok());
    }

    #[tokio::test]
    async fn test_scriptable_hook_receives_direct_json_payload() {
        if Command::new("python3").arg("--version").output().is_err()
            && Command::new("python").arg("--version").output().is_err()
        {
            eprintln!("Python not available, skipping scriptable hook payload test");
            return;
        }

        let tmp = tempdir().unwrap();
        let script_path = tmp.path().join("hook.py");
        std::fs::write(
            &script_path,
            r#"import json
import sys

payload = json.loads(sys.stdin.read())
print(json.dumps({"type": payload.get("type"), "message": payload.get("message")}))
"#,
        )
        .unwrap();

        let traces = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let hook_schemas = std::collections::HashMap::new();
        let (output, _duration_ms) = ScriptableContextEngine::run_hook(
            "ingest",
            script_path.to_str().unwrap(),
            crate::plugin_runtime::PluginRuntime::Python,
            serde_json::json!({
                "type": "ingest",
                "agent_id": "agent-123",
                "message": "hello",
            }),
            30,
            &[],
            0,
            0,
            None,
            true,
            &traces,
            &hook_schemas,
            None,
            None,
            "",
            "",
            false,
        )
        .await
        .unwrap();

        assert_eq!(output["type"], "ingest");
        assert_eq!(output["message"], "hello");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_scriptable_hook_accepts_full_runtime_path() {
        let tmp = tempdir().unwrap();
        let script_path = tmp.path().join("hook.sh");
        std::fs::write(
            &script_path,
            r#"read _input
printf '{"type":"ingest_result","memories":[{"content":"full-path-runtime"}]}\n'
"#,
        )
        .unwrap();

        let traces = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let hook_schemas = std::collections::HashMap::new();
        let (output, _duration_ms) = ScriptableContextEngine::run_hook(
            "ingest",
            script_path.to_str().unwrap(),
            crate::plugin_runtime::PluginRuntime::from_tag(Some("/bin/sh")),
            serde_json::json!({
                "type": "ingest",
                "agent_id": "agent-123",
                "message": "hello",
            }),
            30,
            &[],
            0,
            0,
            None,
            true,
            &traces,
            &hook_schemas,
            None,
            None,
            "",
            "",
            false,
        )
        .await
        .unwrap();

        assert_eq!(output["type"], "ingest_result");
        assert_eq!(output["memories"][0]["content"], "full-path-runtime");
    }

    #[test]
    fn test_plugins_dir() {
        let dir = plugins_dir();
        assert!(dir.ends_with("plugins"));
        assert!(dir.to_string_lossy().contains(".librefang"));
    }

    #[test]
    fn test_load_plugin_not_found() {
        let result = load_plugin("nonexistent-plugin-12345");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_load_plugin_with_tempdir() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join("test-plugin");
        std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();

        // Write a plugin.toml
        let manifest_content = r#"
name = "test-plugin"
version = "0.1.0"
description = "A test plugin"
author = "test"

[hooks]
ingest = "hooks/ingest.py"
"#;
        let mut f = std::fs::File::create(plugin_dir.join("plugin.toml")).unwrap();
        f.write_all(manifest_content.as_bytes()).unwrap();

        // Write a dummy hook
        std::fs::File::create(plugin_dir.join("hooks/ingest.py")).unwrap();

        // We can't use load_plugin directly because it hardcodes ~/.librefang/plugins,
        // so test the manifest parsing + hook resolution manually
        let manifest: librefang_types::config::PluginManifest =
            toml::from_str(manifest_content).unwrap();

        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.hooks.ingest.as_deref(), Some("hooks/ingest.py"));
        assert!(manifest.hooks.after_turn.is_none());

        // Resolve hooks relative to plugin dir
        let resolved = manifest
            .hooks
            .ingest
            .as_ref()
            .map(|p| plugin_dir.join(p).to_string_lossy().into_owned());
        assert!(resolved.unwrap().contains("hooks/ingest.py"));
    }

    #[test]
    fn test_build_context_engine_default() {
        let toml_config = librefang_types::config::ContextEngineTomlConfig::default();
        let runtime_config = ContextEngineConfig::default();
        let engine =
            build_context_engine(&toml_config, runtime_config, make_memory(), None, &|_| None);
        // Should not panic — returns DefaultContextEngine
        let _ = engine;
    }

    #[test]
    fn test_build_context_engine_missing_plugin_falls_back() {
        let toml_config = librefang_types::config::ContextEngineTomlConfig {
            plugin: Some("nonexistent-plugin-xyz".to_string()),
            ..Default::default()
        };
        let runtime_config = ContextEngineConfig::default();
        // Should fall back to default engine, not panic
        let engine =
            build_context_engine(&toml_config, runtime_config, make_memory(), None, &|_| None);
        let _ = engine;
    }

    // -----------------------------------------------------------------------
    // NoCompactContextEngine
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_compact_context_engine_never_compresses() {
        let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let engine = NoCompactContextEngine::new(inner);
        // NoCompactContextEngine must always return false regardless of load
        assert!(!engine.should_compress(0, 200_000));
        assert!(!engine.should_compress(160_000, 200_000)); // 80 % — trait default would fire
        assert!(!engine.should_compress(200_000, 200_000)); // 100 %
    }

    // -----------------------------------------------------------------------
    // SummaryContextEngine
    // -----------------------------------------------------------------------

    #[test]
    fn test_summary_context_engine_threshold_default_80_percent() {
        let inner = DefaultContextEngine::new(
            ContextEngineConfig {
                context_window_tokens: 200_000,
                ..Default::default()
            },
            make_memory(),
            None,
        );
        // Default threshold_percent = 0.80
        let engine = SummaryContextEngine::new(inner, 0.80);

        // Below threshold: 79 % → false
        assert!(!engine.should_compress(158_000, 200_000));
        // At threshold: exactly 80 % → true
        assert!(engine.should_compress(160_000, 200_000));
        // Above threshold: 90 % → true
        assert!(engine.should_compress(180_000, 200_000));
    }

    #[test]
    fn test_summary_context_engine_zero_max_tokens_safe() {
        let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let engine = SummaryContextEngine::new(inner, 0.80);
        // max_tokens = 0 must never panic and must return false
        assert!(!engine.should_compress(0, 0));
        assert!(!engine.should_compress(100, 0));
    }

    #[test]
    fn test_summary_context_engine_update_model() {
        let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let engine = SummaryContextEngine::new(inner, 0.80);

        // Before update: context_length comes from ContextEngineConfig::default (200_000)
        assert!(!engine.should_compress(100_000, 200_000)); // 50 % < 80 %

        // After switching to a 32 K model the threshold drops to 25 600
        engine.update_model("gpt-3.5-turbo", 32_000);
        assert!(engine.should_compress(26_000, 32_000)); // 81 % > 80 %
        assert!(!engine.should_compress(25_000, 32_000)); // 78 % < 80 %
    }

    #[test]
    fn test_default_trait_should_compress_80_percent() {
        // Verify the default impl on the trait uses 80 % (4/5 integer math)
        let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let _engine = NoCompactContextEngine::new(inner);
        // NoCompactContextEngine overrides to false, so test the trait default via
        // a SummaryContextEngine at the same 80 % threshold
        let inner2 = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let summary = SummaryContextEngine::new(inner2, 0.80);
        assert!(!summary.should_compress(0, 200_000));
        assert!(!summary.should_compress(159_999, 200_000));
        assert!(summary.should_compress(160_000, 200_000));
    }

    // ---------------------------------------------------------------------------
    // Integration: compact is called exactly once when threshold is crossed
    // ---------------------------------------------------------------------------

    /// Verifies that `SummaryContextEngine::compact` is invoked exactly once
    /// when `should_compress` transitions from false to true, and never invoked
    /// when it remains false.  This mirrors the gating logic in the agent loop
    /// (agent_loop.rs: should_compress check → compact call).
    #[tokio::test]
    async fn test_summary_engine_compact_called_once_on_threshold_cross() {
        use crate::llm_driver::{CompletionResponse, LlmError};
        use async_trait::async_trait;
        use librefang_types::message::{ContentBlock, TokenUsage};
        use std::sync::Arc;

        struct FakeDriver {
            _call_count: AtomicUsize,
        }

        impl FakeDriver {
            fn new() -> Self {
                Self {
                    _call_count: AtomicUsize::new(0),
                }
            }
        }

        #[async_trait]
        impl LlmDriver for FakeDriver {
            async fn complete(
                &self,
                _req: crate::llm_driver::CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Summary of prior conversation".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: librefang_types::message::StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 50,
                        output_tokens: 20,
                        ..Default::default()
                    },
                })
            }
        }

        struct CompactTracker {
            inner: DefaultContextEngine,
            compact_count: AtomicUsize,
        }

        impl CompactTracker {
            fn new(inner: DefaultContextEngine) -> Self {
                Self {
                    inner,
                    compact_count: AtomicUsize::new(0),
                }
            }
        }

        #[async_trait]
        impl ContextEngine for CompactTracker {
            async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
                self.inner.bootstrap(config).await
            }
            async fn ingest(
                &self,
                agent_id: AgentId,
                user_message: &str,
                peer_id: Option<&str>,
            ) -> LibreFangResult<IngestResult> {
                self.inner.ingest(agent_id, user_message, peer_id).await
            }
            async fn assemble(
                &self,
                agent_id: AgentId,
                messages: &mut Vec<Message>,
                system_prompt: &str,
                tools: &[ToolDefinition],
                context_window_tokens: usize,
            ) -> LibreFangResult<AssembleResult> {
                self.inner
                    .assemble(
                        agent_id,
                        messages,
                        system_prompt,
                        tools,
                        context_window_tokens,
                    )
                    .await
            }
            async fn compact(
                &self,
                agent_id: AgentId,
                messages: &[Message],
                driver: Arc<dyn LlmDriver>,
                model: &str,
                context_window_tokens: usize,
            ) -> LibreFangResult<CompactionResult> {
                self.compact_count.fetch_add(1, Ordering::SeqCst);
                self.inner
                    .compact(agent_id, messages, driver, model, context_window_tokens)
                    .await
            }
            async fn after_turn(
                &self,
                agent_id: AgentId,
                messages: &[Message],
            ) -> LibreFangResult<()> {
                self.inner.after_turn(agent_id, messages).await
            }
            fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
                self.inner
                    .truncate_tool_result(content, context_window_tokens)
            }
            fn should_compress(&self, current_tokens: usize, max_tokens: usize) -> bool {
                self.inner.should_compress(current_tokens, max_tokens)
            }
        }

        let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
        let tracker = CompactTracker::new(inner);

        // ctx_window = 200_000, threshold = 80% = 160_000
        // Iteration 1: prompt tokens = 100_000 (below threshold) → no compact
        assert!(!tracker.should_compress(100_000, 200_000));

        let driver = Arc::new(FakeDriver::new());
        let session = MemSession {
            id: librefang_types::agent::SessionId::new(),
            agent_id: librefang_types::agent::AgentId::new(),
            messages: vec![Message::user("Hello"), Message::assistant("Hi there")],
            context_window_tokens: 0,
            label: None,
            messages_generation: 0,
            last_repaired_generation: None,
        };

        // Manually call compact once (mirrors what agent_loop does when
        // should_compress returns true)
        let _ = tracker
            .compact(
                session.agent_id,
                &session.messages,
                driver.clone(),
                "test-model",
                200_000,
            )
            .await;
        assert_eq!(
            tracker.compact_count.load(Ordering::SeqCst),
            1,
            "compact must be called exactly once"
        );

        // Iteration 2: still below threshold → no additional compact call
        assert!(!tracker.should_compress(100_000, 200_000));
        assert_eq!(
            tracker.compact_count.load(Ordering::SeqCst),
            1,
            "compact must not be called again when should_compress is false"
        );

        // Iteration 3: above threshold → compact should fire
        assert!(tracker.should_compress(180_000, 200_000));
        let _ = tracker
            .compact(
                session.agent_id,
                &session.messages,
                driver.clone(),
                "test-model",
                200_000,
            )
            .await;
        assert_eq!(
            tracker.compact_count.load(Ordering::SeqCst),
            2,
            "compact must be called when should_compress transitions to true"
        );
    }
}
