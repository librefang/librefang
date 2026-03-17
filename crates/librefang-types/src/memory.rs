//! Memory substrate types: fragments, sources, filters, and the unified Memory trait.
//! Also includes proactive memory types for mem0-style API.

use crate::agent::AgentId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Memory levels for multi-level memory (User/Session/Agent)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLevel {
    /// User-level memory (persistent across sessions)
    User,
    /// Session-level memory (current conversation)
    #[default]
    Session,
    /// Agent-level memory (agent-specific learned behaviors)
    Agent,
}

impl MemoryLevel {
    /// Return the scope string used in storage.
    pub fn scope_str(&self) -> &'static str {
        match self {
            MemoryLevel::User => "user_memory",
            MemoryLevel::Session => "session_memory",
            MemoryLevel::Agent => "agent_memory",
        }
    }
}

impl From<&str> for MemoryLevel {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "user" | "user_memory" => MemoryLevel::User,
            "session" | "session_memory" => MemoryLevel::Session,
            "agent" | "agent_memory" => MemoryLevel::Agent,
            _ => MemoryLevel::Session,
        }
    }
}

impl std::str::FromStr for MemoryLevel {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(MemoryLevel::from(s))
    }
}

/// A simple memory item for mem0-style API.
/// This is a simplified version of MemoryFragment for external use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    /// Unique ID.
    pub id: String,
    /// The memory content.
    pub content: String,
    /// Memory level (user/session/agent).
    pub level: MemoryLevel,
    /// Optional category for grouping.
    pub category: Option<String>,
    /// Metadata key-value pairs.
    pub metadata: HashMap<String, serde_json::Value>,
    /// When this memory was created.
    pub created_at: DateTime<Utc>,
}

impl MemoryItem {
    /// Create a new memory item.
    pub fn new(content: String, level: MemoryLevel) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            level,
            category: None,
            metadata: HashMap::new(),
            created_at: Utc::now(),
        }
    }

    /// Create a user-level memory item.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(content.into(), MemoryLevel::User)
    }

    /// Create a session-level memory item.
    pub fn session(content: impl Into<String>) -> Self {
        Self::new(content.into(), MemoryLevel::Session)
    }

    /// Create an agent-level memory item.
    pub fn agent(content: impl Into<String>) -> Self {
        Self::new(content.into(), MemoryLevel::Agent)
    }

    /// Set category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Configuration for proactive memory system.
///
/// Example in config.toml:
/// ```toml
/// [proactive_memory]
/// auto_memorize = true
/// auto_retrieve = true
/// max_retrieve = 10
/// session_ttl_hours = 24
/// extraction_model = "gpt-4o-mini"  # optional, enables LLM-powered extraction
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProactiveMemoryConfig {
    /// Enable auto-memorize after agent execution.
    pub auto_memorize: bool,
    /// Enable auto-retrieve before agent execution.
    pub auto_retrieve: bool,
    /// Maximum memories to retrieve per query.
    pub max_retrieve: usize,
    /// Confidence threshold for near-duplicate detection (0.0 - 1.0).
    pub extraction_threshold: f32,
    /// LLM model to use for extraction. If None, uses rule-based extraction.
    pub extraction_model: Option<String>,
    /// Categories to extract from conversations.
    pub extract_categories: Vec<String>,
    /// Session memory TTL in hours. Memories older than this are cleaned up
    /// automatically before each agent execution. Default: 24 hours.
    pub session_ttl_hours: u32,
}

impl Default for ProactiveMemoryConfig {
    fn default() -> Self {
        Self {
            auto_memorize: true,
            auto_retrieve: true,
            max_retrieve: 10,
            extraction_threshold: 0.7,
            extraction_model: None,
            extract_categories: vec![
                "user_preference".to_string(),
                "important_fact".to_string(),
                "task_context".to_string(),
                "relationship".to_string(),
            ],
            session_ttl_hours: 24,
        }
    }
}

/// Result from LLM-powered memory extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// Extracted memory items.
    pub memories: Vec<MemoryItem>,
    /// Whether extraction found anything worth remembering.
    pub has_content: bool,
    /// Original query that triggered extraction.
    pub trigger: String,
}

/// Trait for LLM-powered memory extraction.
///
/// This trait allows the runtime to inject an LLM client for memory extraction
/// without creating circular dependencies between librefang-memory and librefang-runtime.
///
/// Implement this trait in the runtime to enable automatic memory extraction.
#[async_trait]
pub trait MemoryExtractor: Send + Sync {
    /// Extract important memories from conversation messages using LLM.
    ///
    /// Takes conversation messages and returns extracted memory items.
    /// The implementation should use an LLM to identify:
    /// - User preferences
    /// - Important facts
    /// - Task context
    /// - Relationship information
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
    ) -> crate::error::LibreFangResult<ExtractionResult>;

    /// Generate a search context from retrieved memories.
    ///
    /// Takes retrieved memory items and formats them for injection
    /// into the agent's context prompt.
    fn format_context(&self, memories: &[MemoryItem]) -> String;
}

/// Default implementation of MemoryExtractor that uses simple rule-based extraction.
///
/// This provides basic functionality without requiring an LLM.
pub struct DefaultMemoryExtractor;

#[async_trait]
impl MemoryExtractor for DefaultMemoryExtractor {
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
    ) -> crate::error::LibreFangResult<ExtractionResult> {
        let mut memories = Vec::new();

        // Simple keyword-based extraction (fallback when no LLM available).
        // Only extract from user messages to avoid assistant echo.
        for message in messages {
            let role = message
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user");
            if role != "user" {
                continue;
            }
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let lower = content.to_lowercase();

            // Extract preferences — use strict patterns to avoid false positives
            if lower.contains("i prefer ")
                || lower.contains("i always ")
                || lower.contains("i never ")
                || lower.contains("i dislike ")
                || lower.contains("my favorite ")
            {
                let mut metadata = HashMap::new();
                metadata.insert("extracted_from".to_string(), serde_json::json!(role));
                memories.push(MemoryItem {
                    id: Uuid::new_v4().to_string(),
                    content: content.to_string(),
                    level: MemoryLevel::User,
                    category: Some("user_preference".to_string()),
                    metadata,
                    created_at: Utc::now(),
                });
            }

            // Extract important facts (explicit identity statements)
            if lower.contains("my name is")
                || lower.contains("i work at ")
                || lower.contains("i live in ")
                || lower.contains("my job is ")
            {
                let mut metadata = HashMap::new();
                metadata.insert("extracted_from".to_string(), serde_json::json!(role));
                memories.push(MemoryItem {
                    id: Uuid::new_v4().to_string(),
                    content: content.to_string(),
                    level: MemoryLevel::User,
                    category: Some("important_fact".to_string()),
                    metadata,
                    created_at: Utc::now(),
                });
            }
        }

        Ok(ExtractionResult {
            has_content: !memories.is_empty(),
            memories,
            trigger: "default_extractor".to_string(),
        })
    }

    fn format_context(&self, memories: &[MemoryItem]) -> String {
        if memories.is_empty() {
            return String::new();
        }

        let mut context = String::from("## Relevant Memories\n\n");
        for mem in memories {
            let level_str = match mem.level {
                MemoryLevel::User => "[User Preference]",
                MemoryLevel::Session => "[Session Context]",
                MemoryLevel::Agent => "[Agent Learning]",
            };
            context.push_str(&format!(
                "- {} {}\n  {}\n",
                level_str,
                mem.category.as_deref().unwrap_or("general"),
                mem.content
            ));
        }
        context
    }
}

/// Unique identifier for a memory fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(pub Uuid);

impl MemoryId {
    /// Create a new random MemoryId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MemoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a memory came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// From a conversation/interaction.
    Conversation,
    /// From a document that was processed.
    Document,
    /// From an observation (tool output, web page, etc.).
    Observation,
    /// Inferred by the agent from existing knowledge.
    Inference,
    /// Explicitly provided by the user.
    UserProvided,
    /// From a system event.
    System,
}

/// A single unit of memory stored in the semantic store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFragment {
    /// Unique ID.
    pub id: MemoryId,
    /// Which agent owns this memory.
    pub agent_id: AgentId,
    /// The textual content of this memory.
    pub content: String,
    /// Vector embedding (populated by the semantic store).
    pub embedding: Option<Vec<f32>>,
    /// Arbitrary metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// How this memory was created.
    pub source: MemorySource,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// When this memory was created.
    pub created_at: DateTime<Utc>,
    /// When this memory was last accessed.
    pub accessed_at: DateTime<Utc>,
    /// How many times this memory has been accessed.
    pub access_count: u64,
    /// Memory scope/collection name.
    pub scope: String,
}

/// Filter criteria for memory recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryFilter {
    /// Filter by agent ID.
    pub agent_id: Option<AgentId>,
    /// Filter by source type.
    pub source: Option<MemorySource>,
    /// Filter by scope.
    pub scope: Option<String>,
    /// Minimum confidence threshold.
    pub min_confidence: Option<f32>,
    /// Only memories created after this time.
    pub after: Option<DateTime<Utc>>,
    /// Only memories created before this time.
    pub before: Option<DateTime<Utc>>,
    /// Metadata key-value filters.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl MemoryFilter {
    /// Create a filter for a specific agent.
    pub fn agent(agent_id: AgentId) -> Self {
        Self {
            agent_id: Some(agent_id),
            ..Default::default()
        }
    }

    /// Create a filter for a specific scope.
    pub fn scope(scope: impl Into<String>) -> Self {
        Self {
            scope: Some(scope.into()),
            ..Default::default()
        }
    }
}

/// An entity in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique entity ID.
    pub id: String,
    /// Entity type (Person, Organization, Project, etc.).
    pub entity_type: EntityType,
    /// Display name.
    pub name: String,
    /// Arbitrary properties.
    pub properties: HashMap<String, serde_json::Value>,
    /// When this entity was created.
    pub created_at: DateTime<Utc>,
    /// When this entity was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Types of entities in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// A person.
    Person,
    /// An organization.
    Organization,
    /// A project.
    Project,
    /// A concept or idea.
    Concept,
    /// An event.
    Event,
    /// A location.
    Location,
    /// A document.
    Document,
    /// A tool.
    Tool,
    /// A custom type.
    Custom(String),
}

/// A relation between two entities in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    /// Source entity ID.
    pub source: String,
    /// Relation type.
    pub relation: RelationType,
    /// Target entity ID.
    pub target: String,
    /// Arbitrary properties on the relation.
    pub properties: HashMap<String, serde_json::Value>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// When this relation was created.
    pub created_at: DateTime<Utc>,
}

/// Types of relations in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// Entity works at an organization.
    WorksAt,
    /// Entity knows about a concept.
    KnowsAbout,
    /// Entities are related.
    RelatedTo,
    /// Entity depends on another.
    DependsOn,
    /// Entity is owned by another.
    OwnedBy,
    /// Entity was created by another.
    CreatedBy,
    /// Entity is located in another.
    LocatedIn,
    /// Entity is part of another.
    PartOf,
    /// Entity uses another.
    Uses,
    /// Entity produces another.
    Produces,
    /// A custom relation type.
    Custom(String),
}

/// A pattern for querying the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPattern {
    /// Optional source entity filter.
    pub source: Option<String>,
    /// Optional relation type filter.
    pub relation: Option<RelationType>,
    /// Optional target entity filter.
    pub target: Option<String>,
    /// Maximum traversal depth.
    pub max_depth: u32,
}

/// A result from a graph query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMatch {
    /// The source entity.
    pub source: Entity,
    /// The relation.
    pub relation: Relation,
    /// The target entity.
    pub target: Entity,
}

/// Report from memory consolidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// Number of memories merged.
    pub memories_merged: u64,
    /// Number of memories whose confidence decayed.
    pub memories_decayed: u64,
    /// How long the consolidation took.
    pub duration_ms: u64,
}

/// Format for memory export/import.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ExportFormat {
    /// JSON format.
    Json,
    /// MessagePack binary format.
    MessagePack,
}

/// Report from memory import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    /// Number of entities imported.
    pub entities_imported: u64,
    /// Number of relations imported.
    pub relations_imported: u64,
    /// Number of memories imported.
    pub memories_imported: u64,
    /// Errors encountered during import.
    pub errors: Vec<String>,
}

/// The unified Memory trait that agents interact with.
///
/// This abstracts over the structured store (SQLite), semantic store,
/// and knowledge graph, presenting a single coherent API.
#[async_trait]
pub trait Memory: Send + Sync {
    // -- Key-value operations (structured store) --

    /// Get a value by key for a specific agent.
    async fn get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> crate::error::LibreFangResult<Option<serde_json::Value>>;

    /// Set a key-value pair for a specific agent.
    async fn set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> crate::error::LibreFangResult<()>;

    /// Delete a key-value pair for a specific agent.
    async fn delete(&self, agent_id: AgentId, key: &str) -> crate::error::LibreFangResult<()>;

    // -- Semantic operations --

    /// Store a new memory fragment.
    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> crate::error::LibreFangResult<MemoryId>;

    /// Semantic search for relevant memories.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> crate::error::LibreFangResult<Vec<MemoryFragment>>;

    /// Soft-delete a memory fragment.
    async fn forget(&self, id: MemoryId) -> crate::error::LibreFangResult<()>;

    // -- Knowledge graph operations --

    /// Add an entity to the knowledge graph.
    async fn add_entity(&self, entity: Entity) -> crate::error::LibreFangResult<String>;

    /// Add a relation between entities.
    async fn add_relation(&self, relation: Relation) -> crate::error::LibreFangResult<String>;

    /// Query the knowledge graph.
    async fn query_graph(
        &self,
        pattern: GraphPattern,
    ) -> crate::error::LibreFangResult<Vec<GraphMatch>>;

    // -- Maintenance --

    /// Consolidate and optimize memory.
    async fn consolidate(&self) -> crate::error::LibreFangResult<ConsolidationReport>;

    /// Export all memory data.
    async fn export(&self, format: ExportFormat) -> crate::error::LibreFangResult<Vec<u8>>;

    /// Import memory data.
    async fn import(
        &self,
        data: &[u8],
        format: ExportFormat,
    ) -> crate::error::LibreFangResult<ImportReport>;
}

/// Trait for proactive memory operations (mem0-style API).
///
/// This provides a simple, unified API for memory operations similar to mem0:
/// - search() - semantic search
/// - add() - store with automatic extraction
/// - get() - retrieve user preferences
/// - list() - list memories by category
#[async_trait]
pub trait ProactiveMemory: Send + Sync {
    /// Semantic search for relevant memories.
    async fn search(
        &self,
        query: &str,
        user_id: &str,
        limit: usize,
    ) -> crate::error::LibreFangResult<Vec<MemoryItem>>;

    /// Add memories with automatic extraction (LLM-powered).
    /// Defaults to Session level storage.
    /// Returns the list of memories that were stored.
    async fn add(
        &self,
        messages: &[serde_json::Value],
        user_id: &str,
    ) -> crate::error::LibreFangResult<Vec<MemoryItem>>;

    /// Add memories at a specific memory level (User/Session/Agent).
    async fn add_with_level(
        &self,
        messages: &[serde_json::Value],
        user_id: &str,
        level: MemoryLevel,
    ) -> crate::error::LibreFangResult<()>;

    /// Get user preferences/memories.
    async fn get(&self, user_id: &str) -> crate::error::LibreFangResult<Vec<MemoryItem>>;

    /// List memories by category.
    async fn list(
        &self,
        user_id: &str,
        category: Option<&str>,
    ) -> crate::error::LibreFangResult<Vec<MemoryItem>>;

    /// Delete a specific memory by ID.
    async fn delete(&self, memory_id: &str, user_id: &str) -> crate::error::LibreFangResult<bool>;

    /// Update a memory's content (delete + re-add with same metadata).
    async fn update(
        &self,
        memory_id: &str,
        user_id: &str,
        content: &str,
    ) -> crate::error::LibreFangResult<bool>;
}

/// Trait for proactive memory hooks (auto_memorize, auto_retrieve).
///
/// This provides hooks for automatic memory extraction and retrieval:
/// - auto_memorize() - extract important info after agent runs
/// - auto_retrieve() - proactively load context before agent runs
#[async_trait]
pub trait ProactiveMemoryHooks: Send + Sync {
    /// Extract and store important information after agent execution.
    async fn auto_memorize(
        &self,
        user_id: &str,
        conversation: &[serde_json::Value],
    ) -> crate::error::LibreFangResult<ExtractionResult>;

    /// Proactively retrieve relevant context before agent execution.
    async fn auto_retrieve(
        &self,
        user_id: &str,
        query: &str,
    ) -> crate::error::LibreFangResult<Vec<MemoryItem>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_filter_agent() {
        let id = AgentId::new();
        let filter = MemoryFilter::agent(id);
        assert_eq!(filter.agent_id, Some(id));
        assert!(filter.source.is_none());
    }

    #[test]
    fn test_memory_fragment_serialization() {
        let fragment = MemoryFragment {
            id: MemoryId::new(),
            agent_id: AgentId::new(),
            content: "Test memory".to_string(),
            embedding: None,
            metadata: HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 0.95,
            created_at: Utc::now(),
            accessed_at: Utc::now(),
            access_count: 0,
            scope: "episodic".to_string(),
        };
        let json = serde_json::to_string(&fragment).unwrap();
        let deserialized: MemoryFragment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "Test memory");
    }

    #[test]
    fn test_memory_item_creation() {
        let item = MemoryItem::user("Prefers dark mode");
        assert_eq!(item.level, MemoryLevel::User);
        assert_eq!(item.content, "Prefers dark mode");
    }

    #[test]
    fn test_memory_item_with_category() {
        let item = MemoryItem::session("User asked about pricing").with_category("inquiry");
        assert_eq!(item.category, Some("inquiry".to_string()));
    }

    #[test]
    fn test_proactive_memory_config_default() {
        let config = ProactiveMemoryConfig::default();
        assert!(config.auto_memorize);
        assert!(config.auto_retrieve);
        assert_eq!(config.max_retrieve, 10);
    }
}
