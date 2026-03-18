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
/// Trait for computing text embeddings (re-exported from runtime to avoid circular dep).
#[async_trait]
pub trait EmbeddingFn: Send + Sync {
    /// Compute embedding for a single text.
    async fn embed_one(&self, text: &str) -> LibreFangResult<Vec<f32>>;
}

pub struct ProactiveMemoryStore {
    #[allow(dead_code)]
    substrate: Arc<MemorySubstrate>,
    structured: StructuredStore,
    semantic: SemanticStore,
    knowledge: KnowledgeStore,
    config: ProactiveMemoryConfig,
    /// Memory extractor for LLM-powered extraction
    extractor: Arc<dyn MemoryExtractor>,
    /// Optional embedding driver for vector similarity search.
    /// When present, memories are stored with embeddings and search uses cosine similarity.
    /// When absent, falls back to LIKE text matching.
    embedding: Option<Arc<dyn EmbeddingFn>>,
    /// Per-agent counters for auto-consolidation (runs every 10 auto_memorize calls per agent).
    consolidation_counters: std::sync::Mutex<HashMap<String, u32>>,
}

impl Clone for ProactiveMemoryStore {
    fn clone(&self) -> Self {
        let counters = self
            .consolidation_counters
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        Self {
            substrate: self.substrate.clone(),
            structured: self.structured.clone(),
            semantic: self.semantic.clone(),
            knowledge: self.knowledge.clone(),
            config: self.config.clone(),
            extractor: self.extractor.clone(),
            embedding: self.embedding.clone(),
            consolidation_counters: std::sync::Mutex::new(counters),
        }
    }
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
            embedding: None,
            consolidation_counters: std::sync::Mutex::new(HashMap::new()),
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
            embedding: None,
            consolidation_counters: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Set the embedding driver for vector similarity search.
    ///
    /// When set, memories are stored with embeddings and search uses cosine similarity.
    /// When not set, falls back to LIKE text matching.
    pub fn with_embedding(mut self, driver: Arc<dyn EmbeddingFn>) -> Self {
        self.embedding = Some(driver);
        self
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
        // Generate embedding for the new memory (if driver available)
        let query_embedding = if let Some(ref emb) = self.embedding {
            emb.embed_one(&item.content).await.ok()
        } else {
            None
        };

        // Search for similar existing memories (top 5 candidates).
        // Use vector search if embedding available, otherwise keyword LIKE.
        let filter = Some(MemoryFilter::agent(agent_id));
        let existing = if let Some(ref qe) = query_embedding {
            self.semantic
                .recall_with_embedding(&item.content, 5, filter.clone(), Some(qe))?
        } else {
            let search_query = extract_search_keywords(&item.content);
            let mut results = self.semantic.recall(&search_query, 5, filter.clone())?;
            if results.is_empty() {
                results = self.semantic.recall(&item.content, 5, filter)?;
            }
            results
        };

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
                // Store with embedding if available
                let mem_id = self.semantic.remember_with_embedding(
                    agent_id,
                    &item.content,
                    MemorySource::Conversation,
                    item.level.scope_str(),
                    metadata,
                    query_embedding.as_deref(),
                )?;
                // Also store in KV using the semantic store's ID for consistency
                if let Ok(json) = serde_json::to_value(item) {
                    let _ = self
                        .structured
                        .set(agent_id, &format!("memory:{}", mem_id), json);
                }
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

                // Single fetch to avoid TOCTOU race between reading content and metadata
                let old_frag = self.semantic.get_by_id(old_mid, false)?;
                let old_content = old_frag
                    .as_ref()
                    .map(|f| f.content.clone())
                    .unwrap_or_default();

                let mut metadata = item.metadata.clone();
                metadata.insert("category".to_string(), serde_json::json!(&item.category));
                metadata.insert("updated_from".to_string(), serde_json::json!(existing_id));
                metadata.insert(
                    "previous_content".to_string(),
                    serde_json::json!(old_content),
                );
                metadata.insert(
                    "updated_at".to_string(),
                    serde_json::json!(chrono::Utc::now().to_rfc3339()),
                );

                // Build version history chain
                if let Some(ref old_frag) = old_frag {
                    if let Some(existing_history) = old_frag.metadata.get("version_history") {
                        // Append to existing history
                        let mut history = existing_history.clone();
                        if let Some(arr) = history.as_array_mut() {
                            arr.push(serde_json::json!({
                                "content": old_content,
                                "replaced_at": chrono::Utc::now().to_rfc3339(),
                            }));
                            metadata.insert("version_history".to_string(), history);
                        }
                    } else {
                        // Start new history chain
                        metadata.insert(
                            "version_history".to_string(),
                            serde_json::json!([{
                                "content": old_content,
                                "replaced_at": chrono::Utc::now().to_rfc3339(),
                            }]),
                        );
                    }
                }

                // Update content in-place (preserves ID, agent, scope, access stats)
                self.semantic
                    .update_content(old_mid, &item.content, Some(metadata))?;
                // Also update in KV so get()/list() reflect the change
                if let Ok(json) = serde_json::to_value(item) {
                    let _ = self
                        .structured
                        .set(agent_id, &format!("memory:{}", existing_id), json);
                }

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
    /// Deduplicates: skips if an identical (source, relation, target) already exists.
    pub fn store_relations(&self, triples: &[RelationTriple]) {
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

            // Add relation (skip if already exists)
            let relation_type = parse_relation_type(&triple.relation);
            match self
                .knowledge
                .has_relation(&source_id, &relation_type, &target_id)
            {
                Ok(true) => {
                    tracing::debug!(
                        "Skipping duplicate relation: {} -> {} -> {}",
                        triple.subject,
                        triple.relation,
                        triple.object,
                    );
                }
                Ok(false) => {
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
                Err(e) => {
                    tracing::debug!("Relation dedup check failed (non-fatal): {}", e);
                }
            }
        }
    }

    /// Query the knowledge graph for entities mentioned in a query.
    ///
    /// Extracts candidate entity names from the query, then does targeted
    /// graph lookups instead of loading all relations.
    fn graph_context(&self, query: &str) -> Option<String> {
        // Extract capitalized words and significant terms as entity candidates
        let candidates = extract_entity_candidates(query);
        if candidates.is_empty() {
            return None;
        }

        let mut all_matches = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for candidate in &candidates {
            // Query as source
            if let Ok(matches) = self.knowledge.query_graph(GraphPattern {
                source: Some(candidate.clone()),
                relation: None,
                target: None,
                max_depth: 1,
            }) {
                for m in matches {
                    let key = format!("{}-{:?}-{}", m.source.id, m.relation.relation, m.target.id);
                    if seen.insert(key) {
                        all_matches.push(m);
                    }
                }
            }
            // Query as target
            if let Ok(matches) = self.knowledge.query_graph(GraphPattern {
                source: None,
                relation: None,
                target: Some(candidate.clone()),
                max_depth: 1,
            }) {
                for m in matches {
                    let key = format!("{}-{:?}-{}", m.source.id, m.relation.relation, m.target.id);
                    if seen.insert(key) {
                        all_matches.push(m);
                    }
                }
            }
        }

        if all_matches.is_empty() {
            return None;
        }

        let mut context = String::from("## Knowledge Graph\n\n");
        for m in all_matches.iter().take(10) {
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

    /// Get memory statistics across ALL agents.
    ///
    /// Used by the dashboard to show global memory stats.
    pub async fn stats_all(&self) -> LibreFangResult<MemoryStats> {
        let user_count = self.semantic.count_all(Some(scopes::USER))? as usize;
        let session_count = self.semantic.count_all(Some(scopes::SESSION))? as usize;
        let agent_count = self.semantic.count_all(Some(scopes::AGENT))? as usize;
        let total = user_count + session_count + agent_count;

        // For category breakdown, use semantic recall with no agent filter
        let all_frags = self.semantic.recall("", 500, None)?;
        let mut categories: HashMap<String, usize> = HashMap::new();
        for frag in &all_frags {
            if let Some(cat) = frag.metadata.get("category").and_then(|v| v.as_str()) {
                *categories.entry(cat.to_string()).or_default() += 1;
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

    /// List memories across ALL agents, optionally filtered by category.
    ///
    /// Used by the dashboard to show all memories without agent scoping.
    pub async fn list_all(&self, category: Option<&str>) -> LibreFangResult<Vec<MemoryItem>> {
        // Use semantic recall with no agent filter to get all memories
        let results = self.semantic.recall("", 500, None)?;

        let items: Vec<MemoryItem> = results
            .into_iter()
            .filter(|frag| {
                if let Some(target_cat) = category {
                    frag.metadata.get("category").and_then(|v| v.as_str()) == Some(target_cat)
                } else {
                    true
                }
            })
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
            .collect();

        Ok(items)
    }

    /// Search memories across ALL agents by semantic similarity.
    ///
    /// Used by the dashboard to search all memories without agent scoping.
    pub async fn search_all(&self, query: &str, limit: usize) -> LibreFangResult<Vec<MemoryItem>> {
        // Use vector search if embedding driver available, with no agent filter
        let results = if let Some(ref emb) = self.embedding {
            if let Ok(qe) = emb.embed_one(query).await {
                self.semantic
                    .recall_with_embedding(query, limit, None, Some(&qe))?
            } else {
                self.semantic.recall(query, limit, None)?
            }
        } else {
            self.semantic.recall(query, limit, None)?
        };

        let items: Vec<MemoryItem> = results
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
            .collect();

        Ok(items)
    }

    /// Look up the real agent_id for a memory by its ID.
    ///
    /// Used by delete/update handlers that don't know which agent owns the memory.
    pub fn find_agent_id_for_memory(&self, memory_id: &str) -> LibreFangResult<Option<AgentId>> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = MemoryId(uuid);

        match self.semantic.get_by_id(mid, false)? {
            Some(frag) => Ok(Some(frag.agent_id)),
            None => Ok(None),
        }
    }

    /// Reset (soft-delete) ALL memories for a user/agent.
    pub fn reset(&self, user_id: &str) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let count = self.semantic.forget_by_agent(agent_id)?;

        // Also clean up all memory:* KV entries for this agent
        if let Ok(kv_pairs) = self.structured.list_kv(agent_id) {
            for (key, _) in kv_pairs {
                if key.starts_with("memory:") {
                    let _ = self.structured.delete(agent_id, &key);
                }
            }
        }

        Ok(count)
    }

    /// Clear memories at a specific level for a user/agent.
    ///
    /// Useful for clearing session memories while preserving user preferences.
    pub fn clear_level(&self, user_id: &str, level: MemoryLevel) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let count = self.semantic.forget_by_scope(agent_id, level.scope_str())?;

        // Also clean up matching KV entries for this level
        if let Ok(kv_pairs) = self.structured.list_kv(agent_id) {
            for (key, value) in kv_pairs {
                if !key.starts_with("memory:") {
                    continue;
                }
                // Check if this KV entry's level matches the one being cleared
                let level_str = value
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Session");
                if MemoryLevel::from(level_str) == level {
                    let _ = self.structured.delete(agent_id, &key);
                }
            }
        }

        Ok(count)
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
        let count = self
            .semantic
            .forget_older_than(agent_id, scopes::SESSION, cutoff)?;

        // Also clean up expired session KV entries
        if let Ok(kv_pairs) = self.structured.list_kv(agent_id) {
            for (key, value) in kv_pairs {
                if !key.starts_with("memory:") {
                    continue;
                }
                // Check if this is a session-level memory that's expired
                let level_str = value
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Session");
                if MemoryLevel::from(level_str) != MemoryLevel::Session {
                    continue;
                }
                let created_at_str = value.get("created_at").and_then(|v| v.as_str());
                if let Some(ts) = created_at_str {
                    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(ts) {
                        if created.with_timezone(&chrono::Utc) < cutoff {
                            let _ = self.structured.delete(agent_id, &key);
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Get the version history of a memory.
    ///
    /// Returns a list of previous content values, most recent first.
    /// Each entry has `content` and `replaced_at` timestamp.
    pub fn history(&self, memory_id: &str) -> LibreFangResult<Vec<serde_json::Value>> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = MemoryId(uuid);

        let frag = self
            .semantic
            .get_by_id(mid, false)?
            .ok_or_else(|| LibreFangError::Internal("Memory not found".to_string()))?;

        let history = frag
            .metadata
            .get("version_history")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Return in reverse chronological order (most recent first)
        let mut history = history;
        history.reverse();
        Ok(history)
    }

    /// Consolidate memories: merge near-duplicates and remove stale entries.
    ///
    /// This is the mem0-style maintenance operation that keeps memory clean:
    /// 1. Find duplicate groups using semantic similarity
    /// 2. Merge each group into the most recently accessed memory
    /// 3. Soft-delete the older duplicates
    ///
    /// Returns the number of memories merged (soft-deleted).
    pub async fn consolidate(&self, user_id: &str) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let groups = self.find_duplicates(user_id, None).await?;
        let mut merged_count = 0u64;

        for group in groups {
            if group.len() < 2 {
                continue;
            }

            // Keep the most recently created one as the "winner"
            let winner = group.iter().max_by_key(|m| m.created_at).cloned().unwrap();

            // Soft-delete all others
            for item in &group {
                if item.id != winner.id {
                    if let Ok(uuid) = uuid::Uuid::parse_str(&item.id) {
                        let mid = MemoryId(uuid);
                        if self.semantic.forget(mid).is_ok() {
                            // Also remove the KV entry
                            let _ = self
                                .structured
                                .delete(agent_id, &format!("memory:{}", item.id));
                            merged_count += 1;
                        }
                    }
                }
            }
        }

        tracing::info!(
            "Memory consolidation for {}: merged {} duplicates",
            user_id,
            merged_count
        );
        Ok(merged_count)
    }

    /// Count memories for a user/agent, optionally filtered by level.
    pub fn count(&self, user_id: &str, level: Option<MemoryLevel>) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let scope = level.map(|l| l.scope_str());
        self.semantic.count(agent_id, scope)
    }

    /// Find duplicate/near-duplicate memories for a user/agent.
    ///
    /// Uses both substring containment and word-overlap (Jaccard) similarity
    /// to detect semantic duplicates that simple exact-match would miss.
    /// Uses configurable `duplicate_threshold` from config.
    pub async fn find_duplicates(
        &self,
        user_id: &str,
        level: Option<MemoryLevel>,
    ) -> LibreFangResult<Vec<Vec<MemoryItem>>> {
        let agent_id = Self::parse_agent_id(user_id)?;

        // Try structured store first, fall back to semantic store
        let mut all_items = self.retrieve_memory_items(agent_id, level, None)?;

        // Also search semantic store if structured store returned nothing
        if all_items.is_empty() {
            let scope_filter = level.map(|l| {
                let mut f = MemoryFilter::agent(agent_id);
                f.scope = Some(l.scope_str().to_string());
                f
            });
            let filter = scope_filter.unwrap_or_else(|| MemoryFilter::agent(agent_id));
            let frags = self.semantic.recall("", 500, Some(filter))?;
            all_items = frags
                .into_iter()
                .map(|frag| MemoryItem {
                    id: frag.id.to_string(),
                    content: frag.content,
                    level: MemoryLevel::from(frag.scope.as_str()),
                    category: frag
                        .metadata
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    metadata: frag.metadata,
                    created_at: frag.created_at,
                })
                .collect();
        }

        // Limit to 100 most recent items to avoid O(n^2) blowup
        if all_items.len() > 100 {
            all_items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            all_items.truncate(100);
        }

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

                // Check substring containment
                let is_substring =
                    a_lower.contains(&b_lower) || b_lower.contains(&a_lower) || a_lower == b_lower;

                // Check semantic similarity (Jaccard word overlap)
                let similarity = librefang_types::memory::text_similarity(&a_lower, &b_lower);

                if is_substring || similarity > self.config.duplicate_threshold {
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
    /// Semantic search for relevant memories, enriched with knowledge graph context.
    ///
    /// Uses vector similarity when an embedding driver is configured,
    /// otherwise falls back to LIKE text matching.
    async fn search(
        &self,
        query: &str,
        user_id: &str,
        limit: usize,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        let agent_id = Self::parse_agent_id(user_id)?;

        // Filter by agent to avoid cross-agent leakage
        let filter = Some(MemoryFilter::agent(agent_id));

        // Use vector search if embedding driver available
        let results = if let Some(ref emb) = self.embedding {
            if let Ok(qe) = emb.embed_one(query).await {
                self.semantic
                    .recall_with_embedding(query, limit, filter, Some(&qe))?
            } else {
                self.semantic.recall(query, limit, filter)?
            }
        } else {
            self.semantic.recall(query, limit, filter)?
        };

        let mut items: Vec<MemoryItem> = results
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
            .collect();

        // Enrich with knowledge graph: if entities in query match graph nodes,
        // synthesize a context memory from graph relations.
        if items.len() < limit {
            if let Some(graph_ctx) = self.graph_context(query) {
                items.push(
                    MemoryItem::new(graph_ctx, MemoryLevel::Agent).with_category("knowledge_graph"),
                );
            }
        }

        Ok(items)
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

            let mem_id = self.semantic.remember(
                agent_id,
                &content,
                MemorySource::Conversation,
                scopes::SESSION,
                HashMap::new(),
            )?;

            let item = MemoryItem::new(content, MemoryLevel::Session);
            // Also store in KV using the semantic store's ID for consistency
            if let Ok(json) = serde_json::to_value(&item) {
                let _ = self
                    .structured
                    .set(agent_id, &format!("memory:{}", mem_id), json);
            }
            return Ok(vec![item]);
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

        let mem_id = self.semantic.remember(
            agent_id,
            &content,
            MemorySource::Conversation,
            level.scope_str(),
            HashMap::new(),
        )?;

        // Also store in KV using the semantic store's ID for consistency
        let item = MemoryItem::new(content, level);
        if let Ok(json) = serde_json::to_value(&item) {
            let _ = self
                .structured
                .set(agent_id, &format!("memory:{}", mem_id), json);
        }

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
    async fn delete(&self, memory_id: &str, user_id: &str) -> LibreFangResult<bool> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = librefang_types::memory::MemoryId(uuid);

        // Check if the memory exists before deleting
        if self.semantic.get_by_id(mid, false)?.is_none() {
            return Ok(false);
        }

        self.semantic.forget(mid)?;

        // Also clean up the KV store entry
        if let Ok(agent_id) = Self::parse_agent_id(user_id) {
            let _ = self
                .structured
                .delete(agent_id, &format!("memory:{}", memory_id));
        }

        Ok(true)
    }

    /// Update a memory's content in-place, preserving version history.
    async fn update(&self, memory_id: &str, user_id: &str, content: &str) -> LibreFangResult<bool> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = MemoryId(uuid);

        // Get old memory for version history
        let old_frag = match self.semantic.get_by_id(mid, false)? {
            Some(f) => f,
            None => return Ok(false),
        };

        // Build metadata with version history
        let mut metadata = old_frag.metadata.clone();
        let old_content = old_frag.content.clone();

        metadata.insert(
            "previous_content".to_string(),
            serde_json::json!(old_content),
        );
        metadata.insert(
            "updated_at".to_string(),
            serde_json::json!(chrono::Utc::now().to_rfc3339()),
        );

        // Append to version history chain
        let mut history: Vec<serde_json::Value> = metadata
            .get("version_history")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        history.push(serde_json::json!({
            "content": old_content,
            "replaced_at": chrono::Utc::now().to_rfc3339(),
        }));
        metadata.insert("version_history".to_string(), serde_json::json!(history));

        // Update content in-place (preserves ID, agent, scope, access stats)
        self.semantic.update_content(mid, content, Some(metadata))?;

        // Also update the KV store entry with new content
        if let Ok(agent_id) = Self::parse_agent_id(user_id) {
            let kv_key = format!("memory:{}", memory_id);
            if let Ok(Some(mut kv_val)) = self.structured.get(agent_id, &kv_key) {
                if let Some(obj) = kv_val.as_object_mut() {
                    obj.insert("content".to_string(), serde_json::json!(content));
                    obj.insert(
                        "updated_at".to_string(),
                        serde_json::json!(chrono::Utc::now().to_rfc3339()),
                    );
                }
                let _ = self.structured.set(agent_id, &kv_key, kv_val);
            }
        }

        Ok(true)
    }
}

/// Extract entity-like candidates from a query for knowledge graph lookup.
///
/// Looks for capitalized words (likely proper nouns), normalized entity IDs,
/// and significant multi-word terms.
fn extract_entity_candidates(query: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    // Capitalized words (proper nouns): "Alice", "Google", "Python"
    for word in query.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric());
        if trimmed.len() >= 2 {
            if let Some(first) = trimmed.chars().next() {
                if first.is_uppercase() {
                    candidates.push(trimmed.to_string());
                    // Also try normalized form (for entity ID matching)
                    candidates.push(normalize_entity_id(trimmed));
                }
            }
        }
    }

    // Also try "User" as it's a common entity in proactive memory
    if query.to_lowercase().contains("my ")
        || query.to_lowercase().contains("i ")
        || query.to_lowercase().starts_with("what did i")
    {
        candidates.push("User".to_string());
        candidates.push("user".to_string());
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

/// Extract significant keywords from content for broader LIKE search.
///
/// Instead of searching for the full content string (which requires exact substring match),
/// pick the most distinctive words to find related memories.
fn extract_search_keywords(content: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        "i", "a", "an", "the", "is", "am", "are", "was", "were", "be", "been", "being", "have",
        "has", "had", "do", "does", "did", "will", "would", "could", "should", "may", "might",
        "can", "shall", "for", "and", "but", "or", "nor", "not", "so", "yet", "at", "by", "in",
        "of", "on", "to", "up", "it", "my", "me", "we", "he", "she", "they", "this", "that",
        "with", "from", "all", "very", "just", "also", "than",
    ];

    let words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| {
            let lower = w.to_lowercase();
            lower.len() > 2 && !STOP_WORDS.contains(&lower.as_str())
        })
        .take(4) // Use up to 4 significant words
        .collect();

    if words.is_empty() {
        content.to_string()
    } else {
        // Return the longest keyword for LIKE matching; decide_action handles dedup
        words
            .iter()
            .max_by_key(|w| w.len())
            .unwrap_or(&words[0])
            .to_string()
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
        match s.char_indices().nth(max) {
            Some((idx, _)) => format!("{}...", &s[..idx]),
            None => s.to_string(),
        }
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

        // Auto-consolidation: merge duplicates every 10 auto_memorize calls per agent
        let count = {
            let mut counters = self
                .consolidation_counters
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let entry = counters.entry(user_id.to_string()).or_insert(0);
            *entry += 1;
            *entry
        };
        if count > 0 && count % 10 == 0 {
            match self.consolidate(user_id).await {
                Ok(merged) if merged > 0 => {
                    tracing::info!("Auto-consolidation: merged {} duplicate memories", merged);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("Auto-consolidation failed (non-fatal): {}", e);
                }
            }
        }

        Ok(ExtractionResult {
            has_content: !stored_memories.is_empty(),
            memories: stored_memories,
            relations: extraction_result.relations,
            trigger: extraction_result.trigger,
        })
    }

    /// Proactively retrieve relevant context before agent execution.
    ///
    /// Also performs session TTL cleanup if configured.
    async fn auto_retrieve(&self, user_id: &str, query: &str) -> LibreFangResult<Vec<MemoryItem>> {
        if !self.config.auto_retrieve {
            return Ok(Vec::new());
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Cleanup expired session memories (lightweight, runs on each retrieve)
        if self.config.session_ttl_hours > 0 {
            let ttl = chrono::Duration::hours(self.config.session_ttl_hours as i64);
            if let Err(e) = self.cleanup_expired_sessions(user_id, ttl) {
                tracing::debug!("Session TTL cleanup failed (non-fatal): {}", e);
            }
        }

        // Create filter for this agent
        let filter = Some(MemoryFilter::agent(agent_id));

        // Search across all memory levels — use vector search if available
        let results = if let Some(ref emb) = self.embedding {
            if let Ok(qe) = emb.embed_one(query).await {
                self.semantic.recall_with_embedding(
                    query,
                    self.config.max_retrieve,
                    filter,
                    Some(&qe),
                )?
            } else {
                self.semantic
                    .recall(query, self.config.max_retrieve, filter)?
            }
        } else {
            self.semantic
                .recall(query, self.config.max_retrieve, filter)?
        };

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

        // Get should return empty initially
        let results = store.get(&agent_id).await.unwrap();
        assert!(results.is_empty());

        // Add a user-level memory (via add_with_level)
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode"})],
                &agent_id,
                MemoryLevel::User,
            )
            .await
            .unwrap();

        // Also add via the main add() path which stores in KV
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer Rust programming"})],
                &agent_id,
            )
            .await
            .unwrap();

        // List all memories (includes KV-stored ones)
        let all = store.list(&agent_id, None).await.unwrap();
        // At least the KV-stored memory should be returned
        assert!(!all.is_empty(), "list() should return memories after add()");
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

        // Verify KV entry exists before delete
        let agent_id_parsed = ProactiveMemoryStore::parse_agent_id(&agent_id).unwrap();
        let kv_before = store
            .structured
            .get(agent_id_parsed, &format!("memory:{}", mem_id))
            .unwrap();
        assert!(kv_before.is_some(), "KV entry should exist after add()");

        // Delete it
        let deleted = store.delete(mem_id, &agent_id).await.unwrap();
        assert!(deleted);

        // Verify KV entry is also removed
        let kv_after = store
            .structured
            .get(agent_id_parsed, &format!("memory:{}", mem_id))
            .unwrap();
        assert!(
            kv_after.is_none(),
            "KV entry should be removed after delete()"
        );

        // Deleting non-existent memory should return false
        let deleted_again = store.delete(mem_id, &agent_id).await.unwrap();
        assert!(
            !deleted_again,
            "delete() should return false for non-existent memory"
        );
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

    #[tokio::test]
    async fn test_add_dedup_exact_match_is_noop() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // Add a preference
        let r1 = store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode"})],
                &agent_id,
            )
            .await
            .unwrap();
        assert_eq!(r1.len(), 1);

        // Add the exact same preference again — should be NOOP
        let r2 = store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode"})],
                &agent_id,
            )
            .await
            .unwrap();
        // Should not add a duplicate
        assert!(r2.is_empty());

        // Total count should still be 1
        let count = store.count(&agent_id, None).unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_add_updates_conflicting_preference() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // Add initial preference
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer Python for scripting"})],
                &agent_id,
            )
            .await
            .unwrap();

        // Add a superset preference (contains the old one) — should UPDATE
        let r2 = store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer Python for scripting and data analysis"})],
                &agent_id,
            )
            .await
            .unwrap();
        assert_eq!(r2.len(), 1);

        // Should still have only 1 memory (updated, not duplicated)
        let count = store.count(&agent_id, None).unwrap();
        assert_eq!(count, 1);

        // Content should be the updated version
        let results = store.search("Python", &agent_id, 10).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("data analysis"));
    }

    #[tokio::test]
    async fn test_version_history_tracking() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // Add initial preference
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode always"})],
                &agent_id,
            )
            .await
            .unwrap();

        // Search to get memory ID
        let results = store.search("dark mode", &agent_id, 10).await.unwrap();
        assert!(!results.is_empty());
        let mem_id = results[0].id.clone();

        // Update via the update API
        store
            .update(&mem_id, &agent_id, "I prefer light mode now")
            .await
            .unwrap();

        // The old memory should be soft-deleted, new one created
        // History for the new memory won't have the chain since update() uses delete+re-add
        // But add_with_decision UPDATE preserves history
        let count = store.count(&agent_id, None).unwrap();
        assert!(count >= 1);
    }

    #[tokio::test]
    async fn test_knowledge_graph_stores_relations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate.clone());

        // Manually store a relation
        let triples = vec![librefang_types::memory::RelationTriple {
            subject: "Alice".to_string(),
            subject_type: "person".to_string(),
            relation: "works_at".to_string(),
            object: "Acme Corp".to_string(),
            object_type: "organization".to_string(),
        }];
        store.store_relations(&triples);

        // Query the knowledge graph
        let matches = substrate
            .knowledge()
            .query_graph(GraphPattern {
                source: Some("alice".to_string()),
                relation: None,
                target: None,
                max_depth: 1,
            })
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].target.name, "Acme Corp");
    }

    #[tokio::test]
    async fn test_find_duplicates_semantic() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // Add two semantically similar but not identical memories
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer using dark mode in my editor"})],
                &agent_id,
            )
            .await
            .unwrap();
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "My name is John Smith"})],
                &agent_id,
            )
            .await
            .unwrap();

        // These should not be grouped as duplicates (different content)
        let groups = store.find_duplicates(&agent_id, None).await.unwrap();
        // No duplicate groups expected for distinct content
        for group in &groups {
            assert!(
                group.len() <= 1 || {
                    // If grouped, they should be genuinely similar
                    let a = &group[0].content.to_lowercase();
                    let b = &group[1].content.to_lowercase();
                    librefang_types::memory::text_similarity(a, b) > 0.5
                }
            );
        }
    }

    #[test]
    fn test_text_similarity() {
        use librefang_types::memory::text_similarity;

        // Identical
        assert!((text_similarity("hello world", "hello world") - 1.0).abs() < f32::EPSILON);

        // High overlap
        let sim = text_similarity(
            "i prefer dark mode in my editor",
            "i prefer dark mode in my terminal",
        );
        assert!(sim > 0.5);

        // Low overlap
        let sim = text_similarity("rust programming language", "cooking italian food");
        assert!(sim < 0.2);

        // Empty
        assert!((text_similarity("", "") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_entity_type_parsing() {
        assert_eq!(parse_entity_type("person"), EntityType::Person);
        assert_eq!(parse_entity_type("organization"), EntityType::Organization);
        assert_eq!(parse_entity_type("tool"), EntityType::Tool);
        assert_eq!(
            parse_entity_type("custom_thing"),
            EntityType::Custom("custom_thing".to_string())
        );
    }

    #[test]
    fn test_relation_type_parsing() {
        assert_eq!(parse_relation_type("works_at"), RelationType::WorksAt);
        assert_eq!(parse_relation_type("uses"), RelationType::Uses);
        assert_eq!(parse_relation_type("prefers"), RelationType::Uses);
        assert_eq!(
            parse_relation_type("custom_rel"),
            RelationType::Custom("custom_rel".to_string())
        );
    }

    #[tokio::test]
    async fn test_update_preserves_version_history() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // Add initial memory
        store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode"})],
                &agent_id,
            )
            .await
            .unwrap();

        let results = store.search("dark mode", &agent_id, 10).await.unwrap();
        assert!(!results.is_empty());
        let mem_id = results[0].id.clone();

        // Update it
        store
            .update(&mem_id, &agent_id, "I prefer light mode now")
            .await
            .unwrap();

        // Check version history
        let history = store.history(&mem_id).unwrap();
        assert_eq!(history.len(), 1);
        let prev = history[0].get("content").and_then(|v| v.as_str()).unwrap();
        assert!(prev.contains("dark mode"));

        // Update again
        store
            .update(&mem_id, &agent_id, "I prefer auto mode")
            .await
            .unwrap();

        let history2 = store.history(&mem_id).unwrap();
        assert_eq!(history2.len(), 2);
    }

    #[tokio::test]
    async fn test_default_extractor_extracts_relations() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        // "I work at" should extract a works_at relation
        let result = store
            .auto_memorize(
                &agent_id,
                &[serde_json::json!({
                    "role": "user",
                    "content": "I work at Google"
                })],
            )
            .await
            .unwrap();

        assert!(result.has_content);
        assert!(!result.relations.is_empty());
        assert_eq!(result.relations[0].relation, "works_at");
        assert_eq!(result.relations[0].object, "Google");
    }

    #[tokio::test]
    async fn test_default_extractor_i_use_pattern() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();

        let result = store
            .auto_memorize(
                &agent_id,
                &[serde_json::json!({
                    "role": "user",
                    "content": "I use vim for editing"
                })],
            )
            .await
            .unwrap();

        assert!(result.has_content);
        assert!(!result.relations.is_empty());
        assert_eq!(result.relations[0].relation, "uses");
        assert_eq!(result.relations[0].object, "Vim for editing");
    }

    #[tokio::test]
    async fn test_store_relations_dedup() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.1).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate.clone());

        let triples = vec![librefang_types::memory::RelationTriple {
            subject: "Bob".to_string(),
            subject_type: "person".to_string(),
            relation: "works_at".to_string(),
            object: "Acme".to_string(),
            object_type: "organization".to_string(),
        }];

        // Store twice
        store.store_relations(&triples);
        store.store_relations(&triples);

        // Should only have 1 relation (deduped)
        let matches = substrate
            .knowledge()
            .query_graph(GraphPattern {
                source: Some("bob".to_string()),
                relation: None,
                target: None,
                max_depth: 1,
            })
            .unwrap();
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn test_consolidate_merges_duplicates() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new().to_string();
        let agent_id_parsed =
            AgentId(uuid::Uuid::parse_str(&agent_id).unwrap_or_else(|_| uuid::Uuid::new_v4()));

        // Store two identical memories directly in semantic store (bypassing dedup)
        store
            .semantic
            .remember(
                agent_id_parsed,
                "User prefers dark mode in editor",
                MemorySource::Conversation,
                scopes::USER,
                HashMap::new(),
            )
            .unwrap();
        store
            .semantic
            .remember(
                agent_id_parsed,
                "User prefers dark mode in editor",
                MemorySource::Conversation,
                scopes::USER,
                HashMap::new(),
            )
            .unwrap();

        let count_before = store.count(&agent_id, None).unwrap();
        assert_eq!(count_before, 2);

        // find_duplicates should detect these via semantic store fallback
        let groups = store.find_duplicates(&agent_id, None).await.unwrap();
        assert!(!groups.is_empty());
        assert!(groups[0].len() >= 2);

        // Consolidate should merge them
        let merged = store.consolidate(&agent_id).await.unwrap();
        assert_eq!(merged, 1);

        let count_after = store.count(&agent_id, None).unwrap();
        assert_eq!(count_after, 1);
    }

    #[test]
    fn test_extract_entity_candidates() {
        let candidates = extract_entity_candidates("What does Alice know about Rust?");
        assert!(candidates.contains(&"Alice".to_string()));
        assert!(candidates.contains(&"Rust".to_string()));
        assert!(candidates.contains(&"alice".to_string())); // normalized
    }
}
