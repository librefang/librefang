//! Memory substrate types: fragments, sources, filters, and the unified Memory trait.
//! Also includes proactive memory types for mem0-style API.

use crate::agent::AgentId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Memory levels for multi-level memory (User/Session/Agent)
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// Cosine similarity between this memory's embedding and the query that
    /// retrieved it, in `[-1.0, 1.0]` (#7808).
    ///
    /// `None` whenever the number would be a fiction rather than a measurement: a listing read, a
    /// `content LIKE` / FTS fallback with no embeddings in play, or a row that carries no stored
    /// embedding to compare against.
    /// Callers must not substitute `0.0` for `None` — 0.0 is a real cosine result (orthogonal) and
    /// conflating the two makes an unranked result look like a measured miss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
    /// The storage `scope` string of the fragment this item came from, carried out of the conversion instead of being discarded (#7920).
    ///
    /// [`level`](Self::level) is not a substitute: `MemoryLevel::from` folds every scope it does not recognise — [`EPISODIC_SCOPE`] among them — into `MemoryLevel::Session`, so a raw-dialogue row round-tripped through `MemoryItem` came back labelled `session_memory` and any consumer that classifies by scope filed it as an extracted fact.
    /// `None` means the item was not built from a stored fragment (a freshly extracted candidate, a sidecar reply) and has no storage scope yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
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
            similarity: None,
            scope: None,
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
            similarity: frag.similarity,
            metadata: frag.metadata,
            scope: Some(frag.scope),
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
/// # Use the kernel's default provider:
/// extraction_model = "gpt-4o-mini"
/// # Or target a specific provider with `provider/model` format:
/// extraction_model = "anthropic/claude-haiku-4"
/// # The colon form (`provider:model`) also works:
/// extraction_model = "anthropic:claude-haiku-4"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    ///
    /// The value is parsed by `resolve_extraction_model_target`. Three forms
    /// are accepted, in priority order:
    ///
    /// 1. `provider:model` — e.g. `"anthropic:claude-haiku-4"`
    /// 2. `provider/model` — e.g. `"anthropic/claude-haiku-4"`
    /// 3. Bare model name — e.g. `"gpt-4o-mini"`
    ///
    /// For bare model names the kernel's `default_model.provider` is used as
    /// the driver. Use the `provider/model` form when extraction should run
    /// through a different provider — there is no separate
    /// `extraction_provider` field.
    pub extraction_model: Option<String>,
    /// Categories to extract from conversations.
    pub extract_categories: Vec<String>,
    /// Session memory TTL in hours. Memories older than this are cleaned up
    /// automatically before each agent execution. Default: 24 hours.
    pub session_ttl_hours: u32,
    /// Similarity threshold for duplicate detection (0.0 - 1.0).
    /// When stored embeddings are available, uses vector cosine similarity
    /// (mem0-quality); otherwise falls back to Jaccard word overlap.
    /// Default: 0.85.
    ///
    /// Pre-fix this defaulted to 0.5, which is far too permissive for both
    /// metrics: cosine 0.5 matches "topically related" pairs (including
    /// opposite-meaning sentences that share keywords), and Jaccard 0.5
    /// matches anything with 50% word overlap. 0.85 is the threshold mem0
    /// recommends for "near-duplicate" detection and matches the
    /// industry-standard cosine cut-off for embedding-based dedup.
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
    /// Maximum number of characters that `format_context` (the prompt
    /// injection for `auto_retrieve` memories) may emit, including the
    /// header template (H4 review-followup #8). At ~4 chars per token
    /// this is roughly a token budget; the default 8000 (~2000 tokens)
    /// suits an 8k–32k context window. Operators on larger windows can
    /// raise this without recompiling. Excess memories are dropped and
    /// reported via a footer; the cap is a hard ceiling on the section.
    #[serde(default = "default_format_context_max_chars")]
    pub format_context_max_chars: usize,
    /// Similarity threshold above which `decide_action` returns UPDATE
    /// instead of ADD when the new and existing memory share a
    /// category (M14). Lower than `update_threshold_cross_category`
    /// because same-category memories share semantic context, so a
    /// modest similarity bump is enough evidence to fold them
    /// together. Default 0.7 — empirically the cut-off where
    /// embedding cosine starts catching genuine paraphrases without
    /// merging unrelated facts.
    ///
    /// Conceptually distinct from `duplicate_threshold` (which gates
    /// the post-hoc consolidation sweep — "should two existing
    /// memories be merged into one?"). The two serve different
    /// purposes: UPDATE is per-insertion conflict resolution; dedup
    /// is batch cleanup. Pre-fix both used the same hardcoded
    /// values, which conflated the contracts.
    #[serde(default = "default_update_threshold_same_category")]
    pub update_threshold_same_category: f32,
    /// Same as `update_threshold_same_category` but for memories with
    /// *different* categories. Higher (default 0.8) because
    /// cross-category merges require stronger evidence — the
    /// categories themselves carry semantic distinction that a
    /// purely-text similarity score might miss.
    #[serde(default = "default_update_threshold_cross_category")]
    pub update_threshold_cross_category: f32,
    /// Whether `auto_memorize` stamps each extracted memory with the session it came from, and `auto_retrieve` refuses to surface a memory stamped for a *different* session (#7605).
    ///
    /// The proactive store is per **agent**, not per conversation, so before this switch existed one visitor's turn on a public agent could be auto-retrieved into another visitor's turn purely through memory — even when the two turns were addressed to different `session_id`s and their message histories never touched.
    /// Default `true`: a memory belongs to the conversation that produced it unless an operator says otherwise.
    ///
    /// Setting `false` restores the pre-#7605 behaviour (every memory of an agent is a candidate for every one of that agent's turns).
    /// That is the right choice for a single-user assistant whose `session_mode = "new"` sub-agents are expected to inherit what earlier runs learned; it is the wrong choice for anything serving more than one person.
    ///
    /// Memories written before this shipped carry no session tag and stay recallable from every session, so turning it on does not hide an existing store.
    #[serde(default = "default_true")]
    pub session_scoped_recall: bool,
    /// Minimum cosine similarity a memory must reach against the query before
    /// it is recalled at all — the "nothing rather than noise" floor (#7808).
    ///
    /// Recall over-fetches candidates, re-ranks them by cosine, and truncates to top-k.
    /// With no floor, a sparse store fills that top-k with whatever exists: vectors that fail to
    /// compare sink to the bottom, but merely-irrelevant ones are promoted on merit and then
    /// injected into the prompt as if they were answers.
    /// A floor makes an empty recall possible, which is the honest outcome when nothing stored is
    /// actually about the query.
    ///
    /// `None` (the default) preserves the historical behaviour: every re-ranked candidate is
    /// eligible.
    /// Useful values sit around 0.2–0.4 for `text-embedding-3-small`; too high a floor empties
    /// recall entirely, so raise it while watching what recall returns rather than setting it
    /// blind.
    /// Ignored when no query embedding exists (the `content LIKE` / FTS fallback measures no
    /// similarity), and overridable per agent via
    /// [`ProactiveMemoryOverrides::min_similarity`] or per call by the
    /// `memory_semantic_search` tool's `min_similarity` argument.
    #[serde(default)]
    pub min_similarity: Option<f32>,
    /// Out-of-process memory extractor. When set, extraction is delegated to
    /// the configured subprocess (which may use its own LLM, a local model,
    /// embeddings, etc.) instead of the built-in LLM/rule-based extractor; the
    /// store and the dedup decision stay in Rust. Takes precedence over
    /// `extraction_model`.
    #[serde(default)]
    pub extractor_sidecar: Option<MemoryExtractorSidecarConfig>,
}

/// Configuration for an out-of-process memory extractor (see
/// [`ProactiveMemoryConfig::extractor_sidecar`]).
///
/// **Precedence (review-followup G).** When this table is present
/// AND `command` is non-empty, the sidecar handles every
/// `auto_memorize` call — the kernel's built-in `LlmMemoryExtractor`
/// is **not constructed at all**, and any per-agent
/// [`ProactiveMemoryOverrides::extraction_model`] is silently
/// shadowed. The kernel emits a `WARN` at boot in that case so an
/// operator who set both isn't surprised which wins. A
/// configured-but-empty `command` is treated as a no-op (the kernel
/// falls back to the LLM/rule-based path with a separate `WARN`).
///
/// The sidecar only handles **extraction**. The store, the dedup
/// `decide_action` decision, and the prompt-context formatting all
/// stay in Rust, so the sidecar's output cannot influence retrieval
/// ranking or storage layout — only what gets *proposed* to remember.
///
/// ```toml
/// [proactive_memory.extractor_sidecar]
/// command = "python3"
/// args = ["/home/me/.librefang/memory/extract.py"]
/// request_timeout_secs = 30
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MemoryExtractorSidecarConfig {
    /// Executable to launch (resolved via `PATH`). Empty string disables
    /// the sidecar (operator typo or commented-out args field) — see
    /// the precedence note on the struct.
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Per-extraction wall-clock timeout. `0` means the compiled default (30s).
    pub request_timeout_secs: u64,
}

impl Default for MemoryExtractorSidecarConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            request_timeout_secs: 30,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_memories_per_agent() -> usize {
    1000
}

fn default_format_context_max_chars() -> usize {
    8000
}

fn default_update_threshold_same_category() -> f32 {
    0.7
}

fn default_update_threshold_cross_category() -> f32 {
    0.8
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
                "communication_style".to_string(),
                "preference".to_string(),
                "expertise".to_string(),
                "work_style".to_string(),
                "project_context".to_string(),
                "personal_detail".to_string(),
                "frustration".to_string(),
            ],
            session_ttl_hours: 24,
            duplicate_threshold: 0.85,
            confidence_decay_rate: 0.01,
            max_memories_per_agent: 1000,
            format_context_max_chars: default_format_context_max_chars(),
            update_threshold_same_category: default_update_threshold_same_category(),
            update_threshold_cross_category: default_update_threshold_cross_category(),
            session_scoped_recall: true,
            min_similarity: None,
            extractor_sidecar: None,
        }
    }
}

/// Per-agent override for the kernel-global [`ProactiveMemoryConfig`] (#4870).
///
/// `[proactive_memory]` in `config.toml` sets a single, kernel-wide policy.
/// On hosts that mix one chatty user-facing agent with cron-driven
/// sub-agents (data collectors, ETL, brief composers), enabling
/// `auto_memorize` globally costs an extraction LLM call per sub-agent
/// turn for content that has no recall value. This struct lets an
/// agent's manifest disable proactive memory (in whole, or just one of
/// `auto_memorize` / `auto_retrieve`) without forcing the global policy
/// to follow.
///
/// Each field is `Option<bool>`: `None` inherits the global setting,
/// `Some(b)` overrides it. Resolution lives in
/// [`ProactiveMemoryOverrides::resolve_auto_retrieve`] and
/// [`ProactiveMemoryOverrides::resolve_auto_memorize`] so call sites in
/// the runtime can gate without reproducing the merge logic.
///
/// Boot caveat: the global [`ProactiveMemoryConfig::enabled = false`]
/// short-circuits store construction in
/// `librefang_kernel::kernel::boot`; per-agent `enabled = Some(true)`
/// cannot resurrect a non-existent store. For the same reason, per-field
/// overrides like `auto_memorize = Some(true)` or `auto_retrieve = Some(true)`
/// against a globally-off config are dead letters — the gate they would
/// flip never receives a store to act on. The intended (and currently
/// supported) shape is **per-agent opt-out** when the global is on.
///
/// Example in `agent.toml`:
/// ```toml
/// # full opt-out for this agent
/// [proactive_memory]
/// enabled = false
///
/// # or: keep retrieve, skip memorize (tool-output extraction is noise)
/// [proactive_memory]
/// auto_memorize = false
///
/// # or: per-agent extractor model (#5475) — agent A on a cheap OpenAI
/// # tier while the global default points elsewhere
/// [proactive_memory]
/// extraction_model = "openai/gpt-4o-mini"
/// ```
///
/// **The override surface is `{workspace}/agent.toml`, NOT `config.toml`** (#5476).
/// A `[agents.<name>.proactive_memory]` block in `~/.librefang/config.toml`
/// is silently ignored — `KernelConfig` has no `agents` field, so the
/// block parses but never feeds into any `AgentManifest`. Since #5476
/// the kernel emits a targeted `WARN` at boot (and on
/// `POST /api/config/reload`) when this misplacement is detected, but
/// the load-bearing path is still the agent's own manifest:
/// ```toml
/// # ~/.librefang/config.toml — kernel-global default
/// [proactive_memory]
/// auto_memorize = false
///
/// # ~/.librefang/workspaces/agents/my-agent/agent.toml — per-agent override
/// [proactive_memory]
/// auto_memorize = true
/// ```
///
/// Note: `Copy` was removed when `extraction_model: Option<String>` was
/// added in #5475 — the struct is now small but heap-allocating. Callers
/// that previously moved-by-copy now move-by-clone; that's a trivial
/// `Arc`-free `Option<String>` deep copy and not on a hot path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ProactiveMemoryOverrides {
    /// Override the master switch. `Some(false)` disables both retrieve
    /// and memorize for this agent regardless of the global config.
    /// `Some(true)` is documented but not load-bearing — see the boot
    /// caveat above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Override `auto_memorize`. `Some(false)` skips the after-turn
    /// extraction call for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_memorize: Option<bool>,
    /// Override `auto_retrieve`. `Some(false)` skips the before-turn
    /// retrieval (no memory items injected into the prompt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_retrieve: Option<bool>,
    /// Per-agent override for the LLM model used by proactive memory
    /// extraction (#5475). Same shape as the global
    /// [`ProactiveMemoryConfig::extraction_model`] — accepts
    /// `provider/model`, `provider:model`, or a bare model name that
    /// falls through to the kernel's default driver. `None` (the
    /// default) inherits the kernel-global `[proactive_memory]
    /// extraction_model`.
    ///
    /// Use case: multi-provider deployments where each agent's
    /// extractor should match the provider that hosts its primary
    /// model — e.g. agent A on `openai/gpt-4o-mini`, agent B on
    /// `anthropic/claude-haiku-4-5`, agent C on `gemini/gemini-2.0-flash`.
    /// Without this, the global must pick one extractor that may not
    /// even be reachable from the other agents' provider keys.
    ///
    /// Limitation in this PR: the override switches the model **name**
    /// passed to the boot-time extraction driver. Cross-provider
    /// switching (where the override picks a provider different from
    /// the one the kernel initialised the extraction driver with) is
    /// honoured only when the same driver supports both — typically
    /// within an OpenAI-compatible family. Full per-agent driver
    /// switching (rebuilding the LLM driver per-agent) is tracked as a
    /// follow-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_model: Option<String>,
    /// Override [`ProactiveMemoryConfig::session_scoped_recall`] (#7605).
    ///
    /// `Some(false)` lets one agent recall its own memories across every session while the deployment-wide default keeps sessions isolated — the escape hatch for a single-user agent whose `session_mode = "new"` runs are meant to build on each other.
    /// `Some(true)` opts one agent into isolation when the global default was turned off.
    /// `None` (the default) inherits the kernel-global value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_scoped_recall: Option<bool>,
    /// Per-agent override for [`ProactiveMemoryConfig::min_similarity`] — the
    /// cosine floor below which a memory is not recalled at all (#7808).
    ///
    /// `None` (the default) inherits the kernel-global value.
    /// Set it per agent when one agent's store is dense enough to support a strict floor while
    /// another's is sparse and would go silent under the same number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_similarity: Option<f32>,
    /// Whether this agent may consolidate its **own** semantic memory
    /// unattended, via the `memory_semantic_consolidate` tool (#7808).
    ///
    /// Defaults to `false`, and that default is the whole point.
    /// Consolidation is not a read with a side effect: it groups near-duplicates across the
    /// agent's entire store and soft-deletes every member of each group but one, in a single
    /// unattended call with no per-row confirmation and no undo the agent can reach.
    /// The number of rows it removes is bounded only by how many near-duplicates the configured
    /// `duplicate_threshold` finds, so a threshold tuned loosely enough turns one tool call into a
    /// broad deletion.
    /// A capability that destructive should exist because an agent poisoned by a pile of
    /// reinforcing near-duplicates has no remedy short of a full reset — but it should not arrive
    /// switched on with the rest of the memory surface.
    ///
    /// Leaving it off costs nothing an operator cannot recover: `memory_semantic_duplicates` is
    /// always available, so the agent can still see and report every group it would have merged,
    /// and `POST /api/memory/agents/{id}/consolidate` still performs the merge with a human
    /// deciding when.
    /// What the flag buys is the unattended case — chiefly the auto-dream loop, whose Consolidate
    /// phase exists for exactly this and which is itself already per-agent opt-in.
    ///
    /// ```toml
    /// # {workspace}/agent.toml — NOT config.toml (#5476)
    /// [proactive_memory]
    /// allow_self_consolidation = true
    /// ```
    ///
    /// There is deliberately no kernel-global counterpart.
    /// A deployment-wide "every agent may prune its own store" switch is a single edit that
    /// silently arms destructive maintenance on agents nobody re-examined, and `KernelConfig` has
    /// no `agents` table to scope it back down with (#5476) — so the decision stays where the
    /// blast radius is, in the manifest of the one agent it applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_self_consolidation: Option<bool>,
}

impl ProactiveMemoryOverrides {
    /// Resolve the effective `auto_retrieve` for this agent given the
    /// kernel-global `[proactive_memory]` defaults.
    pub fn resolve_auto_retrieve(&self, global: &ProactiveMemoryConfig) -> bool {
        if matches!(self.enabled, Some(false)) {
            return false;
        }
        if let Some(v) = self.auto_retrieve {
            return v;
        }
        global.enabled && global.auto_retrieve
    }

    /// Resolve the effective `auto_memorize` for this agent given the
    /// kernel-global `[proactive_memory]` defaults.
    pub fn resolve_auto_memorize(&self, global: &ProactiveMemoryConfig) -> bool {
        if matches!(self.enabled, Some(false)) {
            return false;
        }
        if let Some(v) = self.auto_memorize {
            return v;
        }
        global.enabled && global.auto_memorize
    }

    /// Resolve the effective `extraction_model` for this agent given
    /// the kernel-global `[proactive_memory]` defaults (#5475).
    ///
    /// Resolution chain: agent override → kernel-global → `None`
    /// (callers fall back to the agent's primary model). Empty strings
    /// on either side are treated as unset — operators sometimes leave
    /// `extraction_model = ""` to denote "no override", and the
    /// global-side `filter(|s| !s.is_empty())` upstream of boot already
    /// applies that convention.
    pub fn resolve_extraction_model(&self, global: &ProactiveMemoryConfig) -> Option<String> {
        if let Some(m) = self.extraction_model.as_ref() {
            if !m.is_empty() {
                return Some(m.clone());
            }
        }
        global
            .extraction_model
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
    }

    /// Resolve the effective `session_scoped_recall` for this agent given the kernel-global `[proactive_memory]` defaults (#7605).
    ///
    /// Unlike [`Self::resolve_auto_retrieve`] this deliberately ignores the master `enabled` switch: `enabled = false` already means the agent performs no automatic recall at all, so there is nothing left for a scoping policy to decide, and folding it in here would make a disabled agent report "not session-scoped" — the more permissive of the two answers, which is the wrong way for a fail-safe to lean.
    pub fn resolve_session_scoped_recall(&self, global: &ProactiveMemoryConfig) -> bool {
        self.session_scoped_recall
            .unwrap_or(global.session_scoped_recall)
    }

    /// Resolve the effective `min_similarity` floor for this agent given the kernel-global `[proactive_memory]` defaults (#7808).
    ///
    /// Agent override → kernel-global → `None` (no floor, the historical behaviour).
    /// Like [`Self::resolve_session_scoped_recall`] this ignores the master `enabled` switch: an
    /// agent with proactive memory off performs no recall for a floor to apply to.
    pub fn resolve_min_similarity(&self, global: &ProactiveMemoryConfig) -> Option<f32> {
        self.min_similarity.or(global.min_similarity)
    }

    /// Whether this agent may consolidate its own semantic memory unattended (#7808).
    ///
    /// There is no global fallback by design (see [`Self::allow_self_consolidation`]): an
    /// unset override is `false`, so the capability is reachable only from the manifest of the
    /// agent that will do the deleting.
    pub fn resolve_allow_self_consolidation(&self) -> bool {
        self.allow_self_consolidation.unwrap_or(false)
    }

    /// True when *no* field is set — equivalent to `Default::default()`.
    /// Used by call sites that want to skip the resolve dance entirely
    /// for the common "no override" case.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.auto_memorize.is_none()
            && self.auto_retrieve.is_none()
            && self.extraction_model.is_none()
            && self.session_scoped_recall.is_none()
            && self.min_similarity.is_none()
            && self.allow_self_consolidation.is_none()
    }
}

/// A relationship triple extracted from conversation (subject, relation, object).
///
/// Example: ("Alice", "works_at", "Acme Corp")
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MemoryConflict {
    /// The previous memory content that was replaced.
    pub old_content: String,
    /// The new memory content that replaced it.
    pub new_content: String,
    /// The ID of the memory that was updated.
    pub memory_id: String,
}

/// Result from a single memory add operation, including the decision taken.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// `categories` is the caller-supplied list from `ProactiveMemoryConfig::extract_categories`.
    /// Implementations must restrict extracted memories to these categories so that the
    /// config is the single source of truth — not a hardcoded list inside the prompt.
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
        categories: &[String],
    ) -> crate::error::LibreFangResult<ExtractionResult>;

    /// Same as `extract_memories` but also passes the invoking agent's
    /// id, so implementors can route their LLM call through a forked
    /// agent turn (shared prompt cache with the parent) instead of a
    /// standalone provider request. Callers that know the agent id
    /// (notably auto_memorize, which parses it out of `user_id`) should
    /// prefer this method. Default delegates to `extract_memories`,
    /// ignoring `agent_id` — appropriate for the rule-based extractor
    /// which never touches an LLM.
    async fn extract_memories_with_agent_id(
        &self,
        messages: &[serde_json::Value],
        _agent_id: &str,
        categories: &[String],
    ) -> crate::error::LibreFangResult<ExtractionResult> {
        self.extract_memories(messages, categories).await
    }

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
                    // Fall back to text similarity if vectors are not
                    // comparable (dim mismatch, zero magnitude). 0.0 would
                    // mean "fully dissimilar" and incorrectly suppress
                    // legitimate dedup candidates.
                    Some(ref ne) => cosine_similarity(ne, emb)
                        .unwrap_or_else(|| text_similarity(&new_lower, &old_lower)),
                    None => text_similarity(&new_lower, &old_lower),
                }
            } else {
                text_similarity(&new_lower, &old_lower)
            };

            // Very high similarity (≥ 0.95) → NOOP (near-duplicate)
            if similarity >= 0.95 {
                return Ok(MemoryAction::Noop);
            }

            // High similarity or same category → candidate for UPDATE.
            //
            // Thresholds raised from the original 0.5 / 0.6, which were far
            // too permissive in both metrics: cosine 0.5 matches topically
            // related but semantically distinct sentences (incl. opposite
            // meanings sharing keywords), so an UPDATE there silently
            // replaced unrelated memories. The numbers now align with
            // mem0's recommended cut-offs (≈ 0.7 same-category, ≈ 0.8
            // cross-category) and keep the 0.95 NOOP gate for near-exact
            // duplicates.
            //
            // M14: the per-call threshold pair is read from the new
            // memory's metadata, where `add_with_decision` stashes
            // `update_threshold_same_category` /
            // `update_threshold_cross_category` from
            // `ProactiveMemoryConfig`. Falls back to the const defaults
            // for callers that invoke `decide_action` directly
            // (without going through the store).
            let new_cat = new_memory.category.as_deref().unwrap_or("");
            let old_cat = existing
                .metadata
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let same_cat_threshold = new_memory
                .metadata
                .get("_update_threshold_same_cat")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(default_update_threshold_same_category());
            let cross_cat_threshold = new_memory
                .metadata
                .get("_update_threshold_cross_cat")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32)
                .unwrap_or(default_update_threshold_cross_category());
            let update_threshold = if !new_cat.is_empty() && new_cat == old_cat {
                same_cat_threshold
            } else {
                cross_cat_threshold
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
/// Returns `Some(score)` in `[-1.0, 1.0]` where `1.0` means identical
/// direction. Returns `None` when the vectors are not comparable:
/// empty inputs, dimension mismatch, or either vector has zero
/// magnitude. Callers MUST handle `None` explicitly — folding
/// "not comparable" into a 0.0 score silently corrupts ranking
/// because 0.0 means "fully dissimilar" (see #3536).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
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
        None
    } else {
        Some(dot / denom)
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
        similarity: None,
        scope: None,
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
        _categories: &[String],
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
                        "preference",
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
                        "personal_detail",
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
                        "preference",
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
                            "project_context",
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
        // The trait method has no config access; fall back to the const
        // default budget. Callers that have a `ProactiveMemoryConfig`
        // (the store, not the bare trait) should go through
        // `ProactiveMemoryStore::format_context*` which uses the
        // configured `format_context_max_chars`.
        format_memories_with_budget(memories, FORMAT_CONTEXT_MAX_CHARS)
    }
}

/// Default maximum number of characters spent on memory-content
/// bullets in a single prompt injection (H4). At ~4 chars per token
/// this caps the memory section at roughly 2000 tokens, which is a
/// reasonable share of a typical 8k-32k context window.
///
/// Pre-fix `format_context` had no cap at all: 10 memories × 2000
/// chars (`MAX_MEMORY_CONTENT_LENGTH`) could pump 20 KB into every
/// request. The bullet header counts against this budget too so the
/// cap is a true ceiling on prompt-section growth, not just per-bullet
/// content.
///
/// Operators on larger context windows can override via
/// `ProactiveMemoryConfig::format_context_max_chars`
/// (review-followup #8). The const value is the trait-level fallback
/// used by callers that don't have access to a `ProactiveMemoryConfig`
/// (e.g. `DefaultMemoryExtractor` invoked directly from tests).
pub const FORMAT_CONTEXT_MAX_CHARS: usize = 8000;

/// Shared formatter used by both [`DefaultMemoryExtractor::format_context`]
/// and the LLM-backed extractor — keeps the H4 budget logic centralized
/// instead of duplicated.
///
/// `max_chars` is the hard ceiling on the returned string length. Pass
/// `FORMAT_CONTEXT_MAX_CHARS` when no per-call config is available.
pub fn format_memories_with_budget(memories: &[MemoryItem], max_chars: usize) -> String {
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

    let header_len = context.len();
    let mut included = 0usize;
    let total = memories.len();
    for mem in memories {
        let bullet = format!("- {}\n", mem.content);
        // Reserve ~64 chars for the truncation footer so we never emit a
        // bullet that pushes us past the cap and then has no room for the
        // "[+N more]" note.
        if context.len() + bullet.len() > max_chars.saturating_sub(64) {
            break;
        }
        context.push_str(&bullet);
        included += 1;
    }

    if included < total {
        let dropped = total - included;
        context.push_str(&format!(
            "- [+{dropped} additional memor{plural} omitted to keep the prompt within budget]\n",
            plural = if dropped == 1 { "y" } else { "ies" }
        ));
    }

    // Defense-in-depth: if even the header alone exceeded the cap (unlikely
    // — header is ~700 chars), at least guarantee the returned string never
    // exceeds the budget by trimming on a char boundary.
    if context.len() > max_chars {
        let mut cutoff = max_chars;
        while cutoff > 0 && !context.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        context.truncate(cutoff);
    }
    debug_assert!(context.len() <= max_chars);
    let _ = header_len;
    context
}

/// Unique identifier for a memory fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
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

/// Modality of a memory fragment (text, image, or multimodal).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryModality {
    /// Pure text memory.
    #[default]
    Text,
    /// Image-only memory.
    Image,
    /// Combined text + image memory.
    MultiModal,
}

/// Where a memory came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// Optional URL to an associated image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Optional image embedding vector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_embedding: Option<Vec<f32>>,
    /// Modality of this memory (text, image, or multimodal).
    #[serde(default)]
    pub modality: MemoryModality,
    /// Cosine similarity against the query embedding that retrieved this
    /// fragment, carried out of the ranker instead of being discarded (#7808).
    ///
    /// The re-ranking comparator has always computed this number and thrown it away inside the
    /// `sort_by` closure, which is why no caller could ask for "nothing rather than noise" — the
    /// only signal that survived was rank order, and rank order on a sparse store still fills the
    /// top-k with whatever exists.
    /// `None` means no similarity was measured for this fragment: no query embedding, no stored
    /// embedding, or a non-comparable pair (dimension mismatch, zero magnitude).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
}

impl MemoryFragment {
    /// Whether this fragment is raw dialogue rather than an extracted fact.
    ///
    /// Thin wrapper over [`memory_is_raw_dialogue`]; see there for the predicate and why the two classes are budgeted separately.
    pub fn is_raw_dialogue(&self) -> bool {
        memory_is_raw_dialogue(&self.scope, &self.source, &self.metadata)
    }
}

/// Filter criteria for memory recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// Filter by peer ID (for per-user memory isolation in multi-user channels).
    pub peer_id: Option<String>,
    /// Minimum cosine similarity a fragment must reach against the query
    /// embedding to be returned at all (#7808).
    ///
    /// Unlike every other field on this struct this is **not** a SQL predicate: similarity is only
    /// known after the candidate rows have been re-ranked, so it is applied by `recall_impl` after
    /// the cosine pass and before the top-k truncation.
    /// It therefore has no effect on a recall with no query embedding — there is no score to
    /// compare against, and inventing one would silently empty the fallback path.
    ///
    /// Distinct from `min_confidence`, which is decay-derived trust in a memory's content and says
    /// nothing about whether the memory answers *this* query.
    #[serde(default)]
    pub min_similarity: Option<f32>,
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GraphMatch {
    /// The source entity.
    pub source: Entity,
    /// The relation.
    pub relation: Relation,
    /// The target entity.
    pub target: Entity,
}

/// Report from memory consolidation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConsolidationReport {
    /// Number of memories merged.
    pub memories_merged: u64,
    /// Number of memories whose confidence decayed.
    pub memories_decayed: u64,
    /// How long the consolidation took.
    pub duration_ms: u64,
}

/// Format for memory export/import.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ExportFormat {
    /// JSON format.
    Json,
    /// MessagePack binary format.
    MessagePack,
}

/// Report from memory import.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
        peer_id: Option<&str>,
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
    ///
    /// `agent_id` scopes the entity to its owning agent so agent-scoped reads (`GET /api/memory/agents/{id}/relations`) and `delete_by_agent` see it; the empty string is the shared/unscoped sentinel.
    /// `peer_id` scopes the entity to a single user on a multi-user agent (#6494); `None` writes a shared/unscoped entity.
    async fn add_entity(
        &self,
        entity: Entity,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> crate::error::LibreFangResult<String>;

    /// Add a relation between entities.
    ///
    /// `agent_id` scopes the relation to its owning agent (see [`add_entity`](Self::add_entity)).
    /// `peer_id` scopes the relation to a single user (#6494); `None` writes a
    /// shared/unscoped relation.
    async fn add_relation(
        &self,
        relation: Relation,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> crate::error::LibreFangResult<String>;

    /// Query the knowledge graph, optionally scoped to a single user.
    ///
    /// `peer_id` restricts the read to that user's triples (#6494); `None` is
    /// an unscoped read returning every peer's rows (shared semantics).
    async fn query_graph(
        &self,
        pattern: GraphPattern,
        peer_id: Option<&str>,
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

/// Metadata key under which `auto_memorize` tags memories with their
/// originating `(channel, chat)` scope. Format mirrors the kernel's
/// `sender_channel`: either a bare channel type (`"telegram"`) or a
/// chat-qualified form (`"whatsapp:<chatJid>"`). When present, recall
/// filters this against the active request's `chat_scope` so a memory
/// extracted from a group chat cannot bleed into a DM with the same
/// peer — and vice versa (#5227).
///
/// Memories without this key are treated as chat-agnostic (legacy /
/// manually-stored / `MemoryLevel::User`) and remain recallable across
/// all chats for the same `(agent, peer)` pair.
pub const CHAT_SCOPE_METADATA_KEY: &str = "chat_scope";

/// Decide whether a memory (identified by its stored `scope` string and
/// `metadata` map) is allowed to surface in a recall whose active
/// `(channel, chat)` scope is `current`. Returns `true` for three
/// classes that must always cross chats:
///
/// 1. `MemoryLevel::User` — stable per-user facts (the `scope` column
///    stores `"user_memory"` for these). Cross-chat by design.
/// 2. Memories with no `CHAT_SCOPE_METADATA_KEY` tag — pre-#5227
///    rows plus anything written through a non-channel path
///    (dashboard, direct API). Treating them as chat-agnostic avoids
///    silently hiding existing data.
/// 3. Memories whose stamped `chat_scope` equals `current`.
///
/// All other tagged memories are filtered out.
///
/// Pulled out into the types crate so every recall site — proactive
/// (`MemoryItem`), substrate (`MemoryFragment`), context engine — uses
/// the same predicate and cannot drift.
pub fn memory_scope_allows_recall(
    scope: &str,
    metadata: &HashMap<String, serde_json::Value>,
    current: &str,
) -> bool {
    // Class 1 — user-level memories cross chats by design.
    if scope == MemoryLevel::User.scope_str() {
        return true;
    }
    match metadata.get(CHAT_SCOPE_METADATA_KEY) {
        // Class 3 — stamped scope matches the active one.
        Some(serde_json::Value::String(s)) if s == current => true,
        // Stamped scope is set but differs → block.
        Some(serde_json::Value::String(_)) => false,
        // Class 2 — no tag (or non-string sentinel) → chat-agnostic.
        _ => true,
    }
}

/// Metadata key under which `auto_memorize` records the session a memory was extracted from (#7605).
///
/// The value is the turn's `SessionId` rendered as a UUID string — the same identity `POST /api/agents/{id}/message` accepts as `session_id` and `librefang message --session-id` passes, resolved by the ladder in `docs/architecture/session-mode-resolution.md`.
/// There is no second notion of a session here: whatever session the turn's history was read from and written back to is what gets stamped.
///
/// Distinct from [`CHAT_SCOPE_METADATA_KEY`], which answers "which chat on which channel" and is `None` for every non-channel caller (dashboard, REST, CLI) — precisely the callers a multi-user deployment uses.
pub const SESSION_SCOPE_METADATA_KEY: &str = "session_scope";

/// Scope string under which the unconditional per-turn writer files a whole exchange verbatim.
///
/// `agent_loop::prompt::remember_interaction_best_effort` writes one such row per turn; nothing distils them and they carry no TTL, which is why they dominate a mature store (794 of 999 live rows on the installation measured in #7920).
pub const EPISODIC_SCOPE: &str = "episodic";

/// Metadata key an extractor stamps on a fact it distilled, and the marker that separates an extracted fact from raw dialogue.
///
/// `ProactiveMemoryStore::add_with_decision` always sets it; the per-turn dialogue writer never does.
pub const MEMORY_CATEGORY_METADATA_KEY: &str = "category";

/// Whether a stored memory is **raw dialogue** — a whole exchange written verbatim by the per-turn writer — rather than an extracted fact.
///
/// The predicate is the exact write signature of `agent_loop::prompt::remember_interaction_best_effort`: [`MemorySource::Conversation`], scope [`EPISODIC_SCOPE`], and no [`MEMORY_CATEGORY_METADATA_KEY`].
/// Extracted facts always carry a category and always land in a `*_memory` scope, so they can never match; imported and system-sourced rows differ in `source`.
/// It is the same three-part test `SemanticStore::eviction_candidates` applies in SQL (`librefang-memory`, #7756 §1.2), lifted here so the prompt builder and the eviction cap agree on what a class is instead of each carrying its own copy.
///
/// The two classes differ by an order of magnitude in size — 1167 characters against 133 on the measured corpus — which is why the prompt memory section budgets them separately (#7920).
pub fn memory_is_raw_dialogue(
    scope: &str,
    source: &MemorySource,
    metadata: &HashMap<String, serde_json::Value>,
) -> bool {
    if scope != EPISODIC_SCOPE || !matches!(source, MemorySource::Conversation) {
        return false;
    }
    // Mirrors `COALESCE(json_extract(metadata, '$.category'), '') = ''`: a JSON null
    // and a missing key are both "no category", and a non-string value counts as one.
    match metadata.get(MEMORY_CATEGORY_METADATA_KEY) {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

/// Decide whether a memory may surface in a recall running under session `current` (#7605).
///
/// Two classes pass:
///
/// 1. Memories with no [`SESSION_SCOPE_METADATA_KEY`] tag — every row written before this shipped, plus anything stored by hand through the memory tools or the dashboard. Hiding them would blank out existing stores on upgrade, so they stay session-agnostic.
/// 2. Memories stamped with `current`.
///
/// Everything else is filtered out.
///
/// Unlike [`memory_scope_allows_recall`] there is **no `MemoryLevel::User` exemption**.
/// The cross-chat filter can afford one because the chats it separates belong to the same person; the sessions this separates routinely belong to different people, and "user-level" is exactly where an extractor files the personal details that must not cross ("my customer code is PINE-77").
/// A level-based exemption here would leave the reported leak open on the highest-value rows.
pub fn memory_session_scope_allows_recall(
    metadata: &HashMap<String, serde_json::Value>,
    current: &str,
) -> bool {
    match metadata.get(SESSION_SCOPE_METADATA_KEY) {
        Some(serde_json::Value::String(s)) => s == current,
        // No tag (or a non-string sentinel written by an older/foreign
        // producer) → session-agnostic.
        _ => true,
    }
}

/// Trait for proactive memory hooks (auto_memorize, auto_retrieve).
///
/// This provides hooks for automatic memory extraction and retrieval:
/// - auto_memorize() - extract important info after agent runs
/// - auto_retrieve() - proactively load context before agent runs
#[async_trait]
pub trait ProactiveMemoryHooks: Send + Sync {
    /// Extract and store important information after agent execution.
    /// When `peer_id` is `Some`, memories are scoped to that peer for isolation.
    /// When `chat_scope` is `Some`, the originating `(channel, chat)` scope is
    /// stamped onto each memory's metadata so subsequent recalls in a
    /// **different** chat (same peer) will not surface it (#5227). Pass `None`
    /// when the caller has no channel context (e.g. direct API, dashboard) —
    /// memories then remain chat-agnostic.
    ///
    /// When `session_scope` is `Some`, the session that produced the turn is stamped under [`SESSION_SCOPE_METADATA_KEY`] and a later recall running under a *different* session will not surface the memory (#7605).
    /// Pass `None` to store session-agnostic memories — that is what a caller does when the operator has turned [`ProactiveMemoryConfig::session_scoped_recall`] off, and it is the pre-#7605 behaviour.
    async fn auto_memorize(
        &self,
        user_id: &str,
        conversation: &[serde_json::Value],
        peer_id: Option<&str>,
        chat_scope: Option<&str>,
        session_scope: Option<&str>,
    ) -> crate::error::LibreFangResult<ExtractionResult>;

    /// Proactively retrieve relevant context before agent execution.
    /// When `peer_id` is `Some`, only retrieves memories for that peer.
    /// When `chat_scope` is `Some`, memories tagged with a **different**
    /// chat scope are filtered out post-recall — chat-agnostic memories
    /// (no scope tag, or stamped with the current scope) still surface.
    /// This is the read side of the #5227 cross-chat isolation guard.
    ///
    /// When `session_scope` is `Some`, memories stamped for a **different** session are dropped post-recall; untagged memories still surface.
    /// This is the read side of the #7605 cross-session isolation guard, and it composes with the chat filter rather than replacing it — a memory has to clear both.
    async fn auto_retrieve(
        &self,
        user_id: &str,
        query: &str,
        peer_id: Option<&str>,
        chat_scope: Option<&str>,
        session_scope: Option<&str>,
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
            image_url: None,
            image_embedding: None,
            modality: Default::default(),
            similarity: None,
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
    fn test_proactive_memory_overrides_default_inherits_global() {
        // No fields set → resolution returns the global truth verbatim.
        let global = ProactiveMemoryConfig::default();
        let overrides = ProactiveMemoryOverrides::default();
        assert!(overrides.is_empty());
        assert!(overrides.resolve_auto_retrieve(&global));
        assert!(overrides.resolve_auto_memorize(&global));

        let disabled_global = ProactiveMemoryConfig {
            auto_memorize: false,
            ..ProactiveMemoryConfig::default()
        };
        assert!(!overrides.resolve_auto_memorize(&disabled_global));
    }

    #[test]
    fn test_proactive_memory_overrides_per_field_disable() {
        // The issue's main use case: cron sub-agents disable memorize
        // while the global remains on for the user-facing agent.
        let global = ProactiveMemoryConfig::default();
        let overrides = ProactiveMemoryOverrides {
            auto_memorize: Some(false),
            ..Default::default()
        };
        assert!(!overrides.resolve_auto_memorize(&global));
        assert!(
            overrides.resolve_auto_retrieve(&global),
            "retrieve untouched when only auto_memorize override is set"
        );
    }

    #[test]
    fn test_proactive_memory_overrides_master_switch_disables_both() {
        let global = ProactiveMemoryConfig::default();
        let overrides = ProactiveMemoryOverrides {
            enabled: Some(false),
            auto_memorize: Some(true), // Set but should be ignored.
            auto_retrieve: Some(true),
            extraction_model: None,
            session_scoped_recall: None,
            min_similarity: None,
            allow_self_consolidation: None,
        };
        assert!(
            !overrides.resolve_auto_memorize(&global),
            "enabled=false wins over per-field auto_memorize=true"
        );
        assert!(
            !overrides.resolve_auto_retrieve(&global),
            "enabled=false wins over per-field auto_retrieve=true"
        );
    }

    #[test]
    fn test_proactive_memory_overrides_extraction_model_resolution() {
        // #5475: agent override wins over global, global wins over None.
        let mut global = ProactiveMemoryConfig::default();
        let none_override = ProactiveMemoryOverrides::default();
        assert_eq!(
            none_override.resolve_extraction_model(&global),
            None,
            "no override, no global → None"
        );

        global.extraction_model = Some("openai/gpt-4o-mini".to_string());
        assert_eq!(
            none_override.resolve_extraction_model(&global),
            Some("openai/gpt-4o-mini".to_string()),
            "no override → inherit global"
        );

        let agent_override = ProactiveMemoryOverrides {
            extraction_model: Some("anthropic/claude-haiku-4-5".to_string()),
            ..Default::default()
        };
        assert_eq!(
            agent_override.resolve_extraction_model(&global),
            Some("anthropic/claude-haiku-4-5".to_string()),
            "agent override wins over global"
        );

        // Empty-string override is treated as unset (operators sometimes
        // write `extraction_model = ""` to mean "no override").
        let empty_override = ProactiveMemoryOverrides {
            extraction_model: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            empty_override.resolve_extraction_model(&global),
            Some("openai/gpt-4o-mini".to_string()),
            "empty-string override falls through to global"
        );

        // `is_empty()` accounts for the new field.
        assert!(!agent_override.is_empty());
        assert!(none_override.is_empty());
    }

    #[test]
    fn test_proactive_memory_overrides_global_disabled_inherits_off() {
        // Global says off → no override fields set → per-agent stays off.
        let global = ProactiveMemoryConfig {
            enabled: false,
            ..ProactiveMemoryConfig::default()
        };
        let overrides = ProactiveMemoryOverrides::default();
        assert!(!overrides.resolve_auto_memorize(&global));
        assert!(!overrides.resolve_auto_retrieve(&global));
    }

    /// #7808: the similarity floor resolves agent override → kernel-global →
    /// none, and — unlike `resolve_auto_retrieve` — deliberately ignores the
    /// master switch, because an agent with recall off has no recall for a
    /// floor to apply to.
    #[test]
    fn resolve_min_similarity_prefers_the_agent_over_the_deployment() {
        let mut global = ProactiveMemoryConfig::default();
        assert_eq!(
            ProactiveMemoryOverrides::default().resolve_min_similarity(&global),
            None,
            "no floor anywhere means the historical behaviour: every candidate is eligible"
        );

        global.min_similarity = Some(0.3);
        assert_eq!(
            ProactiveMemoryOverrides::default().resolve_min_similarity(&global),
            Some(0.3)
        );

        let overrides = ProactiveMemoryOverrides {
            min_similarity: Some(0.45),
            ..Default::default()
        };
        assert_eq!(overrides.resolve_min_similarity(&global), Some(0.45));

        let disabled = ProactiveMemoryOverrides {
            enabled: Some(false),
            min_similarity: Some(0.45),
            ..Default::default()
        };
        assert_eq!(
            disabled.resolve_min_similarity(&global),
            Some(0.45),
            "the master switch decides whether recall happens, not how it ranks"
        );
    }

    /// #7808: the self-consolidation opt-in has no global fallback by design —
    /// an unset override is `false`, so the capability is reachable only from
    /// the manifest of the agent that will do the deleting.
    #[test]
    fn allow_self_consolidation_defaults_to_off_with_no_global_escape_hatch() {
        assert!(
            !ProactiveMemoryOverrides::default().resolve_allow_self_consolidation(),
            "destructive consolidation must not be on by default"
        );
        assert!(!ProactiveMemoryOverrides {
            allow_self_consolidation: Some(false),
            ..Default::default()
        }
        .resolve_allow_self_consolidation());
        assert!(ProactiveMemoryOverrides {
            allow_self_consolidation: Some(true),
            ..Default::default()
        }
        .resolve_allow_self_consolidation());
    }

    /// `is_empty` short-circuits the resolve dance for the common "no override"
    /// case, so a field it forgets is a field that stops being honoured.
    #[test]
    fn is_empty_accounts_for_every_override_field() {
        assert!(ProactiveMemoryOverrides::default().is_empty());
        for populated in [
            ProactiveMemoryOverrides {
                min_similarity: Some(0.3),
                ..Default::default()
            },
            ProactiveMemoryOverrides {
                allow_self_consolidation: Some(true),
                ..Default::default()
            },
        ] {
            assert!(
                !populated.is_empty(),
                "a set field must make the overrides non-empty: {populated:?}"
            );
        }
    }

    #[test]
    fn test_proactive_memory_overrides_serde_roundtrip() {
        let overrides = ProactiveMemoryOverrides {
            enabled: None,
            auto_memorize: Some(false),
            auto_retrieve: None,
            extraction_model: Some("openai/gpt-4o-mini".to_string()),
            session_scoped_recall: None,
            min_similarity: None,
            allow_self_consolidation: None,
        };
        let toml = toml::to_string(&overrides).expect("serialize");
        // Only the set fields are emitted (skip_serializing_if on None).
        assert!(toml.contains("auto_memorize"));
        assert!(toml.contains("extraction_model"));
        assert!(toml.contains("openai/gpt-4o-mini"));
        assert!(!toml.contains("auto_retrieve"));
        assert!(!toml.contains("enabled"));
        let parsed: ProactiveMemoryOverrides = toml::from_str(&toml).expect("deserialize");
        assert_eq!(parsed.auto_memorize, Some(false));
        assert_eq!(parsed.auto_retrieve, None);
        assert_eq!(parsed.enabled, None);
        assert_eq!(
            parsed.extraction_model,
            Some("openai/gpt-4o-mini".to_string())
        );
    }

    #[test]
    fn session_scope_filter_hides_other_sessions_and_keeps_untagged_rows() {
        let tagged = |scope: &str| {
            let mut m = HashMap::new();
            m.insert(
                SESSION_SCOPE_METADATA_KEY.to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            m
        };
        let a = "11111111-1111-4111-8111-111111111111";
        let b = "22222222-2222-4222-8222-222222222222";

        assert!(
            memory_session_scope_allows_recall(&tagged(a), a),
            "a memory must surface in the session that produced it"
        );
        assert!(
            !memory_session_scope_allows_recall(&tagged(a), b),
            "regression #7605: a memory written in session A must not surface in session B"
        );
        assert!(
            memory_session_scope_allows_recall(&HashMap::new(), a),
            "rows written before #7605 carry no tag and must stay recallable"
        );
    }

    /// The chat filter exempts `MemoryLevel::User`; the session filter must
    /// not. Personal details are exactly what an extractor files as
    /// user-level, and on a public agent two sessions are two people.
    #[test]
    fn session_scope_filter_has_no_user_level_exemption() {
        let mut meta = HashMap::new();
        meta.insert(
            SESSION_SCOPE_METADATA_KEY.to_string(),
            serde_json::Value::String("session-a".to_string()),
        );
        meta.insert(
            CHAT_SCOPE_METADATA_KEY.to_string(),
            serde_json::Value::String("whatsapp:group".to_string()),
        );

        assert!(
            memory_scope_allows_recall(MemoryLevel::User.scope_str(), &meta, "whatsapp:dm"),
            "precondition: the chat filter exempts user-level memories"
        );
        assert!(
            !memory_session_scope_allows_recall(&meta, "session-b"),
            "a user-level memory from session A must still be blocked in session B"
        );
    }

    #[test]
    fn session_scoped_recall_defaults_on_and_is_overridable_per_agent() {
        let global = ProactiveMemoryConfig::default();
        assert!(
            global.session_scoped_recall,
            "sessions are isolated by default — a public agent must not leak one visitor's memories into another's turn without the operator opting into that"
        );

        let inherit = ProactiveMemoryOverrides::default();
        assert!(
            inherit.resolve_session_scoped_recall(&global),
            "an agent that sets nothing inherits the global policy"
        );
        assert!(inherit.is_empty());

        let opt_out = ProactiveMemoryOverrides {
            session_scoped_recall: Some(false),
            ..Default::default()
        };
        assert!(
            !opt_out.resolve_session_scoped_recall(&global),
            "a single-user agent must be able to keep the pre-#7605 agent-wide memory pool"
        );
        assert!(
            !opt_out.is_empty(),
            "is_empty gates the fast path in the runtime; a set override must not look empty"
        );

        let global_off = ProactiveMemoryConfig {
            session_scoped_recall: false,
            ..Default::default()
        };
        assert!(
            !ProactiveMemoryOverrides::default().resolve_session_scoped_recall(&global_off),
            "the global off switch reaches agents that set nothing"
        );
        let opt_in = ProactiveMemoryOverrides {
            session_scoped_recall: Some(true),
            ..Default::default()
        };
        assert!(
            opt_in.resolve_session_scoped_recall(&global_off),
            "one agent must be able to isolate itself when the deployment default is off"
        );
    }

    /// `enabled = false` means the agent does no automatic recall at all, so
    /// the scoping question is moot — but it must not resolve to the
    /// *permissive* answer, or a later refactor that consults this before the
    /// enabled check would silently un-scope the agent.
    #[test]
    fn session_scoped_recall_ignores_the_master_switch() {
        let global = ProactiveMemoryConfig::default();
        let disabled = ProactiveMemoryOverrides {
            enabled: Some(false),
            ..Default::default()
        };
        assert!(disabled.resolve_session_scoped_recall(&global));
    }

    #[test]
    fn session_scoped_recall_survives_a_config_roundtrip() {
        let cfg = ProactiveMemoryConfig {
            session_scoped_recall: false,
            ..Default::default()
        };
        let toml = toml::to_string(&cfg).expect("serialize");
        let parsed: ProactiveMemoryConfig = toml::from_str(&toml).expect("deserialize");
        assert!(!parsed.session_scoped_recall);

        // An operator's config.toml predating this field must keep the
        // isolating default rather than deserializing to `false`.
        let legacy: ProactiveMemoryConfig =
            toml::from_str("auto_memorize = true\n").expect("deserialize legacy");
        assert!(legacy.session_scoped_recall);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b).expect("identical vectors are comparable");
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b).expect("orthogonal vectors are comparable");
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        // Empty vectors are not comparable — must return None, not 0.0.
        assert_eq!(cosine_similarity(&[], &[]), None);
    }

    #[test]
    fn test_cosine_similarity_length_mismatch() {
        // Dim mismatch is not comparable — must return None, not 0.0.
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), None);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        // Zero magnitude → undefined direction → None (not 0.0).
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), None);
        assert_eq!(cosine_similarity(&b, &a), None);
    }

    /// The prompt builder and the eviction cap must agree on what raw dialogue is.
    ///
    /// This is the Rust side of the predicate `SemanticStore::eviction_candidates` applies in SQL: scope `episodic`, source `Conversation`, no `category` in the metadata.
    #[test]
    fn raw_dialogue_is_the_per_turn_writer_signature() {
        let none: HashMap<String, serde_json::Value> = HashMap::new();
        assert!(memory_is_raw_dialogue(
            EPISODIC_SCOPE,
            &MemorySource::Conversation,
            &none
        ));

        // An extracted fact: `*_memory` scope, and a category either way.
        assert!(!memory_is_raw_dialogue(
            MemoryLevel::User.scope_str(),
            &MemorySource::Conversation,
            &none
        ));
        let mut categorised = HashMap::new();
        categorised.insert(
            MEMORY_CATEGORY_METADATA_KEY.to_string(),
            serde_json::json!("preference"),
        );
        assert!(!memory_is_raw_dialogue(
            EPISODIC_SCOPE,
            &MemorySource::Conversation,
            &categorised
        ));

        // An imported row lands in `episodic` with empty metadata but a different source, and the
        // eviction predicate does not class it as raw dialogue either.
        assert!(!memory_is_raw_dialogue(
            EPISODIC_SCOPE,
            &MemorySource::Document,
            &none
        ));

        // `COALESCE(json_extract(metadata, '$.category'), '') = ''`: a null and an empty string are
        // both "no category", a non-string value is not.
        for (value, expected) in [
            (serde_json::Value::Null, true),
            (serde_json::json!(""), true),
            (serde_json::json!(7), false),
        ] {
            let mut md = HashMap::new();
            md.insert(MEMORY_CATEGORY_METADATA_KEY.to_string(), value.clone());
            assert_eq!(
                memory_is_raw_dialogue(EPISODIC_SCOPE, &MemorySource::Conversation, &md),
                expected,
                "category={value:?}"
            );
        }
    }
}
