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

mod scriptable;

pub use self::scriptable::{
    plugins_dir, HookMetrics, HookStats, HookTrace, ScriptableContextEngine,
};

use self::scriptable::{load_plugin, TRACE_BUFFER_CAPACITY};

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
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };

        let mut compaction_config = self.compaction_config.clone();
        compaction_config.context_window_tokens = context_window_tokens;

        compactor::compact_session(
            driver,
            model,
            &session,
            &compaction_config,
            // The trait-level default ContextEngine doesn't carry a
            // catalog reference; rely on the driver's substring fallback.
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
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
mod tests;
