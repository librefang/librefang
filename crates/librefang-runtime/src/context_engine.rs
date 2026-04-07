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
}

impl Default for ContextEngineConfig {
    fn default() -> Self {
        Self {
            context_window_tokens: 200_000,
            stable_prefix_mode: false,
            max_recall_results: 5,
            compaction: None,
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
            ("ingest",           &hooks.ingest),
            ("after_turn",       &hooks.after_turn),
            ("bootstrap",        &hooks.bootstrap),
            ("assemble",         &hooks.assemble),
            ("compact",          &hooks.compact),
            ("prepare_subagent", &hooks.prepare_subagent),
            ("merge_subagent",   &hooks.merge_subagent),
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
        }
    }

    /// Set plugin-level env vars from `[env]` in plugin.toml.
    pub fn with_plugin_env(mut self, env: Vec<(String, String)>) -> Self {
        self.plugin_env = env;
        self
    }

    /// Return a snapshot of all hook invocation metrics.
    pub fn metrics(&self) -> HookMetrics {
        self.metrics.lock().unwrap_or_else(|p| p.into_inner()).clone()
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

    /// Push a trace record into the ring buffer, evicting the oldest if full.
    fn push_trace(
        traces: &std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
        trace: HookTrace,
    ) {
        if let Ok(mut buf) = traces.lock() {
            if buf.len() >= TRACE_BUFFER_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(trace);
        }
    }

    /// Check whether a schema validation warning should be logged.
    ///
    /// Only validates that the output is an object (basic sanity).
    /// Full JSON Schema validation is deferred — this is a structural check only.
    fn validate_schema(schema: &serde_json::Value, value: &serde_json::Value, context: &str) {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            if let Some(obj) = value.as_object() {
                for field in required {
                    if let Some(field_str) = field.as_str() {
                        if !obj.contains_key(field_str) {
                            warn!(
                                context,
                                missing_field = field_str,
                                "Hook schema validation: required field missing"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Returns true when the agent_id passes the configured agent_id_filter.
    fn agent_passes_filter(&self, agent_id: &AgentId) -> bool {
        if self.agent_id_filter.is_empty() {
            return true;
        }
        let id_str = agent_id.0.to_string();
        self.agent_id_filter.iter().any(|f| id_str.contains(f.as_str()))
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
    ) -> Result<(serde_json::Value, u64), String> {
        let resolved = Self::resolve_script_path(script_path);

        if !std::path::Path::new(&resolved).exists() {
            return Err(format!("Hook script not found: {resolved}"));
        }

        // Validate input schema if declared.
        if let Some(schema) = hook_schemas.get(hook_name) {
            if let Some(ref input_schema) = schema.input {
                Self::validate_schema(input_schema, &input, &format!("{hook_name}/input"));
            }
        }

        let config = crate::plugin_runtime::HookConfig {
            timeout_secs,
            plugin_env: plugin_env.to_vec(),
            max_memory_mb,
            allow_network,
            ..Default::default()
        };

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
                tokio::time::sleep(std::time::Duration::from_millis(retry_delay_ms)).await;
                debug!(
                    script = resolved.as_str(),
                    attempt,
                    max_retries,
                    "Retrying hook after failure: {last_err}"
                );
            }
            match crate::plugin_runtime::run_hook_json(&resolved, runtime, &input, &config).await {
                Ok(v) => {
                    let elapsed_ms = t.elapsed().as_millis() as u64;
                    // Validate output schema if declared.
                    if let Some(schema) = hook_schemas.get(hook_name) {
                        if let Some(ref output_schema) = schema.output {
                            Self::validate_schema(output_schema, &v, &format!("{hook_name}/output"));
                        }
                    }
                    Self::push_trace(traces, HookTrace {
                        hook: hook_name.to_string(),
                        started_at: started_at.clone(),
                        elapsed_ms,
                        success: true,
                        error: None,
                        input_preview: input_preview.clone(),
                        output_preview: Some(v.clone()),
                    });
                    return Ok((v, elapsed_ms));
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        let elapsed_ms = t.elapsed().as_millis() as u64;
        let err_msg = format!("Hook script failed after {max_retries} retries: {last_err}");
        Self::push_trace(traces, HookTrace {
            hook: hook_name.to_string(),
            started_at,
            elapsed_ms,
            success: false,
            error: Some(err_msg.clone()),
            input_preview,
            output_preview: None,
        });
        Err(err_msg)
    }

    /// Apply the configured failure policy to a hook error.
    ///
    /// Returns `Ok(None)` when the policy is Warn or Skip (continue with
    /// fallback), or `Err(…)` when the policy is Abort.
    fn apply_failure_policy(
        &self,
        hook: &str,
        err: &str,
    ) -> LibreFangResult<()> {
        use librefang_types::config::HookFailurePolicy;
        match self.on_hook_failure {
            HookFailurePolicy::Warn => {
                warn!(hook, error = err, "Hook failed (warn policy — using fallback)");
                Ok(())
            }
            HookFailurePolicy::Skip => Ok(()), // silent
            HookFailurePolicy::Abort => Err(LibreFangError::Internal(
                format!("Hook '{hook}' failed (abort policy): {err}"),
            )),
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
            match Self::run_hook("bootstrap", script, self.runtime, input, bootstrap_timeout, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
                Ok((_, ms)) => {
                    Self::record_hook(&self.metrics, "bootstrap", ms, true);
                    debug!("Bootstrap hook completed (timeout={bootstrap_timeout}s, {ms}ms)");
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
        if let Some(ref filter) = self.ingest_filter {
            if !user_message.contains(filter.as_str()) {
                debug!(filter = filter.as_str(), "Ingest hook skipped (filter mismatch)");
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
                guard.get(&cache_key).and_then(|(val, exp)| {
                    if exp.elapsed().as_secs() < ttl_secs { Some(val.clone()) } else { None }
                })
            };
            if let Some(cached_output) = cached {
                debug!("Ingest hook cache hit (ttl={}s)", ttl_secs);
                let mut memories = default_result.recalled_memories;
                if let Some(hook_memories) = cached_output.get("memories").and_then(|m| m.as_array()) {
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
                return Ok(IngestResult { recalled_memories: memories });
            }
            // Cache miss — run hook and store result below
            let cache_key_owned = cache_key;
            let cache_arc = self.ingest_cache.clone();
            match Self::run_hook("ingest", script, self.runtime, input.clone(), self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
                Ok((output, ms)) => {
                    Self::record_hook(&self.metrics, "ingest", ms, true);
                    // Store in cache
                    {
                        let mut guard = cache_arc.lock().unwrap();
                        guard.insert(cache_key_owned, (output.clone(), std::time::Instant::now()));
                        // Evict expired entries when cache grows large
                        if guard.len() > 512 {
                            guard.retain(|_, (_, exp)| exp.elapsed().as_secs() < ttl_secs);
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
                    return Ok(IngestResult { recalled_memories: memories });
                }
                Err(err) => {
                    Self::record_hook(&self.metrics, "ingest", 0, false);
                    self.apply_failure_policy("ingest", &err)?;
                    return Ok(default_result); // reached only for Warn/Skip policy
                }
            }
        }

        match Self::run_hook("ingest", script, self.runtime, input, self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
            Ok((output, ms)) => {
                Self::record_hook(&self.metrics, "ingest", ms, true);
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
                .assemble(agent_id, messages, system_prompt, tools, context_window_tokens)
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
            return self.inner.assemble(agent_id, messages, system_prompt, tools, context_window_tokens).await;
        }

        match Self::run_hook("assemble", script, self.runtime, input, self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
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
                    warn!(
                        "Assemble hook returned no 'messages' field, falling back to default"
                    );
                }
                Self::record_hook(&self.metrics, "assemble", ms, false);
                self.inner
                    .assemble(agent_id, messages, system_prompt, tools, context_window_tokens)
                    .await
            }
            Err(e) => {
                Self::record_hook(&self.metrics, "assemble", 0, false);
                self.apply_failure_policy("assemble", &e)?;
                self.inner
                    .assemble(agent_id, messages, system_prompt, tools, context_window_tokens)
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

        let input = serde_json::json!({
            "type": "compact",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_values,
            "model": model,
            "context_window_tokens": context_window_tokens,
        });

        match Self::run_hook("compact", script, self.runtime, input, self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
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
                        return Ok(CompactionResult {
                            summary,
                            kept_messages: compacted,
                            compacted_count: messages.len(),
                            chunks_used: 1,
                            used_fallback: false,
                        });
                    }
                    warn!("Compact hook returned empty messages, falling back to default");
                } else {
                    warn!(
                        "Compact hook returned no 'messages' field, falling back to default"
                    );
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
        // Apply agent_id_filter for after_turn hook.
        if !self.agent_passes_filter(&agent_id) {
            return Ok(());
        }

        let script = script.clone();
        let runtime = self.runtime;
        let timeout_secs = self.hook_timeout_secs;
        let plugin_env = self.plugin_env.clone();
        let metrics = std::sync::Arc::clone(&self.metrics);
        let max_retries = self.max_retries;
        let retry_delay_ms = self.retry_delay_ms;
        let max_memory_mb = self.max_memory_mb;
        let allow_network = self.allow_network;
        let traces = std::sync::Arc::clone(&self.traces);
        let hook_schemas = self.hook_schemas.clone();
        let handle = tokio::spawn(async move {
            match Self::run_hook("after_turn", &script, runtime, input, timeout_secs, &plugin_env, max_retries, retry_delay_ms, max_memory_mb, allow_network, &traces, &hook_schemas).await {
                Ok((_, ms)) => {
                    Self::record_hook(&metrics, "after_turn", ms, true);
                    debug!("After-turn hook completed ({ms}ms)");
                }
                Err(e) => {
                    Self::record_hook(&metrics, "after_turn", 0, false);
                    warn!("After-turn hook failed: {e}");
                }
            }
        });
        tokio::spawn(async move {
            if let Err(e) = handle.await {
                warn!("After-turn hook task panicked: {e}");
            }
        });

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
            match Self::run_hook("prepare_subagent", script, self.runtime, input, self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
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
            match Self::run_hook("merge_subagent", script, self.runtime, input, self.hook_timeout_secs, &self.plugin_env, self.max_retries, self.retry_delay_ms, self.max_memory_mb, self.allow_network, &self.traces, &self.hook_schemas).await {
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

pub fn load_plugin(
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

/// List all installed plugins in `~/.librefang/plugins/`.
pub fn list_installed_plugins() -> Vec<librefang_types::config::PluginManifest> {
    let dir = plugins_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            load_plugin(&name).ok().map(|(manifest, _)| manifest)
        })
        .collect()
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
}

impl StackedContextEngine {
    /// Create a stacked engine from an ordered list of constituent engines.
    ///
    /// Panics if `engines` is empty.
    pub fn new(engines: Vec<Box<dyn ContextEngine>>) -> Self {
        assert!(
            !engines.is_empty(),
            "StackedContextEngine requires at least one engine"
        );
        Self { engines }
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
        let mut all_memories = Vec::new();
        for engine in &self.engines {
            match engine.ingest(agent_id, user_message, peer_id).await {
                Ok(result) => all_memories.extend(result.recalled_memories),
                Err(e) => {
                    warn!(
                        error = %e,
                        "StackedContextEngine: ingest error from engine (skipping)"
                    );
                }
            }
        }
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
        // Run all engines sequentially (best-effort); failures are logged but not propagated.
        // Note: true parallel execution would require Arc-wrapping engines; sequential
        // fire-and-ignore is sufficient since after_turn hook scripts are non-blocking
        // subprocesses that already run in background tasks within each engine.
        for (i, engine) in self.engines.iter().enumerate() {
            if let Err(e) = engine.after_turn(agent_id, messages).await {
                warn!(
                    index = i,
                    error = %e,
                    "StackedContextEngine: after_turn error (best-effort)"
                );
            }
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
        if any { Some(aggregate) } else { None }
    }
}

/// Build a context engine from config.
///
/// Resolution order:
/// 1. If `plugin_stack` has 2+ entries, build a `StackedContextEngine`
/// 2. If `plugin` is set, load plugin manifest and use its hooks
/// 3. If manual `hooks` are set, use them directly
/// 4. Otherwise, return a plain `DefaultContextEngine`
pub fn build_context_engine(
    toml_config: &librefang_types::config::ContextEngineTomlConfig,
    runtime_config: ContextEngineConfig,
    memory: Arc<MemorySubstrate>,
    embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
) -> Box<dyn ContextEngine> {
    // Warn if an unknown engine name is configured
    if toml_config.engine != "default" {
        warn!(
            engine = toml_config.engine.as_str(),
            "Unknown context engine '{}' — only 'default' is built-in, falling back",
            toml_config.engine
        );
    }

    // Plugin stack: 2+ plugins → StackedContextEngine
    if let Some(ref stack) = toml_config.plugin_stack {
        if stack.len() >= 2 {
            let mut engines: Vec<Box<dyn ContextEngine>> = Vec::with_capacity(stack.len());
            for plugin_name in stack {
                let eng_memory = memory.clone();
                let eng_emb = embedding_driver.clone();
                let inner =
                    DefaultContextEngine::new(runtime_config.clone(), eng_memory, eng_emb);
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
                            let env: Vec<(String, String)> =
                                manifest.env.into_iter().collect();
                            engines.push(Box::new(
                                ScriptableContextEngine::new(inner, &hooks)
                                    .with_plugin_env(env),
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
            return Box::new(StackedContextEngine::new(engines));
        }
    }

    let default = DefaultContextEngine::new(runtime_config, memory, embedding_driver);

    // Single plugin takes precedence over manual hooks
    if let Some(ref plugin_name) = toml_config.plugin {
        match load_plugin(plugin_name) {
            Ok((manifest, hooks)) => {
                if hooks.ingest.is_some() || hooks.after_turn.is_some() {
                    let env: Vec<(String, String)> = manifest.env.into_iter().collect();
                    return Box::new(
                        ScriptableContextEngine::new(default, &hooks).with_plugin_env(env),
                    );
                }
                warn!(
                    plugin = plugin_name.as_str(),
                    "Plugin loaded but defines no hooks — using default engine"
                );
                return Box::new(default);
            }
            Err(e) => {
                warn!(
                    plugin = plugin_name.as_str(),
                    error = %e,
                    "Failed to load plugin — falling back to default engine"
                );
                return Box::new(default);
            }
        }
    }

    // Manual hooks
    if toml_config.hooks.ingest.is_some() || toml_config.hooks.after_turn.is_some() {
        Box::new(ScriptableContextEngine::new(default, &toml_config.hooks))
    } else {
        Box::new(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::MemorySubstrate;
    use librefang_types::message::Message;
    use std::process::Command;
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

        let output = ScriptableContextEngine::run_hook(
            script_path.to_str().unwrap(),
            crate::plugin_runtime::PluginRuntime::Python,
            serde_json::json!({
                "type": "ingest",
                "agent_id": "agent-123",
                "message": "hello",
            }),
        )
        .await
        .unwrap();

        assert_eq!(output["type"], "ingest");
        assert_eq!(output["message"], "hello");
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
    fn test_list_installed_plugins_empty() {
        // Should not panic even if the plugins dir doesn't exist
        let plugins = list_installed_plugins();
        // May or may not be empty depending on the environment
        let _ = plugins;
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
        let engine = build_context_engine(&toml_config, runtime_config, make_memory(), None);
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
        let engine = build_context_engine(&toml_config, runtime_config, make_memory(), None);
        let _ = engine;
    }
}
