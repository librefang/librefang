//! Proactive Memory integration for the runtime.
//!
//! This module provides integration between the runtime's hook system and
//! the proactive memory system, enabling:
//! - `auto_retrieve`: Fetch relevant memories before agent execution
//! - `auto_memorize`: Extract and store important info after agent execution

use crate::hooks::{HookContext, HookHandler};
use librefang_memory::{
    ExtractionResult, ProactiveMemoryConfig, ProactiveMemoryHooks, ProactiveMemoryStore,
};
use librefang_types::agent::HookEvent;
use librefang_types::error::LibreFangError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Hook handler that provides proactive memory functionality.
///
/// This handler integrates with the runtime's hook system to:
/// - BeforePromptBuild: Auto-retrieve relevant memories and inject into prompt
/// - AgentLoopEnd: Auto-memorize conversation for future reference
#[derive(Clone)]
pub struct ProactiveMemoryHookHandler {
    /// The proactive memory store.
    memory: Arc<ProactiveMemoryStore>,
    /// Session messages for auto_memorize (populated during loop).
    session_messages: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl ProactiveMemoryHookHandler {
    /// Create a new proactive memory hook handler.
    pub fn new(memory: Arc<ProactiveMemoryStore>) -> Self {
        Self {
            memory,
            session_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get the session messages for auto_memorize.
    pub fn session_messages(&self) -> Arc<Mutex<Vec<serde_json::Value>>> {
        Arc::clone(&self.session_messages)
    }

    /// Capture a message for potential auto_memorize.
    pub async fn capture_message(&self, role: &str, content: &str) {
        let msg = serde_json::json!({
            "role": role,
            "content": content
        });
        self.session_messages.lock().await.push(msg);
    }

    /// Clear captured session messages.
    pub async fn clear_messages(&self) {
        self.session_messages.lock().await.clear();
    }

    /// Get the proactive memory store.
    pub fn memory(&self) -> &ProactiveMemoryStore {
        &self.memory
    }
}

impl HookHandler for ProactiveMemoryHookHandler {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
        let event = ctx.event;

        match event {
            HookEvent::BeforePromptBuild => {
                // Extract user message from hook context
                let user_message = ctx.data
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Clone data for async block
                let memory: Arc<ProactiveMemoryStore> = Arc::clone(&self.memory);
                let user_msg = user_message.to_string();
                let user_id = ctx.agent_id.to_string();

                // Run async auto_retrieve
                tokio::spawn(async move {
                    let result: Result<Vec<librefang_memory::MemoryItem>, LibreFangError> = memory.auto_retrieve(&user_id, &user_msg).await;
                    match result {
                        Ok(memories) if !memories.is_empty() => {
                            // Format context and log (could be injected into prompt)
                            let context = memory.format_context(&memories);
                            debug!(
                                "Proactive memory retrieved {} memories: {}",
                                memories.len(),
                                context.len()
                            );
                        }
                        Ok(_) => {
                            debug!("No proactive memories retrieved");
                        }
                        Err(e) => {
                            warn!("Auto-retrieve failed: {}", e);
                        }
                    }
                });
            }
            HookEvent::AgentLoopEnd => {
                // Extract messages from hook context data (passed from agent_loop)
                let messages_from_hook: Vec<serde_json::Value> = ctx.data
                    .get("messages")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let memory: Arc<ProactiveMemoryStore> = Arc::clone(&self.memory);
                let user_id = ctx.agent_id.to_string();

                tokio::spawn(async move {
                    // Use messages from hook data (passed from agent_loop)
                    if !messages_from_hook.is_empty() {
                        let result: Result<ExtractionResult, LibreFangError> = memory.auto_memorize(&user_id, &messages_from_hook).await;
                        match result {
                            Ok(extraction_result) => {
                                debug!(
                                    "Auto-memorize completed: {} memories extracted, has_content={}",
                                    extraction_result.memories.len(),
                                    extraction_result.has_content
                                );
                            }
                            Err(e) => {
                                warn!("Auto-memorize failed: {}", e);
                            }
                        }
                    } else {
                        debug!("No messages provided for auto_memorize");
                    }
                });
            }
            _ => {
                // Ignore other events
            }
        }

        Ok(())
    }
}

/// Build a PromptContext with proactive memory injected.
///
/// This function is called during prompt building to inject
/// relevant memories into the system prompt.
pub async fn build_prompt_context_with_memory(
    memory: &ProactiveMemoryStore,
    user_id: &str,
    user_message: &str,
) -> Option<String> {
    let result: Result<Vec<librefang_memory::MemoryItem>, LibreFangError> = memory.auto_retrieve(user_id, user_message).await;
    match result {
        Ok(memories) if !memories.is_empty() => {
            Some(memory.format_context(&memories))
        }
        Ok(_) => None,
        Err(e) => {
            warn!("Failed to retrieve proactive memories: {}", e);
            None
        }
    }
}

/// Create a default proactive memory configuration.
pub fn default_proactive_memory_config() -> ProactiveMemoryConfig {
    ProactiveMemoryConfig::default()
}

/// Initialize proactive memory system and register hooks.
///
/// This should be called once during kernel/r runtime initialization.
/// It creates a ProactiveMemoryStore and registers the hook handler
/// for auto_retrieve (BeforePromptBuild) and auto_memorize (AgentLoopEnd).
///
/// Returns the ProactiveMemoryStore for direct access from kernel/agent_loop.
/// Returns None if both auto_retrieve and auto_memorize are disabled.
///
/// # Arguments
/// * `hooks` - The hook registry to register handlers with
/// * `memory` - The memory substrate to use for storage
/// * `config` - Configuration for proactive memory behavior
///
/// # Example
///
/// ```ignore
/// use librefang_runtime::hooks::{HookRegistry, HookEvent};
/// use librefang_memory::MemorySubstrate;
/// use librefang_runtime::proactive_memory::init_proactive_memory;
///
/// let hooks = HookRegistry::new();
/// let memory = MemorySubstrate::open("memory.db")?;
/// init_proactive_memory(&hooks, Arc::new(memory), ProactiveMemoryConfig::default());
/// ```
pub fn init_proactive_memory(
    hooks: &crate::hooks::HookRegistry,
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
) -> Option<Arc<ProactiveMemoryStore>> {
    if !config.auto_retrieve && !config.auto_memorize {
        tracing::debug!("Proactive memory is disabled");
        return None;
    }

    let store = ProactiveMemoryStore::new(memory, config.clone());
    let store_arc = Arc::new(store.clone());
    let handler = ProactiveMemoryHookHandler::new(Arc::clone(&store_arc));

    // Register for BeforePromptBuild (auto_retrieve)
    if config.auto_retrieve {
        hooks.register(
            librefang_types::agent::HookEvent::BeforePromptBuild,
            Arc::new(handler.clone()),
        );
        tracing::info!("Proactive memory auto_retrieve enabled");
    }

    // Register for AgentLoopEnd (auto_memorize)
    if config.auto_memorize {
        hooks.register(
            librefang_types::agent::HookEvent::AgentLoopEnd,
            Arc::new(handler),
        );
        tracing::info!("Proactive memory auto_memorize enabled");
    }

    // Return the store - kernel will wrap in Arc
    Some(store_arc)
}

/// Initialize proactive memory with default configuration.
///
/// Convenience function that uses ProactiveMemoryConfig::default().
/// Returns the ProactiveMemoryStore for direct access from kernel.
pub fn init_proactive_memory_with_defaults(
    hooks: &crate::hooks::HookRegistry,
    memory: Arc<librefang_memory::MemorySubstrate>,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory(hooks, memory, ProactiveMemoryConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = default_proactive_memory_config();
        assert!(config.auto_memorize);
        assert!(config.auto_retrieve);
        assert_eq!(config.max_retrieve, 10);
    }
}
