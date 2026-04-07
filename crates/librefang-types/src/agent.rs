//! Agent-related types: identity, manifests, state, and scheduling.

use crate::tool::ToolDefinition;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Metadata key for stable prefix mode flag.
pub const STABLE_PREFIX_MODE_METADATA_KEY: &str = "stable_prefix_mode";

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    /// Generate a new random UserId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for UserId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Model routing configuration — auto-selects cheap/mid/expensive models by complexity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelRoutingConfig {
    /// Model to use for simple queries.
    pub simple_model: String,
    /// Model to use for medium-complexity queries.
    pub medium_model: String,
    /// Model to use for complex queries.
    pub complex_model: String,
    /// Token count threshold: below this = simple.
    pub simple_threshold: u32,
    /// Token count threshold: above this = complex.
    pub complex_threshold: u32,
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            simple_model: "claude-haiku-4-5-20251001".to_string(),
            medium_model: "claude-sonnet-4-20250514".to_string(),
            complex_model: "claude-sonnet-4-20250514".to_string(),
            simple_threshold: 100,
            complex_threshold: 500,
        }
    }
}

/// Autonomous agent configuration — guardrails for 24/7 agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutonomousConfig {
    /// Cron expression for quiet hours (e.g., "0 22 * * *" to "0 6 * * *").
    pub quiet_hours: Option<String>,
    /// Maximum iterations per invocation (overrides global MAX_ITERATIONS).
    pub max_iterations: u32,
    /// Maximum restarts before the agent is permanently stopped.
    pub max_restarts: u32,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Per-agent heartbeat timeout override in seconds.
    /// When set, the agent is considered unresponsive after this many seconds
    /// of inactivity, instead of using `heartbeat_interval_secs * 2`.
    #[serde(default)]
    pub heartbeat_timeout_secs: Option<u32>,
    /// Per-agent override for how many recent heartbeat turns to keep
    /// when pruning NO_REPLY heartbeat messages from session context.
    #[serde(default)]
    pub heartbeat_keep_recent: Option<usize>,
    /// Channel to send heartbeat status to (e.g., "telegram", "discord").
    pub heartbeat_channel: Option<String>,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            quiet_hours: None,
            max_iterations: 50,
            max_restarts: 10,
            heartbeat_interval_secs: 30,
            heartbeat_timeout_secs: None,
            heartbeat_keep_recent: None,
            heartbeat_channel: None,
        }
    }
}

/// Hook event types that can be intercepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Fires before a tool call is executed. Handler can block the call.
    BeforeToolCall,
    /// Fires after a tool call completes.
    AfterToolCall,
    /// Fires before the system prompt is constructed.
    BeforePromptBuild,
    /// Fires after the agent loop completes.
    AgentLoopEnd,
}

/// Unique identifier for an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// A fixed namespace UUID for deriving deterministic hand-agent IDs.
    /// Generated once via UUID v4; never changes.
    const HAND_NAMESPACE: Uuid = Uuid::from_bytes([
        0x9b, 0x6a, 0xe3, 0x2d, 0x7a, 0x4f, 0x4c, 0x1e, 0x8d, 0x0f, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5,
        0xf6,
    ]);

    /// Generate a new random AgentId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Generate a deterministic AgentId for a hand agent.
    ///
    /// Uses UUID v5 (SHA-1) with a fixed namespace so the same `hand_id`
    /// always maps to the same UUID across daemon restarts.
    pub fn from_hand_id(hand_id: &str) -> Self {
        Self(Uuid::new_v5(&Self::HAND_NAMESPACE, hand_id.as_bytes()))
    }

    /// Generate a deterministic agent ID for a specific role within a multi-agent hand instance.
    ///
    /// **Backward compatibility**: when `instance_id` is `None`, uses the legacy
    /// hash format `"{hand_id}:{role}"` so that existing single-instance hands
    /// keep their original agent IDs (no orphaned cron jobs, memory keys, etc.).
    /// When `instance_id` is `Some`, uses `"{hand_id}:{role}:{instance_id}"` so
    /// that multiple instances of the same hand each get unique, deterministic IDs.
    pub fn from_hand_agent(hand_id: &str, role: &str, instance_id: Option<Uuid>) -> Self {
        let input = match instance_id {
            Some(id) => format!("{hand_id}:{role}:{id}"),
            None => format!("{hand_id}:{role}"),
        };
        Self(Uuid::new_v5(&Self::HAND_NAMESPACE, input.as_bytes()))
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AgentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Unique identifier for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new random SessionId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The current lifecycle state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent has been created but not yet started.
    Created,
    /// Agent is actively running and processing events.
    Running,
    /// Agent is paused and not processing events.
    Suspended,
    /// Agent has been terminated and cannot be resumed.
    Terminated,
    /// Agent crashed and is awaiting recovery.
    Crashed,
}

/// Permission-based operational mode for an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Read-only: agent can observe but cannot call any tools.
    Observe,
    /// Restricted: agent can only call read-only tools (file_read, file_list, memory_recall, web_fetch, web_search).
    Assist,
    /// Unrestricted: agent can use all granted tools.
    #[default]
    Full,
}

impl AgentMode {
    /// Filter a tool list based on this mode.
    pub fn filter_tools(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        match self {
            Self::Observe => vec![],
            Self::Assist => {
                let read_only = [
                    "file_read",
                    "file_list",
                    "memory_list",
                    "memory_recall",
                    "web_fetch",
                    "web_search",
                    "agent_list",
                ];
                tools
                    .into_iter()
                    .filter(|t| read_only.contains(&t.name.as_str()))
                    .collect()
            }
            Self::Full => tools,
        }
    }
}

/// How an agent is scheduled to run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleMode {
    /// Agent wakes up when a message/event arrives (default).
    #[default]
    Reactive,
    /// Agent wakes up on a cron schedule.
    Periodic { cron: String },
    /// Agent monitors conditions and acts when thresholds are met.
    Proactive { conditions: Vec<String> },
    /// Agent runs in a persistent loop.
    Continuous {
        #[serde(default = "default_check_interval")]
        check_interval_secs: u64,
    },
}

fn default_check_interval() -> u64 {
    300
}

/// Resource limits for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceQuota {
    /// Maximum WASM memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum CPU time per invocation in milliseconds.
    pub max_cpu_time_ms: u64,
    /// Maximum tool calls per minute.
    pub max_tool_calls_per_minute: u32,
    /// Maximum LLM tokens per hour.
    pub max_llm_tokens_per_hour: u64,
    /// Maximum network bytes per hour.
    pub max_network_bytes_per_hour: u64,
    /// Maximum cost in USD per hour.
    pub max_cost_per_hour_usd: f64,
    /// Maximum cost in USD per day (0.0 = unlimited).
    pub max_cost_per_day_usd: f64,
    /// Maximum cost in USD per month (0.0 = unlimited).
    pub max_cost_per_month_usd: f64,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            max_memory_bytes: 256 * 1024 * 1024, // 256 MB
            max_cpu_time_ms: 30_000,             // 30 seconds
            max_tool_calls_per_minute: 60,
            max_llm_tokens_per_hour: 0, // unlimited by default
            max_network_bytes_per_hour: 100 * 1024 * 1024, // 100 MB
            max_cost_per_hour_usd: 0.0, // unlimited by default
            max_cost_per_day_usd: 0.0,  // unlimited
            max_cost_per_month_usd: 0.0, // unlimited
        }
    }
}

/// Agent priority level for scheduling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    /// Low priority.
    Low = 0,
    /// Normal priority (default).
    #[default]
    Normal = 1,
    /// High priority.
    High = 2,
    /// Critical priority.
    Critical = 3,
}

/// Named tool presets — expand to tool lists + derived capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolProfile {
    Minimal,
    Coding,
    Research,
    Messaging,
    Automation,
    #[default]
    Full,
    Custom,
}

impl ToolProfile {
    /// Expand profile to tool name list.
    pub fn tools(&self) -> Vec<String> {
        match self {
            Self::Minimal => vec!["file_read", "file_list"],
            Self::Coding => vec![
                "file_read",
                "file_write",
                "file_list",
                "shell_exec",
                "web_fetch",
            ],
            Self::Research => vec!["web_fetch", "web_search", "file_read", "file_write"],
            Self::Messaging => vec![
                "agent_send",
                "agent_list",
                "memory_store",
                "memory_list",
                "memory_recall",
            ],
            Self::Automation => vec![
                "file_read",
                "file_write",
                "file_list",
                "shell_exec",
                "web_fetch",
                "web_search",
                "agent_send",
                "agent_list",
                "memory_store",
                "memory_list",
                "memory_recall",
            ],
            Self::Full | Self::Custom => vec!["*"],
        }
        .into_iter()
        .map(String::from)
        .collect()
    }

    /// Derive ManifestCapabilities implied by this profile.
    pub fn implied_capabilities(&self) -> ManifestCapabilities {
        let tools = self.tools();
        let has_net = tools.iter().any(|t| t.starts_with("web_") || t == "*");
        let has_shell = tools.iter().any(|t| t == "shell_exec" || t == "*");
        let has_agent = tools.iter().any(|t| t.starts_with("agent_") || t == "*");
        let has_memory = tools.iter().any(|t| t.starts_with("memory_") || t == "*");
        ManifestCapabilities {
            tools,
            network: if has_net { vec!["*".into()] } else { vec![] },
            shell: if has_shell { vec!["*".into()] } else { vec![] },
            agent_spawn: has_agent,
            agent_message: if has_agent { vec!["*".into()] } else { vec![] },
            memory_read: if has_memory {
                vec!["*".into()]
            } else {
                vec!["self.*".into()]
            },
            memory_write: vec!["self.*".into()],
            ofp_discover: false,
            ofp_connect: vec![],
        }
    }
}

/// LLM model configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// LLM provider name.
    pub provider: String,
    /// Model identifier.
    #[serde(alias = "name")]
    pub model: String,
    /// Maximum tokens for completion.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// System prompt for the agent.
    pub system_prompt: String,
    /// Optional API key environment variable name.
    pub api_key_env: Option<String>,
    /// Optional base URL override for the provider.
    pub base_url: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "default".to_string(),
            model: "default".to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            system_prompt: "You are a helpful AI agent.".to_string(),
            api_key_env: None,
            base_url: None,
        }
    }
}

/// A fallback model entry in a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackModel {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Tool configuration within an agent manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    /// Tool-specific configuration parameters.
    pub params: HashMap<String, serde_json::Value>,
}

/// Complete agent manifest — defines everything about an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentManifest {
    /// Human-readable agent name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Description of what this agent does.
    pub description: String,
    /// Author identifier.
    pub author: String,
    /// Path to the agent module (WASM or Python file).
    pub module: String,
    /// Scheduling mode.
    pub schedule: ScheduleMode,
    /// LLM model configuration.
    pub model: ModelConfig,
    /// Fallback model chain — tried in order if the primary model fails.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub fallback_models: Vec<FallbackModel>,
    /// Resource quotas.
    pub resources: ResourceQuota,
    /// Priority level.
    pub priority: Priority,
    /// Capability grants (parsed into Capability enum by kernel).
    pub capabilities: ManifestCapabilities,
    /// Named tool profile — expands to tool list + derived capabilities.
    #[serde(default)]
    pub profile: Option<ToolProfile>,
    /// Tool-specific configurations.
    #[serde(default, deserialize_with = "crate::serde_compat::map_lenient")]
    pub tools: HashMap<String, ToolConfig>,
    /// Installed skill references (empty = all skills available).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub skills: Vec<String>,
    /// Explicitly disable all skills, overriding the empty-list = all-skills default.
    #[serde(default)]
    pub skills_disabled: bool,
    /// MCP server allowlist (empty = all connected MCP servers available).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub mcp_servers: Vec<String>,
    /// Custom metadata.
    #[serde(default, deserialize_with = "crate::serde_compat::map_lenient")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Tags for agent discovery and categorization.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tags: Vec<String>,
    /// Model routing configuration — auto-select models by complexity.
    #[serde(default)]
    pub routing: Option<ModelRoutingConfig>,
    /// Autonomous agent configuration — guardrails for 24/7 agents.
    #[serde(default)]
    pub autonomous: Option<AutonomousConfig>,
    /// Pinned model override (used in Stable mode).
    #[serde(default)]
    pub pinned_model: Option<String>,
    /// Agent workspace directory. Auto-created on spawn.
    /// Default: `{workspaces_dir}/{agent_name}-{agent_id_prefix}/`
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// Whether to generate workspace identity files (SOUL.md, USER.md, etc.) on creation.
    #[serde(default = "default_true")]
    pub generate_identity_files: bool,
    /// Per-agent exec policy override. If None, uses global exec_policy.
    /// Accepts string shorthand ("allow", "deny", "full", "allowlist") or full table.
    #[serde(default, deserialize_with = "crate::serde_compat::exec_policy_lenient")]
    pub exec_policy: Option<crate::config::ExecPolicy>,
    /// Tool allowlist — only these tools are available (empty = all tools).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tool_allowlist: Vec<String>,
    /// Tool blocklist — these tools are excluded (applied after allowlist).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tool_blocklist: Vec<String>,
    /// Explicitly disable all tools, overriding profile / capability / filter expansion.
    #[serde(default)]
    pub tools_disabled: bool,
    /// Desired LLM response format (structured output).
    ///
    /// When set, the agent loop passes this to the LLM driver so the model
    /// returns output in the requested format (plain JSON or schema-constrained).
    #[serde(default)]
    pub response_format: Option<crate::config::ResponseFormat>,
    /// Whether this agent is enabled. Disabled agents are not spawned on startup.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Plugin allowlist — only these plugins are available (empty = all plugins).
    /// When set, only plugins whose names appear in this list will be loaded
    /// for this agent. When empty (default), all installed plugins are available.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub allowed_plugins: Vec<String>,
    /// Whether this agent inherits context from the parent workflow when
    /// executed as a subagent. When true (default), previous step outputs
    /// are prepended to the prompt. Set to false to run steps in isolation.
    #[serde(default = "default_true")]
    pub inherit_parent_context: bool,
    /// Per-agent extended thinking configuration.
    /// Overrides the global `[thinking]` config when set.
    #[serde(default)]
    pub thinking: Option<crate::config::ThinkingConfig>,
    /// Per-agent context injections. Merged with global `session.context_injection`
    /// entries when a session starts. Agent-level injections are appended after
    /// global ones within each position group.
    #[serde(default)]
    pub context_injection: Vec<crate::config::ContextInjection>,
    /// Whether this agent was spawned by a Hand. Persisted in the manifest so
    /// it survives restarts without requiring tag-based detection.
    #[serde(default)]
    pub is_hand: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AgentManifest {
    fn default() -> Self {
        Self {
            name: "unnamed".to_string(),
            version: crate::VERSION.to_string(),
            description: String::new(),
            author: String::new(),
            module: "builtin:chat".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: Vec::new(),
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: Vec::new(),
            skills_disabled: false,
            mcp_servers: Vec::new(),
            metadata: HashMap::new(),
            tags: Vec::new(),
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            exec_policy: None,
            tool_allowlist: Vec::new(),
            tool_blocklist: Vec::new(),
            tools_disabled: false,
            response_format: None,
            enabled: true,
            allowed_plugins: Vec::new(),
            inherit_parent_context: true,
            thinking: None,
            context_injection: Vec::new(),
            is_hand: false,
        }
    }
}

/// Capability declarations in a manifest (human-readable TOML format).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ManifestCapabilities {
    /// Allowed network hosts (e.g., ["api.anthropic.com:443"]).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub network: Vec<String>,
    /// Allowed tool IDs.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tools: Vec<String>,
    /// Memory read scopes.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub memory_read: Vec<String>,
    /// Memory write scopes.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub memory_write: Vec<String>,
    /// Whether this agent can spawn sub-agents.
    pub agent_spawn: bool,
    /// Agent message patterns (e.g., ["*"] or ["agent-name"]).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub agent_message: Vec<String>,
    /// Allowed shell commands.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub shell: Vec<String>,
    /// Whether this agent can discover remote agents via OFP.
    pub ofp_discover: bool,
    /// Allowed OFP peer patterns.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub ofp_connect: Vec<String>,
}

/// Human-readable session label (e.g., "support inbox", "research").
/// Max 128 chars, alphanumeric + spaces + hyphens + underscores only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SessionLabel(String);

impl SessionLabel {
    /// Create a new validated session label.
    pub fn new(label: &str) -> Result<Self, crate::error::LibreFangError> {
        let trimmed = label.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err(crate::error::LibreFangError::InvalidInput(
                "Session label must be 1-128 chars".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        {
            return Err(crate::error::LibreFangError::InvalidInput(
                "Session label contains invalid chars".into(),
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Get the label as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Visual identity for an agent — emoji, avatar, color, personality.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentIdentity {
    /// Single emoji character for quick visual identification.
    pub emoji: Option<String>,
    /// Avatar URL (http/https) or data URI.
    pub avatar_url: Option<String>,
    /// Hex color code (e.g., "#FF5C00") for UI accent.
    pub color: Option<String>,
    /// Archetype: "researcher", "coder", "assistant", "writer", "devops", "support", "analyst".
    pub archetype: Option<String>,
    /// Personality vibe: "professional", "friendly", "technical", "creative", "concise", "mentor".
    pub vibe: Option<String>,
    /// Greeting style: "warm", "formal", "playful", "brief".
    pub greeting_style: Option<String>,
}

/// A registered agent entry in the kernel's registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    /// Unique agent ID.
    pub id: AgentId,
    /// Account (tenant) ID for multi-tenant isolation. None = legacy/system, Some = scoped tenant.
    pub account_id: Option<String>,
    /// Human-readable name.
    pub name: String,
    /// Full manifest.
    pub manifest: AgentManifest,
    /// Current lifecycle state.
    pub state: AgentState,
    /// Permission-based operational mode.
    #[serde(default)]
    pub mode: AgentMode,
    /// When the agent was created.
    pub created_at: DateTime<Utc>,
    /// When the agent was last active.
    pub last_active: DateTime<Utc>,
    /// Parent agent (if spawned by another agent).
    pub parent: Option<AgentId>,
    /// Child agents spawned by this agent.
    pub children: Vec<AgentId>,
    /// Active session ID.
    pub session_id: SessionId,
    /// Original TOML manifest path, if this agent was spawned from disk.
    #[serde(default)]
    pub source_toml_path: Option<PathBuf>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Visual identity for dashboard display.
    #[serde(default)]
    pub identity: AgentIdentity,
    /// Whether onboarding (bootstrap) has been completed.
    #[serde(default)]
    pub onboarding_completed: bool,
    /// When onboarding was completed.
    #[serde(default)]
    pub onboarding_completed_at: Option<DateTime<Utc>>,
    /// Whether this agent was spawned by a Hand (true) or is a regular agent (false).
    #[serde(default)]
    pub is_hand: bool,
}

/// A stored prompt version for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptVersion {
    #[serde(default)]
    pub id: Uuid,
    #[serde(default)]
    pub agent_id: AgentId,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub content_hash: String,
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub variables: Vec<String>,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub is_active: bool,
    pub description: Option<String>,
}

/// Success criteria for an A/B experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessCriteria {
    #[serde(default)]
    pub require_user_helpful: bool,
    #[serde(default)]
    pub require_no_tool_errors: bool,
    #[serde(default)]
    pub require_non_empty: bool,
    pub custom_min_score: Option<u8>,
}

impl Default for SuccessCriteria {
    fn default() -> Self {
        Self {
            require_user_helpful: true,
            require_no_tool_errors: true,
            require_non_empty: true,
            custom_min_score: None,
        }
    }
}

/// A/B experiment for prompt testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExperiment {
    #[serde(default)]
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub agent_id: AgentId,
    #[serde(default)]
    pub status: ExperimentStatus,
    #[serde(default)]
    pub traffic_split: Vec<u8>,
    #[serde(default)]
    pub success_criteria: SuccessCriteria,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub variants: Vec<ExperimentVariant>,
}

/// Variant within an experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentVariant {
    #[serde(default)]
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub prompt_version_id: Uuid,
    pub description: Option<String>,
}

/// Experiment status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExperimentStatus {
    #[default]
    Draft,
    Running,
    Paused,
    Completed,
}

/// Metrics per experiment variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentVariantMetrics {
    pub variant_id: Uuid,
    pub variant_name: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    pub avg_cost_usd: f64,
    pub total_cost_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_uniqueness() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_agent_id_display() {
        let id = AgentId::new();
        let display = format!("{}", id);
        assert!(!display.is_empty());
        assert_eq!(display.len(), 36); // UUID v4 string length
    }

    #[test]
    fn test_agent_id_from_hand_id_deterministic() {
        let a = AgentId::from_hand_id("browser");
        let b = AgentId::from_hand_id("browser");
        assert_eq!(a, b, "same hand_id must produce same AgentId");
    }

    #[test]
    fn test_agent_id_from_hand_id_differs_per_hand() {
        let a = AgentId::from_hand_id("browser");
        let b = AgentId::from_hand_id("coder");
        assert_ne!(a, b, "different hand_ids must produce different AgentIds");
    }

    #[test]
    fn test_agent_id_serialization() {
        let id = AgentId::new();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_default_resource_quota() {
        let quota = ResourceQuota::default();
        assert_eq!(quota.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(quota.max_cpu_time_ms, 30_000);
    }

    #[test]
    fn test_user_id_uniqueness() {
        let u1 = UserId::new();
        let u2 = UserId::new();
        assert_ne!(u1, u2);
    }

    #[test]
    fn test_user_id_roundtrip() {
        let u = UserId::new();
        let json = serde_json::to_string(&u).unwrap();
        let back: UserId = serde_json::from_str(&json).unwrap();
        assert_eq!(u, back);
    }

    #[test]
    fn test_model_routing_config_defaults() {
        let cfg = ModelRoutingConfig::default();
        assert!(!cfg.simple_model.is_empty());
        assert!(cfg.simple_threshold < cfg.complex_threshold);
    }

    #[test]
    fn test_model_routing_config_serde() {
        let cfg = ModelRoutingConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ModelRoutingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.simple_model, cfg.simple_model);
    }

    #[test]
    fn test_autonomous_config_defaults() {
        let cfg = AutonomousConfig::default();
        assert_eq!(cfg.max_iterations, 50);
        assert_eq!(cfg.max_restarts, 10);
        assert_eq!(cfg.heartbeat_interval_secs, 30);
        assert!(cfg.quiet_hours.is_none());
    }

    #[test]
    fn test_autonomous_config_serde() {
        let cfg = AutonomousConfig {
            quiet_hours: Some("0 22 * * *".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AutonomousConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.quiet_hours, Some("0 22 * * *".to_string()));
    }

    #[test]
    fn test_manifest_with_routing_and_autonomous() {
        let manifest = AgentManifest {
            routing: Some(ModelRoutingConfig::default()),
            autonomous: Some(AutonomousConfig::default()),
            pinned_model: Some("claude-sonnet-4-20250514".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert!(back.routing.is_some());
        assert!(back.autonomous.is_some());
        assert_eq!(
            back.pinned_model,
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_agent_manifest_serialization() {
        let manifest = AgentManifest {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            author: "test".to_string(),
            module: "test.wasm".to_string(),
            tags: vec!["test".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-agent");
        assert_eq!(deserialized.tags, vec!["test".to_string()]);
    }

    // ----- ToolProfile tests -----

    #[test]
    fn test_tool_profile_minimal() {
        let tools = ToolProfile::Minimal.tools();
        assert_eq!(tools, vec!["file_read", "file_list"]);
    }

    #[test]
    fn test_tool_profile_coding() {
        let tools = ToolProfile::Coding.tools();
        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"shell_exec".to_string()));
        assert!(tools.contains(&"web_fetch".to_string()));
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_tool_profile_research() {
        let tools = ToolProfile::Research.tools();
        assert!(tools.contains(&"web_fetch".to_string()));
        assert!(tools.contains(&"web_search".to_string()));
        assert_eq!(tools.len(), 4);
    }

    #[test]
    fn test_tool_profile_messaging() {
        let tools = ToolProfile::Messaging.tools();
        assert!(tools.contains(&"agent_send".to_string()));
        assert!(tools.contains(&"memory_recall".to_string()));
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_tool_profile_automation() {
        let tools = ToolProfile::Automation.tools();
        assert_eq!(tools.len(), 11);
    }

    #[test]
    fn test_tool_profile_full() {
        let tools = ToolProfile::Full.tools();
        assert_eq!(tools, vec!["*"]);
    }

    #[test]
    fn test_tool_profile_implied_capabilities_coding() {
        let caps = ToolProfile::Coding.implied_capabilities();
        assert!(caps.network.contains(&"*".to_string())); // web_fetch
        assert!(caps.shell.contains(&"*".to_string())); // shell_exec
        assert!(!caps.agent_spawn); // no agent_* tools
        assert!(caps.agent_message.is_empty());
    }

    #[test]
    fn test_tool_profile_implied_capabilities_messaging() {
        let caps = ToolProfile::Messaging.implied_capabilities();
        assert!(caps.network.is_empty());
        assert!(caps.shell.is_empty());
        assert!(caps.agent_spawn);
        assert!(caps.agent_message.contains(&"*".to_string()));
        assert!(caps.memory_read.contains(&"*".to_string()));
    }

    #[test]
    fn test_tool_profile_implied_capabilities_minimal() {
        let caps = ToolProfile::Minimal.implied_capabilities();
        assert!(caps.network.is_empty());
        assert!(caps.shell.is_empty());
        assert!(!caps.agent_spawn);
        assert_eq!(caps.memory_read, vec!["self.*".to_string()]);
    }

    #[test]
    fn test_tool_profile_serde_roundtrip() {
        let profile = ToolProfile::Coding;
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(json, "\"coding\"");
        let back: ToolProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolProfile::Coding);
    }

    // ----- AgentMode tests -----

    #[test]
    fn test_agent_mode_default() {
        assert_eq!(AgentMode::default(), AgentMode::Full);
    }

    #[test]
    fn test_agent_mode_observe_filters_all() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Observe.filter_tools(tools);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_agent_mode_assist_filters_write_tools() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "file_write".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "web_fetch".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "memory_recall".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Assist.filter_tools(tools);
        assert_eq!(filtered.len(), 3);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"web_fetch"));
        assert!(names.contains(&"memory_recall"));
        assert!(!names.contains(&"file_write"));
        assert!(!names.contains(&"shell_exec"));
    }

    #[test]
    fn test_agent_mode_full_passes_all() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Full.filter_tools(tools);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_agent_mode_serde_roundtrip() {
        let mode = AgentMode::Assist;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"assist\"");
        let back: AgentMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AgentMode::Assist);
    }

    // ----- FallbackModel tests -----

    #[test]
    fn test_fallback_model_serde() {
        let fb = FallbackModel {
            provider: "groq".to_string(),
            model: "llama-3.3-70b".to_string(),
            api_key_env: Some("GROQ_API_KEY".to_string()),
            base_url: None,
        };
        let json = serde_json::to_string(&fb).unwrap();
        let back: FallbackModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "groq");
        assert_eq!(back.model, "llama-3.3-70b");
        assert_eq!(back.api_key_env, Some("GROQ_API_KEY".to_string()));
    }

    #[test]
    fn test_manifest_with_new_fields() {
        let manifest = AgentManifest {
            profile: Some(ToolProfile::Coding),
            fallback_models: vec![FallbackModel {
                provider: "groq".to_string(),
                model: "llama-3.3-70b".to_string(),
                api_key_env: None,
                base_url: None,
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.profile, Some(ToolProfile::Coding));
        assert_eq!(back.fallback_models.len(), 1);
    }

    #[test]
    fn test_agent_entry_with_mode() {
        let entry = AgentEntry {
            id: AgentId::new(),
            account_id: None,
            name: "test".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::Assist,
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: vec![],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AgentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mode, AgentMode::Assist);
    }

    #[test]
    fn test_agent_identity_default() {
        let id = AgentIdentity::default();
        assert!(id.emoji.is_none());
        assert!(id.avatar_url.is_none());
        assert!(id.color.is_none());
        assert!(id.archetype.is_none());
        assert!(id.vibe.is_none());
        assert!(id.greeting_style.is_none());
    }

    #[test]
    fn test_agent_identity_serde_roundtrip() {
        let id = AgentIdentity {
            emoji: Some("\u{1F916}".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            color: Some("#FF5C00".to_string()),
            archetype: Some("assistant".to_string()),
            vibe: Some("friendly".to_string()),
            greeting_style: Some("warm".to_string()),
        };
        let json = serde_json::to_string(&id).unwrap();
        let back: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(back.emoji, Some("\u{1F916}".to_string()));
        assert_eq!(back.color, Some("#FF5C00".to_string()));
    }

    #[test]
    fn test_agent_identity_deserialize_missing_fields() {
        // AgentIdentity should deserialize from empty JSON thanks to #[serde(default)]
        let id: AgentIdentity = serde_json::from_str("{}").unwrap();
        assert!(id.emoji.is_none());
    }

    #[test]
    fn test_agent_entry_identity_in_serde() {
        let entry = AgentEntry {
            id: AgentId::new(),
            account_id: None,
            name: "bot".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: vec![],
            identity: AgentIdentity {
                emoji: Some("\u{1F525}".to_string()),
                avatar_url: None,
                color: Some("#00FF00".to_string()),
                ..Default::default()
            },
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AgentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.identity.emoji, Some("\u{1F525}".to_string()));
        assert_eq!(back.identity.color, Some("#00FF00".to_string()));
        assert!(back.identity.avatar_url.is_none());
    }

    // ----- SessionLabel tests -----

    #[test]
    fn test_session_label_valid() {
        let label = SessionLabel::new("support inbox").unwrap();
        assert_eq!(label.as_str(), "support inbox");
    }

    #[test]
    fn test_session_label_with_hyphens_underscores() {
        let label = SessionLabel::new("my-session_2024").unwrap();
        assert_eq!(label.as_str(), "my-session_2024");
    }

    #[test]
    fn test_session_label_trims_whitespace() {
        let label = SessionLabel::new("  research  ").unwrap();
        assert_eq!(label.as_str(), "research");
    }

    #[test]
    fn test_session_label_rejects_empty() {
        assert!(SessionLabel::new("").is_err());
        assert!(SessionLabel::new("   ").is_err());
    }

    #[test]
    fn test_session_label_rejects_too_long() {
        let long = "a".repeat(129);
        assert!(SessionLabel::new(&long).is_err());
    }

    #[test]
    fn test_session_label_rejects_special_chars() {
        assert!(SessionLabel::new("hello@world").is_err());
        assert!(SessionLabel::new("path/traversal").is_err());
        assert!(SessionLabel::new("<script>").is_err());
    }

    #[test]
    fn test_session_label_serde_roundtrip() {
        let label = SessionLabel::new("test label").unwrap();
        let json = serde_json::to_string(&label).unwrap();
        let back: SessionLabel = serde_json::from_str(&json).unwrap();
        assert_eq!(label, back);
    }

    // ----- generate_identity_files field tests -----

    #[test]
    fn test_manifest_generate_identity_files_default_true() {
        let manifest = AgentManifest::default();
        assert!(manifest.generate_identity_files);
    }

    #[test]
    fn test_manifest_generate_identity_files_serde() {
        let json = r#"{"name":"test","generate_identity_files":false}"#;
        let manifest: AgentManifest = serde_json::from_str(json).unwrap();
        assert!(!manifest.generate_identity_files);
    }

    #[test]
    fn test_manifest_generate_identity_files_defaults_on_missing() {
        let json = r#"{"name":"test"}"#;
        let manifest: AgentManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.generate_identity_files);
    }

    // ----- ModelConfig alias tests -----

    #[test]
    fn test_model_config_name_alias_toml() {
        let toml_str = r#"
name = "llama-3.3-70b-versatile"
provider = "groq"
"#;
        let cfg: ModelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model, "llama-3.3-70b-versatile");
        assert_eq!(cfg.provider, "groq");
    }

    #[test]
    fn test_model_config_model_field_still_works() {
        let toml_str = r#"
model = "gpt-4o"
provider = "openai"
"#;
        let cfg: ModelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model, "gpt-4o");
        assert_eq!(cfg.provider, "openai");
    }

    // ----- Multi-line system_prompt TOML tests (wizard generateToml output) -----

    #[test]
    fn test_manifest_multiline_system_prompt_toml() {
        // This is the exact TOML format the dashboard wizard generateToml() now produces
        let toml_str = r#"
name = "brand-guardian"
module = "builtin:chat"

[model]
provider = "google"
model = "gemini-3-flash-preview"
system_prompt = """
You are Brand Guardian, an expert brand strategist.

Your Core Mission:
- Develop brand strategy including purpose, vision, mission, values
- Design complete visual identity systems
- Establish brand voice and messaging architecture

Critical Rules:
- Establish comprehensive brand foundation before tactical implementation
- Ensure all brand elements work as a cohesive system
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "brand-guardian");
        assert_eq!(manifest.model.provider, "google");
        assert_eq!(manifest.model.model, "gemini-3-flash-preview");
        assert!(manifest.model.system_prompt.contains("Brand Guardian"));
        assert!(manifest.model.system_prompt.contains("Critical Rules:"));
        // Verify newlines are preserved
        assert!(manifest.model.system_prompt.contains('\n'));
    }

    #[test]
    fn test_manifest_multiline_system_prompt_with_quotes() {
        // System prompt containing double quotes (common in persona prompts)
        let toml_str = r#"
name = "test-agent"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = """
You are a "helpful" assistant.
When users say "hello", respond warmly.
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.model.system_prompt.contains("\"helpful\""));
        assert!(manifest.model.system_prompt.contains("\"hello\""));
    }

    #[test]
    fn test_manifest_multiline_system_prompt_with_code_blocks() {
        // System prompt containing markdown-style code blocks
        let toml_str = r#"
name = "coder"

[model]
provider = "deepseek"
model = "deepseek-chat"
system_prompt = """
You are a coding assistant.

Example output format:
```python
def hello():
    print("world")
```

Always use proper indentation.
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.model.system_prompt.contains("```python"));
        assert!(manifest.model.system_prompt.contains("def hello()"));
    }

    #[test]
    fn test_manifest_single_line_system_prompt_still_works() {
        // Ensure the old single-line format still parses fine
        let toml_str = r#"
name = "simple"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are a helpful assistant."
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.model.system_prompt, "You are a helpful assistant.");
    }

    #[test]
    fn test_manifest_wizard_custom_profile_with_capabilities() {
        // Full wizard output when profile=custom with capabilities block
        let toml_str = r#"
name = "brand-guardian"
module = "builtin:chat"

[model]
provider = "google"
model = "gemini-3-flash-preview"
system_prompt = """
You are Brand Guardian.
Protect brand consistency across all touchpoints.
"""

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "brand-guardian");
        assert!(manifest.model.system_prompt.contains("Brand Guardian"));
        assert_eq!(manifest.capabilities.memory_read, vec!["*".to_string()]);
        assert_eq!(
            manifest.capabilities.memory_write,
            vec!["self.*".to_string()]
        );
    }

    #[test]
    fn test_manifest_allowed_plugins_default_empty() {
        let manifest = AgentManifest::default();
        assert!(manifest.allowed_plugins.is_empty());
    }

    #[test]
    fn test_manifest_allowed_plugins_from_toml() {
        let toml_str = r#"
name = "restricted-agent"
allowed_plugins = ["web-search", "code-exec"]

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "restricted-agent");
        assert_eq!(
            manifest.allowed_plugins,
            vec!["web-search".to_string(), "code-exec".to_string()]
        );
    }

    #[test]
    fn test_manifest_allowed_plugins_omitted_in_toml() {
        let toml_str = r#"
name = "unrestricted-agent"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.allowed_plugins.is_empty());
    }

    #[test]
    fn test_manifest_allowed_plugins_roundtrip_json() {
        let manifest = AgentManifest {
            allowed_plugins: vec!["qdrant-recall".to_string(), "web-search".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.allowed_plugins,
            vec!["qdrant-recall".to_string(), "web-search".to_string()]
        );
    }

    #[test]
    fn test_manifest_thinking_config_default_is_none() {
        let manifest = AgentManifest::default();
        assert!(manifest.thinking.is_none());
    }

    #[test]
    fn test_manifest_thinking_config_roundtrip_json() {
        let manifest = AgentManifest {
            thinking: Some(crate::config::ThinkingConfig {
                budget_tokens: 5000,
                stream_thinking: true,
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        let tc = back.thinking.unwrap();
        assert_eq!(tc.budget_tokens, 5000);
        assert!(tc.stream_thinking);
    }

    #[test]
    fn test_per_agent_thinking_overrides_global() {
        let global = crate::config::ThinkingConfig {
            budget_tokens: 10_000,
            stream_thinking: false,
        };
        let per_agent = crate::config::ThinkingConfig {
            budget_tokens: 5_000,
            stream_thinking: true,
        };

        let mut manifest = AgentManifest::default();

        // Per-agent is None → should fall back to global
        assert!(manifest.thinking.is_none());
        let resolved = manifest.thinking.clone().unwrap_or_else(|| global.clone());
        assert_eq!(resolved.budget_tokens, 10_000);

        // Per-agent is set → should override global
        manifest.thinking = Some(per_agent);
        let resolved = manifest.thinking.clone().unwrap_or(global);
        assert_eq!(resolved.budget_tokens, 5_000);
        assert!(resolved.stream_thinking);
    }
}
