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
use chrono::Utc;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    memory_scope_allows_recall, text_similarity, DefaultMemoryExtractor, Entity, EntityType,
    ExtractionResult, GraphPattern, MemoryAction, MemoryAddResult, MemoryConflict, MemoryExtractor,
    MemoryFilter, MemoryFragment, MemoryId, MemoryItem, MemoryLevel, MemorySource, ProactiveMemory,
    ProactiveMemoryConfig, ProactiveMemoryHooks, Relation, RelationTriple, RelationType,
    CHAT_SCOPE_METADATA_KEY,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tracing::Instrument;

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
    config: Arc<RwLock<ProactiveMemoryConfig>>,
    /// Memory extractor for LLM-powered extraction
    extractor: Arc<dyn MemoryExtractor>,
    /// Optional embedding driver for vector similarity search.
    /// When present, memories are stored with embeddings and search uses cosine similarity.
    /// When absent, falls back to LIKE text matching.
    embedding: Option<Arc<dyn EmbeddingFn>>,
    /// Per-agent counters for auto-consolidation. Each entry tracks
    /// both the running count (consolidate fires at
    /// `AUTO_CONSOLIDATE_EVERY = 10`) and the last-touched timestamp,
    /// so the maintenance sweep can evict slots that have been idle
    /// for longer than `STALE_COUNTER_IDLE_WINDOW_HOURS` regardless of
    /// their count. Pre-fix this was `HashMap<String, u32>`, which
    /// forced the sweep to use a count threshold and starved
    /// slow-burn agents (1 call per maintenance window or less)
    /// whose counter never climbed past the floor.
    consolidation_counters: Arc<Mutex<HashMap<String, CounterEntry>>>,
    /// Timestamp of the last confidence decay run (at most once per hour).
    last_decay_run: Arc<Mutex<Option<chrono::DateTime<Utc>>>>,
    /// Timestamp of the last session TTL cleanup run (at most once per hour).
    last_cleanup_run: Arc<Mutex<Option<chrono::DateTime<Utc>>>>,
    /// Timestamp of the last `consolidation_counters` idle-window
    /// prune (at most once per hour — review-followup #3). Without
    /// this gate the prune ran on every `maybe_run_maintenance`
    /// call, which is reachable from `search` / `auto_retrieve` /
    /// `consolidate` and fires many times per second on a busy
    /// daemon. The `HashMap::retain` itself is microseconds on
    /// thousand-entry maps, but rate-limiting matches the existing
    /// `last_decay_run` / `last_cleanup_run` pattern so all three
    /// maintenance sub-tasks consume the same scheduling budget.
    last_counter_prune: Arc<Mutex<Option<chrono::DateTime<Utc>>>>,
}

impl Clone for ProactiveMemoryStore {
    fn clone(&self) -> Self {
        Self {
            substrate: self.substrate.clone(),
            structured: self.structured.clone(),
            semantic: self.semantic.clone(),
            knowledge: self.knowledge.clone(),
            config: self.config.clone(),
            extractor: self.extractor.clone(),
            embedding: self.embedding.clone(),
            consolidation_counters: Arc::clone(&self.consolidation_counters),
            last_decay_run: Arc::clone(&self.last_decay_run),
            last_cleanup_run: Arc::clone(&self.last_cleanup_run),
            last_counter_prune: Arc::clone(&self.last_counter_prune),
        }
    }
}

impl ProactiveMemoryStore {
    /// Create a new proactive memory store with default extractor.
    pub fn new(substrate: Arc<MemorySubstrate>, config: ProactiveMemoryConfig) -> Self {
        let pool = substrate.pool();
        let knowledge = substrate.knowledge().clone();
        Self {
            structured: StructuredStore::new(pool.clone()),
            semantic: SemanticStore::new(pool),
            knowledge,
            substrate,
            config: Arc::new(RwLock::new(config)),
            extractor: Arc::new(DefaultMemoryExtractor),
            embedding: None,
            consolidation_counters: Arc::new(Mutex::new(HashMap::new())),
            last_decay_run: Arc::new(Mutex::new(None)),
            last_cleanup_run: Arc::new(Mutex::new(None)),
            last_counter_prune: Arc::new(Mutex::new(None)),
        }
    }

    /// Create a new proactive memory store with custom extractor.
    pub fn with_extractor(
        substrate: Arc<MemorySubstrate>,
        config: ProactiveMemoryConfig,
        extractor: Arc<dyn MemoryExtractor>,
    ) -> Self {
        let pool = substrate.pool();
        let knowledge = substrate.knowledge().clone();
        Self {
            structured: StructuredStore::new(pool.clone()),
            semantic: SemanticStore::new(pool),
            knowledge,
            substrate,
            config: Arc::new(RwLock::new(config)),
            extractor,
            embedding: None,
            consolidation_counters: Arc::new(Mutex::new(HashMap::new())),
            last_decay_run: Arc::new(Mutex::new(None)),
            last_cleanup_run: Arc::new(Mutex::new(None)),
            last_counter_prune: Arc::new(Mutex::new(None)),
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

    /// Get a snapshot of the current config.
    pub fn config(&self) -> ProactiveMemoryConfig {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Hot-swap the runtime config (called on config reload).
    pub fn update_config(&self, new_config: ProactiveMemoryConfig) {
        let mut guard = self.config.write().unwrap_or_else(|e| e.into_inner());
        *guard = new_config;
    }

    /// Decay confidence scores for memories that haven't been accessed recently.
    ///
    /// For each memory not accessed in the last day, applies:
    ///   `effective_rate = decay_rate / boost`, where
    ///   `boost = min(1 + log2(access_count), MAX_BOOST)`.
    ///   `new_confidence = current_confidence * e^(-effective_rate * days_since_access)`
    ///
    /// Popular memories decay *slower* (rate divided by boost) instead of being
    /// multiplied back up. The previous formula multiplied by `boost` and
    /// clamped to `[0,1]`, which made any memory with ≥ 2 accesses effectively
    /// immortal: `0.99 * 2.0 → clamp → 1.0` every tick. The fix preserves the
    /// "popular memories stick around longer" intent while keeping decay
    /// strictly monotonic (per tick, confidence never increases).
    ///
    /// Runs the decay pass immediately on every call — there is no internal
    /// throttle. The once-per-hour cadence is enforced by the periodic
    /// maintenance scheduler (see `run_periodic_maintenance`), so a direct
    /// call (e.g. a manual `/decay` endpoint or a test) decays right away.
    pub fn decay_confidence(&self) -> LibreFangResult<()> {
        let decay_rate = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .confidence_decay_rate;
        if decay_rate <= 0.0 {
            return Ok(());
        }

        let now = Utc::now();
        let one_day_ago = now - chrono::Duration::days(1);

        // Fetch all non-deleted memories that haven't been accessed in > 1 day
        let conn = self
            .semantic
            .pool()
            .get()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, confidence, accessed_at, access_count
                 FROM memories
                 WHERE deleted = 0 AND accessed_at < ?1",
            )
            .map_err(LibreFangError::memory)?;

        let rows: Vec<(String, f64, String, i64)> = stmt
            .query_map(rusqlite::params![one_day_ago.to_rfc3339()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(LibreFangError::memory)?
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!("Failed to read memory row during confidence decay: {}", e);
                    None
                }
            })
            .collect();

        for (id, current_confidence, accessed_str, access_count) in &rows {
            let accessed_at = match chrono::DateTime::parse_from_rfc3339(accessed_str) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse accessed_at '{}' for memory {}, skipping decay: {}",
                        accessed_str,
                        id,
                        e
                    );
                    continue;
                }
            };

            let days_since_access = (now - accessed_at).num_seconds() as f64 / 86400.0;
            if days_since_access <= 0.0 {
                continue;
            }

            // Popular memories decay slower (rate divided by boost), not
            // boosted back up post-decay (which would let access_count >= 2
            // saturate any decay back to 1.0 and freeze the memory forever).
            // MAX_BOOST = 4.0 means an extremely popular memory still decays
            // at 1/4 the configured rate — slow, but never zero.
            const MAX_BOOST: f64 = 4.0;
            let count = (*access_count).max(1) as f64;
            let boost = (1.0 + count.log2()).min(MAX_BOOST);
            let effective_rate = decay_rate / boost;
            let new_confidence =
                (current_confidence * (-effective_rate * days_since_access).exp()).clamp(0.0, 1.0);

            conn.execute(
                "UPDATE memories SET confidence = ?1 WHERE id = ?2",
                rusqlite::params![new_confidence, id],
            )
            .map_err(LibreFangError::memory)?;
        }

        if !rows.is_empty() {
            tracing::debug!(
                "Confidence decay applied to {} memories (rate={})",
                rows.len(),
                decay_rate
            );
        }

        Ok(())
    }

    /// Run confidence decay if at least one hour has elapsed since the last run.
    fn maybe_decay_confidence(&self) {
        let now = Utc::now();
        // Hold the lock for the entire check-and-update to avoid TOCTOU race
        let mut guard = self
            .last_decay_run
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let should_run = match *guard {
            Some(last) => (now - last) >= chrono::Duration::hours(1),
            None => true,
        };

        if should_run {
            // Update timestamp before releasing the lock so concurrent callers won't
            // also decide to run decay.
            *guard = Some(now);
            // Drop the lock before the potentially slow decay_confidence() call
            drop(guard);

            if let Err(e) = self.decay_confidence() {
                tracing::debug!("Confidence decay failed (non-fatal): {}", e);
            }
        }
    }

    /// Delete session-level memories older than `session_ttl_hours` across ALL agents.
    ///
    /// Only deletes "session" level memories — user and agent level memories are
    /// persistent by nature and are not affected. Returns the count of deleted items.
    ///
    /// This is the global variant of `cleanup_expired_sessions` (which is per-agent).
    pub fn cleanup_expired(&self) -> LibreFangResult<u64> {
        let ttl_hours = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .session_ttl_hours;
        if ttl_hours == 0 {
            return Ok(0);
        }

        let cutoff = Utc::now() - chrono::Duration::hours(ttl_hours as i64);

        // Soft-delete expired session memories in the semantic store
        // (across all agents). The KV mirror (`memory:*` keys in
        // `structured`) is no longer written, so there is nothing to
        // clean up there.
        let count = self
            .semantic
            .forget_session_older_than_global(scopes::SESSION, cutoff)?;

        if count > 0 {
            tracing::debug!(
                "Session TTL cleanup: deleted {} expired session memories (ttl={}h)",
                count,
                ttl_hours
            );
        }

        Ok(count)
    }

    /// Run session TTL cleanup if at least one hour has elapsed since the last run.
    fn maybe_cleanup_expired(&self) {
        let now = Utc::now();
        // Hold the lock for the entire check-and-update to avoid TOCTOU race
        let mut guard = self
            .last_cleanup_run
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let should_run = match *guard {
            Some(last) => (now - last) >= chrono::Duration::hours(1),
            None => true,
        };

        if should_run {
            // Update timestamp before releasing the lock so concurrent callers won't
            // also decide to run cleanup.
            *guard = Some(now);
            // Drop the lock before the potentially slow cleanup_expired() call
            drop(guard);

            if let Err(e) = self.cleanup_expired() {
                tracing::debug!("Session TTL cleanup failed (non-fatal): {}", e);
            }
        }
    }

    /// Run periodic maintenance tasks (decay + session cleanup) if enough time has elapsed.
    ///
    /// This is safe to call frequently — each sub-task is rate-limited to at most once per hour.
    /// Called from search, auto_retrieve, and consolidate to ensure maintenance happens
    /// regardless of which code path is exercised.
    fn maybe_run_maintenance(&self) {
        self.maybe_decay_confidence();
        self.maybe_cleanup_expired();
        self.maybe_prune_counters();
    }

    /// Prune idle entries from `consolidation_counters` if at least
    /// one hour has elapsed since the last sweep.
    ///
    /// The pre-fix sweep ran on every `maybe_run_maintenance` call,
    /// which is reachable from `search` / `auto_retrieve` /
    /// `consolidate` and can fire many times per second on a busy
    /// daemon. The `HashMap::retain` itself is microseconds on
    /// thousand-entry maps, but rate-limiting matches the existing
    /// `last_decay_run` / `last_cleanup_run` pattern so all three
    /// maintenance sub-tasks consume the same scheduling budget
    /// (review-followup #3).
    fn maybe_prune_counters(&self) {
        let now = Utc::now();
        let mut guard = self
            .last_counter_prune
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let should_run = match *guard {
            Some(last) => (now - last) >= chrono::Duration::hours(1),
            None => true,
        };
        if !should_run {
            return;
        }
        *guard = Some(now);
        drop(guard);

        // M11: keep `consolidation_counters` from holding stale entries.
        //
        // The auto_memorize counter is incremented every turn and
        // *removed* when it hits `AUTO_CONSOLIDATE_EVERY` (10). Agents
        // that get fewer than 10 calls per their lifetime never trigger
        // the removal, so a long-running daemon with many short-lived
        // chat-burst agents accumulates a tail of forever-stuck-at-1..9
        // entries — small (~50 bytes each) but unbounded.
        //
        // The pre-fix sweep only fired once the map crossed 1000 entries
        // (truncate-to-500 by count DESC), which both delays cleanup
        // until the leak is observable AND deletes the highest-count
        // entries — exactly the agents about to fire a real consolidate.
        //
        // Review-followup B: the sweep uses an *idle-window* check
        // (`last_touched` older than `STALE_COUNTER_IDLE_WINDOW_HOURS`)
        // rather than a count threshold. A count threshold (followup
        // #2's `count >= STALE_COUNTER_FLOOR`) mitigated the original
        // wholesale reset but still starved slow-burn agents whose
        // fire rate stayed below the floor — once-an-hour agents
        // never climbed past 2 because the prune ran every
        // maintenance tick and reset them. Last-touched fixes this:
        // an active slot, however slow, is preserved as long as it's
        // been touched within the window; a truly idle slot is
        // reclaimed within ~2 hours of going quiet.
        if let Ok(mut counters) = self.consolidation_counters.lock() {
            let cutoff = now - chrono::Duration::hours(STALE_COUNTER_IDLE_WINDOW_HOURS);
            let before = counters.len();
            counters.retain(|_, entry| entry.last_touched >= cutoff);
            let dropped = before - counters.len();
            if dropped > 0 {
                tracing::debug!(
                    dropped,
                    remaining = counters.len(),
                    idle_window_hours = STALE_COUNTER_IDLE_WINDOW_HOURS,
                    "Pruned idle consolidation counters"
                );
            }
        }
    }

    /// Export all memories for an agent as a flat JSON-serializable list.
    pub fn export_all(&self, agent_id: &str) -> LibreFangResult<Vec<MemoryExportItem>> {
        let aid = Self::parse_agent_id(agent_id)?;
        let filter = Some(MemoryFilter::agent(aid));

        // Fetch all non-deleted memories for this agent (up to 10k).
        // Read-only: exporting must not bump access_count / accessed_at.
        let frags = self.semantic.recall_readonly("", 10_000, filter)?;

        let items = frags
            .into_iter()
            .map(|frag| {
                let level = MemoryLevel::from(frag.scope.as_str());
                let category = frag
                    .metadata
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let updated_at = frag
                    .metadata
                    .get("updated_at")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                MemoryExportItem {
                    content: frag.content,
                    level: format!("{:?}", level),
                    category,
                    confidence: frag.confidence as f64,
                    created_at: frag.created_at.to_rfc3339(),
                    updated_at,
                    metadata: serde_json::to_value(&frag.metadata).unwrap_or_else(|e| {
                        tracing::warn!("Failed to serialize metadata during export: {e}");
                        serde_json::Value::Object(Default::default())
                    }),
                }
            })
            .collect();

        Ok(items)
    }

    /// Import memories from a flat JSON list. Returns count of successfully imported items.
    pub async fn import_memories(
        &self,
        agent_id: &str,
        items: Vec<MemoryExportItem>,
    ) -> LibreFangResult<usize> {
        let aid = Self::parse_agent_id(agent_id)?;
        let mut imported = 0usize;

        for item in items {
            let level = MemoryLevel::from(item.level.as_str());
            let scope = level.scope_str();

            // Skip only near-verbatim duplicates on bulk import. The
            // threshold here (0.95) is intentionally *higher* than the
            // configured `duplicate_threshold` (which defaults to 0.85
            // and gates extraction-time UPDATE decisions) — a higher
            // value means LESS gets dropped, MORE imports succeed. Bulk
            // import is an explicit operator action so we want a high
            // bar before silently swallowing a row; a paraphrase the
            // operator legitimately wants imported as a distinct memory
            // gets through and can be consolidated later via the
            // on-demand `/api/memory/agents/{id}/consolidate` route.
            //
            // Review-followup #10: the previous comment described 0.95
            // as "stricter than extraction-time dedup", which is true
            // only relative to the extraction-time 0.85 baseline (a
            // higher bar to call something a duplicate ⇒ stricter dedup
            // gating ⇒ more imports get through). Pre-fix this was 0.9,
            // so 0.95 is also slightly *more permissive* for is_duplicate
            // judgements than the original value. Both readings are
            // consistent; the intent is "let almost everything through
            // and let the on-demand consolidator clean up later".
            let filter = Some(MemoryFilter::agent(aid));
            let existing = self.semantic.recall(&item.content, 5, filter)?;
            let is_duplicate = existing.iter().any(|frag| {
                let sim =
                    text_similarity(&item.content.to_lowercase(), &frag.content.to_lowercase());
                sim >= 0.95
            });
            if is_duplicate {
                tracing::debug!(
                    "Skipping duplicate import: {}",
                    truncate_for_log(&item.content, 80)
                );
                continue;
            }

            let mut metadata: HashMap<String, serde_json::Value> = if item.metadata.is_object() {
                serde_json::from_value(item.metadata).unwrap_or_default()
            } else {
                HashMap::new()
            };

            if !item.category.is_empty() {
                metadata.insert("category".to_string(), serde_json::json!(item.category));
            }
            metadata.insert("imported".to_string(), serde_json::json!(true));
            if let Some(ref updated_at) = item.updated_at {
                metadata.insert(
                    "original_updated_at".to_string(),
                    serde_json::json!(updated_at),
                );
            }

            // Generate embedding if driver available
            let embedding = if let Some(ref emb) = self.embedding {
                emb.embed_one(&item.content).await.ok()
            } else {
                None
            };

            let _mem_id = self.semantic.remember_with_embedding(
                aid,
                &item.content,
                MemorySource::System,
                scope,
                metadata,
                embedding.as_deref(),
                None,
                None,
                Default::default(),
            )?;

            imported += 1;
        }

        // Enforce per-agent memory cap after import
        if imported > 0 {
            if let Err(e) = self.evict_if_over_cap(aid, 0) {
                tracing::warn!("import_memories eviction check failed: {}", e);
            }
        }

        tracing::info!("Imported {} memories for agent {}", imported, agent_id);
        Ok(imported)
    }

    /// Parse user_id string into AgentId.
    fn parse_agent_id(user_id: &str) -> LibreFangResult<AgentId> {
        user_id
            .parse()
            .map_err(|e| LibreFangError::Internal(format!("Failed to parse user_id: {}", e)))
    }

    /// Retrieve memory items for an agent, optionally filtered by level
    /// and/or category.
    ///
    /// Reads from the semantic store (the authoritative source).
    /// Pre-fix this read from `structured.list_kv("memory:*")`, but that
    /// mirror was a denormalised cache written best-effort *after* the
    /// semantic insert — a failure of the second write (e.g. tx
    /// contention, schema drift, or a process restart between the two
    /// non-transactional inserts) left rows visible to `search()` (which
    /// reads semantic) but invisible to `list()` / `get()` (which read
    /// KV), producing the long-standing "agent forgets things" /
    /// "dashboard shows memories the agent can't see" split-brain.
    /// Review-followup #5: the KV mirror writes have been deleted entirely
    /// — the read path was already on semantic, and keeping dead writes
    /// just leaked disk and risked future divergence regressions. The KV
    /// `memory:*` namespace is now unused; any legacy entries left over
    /// from older installs are silently ignored (no reader looks at them).
    fn retrieve_memory_items(
        &self,
        agent_id: AgentId,
        level: Option<MemoryLevel>,
        category: Option<&str>,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        // Single-pass scan of all non-deleted memories for the agent.
        // Bound at 10k to match the dashboard `list_all` cap; agents that
        // legitimately have more are paginated at the route layer.
        const RECALL_CAP: usize = 10_000;
        let mut filter = MemoryFilter::agent(agent_id);
        if let Some(target_level) = level {
            filter.scope = Some(target_level.scope_str().to_string());
        }
        // Read-only listing: a polled list/get must not bump access_count /
        // accessed_at, or the decay engine would never see these memories as
        // idle (#5839).
        let frags = self
            .semantic
            .recall_readonly("", RECALL_CAP, Some(filter))?;

        let mut items: Vec<MemoryItem> = frags
            .into_iter()
            .map(MemoryItem::from_fragment)
            .filter(|item| match category {
                Some(target) => item.category.as_deref() == Some(target),
                None => true,
            })
            .collect();

        // Newest first — preserves the prior contract for callers that
        // expect chronological-desc ordering.
        items.sort_by_key(|b| std::cmp::Reverse(b.created_at));

        Ok(items)
    }

    /// Core mem0 decision flow: search for similar memories, decide action, execute.
    ///
    /// Returns `None` if the decision was NOOP (skip duplicate).
    async fn add_with_decision(
        &self,
        agent_id: AgentId,
        item: &MemoryItem,
        peer_id: Option<&str>,
        chat_scope: Option<&str>,
    ) -> LibreFangResult<Option<MemoryAddResult>> {
        // Generate embedding for the new memory (if driver available)
        let query_embedding = if let Some(ref emb) = self.embedding {
            emb.embed_one(&item.content).await.ok()
        } else {
            None
        };

        // Search for similar existing memories. The substrate filter is still
        // `(agent_id, peer_id)`-scoped because the storage layer's `recall`
        // API doesn't filter on `chat_scope` metadata directly; we
        // post-filter below to mirror the read-side semantics from
        // `auto_retrieve`. The candidate fetch is widened from 5 → 20 when
        // a `chat_scope` is active so the post-filter has enough headroom
        // to keep ~5 same-scope (or chat-agnostic) candidates after pruning
        // — mirrors the 4× inflation used by `auto_retrieve` /
        // `setup_recalled_memories` for the same reason.
        //
        // #5227 follow-up (P1, second pass): without the post-filter, the
        // extractor saw memories tagged for ANOTHER chat as dedupe
        // candidates and could NOOP against them — making the new
        // `chat_scope`-stamped row never be written. The subsequent
        // `auto_retrieve` (which DOES filter by scope) would then return
        // nothing for the active chat, silently losing the fact. Or worse,
        // an UPDATE decision would mutate the OTHER chat's memory with the
        // current chat's content.
        let chat_scope_active = chat_scope.map(|s| !s.trim().is_empty()).unwrap_or(false);
        let fetch_limit = if chat_scope_active { 20 } else { 5 };
        let filter = Some({
            let mut f = MemoryFilter::agent(agent_id);
            f.peer_id = peer_id.map(String::from);
            f
        });
        let mut existing = if let Some(ref qe) = query_embedding {
            self.semantic.recall_with_embedding(
                &item.content,
                fetch_limit,
                filter.clone(),
                Some(qe),
            )?
        } else {
            // M13: union the result sets across the top distinctive
            // keywords instead of LIKE-substring matching on just the
            // single longest one. Each keyword is a separate recall
            // (recall takes one LIKE pattern); we dedup by memory id
            // and cap at fetch_limit at the end so the caller sees the
            // same shape as the embedding path.
            //
            // Review-followup #4 / #E: this is up to 4 SQLite roundtrips
            // per insertion (one per keyword), versus the pre-fix 1.
            // Operators running with an embedding driver never hit this
            // branch (the `if let Some(qe)` above takes the vector
            // path). Operators on the no-embedding fallback — CI, local
            // dev without an embedding API, resource-constrained
            // deployments — eat the round-trip cost on writes. The
            // `content` column is an unindexed substring scan either
            // way, so the marginal cost is proportional to row count,
            // not catastrophic in absolute terms. Loop short-circuits
            // as soon as the union hits `fetch_limit`: for agents with
            // a sizeable memory store the first (longest) keyword often
            // already fills the slate, dropping back to a single
            // query; small / fresh stores still pay the full 4× since
            // no individual keyword has enough matches to short-circuit.
            let keywords = extract_search_keywords(&item.content);
            let mut acc: Vec<MemoryFragment> = Vec::new();
            let mut seen_ids: std::collections::HashSet<MemoryId> =
                std::collections::HashSet::new();
            for kw in &keywords {
                let rows = self.semantic.recall(kw, fetch_limit, filter.clone())?;
                for row in rows {
                    if seen_ids.insert(row.id) {
                        acc.push(row);
                    }
                    if acc.len() >= fetch_limit {
                        break;
                    }
                }
                if acc.len() >= fetch_limit {
                    break;
                }
            }
            // Last-resort fallback (content yielded no distinctive
            // keywords at all — e.g. all stop words or single short
            // word). Substring-match the raw content so a near-verbatim
            // duplicate is still detectable.
            if acc.is_empty() {
                acc = self.semantic.recall(&item.content, fetch_limit, filter)?;
            }
            acc
        };

        // Apply the cross-chat isolation filter to the candidate set
        // BEFORE the extractor decides ADD/UPDATE/NOOP. Three classes pass
        // through (same predicate used by `auto_retrieve`):
        //   1. `MemoryLevel::User` rows — explicitly cross-chat stable
        //      facts. The new item being a session-level extraction is
        //      still allowed to dedupe against a user-level memory of the
        //      same content (UPDATE-in-place keeps the more durable level).
        //   2. Memories with no `chat_scope` tag — legacy / chat-agnostic.
        //   3. Memories whose stamped `chat_scope` matches the active one.
        // Anything else is a foreign-chat memory; ignoring it here lets the
        // extractor see an empty/smaller candidate list and pick ADD, so a
        // dedicated row lands for the current chat.
        //
        // When the new item itself is `MemoryLevel::User`, the filter is
        // intentionally skipped — user-level facts are global and SHOULD
        // dedupe against any prior copy regardless of which chat first
        // produced them.
        //
        // Forward-compat note: the only non-User levels the current
        // extractor emits go through this filter, so cross-chat
        // dedupe collisions are impossible today. If a future
        // extractor starts emitting `MemoryLevel::Session` or
        // `MemoryLevel::Workspace` facts in batches (rather than one
        // at a time per agent invocation), the same post-filter
        // pattern needs to apply at the new emission site, or two
        // identical session-level facts produced in different chats
        // could dedupe across them. Search for callers of
        // `add_with_decision` when extending memory levels.
        if chat_scope_active && item.level != MemoryLevel::User {
            let want = chat_scope.unwrap();
            existing.retain(|frag| memory_scope_allows_recall(&frag.scope, &frag.metadata, want));
        }
        // Truncate back to the extractor's expected window so we don't
        // hand it 20 candidates when it was tuned for 5.
        existing.truncate(5);

        // Stash per-call inputs the default `decide_action` heuristic
        // needs (the query embedding for cosine similarity, and the
        // M14 update thresholds for the UPDATE / NOOP gate) in a
        // throwaway clone's metadata. Keeps the trait signature stable
        // without forcing every implementor to take a config handle —
        // the LLM-backed extractor falls back to the default heuristic
        // on driver failure, so it sees the same metadata.
        let mut enriched_item = item.clone();
        if let Some(ref qe) = query_embedding {
            let emb_json: Vec<serde_json::Value> =
                qe.iter().map(|&v| serde_json::json!(v)).collect();
            enriched_item
                .metadata
                .insert("_embedding".to_string(), serde_json::json!(emb_json));
        }
        // Callers must not pre-populate the `_update_threshold_*` keys
        // — they're a private channel between `add_with_decision` and
        // the default `decide_action` heuristic. The `item.metadata`
        // we cloned into `enriched_item` is the caller's untouched
        // input; a hit here is a contract violation worth catching in
        // dev. Production builds are still safe: even if a caller
        // leaks the key, the unconditional `insert` below overwrites
        // it before `decide_action` reads it.
        debug_assert!(
            !item.metadata.contains_key("_update_threshold_same_cat"),
            "callers must not pre-populate `_update_threshold_same_cat` — it is private to add_with_decision"
        );
        debug_assert!(
            !item.metadata.contains_key("_update_threshold_cross_cat"),
            "callers must not pre-populate `_update_threshold_cross_cat` — it is private to add_with_decision"
        );
        let (same_cat_thresh, cross_cat_thresh) = {
            let cfg = self.config.read().unwrap_or_else(|e| e.into_inner());
            (
                cfg.update_threshold_same_category,
                cfg.update_threshold_cross_category,
            )
        };
        enriched_item.metadata.insert(
            "_update_threshold_same_cat".to_string(),
            serde_json::json!(same_cat_thresh),
        );
        enriched_item.metadata.insert(
            "_update_threshold_cross_cat".to_string(),
            serde_json::json!(cross_cat_thresh),
        );
        let decision_item = &enriched_item;

        // Ask the extractor to decide: ADD, UPDATE, or NOOP
        let action = self
            .extractor
            .decide_action(decision_item, &existing)
            .await?;

        // Strip all of `add_with_decision`'s private stash keys from
        // the enriched clone the moment they stop being useful. See
        // `strip_private_stash_keys` for the full rationale.
        strip_private_stash_keys(&mut enriched_item.metadata);

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
                let _mem_id = self.semantic.remember_with_embedding_and_peer(
                    agent_id,
                    &item.content,
                    MemorySource::Conversation,
                    item.level.scope_str(),
                    metadata,
                    query_embedding.as_deref(),
                    None,
                    None,
                    Default::default(),
                    peer_id,
                )?;
                tracing::debug!(
                    "Memory decision: ADD new: {}",
                    truncate_for_log(&item.content, 80)
                );
                Ok(Some(MemoryAddResult {
                    item: item.clone(),
                    action: MemoryAction::Add,
                    replaced_id: None,
                    conflict: None,
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

                // Conflict detection: check if the update looks contradictory
                // rather than a simple refinement.
                let conflict = detect_memory_conflict(&old_content, &item.content, existing_id);

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
                if conflict.is_some() {
                    metadata.insert("conflict_detected".to_string(), serde_json::json!(true));
                }

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

                if conflict.is_some() {
                    tracing::info!(
                        "Memory conflict detected: UPDATE {} (old: '{}' -> new: '{}')",
                        existing_id,
                        truncate_for_log(&old_content, 60),
                        truncate_for_log(&item.content, 60)
                    );
                } else {
                    tracing::debug!(
                        "Memory decision: UPDATE {} -> {}",
                        existing_id,
                        truncate_for_log(&item.content, 80)
                    );
                }
                Ok(Some(MemoryAddResult {
                    item: item.clone(),
                    action: action.clone(),
                    replaced_id: Some(existing_id.clone()),
                    conflict,
                }))
            }
        }
    }

    /// Evict the lowest-confidence memories for an agent if adding `new_count`
    /// memories would exceed the configured `max_memories_per_agent` cap.
    ///
    /// Does nothing when the cap is 0 (disabled) or when there is still room.
    fn evict_if_over_cap(&self, agent_id: AgentId, new_count: usize) -> LibreFangResult<()> {
        let max = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .max_memories_per_agent;
        if max == 0 {
            return Ok(()); // cap disabled
        }

        let current = self.semantic.count(agent_id, None)? as usize;
        let total_after = current + new_count;
        if total_after <= max {
            return Ok(()); // still within budget
        }

        let to_evict_raw = total_after - max;
        // Cap eviction at the number of existing memories — we can't evict more
        // than what actually exists. If new_count alone exceeds the cap, log a warning.
        let to_evict = to_evict_raw.min(current);
        if to_evict < to_evict_raw {
            tracing::warn!(
                agent_id = %agent_id,
                new_count = new_count,
                max = max,
                current_count = current,
                "New memory batch alone exceeds per-agent cap; \
                 cap will be exceeded even after evicting all existing memories"
            );
        }
        tracing::debug!(
            agent_id = %agent_id,
            current_count = current,
            new_count = new_count,
            max = max,
            evicting = to_evict,
            "Per-agent memory cap exceeded, evicting lowest-confidence memories"
        );

        let ids = self.semantic.lowest_confidence(agent_id, to_evict)?;
        for id in &ids {
            self.semantic.forget(*id)?;
        }
        tracing::debug!(
            agent_id = %agent_id,
            evicted = ids.len(),
            "Memory cap eviction complete"
        );
        Ok(())
    }

    /// Store extracted relation triples into the knowledge graph.
    ///
    /// Deduplicates: skips if an identical (source, relation, target) already exists.
    pub fn store_relations(&self, triples: &[RelationTriple], agent_id: &str) {
        for triple in triples {
            let source_type = parse_entity_type(&triple.subject_type);
            let target_type = parse_entity_type(&triple.object_type);

            // Upsert source entity
            let source_id = match self.knowledge.add_entity(
                Entity {
                    id: normalize_entity_id(&triple.subject),
                    entity_type: source_type,
                    name: triple.subject.clone(),
                    properties: HashMap::new(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
                agent_id,
            ) {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("Failed to add entity '{}': {}", triple.subject, e);
                    continue;
                }
            };

            // Upsert target entity
            let target_id = match self.knowledge.add_entity(
                Entity {
                    id: normalize_entity_id(&triple.object),
                    entity_type: target_type,
                    name: triple.object.clone(),
                    properties: HashMap::new(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
                agent_id,
            ) {
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
                    if let Err(e) = self.knowledge.add_relation(
                        Relation {
                            source: source_id,
                            relation: relation_type,
                            target: target_id,
                            properties: HashMap::new(),
                            confidence: 0.9,
                            created_at: chrono::Utc::now(),
                        },
                        agent_id,
                    ) {
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
    /// names appear in the memory content. Honors the configured
    /// `format_context_max_chars` (H4 review-followup #8).
    pub fn format_context_with_query(&self, memories: &[MemoryItem], query: &str) -> String {
        let max_chars = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .format_context_max_chars;
        let mut context = librefang_types::memory::format_memories_with_budget(memories, max_chars);

        // Append knowledge graph context if relevant
        if let Some(graph_ctx) = self.graph_context(query) {
            context.push('\n');
            context.push_str(&graph_ctx);
        }

        context
    }

    /// Format retrieved memories into a context string for prompt injection.
    pub fn format_context(&self, memories: &[MemoryItem]) -> String {
        let max_chars = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .format_context_max_chars;
        librefang_types::memory::format_memories_with_budget(memories, max_chars)
    }

    /// Get memory statistics for a user/agent.
    ///
    /// Uses efficient SQL COUNT queries instead of loading all items.
    pub async fn stats(&self, user_id: &str) -> LibreFangResult<MemoryStats> {
        let agent_id = Self::parse_agent_id(user_id)?;

        let user_count = self.semantic.count(agent_id, Some(scopes::USER))? as usize;
        let session_count = self.semantic.count(agent_id, Some(scopes::SESSION))? as usize;
        let agent_count = self.semantic.count(agent_id, Some(scopes::AGENT))? as usize;
        let total_all = self.semantic.count(agent_id, None)? as usize;
        let total = std::cmp::max(total_all, user_count + session_count + agent_count);

        // Use SQL GROUP BY to count categories without loading all items into memory
        let categories = self.semantic.count_by_category(Some(agent_id))?;

        let cfg = self.config.read().unwrap_or_else(|e| e.into_inner());
        Ok(MemoryStats {
            total,
            user_count,
            session_count,
            agent_count,
            categories,
            enabled: cfg.enabled,
            auto_memorize_enabled: cfg.auto_memorize,
            auto_retrieve_enabled: cfg.auto_retrieve,
            llm_extraction: cfg.extraction_model.is_some(),
        })
    }

    /// Get memory statistics across ALL agents.
    ///
    /// Used by the dashboard to show global memory stats.
    pub async fn stats_all(&self) -> LibreFangResult<MemoryStats> {
        let user_count = self.semantic.count_all(Some(scopes::USER))? as usize;
        let session_count = self.semantic.count_all(Some(scopes::SESSION))? as usize;
        let agent_count = self.semantic.count_all(Some(scopes::AGENT))? as usize;
        // Include all scopes (e.g. "episodic") in total count
        let total_all = self.semantic.count_all(None)? as usize;
        let total = std::cmp::max(total_all, user_count + session_count + agent_count);

        // Use SQL GROUP BY to count categories without loading all items into memory
        let categories = self.semantic.count_by_category(None)?;

        let cfg = self.config.read().unwrap_or_else(|e| e.into_inner());
        Ok(MemoryStats {
            total,
            user_count,
            session_count,
            agent_count,
            categories,
            enabled: cfg.enabled,
            auto_memorize_enabled: cfg.auto_memorize,
            auto_retrieve_enabled: cfg.auto_retrieve,
            llm_extraction: cfg.extraction_model.is_some(),
        })
    }

    /// List memories across ALL agents, optionally filtered by category.
    ///
    /// Used by the dashboard to show all memories without agent scoping.
    pub async fn list_all(&self, category: Option<&str>) -> LibreFangResult<Vec<MemoryItem>> {
        // Use semantic recall with no agent filter to get all memories.
        // Limit to 10000 to avoid unbounded queries; increase if needed.
        // Read-only: this dashboard listing must not bump access_count /
        // accessed_at (#5839).
        let results = self.semantic.recall_readonly("", 10_000, None)?;

        let items: Vec<MemoryItem> = results
            .into_iter()
            .filter(|frag| {
                if let Some(target_cat) = category {
                    frag.metadata.get("category").and_then(|v| v.as_str()) == Some(target_cat)
                } else {
                    true
                }
            })
            .map(MemoryItem::from_fragment)
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
            .map(MemoryItem::from_fragment)
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

        // Clean up knowledge graph entities and relations for this agent.
        // The KV mirror (`memory:*` keys in `structured`) is no longer
        // written, so there is nothing to clean up there.
        if let Err(e) = self.knowledge.delete_by_agent(user_id) {
            tracing::warn!("Failed to clean up knowledge graph for agent {user_id}: {e}");
        }

        Ok(count)
    }

    /// Clear memories at a specific level for a user/agent.
    ///
    /// Useful for clearing session memories while preserving user preferences.
    pub fn clear_level(&self, user_id: &str, level: MemoryLevel) -> LibreFangResult<u64> {
        let agent_id = Self::parse_agent_id(user_id)?;
        let count = self.semantic.forget_by_scope(agent_id, level.scope_str())?;
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
        self.maybe_run_maintenance();
        // Validate the user_id parses to a valid AgentId before doing any
        // work (legacy guard — keeps mismatched callers from spending an
        // O(n²) find_duplicates pass on a malformed id).
        let _agent_id = Self::parse_agent_id(user_id)?;
        let groups = self.find_duplicates(user_id, None).await?;
        let mut merged_count = 0u64;

        for group in groups {
            if group.len() < 2 {
                continue;
            }

            // Keep the most recently created memory as the "winner".
            // Note: MemoryItem doesn't expose accessed_at; created_at is the best
            // available signal here. The newest duplicate typically has the most
            // up-to-date content from the latest extraction.
            let Some(winner) = group.iter().max_by_key(|m| m.created_at).cloned() else {
                continue;
            };

            // Soft-delete all others
            for item in &group {
                if item.id != winner.id {
                    if let Ok(uuid) = uuid::Uuid::parse_str(&item.id) {
                        let mid = MemoryId(uuid);
                        if self.semantic.forget(mid).is_ok() {
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

    /// Query the knowledge graph for relations matching a pattern.
    ///
    /// Wraps `KnowledgeStore::query_graph()` for external API access.
    pub fn query_relations(
        &self,
        pattern: GraphPattern,
    ) -> LibreFangResult<Vec<librefang_types::memory::GraphMatch>> {
        self.knowledge.query_graph(pattern)
    }

    /// Find duplicate/near-duplicate memories for a user/agent.
    ///
    /// Uses a tiered similarity strategy (mem0-style):
    /// 1. **Vector cosine similarity** (when stored embeddings are available) —
    ///    the most accurate method, matching mem0's dedup quality.
    /// 2. **Substring containment** — catches exact and super/sub-string matches.
    /// 3. **Jaccard word overlap** — fallback when no embeddings are stored.
    ///
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
            all_items = frags.into_iter().map(MemoryItem::from_fragment).collect();
        }

        // Limit to 100 most recent items to avoid O(n^2) blowup
        if all_items.len() > 100 {
            all_items.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            all_items.truncate(100);
        }

        // Load stored embeddings for all items (batch query).
        // This enables vector cosine similarity — the same dedup method
        // used by mem0 when a vector store is configured.
        let id_strings: Vec<String> = all_items.iter().map(|m| m.id.clone()).collect();
        let id_refs: Vec<&str> = id_strings.iter().map(|s| s.as_str()).collect();
        let embeddings = self
            .semantic
            .get_embeddings_batch(&id_refs)
            .unwrap_or_default();
        let has_embeddings = !embeddings.is_empty();
        if has_embeddings {
            tracing::debug!(
                "find_duplicates: loaded {} stored embeddings for {} items — using vector cosine similarity",
                embeddings.len(),
                all_items.len()
            );
        }

        let threshold = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .duplicate_threshold;

        let mut used = vec![false; all_items.len()];
        let mut groups: Vec<Vec<MemoryItem>> = Vec::new();

        for i in 0..all_items.len() {
            if used[i] {
                continue;
            }
            used[i] = true; // Mark seed so it cannot be absorbed into a later group
            let mut group = vec![all_items[i].clone()];
            let a_lower = all_items[i].content.to_lowercase();
            let emb_a = embeddings.get(&all_items[i].id);

            for j in (i + 1)..all_items.len() {
                if used[j] {
                    continue;
                }
                let b_lower = all_items[j].content.to_lowercase();

                // Check substring containment (fast path)
                let is_substring =
                    a_lower.contains(&b_lower) || b_lower.contains(&a_lower) || a_lower == b_lower;

                if is_substring {
                    group.push(all_items[j].clone());
                    used[j] = true;
                    continue;
                }

                // Tiered similarity: prefer vector cosine when both have embeddings,
                // fall back to Jaccard word overlap otherwise.
                let emb_b = embeddings.get(&all_items[j].id);
                let similarity = match (emb_a, emb_b) {
                    (Some(a), Some(b)) => {
                        // Vector cosine similarity (mem0-quality dedup).
                        // Fall back to text similarity when vectors are not
                        // comparable; treating that case as 0.0 would silently
                        // suppress legitimate dedup candidates (#3536).
                        librefang_types::memory::cosine_similarity(a, b).unwrap_or_else(|| {
                            librefang_types::memory::text_similarity(&a_lower, &b_lower)
                        })
                    }
                    _ => {
                        // Jaccard word overlap fallback
                        librefang_types::memory::text_similarity(&a_lower, &b_lower)
                    }
                };

                if similarity > threshold {
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

    // ----- RBAC M3 — namespace ACL helpers (#3054) -----

    /// Search wrapper that gates access to the `proactive` namespace and
    /// applies PII redaction before returning fragments to the caller.
    ///
    /// `user_id` here is still the agent identifier the existing
    /// `ProactiveMemory::search` expects — the guard's *user* ACL is a
    /// separate concept resolved by the kernel from the inbound message's
    /// channel binding. Returns
    /// [`LibreFangError::AuthDenied`](librefang_types::error::LibreFangError::AuthDenied)
    /// when the guard refuses the read.
    pub async fn search_with_guard(
        &self,
        query: &str,
        user_id: &str,
        limit: usize,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        let mut items =
            <Self as librefang_types::memory::ProactiveMemory>::search(self, query, user_id, limit)
                .await?;
        guard.redact_all(&mut items);
        Ok(items)
    }

    /// Delete wrapper that gates access to the `proactive` namespace and
    /// honours `delete_allowed`. Mirrors the
    /// [`ProactiveMemory::delete`](librefang_types::memory::ProactiveMemory::delete)
    /// signature on success.
    pub async fn delete_with_guard(
        &self,
        memory_id: &str,
        user_id: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<bool> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_delete("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        <Self as librefang_types::memory::ProactiveMemory>::delete(self, memory_id, user_id).await
    }

    /// Add wrapper that gates writes to the `proactive` namespace.
    pub async fn add_with_guard(
        &self,
        messages: &[serde_json::Value],
        user_id: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_write("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        <Self as librefang_types::memory::ProactiveMemory>::add(self, messages, user_id).await
    }

    /// Cross-agent dashboard search wrapper. Same gating as
    /// [`Self::search_with_guard`] (read access to `proactive` + PII
    /// redaction). Used by the API `/memory/search` endpoint.
    pub async fn search_all_with_guard(
        &self,
        query: &str,
        limit: usize,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        let mut items = self.search_all(query, limit).await?;
        guard.redact_all(&mut items);
        Ok(items)
    }

    /// Cross-agent dashboard listing wrapper. Same gating as
    /// [`Self::search_with_guard`].
    pub async fn list_all_with_guard(
        &self,
        category: Option<&str>,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        let mut items = self.list_all(category).await?;
        guard.redact_all(&mut items);
        Ok(items)
    }

    /// Per-user list wrapper used by `/memory/user/{user_id}` and the
    /// dashboard. Reads memory items for `user_id` and applies PII
    /// redaction when the guard forbids it.
    pub async fn get_with_guard(
        &self,
        user_id: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        let mut items =
            <Self as librefang_types::memory::ProactiveMemory>::get(self, user_id).await?;
        guard.redact_all(&mut items);
        Ok(items)
    }

    /// Per-agent list wrapper.
    pub async fn list_with_guard(
        &self,
        agent_id: &str,
        category: Option<&str>,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<librefang_types::memory::MemoryItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_read("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        let mut items =
            <Self as librefang_types::memory::ProactiveMemory>::list(self, agent_id, category)
                .await?;
        guard.redact_all(&mut items);
        Ok(items)
    }

    /// Reset wrapper. Requires both `write` and `delete` capability on the
    /// `proactive` namespace — wiping every memory for an agent is a write
    /// effect AND a destructive operation.
    pub fn reset_with_guard(
        &self,
        user_id: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<u64> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_delete("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        self.reset(user_id)
    }

    /// Clear-by-level wrapper. Same delete gate as [`Self::reset_with_guard`].
    pub fn clear_level_with_guard(
        &self,
        user_id: &str,
        level: librefang_types::memory::MemoryLevel,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<u64> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_delete("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        self.clear_level(user_id, level)
    }

    /// Export wrapper. Requires the dedicated `export` capability, which is
    /// stricter than ordinary reads — exports bypass PII redaction and dump
    /// raw rows, so a Viewer must not be able to call this even when reads
    /// are allowed.
    pub fn export_all_with_guard(
        &self,
        agent_id: &str,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<Vec<MemoryExportItem>> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_export("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        self.export_all(agent_id)
    }

    /// Import wrapper. Bulk-write into the proactive namespace.
    pub async fn import_memories_with_guard(
        &self,
        agent_id: &str,
        items: Vec<MemoryExportItem>,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<usize> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_write("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        self.import_memories(agent_id, items).await
    }

    /// Manual decay trigger wrapper. Treated as a destructive op (decay
    /// permanently mutates confidence scores across every agent), so it
    /// requires the `delete` capability.
    pub fn decay_confidence_with_guard(
        &self,
        guard: &crate::namespace_acl::MemoryNamespaceGuard,
    ) -> librefang_types::error::LibreFangResult<()> {
        if let crate::namespace_acl::NamespaceGate::Deny(reason) = guard.check_delete("proactive") {
            return Err(librefang_types::error::LibreFangError::AuthDenied(reason));
        }
        self.decay_confidence()
    }
}

/// A flat, JSON-serializable representation of a memory for import/export.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryExportItem {
    pub content: String,
    pub level: String,
    pub category: String,
    pub confidence: f64,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub metadata: serde_json::Value,
}

/// Memory usage statistics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryStats {
    pub total: usize,
    pub user_count: usize,
    pub session_count: usize,
    pub agent_count: usize,
    pub categories: HashMap<String, usize>,
    /// Whether the proactive memory subsystem is enabled.
    pub enabled: bool,
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
        self.maybe_run_maintenance();
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
            .map(MemoryItem::from_fragment)
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
    /// 2. Enforce per-agent memory cap — evict lowest-confidence memories if needed
    /// 3. For each extracted memory, search for similar existing memories
    /// 4. Let extractor decide: ADD (new), UPDATE (replace old), or NOOP (skip)
    /// 5. Execute the decision
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

        let categories = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .extract_categories
            .clone();

        // Step 1: Extract structured memories
        let extraction = self
            .extractor
            .extract_memories(messages, &categories)
            .await?;
        if !extraction.has_content {
            // No extraction signal → no memory. The previous behavior was to
            // stash the raw concatenated message content as a session memory
            // with no category, which (a) flooded the store with verbatim
            // transcripts, (b) produced the `category=null` rows that
            // littered the dashboard, and (c) was the dominant source of
            // duplicate "memories" because every chatty turn dumped the
            // whole conversation. Callers that want raw-content capture
            // should use [`ProactiveMemoryStore::add_with_level`] explicitly.
            tracing::debug!(
                ?agent_id,
                message_count = messages.len(),
                "Extractor returned no signal — skipping (no raw-transcript fallback)"
            );
            return Ok(Vec::new());
        }

        // Step 2-4: For each extracted memory, decide and execute
        let mut results = Vec::new();
        for item in &extraction.memories {
            let result = self.add_with_decision(agent_id, item, None, None).await?;
            if let Some(r) = result {
                results.push(r.item);
            }
        }

        // Step 5: Enforce per-agent memory cap AFTER the decision loop.
        // Memories are already stored, so pass new_count=0 — the current DB count
        // already includes the ADDs, and eviction will trim only the true excess.
        self.evict_if_over_cap(agent_id, 0)?;

        // Step 6: Store extracted relations in knowledge graph
        if !extraction.relations.is_empty() {
            self.store_relations(&extraction.relations, user_id);
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
        let _ = mem_id; // semantic insert is the authoritative write; no KV mirror.

        // Enforce per-agent memory cap
        if let Err(e) = self.evict_if_over_cap(agent_id, 0) {
            tracing::warn!("add_with_level eviction check failed: {}", e);
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
        let agent_id_parsed = Self::parse_agent_id(user_id)?;

        // Check if the memory exists and belongs to this user before deleting
        let frag = match self.semantic.get_by_id(mid, false)? {
            Some(f) => f,
            None => return Ok(false),
        };

        // Verify ownership: memory must belong to the requesting user
        if frag.agent_id != agent_id_parsed {
            return Ok(false);
        }

        self.semantic.forget(mid)?;
        let _ = user_id; // KV mirror removed; semantic.forget is the only write.
        Ok(true)
    }

    /// Update a memory's content in-place, preserving version history.
    async fn update(&self, memory_id: &str, user_id: &str, content: &str) -> LibreFangResult<bool> {
        let uuid = uuid::Uuid::parse_str(memory_id)
            .map_err(|e| LibreFangError::Internal(format!("Invalid memory_id: {e}")))?;
        let mid = MemoryId(uuid);
        let agent_id_parsed = Self::parse_agent_id(user_id)?;

        // Get old memory for version history
        let old_frag = match self.semantic.get_by_id(mid, false)? {
            Some(f) => f,
            None => return Ok(false),
        };

        // Verify ownership: memory must belong to the requesting user
        if old_frag.agent_id != agent_id_parsed {
            return Ok(false);
        }

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

        // Re-embed the updated content so vector search stays accurate
        if let Some(ref embed_fn) = self.embedding {
            match embed_fn.embed_one(content).await {
                Ok(vec) => {
                    if let Err(e) = self.semantic.update_embedding(mid, &vec) {
                        tracing::warn!("Failed to update embedding for memory {memory_id}: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to compute embedding for updated memory {memory_id}: {e}"
                    );
                }
            }
        }

        let _ = user_id; // KV mirror removed; semantic.update_content is authoritative.
        Ok(true)
    }
}

/// Private-stash keys that `add_with_decision` writes onto the
/// `enriched_item` clone for the duration of one `decide_action`
/// call. NOT a public extension surface — operators MUST NOT set
/// these on memories they pass to `add()`.
const ADD_WITH_DECISION_PRIVATE_STASH_KEYS: &[&str] = &[
    "_update_threshold_same_cat",
    "_update_threshold_cross_cat",
    "_embedding",
];

/// Strip every key in [`ADD_WITH_DECISION_PRIVATE_STASH_KEYS`] from
/// the given metadata map.
///
/// Used by `add_with_decision` after `decide_action` returns, so the
/// enriched clone goes back to looking like the caller's original
/// input. Today both ADD and UPDATE branches build their stored
/// metadata from `item.metadata` (the caller's input, NOT
/// `enriched_item.metadata`), so the stash keys never reach the
/// `metadata` column with or without this strip. The strip is
/// defensive insurance for the next refactorer: if someone repoints
/// either branch at `enriched_item.metadata` to thread extraction
/// metadata through, the pollution gets caught at the strip site
/// rather than silently persisted.
///
/// Extracted into a standalone function (review-followup B) so the
/// strip behaviour itself is directly unit-testable — the
/// integration test
/// `add_with_decision_does_not_leak_private_stash_keys_to_stored_metadata`
/// only catches a coordinated two-step regression (strip removed
/// AND branch repointed at enriched), because the current ADD path
/// bypasses `enriched_item.metadata` entirely.
fn strip_private_stash_keys(metadata: &mut HashMap<String, serde_json::Value>) {
    for key in ADD_WITH_DECISION_PRIVATE_STASH_KEYS {
        metadata.remove(*key);
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
///
/// Returns up to 4 distinctive keywords ordered longest-first. Pre-fix
/// (M13) this collapsed the four to the single longest keyword, which
/// (a) wasted the work of the stop-word filter on the other three and
/// (b) gave the dedup-candidate scan a single LIKE substring to match
/// against, frequently a generic word like "analysis" that matched too
/// many unrelated rows OR a long compound term that matched none. The
/// caller iterates and unions per-keyword results, which is closer to
/// what a real FTS index would do for the no-embedding fallback path.
fn extract_search_keywords(content: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "i", "a", "an", "the", "is", "am", "are", "was", "were", "be", "been", "being", "have",
        "has", "had", "do", "does", "did", "will", "would", "could", "should", "may", "might",
        "can", "shall", "for", "and", "but", "or", "nor", "not", "so", "yet", "at", "by", "in",
        "of", "on", "to", "up", "it", "my", "me", "we", "he", "she", "they", "this", "that",
        "with", "from", "all", "very", "just", "also", "than",
    ];

    let mut words: Vec<String> = content
        .split_whitespace()
        .filter_map(|w| {
            let lower = w.to_lowercase();
            if lower.len() > 2 && !STOP_WORDS.contains(&lower.as_str()) {
                Some(lower)
            } else {
                None
            }
        })
        .collect();
    // Dedup while preserving first-occurrence order.
    let mut seen = std::collections::HashSet::new();
    words.retain(|w| seen.insert(w.clone()));
    // Longest-first ordering — long words tend to be more distinctive
    // than short common ones. Stable sort preserves original order
    // within a length bucket.
    words.sort_by_key(|b| std::cmp::Reverse(b.len()));
    words.truncate(4);
    words
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

/// Number of `auto_memorize` calls per agent between consolidate ticks.
/// Each call increments `consolidation_counters[agent]`; when the entry
/// hits this floor a background `consolidate` task is spawned and the
/// entry is removed.
const AUTO_CONSOLIDATE_EVERY: u32 = 10;

/// Idle window (in hours) before a `consolidation_counters` entry is
/// considered stale and reclaimed by the maintenance sweep (M11). Any
/// entry whose `last_touched` is older than this gets dropped on the
/// next prune tick.
///
/// Set to 2 hours, which is `> 2× the rate-limit window` of the
/// maintenance ticks themselves (decay + cleanup + counter prune are
/// each gated at "at most once per hour" — see
/// `maybe_decay_confidence` / `maybe_cleanup_expired` /
/// `maybe_prune_counters`). The double-window margin guarantees a
/// slow-burn agent that fires `auto_memorize` once between two
/// consecutive maintenance ticks keeps its slot.
///
/// Effective reclaim latency for a truly idle slot: **~2–3 hours**.
/// The lower bound is the configured idle window. The upper bound
/// adds up to one prune rate-limit period (1 hour) because the prune
/// itself only runs ≤ once per hour — so an entry that goes quiet
/// just after a prune fires has to wait the full hour for the next
/// prune chance before its idle window can be evaluated
/// (review-followup A — the round-3 commit added the prune
/// rate-limit but kept the "reclaimed within ~2 hours" phrasing,
/// which was accurate only before the rate-limit landed).
///
/// Round-2 followup B history: the previous fix (round-1 followup
/// #2) used a *count threshold* (`AUTO_CONSOLIDATE_EVERY / 4`),
/// which mitigated but did not solve the slow-burn case: any agent
/// firing ≤ 1× per maintenance window would still be reset before
/// climbing past the floor. Using the last-touched timestamp
/// instead closes the gap by making "active" the real condition
/// for keeping the slot, decoupled from how fast the counter climbs.
///
/// Stored as `i64` rather than a `chrono::Duration` constant
/// (review-followup #6): `chrono::Duration::hours` is a `const fn`
/// at the currently pinned `chrono` version but the const-ness is
/// not part of the stable API across `0.4.x` minors, so a future
/// lockfile bump could silently break the build. `i64` hours +
/// `chrono::Duration::hours(...)` at the call site stays valid
/// regardless.
const STALE_COUNTER_IDLE_WINDOW_HOURS: i64 = 2;

/// Per-agent auto-consolidation state. `count` ticks up to
/// `AUTO_CONSOLIDATE_EVERY`; `last_touched` records when the counter
/// was last incremented so the maintenance sweep can distinguish
/// "actively accumulating" slots from "idle but never reached
/// threshold" slots.
#[derive(Debug, Clone, Copy)]
struct CounterEntry {
    count: u32,
    last_touched: chrono::DateTime<Utc>,
}

/// Negation/contradiction words that suggest content is contradictory, not a refinement.
const NEGATION_WORDS: &[&str] = &[
    "not",
    "don't",
    "dont",
    "doesn't",
    "doesnt",
    "never",
    "no longer",
    "changed",
    "switched",
    "stopped",
    "quit",
    "instead",
    "rather than",
    "replaced",
    "moved from",
    "moved to",
    "no more",
];

/// Detect whether a memory update looks like a contradiction rather than a refinement.
///
/// Returns `Some(MemoryConflict)` when:
/// 1. The old and new content have low Jaccard word-overlap similarity (< 0.3), AND
/// 2. The new content contains negation/change words suggesting contradiction.
///
/// This heuristic avoids flagging simple expansions ("likes Python" -> "likes Python and Rust")
/// while catching real contradictions ("likes Python" -> "switched to Rust, no longer uses Python").
fn detect_memory_conflict(
    old_content: &str,
    new_content: &str,
    memory_id: &str,
) -> Option<MemoryConflict> {
    if old_content.is_empty() || new_content.is_empty() {
        return None;
    }

    let old_lower = old_content.to_lowercase();
    let new_lower = new_content.to_lowercase();

    // Check Jaccard similarity — low overlap suggests contradiction
    let similarity = text_similarity(&old_lower, &new_lower);
    if similarity >= 0.3 {
        return None; // Enough overlap: likely a refinement, not a contradiction
    }

    // Check for negation/contradiction words in the new content
    let has_negation = NEGATION_WORDS.iter().any(|word| new_lower.contains(word));

    if has_negation {
        Some(MemoryConflict {
            old_content: old_content.to_string(),
            new_content: new_content.to_string(),
            memory_id: memory_id.to_string(),
        })
    } else {
        None
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
        peer_id: Option<&str>,
        chat_scope: Option<&str>,
    ) -> LibreFangResult<ExtractionResult> {
        let cfg = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if !cfg.enabled || !cfg.auto_memorize || conversation.is_empty() {
            return Ok(ExtractionResult {
                memories: Vec::new(),
                relations: Vec::new(),
                has_content: false,
                trigger: "auto_memorize_disabled".to_string(),
                conflicts: Vec::new(),
            });
        }

        let agent_id = Self::parse_agent_id(user_id)?;

        // Extract memories using the configured extractor. Use the
        // agent-id variant: when the extractor has a kernel handle wired
        // (LlmMemoryExtractor::with_forked_kernel), this routes the LLM
        // call through a forked agent turn so the cache key matches the
        // parent conversation — Anthropic hits cache on (system + tools
        // + messages). The rule-based DefaultMemoryExtractor ignores
        // agent_id via the trait's default forwarding, so nothing changes
        // for kernels running without an LLM extractor.
        let extraction_result = self
            .extractor
            .extract_memories_with_agent_id(
                conversation,
                &agent_id.to_string(),
                &cfg.extract_categories,
            )
            .await?;

        // Apply decision flow for each extracted memory
        let mut stored_memories = Vec::new();
        let mut conflicts = Vec::new();
        for item in &extraction_result.memories {
            // Filter by configured extract_categories (if non-empty)
            if !cfg.extract_categories.is_empty() {
                let cat = item.category.as_deref().unwrap_or("");
                if !cat.is_empty() && !cfg.extract_categories.iter().any(|c| c == cat) {
                    continue;
                }
            }

            // Filter by extraction_threshold: skip low-confidence extractions.
            // Confidence is stored in metadata by the LLM extractor; rule-based
            // extractor defaults to 1.0 (always passes).
            let confidence = item
                .metadata
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0) as f32;
            if confidence < cfg.extraction_threshold {
                continue;
            }

            // Tag with auto_memorize metadata
            let mut enriched = item.clone();
            enriched
                .metadata
                .insert("auto_memorize".to_string(), serde_json::json!(true));
            // #5227: stamp the originating chat scope so a memory
            // extracted from one chat (e.g. a WhatsApp group) cannot be
            // recalled into another chat (e.g. a DM with the same peer)
            // by `auto_retrieve`. User-level memories that should cross
            // chats (e.g. `MemoryLevel::User` set by the LLM extractor or
            // stable preferences) deliberately get their scope stamped
            // here too — the recall filter exempts `MemoryLevel::User`
            // separately, so the tag is harmless for them and crucial
            // for `MemoryLevel::Session` (the default).
            if let Some(scope) = chat_scope {
                if !scope.is_empty() {
                    enriched.metadata.insert(
                        CHAT_SCOPE_METADATA_KEY.to_string(),
                        serde_json::Value::String(scope.to_string()),
                    );
                }
            }

            match self
                .add_with_decision(agent_id, &enriched, peer_id, chat_scope)
                .await
            {
                Ok(Some(result)) => {
                    if let Some(conflict) = result.conflict {
                        conflicts.push(conflict);
                    }
                    stored_memories.push(result.item);
                }
                Ok(None) => {} // NOOP
                Err(e) => {
                    tracing::warn!("auto_memorize decision failed for memory: {}", e);
                }
            }
        }

        // Enforce per-agent memory cap after storing new memories
        if !stored_memories.is_empty() {
            if let Err(e) = self.evict_if_over_cap(agent_id, 0) {
                tracing::warn!("auto_memorize eviction check failed: {}", e);
            }
        }

        // Store extracted relations in knowledge graph
        if !extraction_result.relations.is_empty() {
            self.store_relations(&extraction_result.relations, user_id);
        }

        // Auto-consolidation: merge duplicates every AUTO_CONSOLIDATE_EVERY
        // auto_memorize calls per agent. Detached to a background task
        // by H6 so this branch never blocks the agent's hot path.
        let should_consolidate = {
            let mut counters = self
                .consolidation_counters
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let now = Utc::now();
            let entry = counters.entry(user_id.to_string()).or_insert(CounterEntry {
                count: 0,
                last_touched: now,
            });
            entry.count += 1;
            entry.last_touched = now;
            if entry.count >= AUTO_CONSOLIDATE_EVERY {
                // Remove the entry to prevent unbounded HashMap growth
                counters.remove(user_id);
                true
            } else {
                false
            }
        };
        if should_consolidate {
            // H6: consolidate is O(n²) over up to 100 candidates and runs
            // a SQLite transaction; awaiting it inside the agent's
            // auto_memorize hot path blocks every subsequent turn for the
            // duration. Detach to a background task so the agent keeps
            // going while the dedup pass finishes asynchronously.
            //
            // Failure here is non-fatal (the next tick re-tries). The
            // detached future borrows nothing from `self` thanks to the
            // manual Clone impl on ProactiveMemoryStore (all inner state
            // is Arc'd).
            //
            // Review-followup #6: wrap in a `tracing::Instrument` span so
            // (a) any panic in `consolidate` is attributed in tracing
            // output rather than disappearing silently, and (b) operators
            // can grep `task = "auto_consolidate"` to find the work this
            // detached future is doing.
            let store = self.clone();
            let agent = user_id.to_string();
            let span = tracing::info_span!(
                "auto_consolidate",
                task = "auto_consolidate",
                agent = %agent
            );
            tokio::spawn(
                async move {
                    match store.consolidate(&agent).await {
                        Ok(merged) if merged > 0 => {
                            tracing::info!(
                                merged,
                                "Auto-consolidation (background): merged duplicate memories"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::debug!(
                                error = %e,
                                "Auto-consolidation failed (non-fatal, background)"
                            );
                        }
                    }
                }
                .instrument(span),
            );
        }

        Ok(ExtractionResult {
            has_content: !stored_memories.is_empty(),
            memories: stored_memories,
            relations: extraction_result.relations,
            trigger: extraction_result.trigger,
            conflicts,
        })
    }

    /// Proactively retrieve relevant context before agent execution.
    ///
    /// Also performs session TTL cleanup if configured.
    async fn auto_retrieve(
        &self,
        user_id: &str,
        query: &str,
        peer_id: Option<&str>,
        chat_scope: Option<&str>,
    ) -> LibreFangResult<Vec<MemoryItem>> {
        let cfg = self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if !cfg.enabled || !cfg.auto_retrieve {
            return Ok(Vec::new());
        }

        // Run periodic maintenance (decay + session TTL cleanup), rate-limited
        self.maybe_run_maintenance();

        let agent_id = Self::parse_agent_id(user_id)?;

        // Create filter for this agent, scoped to peer if present
        let filter = Some({
            let mut f = MemoryFilter::agent(agent_id);
            f.peer_id = peer_id.map(String::from);
            f
        });

        // #5227: when an active chat_scope is supplied, fetch a wider
        // candidate set so the post-filter (which drops memories tagged
        // for a *different* chat) is unlikely to starve the caller of
        // results. The semantic store ranks/orders candidates the same
        // way regardless of LIMIT, so an enlarged fetch followed by a
        // local truncate preserves ranking semantics. A 4× factor with a
        // 50-row floor handles the worst observed mixing ratio in the
        // bug reproducer (DM and group turns interleaving within minutes
        // for the same agent+peer).
        let chat_scope_active = chat_scope.map(str::trim).is_some_and(|s| !s.is_empty());
        let fetch_limit = if chat_scope_active {
            (cfg.max_retrieve * 4).max(50)
        } else {
            cfg.max_retrieve
        };

        // Search across all memory levels — use vector search if available
        let results = if let Some(ref emb) = self.embedding {
            if let Ok(qe) = emb.embed_one(query).await {
                self.semantic
                    .recall_with_embedding(query, fetch_limit, filter, Some(&qe))?
            } else {
                self.semantic.recall(query, fetch_limit, filter)?
            }
        } else {
            self.semantic.recall(query, fetch_limit, filter)?
        };

        let items: Vec<MemoryItem> = results.into_iter().map(MemoryItem::from_fragment).collect();

        // Apply the cross-chat isolation filter (#5227) before truncating
        // to `max_retrieve`. Three classes of memory survive the filter:
        //   1. `MemoryLevel::User` — explicitly stable per-user facts;
        //      always cross-chat by design.
        //   2. Memories with no `chat_scope` tag — legacy rows from
        //      before this fix landed, plus any memory written through
        //      a non-channel path (direct API, dashboard); treat as
        //      chat-agnostic to avoid silently hiding existing data.
        //   3. Memories whose `chat_scope` equals the active scope —
        //      same chat, same context, safe to surface.
        let filtered: Vec<MemoryItem> = if chat_scope_active {
            let want = chat_scope.unwrap();
            items
                .into_iter()
                .filter(|m| memory_chat_scope_allows(m, want))
                .take(cfg.max_retrieve)
                .collect()
        } else {
            items.into_iter().take(cfg.max_retrieve).collect()
        };

        Ok(filtered)
    }
}

/// Thin adapter over `memory_scope_allows_recall` for `MemoryItem`,
/// which carries the level enum instead of the storage scope string.
/// Kept private to `proactive` so callers outside this crate use the
/// canonical predicate in `librefang-types` directly.
fn memory_chat_scope_allows(memory: &MemoryItem, current: &str) -> bool {
    memory_scope_allows_recall(memory.level.scope_str(), &memory.metadata, current)
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
                None,
                None,
            )
            .await
            .unwrap();

        assert!(result.has_content);
        // DefaultMemoryExtractor should extract "I prefer" as preference
        assert!(!result.memories.is_empty());
        assert_eq!(result.memories[0].category, Some("preference".to_string()));
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
                None,
                None,
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
        let results = store
            .auto_retrieve(&agent_id, "dark mode", None, None)
            .await
            .unwrap();
        assert!(!results.is_empty());
    }

    /// #5227 — `auto_retrieve` must hide memories tagged with a
    /// **different** `chat_scope` from the active recall, while still
    /// surfacing chat-agnostic and user-level memories. Writes are made
    /// directly through `semantic.remember_with_embedding_and_peer` so the
    /// test exercises the recall-side filter without coupling to the
    /// `DefaultMemoryExtractor`'s rule-based level assignment (which
    /// over-uses `MemoryLevel::User` and would otherwise mask the bug —
    /// in production the LLM extractor defaults to `MemoryLevel::Session`,
    /// matching what this test stamps explicitly).
    #[tokio::test]
    async fn test_auto_retrieve_cross_chat_isolation_5227() {
        use librefang_types::memory::CHAT_SCOPE_METADATA_KEY;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new();
        let agent_id_str = agent_id.to_string();
        let peer = "+15551234567";
        let dm_scope = "whatsapp:+15551234567@s.whatsapp.net";
        let group_scope = "whatsapp:9999@g.us";

        // Helper — write a session-level memory stamped with `scope`.
        let write_scoped = |content: &str, scope: Option<&str>| {
            let mut meta = std::collections::HashMap::new();
            if let Some(s) = scope {
                meta.insert(
                    CHAT_SCOPE_METADATA_KEY.to_string(),
                    serde_json::Value::String(s.to_string()),
                );
            }
            store
                .semantic
                .remember_with_embedding_and_peer(
                    agent_id,
                    content,
                    MemorySource::Conversation,
                    MemoryLevel::Session.scope_str(),
                    meta,
                    None,
                    None,
                    None,
                    Default::default(),
                    Some(peer),
                )
                .unwrap();
        };

        // 1. Group-scoped session memory (the contamination source).
        write_scoped(
            "Discussed shipping project Atlas by Friday in the group",
            Some(group_scope),
        );
        // 2. Legacy memory written before #5227 — no scope tag.
        write_scoped("I prefer dark mode for all editors", None);
        // 3. User-level memory written from the DM (should cross chats).
        let mut user_meta = std::collections::HashMap::new();
        user_meta.insert(
            CHAT_SCOPE_METADATA_KEY.to_string(),
            serde_json::Value::String(dm_scope.to_string()),
        );
        store
            .semantic
            .remember_with_embedding_and_peer(
                agent_id,
                "User's name is John",
                MemorySource::Conversation,
                MemoryLevel::User.scope_str(),
                user_meta,
                None,
                None,
                None,
                Default::default(),
                Some(peer),
            )
            .unwrap();

        // DM-scoped recall must NOT see the group-scoped Atlas memory.
        let dm_hits = store
            .auto_retrieve(&agent_id_str, "project Atlas", Some(peer), Some(dm_scope))
            .await
            .unwrap();
        for c in dm_hits.iter().map(|m| m.content.as_str()) {
            assert!(
                !c.contains("Atlas"),
                "regression: group-scoped memory leaked into DM recall: {c:?}"
            );
        }

        // Conversely, when recalled with the matching scope, the group
        // memory must still surface (the filter must not over-prune).
        let group_hits = store
            .auto_retrieve(
                &agent_id_str,
                "project Atlas",
                Some(peer),
                Some(group_scope),
            )
            .await
            .unwrap();
        assert!(
            group_hits.iter().any(|m| m.content.contains("Atlas")),
            "scope-matching recall must surface the group memory; got {:?}",
            group_hits.iter().map(|m| &m.content).collect::<Vec<_>>()
        );

        // Legacy unscoped memory crosses chats — both recalls hit it.
        let legacy_in_dm = store
            .auto_retrieve(&agent_id_str, "dark mode", Some(peer), Some(dm_scope))
            .await
            .unwrap();
        assert!(
            legacy_in_dm.iter().any(|m| m.content.contains("dark mode")),
            "legacy unscoped memory must remain recallable cross-chat"
        );

        // User-level memory crosses chats too — its stamped scope is
        // ignored because of the level-User exemption.
        let user_in_group = store
            .auto_retrieve(&agent_id_str, "John", Some(peer), Some(group_scope))
            .await
            .unwrap();
        assert!(
            user_in_group.iter().any(|m| m.content.contains("John")),
            "user-level memory must remain recallable cross-chat"
        );

        // When chat_scope is None (no channel context — e.g. dashboard,
        // direct API), the filter is a no-op and everything is visible.
        let unscoped = store
            .auto_retrieve(&agent_id_str, "project Atlas", Some(peer), None)
            .await
            .unwrap();
        assert!(
            unscoped.iter().any(|m| m.content.contains("Atlas")),
            "no-scope recall must preserve legacy behaviour"
        );
    }

    /// #5227 — verify `auto_memorize` itself stamps `chat_scope` onto
    /// stored memories so the recall filter has something to act on.
    /// Uses `DefaultMemoryExtractor`'s "I prefer …" rule, which yields a
    /// `MemoryLevel::User` memory; that's fine — the assertion is only
    /// about the metadata key being present and equal to the scope
    /// supplied by the caller. (Level-User exemption is verified
    /// separately in `test_auto_retrieve_cross_chat_isolation_5227`.)
    #[tokio::test]
    async fn test_auto_memorize_stamps_chat_scope_5227() {
        use librefang_types::memory::CHAT_SCOPE_METADATA_KEY;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let substrate = Arc::new(substrate);
        let store = ProactiveMemoryStore::with_default_config(substrate.clone());
        let agent_id = AgentId::new();
        let agent_id_str = agent_id.to_string();
        let peer = "+15551234567";
        let group_scope = "whatsapp:9999@g.us";

        let result = store
            .auto_memorize(
                &agent_id_str,
                &[serde_json::json!({
                    "role": "user",
                    "content": "I prefer dark mode"
                })],
                Some(peer),
                Some(group_scope),
            )
            .await
            .unwrap();
        assert!(result.has_content, "extractor must produce a memory");

        // Fetch what landed in the substrate and assert the metadata key
        // matches the scope we passed in.
        let mut filter = MemoryFilter::agent(agent_id);
        filter.peer_id = Some(peer.to_string());
        let stored = substrate
            .recall_with_embedding_async("dark mode", 10, Some(filter), None)
            .await
            .unwrap();
        assert!(!stored.is_empty(), "memory must be persisted");
        let with_scope: Vec<_> = stored
            .iter()
            .filter(|f| {
                f.metadata
                    .get(CHAT_SCOPE_METADATA_KEY)
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == group_scope)
            })
            .collect();
        assert!(
            !with_scope.is_empty(),
            "stored memory must carry the originating chat_scope metadata; \
             actual metadata: {:?}",
            stored.iter().map(|f| &f.metadata).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_memory_chat_scope_allows_predicate() {
        use librefang_types::memory::{MemoryItem, MemoryLevel, CHAT_SCOPE_METADATA_KEY};

        let mk = |level: MemoryLevel, scope: Option<&str>| {
            let mut m = MemoryItem::new("c".into(), level);
            if let Some(s) = scope {
                m.metadata.insert(
                    CHAT_SCOPE_METADATA_KEY.to_string(),
                    serde_json::Value::String(s.to_string()),
                );
            }
            m
        };

        // User level always passes.
        assert!(memory_chat_scope_allows(
            &mk(MemoryLevel::User, Some("group")),
            "dm"
        ));

        // No tag passes (legacy / chat-agnostic).
        assert!(memory_chat_scope_allows(
            &mk(MemoryLevel::Session, None),
            "dm"
        ));

        // Matching tag passes.
        assert!(memory_chat_scope_allows(
            &mk(MemoryLevel::Session, Some("dm")),
            "dm"
        ));

        // Non-matching tag is blocked.
        assert!(!memory_chat_scope_allows(
            &mk(MemoryLevel::Session, Some("group")),
            "dm"
        ));

        // Non-string sentinel = treated as missing (defensive).
        let mut m = MemoryItem::new("c".into(), MemoryLevel::Session);
        m.metadata
            .insert(CHAT_SCOPE_METADATA_KEY.to_string(), serde_json::Value::Null);
        assert!(memory_chat_scope_allows(&m, "dm"));
    }

    /// #5227 follow-up — Telegram/Slack/Discord shape regression.
    /// The original PR worked for the WhatsApp gateway because its
    /// `channel` string already embedded the chat
    /// (`"whatsapp:<jid>"`). For native channel adapters where the
    /// chat lives in `SenderContext.chat_id` and `channel` is just
    /// `"telegram"` / `"slack"` / `"discord"`, the kernel inject
    /// site must now compose `"<channel>:<chat_id>"` via
    /// `librefang_types::agent::compose_sender_scope` so DM and group
    /// of the same user get distinct chat scopes. This test pins the
    /// recall-side filter behaviour for that shape.
    ///
    /// Writes via `semantic.remember_with_embedding_and_peer` to keep
    /// the test independent of the rule-based extractor (which would
    /// over-promote preferences to `MemoryLevel::User` and bypass the
    /// filter; production uses an LLM extractor that defaults to
    /// `MemoryLevel::Session`, matching what this test stamps).
    #[tokio::test]
    async fn test_auto_retrieve_cross_chat_isolation_telegram_shape_5227() {
        use librefang_types::agent::compose_sender_scope;
        use librefang_types::memory::CHAT_SCOPE_METADATA_KEY;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new();
        let agent_id_str = agent_id.to_string();
        let peer = "tg-user-7777";

        // Compose chat scopes via the canonical helper — same formula
        // `SessionId::for_sender_scope` uses. The DM and group MUST
        // collapse to distinct strings.
        let dm_scope =
            compose_sender_scope("telegram", Some("dm-7777")).expect("non-empty channel");
        let group_scope =
            compose_sender_scope("telegram", Some("group--999")).expect("non-empty channel");
        assert_ne!(
            dm_scope, group_scope,
            "precondition: helper must yield distinct scopes for DM vs group"
        );
        // Bare `sender_channel` value would collapse them — proves
        // why this follow-up is necessary: the pre-follow-up filter
        // used bare channel and was a no-op on Telegram.
        assert_eq!(
            compose_sender_scope("telegram", None).unwrap(),
            "telegram",
            "bare channel (no chat_id) must remain just 'telegram' — \
             which is exactly why the bare key can't disambiguate"
        );

        let write_scoped = |content: &str, scope: &str| {
            let mut meta = std::collections::HashMap::new();
            meta.insert(
                CHAT_SCOPE_METADATA_KEY.to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            store
                .semantic
                .remember_with_embedding_and_peer(
                    agent_id,
                    content,
                    MemorySource::Conversation,
                    MemoryLevel::Session.scope_str(),
                    meta,
                    None,
                    None,
                    None,
                    Default::default(),
                    Some(peer),
                )
                .unwrap();
        };

        write_scoped(
            "Discussed shipping project Atlas by Friday in the Telegram group",
            &group_scope,
        );

        // DM-scope recall must NOT see the group memory.
        let dm_hits = store
            .auto_retrieve(&agent_id_str, "project Atlas", Some(peer), Some(&dm_scope))
            .await
            .unwrap();
        for content in dm_hits.iter().map(|m| m.content.as_str()) {
            assert!(
                !content.contains("Atlas"),
                "regression: telegram group-scoped memory leaked into DM \
                 recall: {content:?} (dm={dm_scope}, group={group_scope})"
            );
        }

        // Group-scope recall MUST see the group memory (filter must
        // not over-prune the matching chat).
        let group_hits = store
            .auto_retrieve(
                &agent_id_str,
                "project Atlas",
                Some(peer),
                Some(&group_scope),
            )
            .await
            .unwrap();
        assert!(
            group_hits.iter().any(|m| m.content.contains("Atlas")),
            "scope-matching telegram-group recall must surface the group \
             memory; got {:?}",
            group_hits.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
    }

    /// #5227 P1 (second-pass review) — the write-side dedupe in
    /// `add_with_decision` must also honour `chat_scope`, not just the
    /// read-side filter in `auto_retrieve`.
    ///
    /// Repro: the same peer states the same Session-level fact in two
    /// distinct chats (DM and group). Before this fix the second
    /// extraction reached `add_with_decision`, whose dedupe candidates
    /// were `(agent_id, peer_id)`-only — so the extractor saw the first
    /// chat's memory as a duplicate and NOOPed against it. The later
    /// `auto_retrieve(chat=second)` then filtered the first chat's row
    /// out, and the fact silently disappeared from the second chat.
    ///
    /// Expected post-fix behaviour: BOTH chats end up with their own
    /// Session-level row stamped for their respective scope, and both
    /// scope-matching recalls surface the fact.
    #[tokio::test]
    async fn test_add_with_decision_scopes_dedupe_by_chat_5227() {
        use librefang_types::agent::compose_sender_scope;
        use librefang_types::memory::CHAT_SCOPE_METADATA_KEY;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new();
        let agent_id_str = agent_id.to_string();
        let peer = "tg-user-7777";

        let dm_scope =
            compose_sender_scope("telegram", Some("dm-7777")).expect("non-empty channel");
        let group_scope =
            compose_sender_scope("telegram", Some("group--999")).expect("non-empty channel");
        assert_ne!(dm_scope, group_scope);

        // Build a `MemoryLevel::Session` extraction matching the bug
        // scenario. We drive `add_with_decision` directly because the
        // rule-based `DefaultMemoryExtractor` always promotes
        // preference-style content to `MemoryLevel::User`, which is
        // exempt from the cross-chat filter; production uses an LLM
        // extractor that lands the same fact at `Session`.
        let make_item = |content: &str, scope: &str| {
            let mut item = MemoryItem::new(content.to_string(), MemoryLevel::Session);
            item.metadata.insert(
                CHAT_SCOPE_METADATA_KEY.to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            item
        };

        // 1) First chat (DM): write the fact. Empty substrate → ADD.
        let dm_item = make_item("My deadline is Friday", &dm_scope);
        let dm_result = store
            .add_with_decision(agent_id, &dm_item, Some(peer), Some(&dm_scope))
            .await
            .unwrap();
        assert!(dm_result.is_some(), "first write must ADD");

        // 2) Second chat (group): same peer, same content. Pre-fix this
        //    saw the DM row as a duplicate and NOOPed → no row landed for
        //    the group. Post-fix the foreign-chat candidate is ignored
        //    and the extractor picks ADD again.
        let group_item = make_item("My deadline is Friday", &group_scope);
        let group_result = store
            .add_with_decision(agent_id, &group_item, Some(peer), Some(&group_scope))
            .await
            .unwrap();
        assert!(
            group_result.is_some(),
            "second write in a DIFFERENT chat must ADD (not NOOP against the \
             first chat's memory); got NOOP — regression"
        );

        // 3) Both scope-matching recalls must surface the fact for their
        //    chat.
        let dm_hits = store
            .auto_retrieve(&agent_id_str, "deadline", Some(peer), Some(&dm_scope))
            .await
            .unwrap();
        assert!(
            dm_hits.iter().any(|m| m.content.contains("Friday")),
            "DM recall must see the DM memory after the second write; got {:?}",
            dm_hits.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
        let group_hits = store
            .auto_retrieve(&agent_id_str, "deadline", Some(peer), Some(&group_scope))
            .await
            .unwrap();
        assert!(
            group_hits.iter().any(|m| m.content.contains("Friday")),
            "group recall must see its own memory (would be empty pre-fix \
             because the second write NOOPed); got {:?}",
            group_hits.iter().map(|m| &m.content).collect::<Vec<_>>()
        );

        // 4) Same-chat repeat — second write in the SAME chat with same
        //    content MUST still dedupe (NOOP or UPDATE). This guards
        //    against the filter accidentally letting through duplicates
        //    inside one chat.
        let same_chat_dupe = make_item("My deadline is Friday", &dm_scope);
        let dupe_result = store
            .add_with_decision(agent_id, &same_chat_dupe, Some(peer), Some(&dm_scope))
            .await
            .unwrap();
        // The DefaultMemoryExtractor decides NOOP for an exact-content
        // duplicate; we accept any non-ADD outcome (NOOP or UPDATE) to
        // stay decoupled from the heuristic.
        if let Some(r) = &dupe_result {
            assert_ne!(
                r.action,
                MemoryAction::Add,
                "same-chat exact duplicate must dedupe within the chat \
                 (NOOP or UPDATE), not ADD a third row"
            );
        }
    }

    /// #5227 P1 follow-up — User-level extractions are global and MUST
    /// dedupe across chats (level-User memories cross chats by design,
    /// so a second copy in a different chat is genuine duplication).
    /// The write-side scope filter has to skip when the new item is
    /// `MemoryLevel::User`.
    #[tokio::test]
    async fn test_add_with_decision_user_level_dedupes_across_chats_5227() {
        use librefang_types::agent::compose_sender_scope;
        use librefang_types::memory::CHAT_SCOPE_METADATA_KEY;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));
        let agent_id = AgentId::new();
        let peer = "tg-user-8888";

        let dm_scope = compose_sender_scope("telegram", Some("dm-8888")).unwrap();
        let group_scope = compose_sender_scope("telegram", Some("group--1234")).unwrap();

        let make_user_item = |content: &str, scope: &str| {
            let mut item = MemoryItem::new(content.to_string(), MemoryLevel::User);
            item.metadata.insert(
                CHAT_SCOPE_METADATA_KEY.to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            item
        };

        // First write: ADD.
        let first = make_user_item("User's name is John Doe", &dm_scope);
        let r1 = store
            .add_with_decision(agent_id, &first, Some(peer), Some(&dm_scope))
            .await
            .unwrap();
        assert!(r1.is_some(), "first user-level write must ADD");

        // Second write in a different chat, same content. Because both
        // sides are user-level (global), this MUST dedupe — not produce
        // a second physical row.
        let second = make_user_item("User's name is John Doe", &group_scope);
        let r2 = store
            .add_with_decision(agent_id, &second, Some(peer), Some(&group_scope))
            .await
            .unwrap();
        if let Some(r) = &r2 {
            assert_ne!(
                r.action,
                MemoryAction::Add,
                "user-level extractions must dedupe globally regardless of \
                 chat_scope; got ADD which would create cross-chat duplicate \
                 user facts"
            );
        }
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        // `add_with_level` stores raw content unconditionally; the trait
        // `add` path no longer has a raw-transcript fallback (C2 fix), so
        // tests that need a deterministic insert without exercising the
        // extractor pipeline use `add_with_level` directly.
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "Remember this fact"})],
                &agent_id,
                MemoryLevel::Session,
            )
            .await
            .unwrap();

        // Search to get the memory ID
        let results = store
            .search("Remember this fact", &agent_id, 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
        let mem_id = results[0].id.clone();

        // Delete it
        let deleted = store.delete(&mem_id, &agent_id).await.unwrap();
        assert!(deleted);

        // After delete, search must no longer find it (semantic store is
        // the source of truth post-#5839 C1 fix; the KV mirror is gone).
        let results_after = store
            .search("Remember this fact", &agent_id, 10)
            .await
            .unwrap();
        assert!(
            !results_after.iter().any(|m| m.id == mem_id),
            "deleted memory must not appear in subsequent searches"
        );

        // Deleting non-existent memory should return false
        let deleted_again = store.delete(&mem_id, &agent_id).await.unwrap();
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
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "Old content"})],
                &agent_id,
                MemoryLevel::Session,
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
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "First memory"})],
                &agent_id,
                MemoryLevel::Session,
            )
            .await
            .unwrap();
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "Second memory"})],
                &agent_id,
                MemoryLevel::Session,
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

        // Add session-level memory (use add_with_level — the trait
        // `add` path no longer raw-stores when extraction yields nothing).
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "Session info"})],
                &agent_id,
                MemoryLevel::Session,
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

    /// M14 regression: `decide_action` reads
    /// `update_threshold_same_category` /
    /// `update_threshold_cross_category` from the new memory's metadata
    /// (where `add_with_decision` stashes the live config values), so
    /// raising the threshold above the Jaccard similarity between two
    /// memories flips the decision from UPDATE back to ADD.
    #[tokio::test]
    async fn decide_action_honors_config_update_thresholds() {
        let extractor = DefaultMemoryExtractor;
        let agent = AgentId::new();
        let existing = MemoryFragment {
            id: MemoryId::new(),
            agent_id: agent,
            content: "Prefers Python for scripting".to_string(),
            source: MemorySource::Conversation,
            scope: "session_memory".to_string(),
            confidence: 1.0,
            metadata: {
                let mut m = HashMap::new();
                m.insert("category".to_string(), serde_json::json!("preference"));
                m
            },
            created_at: chrono::Utc::now(),
            accessed_at: chrono::Utc::now(),
            access_count: 0,
            embedding: None,
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
        };

        // A new memory with similar content + same category. With the
        // default 0.7 same-cat threshold this should UPDATE.
        let new = MemoryItem::new(
            "Prefers Python for data scripting".to_string(),
            MemoryLevel::Session,
        )
        .with_category("preference");
        let action = extractor
            .decide_action(&new, std::slice::from_ref(&existing))
            .await
            .unwrap();
        match action {
            MemoryAction::Update { .. } | MemoryAction::Noop => {}
            other => panic!("default thresholds should UPDATE or NOOP; got {other:?}"),
        }

        // Same input but with the same-category threshold pushed to
        // 0.99 via the metadata channel `add_with_decision` uses → ADD
        // wins. We only set the same-cat key — both candidate
        // memories share the "preference" category, so the cross-cat
        // branch is unreachable (review-followup #7).
        let mut new_strict = new.clone();
        new_strict
            .metadata
            .insert("_update_threshold_same_cat".into(), serde_json::json!(0.99));
        let action = extractor
            .decide_action(&new_strict, std::slice::from_ref(&existing))
            .await
            .unwrap();
        assert!(
            matches!(action, MemoryAction::Add),
            "raising _update_threshold_same_cat to 0.99 must demote UPDATE → ADD; got {action:?}"
        );
    }

    /// M14 strip regression (review-followup C + #2): a memory created
    /// via `add()` (which goes through `add_with_decision`) must NOT
    /// have any of the private `_update_threshold_*` / `_embedding`
    /// keys in its stored metadata column.
    ///
    /// Scope this test actually covers (review-followup B — the
    /// prior docstring overclaimed): a **coordinated two-step
    /// regression** where (a) the `strip_private_stash_keys` call is
    /// removed AND (b) the ADD or UPDATE branch is repointed at
    /// `enriched_item.metadata` (instead of the caller-supplied
    /// `item.metadata`). Single-step regressions of either kind pass
    /// this test:
    ///   - Strip removed alone → the current ADD/UPDATE branches
    ///     still build their stored metadata from `item.metadata`,
    ///     which never had the stash keys, so stored metadata is
    ///     still clean.
    ///   - Branch repointed alone → `strip_private_stash_keys` has
    ///     already wiped the stash keys from `enriched_item.metadata`
    ///     by the time the branch runs, so stored metadata is still
    ///     clean.
    ///
    /// The strip code itself is unit-tested directly by
    /// [`strip_private_stash_keys_removes_all_private_keys`] further
    /// down, which catches single-step regressions of the strip.
    ///
    /// We attach a mock embedding driver so the `_embedding` stash
    /// path actually fires. Without it the assertion on `_embedding`
    /// would be trivially true (the stash never happens → nothing to
    /// strip → not in metadata). Mock returns a tiny fixed vector so
    /// the round-trip is observable but cheap.
    #[tokio::test]
    async fn add_with_decision_does_not_leak_private_stash_keys_to_stored_metadata() {
        struct MockEmbedding;
        #[async_trait]
        impl EmbeddingFn for MockEmbedding {
            async fn embed_one(
                &self,
                _text: &str,
            ) -> librefang_types::error::LibreFangResult<Vec<f32>> {
                Ok(vec![0.1, 0.2, 0.3, 0.4])
            }
        }

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate))
            .with_embedding(Arc::new(MockEmbedding));
        let agent_id = AgentId::new().to_string();

        // Drive the full `add_with_decision` path. "I prefer X" is a
        // canonical pattern the DefaultMemoryExtractor will pick up,
        // so this goes through the ADD branch (no existing candidate
        // → no UPDATE possible). The mock embedding driver causes
        // `_embedding` to land in `enriched_item.metadata` before
        // `decide_action`, so the strip code on the post-decide path
        // is the only thing keeping it out of the stored column.
        let added = store
            .add(
                &[serde_json::json!({"role": "user", "content": "I prefer dark mode editors"})],
                &agent_id,
            )
            .await
            .unwrap();
        assert!(!added.is_empty(), "extractor should accept a preference");

        // Read back through the same `list()` path the dashboard /
        // auto_retrieve consume.
        let listed = store.list(&agent_id, None).await.unwrap();
        assert!(
            !listed.is_empty(),
            "the row we just added must surface in list()"
        );
        for item in &listed {
            for private_key in [
                "_update_threshold_same_cat",
                "_update_threshold_cross_cat",
                "_embedding",
            ] {
                assert!(
                    !item.metadata.contains_key(private_key),
                    "private stash key `{private_key}` leaked into stored metadata: {:?}",
                    item.metadata
                );
            }
        }
    }

    /// Direct unit test of `strip_private_stash_keys` (review-followup
    /// B). Catches a single-step regression of the strip itself —
    /// e.g. someone deletes one of the `metadata.remove(...)` calls,
    /// drops a key from `ADD_WITH_DECISION_PRIVATE_STASH_KEYS`, or
    /// renames the strip site to a no-op. Complements the
    /// integration test above, which only catches a coordinated
    /// two-step regression (strip removed AND branch repointed at
    /// enriched).
    #[test]
    fn strip_private_stash_keys_removes_all_private_keys() {
        let mut meta = HashMap::new();
        // All three private stash keys are pre-populated, plus a
        // sentinel public key that the strip must NOT touch.
        meta.insert(
            "_update_threshold_same_cat".to_string(),
            serde_json::json!(0.42),
        );
        meta.insert(
            "_update_threshold_cross_cat".to_string(),
            serde_json::json!(0.84),
        );
        meta.insert("_embedding".to_string(), serde_json::json!([1.0, 2.0]));
        meta.insert(
            "category".to_string(),
            serde_json::json!("operator-supplied"),
        );

        strip_private_stash_keys(&mut meta);

        for private_key in ADD_WITH_DECISION_PRIVATE_STASH_KEYS {
            assert!(
                !meta.contains_key(*private_key),
                "strip_private_stash_keys must remove `{private_key}`"
            );
        }
        // Public keys MUST survive — the strip is targeted, not a
        // metadata wipe.
        assert_eq!(
            meta.get("category"),
            Some(&serde_json::json!("operator-supplied")),
            "strip_private_stash_keys must not touch non-private keys"
        );
        // The const drives the strip exhaustively — adding a new
        // key to the const must extend the strip, not require a
        // matching code change in two places. Pin that here.
        assert_eq!(
            ADD_WITH_DECISION_PRIVATE_STASH_KEYS.len(),
            3,
            "ADD_WITH_DECISION_PRIVATE_STASH_KEYS contents drift — \
             update this test alongside the const to ensure new private \
             keys also get stripped"
        );
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
        store.store_relations(&triples, "test-agent");

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

        // Empty — no words to compare, so similarity is 0.0
        assert!((text_similarity("", "")).abs() < f32::EPSILON);
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
                None,
                None,
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
                None,
                None,
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
        store.store_relations(&triples, "test-agent");
        store.store_relations(&triples, "test-agent");

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

    /// M13 regression: `extract_search_keywords` returns multiple
    /// distinctive keywords ordered longest-first (the caller unions
    /// per-keyword LIKE recalls on the no-embedding fallback path) —
    /// pre-fix it collapsed the top 4 to the single longest word,
    /// wasting the other three and giving the no-embedding scan a
    /// frequently-too-generic substring to match against.
    #[test]
    fn extract_search_keywords_returns_multiple_ordered_longest_first() {
        let kws = extract_search_keywords("I prefer Python for data analysis and machine learning");
        // Pre-fix: returned only the single longest word ("analysis").
        // Post-fix: at most 4 distinctive words, longest-first. We
        // don't pin the exact contents because stable-sort within a
        // length bucket is implementation-detail, and we don't pin
        // the exact count either (review-followup #6) so that
        // additions to STOP_WORDS don't silently break this test.
        assert!(
            !kws.is_empty(),
            "should yield at least one keyword; got {kws:?}"
        );
        assert!(
            kws.len() <= 4,
            "must respect the 4-keyword cap; got {kws:?}"
        );
        assert!(
            kws.iter().any(|k| k == "analysis"),
            "longest distinctive word must survive; got {kws:?}"
        );
        // Stop words must not leak (catches stop-list regressions).
        for stop in ["i", "for", "and", "the", "a"] {
            assert!(
                !kws.iter().any(|k| k == stop),
                "stop word '{stop}' leaked into keywords: {kws:?}"
            );
        }
        // Longest-first ordering.
        for window in kws.windows(2) {
            assert!(
                window[0].len() >= window[1].len(),
                "keywords not sorted longest-first: {kws:?}"
            );
        }
    }

    /// Content with no distinctive words returns an empty list — caller
    /// falls back to a raw-content LIKE.
    #[test]
    fn extract_search_keywords_empty_for_all_stop_words() {
        // Stop list covers "i", "am", "the". Short words (<= 2 chars)
        // are dropped by length. Use only such tokens here.
        let kws = extract_search_keywords("I am the");
        assert!(
            kws.is_empty(),
            "all-stop-word content must yield no keywords; got {kws:?}"
        );
    }

    /// RBAC M3 (#3054) regression: when the user's `UserMemoryAccess`
    /// denies the `proactive` namespace, `search_all_with_guard` MUST
    /// return `AuthDenied` rather than leaking fragments back to the
    /// dashboard. Mirror test for `list_all_with_guard`.
    #[tokio::test]
    async fn search_all_with_guard_denies_unauthorised_read() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::user_policy::UserMemoryAccess;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        // Use add_with_level so the test does not depend on the
        // DefaultMemoryExtractor recognising "topsecret" — the trait
        // `add` path no longer has a raw-transcript fallback.
        store
            .add_with_level(
                &[serde_json::json!({"role": "user", "content": "topsecret"})],
                &agent_id,
                MemoryLevel::Session,
            )
            .await
            .unwrap();

        // Guard with NO read access at all.
        let guard = MemoryNamespaceGuard::new(UserMemoryAccess::default());
        let err = store.search_all_with_guard("topsecret", 10, &guard).await;
        assert!(matches!(
            err,
            Err(librefang_types::error::LibreFangError::AuthDenied(_))
        ));

        let err = store.list_all_with_guard(None, &guard).await;
        assert!(matches!(
            err,
            Err(librefang_types::error::LibreFangError::AuthDenied(_))
        ));

        // Guard WITH read access lets the same query through.
        let allow = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            ..Default::default()
        });
        let ok = store
            .search_all_with_guard("topsecret", 10, &allow)
            .await
            .unwrap();
        assert!(!ok.is_empty());
    }

    /// PII redaction MUST replace fields when the guard's `pii_access=false`.
    /// Tests the cross-agent search path; `search_with_guard` is covered
    /// independently via the `namespace_acl` module's own redaction tests.
    #[tokio::test]
    async fn search_all_with_guard_redacts_pii() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::user_policy::UserMemoryAccess;

        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let store = ProactiveMemoryStore::with_default_config(Arc::new(substrate));

        let agent_id = AgentId::new().to_string();
        store
            .add(
                &[serde_json::json!({
                    "role": "user",
                    "content": "Reach me at alice@example.com or 555-123-4567"
                })],
                &agent_id,
            )
            .await
            .unwrap();

        let guard = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["*".into()],
            pii_access: false,
            ..Default::default()
        });
        let items = store
            .search_all_with_guard("alice", 10, &guard)
            .await
            .unwrap();
        for item in items {
            assert!(
                !item.content.contains("alice@example.com"),
                "raw email leaked into search response: {}",
                item.content
            );
            assert!(
                !item.content.contains("555-123-4567"),
                "raw phone leaked into search response: {}",
                item.content
            );
        }
    }

    /// Regression for the "boost makes popular memories immortal" bug.
    ///
    /// Before the fix: a memory with access_count >= 2 ended every decay tick
    /// with `(decayed * 2.0).clamp(0,1) = 1.0`, freezing confidence forever.
    /// After the fix: boost divides the rate, so popular memories decay slower
    /// but still drop monotonically.
    #[test]
    fn popular_memory_decay_is_monotonic_not_frozen() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.05).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate.clone());
        let agent = AgentId::new();

        let mid = store
            .semantic
            .remember(
                agent,
                "preferred editor",
                MemorySource::Conversation,
                "agent_memory",
                HashMap::new(),
            )
            .unwrap();

        // Simulate a popular, stale memory: 5 accesses but last touched 10 days ago.
        let stale = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
        let db = store.semantic.pool().get().unwrap();
        db.execute(
            "UPDATE memories SET confidence = 1.0, access_count = 5, accessed_at = ?1 WHERE id = ?2",
            rusqlite::params![stale, mid.0.to_string()],
        )
        .unwrap();
        drop(db);

        store.decay_confidence().unwrap();

        let after: f64 = store
            .semantic
            .pool()
            .get()
            .unwrap()
            .query_row(
                "SELECT confidence FROM memories WHERE id = ?1",
                rusqlite::params![mid.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            after < 1.0,
            "popular memory must decay below 1.0; got {after} (boost-immortality regression)"
        );
        assert!(after > 0.0, "confidence must remain positive; got {after}");
    }

    /// `metadata["confidence"]` written by the LLM extractor must be honored
    /// at insert time, not silently overwritten by the legacy `1.0` default.
    #[test]
    fn insert_honors_extractor_supplied_confidence() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.05).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate);
        let agent = AgentId::new();
        let mut meta = HashMap::new();
        meta.insert("confidence".to_string(), serde_json::json!(0.42));
        let mid = store
            .semantic
            .remember(
                agent,
                "extracted fact",
                MemorySource::Conversation,
                "agent_memory",
                meta,
            )
            .unwrap();

        let stored: f64 = store
            .semantic
            .pool()
            .get()
            .unwrap()
            .query_row(
                "SELECT confidence FROM memories WHERE id = ?1",
                rusqlite::params![mid.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (stored - 0.42).abs() < 1e-9,
            "extractor confidence must round-trip into the column; got {stored}"
        );
    }

    /// C1 regression: `list()` / `get()` must read from the semantic store
    /// (authoritative), not the best-effort KV mirror. Pre-fix, a memory
    /// that landed in semantic but missed the KV write was invisible to
    /// `list()` while remaining searchable via `search()` — the split-brain
    /// that produced the "agent forgets things" reports.
    #[tokio::test]
    async fn list_reflects_semantic_store_not_kv_mirror() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.05).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate);
        let agent = AgentId::new();
        let user_id = agent.to_string();

        // Write directly to semantic (simulating a path that wrote semantic
        // but no KV mirror — exactly the pre-fix divergence pattern).
        let mut meta = HashMap::new();
        meta.insert("category".to_string(), serde_json::json!("custom_category"));
        store
            .semantic
            .remember(
                agent,
                "semantic-only memory",
                MemorySource::Conversation,
                "agent_memory",
                meta,
            )
            .unwrap();

        // The KV table has no `memory:*` keys for this id; with the old
        // implementation this list() would return empty.
        let listed = store.list(&user_id, None).await.unwrap();
        assert!(
            listed.iter().any(|m| m.content == "semantic-only memory"),
            "list() must surface semantic-store rows even when the KV mirror is empty; \
             got: {listed:?}"
        );

        // Category filter still works through the semantic path.
        let by_cat = store.list(&user_id, Some("custom_category")).await.unwrap();
        assert_eq!(by_cat.len(), 1, "category filter must work post-fix");
    }

    /// C2 regression: when the extractor returns no structured signal,
    /// `add()` must NOT store the raw concatenated message text as a
    /// session memory. The old fallback was the dominant source of
    /// `category=null` junk rows and verbatim-transcript duplicates.
    #[tokio::test]
    async fn add_does_not_store_raw_transcript_fallback() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.05).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate);
        let agent_id = AgentId::new().to_string();

        // Nothing here matches the rule-based extractor — should yield
        // `has_content = false` and therefore no rows.
        let items = store
            .add(
                &[serde_json::json!({
                    "role": "user",
                    "content": "Lorem ipsum dolor sit amet, consectetur adipiscing elit."
                })],
                &agent_id,
            )
            .await
            .unwrap();
        assert!(
            items.is_empty(),
            "no-signal add() must return [], not synthesize a transcript memory; got {items:?}"
        );

        // And nothing landed in the store either.
        let listed = store
            .list_all(None)
            .await
            .expect("list_all after no-signal add()");
        assert!(
            listed.is_empty(),
            "no rows must be persisted on the no-signal path; got {} item(s)",
            listed.len()
        );
    }

    /// RBAC C4 regression: every write-side `*_with_guard` wrapper must
    /// reject the fail-closed Viewer-equivalent ACL (no writes / deletes /
    /// exports). Previously the HTTP routes called the unguarded inherent
    /// methods, so any authenticated caller — including Viewer — could
    /// add / delete / reset / import / export / decay memories.
    #[tokio::test]
    async fn write_guarded_wrappers_reject_viewer_acl() {
        use crate::namespace_acl::MemoryNamespaceGuard;
        use librefang_types::error::LibreFangError;
        use librefang_types::memory::MemoryLevel;
        use librefang_types::user_policy::UserMemoryAccess;

        // Viewer fallback ACL: read-only on `proactive`, no writes, no
        // deletes, no exports — mirrors `anonymous_fallback_acl()` in
        // `routes/memory.rs` and `default_memory_acl(UserRole::Viewer)`.
        let viewer = MemoryNamespaceGuard::new(UserMemoryAccess {
            readable_namespaces: vec!["proactive".into()],
            writable_namespaces: vec![],
            pii_access: false,
            export_allowed: false,
            delete_allowed: false,
        });

        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.05).unwrap());
        let store = ProactiveMemoryStore::with_default_config(substrate);
        let agent = AgentId::new().to_string();

        let add_err = store
            .add_with_guard(
                &[serde_json::json!({"role": "user", "content": "I prefer x"})],
                &agent,
                &viewer,
            )
            .await;
        assert!(
            matches!(add_err, Err(LibreFangError::AuthDenied(_))),
            "add_with_guard must deny Viewer; got {add_err:?}"
        );

        let del_err = store
            .delete_with_guard("00000000-0000-0000-0000-000000000001", &agent, &viewer)
            .await;
        assert!(
            matches!(del_err, Err(LibreFangError::AuthDenied(_))),
            "delete_with_guard must deny Viewer; got {del_err:?}"
        );

        let reset_err = store.reset_with_guard(&agent, &viewer);
        assert!(
            matches!(reset_err, Err(LibreFangError::AuthDenied(_))),
            "reset_with_guard must deny Viewer; got {reset_err:?}"
        );

        let clear_err = store.clear_level_with_guard(&agent, MemoryLevel::Session, &viewer);
        assert!(
            matches!(clear_err, Err(LibreFangError::AuthDenied(_))),
            "clear_level_with_guard must deny Viewer; got {clear_err:?}"
        );

        let export_err = store.export_all_with_guard(&agent, &viewer);
        assert!(
            matches!(export_err, Err(LibreFangError::AuthDenied(_))),
            "export_all_with_guard must deny Viewer; got {export_err:?}"
        );

        let import_err = store
            .import_memories_with_guard(&agent, Vec::new(), &viewer)
            .await;
        assert!(
            matches!(import_err, Err(LibreFangError::AuthDenied(_))),
            "import_memories_with_guard must deny Viewer; got {import_err:?}"
        );

        let decay_err = store.decay_confidence_with_guard(&viewer);
        assert!(
            matches!(decay_err, Err(LibreFangError::AuthDenied(_))),
            "decay_confidence_with_guard must deny Viewer; got {decay_err:?}"
        );
    }
}
