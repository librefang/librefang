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
use librefang_types::memory::{
    ExtractionResult, MemoryAction, MemoryExtractor, MemoryFragment, MemoryItem, MemoryLevel,
};
use std::sync::Arc;
use tracing::warn;

// ---------------------------------------------------------------------------
// EmbeddingDriver → EmbeddingFn bridge
// ---------------------------------------------------------------------------

/// Wraps the runtime's `EmbeddingDriver` to implement `EmbeddingFn` (from librefang-memory).
/// This avoids a circular dependency between librefang-memory and librefang-runtime.
struct EmbeddingBridge(Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>);

#[async_trait::async_trait]
impl librefang_memory::proactive::EmbeddingFn for EmbeddingBridge {
    async fn embed_one(&self, text: &str) -> librefang_types::error::LibreFangResult<Vec<f32>> {
        self.0
            .embed_one(text)
            .await
            .map_err(|e| LibreFangError::Internal(format!("Embedding failed: {e}")))
    }
}

/// Build a context string with proactive memory for prompt injection.
///
/// Includes both semantic memory matches and relevant knowledge graph relations.
pub async fn build_prompt_context_with_memory(
    memory: &ProactiveMemoryStore,
    user_id: &str,
    user_message: &str,
) -> Option<String> {
    let result: Result<Vec<librefang_memory::MemoryItem>, LibreFangError> =
        memory.auto_retrieve(user_id, user_message).await;
    match result {
        Ok(memories) if !memories.is_empty() => {
            Some(memory.format_context_with_query(&memories, user_message))
        }
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
    init_proactive_memory_full(memory, config, None, None)
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
    init_proactive_memory_full(memory, config, Some((driver, model)), None)
}

/// Initialize proactive memory with an embedding driver for vector search.
pub fn init_proactive_memory_with_embedding(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    llm: Option<(Arc<dyn crate::llm_driver::LlmDriver>, String)>,
    embedding: Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>,
) -> Option<Arc<ProactiveMemoryStore>> {
    init_proactive_memory_full(memory, config, llm, Some(embedding))
}

/// Full initialization: LLM extractor + embedding driver (both optional).
pub fn init_proactive_memory_full(
    memory: Arc<librefang_memory::MemorySubstrate>,
    config: ProactiveMemoryConfig,
    llm: Option<(Arc<dyn crate::llm_driver::LlmDriver>, String)>,
    embedding: Option<Arc<dyn crate::embedding::EmbeddingDriver + Send + Sync>>,
) -> Option<Arc<ProactiveMemoryStore>> {
    if !config.auto_retrieve && !config.auto_memorize {
        tracing::debug!("Proactive memory is disabled");
        return None;
    }

    let mut store = if let Some((driver, model)) = llm {
        let extractor = Arc::new(LlmMemoryExtractor::new(driver, model));
        ProactiveMemoryStore::with_extractor(memory, config, extractor)
    } else {
        ProactiveMemoryStore::new(memory, config)
    };

    if let Some(emb) = embedding {
        store = store.with_embedding(Arc::new(EmbeddingBridge(emb)));
        tracing::info!("Proactive memory system initialized (with embeddings)");
    } else {
        tracing::info!("Proactive memory system initialized (text search fallback)");
    }

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

const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are a memory extraction system. Your goal: help a future assistant feel like it truly knows this person — their style, preferences, expertise, and what matters to them.

Extract ONLY clearly stated or strongly demonstrated facts. Do NOT infer personality traits from single messages. Prioritize what would most change how you interact with someone.

## What to extract (in priority order)

1. **Communication style & language**: Concise vs. detailed? Formal vs. casual? Do they write in a specific language (e.g., Chinese, English)? Do they prefer code-heavy answers or conceptual explanations?
2. **Frustrations & pet peeves**: What annoys them? What mistakes should be avoided? These are the most actionable memories — they prevent you from doing things the person hates.
3. **Preferences & opinions**: Tools, languages, frameworks, themes, workflows they like or dislike. Strong opinions about how things should be done.
4. **Work style & autonomy**: Do they want you to just do it, or discuss first? Step-by-step or big-picture? Do they review diffs or trust you?
5. **Technical background**: Expertise level, technologies they work with, role, domain. What they know well vs. what they're learning.
6. **Project context**: Key projects, architectures, recurring tasks, decisions made and why.
7. **Personal details**: Name, timezone, team, anything they voluntarily shared.

## How to write memories

Write each memory as a natural observation that captures nuance — not as a flat database entry.

GOOD: "Prefers concise, direct answers — skips caveats and gets to the point"
BAD: "User prefers concise communication"

GOOD: "Gets frustrated when code suggestions don't compile — always verify before suggesting"
BAD: "User dislikes compilation errors"

GOOD: "Communicates in Chinese; switch to Chinese unless they write in English first"
BAD: "User language: Chinese"

GOOD: "Highly autonomous — wants changes made, not discussed. Just do it and explain after."
BAD: "User prefers autonomous execution"

## Response format

Respond with a JSON object containing two arrays:

1. "memories" - Facts and preferences to remember:
   - "content": the extracted memory (concise, one natural sentence with actionable nuance)
   - "category": one of: communication_style, preference, expertise, work_style, project_context, personal_detail, frustration
   - "level": "user" for personal/preference info, "session" for current task context, "agent" for agent-specific learnings

2. "relations" - Entity relationships (knowledge graph triples):
   - "subject": entity name (e.g., "Alice")
   - "subject_type": person, organization, project, concept, location, tool
   - "relation": works_at, uses, prefers, knows_about, located_in, part_of, depends_on, dislikes, experienced_with
   - "object": related entity name (e.g., "Acme Corp")
   - "object_type": same types as subject_type

Example:
{
  "memories": [
    {"content": "Experienced Rust developer who works on the LibreFang project — treat as expert, skip beginner explanations", "category": "expertise", "level": "user"},
    {"content": "Prefers concise code reviews — skip obvious comments, focus on logic and correctness issues only", "category": "work_style", "level": "user"},
    {"content": "Uses dark mode and minimal UI everywhere — avoid suggesting light themes or busy layouts", "category": "preference", "level": "user"}
  ],
  "relations": [
    {"subject": "User", "subject_type": "person", "relation": "experienced_with", "object": "Rust", "object_type": "tool"},
    {"subject": "User", "subject_type": "person", "relation": "works_at", "object": "LibreFang", "object_type": "project"}
  ]
}

If nothing worth extracting: {"memories": [], "relations": []}"#;

const DECISION_SYSTEM_PROMPT: &str = r#"You are a memory conflict resolution system. Given a NEW memory and a list of EXISTING memories, decide what action to take.

Actions:
- "ADD": The new memory is genuinely new information. No existing memory covers this.
- "UPDATE": The new memory updates/supersedes an existing memory (e.g., user changed preference, corrected a fact). Return the ID of the memory to replace.
- "NOOP": The new memory is a duplicate or already covered by an existing memory. Skip it.

Guidelines:
- If existing memory says "User prefers Python" and new says "User prefers Rust" → UPDATE (preference changed)
- If existing memory says "User's name is John" and new says "User's name is John" → NOOP (duplicate)
- If existing memory says "User works at Acme" and new says "User works at Google now" → UPDATE (fact changed)
- If no existing memory is related → ADD

Respond with a single JSON object:
{"action": "ADD"} or {"action": "UPDATE", "existing_id": "<id>"} or {"action": "NOOP"}

If nothing matches, default to ADD."#;

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
        // Skip system messages — only include user and assistant roles.
        let mut conversation_text = String::new();
        for msg in messages {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            if role == "system" {
                continue;
            }
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if !content.is_empty() {
                conversation_text.push_str(&format!("{role}: {content}\n"));
            }
        }

        if conversation_text.is_empty() {
            return Ok(ExtractionResult {
                has_content: false,
                memories: Vec::new(),
                relations: Vec::new(),
                trigger: "llm_extractor".to_string(),
                conflicts: Vec::new(),
            });
        }

        // Build the LLM request
        let request = crate::llm_driver::CompletionRequest {
            model: self.model.clone(),
            messages: vec![librefang_types::message::Message::user(format!(
                "Extract memories from this conversation:\n\n{conversation_text}"
            ))],
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

    /// LLM-powered conflict resolution: decide ADD/UPDATE/NOOP.
    ///
    /// Sends the new memory and existing candidates to the LLM for reasoning.
    /// Falls back to the default heuristic if the LLM call fails.
    async fn decide_action(
        &self,
        new_memory: &MemoryItem,
        existing_memories: &[MemoryFragment],
    ) -> librefang_types::error::LibreFangResult<MemoryAction> {
        // If no existing memories, always ADD
        if existing_memories.is_empty() {
            return Ok(MemoryAction::Add);
        }

        // Build the context for the LLM
        let mut existing_text = String::new();
        for (i, mem) in existing_memories.iter().enumerate() {
            existing_text.push_str(&format!(
                "{}. [ID: {}] \"{}\"\n",
                i + 1,
                mem.id,
                mem.content
            ));
        }

        let user_msg = format!(
            "NEW MEMORY: \"{}\"\n\nEXISTING MEMORIES:\n{}",
            new_memory.content, existing_text
        );

        let request = crate::llm_driver::CompletionRequest {
            model: self.model.clone(),
            messages: vec![librefang_types::message::Message::user(user_msg)],
            tools: Vec::new(),
            max_tokens: 256,
            temperature: 0.0,
            system: Some(DECISION_SYSTEM_PROMPT.to_string()),
            thinking: None,
            prompt_caching: false,
        };

        match self.driver.complete(request).await {
            Ok(response) => {
                let text = response.text();
                parse_decision_response(&text, existing_memories)
            }
            Err(e) => {
                tracing::warn!("LLM decision call failed, falling back to heuristic: {e}");
                // Fall back to default heuristic
                let default_extractor = librefang_types::memory::DefaultMemoryExtractor;
                default_extractor
                    .decide_action(new_memory, existing_memories)
                    .await
            }
        }
    }

    fn format_context(&self, memories: &[MemoryItem]) -> String {
        if memories.is_empty() {
            return String::new();
        }

        let mut context = String::from(
            "You have the following understanding of this person from previous conversations. \
             This is knowledge you have — not a list to recite. Let it naturally shape how you \
             respond:\n\
             \n\
             - Reference relevant context when it helps (\"since you're working in Rust...\", \
             \"keeping it concise like you prefer...\") but only when it genuinely adds value.\n\
             - Let remembered preferences silently guide your style, format, and depth — you \
             don't need to announce that you're doing so.\n\
             - NEVER say \"based on my memory\", \"according to my records\", \"I recall that you...\", \
             or mechanically list what you know. A friend doesn't preface every remark with \
             \"I remember you told me...\".\n\
             - If a memory is clearly outdated or the user contradicts it, trust the current \
             conversation over stored context.\n\n",
        );
        for mem in memories {
            context.push_str(&format!("- {}\n", mem.content));
        }
        context
    }
}

/// Strip markdown code blocks from LLM output.
///
/// Handles case-insensitive language tags (```json, ```JSON, ```Json, etc.),
/// leading text before the code block, and extracts the content between the
/// first ``` and last ```.
fn strip_code_block(text: &str) -> &str {
    let trimmed = text.trim();
    // Find first ``` and last ```, extract content between them
    if let Some(start) = trimmed.find("```") {
        let after_start = &trimmed[start + 3..];
        // Skip language tag (first line after ```)
        let content_start = after_start.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_start[content_start..];
        if let Some(end) = content.rfind("```") {
            return content[..end].trim();
        }
    }
    trimmed
}

/// Parse the LLM's decision response into a MemoryAction.
fn parse_decision_response(
    text: &str,
    existing_memories: &[MemoryFragment],
) -> librefang_types::error::LibreFangResult<MemoryAction> {
    // Strip markdown code blocks (case-insensitive, handles leading text)
    let json_str = strip_code_block(text);

    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();

    match parsed.get("action").and_then(|v| v.as_str()) {
        Some("NOOP") | Some("noop") => Ok(MemoryAction::Noop),
        Some("UPDATE") | Some("update") => {
            let existing_id = parsed
                .get("existing_id")
                .and_then(|v| v.as_str())
                .map(String::from);

            // Validate the ID exists in our candidates
            if let Some(ref id) = existing_id {
                let valid = existing_memories.iter().any(|m| m.id.to_string() == *id);
                if valid {
                    return Ok(MemoryAction::Update {
                        existing_id: id.clone(),
                    });
                }
            }

            // If ID is invalid/missing, fall back to ADD rather than blindly
            // updating the first candidate — let consolidation merge later.
            Ok(MemoryAction::Add)
        }
        _ => Ok(MemoryAction::Noop),
    }
}

/// Parse the LLM's JSON response into an ExtractionResult.
///
/// Handles two formats:
/// - New: `{"memories": [...], "relations": [...]}`
/// - Legacy: `[...]` (array of memory items, no relations)
fn parse_llm_extraction_response(
    text: &str,
) -> librefang_types::error::LibreFangResult<ExtractionResult> {
    use librefang_types::memory::RelationTriple;

    // Strip markdown code blocks (case-insensitive, handles leading text)
    let json_str = strip_code_block(text);

    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();

    // Extract memories (from object or legacy array)
    let memory_items = if let Some(arr) = parsed.get("memories").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = parsed.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    let memories: Vec<MemoryItem> = memory_items
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

    // Extract relations (knowledge graph triples)
    let relations: Vec<RelationTriple> = parsed
        .get("relations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    Some(RelationTriple {
                        subject: item.get("subject")?.as_str()?.to_string(),
                        subject_type: item
                            .get("subject_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("concept")
                            .to_string(),
                        relation: item.get("relation")?.as_str()?.to_string(),
                        object: item.get("object")?.as_str()?.to_string(),
                        object_type: item
                            .get("object_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("concept")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ExtractionResult {
        has_content: !memories.is_empty() || !relations.is_empty(),
        memories,
        relations,
        trigger: "llm_extractor".to_string(),
        conflicts: Vec::new(),
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

    #[test]
    fn test_parse_llm_extraction_new_format_with_relations() {
        let json = r#"{
            "memories": [
                {"content": "User prefers Rust", "category": "user_preference", "level": "user"}
            ],
            "relations": [
                {"subject": "User", "subject_type": "person", "relation": "prefers", "object": "Rust", "object_type": "tool"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "User prefers Rust");
        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].subject, "User");
        assert_eq!(result.relations[0].relation, "prefers");
        assert_eq!(result.relations[0].object, "Rust");
        assert_eq!(result.relations[0].object_type, "tool");
    }

    #[test]
    fn test_parse_llm_extraction_relations_only() {
        let json = r#"{
            "memories": [],
            "relations": [
                {"subject": "Alice", "subject_type": "person", "relation": "works_at", "object": "Google", "object_type": "organization"}
            ]
        }"#;
        let result = parse_llm_extraction_response(json).unwrap();
        assert!(result.has_content); // relations count as content
        assert!(result.memories.is_empty());
        assert_eq!(result.relations.len(), 1);
    }

    #[test]
    fn test_parse_decision_response_add() {
        let fragments = vec![];
        let result = parse_decision_response(r#"{"action": "ADD"}"#, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Add);
    }

    #[test]
    fn test_parse_decision_response_noop() {
        let fragments = vec![];
        let result = parse_decision_response(r#"{"action": "NOOP"}"#, &fragments).unwrap();
        assert_eq!(result, MemoryAction::Noop);
    }

    #[test]
    fn test_parse_decision_response_update() {
        use librefang_types::memory::{MemoryFragment, MemoryId, MemorySource};
        let mem_id = MemoryId::new();
        let fragments = vec![MemoryFragment {
            id: mem_id,
            agent_id: librefang_types::agent::AgentId::new(),
            content: "Old content".to_string(),
            embedding: None,
            metadata: std::collections::HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            scope: "user_memory".to_string(),
        }];
        let json = format!(r#"{{"action": "UPDATE", "existing_id": "{}"}}"#, mem_id);
        let result = parse_decision_response(&json, &fragments).unwrap();
        assert_eq!(
            result,
            MemoryAction::Update {
                existing_id: mem_id.to_string()
            }
        );
    }

    #[test]
    fn test_parse_decision_response_invalid_defaults_to_noop() {
        let fragments = vec![];
        let result = parse_decision_response("garbage", &fragments).unwrap();
        assert_eq!(result, MemoryAction::Noop);
    }
}
