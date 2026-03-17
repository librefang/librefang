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

use crate::knowledge::KnowledgeStore;
use crate::semantic::SemanticStore;
use crate::structured::StructuredStore;
use crate::MemorySubstrate;

use async_trait::async_trait;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    DefaultMemoryExtractor, Entity, EntityType, ExtractionResult, GraphPattern, MemoryAction,
    MemoryAddResult, MemoryExtractor, MemoryFilter, MemoryId, MemoryItem, MemoryLevel,
    MemorySource, ProactiveMemory, ProactiveMemoryConfig, ProactiveMemoryHooks, Relation,
    RelationTriple, RelationType,
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
/// let context = store.auto_retrieve("user123", "What did I tell you about my preferences?").await.unwrap();
/// ```
#[derive(Clone)]
pub struct ProactiveMemoryStore {
    #[allow(dead_code)]
    substrate: Arc<MemorySubstrate>,
    structured: StructuredStore,
    semantic: SemanticStore,
    knowledge: KnowledgeStore,
    config: ProactiveMemoryConfig,
    /// Memory extractor for LLM-powered extraction
    extractor: Arc<dyn MemoryExtractor>,
}

impl ProactiveMemoryStore {
    /// Create a new proactive memory store with default extractor.
    pub fn new(substrate: Arc<MemorySubstrate>, config: ProactiveMemoryConfig) -> Self {
        let conn = substrate.usage_conn();
        let knowledge = substrate.knowledge().clone();
        Self {
            structured: StructuredStore::new(Arc::clone(&conn)),
            semantic: SemanticStore::new(conn),
            knowledge,
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
        let knowledge = substrate.knowledge().clone();
        Self {
            structured: StructuredStore::new(Arc::clone(&conn)),
            semantic: SemanticStore::new(conn),
            knowledge,
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

    /// Parse user_id string into AgentId.
    fn parse_agent_id(user_id: &str) -> LibreFangResult<AgentId> {
        user_id
            .parse()
            .map_err(|e| LibreFangError::Internal(format!("Failed to parse user_id: {}", e)))
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

    /// Core mem0 decision flow: search for similar memories, decide action, execute.
    ///
    /// Returns `None` if the decision was NOOP (skip duplicate).
    async fn add_with_decision(
        &self,
        agent_id: AgentId,
        item: &MemoryItem,
    ) -> LibreFangResult<Option<MemoryAddResult>> {
        // Search for similar existing memories (top 5 candidates)
        let existing =
            self.semantic
                .recall(&item.content, 5, Some(MemoryFilter::agent(agent_id)))?;

        // Ask the extractor to decide: ADD, UPDATE, or NOOP
        let action = self.extractor.decide_action(item, &existing).await?;

        match action {
            MemoryAction::Noop => {
                tracing::debug!(
                    "Memory decision: NOOP (skip duplicate): {}",
                    truncate_for_log(&item.content, 80)
                );
                Ok(None)
            }
            MemoryAction::Add => {
                let mut metadata = item.metadata.clone();
                metadata.insert("category".to_string(), serde_json::json!(&item.category));
                self.semantic.remember(
                    agent_id,
                    &item.content,
                    MemorySource::Conversation,
                    item.level.scope_str(),
                    metadata,
                )?;
                tracing::debug!(
                    "Memory decision: ADD new: {}",
                    truncate_for_log(&item.content, 80)
                );
                Ok(Some(MemoryAddResult {
                    item: item.clone(),
                    action: MemoryAction::Add,
                    replaced_id: None,
                }))
            }
            MemoryAction::Update { ref existing_id } => {
                // Parse the old memory ID and update in-place
                let old_uuid = uuid::Uuid::parse_str(existing_id).map_err(|e| {
                    LibreFangError::Internal(format!("Invalid existing memory ID: {e}"))
                })?;
                let old_mid = MemoryId(old_uuid);

                let mut metadata = item.metadata.clone();
                metadata.insert("category".to_string(), serde_json::json!(&item.category));
                metadata.insert("updated_from".to_string(), serde_json::json!(existing_id));

                // Update content in-place (preserves ID, agent, scope, access stats)
                self.semantic
                    .update_content(old_mid, &item.content, Some(metadata))?;

                tracing::debug!(
                    "Memory decision: UPDATE {} -> {}",
                    existing_id,
                    truncate_for_log(&item.content, 80)
                );
                Ok(Some(MemoryAddResult {
                    item: item.clone(),
                    action: action.clone(),
                    replaced_id: Some(existing_id.clone()),
                }))
            }
        }
    }

    /// Store extracted relation triples into the knowledge graph.
    ///
    /// For each triple, creates entities (upsert) and a relation between them.
    fn store_relations(&self, triples: &[RelationTriple]) {
        for triple in triples {
            let source_type = parse_entity_type(&triple.subject_type);
            let target_type = parse_entity_type(&triple.object_type);

            // Upsert source entity
            let source_id = match self.knowledge.add_entity(Entity {
                id: normalize_entity_id(&triple.subject),
                entity_type: source_type,
                name: triple.subject.clone(),
                properties: HashMap::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }) {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("Failed to add entity '{}': {}", triple.subject, e);
                    continue;
                }
            };

            // Upsert target entity
            let target_id = match self.knowledge.add_entity(Entity {
                id: normalize_entity_id(&triple.object),
                entity_type: target_type,
                name: triple.object.clone(),
                properties: HashMap::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }) {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("Failed to add entity '{}': {}", triple.object, e);
                    continue;
                }
            };

            // Add relation
            let relation_type = parse_relation_type(&triple.relation);
            if let Err(e) = self.knowledge.add_relation(Relation {
                source: source_id,
                relation: relation_type,
                target: target_id,
                properties: HashMap::new(),
                confidence: 0.9,
                created_at: chrono::Utc::now(),
            }) {
                tracing::warn!(
                    "Failed to add relation '{}' -> '{}': {}",
                    triple.subject,
                    triple.object,
                    e
                );
            }
        }
    }

    /// Query the knowledge graph for entities mentioned in a query.
    ///
    /// Returns formatted context string with related entities and relationships.
    fn graph_context(&self, query: &str) -> Option<String> {
        // Search for entities whose names appear in the query
        let query_lower = query.to_lowercase();

        // Query graph for relations where source or target name matches
        let matches = self
            .knowledge
            .query_graph(GraphPattern {
                source: None,
                relation: None,
                target: None,
                max_depth: 1,
            })
            .unwrap_or_default();

        // Filter to matches relevant to the query
        let relevant: Vec<_> = matches
            .iter()
            .filter(|m| {
                query_lower.contains(&m.source.name.to_lowercase())
                    || query_lower.contains(&m.target.name.to_lowercase())
            })
            .collect();

        if relevant.is_empty() {
            return None;
        }

        let mut context = String::from("## Knowledge Graph\n\n");
        for m in relevant.iter().take(10) {
            context.push_str(&format!(
                "- {} ({:?}) → {:?} → {} ({:?})\n",
                m.source.name,
                m.source.entity_type,
                m.relation.relation,
                m.target.name,
                m.target.entity_type,
            ));
        }
        Some(context)
    }

    /// Format retrieved memories into a context string for prompt injection.
    ///
    /// Also includes relevant knowledge graph relations if any entity
    /// names appear in the memory content.
    pub fn format_context_with_query(&self, memories: &[MemoryItem], query: &str) -> String {
        let mut context = self.extractor.format_context(memories);

        // Append knowledge graph context if relevant
        if let Some(graph_ctx) = self.graph_context(query) {
            context.push('\n');
            context.push_str(&graph_ctx);
        }

        context
    }

    /// Format retrieved memories into a context string for prompt injection.
    pub fn format_context(&self, memories: &[MemoryItem]) -> String {
        self.extractor.format_context(memories)
    }

    /// Get memory statistics for a user/agent.
    ///
    /// Uses efficient SQL COUNT queries instead of loading all items.
    pub async fn stats(&self, user_id: &str) -> LibreFangResult<MemoryStats> {
        let agent_id = Self::parse_agent_id(user_id)?;

        let user_count = self.semantic.count(agent_id, Some(scopes::USER))? as usize;
        let session_count = self.semantic.count(agent_id, Some(scopes::SESSION))? as usize;
        let agent_count = self.semantic.count(agent_id, Some(scopes::AGENT))? as usize;
        let total = user_count + session_count + agent_count;

        // For category breakdown, still need to load items (categories are in metadata)
        let all_items = self.retrieve_memory_items(agent_id, None, None)?;
        let mut categories: HashMap<String, usize> = HashMap::new();
        for item in &all_items {
            if let Some(ref cat) = item.category {
                *categories.entry(cat.clone()).or_default() += 1;
            }
        }

        Ok(MemoryStats {
            total,
            user_count,
            session_count,
            agent_count,
            categories,
            auto_memorize_enabled: self.config.auto_memorize,
            auto_retrieve_enabled: self.config.auto_retrieve,
            llm_extraction: self.config.extraction_model.is_some(),
        })
    }

    /// Reset (soft-delete) ALL memories for a user/agent.
    pub fn reset(&self, user_id: &str) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        self.semantic.forget_by_agent(agent_id)
    }

    /// Clear memories at a specific level for a user/agent.
    ///
    /// Useful for clearing session memories while preserving user preferences.
    pub fn clear_level(&self, user_id: &str, level: MemoryLevel) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        self.semantic.forget_by_scope(agent_id, level.scope_str())
    }

    /// Clean up expired session memories older than the given duration.
    ///
    /// Call this periodically (e.g., on agent loop start) to prevent session
    /// memories from accumulating indefinitely.
    pub fn cleanup_expired_sessions(
        &self,
        user_id: &str,
        max_age: chrono::Duration,
    ) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let cutoff = chrono::Utc::now() - max_age;
        self.semantic
            .forget_older_than(agent_id, scopes::SESSION, cutoff)
    }

    /// Count memories for a user/agent, optionally filtered by level.
    pub fn count(&self, user_id: &str, level: Option<MemoryLevel>) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let scope = level.map(|l| l.scope_str());
        self.semantic.count(agent_id, scope)
    }

    /// Find duplicate/near-duplicate memories for a user/agent.
    ///
    /// Returns groups of memories that have identical or very similar content,
    /// useful for manual review or automated consolidation.
    pub async fn find_duplicates(
        &self,
        user_id: &str,
        level: Option<MemoryLevel>,
    ) -> LibreFangResult<Vec<Vec<MemoryItem>>> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let all_items = self.retrieve_memory_items(agent_id, level, None)?;

        // Group by substring containment (simple O(n^2) approach, fine for small N)
        let mut used = vec![false; all_items.len()];
        let mut groups: Vec<Vec<MemoryItem>> = Vec::new();

        for i in 0..all_items.len() {
            if used[i] {
                continue;
            }
            let mut group = vec![all_items[i].clone()];
            let a_lower = all_items[i].content.to_lowercase();

            for j in (i + 1)..all_items.len() {
                if used[j] {
                    continue;
                }
                let b_lower = all_items[j].content.to_lowercase();
                if a_lower.contains(&b_lower) || b_lower.contains(&a_lower) || a_lower == b_lower {
                    group.push(all_items[j].clone());
                    used[j] = true;
                }
            }

            if group.len() > 1 {
                groups.push(group);
            }
        }

        Ok(groups)
    }
}

/// Memory usage statistics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryStats {
    pub total: usize,
    pub user_count: usize,
    pub session_count: usize,
    pub agent_count: usize,
    pub categories: HashMap<String, usize>,
    /// Whether auto-memorize is enabled.
    pub auto_memorize_enabled: bool,
    /// Whether auto-retrieve is enabled.
    pub auto_retrieve_enabled: bool,
    /// Whether LLM-powered extraction is active.
    pub llm_extraction: bool,
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
        let agent_id = Self::parse_agent_id(user_id)?;

        // Filter by agent to avoid cross-agent leakage
        let filter = Some(MemoryFilter::agent(agent_id));
        let results = self.semantic.recall(query, limit, filter)?;

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

    /// Add memories with automatic extraction and conflict resolution (mem0-style).
    ///
    /// Core flow:
    /// 1. Extract memories from messages using configured extractor
    /// 2. For each extracted memory, search for similar existing memories
    /// 3. Let extractor decide: ADD (new), UPDATE (replace old), or NOOP (skip)
    /// 4. Execute the decision
    ///
    /// Returns the list of memories that were actually stored or updated.
    async fn add(
        &self,
        messages: &[serde_json::Value],
        user_id: &str,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Step 1: Extract structured memories
        let extraction = self.extractor.extract_memories(messages).await?;
        if !extraction.has_content {
            // Fallback: store raw message content as session memory
            let content = messages
                .iter()
                .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                .collect::<Vec<_>>()
                .join("\n");

            if content.is_empty() {
                return Ok(Vec::new());
            }

            self.semantic.remember(
                agent_id,
                &content,
                MemorySource::Conversation,
                scopes::SESSION,
                HashMap::new(),
            )?;

            return Ok(vec![MemoryItem::new(content, MemoryLevel::Session)]);
        }

        // Step 2-4: For each extracted memory, decide and execute
        let mut results = Vec::new();
        for item in &extraction.memories {
            let result = self.add_with_decision(agent_id, item).await?;
            if let Some(r) = result {
                results.push(r.item);
            }
        }

        // Step 5: Store extracted relations in knowledge graph
        if !extraction.relations.is_empty() {
            self.store_relations(&extraction.relations);
        }

        Ok(results)
    }

    /// Add memories at a specific memory level.
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

        let content = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        if content.is_empty() {
            return Ok(());
        }

        self.semantic.remember(
            agent_id,
            &content,
            MemorySource::Conversation,
            level.scope_str(),
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
    async fn list(
        &self,
        user_id: &str,
        category: Option<&str>,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        let agent_id = Self::parse_agent_id(user_id)?;
        self.retrieve_memory_items(agent_id, None, category)
    }

    /// Delete a specific memory by ID.
    async fn delete(&self, memory_id: &str, _user_id: &str) -> LibreFangResult<bool> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = librefang_types::memory::MemoryId(uuid);
        self.semantic.forget(mid)?;
        Ok(true)
    }

    /// Update a memory's content by deleting the old one and storing new content.
    async fn update(&self, memory_id: &str, user_id: &str, content: &str) -> LibreFangResult<bool> {
        let agent_id = Self::parse_agent_id(user_id)?;

        // Delete old
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = librefang_types::memory::MemoryId(uuid);
        self.semantic.forget(mid)?;

        // Store updated content as session memory
        self.semantic.remember(
            agent_id,
            content,
            MemorySource::Conversation,
            scopes::SESSION,
            HashMap::new(),
        )?;

        Ok(true)
    }
}

/// Normalize an entity name into a stable ID (lowercase, spaces → underscores).
fn normalize_entity_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// Parse entity type string from LLM into EntityType enum.
fn parse_entity_type(s: &str) -> EntityType {
    match s.to_lowercase().as_str() {
        "person" => EntityType::Person,
        "organization" | "company" | "org" => EntityType::Organization,
        "project" => EntityType::Project,
        "concept" | "idea" => EntityType::Concept,
        "event" => EntityType::Event,
        "location" | "place" => EntityType::Location,
        "document" | "doc" => EntityType::Document,
        "tool" | "language" | "framework" => EntityType::Tool,
        other => EntityType::Custom(other.to_string()),
    }
}

/// Parse relation type string from LLM into RelationType enum.
fn parse_relation_type(s: &str) -> RelationType {
    match s.to_lowercase().as_str() {
        "works_at" | "employed_at" => RelationType::WorksAt,
        "knows_about" | "knows" => RelationType::KnowsAbout,
        "related_to" => RelationType::RelatedTo,
        "depends_on" => RelationType::DependsOn,
        "owned_by" => RelationType::OwnedBy,
        "created_by" => RelationType::CreatedBy,
        "located_in" | "lives_in" => RelationType::LocatedIn,
        "part_of" => RelationType::PartOf,
        "uses" | "prefers" => RelationType::Uses,
        "produces" => RelationType::Produces,
        other => RelationType::Custom(other.to_string()),
    }
}

/// Truncate a string for log messages.
fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[async_trait]
impl ProactiveMemoryHooks for ProactiveMemoryStore {
    /// Extract and store important information after agent execution (mem0-style).
    ///
    /// Uses the full decision flow:
    /// 1. Extract memories from conversation
    /// 2. For each, search existing + decide ADD/UPDATE/NOOP
    /// 3. Execute decisions
    async fn auto_memorize(
        &self,
        user_id: &str,
        conversation: &[serde_json::Value],
    ) -> LibreFangResult<ExtractionResult> {
        if !self.config.auto_memorize || conversation.is_empty() {
            return Ok(ExtractionResult {
                memories: Vec::new(),
                relations: Vec::new(),
                has_content: false,
                trigger: "auto_memorize_disabled".to_string(),
            });
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Extract memories using configured extractor (LLM or rule-based)
        let extraction_result = self.extractor.extract_memories(conversation).await?;

        // Apply decision flow for each extracted memory
        let mut stored_memories = Vec::new();
        for item in &extraction_result.memories {
            // Tag with auto_memorize metadata
            let mut enriched = item.clone();
            enriched
                .metadata
                .insert("auto_memorize".to_string(), serde_json::json!(true));

            match self.add_with_decision(agent_id, &enriched).await {
                Ok(Some(result)) => stored_memories.push(result.item),
                Ok(None) => {} // NOOP
                Err(e) => {
                    tracing::warn!("auto_memorize decision failed for memory: {}", e);
                }
            }
        }

        // Store extracted relations in knowledge graph
        if !extraction_result.relations.is_empty() {
            self.store_relations(&extraction_result.relations);
        }

        Ok(ExtractionResult {
            has_content: !stored_memories.is_empty(),
            memories: stored_memories,
            relations: extraction_result.relations,
            trigger: extraction_result.trigger,
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
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode"})],
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

        // Run auto_memorize with content matching DefaultMemoryExtractor patterns
        let result = store
            .auto_memorize(
                &agent_id,
                &[serde_json::json!({
                    "role": "user",
                    "content": "I prefer dark mode for all my editors"
                })],
            )
            .await
            .unwrap();

        assert!(result.has_content);
        // DefaultMemoryExtractor should extract "I prefer" as user_preference
        assert!(!result.memories.is_empty());
        assert_eq!(
            result.memories[0].category,
            Some("user_preference".to_string())
        );
    }

    #[tokio::test]
    async fn test_auto_memorize_skips_assistant() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Assistant messages should not be extracted
        let result = store
            .auto_memorize(
                &agent_id,
                &[serde_json::json!({
                    "role": "assistant",
                    "content": "I prefer to help you with that"
                })],
            )
            .await
            .unwrap();

        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }

    #[tokio::test]
    async fn test_auto_retrieve() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Add some content using the same agent_id
        let msg = serde_json::json!({"role": "user", "content": "My name is John"});
        store.add(&[msg], &agent_id).await.unwrap();

        let msg2 = serde_json::json!({"role": "user", "content": "I prefer dark mode"});
        store.add(&[msg2], &agent_id).await.unwrap();

        // Retrieve - should find content from this agent
        let results = store.auto_retrieve(&agent_id, "dark mode").await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "Remember this fact"})],
                &agent_id,
            )
            .await
            .unwrap();

        // Search to get the memory ID
        let results = store
            .search("Remember this fact", &agent_id, 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
        let mem_id = &results[0].id;

        // Delete it
        let deleted = store.delete(mem_id, &agent_id).await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn test_update_memory() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "Old content"})],
                &agent_id,
            )
            .await
            .unwrap();

        let results = store.search("Old content", &agent_id, 10).await.unwrap();
        assert!(!results.is_empty());
        let mem_id = results[0].id.clone();

        // Update it
        let updated = store
            .update(&mem_id, &agent_id, "New content")
            .await
            .unwrap();
        assert!(updated);

        // Search should find new content
        let new_results = store.search("New content", &agent_id, 10).await.unwrap();
        assert!(!new_results.is_empty());
    }

    #[test]
    fn test_memory_level_from_str() {
        assert_eq!(MemoryLevel::from("user"), MemoryLevel::User);
        assert_eq!(MemoryLevel::from("session"), MemoryLevel::Session);
        assert_eq!(MemoryLevel::from("agent"), MemoryLevel::Agent);
        assert_eq!(MemoryLevel::from("unknown"), MemoryLevel::Session);
    }

    #[test]
    fn test_memory_level_scope_str() {
        assert_eq!(MemoryLevel::User.scope_str(), "user_memory");
        assert_eq!(MemoryLevel::Session.scope_str(), "session_memory");
        assert_eq!(MemoryLevel::Agent.scope_str(), "agent_memory");
    }

    #[tokio::test]
    async fn test_reset_agent_memories() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "First memory"})],
                &agent_id,
            )
            .await
            .unwrap();
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "Second memory"})],
                &agent_id,
            )
            .await
            .unwrap();

        // Verify memories exist
        let count = store.count(&agent_id, None).unwrap();
        assert!(count >= 2);

        // Reset all
        let deleted = store.reset(&agent_id).unwrap();
        assert!(deleted >= 2);

        // Verify memories are gone
        let count_after = store.count(&agent_id, None).unwrap();
        assert_eq!(count_after, 0);
    }

    #[tokio::test]
    async fn test_clear_level() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();

        // Add session-level memory (default)
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "Session info"})],
                &agent_id,
            )
            .await
            .unwrap();

        // Add user-level memory
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "User preference"})],
                &agent_id,
                MemoryLevel::User,
            )
            .await
            .unwrap();

        // Clear only session level
        let deleted = store.clear_level(&agent_id, MemoryLevel::Session).unwrap();
        assert!(deleted >= 1);

        // User-level should still exist
        let user_count = store.count(&agent_id, Some(MemoryLevel::User)).unwrap();
        assert!(user_count >= 1);
    }

    #[test]
    fn test_count_memories() {
        // Sync test since count is a sync method
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        let count = store.count(&agent_id, None).unwrap();
        assert_eq!(count, 0);
    }
}
