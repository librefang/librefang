//! Pluggable context engine — lifecycle hooks for context management.
//!
//! Inspired by OpenClaw's ContextEngine, this trait lets developers plug in
//! their own strategies for memory recall, message assembly, compaction, and
//! post-turn bookkeeping without modifying the core agent loop.
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
}

impl Default for ContextEngineConfig {
    fn default() -> Self {
        Self {
            context_window_tokens: 200_000,
            stable_prefix_mode: false,
            max_recall_results: 5,
        }
    }
}

/// Result from the `assemble` lifecycle hook.
#[derive(Debug)]
pub struct AssembleResult {
    /// Messages to send to the LLM, ordered and within budget.
    pub messages: Vec<Message>,
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
    async fn ingest(&self, agent_id: AgentId, user_message: &str) -> LibreFangResult<IngestResult>;

    /// Called before each LLM call to assemble the context window.
    ///
    /// Given the current messages and available tools, return an ordered
    /// set of messages that fits within the token budget. This is the
    /// core hook — it controls exactly what the model "sees".
    ///
    /// The default implementation applies overflow recovery, context
    /// guard compaction, and session repair.
    async fn assemble(
        &self,
        messages: &mut Vec<Message>,
        system_prompt: &str,
        tools: &[ToolDefinition],
    ) -> LibreFangResult<AssembleResult>;

    /// Called when the context window is under pressure.
    ///
    /// Summarize older history to free space. The default implementation
    /// uses LLM-based compaction with 3 strategies (single-pass, chunked,
    /// fallback).
    async fn compact(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
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
    /// Default implementation uses head+tail truncation strategy.
    fn truncate_tool_result(&self, content: &str) -> String;
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
    budget: ContextBudget,
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
        let budget = ContextBudget::new(config.context_window_tokens);
        let compaction_config = CompactionConfig {
            context_window_tokens: config.context_window_tokens,
            ..CompactionConfig::default()
        };
        Self {
            config,
            budget,
            memory,
            embedding_driver,
            compaction_config,
        }
    }

    /// Get the context budget.
    pub fn budget(&self) -> &ContextBudget {
        &self.budget
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

    async fn ingest(&self, agent_id: AgentId, user_message: &str) -> LibreFangResult<IngestResult> {
        // In stable_prefix_mode, skip memory recall to keep system prompt stable for caching.
        if self.config.stable_prefix_mode {
            return Ok(IngestResult {
                recalled_memories: Vec::new(),
            });
        }

        let filter = Some(MemoryFilter {
            agent_id: Some(agent_id),
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
                        .unwrap_or_default()
                }
                Err(e) => {
                    warn!("ContextEngine: embedding recall failed, falling back to text: {e}");
                    self.memory
                        .recall(user_message, limit, filter)
                        .await
                        .unwrap_or_default()
                }
            }
        } else {
            self.memory
                .recall(user_message, limit, filter)
                .await
                .unwrap_or_default()
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
    ) -> LibreFangResult<AssembleResult> {
        // Stage 1: Overflow recovery pipeline (4-stage cascade, respects pinned messages)
        let recovery = recover_from_overflow(
            messages,
            system_prompt,
            tools,
            self.config.context_window_tokens,
        );

        if recovery == RecoveryStage::FinalError {
            warn!("ContextEngine: overflow unrecoverable — suggest /reset or /compact");
        }

        // Re-validate tool_call/tool_result pairing after overflow drains
        if recovery != RecoveryStage::None {
            *messages = crate::session_repair::validate_and_repair(messages);
        }

        // Stage 2: Context guard — compact oversized tool results
        apply_context_guard(messages, &self.budget, tools);

        Ok(AssembleResult {
            messages: messages.clone(),
            recovery,
        })
    }

    async fn compact(
        &self,
        _agent_id: AgentId,
        messages: &[Message],
        driver: Arc<dyn LlmDriver>,
    ) -> LibreFangResult<CompactionResult> {
        // Build a temporary session for the compactor
        let session = librefang_memory::session::Session {
            id: librefang_types::agent::SessionId::new(),
            agent_id: _agent_id,
            messages: messages.to_vec(),
            context_window_tokens: 0,
            label: None,
        };

        compactor::compact_session(driver, "default", &session, &self.compaction_config)
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

    fn truncate_tool_result(&self, content: &str) -> String {
        crate::context_budget::truncate_tool_result_dynamic(content, &self.budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_memory::MemorySubstrate;
    use librefang_types::message::Message;

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
        let result = engine.ingest(AgentId::new(), "hello").await.unwrap();
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
        let result = engine.ingest(agent_id, "Rust").await.unwrap();
        assert_eq!(result.recalled_memories.len(), 1);
        assert!(result.recalled_memories[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_assemble_no_overflow() {
        let config = ContextEngineConfig::default();
        let engine = DefaultContextEngine::new(config, make_memory(), None);
        let mut messages = vec![Message::user("hi"), Message::assistant("hello")];
        let result = engine.assemble(&mut messages, "system", &[]).await.unwrap();
        assert_eq!(result.recovery, RecoveryStage::None);
        assert_eq!(result.messages.len(), 2);
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
                    Message::user(&format!("msg{}: {}", i, "x".repeat(200)))
                } else {
                    Message::assistant(&format!("msg{}: {}", i, "x".repeat(200)))
                }
            })
            .collect();

        let result = engine.assemble(&mut messages, "system", &[]).await.unwrap();
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
        let truncated = engine.truncate_tool_result(&big_content);
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
}
