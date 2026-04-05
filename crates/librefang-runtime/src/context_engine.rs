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
// Scriptable context engine — wraps DefaultContextEngine + Python script hooks
// ---------------------------------------------------------------------------

/// Context engine that delegates to a [`DefaultContextEngine`] for heavy
/// operations (assemble, compact) and optionally invokes Python scripts
/// for light lifecycle hooks (ingest, after_turn).
///
/// This allows users to customize context management without recompiling:
///
/// ```toml
/// [context_engine.hooks]
/// ingest = "~/.librefang/plugins/my_recall.py"
/// after_turn = "~/.librefang/plugins/my_indexer.py"
/// ```
///
/// Scripts use the same JSON stdin/stdout protocol as Python agents:
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
}

impl ScriptableContextEngine {
    /// Create a scriptable context engine from config.
    pub fn new(
        inner: DefaultContextEngine,
        hooks: &librefang_types::config::ContextEngineHooks,
    ) -> Self {
        Self {
            inner,
            ingest_script: hooks.ingest.clone(),
            after_turn_script: hooks.after_turn.clone(),
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

    /// Run a Python hook script with JSON input, return parsed JSON output.
    async fn run_hook(
        script_path: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let resolved = Self::resolve_script_path(script_path);

        // Validate path safety
        crate::python_runtime::validate_script_path(&resolved)
            .map_err(|e| format!("Hook script validation failed: {e}"))?;

        if !std::path::Path::new(&resolved).exists() {
            return Err(format!("Hook script not found: {resolved}"));
        }

        let config = crate::python_runtime::PythonConfig {
            timeout_secs: 30, // hooks should be fast
            ..Default::default()
        };

        let result = crate::python_runtime::run_python_json(&resolved, &input, &config)
            .await
            .map_err(|e| format!("Hook script failed: {e}"))?;

        // Parse response as JSON; wrap plain-text output gracefully
        Ok(serde_json::from_str(&result.response)
            .unwrap_or_else(|_| serde_json::json!({"text": result.response})))
    }
}

#[async_trait]
impl ContextEngine for ScriptableContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        // Validate hook scripts exist at startup
        if let Some(ref path) = self.ingest_script {
            let resolved = Self::resolve_script_path(path);
            if !std::path::Path::new(&resolved).exists() {
                warn!("Ingest hook script not found: {resolved}");
            } else {
                debug!("Ingest hook configured: {resolved}");
            }
        }
        if let Some(ref path) = self.after_turn_script {
            let resolved = Self::resolve_script_path(path);
            if !std::path::Path::new(&resolved).exists() {
                warn!("After-turn hook script not found: {resolved}");
            } else {
                debug!("After-turn hook configured: {resolved}");
            }
        }
        self.inner.bootstrap(config).await
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

        // Run default recall first (for embedding-based memories)
        let default_result = self.inner.ingest(agent_id, user_message, peer_id).await?;

        // Run the Python hook for additional/custom recall
        let input = serde_json::json!({
            "type": "ingest",
            "agent_id": agent_id.0.to_string(),
            "message": user_message,
            "peer_id": peer_id,
        });

        match Self::run_hook(script, input).await {
            Ok(output) => {
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
                warn!("Ingest hook failed, using default result: {e}");
                Ok(default_result)
            }
        }
    }

    async fn assemble(
        &self,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
        context_window_tokens: usize,
    ) -> LibreFangResult<AssembleResult> {
        // Always delegate to Rust — too performance-critical for Python
        self.inner
            .assemble(messages, system_prompt, tools, context_window_tokens)
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
        // Always delegate to Rust — requires LLM driver access
        self.inner
            .compact(agent_id, messages, driver, model, context_window_tokens)
            .await
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        // Run default after_turn first
        self.inner.after_turn(agent_id, messages).await?;

        // If no after_turn script, we're done
        let Some(ref script) = self.after_turn_script else {
            return Ok(());
        };

        // Build a compact representation of messages (not full content, to keep it fast)
        let msg_summaries: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": serde_json::to_value(m.role).unwrap_or_default(),
                    "content": m.content.text_content().chars().take(500).collect::<String>(),
                })
            })
            .collect();

        let input = serde_json::json!({
            "type": "after_turn",
            "agent_id": agent_id.0.to_string(),
            "messages": msg_summaries,
        });

        // Spawn as fire-and-forget — after_turn is best-effort, don't block the agent.
        // Log if the task panics so failures aren't silently swallowed.
        let script = script.clone();
        let handle = tokio::spawn(async move {
            match Self::run_hook(&script, input).await {
                Ok(_) => debug!("After-turn hook completed"),
                Err(e) => warn!("After-turn hook failed: {e}"),
            }
        });
        tokio::spawn(async move {
            if let Err(e) = handle.await {
                warn!("After-turn hook task panicked: {e}");
            }
        });

        Ok(())
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        self.inner
            .truncate_tool_result(content, context_window_tokens)
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
    };

    debug!(
        plugin = plugin_name,
        dir = %plugin_dir.display(),
        ingest = ?resolved_hooks.ingest,
        after_turn = ?resolved_hooks.after_turn,
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

/// Build a context engine from config.
///
/// Resolution order:
/// 1. If `plugin` is set, load plugin manifest and use its hooks
/// 2. If manual `hooks` are set, use them directly
/// 3. Otherwise, return a plain `DefaultContextEngine`
pub fn build_context_engine(
    toml_config: &librefang_types::config::ContextEngineTomlConfig,
    runtime_config: ContextEngineConfig,
    memory: Arc<MemorySubstrate>,
    embedding_driver: Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
) -> Box<dyn ContextEngine> {
    let default = DefaultContextEngine::new(runtime_config, memory, embedding_driver);

    // Warn if an unknown engine name is configured
    if toml_config.engine != "default" {
        warn!(
            engine = toml_config.engine.as_str(),
            "Unknown context engine '{}' — only 'default' is built-in, falling back",
            toml_config.engine
        );
    }

    // Plugin takes precedence
    if let Some(ref plugin_name) = toml_config.plugin {
        match load_plugin(plugin_name) {
            Ok((_manifest, hooks)) => {
                if hooks.ingest.is_some() || hooks.after_turn.is_some() {
                    return Box::new(ScriptableContextEngine::new(default, &hooks));
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
            .assemble(&mut messages, "system", &[], 200_000)
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
            .assemble(&mut messages, "system", &[], 100)
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
