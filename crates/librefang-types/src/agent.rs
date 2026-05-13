//! Agent-related types: identity, manifests, state, and scheduling.

use crate::tool::ToolDefinition;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Metadata key for stable prefix mode flag.
pub const STABLE_PREFIX_MODE_METADATA_KEY: &str = "stable_prefix_mode";

/// Stable namespace for deriving deterministic [`UserId`] values from
/// [`UserConfig::name`]. Generated once and frozen — changing this constant
/// rotates every existing `UserId` and breaks audit-log correlation across
/// the whole fleet, so it must never be changed.
pub const LIBREFANG_USER_NAMESPACE: Uuid =
    Uuid::from_u128(0x4c46_4147_5f55_5345_525f_4e53_5f76_3501);

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    /// Generate a new random UserId.
    ///
    /// Prefer [`UserId::from_name`] for users that come from configuration —
    /// random UUIDs change on every restart, which makes audit-log
    /// correlation across daemon restarts impossible.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Derive a stable UserId from a user's configured name.
    ///
    /// Uses UUID v5 with [`LIBREFANG_USER_NAMESPACE`] so the same name
    /// always maps to the same id — across restarts, across config reloads,
    /// across nodes. Renaming a user produces a new id (intentionally —
    /// rename = new identity, old audit history stays attached to the old id).
    pub fn from_name(name: &str) -> Self {
        Self(Uuid::new_v5(&LIBREFANG_USER_NAMESPACE, name.as_bytes()))
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

impl AutonomousConfig {
    /// Default cap on LLM iterations per agent-loop invocation. Policy
    /// constant shared with `librefang-runtime` — see `agent_loop::MAX_ITERATIONS`
    /// which re-exports this value so the runtime's own fallback path
    /// stays in lockstep with the manifest default.
    pub const DEFAULT_MAX_ITERATIONS: u32 = 50;
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            quiet_hours: None,
            max_iterations: Self::DEFAULT_MAX_ITERATIONS,
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
    /// Fires after tool execution to allow rewriting the result string.
    /// The first handler returning Ok(Some(s)) wins; others are skipped.
    TransformToolResult,
    /// Fires before the system prompt is constructed.
    BeforePromptBuild,
    /// Fires after the agent loop completes.
    AgentLoopEnd,
}

/// Unique identifier for an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Fixed namespace UUID for all deterministic agent ID derivation.
    /// Uses a single namespace with typed prefixes to avoid collisions
    /// between agents, hands, and hand-roles sharing the same name.
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0x9b, 0x6a, 0xe3, 0x2d, 0x7a, 0x4f, 0x4c, 0x1e, 0x8d, 0x0f, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5,
        0xf6,
    ]);

    /// Generate a new random AgentId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Generate a deterministic AgentId from an agent name.
    ///
    /// Uses UUID v5 (SHA-1) so the same agent name always produces the same
    /// ID across daemon restarts, preserving session history associations.
    pub fn from_name(name: &str) -> Self {
        Self(Uuid::new_v5(
            &Self::NAMESPACE,
            format!("agent:{name}").as_bytes(),
        ))
    }

    /// Generate a deterministic AgentId for a hand.
    ///
    /// Uses UUID v5 with a `hand:` prefix so the same `hand_id`
    /// always maps to the same UUID across daemon restarts.
    pub fn from_hand_id(hand_id: &str) -> Self {
        // Backward compat: existing hands used bare hand_id without prefix.
        // Keep the same input to preserve existing IDs.
        Self(Uuid::new_v5(&Self::NAMESPACE, hand_id.as_bytes()))
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
        Self(Uuid::new_v5(&Self::NAMESPACE, input.as_bytes()))
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

/// Fixed UUID v5 namespace for deriving per-channel session IDs.
/// Generated once via `uuidgen`, never changes — ensures deterministic session
/// keys across restarts. Intentionally NOT an RFC 4122 well-known namespace
/// (DNS/URL/OID/X500) to avoid collisions with other UUID v5 consumers.
const CHANNEL_SESSION_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0xa3, 0x4e, 0x7c, 0x01, 0x8f, 0x2b, 0x4d, 0x6a, 0x91, 0x5c, 0xd7, 0x3e, 0xf4, 0x0a, 0xb8, 0x52,
]);

/// Distinct UUID v5 namespace for per-fire cron session IDs. Disjoint from
/// `CHANNEL_SESSION_NAMESPACE` so a `for_cron_run` id can never collide with
/// a `for_channel` id even if input strings happen to coincide.
const CRON_RUN_SESSION_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x7e, 0x91, 0x2c, 0x4f, 0xb5, 0xa3, 0x48, 0xd1, 0xa0, 0x6c, 0xe2, 0x83, 0x1f, 0x57, 0xc4, 0x09,
]);

impl SessionId {
    /// Create a new random SessionId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Derive a deterministic session ID from an agent ID and channel name.
    ///
    /// Uses UUID v5 (SHA-1 based) so the same `(agent_id, channel)` pair always
    /// produces the same `SessionId`, even across process restarts.
    pub fn for_channel(agent_id: AgentId, channel: &str) -> Self {
        let name = format!("{}:{}", agent_id.0, channel.to_lowercase());
        Self(uuid::Uuid::new_v5(
            &CHANNEL_SESSION_NAMESPACE,
            name.as_bytes(),
        ))
    }

    /// Derive the per-channel `SessionId` for a `(channel, chat_id)` pair
    /// using the canonical scope construction shared by the kernel's inbound
    /// resolver and the channel-bridge `/new` / `/reboot` / `/compact`
    /// commands. Empty `chat_id` collapses to channel-only — matching what
    /// `build_sender_context` does when `sender.platform_id` is empty.
    ///
    /// This is the single source of truth for the scope formula. The
    /// inbound message resolver (`kernel/messaging.rs::send_message_full`,
    /// the dispatch resolver in `kernel/mod.rs::resolve_dispatch_session_id`,
    /// and the agent-execution path in `kernel/agent_execution.rs`) plus
    /// the channel-bridge reset helper in `librefang-api::channel_bridge`
    /// all call this so they cannot drift. Without this helper, the
    /// scope-derivation literal lived inline in 4 places — any one of
    /// them silently disagreeing would re-introduce #4868 (channel `/new`
    /// deleting the wrong sid).
    pub fn for_sender_scope(agent_id: AgentId, channel: &str, chat_id: Option<&str>) -> Self {
        let scope = match chat_id {
            Some(cid) if !cid.is_empty() => format!("{channel}:{cid}"),
            _ => channel.to_string(),
        };
        Self::for_channel(agent_id, &scope)
    }

    /// Derive a per-fire cron session id keyed by `(agent, run_key)`.
    ///
    /// Used when a cron job is configured with `session_mode = "new"` and
    /// each fire must land on its own isolated session — prior fires must
    /// not leak context into the current run, and the agent's persistent
    /// `(agent, "cron")` session must stay untouched.
    ///
    /// `run_key` should uniquely identify the fire (typical choice:
    /// `"<job_uuid>:<rfc3339_timestamp>"`). Lower-cased before hashing so
    /// a caller's case quirks don't fan id space. Determinism (vs.
    /// `SessionId::new()`) makes a fire's session id reproducible from logs
    /// for debugging.
    pub fn for_cron_run(agent_id: AgentId, run_key: &str) -> Self {
        let name = format!("{}:{}", agent_id.0, run_key.to_lowercase());
        Self(uuid::Uuid::new_v5(
            &CRON_RUN_SESSION_NAMESPACE,
            name.as_bytes(),
        ))
    }

    /// Derive a session ID from a structured (channel, account, conversation) key.
    ///
    /// Backward compatible with `for_channel`: when `account` is empty, the
    /// resulting id is identical to `for_channel(agent, channel)` if
    /// `conversation` is also empty, or to `for_channel(agent, format!("{channel}:{conversation}"))`
    /// when only `conversation` is set. This preserves existing session ids for
    /// channels that never carried an account dimension.
    ///
    /// When `account` is non-empty, a `v2:` byte prefix is mixed in so the
    /// hash space is disjoint from the legacy format — avoids collisions even
    /// if a real channel/account ever lines up textually with a legacy scope.
    pub fn from_route_key(
        agent_id: AgentId,
        channel: &str,
        account: &str,
        conversation: &str,
    ) -> Self {
        if account.is_empty() {
            let scope = if conversation.is_empty() {
                channel.to_string()
            } else {
                format!("{channel}:{conversation}")
            };
            return Self::for_channel(agent_id, &scope);
        }
        let name = format!(
            "v2:{}:{}:{}:{}",
            agent_id.0,
            channel.to_lowercase(),
            account.to_lowercase(),
            conversation.to_lowercase()
        );
        Self(uuid::Uuid::new_v5(
            &CHANNEL_SESSION_NAMESPACE,
            name.as_bytes(),
        ))
    }
}

impl std::str::FromStr for SessionId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(s).map(SessionId)
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

/// Scope passed to `reset_session` / `reboot_session` so callers can choose
/// between agent-wide nuke and single-session reset (#4868).
///
/// Channel `/new` / `/reboot` use `Session(for_channel(agent, channel:chat))`
/// so resetting in one chat does not wipe transcripts on every other surface
/// the same agent serves. Dashboard / explicit "reset agent" surfaces use
/// `Agent` to preserve the historical full-wipe semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetScope {
    /// Reset every session belonging to this agent (default + per-channel).
    Agent,
    /// Reset exactly one session id; sibling sessions are untouched.
    Session(SessionId),
}

/// Snapshot of a single in-flight (agent, session) loop, returned by
/// `GET /api/agents/{id}/runtime`.
///
/// The state field is intentionally a single `Running` variant for now —
/// fine-grained sub-states (`WaitingLLM` / `ExecutingTool(name)`) require
/// the agent loop to write back its current step, which is a separate
/// follow-up. The wire format leaves room for that without a breaking
/// change: deserialisers should treat unknown variants as opaque.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningSessionSnapshot {
    /// The session that's currently executing.
    pub session_id: SessionId,
    /// When the loop was spawned.
    pub started_at: DateTime<Utc>,
    /// Coarse-grained execution state.
    #[serde(default)]
    pub state: RunningSessionState,
}

/// Coarse-grained execution state for a `RunningSessionSnapshot`. Only
/// `Running` is emitted today; the enum exists so callers can pattern-match
/// instead of hard-coding strings, and so future fine-grained states slot
/// in without breaking the wire format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningSessionState {
    /// Loop has been spawned and not yet completed.
    #[default]
    Running,
}

/// How sessions are resolved for non-channel (automated) invocations.
///
/// Controls whether background ticks, triggers, and `agent_send` calls
/// reuse the agent's persistent session or create a fresh one each time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Reuse the agent's persistent session (default, backward-compatible).
    #[default]
    Persistent,
    /// Create a fresh session for each invocation.
    New,
}

/// Web search augmentation mode.
///
/// Controls whether the agent loop automatically searches the web using the
/// user's message and injects results into the LLM context before the call.
/// This enables models that don't support tool/function calling (e.g. Ollama
/// Gemma4) to benefit from web search without needing to invoke the `web_search` tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchAugmentationMode {
    /// Disabled.
    Off,
    /// Augment only when the model catalog reports `supports_tools == false` (default).
    #[default]
    Auto,
    /// Always search the web before every LLM call.
    Always,
}

/// The current lifecycle state of an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent has been created but not yet started.
    #[default]
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
    ///
    /// - `None` = not configured (inherit global default from `[budget]`).
    /// - `Some(0)` = explicitly unlimited.
    /// - `Some(n)` = limit to `n` tokens per hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_llm_tokens_per_hour: Option<u64>,
    /// Fraction of the hourly token budget allowed in any single minute.
    ///
    /// - `None` = not configured (uses compiled default `0.2`, i.e. 1/5 of hourly budget).
    /// - `Some(r)` = allow `r × max_llm_tokens_per_hour` tokens per minute.
    ///
    /// Clamped to `0.01..=1.0` at enforcement time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burst_ratio: Option<f32>,
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
            max_llm_tokens_per_hour: None, // inherit global default
            burst_ratio: None,             // inherit compiled default (0.2 = 1/5)
            max_network_bytes_per_hour: 100 * 1024 * 1024, // 100 MB
            max_cost_per_hour_usd: 0.0,    // unlimited by default
            max_cost_per_day_usd: 0.0,     // unlimited
            max_cost_per_month_usd: 0.0,   // unlimited
        }
    }
}

impl ResourceQuota {
    /// Return the effective hourly token limit as a plain `u64`.
    ///
    /// * `None` and `Some(0)` both yield `0` (unlimited).
    /// * `Some(n)` yields `n`.
    ///
    /// Callers that enforce the limit should skip enforcement when the
    /// returned value is `0`.
    pub fn effective_token_limit(&self) -> u64 {
        self.max_llm_tokens_per_hour.unwrap_or(0)
    }

    /// Resolved burst ratio: agent override > global default > 0.2.
    /// Clamped to [0.01, 1.0]. NaN/Inf fall back to 0.2.
    pub fn effective_burst_ratio(&self, global_default: f32) -> f32 {
        let raw = self.burst_ratio.unwrap_or(if global_default > 0.0 {
            global_default
        } else {
            0.2
        });
        if !raw.is_finite() {
            return 0.2;
        }
        raw.clamp(0.01, 1.0)
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
                "channel_send",
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
                "channel_send",
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
    /// Optional override for this model's context window (in tokens).
    ///
    /// When set, takes precedence over registry / runtime-probed values.
    /// Use it to force a value when the model's actual context differs
    /// from what the catalog reports (e.g. a self-hosted Ollama model
    /// configured with `num_ctx` smaller than the model's nominal
    /// length, or a vLLM endpoint with a custom `--max-model-len`).
    ///
    /// `None` (the default) means "let the runtime resolve it from the
    /// registry, persisted cache, or live `/v1/models` probe".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Optional override for this model's maximum output tokens.
    ///
    /// Same precedence and semantics as [`Self::context_window`]. Useful
    /// when a self-hosted endpoint advertises a smaller usable cap than
    /// the catalog default (e.g. a quantised checkpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Provider-specific extension parameters that are flattened directly
    /// into the API request body.
    ///
    /// For example, Qwen 3.6's `enable_memory` parameter for agent memory
    /// support. When serialized, these keys are merged into the top-level
    /// API request body via `#[serde(flatten)]`. If a key conflicts with a
    /// standard field (e.g. `temperature`), the `extra_params` value takes
    /// precedence because it is serialized last.
    #[serde(default, flatten)]
    pub extra_params: std::collections::HashMap<String, serde_json::Value>,
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
            context_window: None,
            max_output_tokens: None,
            extra_params: std::collections::HashMap::new(),
        }
    }
}

/// A fallback model entry in a chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackModel {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Provider-specific extension parameters that are flattened directly
    /// into the API request body.
    #[serde(default, flatten)]
    pub extra_params: std::collections::HashMap<String, serde_json::Value>,
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
    /// Session mode for automated (non-channel) invocations.
    /// Controls whether background ticks, triggers, and `agent_send` calls
    /// reuse the agent's persistent session or create a fresh one.
    ///
    /// Resolved session id precedence (highest first):
    /// 1. Explicit `session_id_override` from the dispatch caller (HTTP /
    ///    multi-tab UIs, fork plumbing) — always wins.
    /// 2. Per-trigger override (`Trigger.session_mode`) — honored for
    ///    event triggers and `agent_send`.
    /// 3. Channel branch — ALWAYS uses
    ///    `SessionId::for_channel(agent, "channel:chat")`, overriding both
    ///    per-trigger and manifest values whenever a non-empty
    ///    `SenderContext.channel` is present (and `use_canonical_session`
    ///    is false).
    /// 4. Cron — synthesizes `SenderContext{channel:"cron"}` and takes
    ///    the channel branch, so all cron fires for an agent share one
    ///    `(agent, "cron")` session unless the per-job `session_mode` is
    ///    set to `New`. The cron dispatcher sidesteps the channel branch
    ///    by passing an explicit `session_id_override` derived from the
    ///    job id and fire timestamp; manifest-level `session_mode = "new"`
    ///    is also consulted as a fallback when the per-job override is
    ///    unset.
    /// 5. Manifest `session_mode` — final fallback; honored by event
    ///    triggers and `agent_send` when no higher-priority signal
    ///    applies.
    ///
    /// Diagnostic: when the channel branch or the cron path overrides a
    /// non-default manifest setting, the kernel emits a
    /// `tracing::debug!` event with `resolution_source` set to
    /// `"channel-derived"`, `"cron-job-override"`, or
    /// `"cron-manifest-fallback"` so operators can grep logs to confirm
    /// why a `session_mode = "new"` declaration is or is not being
    /// honored.
    #[serde(default)]
    pub session_mode: SessionMode,
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
    /// Explicitly disable all MCP server tools for this agent. Mirrors
    /// `skills_disabled` — `mcp_servers = []` means "all", this means "none".
    ///
    /// **Scope**: this flag hides MCP tools and the MCP server summary from
    /// *this agent's* LLM prompt only. MCP servers defined in `KernelConfig`
    /// still start globally if any other agent uses them — no server process
    /// is stopped or skipped.
    ///
    /// **Hot-reload**: takes effect on the next `available_tools()` call after
    /// the per-agent tools cache is evicted. `reload_agent_from_disk` (the
    /// file-watcher path) evicts it automatically, so toggling `mcp_disabled`
    /// in `agent.toml` at runtime takes effect without an agent respawn.
    #[serde(default)]
    pub mcp_disabled: bool,
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
    /// Named shared workspaces this agent can access.
    ///
    /// Each entry maps a symbolic name to a directory path (relative to `workspaces_dir`)
    /// and an access mode. Multiple agents can declare the same path — they share the
    /// directory without identity-file collisions because identity files live in the
    /// agent's private home (`.identity/`), not in the shared workspace.
    ///
    /// ```toml
    /// [workspaces]
    /// library  = { path = "shared/library",  mode = "rw" }
    /// archive  = { path = "shared/archive",  mode = "r"  }
    /// ```
    #[serde(default)]
    pub workspaces: HashMap<String, WorkspaceDecl>,
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
    /// Web search augmentation mode — automatically search the web using the
    /// user's message and inject results into the LLM context before the call.
    /// Useful for models that don't support tool/function calling (e.g. Ollama).
    #[serde(default)]
    pub web_search_augmentation: WebSearchAugmentationMode,
    /// Whether this agent participates in background auto-dream consolidation.
    /// When true AND the global `[auto_dream] enabled = true` config is set,
    /// the scheduler periodically asks this agent to reflect on and
    /// consolidate its own memory. Off by default — opt-in per agent because
    /// consolidation costs tokens.
    #[serde(default)]
    pub auto_dream_enabled: bool,
    /// Optional override for `[auto_dream] min_hours`. When `Some`, this
    /// agent's time gate uses this value instead of the global setting —
    /// useful for heterogeneous fleets where a chatty agent wants shorter
    /// intervals and a quiet agent wants longer. `None` inherits the global.
    #[serde(default)]
    pub auto_dream_min_hours: Option<f64>,
    /// Optional override for `[auto_dream] min_sessions`. Same semantics as
    /// `auto_dream_min_hours`. `Some(0)` explicitly disables the session
    /// gate for this agent (still subject to time / lock gates).
    #[serde(default)]
    pub auto_dream_min_sessions: Option<u32>,
    /// Whether to surface tool execution progress (`🔧 tool_name`,
    /// `⚠️ tool_name failed`) inside the channel reply. When `true`
    /// (default), the streaming bridge injects short progress lines into
    /// the user-facing text so they can see what the agent is doing.
    /// Set to `false` for agents whose output should stay pristine —
    /// e.g. agents posting to public timelines, or agents whose responses
    /// are consumed by downstream parsers that would choke on the markers.
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Whether background skill evolution review runs after each turn.
    /// Defaults to `true`. Set to `false` for A2A worker agents or any
    /// agent where responsiveness to triggers matters more than automatic
    /// skill distillation — skipping evolution means the agent never holds
    /// the background LLM budget or concurrency semaphore after a turn.
    #[serde(default = "default_true")]
    pub auto_evolve: bool,
    /// Per-agent channel behavior overrides (dm_policy, group_policy, etc.).
    /// When set, these take priority over the channel-level `ChannelOverrides`
    /// for this specific agent. Follows the same pattern as `exec_policy`.
    #[serde(default)]
    pub channel_overrides: Option<crate::config::ChannelOverrides>,
    /// Per-agent override for the message-history trim cap. When set,
    /// takes precedence over `KernelConfig.max_history_messages` and the
    /// compiled-in default (`agent_loop::DEFAULT_MAX_HISTORY_MESSAGES`).
    /// `None` means inherit from kernel config / default. Values below 4
    /// are silently clamped at runtime with a warning log.
    #[serde(default)]
    pub max_history_messages: Option<usize>,
    /// Trigger-dispatch-only: cap on concurrent invocations from the
    /// kernel's event-trigger fan-out (`TaskPosted` / `MessageReceived`
    /// / …). Channel messages, cron jobs, and `agent_send` are NOT
    /// throttled by this knob — they continue to serialize at the
    /// existing per-agent / per-session locks inside `send_message_full`.
    /// Despite the unqualified field name, the scope is intentionally
    /// narrow.
    ///
    /// `None` means inherit from `KernelConfig.queue.concurrency.default_per_agent`
    /// (today: 1). `Some(1)` is identical to the legacy per-agent
    /// serialization behavior. `Some(0)` is treated as `Some(1)` (the
    /// resolver floors at 1 — `0` would deadlock the agent).
    ///
    /// Concurrent fires only make sense when each fire runs in its own
    /// session, so caps `> 1` require `session_mode = "new"` on the
    /// **manifest** (per-trigger `session_mode` overrides do NOT unlock
    /// the cap — the per-agent semaphore is sized once from the
    /// manifest default). `persistent` + cap `> 1` is auto-clamped to
    /// `1` with a `WARN` log; parallel writes to a single persistent
    /// session's history are undefined.
    ///
    /// Hot-reload: the per-agent semaphore is sized on first dispatch
    /// and is NOT invalidated by `manifest_swap`. To pick up a new cap
    /// the agent must be killed and respawned (or the daemon restarted);
    /// an in-place activate / status flip silently retains the old
    /// capacity.
    #[serde(default)]
    pub max_concurrent_invocations: Option<u32>,
    /// If true, the agent's `context.md` is read once at session start and
    /// reused. Default is `false`: the runtime re-reads `context.md` before
    /// every turn so external writers (cron jobs, integrations) reach the LLM
    /// on the next message.
    #[serde(default)]
    pub cache_context: bool,
    /// Per-agent tool-execution backend override (#3332).
    ///
    /// Resolution order: this field > `KernelConfig.tool_exec.kind` >
    /// compiled-in default (`local`). The backend selected here is used
    /// for shell / docker_exec / tool process spawns originating from
    /// this agent. Setting this to `ssh` or `daytona` requires the
    /// matching subtable in `config.toml: [tool_exec.ssh]` /
    /// `[tool_exec.daytona]` so the runtime knows where to dispatch.
    /// `None` means inherit from kernel config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_exec_backend: Option<crate::tool_exec::BackendKind>,
    /// Skill workshop (#3328): passive after-turn capture of reusable
    /// workflows from successful interactions. Default disabled — opt-in
    /// per agent via `[skill_workshop]` in `agent.toml`.
    #[serde(default)]
    pub skill_workshop: SkillWorkshopConfig,
    /// Per-agent override for the kernel-global `[proactive_memory]`
    /// policy (#4870). Each field of
    /// [`crate::memory::ProactiveMemoryOverrides`] is an `Option<bool>`
    /// that, when set, supersedes the matching field in
    /// `KernelConfig.proactive_memory` for this agent only. Default is
    /// all-`None` (inherit global). Boot caveat: the global
    /// `proactive_memory.enabled = false` still short-circuits store
    /// construction, so this knob is primarily a per-agent opt-out
    /// vector — see the struct doc for details.
    #[serde(default)]
    pub proactive_memory: crate::memory::ProactiveMemoryOverrides,
    /// Per-agent override for the kernel-global `[compaction]` policy
    /// (#4976). Each field of [`CompactionOverrides`] is an `Option`
    /// that, when set, supersedes the matching field in
    /// `KernelConfig.compaction` for this agent only. Default `None`
    /// means inherit global for every field.
    ///
    /// Use case: a chat agent with short exchanges wants aggressive
    /// pruning (low `keep_recent`), while a workflow orchestrator with
    /// long multi-tool sessions wants high `keep_recent` and a larger
    /// summary budget. The global config can no longer be one-size-
    /// fits-all once both agents share a daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionOverrides>,
}

/// Per-agent override for the kernel-global `[compaction]` configuration
/// (#4976). Mirrors the user-facing fields of
/// [`crate::config::CompactionTomlConfig`] as `Option<_>` so a manifest
/// can override individual knobs while inheriting the rest from
/// `KernelConfig.compaction`.
///
/// Internal algorithmic knobs (`base_chunk_ratio`, `safety_margin`,
/// `summarization_overhead_tokens`, …) live only on the runtime
/// `CompactionConfig` and are intentionally not exposed here — they are
/// implementation details of the summarisation algorithm, not policy.
///
/// Resolution order at compaction time:
/// 1. agent.toml `[compaction]` field (this struct) — when `Some(_)`
/// 2. config.toml `[compaction]` — global default
/// 3. compiled-in defaults inside `CompactionTomlConfig::default()`
///
/// Example in `agent.toml` (all fields optional):
/// ```toml
/// [compaction]
/// max_summary_tokens = 8192
/// keep_recent = 20
/// threshold_messages = 50
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CompactionOverrides {
    /// Override `threshold_messages` — message count that triggers compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_messages: Option<usize>,
    /// Override `keep_recent` — recent messages preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_recent: Option<usize>,
    /// Override `max_summary_tokens` — token budget for the summary output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_summary_tokens: Option<usize>,
    /// Override `token_threshold_ratio` — fraction of the model's
    /// context window that triggers token-based compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_threshold_ratio: Option<f64>,
    /// Override `max_chunk_chars` — maximum chars per summarisation chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chunk_chars: Option<usize>,
    /// Override `max_retries` — max retry attempts for LLM summarisation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

impl CompactionOverrides {
    /// Merge this per-agent override on top of the kernel-global
    /// `CompactionTomlConfig`. For each field, `Some(_)` on the override
    /// wins; `None` falls through to the global value.
    ///
    /// Returns a fresh `CompactionTomlConfig` — the global is not mutated,
    /// so different agents can produce different merged configs from the
    /// same global snapshot.
    pub fn resolve(
        &self,
        global: &crate::config::CompactionTomlConfig,
    ) -> crate::config::CompactionTomlConfig {
        crate::config::CompactionTomlConfig {
            threshold_messages: self.threshold_messages.unwrap_or(global.threshold_messages),
            keep_recent: self.keep_recent.unwrap_or(global.keep_recent),
            max_summary_tokens: self.max_summary_tokens.unwrap_or(global.max_summary_tokens),
            token_threshold_ratio: self
                .token_threshold_ratio
                .unwrap_or(global.token_threshold_ratio),
            max_chunk_chars: self.max_chunk_chars.unwrap_or(global.max_chunk_chars),
            max_retries: self.max_retries.unwrap_or(global.max_retries),
        }
    }

    /// True when no field is set — equivalent to `Default::default()`.
    /// Call sites can use this to skip the resolve dance entirely for
    /// the common "no override" case.
    pub fn is_empty(&self) -> bool {
        self.threshold_messages.is_none()
            && self.keep_recent.is_none()
            && self.max_summary_tokens.is_none()
            && self.token_threshold_ratio.is_none()
            && self.max_chunk_chars.is_none()
            && self.max_retries.is_none()
    }
}

/// Access mode for a named workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    /// Full read-write access (default).
    #[default]
    #[serde(alias = "rw", alias = "read-write")]
    ReadWrite,
    /// Read-only access — write tool calls are rejected by the kernel.
    #[serde(alias = "r", alias = "read", alias = "read-only")]
    ReadOnly,
}

/// Declaration of a named workspace in `agent.toml`.
///
/// Exactly one of `path` or `mount` must be set:
/// * `path` — relative to the configured `workspaces_dir`. The kernel
///   creates the directory if it does not exist. This is the original
///   shared-workspace mechanism and is the right choice for directories
///   that LibreFang owns.
/// * `mount` — an absolute path to a directory that already exists on
///   the host (e.g. an Obsidian vault). The kernel never creates the
///   target. The path must canonicalize to a prefix of one of the
///   `allowed_mount_roots` entries in `config.toml`; otherwise the
///   declaration is rejected at boot. See issue #3230.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceDecl {
    /// Path relative to `workspaces_dir` (e.g. `"shared/library"`).
    /// Mutually exclusive with `mount`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Absolute path to an existing host directory (e.g. an Obsidian
    /// vault). Mutually exclusive with `path`. Must be whitelisted via
    /// `config.toml: allowed_mount_roots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<PathBuf>,
    /// Access mode. Defaults to read-write.
    #[serde(default)]
    pub mode: WorkspaceMode,
}

impl WorkspaceDecl {
    /// Whether this declaration targets a directory outside `workspaces_dir`.
    /// External targets require an entry in `config.toml: allowed_mount_roots`.
    pub fn is_external_mount(&self) -> bool {
        self.mount.is_some()
    }
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
            session_mode: SessionMode::default(),
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
            mcp_disabled: false,
            metadata: HashMap::new(),
            tags: Vec::new(),
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            workspaces: HashMap::new(),
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
            web_search_augmentation: WebSearchAugmentationMode::default(),
            auto_dream_enabled: false,
            auto_dream_min_hours: None,
            auto_dream_min_sessions: None,
            show_progress: true,
            auto_evolve: true,
            max_concurrent_invocations: None,
            channel_overrides: None,
            max_history_messages: None,
            cache_context: false,
            tool_exec_backend: None,
            skill_workshop: SkillWorkshopConfig::default(),
            proactive_memory: crate::memory::ProactiveMemoryOverrides::default(),
            compaction: None,
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

/// Skill workshop configuration (#3328).
///
/// Controls passive, after-turn capture of reusable workflows from
/// successful interactions. When the user spends multiple turns teaching
/// the agent a multi-step workflow, the workshop detects the teaching
/// signal (`from now on, always X` / `no, do it like Y` / repeated tool
/// patterns) and stores a draft skill in
/// `~/.librefang/skills/pending/<agent_id>/<uuid>.toml` for review.
///
/// **Default on with the conservative knob set** (`enabled=true`,
/// `auto_capture=true`, `review_mode="heuristic"` so no LLM call,
/// `approval_policy="pending"` so no auto-promote, `max_pending=20`).
/// Candidates land in `pending/` and wait for human review via
/// `librefang skill pending approve <id>` (CLI) or the Skills page
/// (dashboard). With `approval_policy = "auto"`, the workshop still
/// writes the candidate to `pending/` first — for the audit trail and
/// to give the prompt-injection scan a place to fail loudly — and then
/// promotes it through `evolution::create_skill` and removes the
/// pending file. Either way the active skill goes through the same
/// `SkillVerifier::scan_prompt_content` gate that guards marketplace
/// skills.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillWorkshopConfig {
    /// Master switch. When `false`, the after-turn hook is a no-op for
    /// this agent (no scanning, no LLM call, no disk write).
    pub enabled: bool,
    /// When `true`, the after-turn hook runs the heuristic / LLM
    /// capture pass. When `false`, the hook short-circuits before
    /// scanning, so no new candidates are produced — the
    /// approval-side CLI / dashboard / API still work over whatever
    /// is already in `pending/`. Independent of `enabled` so an
    /// operator can pause capture on a single agent during a busy
    /// session without flipping the master switch.
    pub auto_capture: bool,
    /// What the workshop does with a positively-classified candidate.
    pub approval_policy: ApprovalPolicy,
    /// How candidates are classified.
    pub review_mode: ReviewMode,
    /// Maximum number of pending candidates retained per agent. When the
    /// limit is reached, the oldest candidate (by `captured_at`) is
    /// deleted before the new one is written. Set to `0` to skip the
    /// disk write entirely (the scanner still runs; if you want to
    /// skip the scan as well, use `auto_capture = false`).
    pub max_pending: u32,
    /// Optional time-to-live for pending candidates, in days. When set
    /// to `Some(n)` with `n > 0`, `save_candidate` prunes any pending
    /// candidate whose `captured_at` is older than `n` days before
    /// writing the new one — bounds the worst case where a user opts
    /// in but never reviews the pending tree. `None` (default) and
    /// `Some(0)` both keep the historical behaviour: candidates only
    /// age out when the per-agent `max_pending` cap evicts them by
    /// LRU. `Some(0)` is treated as "disabled" rather than "expire
    /// everything" because the latter would be a footgun for an
    /// operator who configured zero expecting the disabled meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pending_age_days: Option<u32>,
}

impl Default for SkillWorkshopConfig {
    fn default() -> Self {
        // Default-OFF — opt-in per the original #3328 acceptance
        // criteria. Operators who want the workshop set
        // `[skill_workshop] enabled = true` in `agent.toml`. The
        // remaining knobs document the conservative shape the
        // workshop takes once it IS turned on: heuristic-only review
        // (no LLM cost), pending policy (every candidate waits for
        // human approve / reject), 20-candidate cap.
        Self {
            enabled: false,
            auto_capture: true,
            approval_policy: ApprovalPolicy::Pending,
            review_mode: ReviewMode::Heuristic,
            max_pending: 20,
            // No TTL by default — the cap is the only aging mechanism
            // unless the operator opts in.
            max_pending_age_days: None,
        }
    }
}

/// What happens to a positively-classified skill workshop candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalPolicy {
    /// Default. Write the candidate to
    /// `~/.librefang/skills/pending/<agent_id>/<uuid>.toml`. The skill
    /// is not loaded into the active registry until a user approves it.
    #[default]
    Pending,
    /// Bypass review — write directly to `~/.librefang/skills/active/`
    /// and reload the skill registry. The same prompt-injection scan
    /// that gates marketplace skills still applies. Intended for trust-
    /// established power-user setups; do not enable on shared accounts.
    Auto,
}

/// How a skill workshop candidate is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    /// Cheap pattern match only — no LLM call, no token spend.
    #[default]
    Heuristic,
    /// Heuristic gate first; only candidates that pass the heuristic
    /// are forwarded to a cheap auxiliary LLM
    /// (`AuxTask::SkillWorkshopReview`) for confirmation and
    /// refinement. The LLM may reject a candidate that the heuristic
    /// accepted, but it cannot resurrect one the heuristic dropped —
    /// keeps the LLM call rate bounded by the heuristic's hit rate.
    /// `both` from openclaw's vocabulary is accepted as an alias and
    /// is functionally identical: a parallel "run both gates
    /// independently and union accepts" mode would require feeding
    /// the LLM raw turn text without a heuristic seed and is
    /// out-of-scope for the current pipeline.
    #[serde(
        alias = "threshold-llm",
        alias = "threshold_llm",
        alias = "both",
        alias = "Both"
    )]
    ThresholdLlm,
    /// Run the heuristic scanners but drop every hit before writing
    /// to disk. No new candidates are produced. Use `auto_capture =
    /// false` instead if you want to also skip the regex scan; this
    /// variant exists for the test path that wants the scanner code
    /// exercised without polluting the pending tree.
    None,
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

    // -----------------------------------------------------------------------
    // Session auto-reset state (mirrors hermes-agent SessionEntry flags)
    // -----------------------------------------------------------------------
    /// When `true`, the next call to `execute_llm_agent` will force a hard
    /// reset (cleared message history) before processing the message.
    /// The session_id is kept; only the stored messages are wiped.
    /// Set by operator action or stuck-loop recovery.  Takes priority over
    /// `resume_pending`.
    ///
    /// Named `force_session_wipe` (not `suspended`) to avoid confusion with
    /// `AgentState::Suspended` / `suspend_agent()`.
    #[serde(default)]
    pub force_session_wipe: bool,

    /// When `true`, the agent was interrupted by a restart/shutdown but
    /// recovery is expected.  Unlike `suspended`, the existing `session_id`
    /// is preserved and the agent continues on the same transcript.
    /// Cleared after the next successful turn.
    #[serde(default)]
    pub resume_pending: bool,

    /// The reason for the most recent automatic session reset, if any.
    /// `None` until the first auto-reset occurs.
    #[serde(default)]
    pub reset_reason: Option<crate::config::SessionResetReason>,

    /// Sticky flag: `true` once the agent has processed at least one real
    /// inbound message, channel event, or autonomous tick.
    ///
    /// Used by the heartbeat monitor (`crate::heartbeat::check_agents`) to
    /// distinguish agents that have genuinely been alive (and may now be
    /// hanging) from agents that were spawned but never received any work
    /// — the latter must not be flagged unresponsive, which would push them
    /// into a crash-recover loop (openfang #844).
    ///
    /// Replaces the older time-window heuristic
    /// (`last_active - created_at <= IDLE_GRACE_SECS`) which was fragile
    /// because administrative `set_state` / metadata writes also bump
    /// `last_active`. Bookkeeping bumps must NOT flip this flag — only
    /// real message-dispatch paths.
    #[serde(default)]
    pub has_processed_message: bool,
}

impl Default for AgentEntry {
    fn default() -> Self {
        let now = chrono::Utc::now();
        Self {
            id: AgentId::default(),
            name: String::new(),
            manifest: AgentManifest::default(),
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: now,
            last_active: now,
            parent: None,
            children: Vec::new(),
            session_id: SessionId::default(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            force_session_wipe: false,
            resume_pending: false,
            reset_reason: None,
            has_processed_message: false,
        }
    }
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
    fn test_user_id_from_name_is_stable() {
        // Same name → same id, across calls. This is the contract that lets
        // audit log entries survive daemon restarts.
        assert_eq!(UserId::from_name("Alice"), UserId::from_name("Alice"));
        assert_eq!(UserId::from_name(""), UserId::from_name(""));
    }

    #[test]
    fn test_user_id_from_name_differs_per_name() {
        assert_ne!(UserId::from_name("Alice"), UserId::from_name("Bob"));
        // Case-sensitive — caller controls normalization.
        assert_ne!(UserId::from_name("alice"), UserId::from_name("Alice"));
    }

    #[test]
    fn test_user_id_from_name_is_v5() {
        // UUID v5 (SHA-1 + namespace) — version nibble must be 5.
        let id = UserId::from_name("Alice");
        assert_eq!(id.0.get_version_num(), 5);
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
        assert_eq!(cfg.max_iterations, AutonomousConfig::DEFAULT_MAX_ITERATIONS);
        assert_eq!(cfg.max_restarts, 10);
        assert_eq!(cfg.heartbeat_interval_secs, 30);
        assert!(cfg.quiet_hours.is_none());
    }

    #[test]
    fn test_default_max_iterations_is_policy_constant() {
        // Tripwire: if anyone changes the policy cap, at least one test must
        // break so a second reviewer sees the intent.
        assert_eq!(AutonomousConfig::DEFAULT_MAX_ITERATIONS, 50);
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
            pinned_model: Some("sonnet".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert!(back.routing.is_some());
        assert!(back.autonomous.is_some());
        assert_eq!(back.pinned_model, Some("sonnet".to_string()));
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
    fn effective_burst_ratio_defaults_to_one_fifth() {
        let q = ResourceQuota::default();
        assert!((q.effective_burst_ratio(0.0) - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_burst_ratio_uses_global_default() {
        let q = ResourceQuota::default();
        assert!((q.effective_burst_ratio(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_burst_ratio_agent_overrides_global() {
        let q = ResourceQuota {
            burst_ratio: Some(0.3),
            ..Default::default()
        };
        assert!((q.effective_burst_ratio(0.8) - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_burst_ratio_clamps_below_minimum() {
        let q = ResourceQuota {
            burst_ratio: Some(0.0),
            ..Default::default()
        };
        assert!((q.effective_burst_ratio(0.0) - 0.01).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_burst_ratio_clamps_above_one() {
        let q = ResourceQuota {
            burst_ratio: Some(2.5),
            ..Default::default()
        };
        assert!((q.effective_burst_ratio(0.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_burst_ratio_nan_falls_back_to_default() {
        let q = ResourceQuota {
            burst_ratio: Some(f32::NAN),
            ..Default::default()
        };
        assert!((q.effective_burst_ratio(0.0) - 0.2).abs() < f32::EPSILON);
    }

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
        assert!(tools.contains(&"channel_send".to_string()));
        assert!(tools.contains(&"memory_recall".to_string()));
        assert_eq!(tools.len(), 6);
    }

    #[test]
    fn test_tool_profile_automation() {
        let tools = ToolProfile::Automation.tools();
        assert!(tools.contains(&"channel_send".to_string()));
        assert_eq!(tools.len(), 12);
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
            extra_params: std::collections::HashMap::new(),
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
                extra_params: std::collections::HashMap::new(),
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
            ..Default::default()
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
            ..Default::default()
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

    #[test]
    fn test_model_config_extra_params_roundtrip() {
        let mut extra = std::collections::HashMap::new();
        extra.insert("enable_memory".to_string(), serde_json::json!(true));
        extra.insert("memory_max_window".to_string(), serde_json::json!(50));

        let config = ModelConfig {
            provider: "qwen".to_string(),
            model: "qwen3.6".to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            system_prompt: "test".to_string(),
            api_key_env: None,
            base_url: None,
            context_window: None,
            max_output_tokens: None,
            extra_params: extra,
        };

        // Serialize to TOML
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("enable_memory = true"));
        assert!(toml_str.contains("memory_max_window = 50"));

        // Deserialize back
        let parsed: ModelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            parsed.extra_params.get("enable_memory").unwrap(),
            &serde_json::json!(true)
        );
        assert_eq!(
            parsed.extra_params.get("memory_max_window").unwrap(),
            &serde_json::json!(50)
        );
    }

    #[test]
    fn test_model_config_extra_params_empty_by_default() {
        let config = ModelConfig::default();
        assert!(config.extra_params.is_empty());
    }

    #[test]
    fn test_session_id_for_channel_deterministic() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let s1 = SessionId::for_channel(agent, "telegram");
        let s2 = SessionId::for_channel(agent, "telegram");
        assert_eq!(
            s1, s2,
            "Same (agent, channel) must produce identical SessionId"
        );
    }

    #[test]
    fn test_session_id_for_channel_differs_by_channel() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let telegram = SessionId::for_channel(agent, "telegram");
        let whatsapp = SessionId::for_channel(agent, "whatsapp");
        assert_ne!(
            telegram, whatsapp,
            "Different channels must produce different SessionIds"
        );
    }

    #[test]
    fn test_session_id_for_channel_differs_by_agent() {
        let agent_a =
            AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let agent_b =
            AgentId(uuid::Uuid::parse_str("f1f2f3f4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let sa = SessionId::for_channel(agent_a, "telegram");
        let sb = SessionId::for_channel(agent_b, "telegram");
        assert_ne!(
            sa, sb,
            "Different agents must produce different SessionIds for same channel"
        );
    }

    #[test]
    fn test_session_id_for_channel_cron_distinct() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let cron = SessionId::for_channel(agent, "cron");
        let telegram = SessionId::for_channel(agent, "telegram");
        let whatsapp = SessionId::for_channel(agent, "whatsapp");
        assert_ne!(cron, telegram, "Cron session must differ from telegram");
        assert_ne!(cron, whatsapp, "Cron session must differ from whatsapp");
    }

    #[test]
    fn test_session_id_for_channel_is_uuid_v5() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let sid = SessionId::for_channel(agent, "telegram");
        assert_eq!(sid.0.get_version_num(), 5, "SessionId must be UUID v5");
    }

    #[test]
    fn session_id_from_str_parses_uuid() {
        use std::str::FromStr;
        let s = "550e8400-e29b-41d4-a716-446655440000";
        let sid = SessionId::from_str(s).expect("valid uuid");
        assert_eq!(sid.0.to_string(), s);
    }

    #[test]
    fn session_id_from_str_rejects_garbage() {
        use std::str::FromStr;
        assert!(SessionId::from_str("not-a-uuid").is_err());
    }

    #[test]
    fn for_cron_run_deterministic() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let a = SessionId::for_cron_run(agent, "job-uuid:2026-04-25T10:00:00Z");
        let b = SessionId::for_cron_run(agent, "job-uuid:2026-04-25T10:00:00Z");
        assert_eq!(a, b);
    }

    #[test]
    fn for_cron_run_distinguishes_fires() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let fire_a = SessionId::for_cron_run(agent, "job-uuid:2026-04-25T10:00:00Z");
        let fire_b = SessionId::for_cron_run(agent, "job-uuid:2026-04-25T10:05:00Z");
        assert_ne!(
            fire_a, fire_b,
            "different fires must yield different sessions"
        );
    }

    #[test]
    fn for_cron_run_distinct_from_for_channel_cron() {
        // Same input string, different namespaces → different UUIDs. Prevents
        // a per-fire cron session from ever colliding with the persistent
        // (agent, channel="cron") session.
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let persistent = SessionId::for_channel(agent, "cron");
        let isolated = SessionId::for_cron_run(agent, "cron");
        assert_ne!(persistent, isolated);
    }

    #[test]
    fn from_route_key_empty_account_matches_for_channel() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        // Plain channel, no conversation: must equal for_channel(agent, channel).
        assert_eq!(
            SessionId::from_route_key(agent, "telegram", "", ""),
            SessionId::for_channel(agent, "telegram"),
        );
        // Channel + conversation, empty account: must equal for_channel(agent, "channel:conv")
        // — preserves legacy `format!("{channel}:{chat_id}")` scope.
        assert_eq!(
            SessionId::from_route_key(agent, "telegram", "", "12345"),
            SessionId::for_channel(agent, "telegram:12345"),
        );
    }

    #[test]
    fn from_route_key_separates_accounts() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let alice = SessionId::from_route_key(agent, "telegram", "alice", "12345");
        let bob = SessionId::from_route_key(agent, "telegram", "bob", "12345");
        assert_ne!(
            alice, bob,
            "Different accounts on same channel+conversation must yield different sessions"
        );
        // And neither should collide with the legacy account-less id.
        let legacy = SessionId::from_route_key(agent, "telegram", "", "12345");
        assert_ne!(alice, legacy);
        assert_ne!(bob, legacy);
    }

    #[test]
    fn from_route_key_normalizes_case() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let mixed = SessionId::from_route_key(agent, "Telegram", "Alice", "ABC");
        let lower = SessionId::from_route_key(agent, "telegram", "alice", "abc");
        assert_eq!(
            mixed, lower,
            "channel/account/conversation must be case-insensitive"
        );
    }

    #[test]
    fn from_route_key_deterministic() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let a = SessionId::from_route_key(agent, "matrix", "alice@example.org", "!room:server");
        let b = SessionId::from_route_key(agent, "matrix", "alice@example.org", "!room:server");
        assert_eq!(a, b);
    }

    // ── WorkspaceDecl: path / mount mutual-exclusion (#3230) ──────────────

    #[test]
    fn workspace_decl_path_only_deserializes() {
        let s = r#"path = "shared/library"
mode = "rw"
"#;
        let d: WorkspaceDecl = toml::from_str(s).unwrap();
        assert_eq!(
            d.path.as_deref(),
            Some(std::path::Path::new("shared/library"))
        );
        assert!(d.mount.is_none());
        assert_eq!(d.mode, WorkspaceMode::ReadWrite);
        assert!(!d.is_external_mount());
    }

    #[test]
    fn workspace_decl_mount_only_deserializes() {
        let s = r#"mount = "/Users/me/Obsidian"
mode = "r"
"#;
        let d: WorkspaceDecl = toml::from_str(s).unwrap();
        assert_eq!(
            d.mount.as_deref(),
            Some(std::path::Path::new("/Users/me/Obsidian"))
        );
        assert!(d.path.is_none());
        assert_eq!(d.mode, WorkspaceMode::ReadOnly);
        assert!(d.is_external_mount());
    }

    /// Both fields can deserialize together — the kernel rejects the
    /// combination at boot (see `resolve_workspace_decl`). Schema-level
    /// rejection would break agent.toml hot-reload from the dashboard
    /// (the user couldn't even see the validation error in context).
    #[test]
    fn workspace_decl_both_fields_deserialize_runtime_rejects() {
        let s = r#"path = "rel"
mount = "/abs"
"#;
        let d: WorkspaceDecl = toml::from_str(s).unwrap();
        assert!(d.path.is_some() && d.mount.is_some());
    }

    #[test]
    fn workspace_decl_neither_field_deserializes() {
        let s = "mode = \"r\"\n";
        let d: WorkspaceDecl = toml::from_str(s).unwrap();
        assert!(d.path.is_none() && d.mount.is_none());
    }

    // ── mcp_disabled field (#4808) ─────────────────────────────────────────

    #[test]
    fn mcp_disabled_default_is_false() {
        let manifest = AgentManifest::default();
        assert!(!manifest.mcp_disabled, "mcp_disabled must default to false");
    }

    #[test]
    fn mcp_disabled_toml_roundtrip() {
        let toml_str = r#"
name = "no-mcp-agent"
mcp_disabled = true

[model]
provider = "anthropic"
model = "claude-3-haiku-20240307"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(
            manifest.mcp_disabled,
            "mcp_disabled must survive TOML deserialization"
        );

        let re_serialized = toml::to_string(&manifest).unwrap();
        let re_parsed: AgentManifest = toml::from_str(&re_serialized).unwrap();
        assert!(
            re_parsed.mcp_disabled,
            "mcp_disabled must survive TOML roundtrip"
        );
    }

    #[test]
    fn mcp_disabled_json_roundtrip() {
        let manifest = AgentManifest {
            mcp_disabled: true,
            mcp_servers: vec!["foo".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert!(
            back.mcp_disabled,
            "mcp_disabled must survive JSON roundtrip"
        );
        assert_eq!(back.mcp_servers, vec!["foo".to_string()]);
    }

    #[test]
    fn mcp_disabled_absent_from_toml_is_false() {
        // When the field is omitted from TOML, serde(default) should give false.
        let toml_str = r#"
name = "normal-agent"

[model]
provider = "anthropic"
model = "claude-3-haiku-20240307"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(
            !manifest.mcp_disabled,
            "mcp_disabled must be false when absent from TOML"
        );
    }

    // ----- #4976: per-agent compaction overrides -----

    #[test]
    fn compaction_overrides_all_none_returns_global_unchanged() {
        let global = crate::config::CompactionTomlConfig::default();
        let overrides = CompactionOverrides::default();
        let merged = overrides.resolve(&global);
        assert_eq!(merged.threshold_messages, global.threshold_messages);
        assert_eq!(merged.keep_recent, global.keep_recent);
        assert_eq!(merged.max_summary_tokens, global.max_summary_tokens);
        assert_eq!(merged.token_threshold_ratio, global.token_threshold_ratio);
        assert_eq!(merged.max_chunk_chars, global.max_chunk_chars);
        assert_eq!(merged.max_retries, global.max_retries);
        assert!(overrides.is_empty());
    }

    #[test]
    fn compaction_overrides_partial_only_overrides_set_fields() {
        let global = crate::config::CompactionTomlConfig {
            threshold_messages: 30,
            keep_recent: 10,
            max_summary_tokens: 1024,
            token_threshold_ratio: 0.7,
            max_chunk_chars: 80_000,
            max_retries: 3,
        };
        let overrides = CompactionOverrides {
            keep_recent: Some(20),
            max_summary_tokens: Some(8192),
            ..Default::default()
        };
        let merged = overrides.resolve(&global);
        // Override wins:
        assert_eq!(merged.keep_recent, 20);
        assert_eq!(merged.max_summary_tokens, 8192);
        // Falls through to global:
        assert_eq!(merged.threshold_messages, 30);
        assert_eq!(merged.token_threshold_ratio, 0.7);
        assert_eq!(merged.max_chunk_chars, 80_000);
        assert_eq!(merged.max_retries, 3);
        assert!(!overrides.is_empty());
    }

    #[test]
    fn compaction_overrides_all_some_overrides_every_field() {
        let global = crate::config::CompactionTomlConfig::default();
        let overrides = CompactionOverrides {
            threshold_messages: Some(50),
            keep_recent: Some(20),
            max_summary_tokens: Some(8192),
            token_threshold_ratio: Some(0.5),
            max_chunk_chars: Some(120_000),
            max_retries: Some(5),
        };
        let merged = overrides.resolve(&global);
        assert_eq!(merged.threshold_messages, 50);
        assert_eq!(merged.keep_recent, 20);
        assert_eq!(merged.max_summary_tokens, 8192);
        assert!((merged.token_threshold_ratio - 0.5).abs() < f64::EPSILON);
        assert_eq!(merged.max_chunk_chars, 120_000);
        assert_eq!(merged.max_retries, 5);
    }

    #[test]
    fn compaction_overrides_toml_roundtrip_partial() {
        // Issue #4976 example: partial overrides parse cleanly.
        let toml_str = r#"
name = "orchestrator"

[model]
provider = "anthropic"
model = "claude-3-sonnet-20240229"

[compaction]
max_summary_tokens = 8192
keep_recent = 20
threshold_messages = 50
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        let overrides = manifest
            .compaction
            .expect("compaction section must be parsed");
        assert_eq!(overrides.max_summary_tokens, Some(8192));
        assert_eq!(overrides.keep_recent, Some(20));
        assert_eq!(overrides.threshold_messages, Some(50));
        // Unset fields stay None — they fall through to global at resolve time.
        assert_eq!(overrides.token_threshold_ratio, None);
        assert_eq!(overrides.max_chunk_chars, None);
        assert_eq!(overrides.max_retries, None);
    }

    #[test]
    fn compaction_overrides_absent_from_toml_is_none() {
        // No `[compaction]` section → field is None → resolve() inherits global.
        let toml_str = r#"
name = "chat-agent"

[model]
provider = "anthropic"
model = "claude-3-haiku-20240307"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(
            manifest.compaction.is_none(),
            "missing [compaction] must deserialize to None, not Default::default()"
        );
    }
}
