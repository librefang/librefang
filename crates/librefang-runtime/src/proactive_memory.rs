//! Proactive Memory integration for the runtime.
//!
//! This module provides:
//! - `init_proactive_memory`: Create a ProactiveMemoryStore for the kernel
//! - `build_prompt_context_with_memory`: Format retrieved memories for prompt injection
//!
//! The actual auto_retrieve and auto_memorize calls happen directly in `agent_loop.rs`
//! rather than through fire-and-forget hooks, ensuring results are properly consumed.

use librefang_memory::{ProactiveMemoryConfig, ProactiveMemoryHooks, ProactiveMemoryStore};
use librefang_types::error::LibreFangError;
use librefang_types::memory::{ExtractionResult, MemoryExtractor, MemoryItem, MemoryLevel};
use std::sync::Arc;
use tracing::warn;

/// Build a context string with proactive memory for prompt injection.
pub async fn build_prompt_context_with_memory(
    memory: &ProactiveMemoryStore,
    user_id: &str,
    user_message: &str,
) -> Option<String> {
    let result: Result<Vec<librefang_memory::MemoryItem>, LibreFangError> =
        memory.auto_retrieve(user_id, user_message).await;
    match result {
        Ok(memories) if !memories.is_empty() => Some(memory.format_context(&memories)),
        Ok(_) => None,
        Err(e) => {
            warn!("Failed to retrieve proactive memories: {}", e);
            None
        }
    }
}

/// Initialize proactive memory system.
///
/// Creates a `ProactiveMemoryStore` if either auto_retrieve or auto_memorize is enabled.
/// The store is used directly by `agent_loop` — no hook registration needed since
/// the loop calls `auto_retrieve`/`auto_memorize` inline for proper result consumption.
///
/// Returns `None` if both features are disabled.
pub fn init_proactive_memory(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
) -> Option<Arc<ProactiveMemoryStore>> {
    if !config.auto_retrieve && !config.auto_memorize {
        tracing::debug!("Proactive memory is disabled");
        return None;
    }

    let store = ProactiveMemoryStore::new(memory, config);
    tracing::info!("Proactive memory system initialized");
    Some(Arc::new(store))
}

/// Initialize proactive memory with an LLM-powered extractor.
///
/// When a driver is provided, memory extraction uses the LLM for high-quality
/// results. Falls back to `init_proactive_memory` (rule-based) if no driver.
pub fn init_proactive_memory_with_llm(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    driver: Arc<dyn crate::llm_driver::LlmDriver>,
    model: String,
) -> Option<Arc<ProactiveMemoryStore>> {
    if !config.auto_retrieve && !config.auto_memorize {
        tracing::debug!("Proactive memory is disabled");
        return None;
    }

    let extractor = Arc::new(LlmMemoryExtractor::new(driver, model));
    let store = ProactiveMemoryStore::with_extractor(memory, config, extractor);
    tracing::info!("Proactive memory system initialized (LLM-powered extraction)");
    Some(Arc::new(store))
}

/// Initialize proactive memory with default configuration.
pub fn init_proactive_memory_with_defaults(
    memory: Arc<librefang_memory::MemorySubstrate>,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory(memory, ProactiveMemoryConfig::default())
}

// ---------------------------------------------------------------------------
// LLM-powered Memory Extractor
// ---------------------------------------------------------------------------

const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a memory extraction system. Analyze the conversation and extract important information that should be remembered for future interactions.

Extract ONLY clearly stated facts, preferences, and important context. Do NOT infer or assume.

Categories:
- user_preference: Explicit preferences (e.g., "I prefer dark mode", "I always use Python")
- important_fact: Personal facts (e.g., "My name is John", "I work at Acme Corp")
- task_context: Project or task context (e.g., "We're migrating to PostgreSQL")

Respond with a JSON array of objects, each with:
- "content": the extracted memory (concise, one sentence)
- "category": one of the categories above
- "level": "user" for personal info, "session" for task context, "agent" for agent-specific

Example response:
[{"content": "User prefers Rust over Go", "category": "user_preference", "level": "user"}]

If nothing worth remembering, respond with: []"#;

/// LLM-powered memory extractor that uses a language model to identify
/// important information from conversations.
pub struct LlmMemoryExtractor {
    driver: Arc<dyn crate::llm_driver::LlmDriver>,
    model: String,
}

impl LlmMemoryExtractor {
    pub fn new(driver: Arc<dyn crate::llm_driver::LlmDriver>, model: String) -> Self {
        Self { driver, model }
    }
}

#[async_trait::async_trait]
impl MemoryExtractor for LlmMemoryExtractor {
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
    ) -> librefang_types::error::LibreFangResult<ExtractionResult> {
        // Build a condensed version of the conversation for the LLM
        let mut conversation_text = String::new();
        for msg in messages {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if !content.is_empty() {
                conversation_text.push_str(&format!("{role}: {content}\n"));
            }
        }

        if conversation_text.is_empty() {
            return Ok(ExtractionResult {
                has_content: false,
                memories: Vec::new(),
                trigger: "llm_extractor".to_string(),
            });
        }

        // Build the LLM request
        let request = crate::llm_driver::CompletionRequest {
            model: self.model.clone(),
            messages: vec![
                librefang_types::message::Message::system(EXTRACTION_SYSTEM_PROMPT),
                librefang_types::message::Message::user(format!(
                    "Extract memories from this conversation:\n\n{conversation_text}"
                )),
            ],
            tools: Vec::new(),
            max_tokens: 1024,
            temperature: 0.1,
            system: Some(EXTRACTION_SYSTEM_PROMPT.to_string()),
            thinking: None,
            prompt_caching: false,
        };

        let response = self
            .driver
            .complete(request)
            .await
            .map_err(|e| LibreFangError::Internal(format!("LLM extraction failed: {e}")))?;

        let text = response.text();
        parse_llm_extraction_response(&text)
    }

    fn format_context(&self, memories: &[MemoryItem]) -> String {
        if memories.is_empty() {
            return String::new();
        }

        let mut context = String::from("## Relevant Memories\n\n");
        for mem in memories {
            let level_str = match mem.level {
                MemoryLevel::User => "[User]",
                MemoryLevel::Session => "[Session]",
                MemoryLevel::Agent => "[Agent]",
            };
            context.push_str(&format!("- {} {}\n", level_str, mem.content));
        }
        context
    }
}

/// Parse the LLM's JSON response into an ExtractionResult.
fn parse_llm_extraction_response(
    text: &str,
) -> librefang_types::error::LibreFangResult<ExtractionResult> {
    // Try to extract JSON array from the response (may be wrapped in markdown code blocks)
    let json_str = text
        .trim()
        .strip_prefix("```json")
        .or_else(|| text.trim().strip_prefix("```"))
        .unwrap_or(text.trim());
    let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

    let items: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap_or_default();

    let memories: Vec<MemoryItem> = items
        .into_iter()
        .filter_map(|item| {
            let content = item.get("content")?.as_str()?.to_string();
            let category = item
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_string();
            let level = match item.get("level").and_then(|v| v.as_str()) {
                Some("user") => MemoryLevel::User,
                Some("agent") => MemoryLevel::Agent,
                _ => MemoryLevel::Session,
            };

            let mut metadata = std::collections::HashMap::new();
            metadata.insert("extracted_by".to_string(), serde_json::json!("llm"));

            Some(MemoryItem {
                id: uuid::Uuid::new_v4().to_string(),
                content,
                level,
                category: Some(category),
                metadata,
                created_at: chrono::Utc::now(),
            })
        })
        .collect();

    Ok(ExtractionResult {
        has_content: !memories.is_empty(),
        memories,
        trigger: "llm_extractor".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_when_both_off() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let config = ProactiveMemoryConfig {
            auto_memorize: false,
            auto_retrieve: false,
            ..Default::default()
        };
        assert!(init_proactive_memory(Arc::new(substrate), config).is_none());
    }

    #[test]
    fn test_enabled_by_default() {
        let substrate = librefang_memory::MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = init_proactive_memory_with_defaults(Arc::new(substrate));
        assert!(store.is_some());
    }

    #[test]
    fn test_parse_llm_extraction_json() {
        let json =
            r#"[{"content": "User prefers Rust", "category": "user_preference", "level": "user"}]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "User prefers Rust");
        assert_eq!(
            result.memories[0].category,
            Some("user_preference".to_string())
        );
        assert_eq!(result.memories[0].level, MemoryLevel::User);
    }

    #[test]
    fn test_parse_llm_extraction_code_block() {
        let json = "```json\n[{\"content\": \"Works at Acme\", \"category\": \"important_fact\", \"level\": \"user\"}]\n```";
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "Works at Acme");
    }

    #[test]
    fn test_parse_llm_extraction_empty() {
        let result = parse_llm_extraction_response("[]").unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }

    #[test]
    fn test_parse_llm_extraction_invalid() {
        let result = parse_llm_extraction_response("not json at all").unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }

    #[test]
    fn test_parse_llm_extraction_levels() {
        let json = r#"[
            {"content": "a", "level": "user"},
            {"content": "b", "level": "session"},
            {"content": "c", "level": "agent"},
            {"content": "d"}
        ]"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert_eq!(result.memories.len(), 4);
        assert_eq!(result.memories[0].level, MemoryLevel::User);
        assert_eq!(result.memories[1].level, MemoryLevel::Session);
        assert_eq!(result.memories[2].level, MemoryLevel::Agent);
        assert_eq!(result.memories[3].level, MemoryLevel::Session); // default
    }
}
