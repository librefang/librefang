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
    /// How this memory was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Confidence score (0.0 - 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// When this memory was last accessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed_at: Option<DateTime<Utc>>,
    /// How many times this memory has been accessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_count: Option<u64>,
    /// Which agent owns this memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
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
            source: None,
            confidence: None,
            accessed_at: None,
            access_count: None,
            agent_id: None,
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

    /// Create from a MemoryFragment.
    pub fn from_fragment(frag: MemoryFragment) -> Self {
        let level = MemoryLevel::from(frag.scope.as_str());
        let source_str = serde_json::to_value(&frag.source)
            .ok()
            .and_then(|v| v.as_str().map(String::from));
        Self {
            id: frag.id.to_string(),
            content: frag.content,
            level,
            category: frag
                .metadata
                .get("category")
                .and_then(|v| v.as_str())
                .map(String::from),
            source: source_str,
            confidence: Some(frag.confidence),
            accessed_at: Some(frag.accessed_at),
            access_count: Some(frag.access_count),
            agent_id: Some(frag.agent_id.to_string()),
            created_at: frag.created_at,
            metadata: frag.metadata,
        }
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
    /// Master toggle — when false, the entire proactive memory subsystem is disabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
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
    /// Similarity threshold for duplicate detection (0.0 - 1.0).
    /// When stored embeddings are available, uses vector cosine similarity
    /// (mem0-quality); otherwise falls back to Jaccard word overlap.
    /// Default: 0.5.
    pub duplicate_threshold: f32,
    /// Confidence decay rate per day. Memories lose confidence over time when
    /// not accessed, following exponential decay: `conf * e^(-rate * days)`.
    /// Default: 0.01 (very slow — takes ~70 days to halve).
    pub confidence_decay_rate: f64,
    /// Maximum number of memories allowed per agent. When adding new memories
    /// would exceed this cap, the oldest/lowest-confidence memories are evicted
    /// first. Default: 1000. Set to 0 to disable the cap.
    #[serde(default = "default_max_memories_per_agent")]
    pub max_memories_per_agent: usize,
}

fn default_true() -> bool {
    true
}

fn default_max_memories_per_agent() -> usize {
    1000
}

impl Default for ProactiveMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
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
            duplicate_threshold: 0.5,
            confidence_decay_rate: 0.01,
            max_memories_per_agent: 1000,
        }
    }
}

/// A relationship triple extracted from conversation (subject, relation, object).
///
/// Example: ("Alice", "works_at", "Acme Corp")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationTriple {
    /// Subject entity name.
    pub subject: String,
    /// Subject entity type (person, organization, project, etc.).
    pub subject_type: String,
    /// Relationship type.
    pub relation: String,
    /// Object entity name.
    pub object: String,
    /// Object entity type.
    pub object_type: String,
}

/// Result from LLM-powered memory extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// Extracted memory items.
    pub memories: Vec<MemoryItem>,
    /// Extracted relationship triples for knowledge graph.
    pub relations: Vec<RelationTriple>,
    /// Whether extraction found anything worth remembering.
    pub has_content: bool,
    /// Original query that triggered extraction.
    pub trigger: String,
    /// Detected conflicts where new info contradicts existing memories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<MemoryConflict>,
}

/// A detected conflict between old and new memory content.
///
/// This is surfaced when an Update action replaces old content with new content
/// that appears contradictory (low similarity + negation patterns), rather than
/// being a simple refinement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConflict {
    /// The previous memory content that was replaced.
    pub old_content: String,
    /// The new memory content that replaced it.
    pub new_content: String,
    /// The ID of the memory that was updated.
    pub memory_id: String,
}

/// Result from a single memory add operation, including the decision taken.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAddResult {
    /// The memory item that was stored (or the updated version).
    pub item: MemoryItem,
    /// What action was taken.
    pub action: MemoryAction,
    /// If updated, the ID of the old memory that was replaced.
    pub replaced_id: Option<String>,
    /// Detected conflict when an update appears contradictory rather than a refinement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<MemoryConflict>,
}

/// Action to take when a new memory conflicts with an existing one.
///
/// This is the core mem0 decision: when we extract a new memory, should we
/// add it as new, update an existing one, or skip it?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum MemoryAction {
    /// Store as a new memory (no conflict with existing).
    Add,
    /// Update an existing memory (new info supersedes old).
    Update {
        /// ID of the existing memory to replace.
        existing_id: String,
    },
    /// Skip — duplicate or subsumed by existing memory.
    Noop,
}

/// Trait for LLM-powered memory extraction and conflict resolution.
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

    /// Decide what to do with a new memory given existing similar memories.
    ///
    /// This is the core mem0 decision flow:
    /// - **Add**: No conflict, store as new memory.
    /// - **Update(id)**: New info supersedes existing memory `id`.
    /// - **Noop**: Duplicate or already subsumed by existing memory.
    ///
    /// Default implementation uses a tiered heuristic:
    /// 1. Substring containment (exact / superset / subset detection)
    /// 2. Vector cosine similarity (when stored embeddings are available —
    ///    matches mem0's dedup quality)
    /// 3. Jaccard word overlap (fallback when no embeddings)
    ///
    /// LLM-powered implementations should use the model to reason about conflicts.
    async fn decide_action(
        &self,
        new_memory: &MemoryItem,
        existing_memories: &[MemoryFragment],
    ) -> crate::error::LibreFangResult<MemoryAction> {
        let new_lower = new_memory.content.to_lowercase();

        // Track the best update candidate (highest similarity)
        let mut best_update: Option<(f32, String)> = None;

        for existing in existing_memories {
            let old_lower = existing.content.to_lowercase();

            // Exact match → skip
            if new_lower == old_lower {
                return Ok(MemoryAction::Noop);
            }

            // Existing already contains new info → skip
            if old_lower.contains(&new_lower) {
                return Ok(MemoryAction::Noop);
            }

            // New info contains old → update (new is more complete)
            if new_lower.contains(&old_lower) {
                return Ok(MemoryAction::Update {
                    existing_id: existing.id.to_string(),
                });
            }

            // Compute similarity: prefer vector cosine when the existing
            // memory has a stored embedding. This matches mem0's dedup
            // quality — cosine similarity on embeddings captures semantic
            // equivalence that Jaccard word overlap misses (e.g. synonyms,
            // rephrasing, different languages).
            let similarity = if let Some(ref emb) = existing.embedding {
                // Use the new memory's embedding from metadata if available
                // (stashed by add_with_decision when embedding driver is active).
                let new_emb = new_memory
                    .metadata
                    .get("_embedding")
                    .and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_f64().map(|f| f as f32))
                                .collect::<Vec<f32>>()
                        })
                    })
                    .filter(|e| !e.is_empty());
                match new_emb {
                    Some(ref ne) => cosine_similarity(ne, emb),
                    None => text_similarity(&new_lower, &old_lower),
                }
            } else {
                text_similarity(&new_lower, &old_lower)
            };

            // Very high similarity (≥ 0.95) → NOOP (near-duplicate)
            if similarity >= 0.95 {
                return Ok(MemoryAction::Noop);
            }

            // High similarity or same category → candidate for UPDATE
            let new_cat = new_memory.category.as_deref().unwrap_or("");
            let old_cat = existing
                .metadata
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let update_threshold = if !new_cat.is_empty() && new_cat == old_cat {
                0.5 // Lower threshold for same-category memories
            } else {
                0.6
            };

            if similarity > update_threshold
                && best_update
                    .as_ref()
                    .is_none_or(|(best_sim, _)| similarity > *best_sim)
            {
                best_update = Some((similarity, existing.id.to_string()));
            }
        }

        // Return the best update candidate, or ADD if none found
        if let Some((_, existing_id)) = best_update {
            Ok(MemoryAction::Update { existing_id })
        } else {
            Ok(MemoryAction::Add)
        }
    }

    /// Generate a search context from retrieved memories.
    ///
    /// Takes retrieved memory items and formats them for injection
    /// into the agent's context prompt.
    fn format_context(&self, memories: &[MemoryItem]) -> String;
}

/// Extract the phrase after a pattern, taking up to the first sentence boundary.
fn extract_after_pattern(text: &str, pattern: &str) -> Option<String> {
    let idx = text.find(pattern)?;
    let rest = &text[idx + pattern.len()..];
    // Take until sentence boundary or end
    let end = rest
        .find(['.', ',', '!', '?', ';', '\n'])
        .unwrap_or(rest.len());
    let phrase = rest[..end].trim();
    if phrase.is_empty() {
        None
    } else {
        Some(phrase.to_string())
    }
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Simple word-overlap similarity (Jaccard index on words).
pub fn text_similarity(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if words_a.is_empty() && words_b.is_empty() {
        return 0.0;
    }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

/// Compute cosine similarity between two embedding vectors.
///
/// Returns a value in `[-1.0, 1.0]` where `1.0` means identical direction.
/// Returns `0.0` for empty or mismatched-length vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Helper to push a memory item with extracted content (not the whole message).
fn push_memory(
    memories: &mut Vec<MemoryItem>,
    content: &str,
    level: MemoryLevel,
    category: &str,
    role: &str,
) {
    // Dedup: skip if we already extracted identical content
    if memories.iter().any(|m| m.content == content) {
        return;
    }
    let mut metadata = HashMap::new();
    metadata.insert("extracted_from".to_string(), serde_json::json!(role));
    memories.push(MemoryItem {
        id: Uuid::new_v4().to_string(),
        content: content.to_string(),
        level,
        category: Some(category.to_string()),
        metadata,
        created_at: Utc::now(),
        source: None,
        confidence: None,
        accessed_at: None,
        access_count: None,
        agent_id: None,
    });
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
        let mut relations = Vec::new();

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

            // ── Preference patterns ──
            // Store extracted phrase, not the whole message
            let pref_patterns: &[(&str, &str)] = &[
                ("i prefer ", "prefers"),
                ("i always ", "prefers"),
                ("i never ", "dislikes"),
                ("i dislike ", "dislikes"),
                ("my favorite ", "prefers"),
                ("i like to ", "prefers"),
                ("i don't like ", "dislikes"),
                ("i'd rather ", "prefers"),
                ("i want ", "prefers"),
                ("i need ", "prefers"),
            ];
            for &(pattern, rel) in pref_patterns {
                if let Some(phrase) = extract_after_pattern(&lower, pattern) {
                    let extracted = format!("User {pattern}{phrase}");
                    push_memory(
                        &mut memories,
                        &extracted,
                        MemoryLevel::User,
                        "user_preference",
                        role,
                    );
                    relations.push(RelationTriple {
                        subject: "User".to_string(),
                        subject_type: "person".to_string(),
                        relation: rel.to_string(),
                        object: capitalize_first(&phrase),
                        object_type: "concept".to_string(),
                    });
                }
            }

            // ── Identity / fact patterns ──
            let fact_patterns: &[(&str, &str, &str)] = &[
                ("my name is ", "is_named", "person"),
                ("i work at ", "works_at", "organization"),
                ("i'm working at ", "works_at", "organization"),
                ("i work on ", "works_on", "project"),
                ("i'm working on ", "works_on", "project"),
                ("i live in ", "located_in", "location"),
                ("i'm from ", "located_in", "location"),
                ("my job is ", "works_as", "concept"),
                ("i'm a ", "works_as", "concept"),
                ("i am a ", "works_as", "concept"),
                ("my team is ", "part_of", "organization"),
                ("my project is ", "works_on", "project"),
                ("our project ", "works_on", "project"),
                ("we're building ", "works_on", "project"),
                ("we are building ", "works_on", "project"),
                ("we're migrating to ", "uses", "tool"),
                ("we are migrating to ", "uses", "tool"),
            ];
            for &(pattern, rel, obj_type) in fact_patterns {
                if let Some(phrase) = extract_after_pattern(&lower, pattern) {
                    let extracted = format!("User {pattern}{phrase}");
                    push_memory(
                        &mut memories,
                        &extracted,
                        MemoryLevel::User,
                        "important_fact",
                        role,
                    );
                    relations.push(RelationTriple {
                        subject: "User".to_string(),
                        subject_type: "person".to_string(),
                        relation: rel.to_string(),
                        object: capitalize_first(&phrase),
                        object_type: obj_type.to_string(),
                    });
                }
            }

            // ── Tool/technology usage ──
            let tool_patterns: &[&str] = &[
                "i use ",
                "i'm using ",
                "i am using ",
                "we use ",
                "we're using ",
                "our stack includes ",
                "our tech stack is ",
                "i code in ",
                "i program in ",
                "i write in ",
                "i develop in ",
            ];
            for pattern in tool_patterns {
                if let Some(phrase) = extract_after_pattern(&lower, pattern) {
                    let extracted = format!("User {pattern}{phrase}");
                    push_memory(
                        &mut memories,
                        &extracted,
                        MemoryLevel::User,
                        "user_preference",
                        role,
                    );
                    relations.push(RelationTriple {
                        subject: "User".to_string(),
                        subject_type: "person".to_string(),
                        relation: "uses".to_string(),
                        object: capitalize_first(&phrase),
                        object_type: "tool".to_string(),
                    });
                }
            }

            // ── Task context (session-level) ──
            let task_patterns: &[&str] = &[
                "i'm trying to ",
                "i am trying to ",
                "i want to ",
                "i need to ",
                "the goal is to ",
                "we need to ",
                "the task is ",
                "the problem is ",
                "the issue is ",
                "the bug is ",
                "i'm debugging ",
                "i'm fixing ",
            ];
            for pattern in task_patterns {
                if let Some(phrase) = extract_after_pattern(&lower, pattern) {
                    // Only extract if the phrase is substantial (>10 chars)
                    if phrase.len() > 10 {
                        let extracted = format!("User {pattern}{phrase}");
                        push_memory(
                            &mut memories,
                            &extracted,
                            MemoryLevel::Session,
                            "task_context",
                            role,
                        );
                    }
                }
            }
        }

        Ok(ExtractionResult {
            has_content: !memories.is_empty() || !relations.is_empty(),
            memories,
            relations,
            trigger: "default_extractor".to_string(),
            conflicts: Vec::new(),
        })
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

// ---------------------------------------------------------------------------
// VectorStore trait — backend-agnostic vector storage abstraction
// ---------------------------------------------------------------------------

/// Search result from a vector store query.
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// The memory ID.
    pub id: String,
    /// The stored text payload.
    pub payload: String,
    /// Cosine similarity score (0.0–1.0).
    pub score: f32,
    /// Arbitrary metadata.
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Backend-agnostic vector store interface.
///
/// This trait abstracts the vector storage layer, enabling pluggable backends
/// (SQLite, Qdrant, Pinecone, Chroma, PgVector, Milvus, etc.).
///
/// The default implementation uses SQLite with BLOB-serialized embeddings and
/// in-process cosine similarity re-ranking. External backends can implement
/// this trait to offload ANN search to a dedicated vector database.
///
/// # Example (implementing for Qdrant)
///
/// ```ignore
/// struct QdrantVectorStore { client: QdrantClient, collection: String }
///
/// #[async_trait]
/// impl VectorStore for QdrantVectorStore {
///     async fn insert(&self, id: &str, embedding: &[f32], payload: &str,
///                     metadata: HashMap<String, serde_json::Value>) -> LibreFangResult<()> {
///         self.client.upsert_points(&self.collection, vec![point]).await?;
///         Ok(())
///     }
///     // ...
/// }
/// ```
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Insert or update a vector with its payload and metadata.
    async fn insert(
        &self,
        id: &str,
        embedding: &[f32],
        payload: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> crate::error::LibreFangResult<()>;

    /// Search for the `limit` nearest vectors to `query_embedding`.
    ///
    /// The returned results are ordered by descending similarity score.
    /// Implementations should apply the provided `filter` (agent, scope, etc.).
    async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> crate::error::LibreFangResult<Vec<VectorSearchResult>>;

    /// Delete a vector by ID.
    async fn delete(&self, id: &str) -> crate::error::LibreFangResult<()>;

    /// Retrieve stored embeddings for a batch of IDs.
    ///
    /// Returns a map of `id -> embedding`. IDs without stored embeddings
    /// are omitted from the result.
    async fn get_embeddings(
        &self,
        ids: &[&str],
    ) -> crate::error::LibreFangResult<HashMap<String, Vec<f32>>>;

    /// Return the name of this backend (e.g. "sqlite", "qdrant", "pinecone").
    fn backend_name(&self) -> &str;
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

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }
}
