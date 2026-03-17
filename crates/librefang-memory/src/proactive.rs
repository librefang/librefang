//! Proactive Memory System - mem0-style API with auto-memorize and auto-retrieve.
//!
//! This module provides:
//! - Unified memory API (mem0-style): search(), add(), get(), list()
//! - Proactive hooks: auto_memorize(), auto_retrieve()
//! - Multi-level memory: User, Session, Agent
//!
//! # Architecture
//!
//! ```text
//! +-------------------+
//! |  ProactiveMemory  |  <-- External API (mem0-style)
//! +-------------------+
//!         |
//! +-------------------+
//! | ProactiveMemoryStore |  <-- Implementation
//! +-------------------+
//!         |
//! +-------------------+
//! |  MemorySubstrate  |  <-- Existing storage
//! +-------------------+
//! ```

use crate::semantic::SemanticStore;
use crate::structured::StructuredStore;
use crate::MemorySubstrate;

use async_trait::async_trait;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    DefaultMemoryExtractor, ExtractionResult, MemoryExtractor, MemoryFilter, MemoryItem,
    MemoryLevel, MemorySource, ProactiveMemory, ProactiveMemoryConfig, ProactiveMemoryHooks,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Scope names for multi-level memory.
pub mod scopes {
    pub const USER: &str = "user_memory";
    pub const SESSION: &str = "session_memory";
    pub const AGENT: &str = "agent_memory";
}

/// Category names for memory classification.
pub mod categories {
    pub const USER_PREFERENCE: &str = "user_preference";
    pub const IMPORTANT_FACT: &str = "important_fact";
    pub const TASK_CONTEXT: &str = "task_context";
    pub const RELATIONSHIP: &str = "relationship";
}

/// Proactive memory store - implements mem0-style API on top of MemorySubstrate.
///
/// This wraps the existing MemorySubstrate with a simpler, user-friendly API:
/// - search(): Semantic search across all memory levels
/// - add(): Store with automatic extraction
/// - get(): Retrieve user-level memories
/// - list(): List memories by category
///
/// # Example
///
/// ```ignore
/// use librefang_memory::{ProactiveMemoryStore, ProactiveMemory, ProactiveMemoryHooks, MemorySubstrate};
/// use std::sync::Arc;
///
/// // Create memory substrate
/// let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
/// let substrate = Arc::new(substrate);
///
/// // Create proactive memory store
/// let store = ProactiveMemoryStore::with_default_config(substrate);
/// let store = Arc::new(store);
///
/// // Use mem0-style API
/// let user_id = "user123";
///
/// // Add memories
/// store.add(&[serde_json::json!({
///     "role": "user",
///     "content": "I prefer dark mode and use Python daily"
/// })], user_id).await.unwrap();
///
/// // Search memories
/// let results = store.search("preferences", user_id, 10).await.unwrap();
///
/// // Auto-retrieve before agent execution
/// let context = store.auto_retrieve("What did I tell you about my preferences?").await.unwrap();
/// ```
#[derive(Clone)]
pub struct ProactiveMemoryStore {
    #[allow(dead_code)]
    substrate: Arc<MemorySubstrate>,
    structured: StructuredStore,
    semantic: SemanticStore,
    config: ProactiveMemoryConfig,
    /// Memory extractor for LLM-powered extraction
    extractor: Arc<dyn MemoryExtractor>,
}

impl ProactiveMemoryStore {
    /// Create a new proactive memory store with default extractor.
    pub fn new(substrate: Arc<MemorySubstrate>, config: ProactiveMemoryConfig) -> Self {
        let conn = substrate.usage_conn();
        Self {
            structured: StructuredStore::new(Arc::clone(&conn)),
            semantic: SemanticStore::new(conn),
            substrate,
            config,
            extractor: Arc::new(DefaultMemoryExtractor),
        }
    }

    /// Create a new proactive memory store with custom extractor.
    pub fn with_extractor(
        substrate: Arc<MemorySubstrate>,
        config: ProactiveMemoryConfig,
        extractor: Arc<dyn MemoryExtractor>,
    ) -> Self {
        let conn = substrate.usage_conn();
        Self {
            structured: StructuredStore::new(Arc::clone(&conn)),
            semantic: SemanticStore::new(conn),
            substrate,
            config,
            extractor,
        }
    }

    /// Create with default configuration.
    pub fn with_default_config(substrate: Arc<MemorySubstrate>) -> Self {
        Self::new(substrate, ProactiveMemoryConfig::default())
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &ProactiveMemoryConfig {
        &self.config
    }

    /// Update the config.
    pub fn update_config(&self, config: ProactiveMemoryConfig) {
        // Note: this would need interior mutability in production
        // For now, config is set at creation time
        let _ = config;
    }

    /// Parse user_id string into AgentId.
    fn parse_agent_id(user_id: &str) -> LibreFangResult<AgentId> {
        user_id.parse().map_err(|e| {
            LibreFangError::Internal(format!("Failed to parse user_id: {}", e))
        })
    }

    /// Retrieve memory items from storage.
    fn retrieve_memory_items(
        &self,
        agent_id: AgentId,
        level: Option<MemoryLevel>,
        category: Option<&str>,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        // Get all keys that match "memory:*"
        let kv_pairs = self.structured.list_kv(agent_id)?;
        let mut items = Vec::new();

        for (key, value) in kv_pairs {
            if !key.starts_with("memory:") {
                continue;
            }

            if let Ok(mem) = serde_json::from_value::<serde_json::Value>(value) {
                let level_str = mem
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Session");
                let cat = mem.get("category").and_then(|v| v.as_str());

                // Filter by level
                if let Some(target_level) = level {
                    let current_level = MemoryLevel::from(level_str);
                    if current_level != target_level {
                        continue;
                    }
                }

                // Filter by category
                if let Some(target_cat) = category {
                    if cat != Some(target_cat) {
                        continue;
                    }
                }

                let content = mem
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let created_at_str = mem.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                let created_at = chrono::DateTime::parse_from_rfc3339(created_at_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());

                let metadata = mem
                    .get("metadata")
                    .and_then(|v| {
                        serde_json::from_value::<HashMap<String, serde_json::Value>>(v.clone()).ok()
                    })
                    .unwrap_or_default();

                let mut item = MemoryItem::new(content, MemoryLevel::from(level_str));
                item.id = key.strip_prefix("memory:").unwrap_or(&key).to_string();
                if let Some(c) = cat {
                    item = item.with_category(c);
                }
                item.metadata = metadata;
                item.created_at = created_at;

                items.push(item);
            }
        }

        // Sort by created_at descending
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(items)
    }

    /// Build extraction prompt for LLM.
    #[allow(dead_code)]
    fn build_extraction_prompt(&self, messages: &[serde_json::Value]) -> String {
        let messages_text: Vec<String> = messages
            .iter()
            .filter_map(|m| {
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                if content.is_empty() {
                    None
                } else {
                    Some(format!("{}: {}", role, content))
                }
            })
            .collect();

        format!(
            r#"Extract important information from this conversation that should be remembered.

Categories to extract:
- user_preference: User's stated preferences, likes, dislikes
- important_fact: Factual information the user shared
- task_context: Context about ongoing tasks or projects
- relationship: Information about relationships or interactions

Conversation:
{}

Extract each piece of important information as a JSON array with format:
{{"content": "...", "level": "user|session|agent", "category": "..."}}

Only extract information worth remembering (confidence > 0.7).
Return an empty array if nothing important was discussed."#,
            messages_text.join("\n")
        )
    }

    /// Parse LLM extraction response.
    #[allow(dead_code)]
    fn parse_extraction_response(&self, response: &str) -> Vec<MemoryItem> {
        // Try to parse as JSON array
        if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(response) {
            items
                .into_iter()
                .filter_map(|item| {
                    let content = item.get("content")?.as_str()?.to_string();
                    let level_str = item
                        .get("level")
                        .and_then(|v| v.as_str())
                        .unwrap_or("session");
                    let category = item
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let level = MemoryLevel::from(level_str);

                    let mut memory_item = MemoryItem::new(content, level);
                    if let Some(cat) = category {
                        memory_item = memory_item.with_category(cat);
                    }

                    Some(memory_item)
                })
                .collect()
        } else {
            Vec::new()
        }
    }
}

#[async_trait]
impl ProactiveMemory for ProactiveMemoryStore {
    /// Semantic search for relevant memories.
    async fn search(
        &self,
        query: &str,
        user_id: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        let _agent_id = Self::parse_agent_id(user_id)?;

        // Use the semantic store directly
        let results = self.semantic.recall(query, limit, None)?;

        Ok(results
            .into_iter()
            .map(|frag| {
                let level = MemoryLevel::from(frag.scope.as_str());
                MemoryItem {
                    id: frag.id.to_string(),
                    content: frag.content,
                    level,
                    category: frag
                        .metadata
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    metadata: frag.metadata,
                    created_at: frag.created_at,
                }
            })
            .take(limit)
            .collect())
    }

    /// Add memories with automatic extraction.
    ///
    /// Note: This is a simplified implementation. In production, this would:
    /// 1. Send messages to LLM for extraction
    /// 2. Parse extraction results
    /// 3. Store extracted memories
    async fn add(&self, messages: &[serde_json::Value], user_id: &str) -> LibreFangResult<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // For now, store the raw messages as session memory
        // In production, we'd use LLM extraction here
        let content = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        if content.is_empty() {
            return Ok(());
        }

        // Store as session memory using semantic store directly
        self.semantic.remember(
            agent_id,
            &content,
            MemorySource::Conversation,
            scopes::SESSION,
            HashMap::new(),
        )?;

        Ok(())
    }

    /// Add memories at a specific memory level.
    ///
    /// This allows storing memories at User (persistent), Session (current conversation),
    /// or Agent (agent-specific) levels.
    async fn add_with_level(
        &self,
        messages: &[serde_json::Value],
        user_id: &str,
        level: MemoryLevel,
    ) -> LibreFangResult<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Extract content from messages
        let content = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        if content.is_empty() {
            return Ok(());
        }

        // Map MemoryLevel to scope string
        let scope = match level {
            MemoryLevel::User => scopes::USER,
            MemoryLevel::Session => scopes::SESSION,
            MemoryLevel::Agent => scopes::AGENT,
        };

        // Store with the specified scope
        self.semantic.remember(
            agent_id,
            &content,
            MemorySource::Conversation,
            scope,
            HashMap::new(),
        )?;

        Ok(())
    }

    /// Get user-level memories (preferences).
    async fn get(&self, user_id: &str) -> LibreFangResult<Vec<MemoryItem>> {
        let agent_id = Self::parse_agent_id(user_id)?;
        self.retrieve_memory_items(agent_id, Some(MemoryLevel::User), None)
    }

    /// List memories by category.
    async fn list(&self, user_id: &str, category: Option<&str>) -> LibreFangResult<Vec<MemoryItem>> {
        let agent_id = Self::parse_agent_id(user_id)?;
        self.retrieve_memory_items(agent_id, None, category)
    }
}

/// Additional methods for ProactiveMemoryStore
impl ProactiveMemoryStore {
    /// Format retrieved memories into a context string for prompt injection.
    ///
    /// This is useful for auto_retrieve - after getting memories, you can
    /// format them into a string to inject into the agent's system prompt.
    pub fn format_context(&self, memories: &[MemoryItem]) -> String {
        self.extractor.format_context(memories)
    }
}

#[async_trait]
impl ProactiveMemoryHooks for ProactiveMemoryStore {
    /// Extract and store important information after agent execution.
    ///
    /// This is a simplified implementation that stores conversation content.
    /// In production, this would use LLM extraction to identify important
    /// information worth remembering.
    async fn auto_memorize(
        &self,
        user_id: &str,
        conversation: &[serde_json::Value],
    ) -> LibreFangResult<ExtractionResult> {
        if !self.config.auto_memorize || conversation.is_empty() {
            return Ok(ExtractionResult {
                memories: Vec::new(),
                has_content: false,
                trigger: "auto_memorize_disabled".to_string(),
            });
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Use the extractor to extract memories (LLM-powered if configured)
        let extraction_result = self.extractor.extract_memories(conversation).await?;

        // Store the extracted memories
        for item in &extraction_result.memories {
            let scope = match item.level {
                MemoryLevel::User => scopes::USER,
                MemoryLevel::Session => scopes::SESSION,
                MemoryLevel::Agent => scopes::AGENT,
            };

            let mut metadata = item.metadata.clone();
            metadata.insert("memory_id".to_string(), serde_json::json!(&item.id));
            metadata.insert("category".to_string(), serde_json::json!(&item.category));
            metadata.insert("auto_memorize".to_string(), serde_json::json!(true));

            self.semantic.remember(
                agent_id,
                &item.content,
                MemorySource::Inference,
                scope,
                metadata,
            )?;
        }

        // Also store the full conversation as session memory for reference
        let content = conversation
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        if !content.is_empty() {
            let mut metadata = HashMap::new();
            metadata.insert("auto_memorize".to_string(), serde_json::json!(true));
            metadata.insert(
                "extraction_performed".to_string(),
                serde_json::json!(extraction_result.has_content),
            );
            self.semantic.remember(
                agent_id,
                &content,
                MemorySource::Conversation,
                scopes::SESSION,
                metadata,
            )?;
        }

        Ok(ExtractionResult {
            memories: Vec::new(), // Would be populated by LLM extraction in production
            has_content: !content.is_empty(),
            trigger: "conversation".to_string(),
        })
    }

    /// Proactively retrieve relevant context before agent execution.
    async fn auto_retrieve(&self, user_id: &str, query: &str) -> LibreFangResult<Vec<MemoryItem>> {
        if !self.config.auto_retrieve {
            return Ok(Vec::new());
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Create filter for this agent
        let filter = Some(MemoryFilter::agent(agent_id));

        // Search across all memory levels using semantic store
        let results = self
            .semantic
            .recall(query, self.config.max_retrieve, filter)?;

        Ok(results
            .into_iter()
            .map(|frag| {
                let level = MemoryLevel::from(frag.scope.as_str());
                MemoryItem {
                    id: frag.id.to_string(),
                    content: frag.content,
                    level,
                    category: frag
                        .metadata
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    metadata: frag.metadata,
                    created_at: frag.created_at,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_proactive_memory_search() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        // Add some memories
        let agent_id = AgentId::new().to_string();
        store
            .add(
                &[
                    serde_json::json!({"role": "user", "content": "I prefer dark mode"})
                ],
                &agent_id,
            )
            .await
            .unwrap();

        // Search
        let results = store.search("dark mode", &agent_id, 10).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_proactive_memory_get() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Get should return empty for now (no user-level memories stored)
        let results = store.get(&agent_id).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_auto_memorize() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Run auto_memorize
        let result = store
            .auto_memorize(
                &agent_id,
                &[serde_json::json!({
                    "role": "user",
                    "content": "Remember that I prefer dark mode"
                })],
            )
            .await
            .unwrap();

        assert!(result.has_content);
    }

    #[tokio::test]
    async fn test_auto_retrieve() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Add some content using the same agent_id
        let msg = serde_json::json!({"role": "user", "content": "My name is John"});
        store.add(&[msg], &agent_id).await.unwrap();

        // Also add using the search method to verify storage works
        let msg2 = serde_json::json!({"role": "user", "content": "I prefer dark mode"});
        store.add(&[msg2], &agent_id).await.unwrap();

        // Retrieve - should find content from this agent
        let results = store.auto_retrieve(&agent_id, "dark mode").await.unwrap();

        // Print for debugging
        println!("Auto-retrieve results: {:?}", results.len());

        // Should find at least one result
        assert!(results.len() > 0);
    }

    #[test]
    fn test_memory_level_from_str() {
        assert_eq!(MemoryLevel::from("user"), MemoryLevel::User);
        assert_eq!(MemoryLevel::from("session"), MemoryLevel::Session);
        assert_eq!(MemoryLevel::from("agent"), MemoryLevel::Agent);
        assert_eq!(MemoryLevel::from("unknown"), MemoryLevel::Session);
    }
}
