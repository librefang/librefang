//! All configuration struct and enum type definitions, including Default impls and associated helper functions.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use super::serde_helpers::{deserialize_string_or_int_vec, OneOrMany};
use super::DEFAULT_API_LISTEN;

pub use librefang_storage::{
    RemoteSurrealConfig, StorageBackendKind, StorageConfig,
    DEFAULT_DATABASE_NAME as STORAGE_DEFAULT_DATABASE_NAME,
    DEFAULT_NAMESPACE_NAME as STORAGE_DEFAULT_NAMESPACE_NAME,
};

/// DM (direct message) policy for a channel.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    /// Respond to all DMs.
    #[default]
    Respond,
    /// Only respond to DMs from allowed users.
    AllowedOnly,
    /// Ignore all DMs.
    Ignore,
}

/// Group message policy for a channel.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GroupPolicy {
    /// Respond to all group messages.
    All,
    /// Only respond when mentioned (@bot).
    #[default]
    MentionOnly,
    /// Only respond to slash commands.
    CommandsOnly,
    /// Ignore all group messages.
    Ignore,
}

/// Prefix style applied to outbound agent messages on a channel.
///
/// When enabled, the channel bridge wraps the responding agent's reply with
/// its name so end-users can tell which agent authored the message when
/// multiple agents share the same channel. Default is `Off` to preserve
/// existing behavior.
///
/// Platform-native identity (e.g. Slack per-message bot username override,
/// Discord embed author field) is intentionally out of scope here.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PrefixStyle {
    /// No prefix — byte-identical to pre-feature behavior.
    #[default]
    Off,
    /// Plain bracketed name: `[agent-name] text`.
    Bracket,
    /// Bold bracketed name via markdown: `**[agent-name]** text`.
    /// Renders bold on platforms that support markdown (Discord, Telegram
    /// markdown mode, Slack mrkdwn treats it as bold too).
    BoldBracket,
}

/// Output format hint for channel-specific message formatting.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Standard Markdown (default).
    #[default]
    Markdown,
    /// Telegram HTML subset.
    TelegramHtml,
    /// Slack mrkdwn format.
    SlackMrkdwn,
    /// Plain text (no formatting).
    PlainText,
}

/// Auto-routing strategy for a channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoRouteStrategy {
    /// Disable auto-routing entirely (default). Channel messages always go to
    /// the configured agent without keyword/semantic classification.
    #[default]
    Off,
    /// Only route if the cache already has an entry; never trigger LLM
    /// classification on the first message.
    ExplicitOnly,
    /// Use the cached route for up to `auto_route_ttl_minutes`; re-classify
    /// via LLM once the TTL expires.
    StickyTtl,
    /// Use a cheap metadata heuristic to decide whether the cached route is
    /// still valid; fall back to full LLM classification after
    /// `auto_route_divergence_count` consecutive mismatches.
    StickyHeuristic,
}

/// Per-channel behavior overrides.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ChannelOverrides {
    /// Model override (uses agent's default if None).
    #[serde(default)]
    pub model: Option<String>,
    /// System prompt override.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// DM policy.
    #[serde(default)]
    pub dm_policy: DmPolicy,
    /// Group message policy.
    #[serde(default)]
    pub group_policy: GroupPolicy,
    /// Regex patterns that can trigger a reply in group chats when
    /// `group_policy` is `mention_only`.
    #[serde(default)]
    pub group_trigger_patterns: Vec<String>,
    /// Enable LLM-based reply-intent precheck for group messages.
    /// When true and group_policy is "all", a lightweight classifier decides
    /// whether to reply before running the full agent loop.
    #[serde(default)]
    pub reply_precheck: bool,
    /// Model override for the reply precheck classifier (default: agent's model).
    #[serde(default)]
    pub reply_precheck_model: Option<String>,
    /// Global rate limit for this channel (messages per minute, 0 = unlimited).
    #[serde(default)]
    pub rate_limit_per_minute: u32,
    /// Per-user rate limit (messages per minute, 0 = unlimited).
    #[serde(default)]
    pub rate_limit_per_user: u32,
    /// Enable thread replies.
    #[serde(default)]
    pub threading: bool,
    /// Output format override.
    #[serde(default)]
    pub output_format: Option<OutputFormat>,
    /// Usage footer mode override.
    #[serde(default)]
    pub usage_footer: Option<UsageFooterMode>,
    /// Typing indicator mode override.
    #[serde(default)]
    pub typing_mode: Option<TypingMode>,
    /// Message debounce window in milliseconds. Default: 0 (disabled).
    #[serde(default)]
    pub message_debounce_ms: u64,
    /// Maximum time to buffer messages before forcing a dispatch. Default: 30000ms.
    #[serde(default = "default_message_debounce_max_ms")]
    pub message_debounce_max_ms: u64,
    /// Maximum number of messages to buffer per sender before forcing dispatch. Default: 64.
    #[serde(default = "default_message_debounce_max_buffer")]
    pub message_debounce_max_buffer: usize,
    /// Remove the reaction emoji on task completion instead of showing a
    /// "done" reaction.  When `true`, the bot clears all its reactions once
    /// the response is delivered, keeping the chat cleaner.  Default: `false`
    /// (show the done reaction for backward compatibility).
    #[serde(default)]
    pub clear_done_reaction: bool,
    /// When `true`, all built-in slash commands (`/agent`, `/new`, `/help`, …)
    /// are disabled on this channel and any leading-slash text is forwarded
    /// to the agent as normal message content. Use this for public-facing
    /// bots where end users must not be able to switch agents or reset
    /// sessions. Takes precedence over `allowed_commands` / `blocked_commands`.
    #[serde(default)]
    pub disable_commands: bool,
    /// Whitelist of built-in command names (without the leading `/`) that
    /// are allowed on this channel. When non-empty, any command outside this
    /// list is treated as normal text and forwarded to the agent. Leave
    /// empty to fall back to `blocked_commands`.
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Blacklist of built-in command names (without the leading `/`) that
    /// are blocked on this channel. Applied only when `allowed_commands` is
    /// empty. Blocked commands are treated as normal text and forwarded to
    /// the agent.
    #[serde(default)]
    pub blocked_commands: Vec<String>,
    /// Auto-routing strategy for this channel. Defaults to `off` (no routing).
    #[serde(default)]
    pub auto_route: AutoRouteStrategy,
    /// How long (in minutes) a cached route stays valid for `sticky_ttl` strategy.
    #[serde(default = "default_auto_route_ttl")]
    pub auto_route_ttl_minutes: u32,
    /// Minimum heuristic confidence score (0–10) before a route is cached for
    /// `sticky_heuristic` strategy.
    #[serde(default = "default_auto_route_confidence")]
    pub auto_route_confidence_threshold: u32,
    /// Extra score added to the cached route in `sticky_heuristic` to prefer
    /// stability over churn.
    #[serde(default = "default_auto_route_bonus")]
    pub auto_route_sticky_bonus: u32,
    /// How many consecutive heuristic mismatches trigger a full LLM
    /// re-classification in `sticky_heuristic` mode.
    #[serde(default = "default_auto_route_divergence")]
    pub auto_route_divergence_count: u32,
    /// Prefix outbound messages with the responding agent's name.
    ///
    /// Defaults to `PrefixStyle::Off` so enabling this feature is opt-in per
    /// channel and existing configs keep their current output byte-for-byte.
    #[serde(default)]
    pub prefix_agent_name: PrefixStyle,
    /// Whether thread-ownership claiming applies to this channel. When set
    /// to `false`, every routed agent dispatches even if another agent
    /// already replied in the same thread — useful for "broadcast" channels
    /// where multiple agents are intended to chime in together. Default
    /// `true`: a single agent owns each `(channel, thread)` for the
    /// configured TTL (and an explicit @-mention re-claims for the new
    /// agent). DMs always bypass the registry. See #3334.
    #[serde(default = "default_thread_ownership_enabled")]
    pub thread_ownership_enabled: bool,
}

fn default_thread_ownership_enabled() -> bool {
    true
}

impl Default for ChannelOverrides {
    fn default() -> Self {
        Self {
            model: None,
            system_prompt: None,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            group_trigger_patterns: Vec::new(),
            reply_precheck: false,
            reply_precheck_model: None,
            rate_limit_per_minute: 0,
            rate_limit_per_user: 0,
            threading: false,
            output_format: None,
            usage_footer: None,
            typing_mode: None,
            message_debounce_ms: 0,
            message_debounce_max_ms: 30000,
            message_debounce_max_buffer: 64,
            clear_done_reaction: false,
            disable_commands: false,
            allowed_commands: Vec::new(),
            blocked_commands: Vec::new(),
            auto_route: AutoRouteStrategy::Off,
            auto_route_ttl_minutes: default_auto_route_ttl(),
            auto_route_confidence_threshold: default_auto_route_confidence(),
            auto_route_sticky_bonus: default_auto_route_bonus(),
            auto_route_divergence_count: default_auto_route_divergence(),
            prefix_agent_name: PrefixStyle::Off,
            thread_ownership_enabled: true,
        }
    }
}

fn default_message_debounce_max_ms() -> u64 {
    30000
}

fn default_message_debounce_max_buffer() -> usize {
    64
}

fn default_auto_route_ttl() -> u32 {
    30
}

fn default_auto_route_confidence() -> u32 {
    6
}

fn default_auto_route_bonus() -> u32 {
    4
}

fn default_auto_route_divergence() -> u32 {
    2
}

/// Controls what usage info appears in response footers.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UsageFooterMode {
    /// Don't show usage info.
    Off,
    /// Show token counts only.
    Tokens,
    /// Show estimated cost only.
    Cost,
    /// Show tokens + cost (default).
    #[default]
    Full,
}

/// Kernel operating mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum KernelMode {
    /// Conservative mode — no auto-updates, pinned models, stability-first.
    Stable,
    /// Default balanced mode.
    #[default]
    Default,
    /// Developer mode — experimental features enabled.
    Dev,
}

/// CLI update channel (like Apple software update channels).
///
/// Controls which GitHub releases are considered for `librefang update`:
/// - **Stable**: only non-prerelease tags (default).
/// - **Beta**: stable + beta tags (excludes `-rc`).
/// - **Rc**: all tags including release candidates.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
    Rc,
}

impl std::fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stable => write!(f, "stable"),
            Self::Beta => write!(f, "beta"),
            Self::Rc => write!(f, "rc"),
        }
    }
}

impl std::str::FromStr for UpdateChannel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stable" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            "rc" => Ok(Self::Rc),
            _ => Err(format!(
                "unknown update channel: {s} (expected: stable, beta, rc)"
            )),
        }
    }
}

/// User configuration for RBAC multi-user support.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UserConfig {
    /// User display name.
    pub name: String,
    /// User role (owner, admin, user, viewer).
    #[serde(default = "default_role")]
    pub role: String,
    /// Channel bindings: maps channel platform IDs to this user.
    /// e.g., {"telegram": "123456", "discord": "987654"}
    #[serde(default)]
    pub channel_bindings: HashMap<String, String>,
    /// Optional API key hash for API authentication.
    #[serde(default)]
    pub api_key_hash: Option<String>,
    /// RBAC M5: per-user spend caps. `None` means "no per-user cap" — the
    /// user is still bounded by global / per-agent / per-provider budgets.
    /// See [`UserBudgetConfig`] for the supported windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<UserBudgetConfig>,
    /// Per-user tool allow/deny lists. Layered ON TOP of the per-agent
    /// `ToolPolicy` and any channel rules in `ApprovalPolicy`.
    /// `None` means "no per-user policy — defer to other layers".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<crate::user_policy::UserToolPolicy>,
    /// Bulk allow/deny by `ToolGroup` category (groups are declared in
    /// `KernelConfig.tool_policy.groups`). Lets admins say
    /// `denied_groups = ["dangerous"]` without listing each tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_categories: Option<crate::user_policy::UserToolCategories>,
    /// Memory namespace ACL — controls reads/writes to memory scopes
    /// (`proactive`, `kv:*`, etc.) and PII redaction. `None` means
    /// "use the role default ACL".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_access: Option<crate::user_policy::UserMemoryAccess>,
    /// Per-channel tool overrides for THIS user. Keyed by channel adapter
    /// name (e.g. `"telegram"`, `"discord"`). Layers on top of the global
    /// `ApprovalPolicy.channel_rules` — both must agree to allow.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_tool_rules: HashMap<String, crate::user_policy::ChannelToolPolicy>,
}

fn default_role() -> String {
    "user".to_string()
}

impl Default for UserConfig {
    fn default() -> Self {
        // Mirrors the per-field `#[serde(default)]` attributes above so a
        // hand-built `UserConfig::default()` matches what
        // `serde::from_str("name = \"x\"")` would produce. Tests use
        // `UserConfig { name: ..., role: ..., api_key_hash: ...,
        // ..Default::default() }` to avoid restating every optional
        // RBAC field at every fixture site.
        Self {
            name: String::new(),
            role: default_role(),
            channel_bindings: HashMap::new(),
            api_key_hash: None,
            budget: None,
            tool_policy: None,
            tool_categories: None,
            memory_access: None,
            channel_tool_rules: HashMap::new(),
        }
    }
}

/// RBAC M5: per-user spending budget.
///
/// Mirrors the global [`BudgetConfig`] window structure (hourly / daily /
/// monthly) so the same cost-attribution pipeline can enforce both. Set
/// any limit to `0.0` for "unlimited on that window". `alert_threshold`
/// is the fraction of any limit at which the metering layer should emit
/// a `BudgetExceeded` audit pre-warning (default 0.8, clamped to 0..=1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct UserBudgetConfig {
    /// Maximum cost in USD per hour for this user (0.0 = unlimited).
    pub max_hourly_usd: f64,
    /// Maximum cost in USD per day for this user (0.0 = unlimited).
    pub max_daily_usd: f64,
    /// Maximum cost in USD per month for this user (0.0 = unlimited).
    pub max_monthly_usd: f64,
    /// Alert threshold (0..=1). Metering surfaces a BudgetExceeded audit
    /// when *any* window reaches this fraction of its limit. Defaults to
    /// 0.8 — same default as the global budget — for consistency.
    pub alert_threshold: f64,
}

impl Default for UserBudgetConfig {
    fn default() -> Self {
        Self {
            max_hourly_usd: 0.0,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
        }
    }
}

/// Maps platform-native group/server roles (Telegram admin, Discord guild role,
/// Slack workspace owner, etc.) to LibreFang `UserRole` values.
///
/// Resolution order in `AuthManager::resolve_role_for_sender` is:
/// 1. Explicit `UserConfig.role` for a registered user — wins outright.
/// 2. Channel-derived role from this mapping — applied when the user is
///    recognised on a platform but has no explicit `UserConfig` role.
/// 3. Default-deny — fall through to `guest`.
///
/// All sub-tables are optional — a missing channel just means "no
/// channel-derived role" for that platform. Each per-channel struct keeps
/// platform-shaped fields rather than a single uniform schema because the
/// underlying APIs disagree about role granularity (Telegram has 3 fixed
/// statuses, Discord has named guild roles, Slack collapses to
/// owner/admin/member/guest).
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ChannelRoleMapping {
    /// Telegram chat-status → LibreFang role mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram: Option<TelegramRoleMapping>,
    /// Discord guild-role → LibreFang role mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord: Option<DiscordRoleMapping>,
    /// Slack workspace-role → LibreFang role mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slack: Option<SlackRoleMapping>,
}

impl ChannelRoleMapping {
    /// Returns true when no platform mapping is configured.
    pub fn is_empty(&self) -> bool {
        self.telegram.is_none() && self.discord.is_none() && self.slack.is_none()
    }
}

/// Telegram-side mapping. Telegram exposes three statuses for a member of a
/// chat: `creator`, `administrator`, `member` (plus `restricted`/`left`/
/// `kicked` which we collapse into "no derived role").
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TelegramRoleMapping {
    /// LibreFang role assigned when Telegram reports `status = "administrator"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_role: Option<String>,
    /// LibreFang role assigned when Telegram reports `status = "creator"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_role: Option<String>,
    /// LibreFang role assigned when Telegram reports `status = "member"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_role: Option<String>,
}

/// Discord-side mapping. A user may hold any number of guild roles
/// simultaneously; the resolver walks **every** role the user has,
/// looks each one up in `role_map`, and picks the **highest-privilege**
/// match (`Owner` > `Admin` > `User` > `Viewer`). Declaration order
/// in `config.toml` is irrelevant — the privilege ordering on the
/// LibreFang side decides the winner. This protects against Discord-
/// side role ordering (which is outside our control) deciding the
/// effective LibreFang permissions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DiscordRoleMapping {
    /// Discord role name → LibreFang role string (`owner` / `admin` /
    /// `user` / `viewer` / `guest`). Iteration order is irrelevant —
    /// the translator scans every match the user holds and returns the
    /// most privileged. Typo'd LibreFang role strings (e.g. `"admn"`)
    /// are silently skipped, falling back to default-deny `Viewer`.
    pub role_map: HashMap<String, String>,
}

/// Slack-side mapping. Slack's `users.info` exposes `is_owner` /
/// `is_admin` / `is_restricted` / `is_ultra_restricted`. Precedence
/// (owner > admin > guest > member) is collapsed inside the channel
/// adapter (`SlackAdapter::parse_users_info_response`) into a single
/// platform token before this mapping ever sees it; the translator
/// here is a flat lookup, not a precedence ladder. Each step is
/// optional — leave a field unset to fall through to default-deny
/// `Viewer` for that platform tier.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SlackRoleMapping {
    /// LibreFang role for `is_owner = true` users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_role: Option<String>,
    /// LibreFang role for `is_admin = true` users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_role: Option<String>,
    /// LibreFang role for regular workspace members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_role: Option<String>,
    /// LibreFang role for single/multi-channel guests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_role: Option<String>,
}

/// Web search provider selection.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    /// Brave Search API.
    Brave,
    /// Tavily AI-agent-native search.
    Tavily,
    /// Perplexity AI search.
    Perplexity,
    /// Jina AI search.
    Jina,
    /// DuckDuckGo HTML (no API key needed).
    DuckDuckGo,
    /// SearXNG self-hosted search (no API key needed).
    Searxng,
    /// Auto-select based on available API keys
    /// (Tavily → Brave → Jina → Perplexity → Searxng → DuckDuckGo).
    #[default]
    Auto,
}

/// Web tools configuration (search + fetch).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WebConfig {
    /// Which search provider to use.
    pub search_provider: SearchProvider,
    /// Cache TTL in minutes (0 = disabled).
    pub cache_ttl_minutes: u64,
    /// HTTP timeout for all web search requests (seconds).
    /// Recommended: 15 for most providers, 30+ for Jina.
    #[serde(default = "default_search_timeout_secs")]
    pub timeout_secs: u64,
    /// Brave Search configuration.
    pub brave: BraveSearchConfig,
    /// Tavily Search configuration.
    pub tavily: TavilySearchConfig,
    /// Perplexity Search configuration.
    pub perplexity: PerplexitySearchConfig,
    /// Jina Search configuration.
    pub jina: JinaSearchConfig,
    /// SearXNG self-hosted search configuration.
    pub searxng: SearxngSearchConfig,
    /// Web fetch configuration.
    pub fetch: WebFetchConfig,
}

fn default_search_timeout_secs() -> u64 {
    15
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: SearchProvider::default(),
            cache_ttl_minutes: 15,
            timeout_secs: default_search_timeout_secs(),
            brave: BraveSearchConfig::default(),
            tavily: TavilySearchConfig::default(),
            perplexity: PerplexitySearchConfig::default(),
            jina: JinaSearchConfig::default(),
            searxng: SearxngSearchConfig::default(),
            fetch: WebFetchConfig::default(),
        }
    }
}

/// Brave Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BraveSearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Maximum results to return.
    pub max_results: usize,
    /// Country code for search localization (e.g., "US").
    pub country: String,
    /// Search language (e.g., "en").
    pub search_lang: String,
    /// Freshness filter (e.g., "pd" = past day, "pw" = past week).
    pub freshness: String,
}

impl Default for BraveSearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "BRAVE_API_KEY".to_string(),
            max_results: 5,
            country: String::new(),
            search_lang: String::new(),
            freshness: String::new(),
        }
    }
}

/// Tavily Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TavilySearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Search depth: "basic" or "advanced".
    pub search_depth: String,
    /// Maximum results to return.
    pub max_results: usize,
    /// Include AI-generated answer summary.
    pub include_answer: bool,
}

impl Default for TavilySearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "TAVILY_API_KEY".to_string(),
            search_depth: "basic".to_string(),
            max_results: 5,
            include_answer: true,
        }
    }
}

/// Perplexity Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PerplexitySearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Model to use for search (e.g., "sonar").
    pub model: String,
}

impl Default for PerplexitySearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "PERPLEXITY_API_KEY".to_string(),
            model: "sonar".to_string(),
        }
    }
}

/// Jina Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct JinaSearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Maximum results to return.
    pub max_results: usize,
    /// Country/region code for geolocation (e.g., "US").
    pub country: String,
    /// Language code (e.g., "en").
    pub language: String,
    /// Use EU endpoint (https://eu.s.jina.ai/) instead of global.
    pub use_eu_endpoint: bool,
    /// Disable Jina server-side cache.
    pub no_cache: bool,
}

impl Default for JinaSearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "JINA_API_KEY".to_string(),
            max_results: 5,
            country: String::new(),
            language: String::new(),
            use_eu_endpoint: false,
            no_cache: false,
        }
    }
}

/// SearXNG self-hosted search configuration.
///
/// Requires only a `url`; SearXNG public instances reject `limit` and the
/// LLM-facing `max_results` is taken from the per-call `tool_args` (the
/// runtime truncates client-side after fetching).
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SearxngSearchConfig {
    /// Base URL of the SearXNG instance (e.g., "https://search.example.com").
    /// Empty means the provider is disabled.
    pub url: String,
}

/// Web fetch configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WebFetchConfig {
    /// Maximum characters to return in content.
    pub max_chars: usize,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// HTTP request timeout in seconds.
    pub timeout_secs: u64,
    /// Enable HTML→Markdown readability extraction.
    pub readability: bool,
    /// Hosts/CIDRs that are exempt from SSRF blocking (e.g. internal services in K8s).
    /// Cloud metadata endpoints (169.254.x.x, etc.) remain blocked unconditionally.
    #[serde(default)]
    pub ssrf_allowed_hosts: Vec<String>,
    /// Maximum bytes a single `web_fetch_to_file` download may write to disk.
    /// Caps response size before the body reaches the workspace; an agent-supplied
    /// `max_bytes` parameter is further clamped down to this value, never up.
    #[serde(default = "default_web_fetch_to_file_max_bytes")]
    pub max_file_bytes: u64,
}

fn default_web_fetch_to_file_max_bytes() -> u64 {
    50 * 1024 * 1024
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_chars: 50_000,
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            timeout_secs: 30,
            readability: true,
            ssrf_allowed_hosts: vec![],
            max_file_bytes: default_web_fetch_to_file_max_bytes(),
        }
    }
}

/// Browser automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BrowserConfig {
    /// Enable the built-in CDP browser tools (browser_navigate, browser_click,
    /// etc.).  Set to `false` when using an external browser MCP server such as
    /// CamoFox, which replaces these tools with its own set.
    pub enabled: bool,
    /// Run browser in headless mode (no visible window).
    pub headless: bool,
    /// Viewport width in pixels.
    pub viewport_width: u32,
    /// Viewport height in pixels.
    pub viewport_height: u32,
    /// Per-action timeout in seconds.
    pub timeout_secs: u64,
    /// Idle timeout — auto-close session after this many seconds of inactivity.
    pub idle_timeout_secs: u64,
    /// Maximum concurrent browser sessions.
    pub max_sessions: usize,
    /// Path to Chromium/Chrome binary. Auto-detected if None.
    pub chromium_path: Option<String>,
    /// Remote CDP endpoint to attach to instead of spawning a local Chromium.
    ///
    /// Accepted formats:
    /// - `ws://host:port/devtools/browser/<id>` — page-level WebSocket (direct attach)
    /// - `http://host:port` — HTTP discovery endpoint; librefang calls `GET /json/new`
    ///   to create a fresh tab and connects to the returned WebSocket URL.
    ///
    /// When set, `headless`, `chromium_path`, and local-process discovery are
    /// ignored. Browser lifecycle (start/stop) is the operator's responsibility.
    ///
    /// **Security**: CDP is unauthenticated. Never expose the debugging port on a
    /// public interface. Use SSH tunnels, WireGuard, or a trusted-network path.
    #[serde(default)]
    pub cdp_endpoint: Option<String>,
    /// Environment variable that holds a bearer token for the CDP endpoint.
    ///
    /// Some CDP proxies (e.g. Browserless) require `Authorization: Bearer <token>`
    /// on the WebSocket upgrade request. Set this to the name of an env var that
    /// contains the token (e.g. `"LIBREFANG_CDP_TOKEN"`); librefang reads the
    /// value at connect time and never logs it.
    #[serde(default)]
    pub cdp_auth_token_env: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            headless: true,
            viewport_width: 1280,
            viewport_height: 720,
            timeout_secs: 30,
            idle_timeout_secs: 300,
            max_sessions: 5,
            chromium_path: None,
            cdp_endpoint: None,
            cdp_auth_token_env: None,
        }
    }
}

/// Config hot-reload mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReloadMode {
    /// No automatic reloading.
    Off,
    /// Full restart on config change.
    Restart,
    /// Hot-reload safe sections only (channels, skills, heartbeat).
    Hot,
    /// Hot-reload where possible, flag restart-required otherwise.
    #[default]
    Hybrid,
}

/// Configuration for config file watching and hot-reload.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ReloadConfig {
    /// Reload mode. Default: hybrid.
    pub mode: ReloadMode,
    /// Debounce window in milliseconds. Default: 500.
    pub debounce_ms: u64,
}

impl Default for ReloadConfig {
    fn default() -> Self {
        Self {
            mode: ReloadMode::default(),
            debounce_ms: 500,
        }
    }
}

/// API and WebSocket rate limiting configuration.
///
/// Controls GCRA token-bucket rate limiting for HTTP API requests and
/// per-connection limits for WebSocket connections.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RateLimitConfig {
    /// API token budget per minute per IP (GCRA algorithm). Default: 500.
    #[serde(default = "default_api_requests_per_minute")]
    pub api_requests_per_minute: u32,
    /// Retry-After header value in seconds when rate limited. Default: 60.
    #[serde(default = "default_retry_after_secs")]
    pub retry_after_secs: u64,
    /// Maximum concurrent WebSocket connections per IP. Default: 5.
    #[serde(default = "default_max_ws_per_ip")]
    pub max_ws_per_ip: usize,
    /// Maximum WebSocket messages per minute per connection. Default: 10.
    #[serde(default = "default_ws_messages_per_minute")]
    pub ws_messages_per_minute: u32,
    /// Maximum terminal WebSocket input messages per minute per connection.
    /// Default: 3600.
    ///
    /// Terminal sessions send one WebSocket message per keystroke, so the
    /// generic `ws_messages_per_minute = 10` (sized for chat WS where a
    /// "message" is a whole utterance) is two orders of magnitude too low
    /// for an interactive PTY — typing `vim` + `:wq` in vim already
    /// exhausts the budget and the session appears to freeze. 3600/min
    /// (60/sec ≈ 720 WPM) covers any human typing speed plus TUI
    /// navigation bursts while still capping pathological floods.
    #[serde(default = "default_ws_terminal_messages_per_minute")]
    pub ws_terminal_messages_per_minute: u32,
    /// WebSocket idle timeout in seconds (close after inactivity). Default: 1800.
    #[serde(default = "default_ws_idle_timeout_secs")]
    pub ws_idle_timeout_secs: u64,
    /// Text delta debounce interval in milliseconds. Default: 100.
    #[serde(default = "default_ws_debounce_ms")]
    pub ws_debounce_ms: u64,
    /// Flush text buffer when it exceeds this many characters. Default: 200.
    #[serde(default = "default_ws_debounce_chars")]
    pub ws_debounce_chars: usize,
    /// Max login attempts per IP per 15-minute window on auth endpoints
    /// (`/api/auth/dashboard-login`, `/api/auth/login*`). Default: 10.
    /// Set to 0 to disable the per-IP auth rate limiter.
    #[serde(default = "default_auth_rate_limit_per_ip")]
    pub auth_rate_limit_per_ip: u32,
}

fn default_api_requests_per_minute() -> u32 {
    500
}
fn default_retry_after_secs() -> u64 {
    60
}
fn default_max_ws_per_ip() -> usize {
    5
}
fn default_ws_messages_per_minute() -> u32 {
    10
}
fn default_ws_terminal_messages_per_minute() -> u32 {
    3600
}
fn default_ws_idle_timeout_secs() -> u64 {
    1800
}
fn default_ws_debounce_ms() -> u64 {
    100
}
fn default_ws_debounce_chars() -> usize {
    200
}
fn default_auth_rate_limit_per_ip() -> u32 {
    10
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            api_requests_per_minute: default_api_requests_per_minute(),
            retry_after_secs: default_retry_after_secs(),
            max_ws_per_ip: default_max_ws_per_ip(),
            ws_messages_per_minute: default_ws_messages_per_minute(),
            ws_terminal_messages_per_minute: default_ws_terminal_messages_per_minute(),
            ws_idle_timeout_secs: default_ws_idle_timeout_secs(),
            ws_debounce_ms: default_ws_debounce_ms(),
            ws_debounce_chars: default_ws_debounce_chars(),
            auth_rate_limit_per_ip: default_auth_rate_limit_per_ip(),
        }
    }
}

/// Webhook trigger authentication configuration.
///
/// Controls the `/hooks/wake` and `/hooks/agent` endpoints for external
/// systems to trigger agent actions.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WebhookTriggerConfig {
    /// Enable webhook trigger endpoints. Default: false.
    pub enabled: bool,
    /// Env var name holding the bearer token (NOT the token itself).
    /// MUST be set if enabled=true. Token must be >= 32 chars.
    pub token_env: String,
    /// Max payload size in bytes. Default: 65536.
    pub max_payload_bytes: usize,
    /// Rate limit: max requests per minute per IP. Default: 30.
    pub rate_limit_per_minute: u32,
}

impl Default for WebhookTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token_env: "LIBREFANG_WEBHOOK_TOKEN".to_string(),
            max_payload_bytes: 65536,
            rate_limit_per_minute: 30,
        }
    }
}

/// Fallback provider chain — tried in order if the primary provider fails.
///
/// Configurable in `config.toml` under `[[fallback_providers]]`:
/// ```toml
/// [[fallback_providers]]
/// provider = "ollama"
/// model = "llama3.2:latest"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FallbackProviderConfig {
    /// Provider name (e.g., "ollama", "groq").
    pub provider: String,
    /// Model to use from this provider.
    pub model: String,
    /// Environment variable for API key (empty for local providers).
    #[serde(default)]
    pub api_key_env: String,
    /// Base URL override (uses catalog default if None).
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Side-task category for the auxiliary LLM client.
///
/// Each variant maps to a separate fallback chain in `[llm.auxiliary]` so
/// users can pick a cheap model per task without polluting the primary
/// agent's provider list. See `librefang_runtime::aux_client` for the
/// resolution algorithm and the published default chains.
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AuxTask {
    /// LLM-driven context compression (history summarisation).
    Compression,
    /// Session / trajectory title generation.
    Title,
    /// Search-result summarisation.
    Search,
    /// Image / video / vision-capable description.
    Vision,
    /// Browser-tool vision-driven page understanding.
    BrowserVision,
    /// Tool-result history fold (#3347 3/N): summarise stale tool results
    /// from turns older than `history_fold_after_turns` into a compact stub.
    Fold,
    /// Skill workshop (#3328) candidate review: classify whether a
    /// captured workflow is worth promoting to a draft skill, and refine
    /// its name / one-line summary if so. Cheap classification call,
    /// runs at most once per turn that produced a heuristic match.
    SkillReview,
    /// Skill workshop (#3328) — separate aux slot from `SkillReview` so
    /// the workshop's after-turn capture review can be costed and
    /// configured independently of the existing `background_skill_review`
    /// pipeline (which also resolves through `SkillReview`). Operators
    /// can disable one without disabling the other; budget tooling sees
    /// distinct line items.
    SkillWorkshopReview,
    /// Session-end summary (#4869): on `reset_session` / `/new`, the
    /// kernel asks the auxiliary LLM to produce a real summary of the
    /// session that's about to be deleted (`kv_store` keyed by the
    /// session id, plus a markdown file in the agent's workspace). The
    /// pre-#4869 implementation built the summary from the last 10
    /// `Text`-only user messages, which collapsed to "thanks / sure"
    /// pleasantries on any non-trivial conversation. Routing through
    /// `[llm.auxiliary]` keeps the cost on the cheap tier; when no aux
    /// chain resolves, the kernel falls back to the historical trivial
    /// summary and logs a WARN so operators see the degraded path.
    SessionSummary,
}

impl AuxTask {
    /// Stable string slug used in TOML and logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuxTask::Compression => "compression",
            AuxTask::Title => "title",
            AuxTask::Search => "search",
            AuxTask::Vision => "vision",
            AuxTask::BrowserVision => "browser_vision",
            AuxTask::Fold => "fold",
            AuxTask::SkillReview => "skill_review",
            AuxTask::SkillWorkshopReview => "skill_workshop_review",
            AuxTask::SessionSummary => "session_summary",
        }
    }
}

impl std::fmt::Display for AuxTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Auxiliary LLM client configuration — one cheap-tier fallback chain per
/// side task.
///
/// Each entry is a list of `provider:model` references resolved in order
/// against the user's already-configured credentials. A task with no entries
/// (or whose entries cannot be initialised because the relevant API keys are
/// missing) silently falls back to the primary driver — the auxiliary client
/// is purely a routing optimisation, never a permission gate.
///
/// ```toml
/// [llm.auxiliary]
/// compression    = ["openrouter:anthropic/claude-3-5-haiku", "anthropic:haiku"]
/// title          = ["openrouter:meta-llama/llama-3.1-8b-instruct", "groq:llama-3.1-8b-instant"]
/// search         = ["openrouter:anthropic/claude-3-5-haiku", "openai:gpt-4o-mini"]
/// vision         = ["anthropic:sonnet", "openai:gpt-4o-mini"]
/// browser_vision = ["anthropic:sonnet", "openai:gpt-4o-mini"]
/// ```
///
/// Uses `BTreeMap` rather than `HashMap` so serialised output is
/// deterministic (see issue #3298).
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct AuxiliaryConfig {
    /// Per-task ordered chain of `provider:model` references.
    pub tasks: BTreeMap<AuxTask, Vec<String>>,
}

impl AuxiliaryConfig {
    /// Build an empty config (every task falls back to the primary driver).
    pub fn empty() -> Self {
        Self {
            tasks: BTreeMap::new(),
        }
    }

    /// Lookup the configured chain for `task`, if any.
    pub fn chain_for(&self, task: AuxTask) -> Option<&[String]> {
        self.tasks.get(&task).map(|v| v.as_slice())
    }

    /// Whether this config has any user-supplied entries.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// Top-level `[llm]` section. Currently only carries `auxiliary` — primary
/// driver configuration still lives in `[default_model]` / `[[fallback_providers]]`
/// so this struct exists purely to namespace future LLM-routing knobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LlmConfig {
    /// Per-task auxiliary fallback chains. See [`AuxiliaryConfig`].
    pub auxiliary: AuxiliaryConfig,
}

/// Text-to-speech configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TtsConfig {
    /// Enable TTS. Default: false.
    pub enabled: bool,
    /// Default provider: "openai", "elevenlabs", or "google_tts".
    pub provider: Option<String>,
    /// OpenAI TTS settings.
    pub openai: TtsOpenAiConfig,
    /// ElevenLabs TTS settings.
    pub elevenlabs: TtsElevenLabsConfig,
    /// Google Cloud TTS settings.
    pub google: TtsGoogleConfig,
    /// Max text length for TTS (chars). Default: 4096.
    pub max_text_length: usize,
    /// Timeout per TTS request in seconds. Default: 30.
    pub timeout_secs: u64,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            openai: TtsOpenAiConfig::default(),
            elevenlabs: TtsElevenLabsConfig::default(),
            google: TtsGoogleConfig::default(),
            max_text_length: 4096,
            timeout_secs: 30,
        }
    }
}

/// OpenAI TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TtsOpenAiConfig {
    /// Voice: alloy, echo, fable, onyx, nova, shimmer. Default: "alloy".
    pub voice: String,
    /// Model: "tts-1" or "tts-1-hd". Default: "tts-1".
    pub model: String,
    /// Output format: "mp3", "opus", "aac", "flac". Default: "mp3".
    pub format: String,
    /// Speed: 0.25 to 4.0. Default: 1.0.
    pub speed: f32,
}

impl Default for TtsOpenAiConfig {
    fn default() -> Self {
        Self {
            voice: "alloy".to_string(),
            model: "tts-1".to_string(),
            format: "mp3".to_string(),
            speed: 1.0,
        }
    }
}

/// ElevenLabs TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TtsElevenLabsConfig {
    /// Voice ID. Default: "21m00Tcm4TlvDq8ikWAM" (Rachel).
    pub voice_id: String,
    /// Model ID. Default: "eleven_monolingual_v1".
    pub model_id: String,
    /// Stability (0.0-1.0). Default: 0.5.
    pub stability: f32,
    /// Similarity boost (0.0-1.0). Default: 0.75.
    pub similarity_boost: f32,
}

impl Default for TtsElevenLabsConfig {
    fn default() -> Self {
        Self {
            voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
            model_id: "eleven_monolingual_v1".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
        }
    }
}

/// Google Cloud TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TtsGoogleConfig {
    /// Voice name (e.g. "en-US-Standard-F", "pl-PL-Wavenet-A"). Default: "en-US-Standard-F".
    pub voice: String,
    /// Language code (e.g. "en-US", "pl-PL"). Default: "en-US".
    pub language_code: String,
    /// Speaking rate: 0.25 to 4.0. Default: 1.0.
    pub speaking_rate: f32,
    /// Pitch adjustment: -20.0 to 20.0. Default: 0.0.
    pub pitch: f32,
    /// Output format: "mp3", "opus", "wav". Default: "mp3".
    pub format: String,
}

impl Default for TtsGoogleConfig {
    fn default() -> Self {
        Self {
            voice: "en-US-Standard-F".to_string(),
            language_code: "en-US".to_string(),
            speaking_rate: 1.0,
            pitch: 0.0,
            format: "mp3".to_string(),
        }
    }
}

/// Docker container sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DockerSandboxConfig {
    /// Enable Docker sandbox. Default: false.
    pub enabled: bool,
    /// Docker image for exec sandbox. Default: "python:3.12-slim".
    pub image: String,
    /// Container name prefix. Default: "librefang-sandbox".
    pub container_prefix: String,
    /// Working directory inside container. Default: "/workspace".
    pub workdir: String,
    /// Network mode: "none", "bridge", or custom. Default: "none".
    pub network: String,
    /// Memory limit (e.g., "256m", "1g"). Default: "512m".
    pub memory_limit: String,
    /// CPU limit (e.g., 0.5, 1.0, 2.0). Default: 1.0.
    pub cpu_limit: f64,
    /// Max execution time in seconds. Default: 60.
    pub timeout_secs: u64,
    /// Read-only root filesystem. Default: true.
    pub read_only_root: bool,
    /// Additional capabilities to add. Default: empty (drop all).
    pub cap_add: Vec<String>,
    /// tmpfs mounts. Default: ["/tmp:size=64m"].
    pub tmpfs: Vec<String>,
    /// PID limit. Default: 100.
    pub pids_limit: u32,
    /// Docker sandbox mode: off, non_main, all. Default: off.
    #[serde(default)]
    pub mode: DockerSandboxMode,
    /// Container lifecycle scope. Default: session.
    #[serde(default)]
    pub scope: DockerScope,
    /// Cooldown before reusing a released container (seconds). Default: 300.
    #[serde(default = "default_reuse_cool_secs")]
    pub reuse_cool_secs: u64,
    /// Idle timeout — destroy containers after N seconds of inactivity. Default: 86400 (24h).
    #[serde(default = "default_docker_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Maximum age before forced destruction (seconds). Default: 604800 (7 days).
    #[serde(default = "default_docker_max_age")]
    pub max_age_secs: u64,
    /// Paths blocked from bind mounting.
    #[serde(default)]
    pub blocked_mounts: Vec<String>,
}

fn default_reuse_cool_secs() -> u64 {
    300
}
fn default_docker_idle_timeout() -> u64 {
    86400
}
fn default_docker_max_age() -> u64 {
    604800
}

impl Default for DockerSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            image: "python:3.12-slim".to_string(),
            container_prefix: "librefang-sandbox".to_string(),
            workdir: "/workspace".to_string(),
            network: "none".to_string(),
            memory_limit: "512m".to_string(),
            cpu_limit: 1.0,
            timeout_secs: 60,
            read_only_root: true,
            cap_add: Vec::new(),
            tmpfs: vec!["/tmp:size=64m".to_string()],
            pids_limit: 100,
            mode: DockerSandboxMode::Off,
            scope: DockerScope::Session,
            reuse_cool_secs: default_reuse_cool_secs(),
            idle_timeout_secs: default_docker_idle_timeout(),
            max_age_secs: default_docker_max_age(),
            blocked_mounts: Vec::new(),
        }
    }
}

/// Device pairing configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PairingConfig {
    /// Enable device pairing. Default: false.
    pub enabled: bool,
    /// Max paired devices. Default: 10.
    pub max_devices: usize,
    /// Pairing token expiry in seconds. Default: 300 (5 min).
    pub token_expiry_secs: u64,
    /// Public base URL the QR code points mobile clients at, e.g.
    /// `https://librefang.example.com`. When set, takes precedence over
    /// the request `Host` header — required for HTTPS reverse-proxy
    /// deployments where trusting client-supplied `X-Forwarded-Proto`
    /// would let any authenticated dashboard caller forge the scheme.
    /// When `None`, the daemon falls back to `Host` + the runtime scheme.
    pub public_base_url: Option<String>,
    /// Push notification provider: "none", "ntfy", "gotify".
    pub push_provider: String,
    /// Ntfy server URL (if push_provider = "ntfy").
    pub ntfy_url: Option<String>,
    /// Ntfy topic (if push_provider = "ntfy").
    pub ntfy_topic: Option<String>,
}

impl Default for PairingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_devices: 10,
            token_expiry_secs: 300,
            public_base_url: None,
            push_provider: "none".to_string(),
            ntfy_url: None,
            ntfy_topic: None,
        }
    }
}

/// Skills configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SkillsConfig {
    /// Whether user-installed skills from the skills directory are loaded. Default: true.
    pub load_user: bool,
    /// Extra skill directories to scan in addition to `~/.librefang/skills/`.
    /// Each entry must be an absolute path. Scanned read-only after the
    /// primary skills dir; local skills with the same name win.
    #[serde(default)]
    pub extra_dirs: Vec<std::path::PathBuf>,
    /// Names of skills to skip at load time. Useful for quickly disabling
    /// a skill (agent-evolved or marketplace-installed) without deleting
    /// its directory. Matching is case-sensitive on the skill manifest name.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Operator-side gate over skill `env_passthrough` requests: glob
    /// patterns that block matching env-var names regardless of what the
    /// skill manifest declares. Defaults to a deny list covering common
    /// credential conventions (`*_KEY`, `*_TOKEN`, `*_PASSWORD`, `*_SECRET`,
    /// `*_API_KEY`, `AWS_*`, `GITHUB_*`). Set to an empty list to disable
    /// the operator deny check; the built-in `FORBIDDEN_PASSTHROUGH` and
    /// kernel-reserved hard blocks still apply.
    #[serde(default = "default_env_passthrough_denied_patterns")]
    pub env_passthrough_denied_patterns: Vec<String>,
    /// Per-skill explicit allow overrides. Lets the operator grant a
    /// specific skill an env var that would otherwise be blocked by
    /// `env_passthrough_denied_patterns`. Cannot bypass the built-in
    /// `FORBIDDEN_PASSTHROUGH` hard block.
    ///
    /// Example: `{ "gog" = ["GOG_KEYRING_PASSWORD"] }`.
    #[serde(default)]
    pub env_passthrough_per_skill: std::collections::HashMap<String, Vec<String>>,
}

/// Operator-side gate over skill `env_passthrough` requests.
///
/// The skill manifest declares which host env vars the skill *wants*; this
/// policy is the operator's final say on which of those requests get
/// granted. Constructed from `[skills]` config at the call site in the
/// runtime; the resolution algorithm lives in `librefang-skills::loader`.
///
/// Resolution order (applied per-skill):
///
/// 1. Hard block: names in the built-in `FORBIDDEN_PASSTHROUGH` list
///    (`LD_PRELOAD`, `PYTHONPATH`, …) — never overridable.
/// 2. Hard block: names the kernel sets explicitly per-runtime
///    (`PATH`, `HOME`, `PYTHONIOENCODING`, …).
/// 3. Operator deny: names matching `denied_patterns` are dropped *unless*
///    listed in `per_skill_overrides[skill_name]`.
/// 4. Anything else is forwarded to the subprocess.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvPassthroughPolicy {
    /// Glob patterns that block matching env-var names regardless of skill
    /// manifest. Operators can override per-skill via `per_skill_overrides`.
    pub denied_patterns: Vec<String>,
    /// Per-skill explicit allow overrides. Keyed by skill manifest name.
    /// Cannot bypass the built-in `FORBIDDEN_PASSTHROUGH` hard block.
    pub per_skill_overrides: std::collections::HashMap<String, Vec<String>>,
}

impl EnvPassthroughPolicy {
    /// Construct a policy from a `[skills]` config block, or `None` when the
    /// config carries neither deny patterns nor per-skill overrides. Returning
    /// `None` lets the caller (and `KernelHandle::skill_env_passthrough_policy`)
    /// skip the operator-gate plumbing entirely — only the built-in
    /// `FORBIDDEN_PASSTHROUGH` and kernel-reserved hard blocks apply in that
    /// case. Note that `SkillsConfig::default()` ships with a non-empty deny
    /// list, so the default config still produces `Some(...)`; `None` only
    /// arises when an operator has explicitly cleared both fields.
    pub fn from_skills_config(cfg: &SkillsConfig) -> Option<Self> {
        if cfg.env_passthrough_denied_patterns.is_empty()
            && cfg.env_passthrough_per_skill.is_empty()
        {
            return None;
        }
        Some(Self {
            denied_patterns: cfg.env_passthrough_denied_patterns.clone(),
            per_skill_overrides: cfg.env_passthrough_per_skill.clone(),
        })
    }
}

fn default_env_passthrough_denied_patterns() -> Vec<String> {
    vec![
        "*_KEY".to_string(),
        "*_TOKEN".to_string(),
        "*_PASSWORD".to_string(),
        "*_SECRET".to_string(),
        "*_API_KEY".to_string(),
        "AWS_*".to_string(),
        "GITHUB_*".to_string(),
    ]
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            load_user: true,
            extra_dirs: Vec::new(),
            disabled: Vec::new(),
            env_passthrough_denied_patterns: default_env_passthrough_denied_patterns(),
            env_passthrough_per_skill: std::collections::HashMap::new(),
        }
    }
}

/// Extensions & integrations configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ExtensionsConfig {
    /// Enable auto-reconnect for MCP integrations.
    pub auto_reconnect: bool,
    /// Maximum reconnect attempts before giving up.
    pub reconnect_max_attempts: u32,
    /// Maximum backoff duration in seconds.
    pub reconnect_max_backoff_secs: u64,
    /// Health check interval in seconds.
    pub health_check_interval_secs: u64,
}

impl Default for ExtensionsConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            reconnect_max_attempts: 10,
            reconnect_max_backoff_secs: 300,
            health_check_interval_secs: 60,
        }
    }
}

/// Credential vault configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct VaultConfig {
    /// Whether the vault is enabled (auto-detected if vault.enc exists).
    pub enabled: bool,
    /// Custom vault file path (default: ~/.librefang/vault.enc).
    pub path: Option<PathBuf>,
    /// Whether to store the vault master key in the OS keyring
    /// (Linux Secret Service / macOS Keychain / Windows Credential Manager).
    ///
    /// `None` = use the platform default. macOS defaults to `false` because
    /// the Keychain ACL is per-binary signature: every `cargo build` produces
    /// a new signature and triggers a fresh "allow" prompt on daemon
    /// restart. Linux and Windows default to `true`.
    ///
    /// The env var `LIBREFANG_VAULT_NO_KEYRING=1` overrides this setting
    /// and forces the file-based fallback regardless of the config or
    /// platform default. The fallback path is resolved via
    /// `dirs::data_local_dir()`:
    /// - macOS: `~/Library/Application Support/librefang/.keyring`
    /// - Linux: `~/.local/share/librefang/.keyring`
    /// - Windows: `%LOCALAPPDATA%\librefang\.keyring`
    ///
    /// The file is AES-256-GCM-wrapped with an Argon2id-derived
    /// machine-fingerprint key and stored mode 0600.
    pub use_os_keyring: Option<bool>,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
            use_os_keyring: None,
        }
    }
}

/// Agent binding — routes specific channel/account/peer patterns to agents.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentBinding {
    /// Target agent name or ID.
    pub agent: String,
    /// Match criteria (all specified fields must match).
    pub match_rule: BindingMatchRule,
}

/// Match rule for agent bindings. All specified (non-None) fields must match.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BindingMatchRule {
    /// Channel type (e.g., "discord", "telegram", "slack").
    pub channel: Option<String>,
    /// Specific account/bot ID within the channel.
    pub account_id: Option<String>,
    /// Peer/user ID for DM routing.
    pub peer_id: Option<String>,
    /// Guild/server ID (Discord/Slack).
    pub guild_id: Option<String>,
    /// Role-based routing (user must have at least one).
    #[serde(default)]
    pub roles: Vec<String>,
}

impl BindingMatchRule {
    /// Calculate specificity score for binding priority ordering.
    /// Higher = more specific = checked first.
    pub fn specificity(&self) -> u32 {
        let mut score = 0u32;
        if self.peer_id.is_some() {
            score += 8;
        }
        if self.guild_id.is_some() {
            score += 4;
        }
        if !self.roles.is_empty() {
            score += 2;
        }
        if self.account_id.is_some() {
            score += 2;
        }
        if self.channel.is_some() {
            score += 1;
        }
        score
    }
}

/// Broadcast config — send same message to multiple agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BroadcastConfig {
    /// Broadcast strategy.
    pub strategy: BroadcastStrategy,
    /// Map of peer_id -> list of agent names to receive the message.
    pub routes: HashMap<String, Vec<String>>,
}

/// Broadcast delivery strategy.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum BroadcastStrategy {
    /// Send to all agents simultaneously.
    #[default]
    Parallel,
    /// Send to agents one at a time in order.
    Sequential,
}

/// Auto-reply engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AutoReplyConfig {
    /// Enable auto-reply engine. Default: false.
    pub enabled: bool,
    /// Max concurrent auto-reply tasks. Default: 3.
    pub max_concurrent: usize,
    /// Default timeout per reply in seconds. Default: 120.
    pub timeout_secs: u64,
    /// Patterns that suppress auto-reply (e.g., "/stop", "/pause").
    pub suppress_patterns: Vec<String>,
}

impl Default for AutoReplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent: 3,
            timeout_secs: 120,
            suppress_patterns: vec!["/stop".to_string(), "/pause".to_string()],
        }
    }
}

/// File-based input inbox configuration.
///
/// When enabled, the kernel polls a directory for text files and dispatches
/// their contents as messages to agents.  Files are moved to a `processed/`
/// subdirectory after delivery.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct InboxConfig {
    /// Enable inbox watcher. Default: false.
    pub enabled: bool,
    /// Directory to watch. Default: `~/.librefang/inbox/`
    pub directory: Option<String>,
    /// Poll interval in seconds. Default: 5.
    pub poll_interval_secs: u64,
    /// Default agent name to send files to when no `agent:` directive is found.
    pub default_agent: Option<String>,
}

impl Default for InboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: None,
            poll_interval_secs: 5,
            default_agent: None,
        }
    }
}

/// Default OTLP gRPC endpoint — matches the port the bundled observability
/// stack (Tempo / OTel collector) binds when
/// `auto_start_observability_stack = true`.
pub const DEFAULT_OTLP_ENDPOINT: &str = "http://localhost:4317";

/// Telemetry / observability configuration.
///
/// ```toml
/// [telemetry]
/// enabled = true                              # OpenTelemetry OTLP tracing
/// otlp_endpoint = "http://localhost:4317"
/// service_name = "librefang"
/// sample_rate = 1.0
/// prometheus_enabled = true                   # Prometheus metrics at /api/metrics
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Enable OpenTelemetry OTLP tracing export.
    pub enabled: bool,
    /// OTLP gRPC endpoint (default: "http://localhost:4317").
    pub otlp_endpoint: String,
    /// Service name reported to the OTel collector.
    pub service_name: String,
    /// Trace sampling rate (0.0 to 1.0). Default: 1.0 (sample everything).
    pub sample_rate: f64,
    /// Enable Prometheus metrics endpoint at /api/metrics.
    pub prometheus_enabled: bool,
    /// Auto-start the bundled observability Docker stack (Grafana, Prometheus,
    /// Tempo, OTel collector) on daemon boot. Default: `false`.
    ///
    /// Off by default because spinning up four containers on every `librefang
    /// start` is a strong implicit side-effect — operators usually prefer
    /// `librefang start` to leave the host untouched. Existing dashboards /
    /// custom OTel collectors keep working as long as `otlp_endpoint` points
    /// at them; the stack is only useful for the bundled local view.
    ///
    /// Issue #3136.
    pub auto_start_observability_stack: bool,
    /// Emit `x-librefang-{agent,session,step}-id` HTTP headers on outbound
    /// OpenAI-compatible LLM requests so observability sidecars (logging
    /// gateways, audit proxies, OTel collectors that shape spans from request
    /// metadata) can correlate request log records to the originating agent /
    /// session / agent-loop iteration without parsing the JSON body.
    /// Default: `true`.
    ///
    /// Set to `false` to suppress all three headers wire-side regardless of
    /// whether the kernel populated `CompletionRequest`'s caller-id fields.
    /// Useful for operators with strict zero-egress policies (regulated
    /// tenants, EU healthcare) who want no LibreFang-internal identifiers
    /// crossing the upstream-provider boundary, even though the IDs are
    /// opaque UUIDs / integers and carry no PII.
    ///
    /// Currently consulted only by the OpenAI-compatible driver. Other
    /// drivers (Anthropic, Gemini, Bedrock, Vertex, ChatGPT, Copilot,
    /// Claude Code, Codex, Gemini CLI, Qwen Code) do not emit these headers
    /// today; when they grow per-driver header-emission support, they will
    /// honour the same flag.
    pub emit_caller_trace_headers: bool,
}

impl TelemetryConfig {
    /// Whether OTLP exporter init should be skipped because no collector is
    /// reachable. Returns `true` when:
    ///
    /// - `otlp_endpoint` is empty (operator opted out), OR
    /// - `otlp_endpoint` is the default `http://localhost:4317` AND the bundled
    ///   observability stack is not actually running — either the operator
    ///   didn't opt in (`auto_start_observability_stack = false`), or they
    ///   opted in but startup failed (Docker missing, port conflict, compose
    ///   error). In both cases nothing listens on 4317 and the
    ///   `BatchSpanProcessor` would spam `ConnectionRefused` every export
    ///   interval.
    ///
    /// `stack_running` reflects the runtime fact, not the config intent — call
    /// sites pass `Some(handle).is_some()` (or equivalent) after attempting
    /// startup. Operators with an external collector on the default port
    /// should set `otlp_endpoint` to the collector's address to opt back in;
    /// the bundled-stack opt-in only helps when the stack actually comes up.
    pub fn otlp_export_disabled(&self, stack_running: bool) -> bool {
        if self.otlp_endpoint.is_empty() {
            return true;
        }
        self.otlp_endpoint == DEFAULT_OTLP_ENDPOINT && !stack_running
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            otlp_endpoint: DEFAULT_OTLP_ENDPOINT.to_string(),
            service_name: "librefang".to_string(),
            sample_rate: 1.0,
            prometheus_enabled: true,
            auto_start_observability_stack: false,
            emit_caller_trace_headers: true,
        }
    }
}

/// Configuration for prompt versioning and A/B testing.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PromptIntelligenceConfig {
    /// Enable prompt versioning and A/B testing. Default: false.
    pub enabled: bool,
    /// Hash prompts using SHA-256 for version identification. Default: true.
    pub hash_prompts: bool,
    /// Maximum number of versions to keep per agent. Default: 50.
    pub max_versions_per_agent: u32,
}

impl Default for PromptIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            hash_prompts: true,
            max_versions_per_agent: 50,
        }
    }
}

/// Canvas (Agent-to-UI) configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CanvasConfig {
    /// Enable canvas tool. Default: false.
    pub enabled: bool,
    /// Max HTML size in bytes. Default: 512KB.
    pub max_html_bytes: usize,
    /// Allowed HTML tags (empty = all safe tags allowed).
    #[serde(default)]
    pub allowed_tags: Vec<String>,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_html_bytes: 512 * 1024,
            allowed_tags: Vec::new(),
        }
    }
}

/// Shell/exec security mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExecSecurityMode {
    /// Block all shell execution.
    #[serde(alias = "none", alias = "disabled")]
    Deny,
    /// Only allow commands in safe_bins or allowed_commands.
    #[default]
    #[serde(alias = "restricted")]
    Allowlist,
    /// Allow all commands (unsafe, dev only).
    #[serde(alias = "allow", alias = "all", alias = "unrestricted")]
    Full,
}

/// Shell/exec security policy.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ExecPolicy {
    /// Security mode: "deny" blocks all, "allowlist" only allows listed,
    /// "full" allows all (unsafe, dev only).
    pub mode: ExecSecurityMode,
    /// Commands that bypass allowlist (stdin-only utilities).
    pub safe_bins: Vec<String>,
    /// Global command allowlist (when mode = allowlist).
    pub allowed_commands: Vec<String>,
    /// Environment variables explicitly allowed to pass through to `shell_exec`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_env_vars: Vec<String>,
    /// Max execution timeout in seconds. Default: 30.
    pub timeout_secs: u64,
    /// Max output size in bytes. Default: 100KB.
    pub max_output_bytes: usize,
    /// No-output idle timeout in seconds. When > 0, kills processes that
    /// produce no stdout/stderr output for this duration. Default: 30.
    #[serde(default = "default_no_output_timeout")]
    pub no_output_timeout_secs: u64,
}

fn default_no_output_timeout() -> u64 {
    30
}

impl Default for ExecPolicy {
    fn default() -> Self {
        Self {
            mode: ExecSecurityMode::default(),
            safe_bins: vec![
                "sleep", "true", "false", "cat", "sort", "uniq", "cut", "tr", "head", "tail", "wc",
                "date", "echo", "printf", "basename", "dirname", "pwd", "env",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            allowed_commands: Vec::new(),
            allowed_env_vars: Vec::new(),
            timeout_secs: 30,
            max_output_bytes: 100 * 1024,
            no_output_timeout_secs: default_no_output_timeout(),
        }
    }
}

// ---------------------------------------------------------------------------
// Gap 2: No-output idle timeout for subprocess sandbox
// ---------------------------------------------------------------------------

/// Reason a subprocess was terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationReason {
    /// Process exited normally.
    Exited(i32),
    /// Absolute timeout exceeded.
    AbsoluteTimeout,
    /// No output timeout exceeded.
    NoOutputTimeout,
}

// ---------------------------------------------------------------------------
// Gap 3: Auth profile rotation — multi-key per provider
// ---------------------------------------------------------------------------

/// A named authentication profile for a provider.
///
/// Multiple profiles can be configured per provider to enable key rotation
/// when one key gets rate-limited or has billing issues.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AuthProfile {
    /// Profile name (e.g., "primary", "secondary").
    pub name: String,
    /// Environment variable holding the API key.
    pub api_key_env: String,
    /// Priority (lower = preferred). Default: 0.
    #[serde(default)]
    pub priority: u32,
}

/// SECURITY: Custom Debug impl redacts env var name.
impl std::fmt::Debug for AuthProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthProfile")
            .field("name", &self.name)
            .field("api_key_env", &"<redacted>")
            .field("priority", &self.priority)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Gap 5: Docker sandbox maturity
// ---------------------------------------------------------------------------

/// Docker sandbox activation mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DockerSandboxMode {
    /// Docker sandbox disabled.
    #[default]
    Off,
    /// Only use Docker for non-main agents.
    NonMain,
    /// Use Docker for all agents.
    All,
}

/// Docker container lifecycle scope.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DockerScope {
    /// Container per session (destroyed when session ends).
    #[default]
    Session,
    /// Container per agent (reused across sessions).
    Agent,
    /// Shared container pool.
    Shared,
}

// ---------------------------------------------------------------------------
// Gap 6: Typing indicator modes
// ---------------------------------------------------------------------------

/// Typing indicator behavior mode.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TypingMode {
    /// Send typing indicator immediately on message receipt (default).
    #[default]
    Instant,
    /// Send typing indicator only when first text delta arrives.
    Message,
    /// Send typing indicator only during LLM reasoning.
    Thinking,
    /// Never send typing indicators.
    Never,
}

// ---------------------------------------------------------------------------
// Gap 7: Thinking level support
// ---------------------------------------------------------------------------

/// Extended thinking configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ThinkingConfig {
    /// Maximum tokens for thinking (budget).
    pub budget_tokens: u32,
    /// Whether to stream thinking tokens to the client.
    pub stream_thinking: bool,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            budget_tokens: 10_000,
            stream_thinking: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Gap 8: Structured output / response format
// ---------------------------------------------------------------------------

/// Desired response format from the LLM.
///
/// - `Text` — default free-form text (no constraint).
/// - `Json` — ask the model to respond with valid JSON (`json_object` mode).
/// - `JsonSchema` — constrain output to a specific JSON Schema (OpenAI
///   `json_schema` mode; for providers without native support the schema is
///   injected into the system prompt).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Free-form text (default behaviour).
    #[default]
    Text,
    /// Valid JSON object (no schema constraint).
    Json,
    /// JSON output that must conform to the supplied schema.
    JsonSchema {
        /// Schema name (sent to OpenAI as `json_schema.name`).
        name: String,
        /// The JSON Schema definition.
        schema: serde_json::Value,
        /// Whether to enable strict schema adherence (OpenAI).
        #[serde(default)]
        strict: Option<bool>,
    },
}

/// Configuration for a sidecar channel adapter (external process-based).
///
/// Sidecar adapters allow external processes written in any language to act as
/// channel adapters. Communication uses newline-delimited JSON over stdin/stdout.
///
/// Configure in config.toml:
/// ```toml
/// [[sidecar_channels]]
/// name = "my-telegram"
/// command = "python3"
/// args = ["adapters/telegram_adapter.py"]
/// env = { TELEGRAM_BOT_TOKEN = "xxx" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SidecarChannelConfig {
    /// Display name for this adapter.
    pub name: String,
    /// Command to execute (e.g., "python3", "/usr/local/bin/my-adapter").
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables to pass to the subprocess.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Channel type identifier (defaults to Custom(name)).
    #[serde(default)]
    pub channel_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Session auto-reset policy types
// ---------------------------------------------------------------------------

/// Which automatic-reset strategy is active for a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionResetMode {
    /// No automatic reset. Sessions persist indefinitely. (default)
    #[default]
    Off,
    /// Reset after `idle_minutes` of inactivity.
    Idle,
    /// Reset once per day at `daily_at_hour` (local clock, 0-23).
    Daily,
    /// Reset when *either* idle or daily condition is satisfied.
    Both,
}

/// Why a session was last reset (stored on [`AgentEntry`] for observability).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionResetReason {
    /// Last-active exceeded `idle_minutes`.
    Idle,
    /// The daily fixed-time boundary was crossed.
    Daily,
    /// Session was flagged `suspended` (forced by operator / stuck-loop recovery).
    Suspended,
    /// Manual reset requested via API or CLI.
    Manual,
}

impl std::fmt::Display for SessionResetReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => f.write_str("idle"),
            Self::Daily => f.write_str("daily"),
            Self::Suspended => f.write_str("suspended"),
            Self::Manual => f.write_str("manual"),
        }
    }
}

/// Per-session auto-reset policy.
///
/// Configured inside `[session.reset]` in `config.toml`:
/// ```toml
/// [session.reset]
/// mode = "idle"
/// idle_minutes = 1440   # 24 h
///
/// # or
/// mode = "both"
/// idle_minutes = 60
/// daily_at_hour = 4
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionResetPolicy {
    /// Which reset strategy (or strategies) to apply.
    pub mode: SessionResetMode,
    /// Inactivity threshold in minutes for `Idle` / `Both` modes.
    /// Default: 1440 (24 hours).
    pub idle_minutes: u64,
    /// Hour of day (0–23, local clock) at which the `Daily` / `Both` reset fires.
    /// Default: 4 (04:00 local).
    pub daily_at_hour: u8,
}

impl Default for SessionResetPolicy {
    fn default() -> Self {
        Self {
            mode: SessionResetMode::Off,
            idle_minutes: 1440,
            daily_at_hour: 4,
        }
    }
}

// ---------------------------------------------------------------------------

/// Session retention policy configuration.
///
/// Controls automatic cleanup of idle or excess sessions and optional
/// startup prompt injection.
/// Configure in `config.toml`:
/// ```toml
/// [session]
/// retention_days = 30
/// max_sessions_per_agent = 100
/// cleanup_interval_hours = 24
/// reset_prompt = "You are a helpful coding assistant. Always respond in English."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionConfig {
    /// Maximum age for idle sessions before automatic cleanup (days, 0 = unlimited).
    pub retention_days: u32,
    /// Maximum number of sessions per agent (oldest pruned first, 0 = unlimited).
    pub max_sessions_per_agent: u32,
    /// How often the cleanup job runs (in hours).
    pub cleanup_interval_hours: u32,
    /// Optional message injected as the first system message when a new session
    /// starts or when the session is reset. Useful for setting up persistent
    /// context or instructions across all agents.
    #[serde(default)]
    pub reset_prompt: Option<String>,
    /// Context injections applied to every new or reset session.
    /// Each entry specifies content, a positional slot, and an optional condition.
    #[serde(default)]
    pub context_injection: Vec<ContextInjection>,
    /// Optional shell script to run when a new session is created (fire-and-forget).
    #[serde(default)]
    pub on_session_start_script: Option<String>,
    /// Automatic session-reset policy (idle timeout and/or daily fixed-time reset).
    /// Default: `mode = "off"` — no automatic resets, fully backward-compatible.
    #[serde(default)]
    pub reset: SessionResetPolicy,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            retention_days: 0,
            max_sessions_per_agent: 0,
            cleanup_interval_hours: 24,
            reset_prompt: None,
            context_injection: Vec::new(),
            on_session_start_script: None,
            reset: SessionResetPolicy::default(),
        }
    }
}

/// Session compaction configuration (exposed in `[compaction]` TOML section).
///
/// Controls when and how the LLM-based history compaction runs.
/// Internal algorithmic ratios (base_chunk_ratio, safety_margin, etc.) are kept
/// as private constants inside the runtime compactor and are not exposed here.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CompactionTomlConfig {
    /// Number of messages that triggers compaction (default: 30).
    #[serde(default = "default_compaction_threshold")]
    pub threshold_messages: usize,
    /// Number of recent messages to preserve verbatim (default: 10).
    #[serde(default = "default_compaction_keep_recent")]
    pub keep_recent: usize,
    /// Maximum tokens for summary output (default: 1024).
    #[serde(default = "default_compaction_max_summary_tokens")]
    pub max_summary_tokens: usize,
    /// Token threshold ratio to trigger compaction (default: 0.7).
    /// Compaction fires when estimated session tokens exceed this fraction
    /// of the model's context window.
    #[serde(default = "default_compaction_token_threshold_ratio")]
    pub token_threshold_ratio: f64,
    /// Maximum characters per summarization chunk (default: 80000).
    #[serde(default = "default_compaction_max_chunk_chars")]
    pub max_chunk_chars: usize,
    /// Maximum retries for LLM summarization (default: 3).
    #[serde(default = "default_compaction_max_retries")]
    pub max_retries: u32,
}

fn default_compaction_threshold() -> usize {
    30
}
fn default_compaction_keep_recent() -> usize {
    10
}
fn default_compaction_max_summary_tokens() -> usize {
    1024
}
fn default_compaction_token_threshold_ratio() -> f64 {
    0.7
}
fn default_compaction_max_chunk_chars() -> usize {
    80_000
}
fn default_compaction_max_retries() -> u32 {
    3
}

impl Default for CompactionTomlConfig {
    fn default() -> Self {
        Self {
            threshold_messages: default_compaction_threshold(),
            keep_recent: default_compaction_keep_recent(),
            max_summary_tokens: default_compaction_max_summary_tokens(),
            token_threshold_ratio: default_compaction_token_threshold_ratio(),
            max_chunk_chars: default_compaction_max_chunk_chars(),
            max_retries: default_compaction_max_retries(),
        }
    }
}

/// Where a context injection should be placed in the session message list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InjectionPosition {
    /// Prepended to the system prompt area.
    #[default]
    System,
    /// Inserted right before the latest user message.
    BeforeUser,
    /// Placed immediately after the reset prompt (if any).
    AfterReset,
}

/// A single context injection entry.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextInjection {
    /// A short label for logging / debugging.
    pub name: String,
    /// The content to inject.
    pub content: String,
    /// Where in the message list this content should appear.
    #[serde(default)]
    pub position: InjectionPosition,
    /// Optional condition expression (e.g. `"agent.tags contains 'chat'"`).
    /// If `None`, the injection always applies.
    #[serde(default)]
    pub condition: Option<String>,
}

/// Message queue configuration.
///
/// Controls queue depth limits and task TTL for the agent command queue.
///
/// Configure in config.toml:
/// ```toml
/// [queue]
/// max_depth_per_agent = 100
/// max_depth_global = 1000
/// task_ttl_secs = 3600
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct QueueConfig {
    /// Max queue depth per agent (0 = unlimited).
    pub max_depth_per_agent: u32,
    /// Max queue depth globally (0 = unlimited).
    pub max_depth_global: u32,
    /// Task TTL in seconds (unprocessed tasks expire, 0 = unlimited).
    pub task_ttl_secs: u64,
    /// Per-lane concurrency limits.
    #[serde(default)]
    pub concurrency: QueueConcurrencyConfig,
    /// How many days to keep `completed` / `failed` / `cancelled` rows in
    /// `task_queue` before the periodic retention sweep hard-deletes them.
    /// Default: 7. Set to 0 to disable pruning (queue grows forever — #3466).
    #[serde(default = "default_task_queue_retention_days")]
    pub task_queue_retention_days: u64,
}

fn default_task_queue_retention_days() -> u64 {
    7
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_depth_per_agent: 0,
            max_depth_global: 0,
            task_ttl_secs: 3600,
            concurrency: QueueConcurrencyConfig::default(),
            task_queue_retention_days: default_task_queue_retention_days(),
        }
    }
}

/// Per-lane concurrency limits for the command queue.
///
/// Configure in config.toml:
/// ```toml
/// [queue.concurrency]
/// main_lane = 3
/// cron_lane = 2
/// subagent_lane = 3
/// trigger_lane = 8
/// default_per_agent = 1
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct QueueConcurrencyConfig {
    /// Main lane concurrent limit (user messages).
    pub main_lane: usize,
    /// Cron lane concurrent limit (scheduled jobs).
    pub cron_lane: usize,
    /// Subagent lane concurrent limit (child agents).
    pub subagent_lane: usize,
    /// Trigger lane concurrent limit — global cap on event-trigger
    /// (`TaskPosted`, `MessageReceived`, …) dispatches in flight at the
    /// same time, across all agents. Acquired BEFORE the per-agent
    /// semaphore so a single hot agent cannot starve the kernel.
    /// Default `8`. `0` is rewritten to `1` by validation.
    pub trigger_lane: usize,
    /// Default per-agent invocation cap when an agent's manifest does
    /// not set `max_concurrent_invocations`. `1` reproduces the
    /// legacy per-agent-mutex serialization that pre-existed this
    /// knob — change deliberately. `0` is rewritten to `1` by
    /// validation. Typed `usize` to match the sibling lane fields and
    /// to feed `Semaphore::new` without a cast.
    pub default_per_agent: usize,
    /// Per-fire timeout (seconds) for trigger dispatches. Bounds the
    /// duration a single fire holds its `Lane::Trigger` and per-agent
    /// permits, preventing one stuck LLM call from starving every
    /// other agent's triggers kernel-wide (issue #3446). `0` is
    /// rewritten to the default by validation.
    pub trigger_fire_timeout_secs: u64,
}

impl Default for QueueConcurrencyConfig {
    fn default() -> Self {
        Self {
            main_lane: 3,
            cron_lane: 2,
            subagent_lane: 3,
            trigger_lane: 8,
            default_per_agent: 1,
            trigger_fire_timeout_secs: 300,
        }
    }
}

/// Task-board (shared-task-queue) safety knobs.
///
/// When a worker agent calls `task_claim` the task transitions to
/// `in_progress` and `claimed_at` is stamped. If the worker's LLM stalls
/// (empty response, crash, timeout) it will never call `task_complete`
/// and the task would otherwise stay `in_progress` forever — no retry,
/// no external signal, no way for the delegator to know something broke
/// (issues #2923, #2926).
///
/// The sweeper runs in the kernel and flips stuck tasks back to `pending`
/// so they can be reclaimed, clearing `assigned_to` in the process.
///
/// Configure in config.toml:
/// ```toml
/// [task_board]
/// claim_ttl_secs = 600         # 10 minutes — auto-reset after this
/// sweep_interval_secs = 30     # how often the sweeper runs
/// ```
///
/// Setting `claim_ttl_secs = 0` disables the sweeper entirely — useful
/// for long-running human-in-the-loop tasks where a 10 minute reset
/// would be wrong.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TaskBoardConfig {
    /// How long an `in_progress` task may stay claimed before the sweeper
    /// resets it to `pending`. Default: 600 s (10 minutes). 0 disables.
    pub claim_ttl_secs: u64,
    /// How often the sweeper scans for stuck tasks. Default: 30 s.
    pub sweep_interval_secs: u64,
    /// Maximum number of auto-resets before a stuck task is marked `failed`.
    /// Default: 0 = no limit (retry indefinitely).
    pub max_retries: u32,
}

impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            claim_ttl_secs: 600,
            sweep_interval_secs: 30,
            max_retries: 0,
        }
    }
}

/// HTTP proxy configuration.
///
/// Configure in config.toml:
/// ```toml
/// [proxy]
/// http_proxy = "http://proxy.corp.example:8080"
/// https_proxy = "http://proxy.corp.example:8080"
/// no_proxy = "localhost,127.0.0.1,.internal.corp"
/// ```
///
/// Environment variables `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` are also
/// respected as fallbacks when the config fields are empty.
#[derive(Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ProxyConfig {
    /// HTTP proxy URL (e.g. `http://proxy:8080`).
    /// Falls back to `HTTP_PROXY` / `http_proxy` env var.
    #[serde(default)]
    pub http_proxy: Option<String>,
    /// HTTPS proxy URL (e.g. `http://proxy:8080`).
    /// Falls back to `HTTPS_PROXY` / `https_proxy` env var.
    #[serde(default)]
    pub https_proxy: Option<String>,
    /// Comma-separated list of hosts/domains that should bypass the proxy.
    /// Falls back to `NO_PROXY` / `no_proxy` env var.
    #[serde(default)]
    pub no_proxy: Option<String>,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field(
                "http_proxy",
                &self.http_proxy.as_deref().map(redact_proxy_url),
            )
            .field(
                "https_proxy",
                &self.https_proxy.as_deref().map(redact_proxy_url),
            )
            .field("no_proxy", &self.no_proxy)
            .finish()
    }
}

/// Redact credentials from a proxy URL for safe logging.
///
/// Turns `http://user:pass@host:port/path` into `http://***@host:port/path`.
/// Returns the URL unchanged if it contains no `@` (no credentials).
pub fn redact_proxy_url(url: &str) -> String {
    // Find the scheme separator "://"
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        // If there is an `@`, credentials are present before it
        if let Some(at_pos) = after_scheme.find('@') {
            let host_and_rest = &after_scheme[at_pos..]; // includes '@'
            return format!("{}://***{}", &url[..scheme_end], host_and_rest);
        }
    }
    url.to_string()
}

// ── Trigger system defaults ────────────────────────────────────────────

fn default_trigger_cooldown_secs() -> u64 {
    5
}
fn default_max_triggers_per_event() -> usize {
    10
}
fn default_max_trigger_depth() -> usize {
    5
}
fn default_max_workflow_secs() -> u64 {
    3600
}

/// Event-driven trigger system configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriggersConfig {
    /// Default cooldown between trigger firings in seconds (default: 5).
    #[serde(default = "default_trigger_cooldown_secs")]
    pub cooldown_secs: u64,
    /// Maximum triggers that can fire per single event (default: 10).
    #[serde(default = "default_max_triggers_per_event")]
    pub max_per_event: usize,
    /// Maximum trigger recursion depth (default: 5).
    #[serde(default = "default_max_trigger_depth")]
    pub max_depth: usize,
    /// Maximum workflow execution time in seconds (default: 3600).
    #[serde(default = "default_max_workflow_secs")]
    pub max_workflow_secs: u64,
}

impl Default for TriggersConfig {
    fn default() -> Self {
        Self {
            cooldown_secs: default_trigger_cooldown_secs(),
            max_per_event: default_max_triggers_per_event(),
            max_depth: default_max_trigger_depth(),
            max_workflow_secs: default_max_workflow_secs(),
        }
    }
}

/// Configuration for the Universal Agent Runtime (UAR) integration.
///
/// All fields are optional; unset values fall back to the corresponding
/// `UAR_LLM__*` / `LLM_*` environment variables that the UAR driver already
/// consults at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct UarConfig {
    /// LLM API key forwarded to the UAR `LiterLlmDriver`.
    ///
    /// Takes precedence over `UAR_LLM__API_KEY` and `LLM_API_KEY` env vars.
    /// Use `vault:KEY` syntax to store securely:
    /// `api_key = "vault:uar_api_key"`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,

    /// Default model in `provider/model` format (e.g. `"openai/gpt-4o"`).
    ///
    /// Takes precedence over `UAR_LLM__MODEL` and `LLM_MODEL` env vars.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,

    /// Filesystem path for the embedded SurrealDB (RocksDB) data store.
    ///
    /// Defaults to `<kernel.data_dir>/uar-surreal`.  Set an explicit path
    /// when you want to place the UAR database on a separate volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surreal_data_dir: Option<PathBuf>,

    /// Optional base URL override for the UAR LLM backend.
    ///
    /// Useful for pointing at a local proxy (e.g. `"http://localhost:4000"`)
    /// without changing the model string.  Takes precedence over
    /// `UAR_LLM__BASE_URL` and `LLM_BASE_URL` env vars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// When set, UAR connects to this remote SurrealDB instance instead of
    /// the embedded path in [`Self::surreal_data_dir`].
    ///
    /// If both fields are set, [`Self::remote`] wins. This is the second
    /// half of the shared-remote-instance design from Phase 4b of the
    /// `surrealdb-storage-swap` plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteSurrealConfig>,

    /// When `true`, UAR reuses the same `RemoteSurrealConfig` as
    /// `KernelConfig.storage` (with `namespace = "uar"` so isolation is
    /// preserved). Defaults to `false`.
    #[serde(default)]
    pub share_librefang_storage: bool,
}

/// Top-level kernel configuration.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct KernelConfig {
    /// Configuration schema version for automatic migration.
    /// Old configs without this field default to 1 (via `default_config_version`).
    #[serde(default = "super::version::default_config_version")]
    pub config_version: u32,
    /// LibreFang home directory (default: ~/.librefang).
    pub home_dir: PathBuf,
    /// Data directory for databases (default: ~/.librefang/data).
    pub data_dir: PathBuf,
    /// Log level (trace, debug, info, warn, error).
    pub log_level: String,
    /// API listen address (e.g., "0.0.0.0:4545").
    #[serde(alias = "listen_addr")]
    pub api_listen: String,
    /// Allowed CORS origins. When non-empty, these origins are added to the
    /// CORS allow list (in addition to localhost). Accepts exact origin strings
    /// like `"https://dash.example.com"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cors_origin: Vec<String>,
    /// Hostnames allowed to drive the OAuth `redirect_uri` when starting an
    /// MCP auth flow. The MCP auth-start handler derives the callback URL
    /// from the incoming request's `Origin` / `X-Forwarded-Host` / `Host`
    /// headers; without an allowlist a spoofed Host header could redirect
    /// the authorization code to an attacker-controlled origin. Loopback
    /// addresses (`localhost`, `127.0.0.1`, `::1`) are always accepted so
    /// local development keeps working with an empty list. Entries are
    /// hostnames without port, e.g. `"dash.example.com"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_hosts: Vec<String>,
    /// CIDR ranges (or single IPs) of reverse proxies that are trusted to
    /// set forwarding headers (`X-Forwarded-For`, `X-Real-IP`,
    /// `CF-Connecting-IP`, `Forwarded`). Used together with
    /// `trust_forwarded_for`: header trust is **only** applied when the
    /// TCP peer matches one of these entries.
    ///
    /// Without this allowlist, trusting forwarding headers lets any
    /// internet client forge a per-request source IP and bypass per-IP
    /// rate limits and connection caps. The allowlist closes that hole
    /// by gating header trust on a verified upstream proxy.
    ///
    /// Entries are CIDRs (`"172.19.0.0/16"`, `"10.0.0.0/8"`,
    /// `"2001:db8::/32"`) or bare IPs (`"127.0.0.1"`, `"::1"`). An empty
    /// list (default) disables header trust regardless of
    /// `trust_forwarded_for`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_proxies: Vec<String>,
    /// Master switch for forwarding-header trust. Defaults to `false`.
    /// When `true` AND the TCP peer matches `trusted_proxies`, the
    /// daemon resolves the real client IP from forwarding headers
    /// (preference: `CF-Connecting-IP` → `X-Real-IP` → `Forwarded`
    /// (RFC 7239) → rightmost-untrusted hop in `X-Forwarded-For`).
    /// Without both flags set, header trust stays off and the TCP peer
    /// is used everywhere — the safe default for any non-proxied
    /// deployment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trust_forwarded_for: bool,
    /// Host directories under which `agent.toml: [workspaces].<name>.mount`
    /// declarations may resolve. Each declared mount is canonicalized at
    /// boot and must be a path prefix of one of these (also canonicalized)
    /// roots; otherwise it is rejected with a warning. Empty (default)
    /// denies all external mounts — the safe default. See issue #3230.
    ///
    /// Example:
    /// ```toml
    /// allowed_mount_roots = [
    ///   "/Users/alice/Documents",
    ///   "/data/shared",
    /// ]
    /// ```
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_mount_roots: Vec<PathBuf>,
    /// Whether to enable the OFP network layer.
    pub network_enabled: bool,
    /// Operator override for the agent-loop iteration cap. When set, any
    /// agent without its own `[autonomous] max_iterations` uses this value
    /// instead of the compiled-in default
    /// (`AutonomousConfig::DEFAULT_MAX_ITERATIONS`). Lower it when running
    /// cheap models to bound cost per turn; raise it for long-horizon
    /// autonomous agents. `None` means "use the compiled-in default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_max_iterations: Option<u32>,
    /// Operator override for the agent message-history trim cap. When set,
    /// any agent without its own `max_history_messages` uses this value
    /// instead of the compiled-in default
    /// (`agent_loop::DEFAULT_MAX_HISTORY_MESSAGES`). Lower it to bound
    /// per-turn token cost; raise it for long-context models. `None` means
    /// "use the compiled-in default". Values below 4 are silently clamped
    /// at runtime with a warning log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_history_messages: Option<usize>,
    /// Kernel-wide Smart Model Router defaults applied to any agent whose
    /// `agent.toml` does not set its own `[routing]` block. The `init` wizard
    /// writes user-selected tier models here under `[default_routing]` so the
    /// chosen routing actually reaches the kernel — see issue #4466. Per-agent
    /// `routing` always wins when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_routing: Option<crate::agent::ModelRoutingConfig>,
    /// Default LLM provider configuration.
    pub default_model: DefaultModelConfig,
    /// Memory substrate configuration.
    pub memory: MemoryConfig,
    /// Memory wiki: durable markdown knowledge vault with provenance and
    /// optional Obsidian-friendly export. Off by default; see
    /// `librefang-memory-wiki` and issue #3329 for the runbook.
    #[serde(default)]
    pub memory_wiki: MemoryWikiConfig,
    /// Network configuration.
    pub network: NetworkConfig,
    /// Channel bridge configuration (Telegram, etc.).
    pub channels: ChannelsConfig,
    /// API authentication key. When set, all API endpoints (except /api/health)
    /// require a `Authorization: Bearer <key>` header.
    /// If empty, the API is unauthenticated (local development only).
    pub api_key: String,
    /// Controls whether the dashboard read-endpoint allowlist (agents,
    /// config, budget, sessions, approvals, hands, skills, workflows, …)
    /// requires a bearer token.
    ///
    /// * `None` (default, unset in config.toml) — **derive from
    ///   configured auth**: the reads allowlist is collapsed *automatically*
    ///   whenever any authentication is configured (non-empty `api_key`,
    ///   per-user keys, or dashboard credentials). This is the safe
    ///   default: operators who already set an `api_key` shouldn't also
    ///   have to remember a separate flag before their read endpoints
    ///   stop leaking agent IDs to the LAN.
    /// * `Some(true)` — state the intent explicitly. The daemon logs a
    ///   boot-time warning if no authentication is actually configured
    ///   (so an accidental `api_key = ""` redeploy is visible in the
    ///   logs), but the middleware itself only enforces the closed
    ///   allowlist when some form of auth is also configured: with no
    ///   `api_key`, user keys, or dashboard credentials there is nothing
    ///   to authenticate against and reads fall through to the
    ///   unauthenticated local-development bypass. Configure an
    ///   `api_key` (or per-user keys / dashboard credentials) alongside
    ///   this flag to actually close the allowlist.
    /// * `Some(false)` — force the allowlist open even when `api_key`
    ///   is set. Provided as an explicit escape hatch for deployments
    ///   that front the daemon with an external auth proxy and want the
    ///   in-tree dashboard to keep rendering before the reverse proxy
    ///   has attached its own credentials.
    ///
    /// Unauthenticated static assets, OAuth flow endpoints, and
    /// `/api/health*` stay reachable in every mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_auth_for_reads: Option<bool>,
    /// Hex-encoded Ed25519 public keys (32 bytes → 64 hex chars) allowed to
    /// sign agent manifests. `verify_signed_manifest` requires the envelope's
    /// `signer_public_key` to be on this list before accepting a signature —
    /// without a trust anchor, a self-signed envelope from any attacker
    /// passes internal-consistency checks and would be indistinguishable
    /// from a legitimate one. When empty, `SignedManifest` JSON payloads are
    /// rejected outright (fail-closed). Raw unsigned TOML manifests are
    /// unaffected; this list only gates the signed-envelope path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_manifest_signers: Vec<String>,
    /// Dashboard login username. When both dashboard_user and dashboard_pass
    /// are set, the dashboard requires username/password login.
    /// Can also be set via `LIBREFANG_DASHBOARD_USER` env var.
    #[serde(default)]
    pub dashboard_user: String,
    /// Dashboard login password. Can also be set via `LIBREFANG_DASHBOARD_PASS`
    /// env var. **Recommended**: use `vault:KEY` syntax for secure storage.
    /// Example: `dashboard_pass = "vault:dashboard_password"`
    /// then run `librefang vault set dashboard_password`.
    #[serde(default)]
    pub dashboard_pass: String,
    /// Argon2id hash of the dashboard password (PHC-format string).
    /// When set, the password is verified against this hash instead of
    /// the plaintext `dashboard_pass` value. Populated automatically on
    /// first successful login (transparent upgrade from plaintext).
    #[serde(default)]
    pub dashboard_pass_hash: String,
    /// Kernel operating mode (stable, default, dev).
    #[serde(default)]
    pub mode: KernelMode,
    /// Language/locale for CLI and messages (default: "en").
    #[serde(default = "default_language")]
    pub language: String,
    /// User configurations for RBAC multi-user support.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<UserConfig>,
    /// Maps platform-native channel roles (Telegram admin, Discord guild
    /// roles, Slack workspace roles) to LibreFang `UserRole`. Used by
    /// `AuthManager::resolve_role_for_sender` after explicit `UserConfig.role`
    /// is consulted (explicit beats channel-derived; both beat default-deny).
    #[serde(default, skip_serializing_if = "ChannelRoleMapping::is_empty")]
    pub channel_role_mapping: ChannelRoleMapping,
    /// MCP server configurations for external tool integration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerConfigEntry>,
    /// Reusable named taint rule sets referenced by
    /// [`McpTaintToolPolicy::rule_sets`]. Each entry defines a group of
    /// taint rules with a severity action (block / warn / log) that the
    /// MCP scanner applies to every tool that opts in.
    ///
    /// **Hot-reload caveat:** the kernel snapshots this list onto each
    /// connected MCP server at install / reload time. Edits to
    /// `[[taint_rules]]` followed by a config reload do NOT propagate to
    /// already-connected MCP servers until the server itself is reloaded
    /// (e.g. via `reload_mcp_server_config` or a daemon restart). The
    /// snapshot keeps the scanner's view stable for the lifetime of a
    /// single tool call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taint_rules: Vec<NamedTaintRuleSet>,
    /// A2A (Agent-to-Agent) protocol configuration.
    #[serde(default)]
    pub a2a: Option<A2aConfig>,
    /// Usage footer mode (what to show after each response).
    #[serde(default)]
    pub usage_footer: UsageFooterMode,
    /// Cost optimization mode for stable prompt prefixes.
    ///
    /// When enabled, LibreFang avoids volatile system-prompt additions that
    /// change every turn (for example recalled memory append and canonical
    /// context injection), improving provider-side prompt cache hit rates.
    #[serde(default)]
    pub stable_prefix_mode: bool,
    /// Web tools configuration (search + fetch).
    #[serde(default)]
    pub web: WebConfig,
    /// Fallback providers tried in order if the primary fails.
    /// Configure in config.toml as `[[fallback_providers]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_providers: Vec<FallbackProviderConfig>,
    /// `[llm]` section — currently carries the auxiliary side-task chain
    /// configuration. See [`LlmConfig`] / [`AuxiliaryConfig`].
    #[serde(default)]
    pub llm: LlmConfig,
    /// Browser automation configuration.
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Extensions & integrations configuration.
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    /// Skills configuration (bundled + user-installed skills).
    #[serde(default)]
    pub skills: SkillsConfig,
    /// Credential vault configuration.
    #[serde(default)]
    pub vault: VaultConfig,
    /// Root directory for agent workspaces. Default: `~/.librefang/workspaces`
    #[serde(default)]
    pub workspaces_dir: Option<PathBuf>,
    /// Custom log directory. When set, log files are written here instead of
    /// the default `~/.librefang/` directory.
    #[serde(default)]
    pub log_dir: Option<PathBuf>,
    /// Media understanding configuration.
    #[serde(default)]
    pub media: crate::media::MediaConfig,
    /// Link understanding configuration.
    #[serde(default)]
    pub links: crate::media::LinkConfig,
    /// Config hot-reload settings.
    #[serde(default)]
    pub reload: ReloadConfig,
    /// Webhook trigger configuration (external event injection).
    #[serde(default)]
    pub webhook_triggers: Option<WebhookTriggerConfig>,
    /// Event-driven trigger system configuration (cooldowns, depth limits, etc.).
    #[serde(default)]
    pub triggers: TriggersConfig,
    /// Execution approval policy.
    #[serde(default, alias = "approval_policy")]
    pub approval: crate::approval::ApprovalPolicy,
    /// Notification engine configuration for approval alerts and task state notifications.
    #[serde(default)]
    pub notification: crate::approval::NotificationConfig,
    /// Cron scheduler max total jobs across all agents. Default: 500.
    #[serde(default = "default_max_cron_jobs")]
    pub max_cron_jobs: usize,
    /// Maximum estimated token count for the cron session before automatic
    /// pruning. Oldest messages are removed from the front of the session
    /// until the estimated count falls below this threshold.
    ///
    /// `None` (default) disables pruning and preserves existing behaviour.
    /// Set to e.g. `100000` for a rolling 100k-token window.
    #[serde(default)]
    pub cron_session_max_tokens: Option<u64>,
    /// Maximum number of messages retained in a cron session. When the
    /// session exceeds this count the oldest messages are pruned before
    /// each cron fire. Applied in addition to `cron_session_max_tokens`.
    ///
    /// `None` (default) disables message-count pruning.
    #[serde(default)]
    pub cron_session_max_messages: Option<usize>,
    /// Fraction of the effective token budget (post-prune) at which the
    /// kernel emits a `tracing::warn!` for a Persistent cron session
    /// approaching the provider context window. Closes the operator-
    /// visibility gap from #3693: pruning prevents the hard 400 from
    /// the provider, but without this warn the trend is invisible until
    /// a fire actually fails.
    ///
    /// Applied against the effective limit:
    ///   1. `cron_session_max_tokens` if set, else
    ///   2. [`Self::cron_session_warn_total_tokens`] as a fallback ceiling.
    ///
    /// Skipped entirely when both are `None`, or when this value is
    /// `None` / `<= 0.0` / `> 1.0`.
    ///
    /// Default: `Some(0.8)` — warn at 80% of the budget.
    #[serde(default = "default_cron_session_warn_fraction")]
    pub cron_session_warn_fraction: Option<f64>,
    /// Fallback context-window ceiling used by
    /// [`Self::cron_session_warn_fraction`] when
    /// `cron_session_max_tokens` is unset. Lets operators get growth
    /// warnings even on agents that have not opted into pruning.
    ///
    /// Default: `Some(200_000)` — matches the typical Claude / GPT-4
    /// long-context window. Set to `None` to disable the fallback
    /// (warn only fires when `cron_session_max_tokens` is explicitly
    /// configured).
    #[serde(default = "default_cron_session_warn_total_tokens")]
    pub cron_session_warn_total_tokens: Option<u64>,
    /// Compaction strategy applied when the cron session exceeds
    /// `cron_session_max_tokens` or `cron_session_max_messages` (#3693).
    ///
    /// - `"prune"` (default) — drop oldest messages from the front until the
    ///   budget is satisfied. Identical to the pre-#3693 behaviour.
    /// - `"summarize_trim"` — summarize the messages that would be dropped with
    ///   a lightweight LLM call, replace them with a single synthetic summary
    ///   message, then keep the most recent
    ///   `cron_session_compaction_keep_recent` messages verbatim. Falls back to
    ///   `prune` (with a `tracing::warn!`) when the LLM call fails.
    ///
    /// Only takes effect when at least one size cap (`cron_session_max_tokens`
    /// or `cron_session_max_messages`) is configured. The field is ignored and
    /// session-mode-`new` jobs always skip this path entirely.
    #[serde(default)]
    pub cron_session_compaction_mode: CronCompactionMode,
    /// Number of recent messages to preserve verbatim after summarization when
    /// `cron_session_compaction_mode = "summarize_trim"` is active.
    ///
    /// The LLM summary replaces everything older than the kept tail.
    ///
    /// **Reasonable range**: `1` – `64`. Values below `1` are clamped to `1` at
    /// runtime. Values larger than the current session length are silently
    /// clamped to the session length (nothing gets summarized in that case and
    /// the code falls back to plain prune). Setting this to a very large number
    /// defeats the purpose of summarization. Default: `8`.
    #[serde(default = "default_cron_session_compaction_keep_recent")]
    pub cron_session_compaction_keep_recent: usize,
    /// Config include files — loaded and deep-merged before the root config.
    /// Paths are relative to the root config file's directory.
    /// Security: absolute paths and `..` components are rejected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    /// Shell/exec security policy.
    #[serde(default)]
    pub exec_policy: ExecPolicy,
    /// Agent bindings for multi-account routing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AgentBinding>,
    /// Broadcast routing configuration.
    #[serde(default)]
    pub broadcast: BroadcastConfig,
    /// Auto-reply background engine configuration.
    #[serde(default)]
    pub auto_reply: AutoReplyConfig,
    /// Canvas (A2UI) configuration.
    #[serde(default)]
    pub canvas: CanvasConfig,
    /// Text-to-speech configuration.
    #[serde(default)]
    pub tts: TtsConfig,
    /// Docker container sandbox configuration.
    #[serde(default)]
    pub docker: DockerSandboxConfig,
    /// Pluggable tool-execution backend selection (#3332).
    /// Default: `local` — keeps the long-standing subprocess-on-daemon
    /// behavior. Set to `ssh` / `daytona` (with the matching subtable)
    /// to route tool exec to a remote / managed host. Per-agent
    /// override available via `agent.toml: tool_exec_backend`.
    #[serde(default)]
    pub tool_exec: crate::tool_exec::ToolExecConfig,
    /// Device pairing configuration.
    #[serde(default)]
    pub pairing: PairingConfig,
    /// Auth profiles for key rotation (provider name → profiles).
    /// Uses `BTreeMap` for deterministic serialisation order (avoids prompt-cache
    /// invalidation when the same providers are configured across restarts; see #3757).
    #[serde(default)]
    pub auth_profiles: BTreeMap<String, Vec<AuthProfile>>,
    /// Extended thinking configuration.
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    /// Global spending budget configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Provider base URL overrides (provider ID → custom base URL).
    /// e.g. `ollama = "http://192.168.1.100:11434/v1"`
    /// Uses `BTreeMap` for deterministic serialisation order (see #3757).
    #[serde(default)]
    pub provider_urls: BTreeMap<String, String>,
    /// Per-provider proxy URL overrides (provider ID → proxy URL).
    /// Allows routing specific providers through a proxy while others connect directly.
    /// e.g. `openai = "http://proxy.corp:8080"`, `ollama = ""` (direct)
    /// Uses `BTreeMap` for deterministic serialisation order (see #3757).
    #[serde(default)]
    pub provider_proxy_urls: BTreeMap<String, String>,
    /// Per-provider HTTP request timeout overrides in seconds (provider ID → seconds).
    ///
    /// Overrides the HTTP client's default read timeout for LLM API requests to the
    /// specified provider. Useful for slower providers or long-context workloads.
    /// e.g. `ollama = 300`, `anthropic = 120`
    ///
    /// Only applies to HTTP API drivers (OpenAI-compatible, Anthropic, Gemini, etc.).
    /// CLI-based providers (claude-code, qwen-code, etc.) use `message_timeout_secs`.
    /// Uses `BTreeMap` for deterministic serialisation order (see #3757).
    #[serde(default)]
    pub provider_request_timeout_secs: BTreeMap<String, u64>,
    /// Provider region selection (provider ID → region name).
    /// Selects a regional endpoint from the provider's `[provider.regions]` map.
    /// e.g. `qwen = "us"` to use the US endpoint instead of China mainland.
    /// Uses `BTreeMap` for deterministic serialisation order (see #3757).
    #[serde(default)]
    pub provider_regions: BTreeMap<String, String>,
    /// Provider API key env var overrides (provider ID → env var name).
    /// For custom/unknown providers, maps the provider name to the environment
    /// variable holding the API key. e.g. `nvidia = "NVIDIA_API_KEY"`.
    /// If not set, the convention `{PROVIDER_UPPER}_API_KEY` is used automatically.
    /// Uses `BTreeMap` for deterministic serialisation order (see #3757).
    #[serde(default)]
    pub provider_api_keys: BTreeMap<String, String>,
    /// Interval in seconds between reachability probes of local providers
    /// (Ollama, vLLM, LM Studio, lemonade).
    ///
    /// Lower values make the dashboard react faster to `brew services
    /// start/stop ollama` at the cost of extra HTTP calls to `/api/tags`.
    /// 60 s is the default — it keeps the UI responsive without noticeably
    /// loading a local Ollama daemon. Dev machines flipping Ollama on/off
    /// frequently can drop to 10; long-lived production boxes can raise to
    /// 300+ since the state rarely changes.
    ///
    /// Zero or values below the probe timeout (2 s) are treated as 60.
    #[serde(default = "default_local_probe_interval_secs")]
    pub local_probe_interval_secs: u64,
    /// Vertex AI provider configuration.
    #[serde(default)]
    pub vertex_ai: VertexAiConfig,
    /// Azure OpenAI provider configuration.
    #[serde(default)]
    pub azure_openai: AzureOpenAiConfig,
    /// OAuth client ID overrides for PKCE flows.
    #[serde(default)]
    pub oauth: OAuthConfig,
    /// Sidecar channel adapters (external process-based).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecar_channels: Vec<SidecarChannelConfig>,
    /// HTTP proxy configuration for all outbound connections.
    #[serde(default)]
    pub proxy: ProxyConfig,
    /// Enable LLM provider prompt caching (default: true).
    ///
    /// When enabled, the runtime adds provider-specific cache hints to system
    /// prompts and tool definitions so that repeated prefixes are cached:
    /// - **Anthropic**: `cache_control: {"type": "ephemeral"}` on system blocks.
    /// - **OpenAI**: automatic prefix caching (response cache stats are parsed).
    #[serde(default = "default_prompt_caching")]
    pub prompt_caching: bool,
    /// Session retention policy (automatic cleanup of old/excess sessions).
    #[serde(default)]
    pub session: SessionConfig,
    /// Session compaction configuration (LLM-based history summarization).
    #[serde(default)]
    pub compaction: CompactionTomlConfig,
    /// Message queue configuration (depth limits, TTL, concurrency).
    #[serde(default)]
    pub queue: QueueConfig,
    /// Task-board (shared task queue) safety knobs — see [`TaskBoardConfig`].
    #[serde(default)]
    pub task_board: TaskBoardConfig,
    /// External authentication provider configuration (OAuth2/OIDC).
    #[serde(default)]
    pub external_auth: ExternalAuthConfig,
    /// Tool policy configuration (global deny/allow rules, groups, depth limits).
    #[serde(default)]
    pub tool_policy: crate::tool_policy::ToolPolicy,
    /// Proactive memory (mem0-style) configuration.
    #[serde(default)]
    pub proactive_memory: crate::memory::ProactiveMemoryConfig,
    /// Auto-dream (background memory consolidation) configuration.
    #[serde(default)]
    pub auto_dream: AutoDreamConfig,
    /// Pluggable context engine configuration.
    #[serde(default)]
    pub context_engine: ContextEngineTomlConfig,
    /// Audit log configuration.
    #[serde(default)]
    pub audit: AuditConfig,
    /// Health check configuration.
    #[serde(default)]
    pub health_check: HealthCheckConfig,
    /// Heartbeat monitor configuration (global defaults for autonomous agents).
    #[serde(default)]
    pub heartbeat: HeartbeatTomlConfig,
    /// Plugin registry configuration.
    #[serde(default)]
    pub plugins: PluginsConfig,
    /// Registry sync configuration (cache TTL, etc.).
    #[serde(default)]
    pub registry: RegistryConfig,
    /// PII privacy controls for LLM context filtering.
    #[serde(default)]
    pub privacy: PrivacyConfig,
    /// Strict config mode: when `true`, the daemon refuses to start if the
    /// config file contains unknown or unrecognised fields. When `false`
    /// (the default), unknown fields are logged as warnings but the daemon
    /// boots normally. This is the "tolerant mode" toggle.
    #[serde(default)]
    pub strict_config: bool,
    /// Override path to the Qwen Code CLI binary.
    ///
    /// When LibreFang runs as a daemon/service the subprocess may not inherit
    /// the user's full PATH, so the `qwen` binary is not found even though it
    /// is installed.  Set this to the absolute path of the CLI
    /// (e.g. `"/home/user/.local/bin/qwen"`).
    ///
    /// Alternatively you can set `provider_urls.qwen-code` to the same value.
    #[serde(default)]
    pub qwen_code_path: Option<String>,
    /// Input sanitization / prompt-injection detection for channel messages.
    #[serde(default)]
    pub sanitize: SanitizeConfig,
    /// File-based input inbox configuration.
    /// Drop text files into a directory and they are dispatched to agents.
    #[serde(default)]
    pub inbox: InboxConfig,
    /// Telemetry / observability configuration (OpenTelemetry + Prometheus).
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    /// Prompt intelligence configuration (versioning + A/B testing).
    #[serde(default)]
    pub prompt_intelligence: PromptIntelligenceConfig,
    /// CLI update channel (stable, beta, rc).
    /// Controls which releases `librefang update` considers.
    #[serde(default)]
    pub update_channel: UpdateChannel,
    /// API and WebSocket rate limiting configuration.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// Timeout for individual tool executions in seconds.
    /// Increase for browser automation or long-running builds.
    #[serde(default = "default_tool_timeout_secs")]
    pub tool_timeout_secs: u64,
    /// Per-tool timeout overrides. Exact key matches take priority over glob
    /// patterns; among globs, the longest matching pattern wins (most specific
    /// first). Falls back to `tool_timeout_secs` when no entry matches.
    ///
    /// Example:
    /// ```toml
    /// [tool_timeouts]
    /// agent_send    = 600
    /// agent_spawn   = 600
    /// "mcp_browser_*" = 900
    /// shell_exec    = 300
    /// ```
    #[serde(default)]
    pub tool_timeouts: std::collections::BTreeMap<String, u64>,
    /// Maximum upload size in bytes (default: 10 MB).
    /// Enterprise deployments may need larger file uploads.
    #[serde(default = "default_max_upload_size_bytes")]
    pub max_upload_size_bytes: usize,
    /// Maximum number of concurrent background LLM calls across all agents.
    /// Increase on high-core servers that can handle more parallel inference.
    #[serde(default = "default_max_concurrent_bg_llm")]
    pub max_concurrent_bg_llm: usize,
    /// Maximum inter-agent call depth to prevent infinite recursion (A->B->C->...).
    /// Complex workflows may need deeper agent chains.
    #[serde(default = "default_max_agent_call_depth")]
    pub max_agent_call_depth: u32,
    /// Maximum request body size in bytes (global safety net).
    /// Individual endpoints may enforce tighter limits.
    #[serde(default = "default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
    /// Terminal / CLI access control configuration.
    #[serde(default)]
    pub terminal: TerminalConfig,
    /// Universal Agent Runtime (UAR) integration configuration.
    ///
    /// When `None` (the default) the UAR driver is not active; agents use the
    /// provider configured in `default_model`.  Set `[uar]` in `config.toml`
    /// to route requests through the UAR's unified `provider/model` addressing
    /// (142+ providers via liter-llm), enable the embedded SurrealDB memory
    /// layer, and unlock cross-framework A2A discovery.
    ///
    /// ```toml
    /// [uar]
    /// # API key for the UAR LLM backend (overrides UAR_LLM__API_KEY env var).
    /// # api_key = "sk-..."
    ///
    /// # Model in `provider/model` format.  Defaults to UAR_LLM__MODEL env var.
    /// # model = "openai/gpt-4o"
    ///
    /// # Path for the embedded SurrealDB data directory.
    /// # Defaults to `<data_dir>/uar-surreal`.
    /// # surreal_data_dir = "/var/lib/librefang/uar-surreal"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uar: Option<UarConfig>,

    /// Storage backend configuration (Phase 4 of `surrealdb-storage-swap`).
    ///
    /// Defaults to embedded SurrealDB at `<data_dir>/librefang.surreal`. When
    /// the user promotes to a remote SurrealDB through the wizard or
    /// `librefang storage link-uar`, this struct holds the connection
    /// parameters and credential references.
    #[serde(default)]
    pub storage: StorageConfig,

    /// Direct tool-invocation endpoint allowlist. Fail-closed: the
    /// `POST /api/tools/{name}/invoke` route rejects every request unless
    /// `tool_invoke.enabled` is `true` and the tool name matches a pattern
    /// in `tool_invoke.allowlist`.
    #[serde(default)]
    pub tool_invoke: ToolInvokeConfig,
    /// Parallel-tool dispatcher configuration. PR-3 schema only — runtime
    /// integration lands in PR-4. See [`ParallelToolsConfig`].
    #[serde(default)]
    pub parallel_tools: ParallelToolsConfig,
    /// Tool-result context budget and artifact spill configuration.
    /// See [`ToolResultsConfig`] for knob descriptions.  The primary active
    /// mechanism is artifact spill (responses > `spill_threshold_bytes` are
    /// written to disk and replaced with a stub + `read_artifact` handle).
    /// The cumulative budget and history fold knobs are wired but deferred
    /// — see #3347 2/N and 3/N.
    #[serde(default)]
    pub tool_results: ToolResultsConfig,
    /// How long (in minutes) a workflow run may remain in the `Running` or
    /// `Pending` state before it is considered stale after a daemon restart.
    ///
    /// On boot the engine scans all persisted runs and marks any that are
    /// older than this threshold as `Failed` with the error
    /// `"Interrupted by daemon restart"`.  Set to `0` to disable recovery.
    /// Default: `60` minutes.
    #[serde(default = "default_workflow_stale_timeout_minutes")]
    pub workflow_stale_timeout_minutes: u64,
    /// Default wall-clock timeout (seconds) for an entire workflow run.
    ///
    /// Individual workflows can override this via `Workflow::total_timeout_secs`.
    /// When both are `None` the workflow runs unbounded (no total timeout).
    /// Default: `None` (unbounded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_default_total_timeout_secs: Option<u64>,
}

/// Input sanitization mode for channel messages.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SanitizeMode {
    /// No checking — all messages pass through. Set `mode = "off"` in
    /// `[sanitize]` to opt out of prompt-injection detection.
    Off,
    /// Log a warning but allow the message through.
    Warn,
    /// Reject the message and send an error to the user (default).
    #[default]
    Block,
}

/// Configuration for channel input sanitization / prompt-injection detection.
///
/// The sanitizer is **enabled by default** (mode = "block"). To opt out set
/// `disable_input_sanitizer = true` in `[sanitize]` or change `mode`:
///
/// ```toml
/// [sanitize]
/// mode = "block"          # off | warn | block  (default: block)
/// max_message_length = 32768
/// custom_block_patterns = ["(?i)secret\\s+code"]
/// # disable_input_sanitizer = true  # emergency opt-out
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SanitizeConfig {
    /// Sanitization mode.
    pub mode: SanitizeMode,
    /// Maximum allowed message length in bytes (default: 32 768).
    pub max_message_length: usize,
    /// Additional regex patterns that should trigger a block/warn.
    pub custom_block_patterns: Vec<String>,
    /// Emergency opt-out: set to `true` to disable all input sanitization.
    /// Not recommended for production — prefer `mode = "warn"` for monitoring
    /// without blocking, or `mode = "off"` for a softer disable.
    #[serde(default)]
    pub disable_input_sanitizer: bool,
}

impl Default for SanitizeConfig {
    fn default() -> Self {
        Self {
            mode: SanitizeMode::Block,
            max_message_length: 32768,
            custom_block_patterns: Vec::new(),
            disable_input_sanitizer: false,
        }
    }
}

/// Azure OpenAI provider configuration.
///
/// Azure OpenAI uses a different URL format and authentication header
/// than standard OpenAI. Configure in config.toml:
/// ```toml
/// [azure_openai]
/// endpoint = "https://my-resource.openai.azure.com"
/// deployment = "gpt-4o"
/// api_version = "2024-02-01"
/// ```
///
/// Environment variable fallbacks:
/// - `AZURE_OPENAI_ENDPOINT` for the resource URL
/// - `AZURE_OPENAI_API_VERSION` for the API version (default: "2024-02-01")
/// - `AZURE_OPENAI_DEPLOYMENT` for the deployment name
/// - `AZURE_OPENAI_API_KEY` for the API key
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AzureOpenAiConfig {
    /// Azure resource endpoint URL (e.g., "https://my-resource.openai.azure.com").
    /// Falls back to `AZURE_OPENAI_ENDPOINT` env var.
    pub endpoint: Option<String>,
    /// Azure OpenAI API version (default: "2024-02-01").
    /// Falls back to `AZURE_OPENAI_API_VERSION` env var.
    pub api_version: Option<String>,
    /// Azure deployment name (e.g., "gpt-4o").
    /// Falls back to `AZURE_OPENAI_DEPLOYMENT` env var.
    /// If not set, the model name from `default_model.model` is used.
    pub deployment: Option<String>,
}

/// Vertex AI provider configuration.
///
/// Configure in config.toml:
/// ```toml
/// [vertex_ai]
/// project_id = "my-gcp-project"
/// region = "us-central1"
/// credentials_path = "/path/to/service-account.json"
/// ```
///
/// Credentials resolution order:
/// 1. `credentials_path` in config (JSON string or file path)
/// 2. `VERTEX_AI_SERVICE_ACCOUNT_JSON` env var
/// 3. `GOOGLE_APPLICATION_CREDENTIALS` env var (file path)
/// 4. `gcloud auth print-access-token` CLI fallback
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct VertexAiConfig {
    /// GCP project ID. Falls back to `VERTEX_AI_PROJECT_ID`,
    /// `GOOGLE_CLOUD_PROJECT`, or the `project_id` field in the service account JSON.
    pub project_id: Option<String>,
    /// GCP region for the Vertex AI endpoint (default: "us-central1").
    /// Falls back to `VERTEX_AI_REGION` or `GOOGLE_CLOUD_REGION` env var.
    pub region: Option<String>,
    /// Path to a GCP service account JSON key file, or the raw JSON string.
    /// Falls back to `VERTEX_AI_SERVICE_ACCOUNT_JSON` or
    /// `GOOGLE_APPLICATION_CREDENTIALS` env var.
    pub credentials_path: Option<String>,
}

/// External authentication provider configuration (OAuth2/OIDC).
///
/// Allows delegating user authentication to an external identity provider
/// (Okta, Auth0, Keycloak, Google, GitHub, Microsoft, etc.).
///
/// Single provider (backward-compatible):
/// ```toml
/// [external_auth]
/// enabled = true
/// issuer_url = "https://accounts.google.com"
/// client_id = "your-client-id.apps.googleusercontent.com"
/// client_secret_env = "LIBREFANG_OAUTH_CLIENT_SECRET"
/// redirect_url = "http://127.0.0.1:4545/api/auth/callback"
/// scopes = ["openid", "profile", "email"]
/// ```
///
/// Multiple providers:
/// ```toml
/// [external_auth]
/// enabled = true
///
/// [[external_auth.providers]]
/// id = "google"
/// display_name = "Google"
/// issuer_url = "https://accounts.google.com"
/// client_id = "your-google-client-id"
/// client_secret_env = "GOOGLE_OAUTH_CLIENT_SECRET"
///
/// [[external_auth.providers]]
/// id = "github"
/// display_name = "GitHub"
/// issuer_url = "https://token.actions.githubusercontent.com"
/// auth_url = "https://github.com/login/oauth/authorize"
/// token_url = "https://github.com/login/oauth/access_token"
/// userinfo_url = "https://api.github.com/user"
/// client_id = "your-github-client-id"
/// Pluggable context engine configuration.
///
/// Configure in config.toml:
/// ```toml
/// [context_engine]
/// engine = "default"     # built-in engine: "default"
///
/// [context_engine.hooks]
/// ingest = "~/.librefang/plugins/my_recall.py"
/// after_turn = "~/.librefang/plugins/my_indexer.py"
/// ```
///
/// Heavy hooks (`assemble`, `compact`) always run in Rust for performance.
/// Light hooks (`ingest`, `after_turn`) can be overridden with Python scripts
/// using the same JSON stdin/stdout protocol as Python agents.
///
/// # Usage
///
/// **Simple (plugin-based):**
/// ```toml
/// [context_engine]
/// plugin = "qdrant-recall"   # resolves to ~/.librefang/plugins/qdrant-recall/
/// ```
///
/// **Manual (direct hook paths):**
/// ```toml
/// [context_engine.hooks]
/// ingest = "~/.librefang/scripts/my_recall.py"
/// after_turn = "~/.librefang/scripts/my_indexer.py"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ContextEngineTomlConfig {
    /// Built-in engine name. Supported values:
    /// - `"default"`: plain [`DefaultContextEngine`] with no additional wrapping
    /// - `"summary"`: [`SummaryContextEngine`] — threshold-gated LLM summarisation
    ///   that fires when prompt tokens cross ~80 % of the context window
    /// - `"no_compact"`: [`NoCompactContextEngine`] — disables automatic compaction
    ///   while wiring all other lifecycle hooks
    ///
    /// Default: `"default"`.
    pub engine: String,
    /// Plugin name. Resolves to `~/.librefang/plugins/<name>/plugin.toml`.
    /// Takes precedence over manual `hooks` if set.
    pub plugin: Option<String>,
    /// Stack multiple plugins on a single context engine.
    ///
    /// When 2 or more plugin names are listed the runtime builds a
    /// [`StackedContextEngine`] that chains them in declaration order.
    /// Ignored when fewer than 2 entries are present; use `plugin` for the
    /// single-plugin case instead.
    ///
    /// Example:
    /// ```toml
    /// [context_engine]
    /// plugin_stack = ["qdrant-recall", "my-indexer"]
    /// ```
    #[serde(default)]
    pub plugin_stack: Option<Vec<String>>,
    /// Priority weight for each layer in `plugin_stack` (default 1.0).
    ///
    /// Higher weights cause that layer's recalled memories to appear first in
    /// the merged ingest result.  Values are matched by position — the first
    /// weight applies to the first entry in `plugin_stack`, and so on.
    /// Missing trailing weights default to `1.0`.
    ///
    /// Example:
    /// ```toml
    /// [context_engine]
    /// plugin_stack = ["qdrant-recall", "my-indexer"]
    /// plugin_stack_weights = [2.0, 1.0]   # qdrant-recall has higher priority
    /// ```
    #[serde(default)]
    pub plugin_stack_weights: Vec<f32>,
    /// Optional Python script hooks that override specific lifecycle methods.
    pub hooks: ContextEngineHooks,
    /// Plugin registries (GitHub repos) to browse for installable plugins.
    /// Defaults to the official `librefang/librefang-registry`.
    #[serde(default = "default_plugin_registries")]
    pub plugin_registries: Vec<PluginRegistrySource>,
}

impl Default for ContextEngineTomlConfig {
    fn default() -> Self {
        Self {
            engine: "default".to_string(),
            plugin: None,
            plugin_stack: None,
            plugin_stack_weights: Vec::new(),
            hooks: ContextEngineHooks::default(),
            plugin_registries: default_plugin_registries(),
        }
    }
}

/// A plugin registry source — a GitHub `owner/repo` with a `plugins/` directory.
///
/// ```toml
/// [[context_engine.plugin_registries]]
/// name = "Official"
/// github_repo = "librefang/librefang-registry"
///
/// [[context_engine.plugin_registries]]
/// name = "My Company"
/// github_repo = "acme-corp/librefang-plugins"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PluginRegistrySource {
    /// Human-readable label shown in the dashboard.
    pub name: String,
    /// GitHub `owner/repo` (e.g. `"librefang/librefang-registry"`).
    pub github_repo: String,
}

/// Default: official registry only.
fn default_plugin_registries() -> Vec<PluginRegistrySource> {
    vec![PluginRegistrySource {
        name: "Official".to_string(),
        github_repo: "librefang/librefang-registry".to_string(),
    }]
}

/// Script overrides for individual context engine lifecycle hooks.
///
/// Hook scripts speak a language-agnostic JSON-over-stdin/stdout protocol —
/// they read one JSON object from stdin and emit one JSON line on stdout.
/// The `runtime` field picks which interpreter / launcher to use; it defaults
/// to `"python"` so existing Python plugins keep working without edits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ContextEngineHooks {
    /// Script for the `ingest` hook (called on new user message).
    /// Receives: `{"type": "ingest", "agent_id": "...", "message": "..."}`
    /// Returns: `{"type": "ingest_result", "memories": [{"content": "..."}]}`
    pub ingest: Option<String>,
    /// Script for the `after_turn` hook (called after each turn).
    /// Receives: `{"type": "after_turn", "agent_id": "...", "messages": [...]}`
    /// Returns: `{"type": "ok"}` (acknowledgement)
    pub after_turn: Option<String>,
    /// Script for the `bootstrap` hook (called once on engine init).
    /// Receives: `{"type": "bootstrap", "context_window_tokens": N, "stable_prefix_mode": bool, "max_recall_results": N}`
    /// Returns: `{"type": "ok"}`
    pub bootstrap: Option<String>,
    /// Script for the `assemble` hook (called before each LLM call).
    /// Receives: `{"type": "assemble", "agent_id": "...", "messages": [...], "system_prompt": "...", "context_window_tokens": N}`
    /// Returns: `{"type": "assemble_result", "messages": [...]}` — script controls what the model sees.
    /// Falls back to default engine if script fails or returns no messages.
    pub assemble: Option<String>,
    /// Script for the `compact` hook (called under context pressure).
    /// Receives: `{"type": "compact", "agent_id": "...", "messages": [...], "model": "...", "context_window_tokens": N}`
    /// Returns: `{"type": "compact_result", "messages": [...]}` — compacted message list.
    /// Falls back to default LLM-based compaction if script fails.
    pub compact: Option<String>,
    /// Script for the `prepare_subagent` hook (called before sub-agent spawn).
    /// Receives: `{"type": "prepare_subagent", "parent_id": "...", "child_id": "..."}`
    /// Returns: `{"type": "ok"}`
    pub prepare_subagent: Option<String>,
    /// Script for the `merge_subagent` hook (called after sub-agent completes).
    /// Receives: `{"type": "merge_subagent", "parent_id": "...", "child_id": "..."}`
    /// Returns: `{"type": "ok"}`
    pub merge_subagent: Option<String>,
    /// Which runtime launches the hook scripts.
    ///
    /// Supported: `"python"` (default, runs `.py` via `python3`), `"native"`
    /// (exec a pre-compiled binary directly), `"v"` (`v run *.v`), `"node"`,
    /// `"deno"`, `"go"` (`go run *.go`). Unknown values fall back to
    /// `"python"` with a warning.
    pub runtime: Option<String>,
    /// Per-invocation timeout for hook scripts, in seconds.
    ///
    /// Defaults to `30`. The `bootstrap` hook gets **double** this value because
    /// it runs only once and may need time to connect to external services (e.g.
    /// a vector database). Set higher if your hooks do heavy I/O at startup.
    #[serde(default)]
    pub hook_timeout_secs: Option<u64>,
    /// What to do when a hook script fails (crash, timeout, bad JSON).
    ///
    /// - `"warn"` (default) — log a warning, continue with fallback behaviour.
    /// - `"abort"` — propagate the error to the caller; the agent turn fails.
    /// - `"skip"` — silently ignore the failure, no log, use fallback.
    #[serde(default)]
    pub on_hook_failure: HookFailurePolicy,
    /// How many times to retry a failing hook before applying `on_hook_failure`.
    ///
    /// Defaults to `0` (no retries). Each retry respects the same timeout.
    /// Useful for hooks that call flaky external services.
    #[serde(default)]
    pub max_retries: u32,
    /// Milliseconds to wait between hook retries.
    ///
    /// Defaults to `500`. Ignored when `max_retries = 0`.
    #[serde(default = "default_hook_retry_delay_ms")]
    pub retry_delay_ms: u64,
    /// Optional substring filter for the `ingest` hook.
    ///
    /// When set, the ingest hook is only invoked if the incoming user message
    /// contains this string (case-sensitive). If the message does not match,
    /// the default recall path runs without starting a subprocess.
    ///
    /// Example: `ingest_filter = "remember"` — only index messages that
    /// explicitly ask the agent to remember something.
    #[serde(default)]
    pub ingest_filter: Option<String>,
    /// Hook protocol version this plugin was written for.
    ///
    /// LibreFang's current hook protocol is version **1**. If a plugin declares
    /// a higher version the runtime logs a compatibility warning and may refuse
    /// to load. Omit or set to `1` for full compatibility.
    #[serde(default)]
    pub hook_protocol_version: Option<u32>,
    /// Memory limit (MiB) for each hook subprocess.
    ///
    /// Enforced via `RLIMIT_AS` on Linux. On other platforms a warning is
    /// logged and the limit is not applied. Omit to use the OS default.
    #[serde(default)]
    pub max_memory_mb: Option<u64>,
    /// Whether hook subprocesses are allowed to make network connections.
    ///
    /// When `false` the runtime attempts soft network isolation: on Linux it
    /// wraps the hook with `unshare --net` (if available); on other platforms
    /// it injects `no_proxy=*` / `NO_PROXY=*` into the subprocess environment.
    /// Defaults to `true`.
    #[serde(default = "default_true_bool")]
    pub allow_network: bool,
    /// Restrict the `ingest`/`after_turn`/`assemble` hooks to specific agent IDs.
    ///
    /// Each entry is matched as a substring of the agent's UUID string. Leave
    /// empty (default) to run hooks for every agent.
    ///
    /// ```toml
    /// only_for_agent_ids = ["3f2a", "9c01"]  # prefix match is fine
    /// ```
    #[serde(default)]
    pub only_for_agent_ids: Vec<String>,
    /// Per-hook JSON Schema definitions for input/output validation.
    ///
    /// Map keys are hook names (`"ingest"`, `"assemble"`, …). Each value is
    /// an object with optional `"input"` and `"output"` JSON Schema objects.
    /// When declared, the runtime validates hook payloads and responses against
    /// the schema and logs a warning on mismatch (never blocks execution).
    ///
    /// ```toml
    /// [hooks.hook_schemas.ingest.output]
    /// type = "object"
    /// required = ["memories"]
    /// ```
    #[serde(default)]
    pub hook_schemas: std::collections::HashMap<String, HookSchema>,
    /// Optional TTL (seconds) for caching `ingest` hook results.
    ///
    /// When set, the runtime caches the hook output keyed on the exact input
    /// JSON. Subsequent calls with identical input within the TTL window skip
    /// the subprocess entirely and return the cached result. Useful for
    /// embedding-based recall hooks that are deterministic and expensive.
    ///
    /// Set to `0` or omit to disable caching (default).
    ///
    /// ```toml
    /// hook_cache_ttl_secs = 60   # cache ingest results for 1 minute
    /// ```
    #[serde(default)]
    pub hook_cache_ttl_secs: Option<u64>,
    /// Keep hook subprocesses alive between calls (persistent process pool).
    ///
    /// When `true`, the runtime keeps one subprocess per hook script alive
    /// between invocations, communicating via JSON-lines on stdin/stdout.
    /// Eliminates interpreter startup overhead (significant for Python/Node).
    /// Defaults to `false`.
    ///
    /// ```toml
    /// persistent_subprocess = true
    /// ```
    #[serde(default)]
    pub persistent_subprocess: bool,
    /// Cache TTL (seconds) for `assemble` hook results.
    ///
    /// When set, identical assemble inputs (same messages + system_prompt) return
    /// the cached output without invoking the subprocess. Useful for expensive
    /// context-shaping hooks that produce deterministic output.
    #[serde(default)]
    pub assemble_cache_ttl_secs: Option<u64>,
    /// Cache TTL (seconds) for `compact` hook results.
    #[serde(default)]
    pub compact_cache_ttl_secs: Option<u64>,
    /// Execution priority in a stacked engine (higher = runs first).
    ///
    /// Plugins with higher priority run first for `ingest` and `assemble`
    /// hooks. Plugins with equal priority keep declaration order.
    /// Defaults to `0`.
    ///
    /// ```toml
    /// priority = 10   # run before plugins with default priority 0
    /// ```
    #[serde(default)]
    pub priority: i32,
    /// Regex filter for the `ingest` hook (applied before `ingest_filter`).
    ///
    /// The hook is only invoked when the user message matches this regex.
    /// ```toml
    /// ingest_regex = "(?i)remember|note|save"
    /// ```
    #[serde(default)]
    pub ingest_regex: Option<String>,
    /// Declared environment variable schema for this plugin.
    ///
    /// Maps env var name → description. Keys prefixed with `!` are required;
    /// the runtime warns at load time if a required var is not set.
    ///
    /// ```toml
    /// [hooks.env_schema]
    /// "!QDRANT_URL" = "Required: Qdrant HTTP endpoint"
    /// "COLLECTION"  = "Optional: collection name (default: memories)"
    /// ```
    #[serde(default)]
    pub env_schema: std::collections::HashMap<String, String>,
    /// Enable shared state KV store for this plugin's hooks.
    ///
    /// When enabled, the runtime injects `LIBREFANG_STATE_FILE=/path/to/state.json`
    /// into every hook subprocess. Hooks can read/write this JSON file to persist
    /// state across calls. The file is scoped per-plugin.
    ///
    /// ```toml
    /// enable_shared_state = true
    /// ```
    #[serde(default)]
    pub enable_shared_state: bool,
    /// Circuit-breaker configuration for hook failures.
    ///
    /// After `max_failures` consecutive failures the hook is suspended for
    /// `reset_secs` seconds before being retried in half-open state.
    ///
    /// ```toml
    /// [hooks.circuit_breaker]
    /// max_failures = 5
    /// reset_secs   = 60
    /// ```
    #[serde(default)]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    /// Maximum concurrent `after_turn` background tasks (default 16).
    #[serde(default = "default_after_turn_queue_depth")]
    pub after_turn_queue_depth: u32,
    /// Pre-warm persistent subprocesses at engine init (requires `persistent_subprocess = true`).
    #[serde(default)]
    pub prewarm_subprocesses: bool,
    /// Restrict hook filesystem access: sets `HOME=/dev/null`, per-call `TMPDIR`,
    /// and `LIBREFANG_READONLY_FS=1`. Defaults to `true` (no restriction).
    #[serde(default = "default_true_bool")]
    pub allow_filesystem: bool,
    /// OTel OTLP gRPC endpoint for hook span export (overrides global setting).
    #[serde(default)]
    pub otel_endpoint: Option<String>,
    /// Script path for the `on_event` hook.
    /// Called when another plugin emits an event via the event bus.
    #[serde(default)]
    pub on_event: Option<String>,
    /// Vault secret names that this plugin's hooks are allowed to access.
    ///
    /// Each entry is a key name in the LibreFang credential vault. The runtime
    /// resolves the secret value at engine init time and injects it into every
    /// hook subprocess as `LIBREFANG_SECRET_<NAME>` (uppercased). If a named
    /// secret does not exist in the vault a warning is logged and the variable
    /// is not injected.
    ///
    /// ```toml
    /// [hooks]
    /// allowed_secrets = ["GITHUB_TOKEN", "OPENAI_KEY"]
    /// ```
    #[serde(default)]
    pub allowed_secrets: Vec<String>,
}

/// Circuit-breaker settings for a hook.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CircuitBreakerConfig {
    /// Consecutive failures before the circuit opens.
    #[serde(default = "default_cb_max_failures")]
    pub max_failures: u32,
    /// Cooldown in seconds before half-open retry.
    #[serde(default = "default_cb_reset_secs")]
    pub reset_secs: u64,
}

fn default_cb_max_failures() -> u32 {
    5
}
fn default_cb_reset_secs() -> u64 {
    60
}
fn default_after_turn_queue_depth() -> u32 {
    16
}

fn default_true_bool() -> bool {
    true
}

/// Per-hook input/output JSON Schema definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HookSchema {
    /// JSON Schema for the value sent to the hook script on stdin.
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// JSON Schema for the value the hook script must return on stdout.
    #[serde(default)]
    pub output: Option<serde_json::Value>,
}

fn default_hook_retry_delay_ms() -> u64 {
    500
}

/// What to do when a hook script invocation fails.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookFailurePolicy {
    /// Log a warning and continue with the engine's built-in fallback (default).
    #[default]
    Warn,
    /// Propagate the error to the caller — the current agent operation fails.
    Abort,
    /// Silently ignore the failure and proceed with fallback, no log emitted.
    Skip,
}

/// Plugin manifest — parsed from `~/.librefang/plugins/<name>/plugin.toml`.
///
/// Type of a plugin config field.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PluginConfigFieldType {
    #[default]
    String,
    Number,
    Boolean,
}

/// A single user-configurable field declared in `[config]` of plugin.toml.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct PluginConfigField {
    /// Field value type.
    #[serde(rename = "type", default)]
    pub field_type: PluginConfigFieldType,
    /// Default value (always a JSON-compatible value).
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// Human-readable description of what this field controls.
    #[serde(default)]
    pub description: Option<String>,
}

/// # Example `plugin.toml`
///
/// ```toml
/// name = "qdrant-recall"
/// version = "0.1.0"
/// description = "Vector recall via Qdrant"
/// author = "librefang"
///
/// [hooks]
/// ingest = "hooks/ingest.py"      # relative to plugin dir
/// after_turn = "hooks/after_turn.py"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PluginManifest {
    /// Plugin name (must match directory name).
    pub name: String,
    /// Semver version string.
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Plugin author.
    #[serde(default)]
    pub author: Option<String>,
    /// Hook script paths, relative to the plugin directory.
    #[serde(default)]
    pub hooks: ContextEngineHooks,
    /// Dependencies file (relative to plugin dir). For Python: `requirements.txt`.
    /// Other runtimes ignore this field (use `go.mod`, `package.json`, etc. directly).
    #[serde(default)]
    pub requirements: Option<String>,
    /// Environment variables injected into every hook subprocess spawned by this plugin.
    ///
    /// Values starting with `${VAR_NAME}` are expanded from the daemon's own environment
    /// at invocation time. Unknown references expand to an empty string.
    ///
    /// ```toml
    /// [env]
    /// QDRANT_URL     = "http://localhost:6333"
    /// COLLECTION     = "agent-memories"
    /// QDRANT_API_KEY = "${QDRANT_API_KEY}"   # expanded from daemon env
    /// ```
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Minimum LibreFang version required by this plugin.
    ///
    /// The runtime refuses to load the plugin when the running daemon's version
    /// is lower than this string (compared lexicographically on the semver
    /// portion before any `-` pre-release suffix). Omit to allow all versions.
    ///
    /// ```toml
    /// librefang_min_version = "2026.4.0"
    /// ```
    #[serde(default)]
    pub librefang_min_version: Option<String>,
    /// SHA-256 integrity hashes for hook script files.
    ///
    /// Maps a file path (relative to the plugin directory) to its expected
    /// lowercase hex SHA-256 digest. Verified at load time; mismatches abort
    /// loading with an error so tampered scripts are never executed.
    ///
    /// Generate with: `sha256sum hooks/ingest.py`
    ///
    /// ```toml
    /// [integrity]
    /// "hooks/ingest.py"    = "e3b0c44298fc1c149afb..."
    /// "hooks/after_turn.py" = "a87ff679a2f3e71d9181..."
    /// ```
    #[serde(default)]
    pub integrity: std::collections::HashMap<String, String>,
    /// Other plugins this plugin depends on.
    ///
    /// Listed names must be installed (present in `~/.librefang/plugins/`)
    /// before this plugin is allowed to load. The runtime returns an error
    /// listing any missing dependencies.
    ///
    /// ```toml
    /// plugin_depends = ["base-recall", "embedding-indexer"]
    /// ```
    #[serde(default)]
    pub plugin_depends: Vec<String>,
    /// User-configurable plugin settings declared in `[config]`.
    ///
    /// ```toml
    /// [config]
    /// model = { type = "string", default = "small", description = "Whisper model size" }
    /// max_file_size_mb = { type = "number", default = 10 }
    /// ```
    ///
    /// The resolved config (defaults merged with user overrides) is written as JSON to
    /// the path in `LIBREFANG_PLUGIN_CONFIG` before each hook subprocess runs.
    #[serde(default)]
    pub config: std::collections::HashMap<String, PluginConfigField>,
    /// System binaries required by this plugin.
    ///
    /// The runtime checks each binary against `PATH` at install and lint time
    /// and warns when one is missing. Hooks still execute — this is advisory only.
    ///
    /// ```toml
    /// [[requires]]
    /// binary = "ffmpeg"
    /// install_hint = "brew install ffmpeg"
    /// ```
    #[serde(default)]
    pub requires: Vec<PluginSystemRequirement>,
    /// Per-language translation overrides for `name` and `description`.
    ///
    /// Keyed by BCP-47 language tag (`zh`, `zh-TW`, `ja`, `ko`, `de`, `es`,
    /// `fr`, …). API routes resolve `Accept-Language` against this table and
    /// fall back to the top-level English fields when no entry matches.
    ///
    /// ```toml
    /// [i18n.zh]
    /// name = "自动摘要"
    /// description = "持续维护会话摘要，帮助 Agent 在长对话中不丢失上下文。"
    /// ```
    #[serde(default)]
    pub i18n: std::collections::HashMap<String, PluginI18n>,
}

/// A per-language override for a plugin's user-facing strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PluginI18n {
    /// Localized plugin name. Falls back to the top-level `name`.
    #[serde(default)]
    pub name: Option<String>,
    /// Localized description. Falls back to the top-level `description`.
    #[serde(default)]
    pub description: Option<String>,
}

/// A single system-binary requirement declared in `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct PluginSystemRequirement {
    /// Name of the binary that must exist on `PATH`.
    pub binary: String,
    /// Human-readable install hint shown when the binary is missing.
    #[serde(default)]
    pub install_hint: Option<String>,
}

/// client_secret_env = "GITHUB_OAUTH_CLIENT_SECRET"
/// scopes = ["read:user", "user:email"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ExternalAuthConfig {
    /// Whether external auth is enabled.
    pub enabled: bool,
    /// OIDC issuer URL (e.g., `https://accounts.google.com`).
    /// Used to discover the OIDC configuration at `{issuer_url}/.well-known/openid-configuration`.
    pub issuer_url: String,
    /// OAuth2 client ID registered with the identity provider.
    pub client_id: String,
    /// Environment variable holding the OAuth2 client secret.
    /// The secret itself is never stored in config.
    #[serde(default = "default_oauth_client_secret_env")]
    pub client_secret_env: String,
    /// Redirect URL for the OAuth2 authorization code flow callback.
    /// Defaults to `http://127.0.0.1:4545/api/auth/callback`.
    #[serde(default = "default_redirect_url")]
    pub redirect_url: String,
    /// OAuth2 scopes to request.
    #[serde(default = "default_oauth_scopes")]
    pub scopes: Vec<String>,
    /// Allowed email domains for authorization (empty = allow all).
    /// e.g., `["example.com", "corp.example.com"]`
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// JWT audience claim to validate (defaults to `client_id` if empty).
    #[serde(default)]
    pub audience: String,
    /// Session token lifetime in seconds. Default: 86400 (24 hours).
    #[serde(default = "default_session_ttl")]
    pub session_ttl_secs: u64,
    /// Multiple OIDC/OAuth2 providers.
    /// When configured, these take precedence over the top-level single-provider fields.
    #[serde(default)]
    pub providers: Vec<OidcProvider>,
    /// Require `email_verified = true` in the OIDC ID token before allowing login.
    /// Defaults to `true`. Set to `false` only if your identity provider does not
    /// set this claim. When `true`, logins where the claim is absent or false are
    /// rejected — prevents `allowed_domains` impersonation via unverified addresses (#3703).
    #[serde(default = "default_true")]
    pub require_email_verified: bool,
}

/// Configuration for a single OIDC/OAuth2 provider.
///
/// Supports standard OIDC providers (Google, Azure AD, Keycloak) that use
/// `.well-known/openid-configuration` discovery, as well as non-OIDC OAuth2
/// providers (GitHub) where explicit `auth_url`, `token_url`, and `userinfo_url`
/// are specified.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OidcProvider {
    /// Unique identifier for this provider (e.g., "google", "github", "keycloak").
    pub id: String,
    /// Human-readable display name (e.g., "Google", "GitHub", "Corporate SSO").
    #[serde(default)]
    pub display_name: String,
    /// OIDC issuer URL for discovery. Leave empty for non-OIDC providers (e.g., GitHub).
    #[serde(default)]
    pub issuer_url: String,
    /// Explicit authorization endpoint (overrides OIDC discovery).
    #[serde(default)]
    pub auth_url: String,
    /// Explicit token endpoint (overrides OIDC discovery).
    #[serde(default)]
    pub token_url: String,
    /// Explicit userinfo endpoint (overrides OIDC discovery).
    #[serde(default)]
    pub userinfo_url: String,
    /// Explicit JWKS URI (overrides OIDC discovery).
    #[serde(default)]
    pub jwks_uri: String,
    /// OAuth2 client ID.
    pub client_id: String,
    /// Environment variable name holding the client secret.
    #[serde(default = "default_oauth_client_secret_env")]
    pub client_secret_env: String,
    /// OAuth2 redirect URI. Defaults to `http://127.0.0.1:4545/api/auth/callback`.
    #[serde(default = "default_redirect_url")]
    pub redirect_url: String,
    /// OAuth2 scopes to request.
    #[serde(default = "default_oauth_scopes")]
    pub scopes: Vec<String>,
    /// Allowed email domains (empty = allow all).
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// JWT audience claim to validate.
    #[serde(default)]
    pub audience: String,
    /// Override the global `require_email_verified` setting for this provider.
    /// `None` means inherit from `ExternalAuthConfig::require_email_verified`.
    /// Set to `false` only if this specific provider does not issue `email_verified`
    /// claims (e.g. GitHub's user API does not include the field for OAuth2 flows).
    #[serde(default)]
    pub require_email_verified: Option<bool>,
}

fn default_oauth_client_secret_env() -> String {
    "LIBREFANG_OAUTH_CLIENT_SECRET".to_string()
}

fn default_redirect_url() -> String {
    "http://127.0.0.1:4545/api/auth/callback".to_string()
}

fn default_oauth_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
    ]
}

fn default_session_ttl() -> u64 {
    86400
}

impl Default for ExternalAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret_env: default_oauth_client_secret_env(),
            redirect_url: default_redirect_url(),
            scopes: default_oauth_scopes(),
            allowed_domains: Vec::new(),
            audience: String::new(),
            session_ttl_secs: default_session_ttl(),
            providers: Vec::new(),
            require_email_verified: true,
        }
    }
}

/// OAuth client ID overrides for PKCE flows.
///
/// Configure in config.toml:
/// ```toml
/// [oauth]
/// google_client_id = "your-google-client-id"
/// github_client_id = "your-github-client-id"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct OAuthConfig {
    /// Google OAuth2 client ID for PKCE flow.
    pub google_client_id: Option<String>,
    /// GitHub OAuth client ID for PKCE flow.
    pub github_client_id: Option<String>,
    /// Microsoft (Entra ID) OAuth client ID.
    pub microsoft_client_id: Option<String>,
    /// Slack OAuth client ID.
    pub slack_client_id: Option<String>,
}

/// Per-provider spending limits.
///
/// Lets you cap spend on paid providers (e.g. Moonshot, OpenAI) without
/// throttling free local providers (e.g. litellm, ollama). All limits
/// default to 0 which means "unlimited" — only non-zero limits are enforced.
/// Keyed by the provider id in `BudgetConfig.providers`, which must match
/// the `model.provider` field of the agent's `ModelConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(default)]
pub struct ProviderBudget {
    /// Maximum cost in USD per hour for this provider (0.0 = unlimited).
    pub max_cost_per_hour_usd: f64,
    /// Maximum cost in USD per day for this provider (0.0 = unlimited).
    pub max_cost_per_day_usd: f64,
    /// Maximum cost in USD per month for this provider (0.0 = unlimited).
    pub max_cost_per_month_usd: f64,
    /// Maximum total tokens per hour for this provider (0 = unlimited).
    pub max_tokens_per_hour: u64,
}

/// Global spending budget configuration.
///
/// Set limits to 0.0 for unlimited. All limits apply across all agents.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BudgetConfig {
    /// Maximum total cost in USD per hour (0.0 = unlimited).
    pub max_hourly_usd: f64,
    /// Maximum total cost in USD per day (0.0 = unlimited).
    pub max_daily_usd: f64,
    /// Maximum total cost in USD per month (0.0 = unlimited).
    pub max_monthly_usd: f64,
    /// Alert threshold as a fraction (0.0 - 1.0). Trigger warnings at this % of any limit.
    pub alert_threshold: f64,
    /// Default per-agent hourly token limit override. When set (> 0), all agents
    /// will be overridden to this value. Set to 0 to keep each agent's own limit.
    /// Use this to globally raise or lower the token budget for all agents.
    pub default_max_llm_tokens_per_hour: u64,
    /// Per-provider spending caps, keyed by provider id (e.g. `"moonshot"`,
    /// `"openai"`, `"litellm"`). Missing providers are unlimited.
    ///
    /// `BTreeMap` so the serialised form is deterministic — `[budget].providers`
    /// round-trips byte-identically across reloads, so `field_changed` in
    /// `config_reload::build_reload_plan` (which compares JSON forms) does not
    /// emit a spurious `HotAction::UpdateBudget` from HashMap iteration-order
    /// drift when the operator hasn't actually touched the caps.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub providers: std::collections::BTreeMap<String, ProviderBudget>,
    /// Global default burst ratio for all agents (`0.0` = unset, use
    /// compiled default `0.2`). Overridden per-agent via
    /// `ResourceQuota.burst_ratio`.
    ///
    /// Validated at parse time: NaN, infinity, and values outside
    /// `[0.0, 1.0]` are rejected with a serde error rather than
    /// silently sanitised at the rate-limiter use site. The
    /// compiled-default fallback for an out-of-range agent override
    /// still lives in `ResourceQuota::effective_burst_ratio` (defence
    /// in depth), but operator-provided config that's nonsensical
    /// should fail loud at boot, not paper over a typo.
    #[serde(default, deserialize_with = "deserialize_default_burst_ratio")]
    pub default_burst_ratio: f32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_hourly_usd: 0.0,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
            default_max_llm_tokens_per_hour: 0,
            providers: std::collections::BTreeMap::new(),
            default_burst_ratio: 0.0,
        }
    }
}

fn default_max_cron_jobs() -> usize {
    500
}

/// Validate `BudgetConfig::default_burst_ratio` at parse time.
///
/// Rejects NaN, ±infinity, and values outside `[0.0, 1.0]`. `0.0` is
/// the explicit "unset, use compiled default" sentinel and is allowed.
fn deserialize_default_burst_ratio<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let v = f32::deserialize(deserializer)?;
    if !v.is_finite() {
        return Err(D::Error::custom(format!(
            "default_burst_ratio must be finite (got {v}); use 0.0 to leave unset"
        )));
    }
    if !(0.0..=1.0).contains(&v) {
        return Err(D::Error::custom(format!(
            "default_burst_ratio must be in [0.0, 1.0] (got {v}); 0.0 = unset, 1.0 = full hourly budget per minute"
        )));
    }
    Ok(v)
}

/// Compaction strategy for Persistent cron sessions (#3693).
///
/// Controls what happens when the session exceeds `cron_session_max_tokens`
/// or `cron_session_max_messages` before a cron fire.
///
/// Configure in config.toml:
/// ```toml
/// cron_session_compaction_mode = "summarize_trim"
/// cron_session_compaction_keep_recent = 8
/// ```
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CronCompactionMode {
    /// Drop oldest messages from the front of the session until the budget is
    /// satisfied. Fast and deterministic, but lossy — dropped context is gone.
    /// This is the historical behaviour and remains the default.
    #[default]
    Prune,
    /// Summarize messages that would be dropped using an LLM call, then keep
    /// the summary as a synthetic assistant message followed by the most recent
    /// `cron_session_compaction_keep_recent` messages. Preserves semantic
    /// continuity at the cost of a lightweight LLM round-trip per fire when the
    /// session exceeds the budget. Falls back to `Prune` on LLM failure with a
    /// `tracing::warn!`.
    SummarizeTrim,
}

/// Default number of recent messages kept verbatim when
/// `cron_session_compaction_mode = "summarize_trim"` runs (#3693).
fn default_cron_session_compaction_keep_recent() -> usize {
    8
}

/// Default warn fraction for cron session size (#3693): 80% of the
/// effective token budget.
fn default_cron_session_warn_fraction() -> Option<f64> {
    Some(0.8)
}

/// Default fallback ceiling for cron session warn (#3693): 200k tokens
/// — matches typical long-context provider windows so operators get a
/// signal even without an explicit `cron_session_max_tokens` cap.
fn default_cron_session_warn_total_tokens() -> Option<u64> {
    Some(200_000)
}

/// Default stale workflow run timeout in minutes (60 minutes = 1 hour).
fn default_workflow_stale_timeout_minutes() -> u64 {
    60
}

/// Default tool execution timeout in seconds (120s).
fn default_tool_timeout_secs() -> u64 {
    120
}

/// Default maximum upload size in bytes (10 MB).
fn default_max_upload_size_bytes() -> usize {
    10 * 1024 * 1024
}

/// Default maximum concurrent background LLM calls.
fn default_max_concurrent_bg_llm() -> usize {
    5
}

/// Default maximum inter-agent call depth.
fn default_max_agent_call_depth() -> u32 {
    5
}

/// Default maximum request body size in bytes (8 MiB).
///
/// Raised from 1 MiB so that dashboard uploads of ~1 MiB files (which incur
/// multipart-form overhead) no longer get rejected with 413. Per-route caps
/// (A2A 1 MiB, webhook 1 MiB) still bound external attack surface (#3493).
fn default_max_request_body_bytes() -> usize {
    8 * 1_024 * 1_024
}

/// Audit log configuration.
///
/// Configure in config.toml:
/// ```toml
/// [audit]
/// retention_days = 90
/// # Optional override for the external tip-anchor path. Relative
/// # paths resolve against `data_dir`. Leave unset for the default
/// # `data_dir/audit.anchor`.
/// anchor_path = "/var/log/librefang/audit.anchor"
///
/// [audit.retention]
/// trim_interval_secs = 3600
/// max_in_memory_entries = 50000
///
/// [audit.retention.retention_days_by_action]
/// ToolInvoke = 14
/// LlmCompletion = 14
/// RoleChange = 365
/// PermissionDenied = 365
/// BudgetExceeded = 365
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AuditConfig {
    /// How many days to retain audit log entries. Default: 90. Set to 0 for unlimited.
    ///
    /// **Coarse global retention.** This drives the legacy day-based prune
    /// over the SQLite table. For per-category in-memory retention with
    /// chain-anchor-preserving trim, see `retention` below.
    pub retention_days: u32,
    /// Optional override for the external Merkle-tip anchor file that
    /// `AuditLog::with_db_anchored` uses to detect full rewrites of
    /// `audit_entries`. When unset the daemon writes to
    /// `data_dir/audit.anchor`, which catches most casual tampering but
    /// sits in the same filesystem namespace as the SQLite file it is
    /// meant to verify. Operators who want a stronger boundary can
    /// point this at a path the daemon can write to but unprivileged
    /// code cannot — a chmod-0400 file owned by a dedicated user, a
    /// `systemd ReadOnlyPaths=` mount, an NFS share, or a pipe to
    /// `logger`. Relative paths are resolved against `data_dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_path: Option<PathBuf>,
    /// Per-`AuditAction` retention policy used by the periodic trim job
    /// over the in-memory audit window. Defaults preserve every entry.
    #[serde(default)]
    pub retention: AuditRetentionConfig,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            retention_days: 90,
            anchor_path: None,
            retention: AuditRetentionConfig::default(),
        }
    }
}

/// Per-`AuditAction` retention policy for the in-memory audit window.
///
/// The audit log is a Merkle-style hash chain — every entry's hash mixes
/// the previous entry's hash. Naively dropping a prefix would break
/// chain verification of the surviving entries because their `prev_hash`
/// would point at a hash no longer present. The trim implementation
/// solves this by remembering the last-dropped entry's hash as a
/// **chain anchor** so verification of the surviving prefix can validate
/// continuity against the anchor instead of a missing row.
///
/// Critical actions (`RoleChange`, `PermissionDenied`, `BudgetExceeded`)
/// should keep long retention windows; noisy actions (`ToolInvoke`) can
/// be pruned far more aggressively. Actions absent from the map are
/// kept forever so operators that don't opt in never silently lose
/// audit history.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AuditRetentionConfig {
    /// How often the trim job runs. `None` (or 0) disables periodic trimming.
    /// Reasonable default for production: 3600 (one hour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_interval_secs: Option<u64>,
    /// Per-`AuditAction` retention windows in days. Key is the
    /// `AuditAction` `Display` string (e.g. `"ToolInvoke"`). Missing
    /// entries mean "keep forever".
    #[serde(default)]
    pub retention_days_by_action: HashMap<String, u32>,
    /// Hard cap on the in-memory audit window — protects against runaway
    /// growth even when no per-action policy is configured. `None` or 0
    /// means unlimited. When the cap is exceeded the trim job drops the
    /// oldest entries down to the cap regardless of their action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_in_memory_entries: Option<usize>,
}

/// PII privacy mode for LLM context filtering.
///
/// Controls how personally identifiable information is handled before
/// messages are sent to LLM providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    /// No PII filtering — messages are sent as-is.
    #[default]
    Off,
    /// Replace detected PII with `[REDACTED]`.
    Redact,
    /// Replace detected PII with stable pseudonyms (User-A, User-B, etc.).
    /// Pseudonym mappings are stable within a session.
    Pseudonymize,
}

/// PII privacy controls for LLM context.
///
/// When enabled, the runtime filters personally identifiable information
/// (emails, phone numbers, credit card numbers, SSNs) from user messages
/// and sender context before they are sent to LLM providers.
///
/// Configure in config.toml:
/// ```toml
/// [privacy]
/// mode = "pseudonymize"  # off | redact | pseudonymize
/// redact_patterns = ["\\b(CUSTOM_ID_\\d+)\\b"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Privacy mode: off, redact, or pseudonymize.
    #[serde(default)]
    pub mode: PrivacyMode,
    /// Additional regex patterns to match and redact/pseudonymize.
    /// These are applied in addition to the built-in PII patterns.
    #[serde(default)]
    pub redact_patterns: Vec<String>,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            mode: PrivacyMode::Off,
            redact_patterns: Vec::new(),
        }
    }
}

/// Health check configuration.
///
/// Configure in config.toml:
/// ```toml
/// [health_check]
/// health_check_interval_secs = 60
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct HealthCheckConfig {
    /// Interval in seconds between periodic health checks of LLM providers. Default: 60.
    pub health_check_interval_secs: u64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            health_check_interval_secs: 60,
        }
    }
}

/// Heartbeat monitor configuration (global defaults).
///
/// Configure in config.toml:
/// ```toml
/// [heartbeat]
/// check_interval_secs = 30
/// default_timeout_secs = 60
/// keep_recent = 10
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct HeartbeatTomlConfig {
    /// How often to run the heartbeat check (seconds). Default: 30.
    pub check_interval_secs: u64,
    /// Default threshold for unresponsiveness (seconds). Default: 60.
    pub default_timeout_secs: u64,
    /// How many recent heartbeat turns to keep when pruning session context. Default: 10.
    pub keep_recent: usize,
}

impl Default for HeartbeatTomlConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 30,
            default_timeout_secs: 60,
            keep_recent: 10,
        }
    }
}

/// Auto-dream (background memory consolidation) configuration.
///
/// Global toggle and scheduling knobs for the per-agent auto-dream system.
/// Individual agents still opt in via `auto_dream_enabled = true` on their
/// manifest — this config only governs *when* the scheduler looks and what
/// thresholds apply. A dream fires for an agent when all of these hold:
///
///   * `[auto_dream] enabled = true` (this struct)
///   * agent manifest has `auto_dream_enabled = true`
///   * at least `min_hours` have passed since that agent's last dream
///   * at least `min_sessions` sessions were touched since then
///
/// Configure in config.toml:
/// ```toml
/// [auto_dream]
/// enabled = false
/// min_hours = 24
/// min_sessions = 5
/// check_interval_secs = 86400
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AutoDreamConfig {
    /// Master toggle. Default: disabled — when false, no dream fires regardless
    /// of per-agent opt-in.
    pub enabled: bool,
    /// Minimum hours since that agent's last consolidation before the next
    /// one fires. Default: 24.
    #[serde(default = "default_auto_dream_min_hours")]
    pub min_hours: f64,
    /// Minimum number of sessions touched since that agent's last
    /// consolidation before the next one fires. Default: 5. Set to 0 to
    /// disable the session-count gate entirely.
    #[serde(default = "default_auto_dream_min_sessions")]
    pub min_sessions: u32,
    /// How often the *backstop* scheduler loop wakes up to check gates, in
    /// seconds. Default: 86400 (1 day). The primary trigger is the
    /// `AgentLoopEnd` hook that fires the moment a turn completes — the
    /// scheduler only catches opted-in agents that may go a long time
    /// without any turn (e.g., a channel bot waiting for inbound traffic).
    /// Lowering this just increases the rate of stat/SQL probes that mostly
    /// find nothing to do; raising it delays dreams only for the idle
    /// never-turned case.
    #[serde(default = "default_auto_dream_check_interval_secs")]
    pub check_interval_secs: u64,
    /// Optional override for the lock directory. When empty, defaults to
    /// `<data_dir>/auto_dream/`. Per-agent locks are stored as
    /// `<dir>/<agent_id>.lock`.
    #[serde(default)]
    pub lock_dir: String,
    /// Timeout for a single dream invocation in seconds. Default: 600.
    #[serde(default = "default_auto_dream_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_auto_dream_min_hours() -> f64 {
    24.0
}

fn default_auto_dream_min_sessions() -> u32 {
    5
}

fn default_auto_dream_check_interval_secs() -> u64 {
    // 1 day. Dreams are primarily triggered by the AgentLoopEnd hook the
    // moment a turn ends, not by this scheduler. The scheduler exists to
    // catch the "agent is opted-in but has no activity" edge case (e.g.
    // channel bots) where no turn ever fires. 1 day is frequent enough for
    // that fallback without wasting 144× more stat calls per day.
    86_400
}

fn default_auto_dream_timeout_secs() -> u64 {
    600
}

impl Default for AutoDreamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_hours: default_auto_dream_min_hours(),
            min_sessions: default_auto_dream_min_sessions(),
            check_interval_secs: default_auto_dream_check_interval_secs(),
            lock_dir: String::new(),
            timeout_secs: default_auto_dream_timeout_secs(),
        }
    }
}

/// Registry sync configuration.
///
/// Configure in config.toml:
/// ```toml
/// [registry]
/// cache_ttl_secs = 86400
/// # Optional: proxy/mirror prefix for users behind the GFW.
/// # All GitHub URLs are prefixed with this value, e.g.
/// #   registry_mirror = "https://ghproxy.cn"
/// # turns "https://github.com/..." into "https://ghproxy.cn/https://github.com/..."
/// registry_mirror = ""
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RegistryConfig {
    /// Cache TTL for registry sync in seconds (default: 86400 = 24 hours).
    /// The registry is re-downloaded when the local cache is older than this.
    #[serde(default = "default_registry_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Mirror/proxy prefix for GitHub URLs. When non-empty, all outbound
    /// GitHub requests (tarball downloads, git clones, raw content fetches)
    /// are prefixed with this URL. Useful for users in China Mainland where
    /// direct GitHub access is slow or blocked.
    ///
    /// Example: `"https://ghproxy.cn"` rewrites
    /// `https://github.com/...` → `https://ghproxy.cn/https://github.com/...`
    #[serde(default)]
    pub registry_mirror: String,
}

fn default_registry_cache_ttl_secs() -> u64 {
    86400
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            cache_ttl_secs: default_registry_cache_ttl_secs(),
            registry_mirror: String::new(),
        }
    }
}

/// Plugin registry configuration.
///
/// Configure in config.toml:
/// ```toml
/// [plugins]
/// plugin_registries = ["librefang/plugin-registry"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PluginsConfig {
    /// Additional GitHub `owner/repo` plugin registries to search.
    /// Merged with `context_engine.plugin_registries`.
    pub plugin_registries: Vec<String>,
}

fn default_prompt_caching() -> bool {
    true
}

/// Taint skip rules for a single argument path within a tool.
///
/// The policy key is a minimal JSONPath expression matched by the runtime
/// scanner. Supported wildcard syntax:
///
/// - `$.foo`      — exact property at any depth specified literally.
/// - `$.foo.*`    — any direct child of `$.foo` (single segment, non-array).
/// - `$.foo[*]`   — any array element of `$.foo` (e.g. `$.foo[0]`, `$.foo[42]`).
/// - `$.*`        — any top-level property.
///
/// Wildcards do NOT span multiple segments: `$.foo.*` matches `$.foo.bar`
/// but not `$.foo.bar.baz`. Use exact paths plus rule_sets for deep
/// exemptions across many paths.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct McpTaintPathPolicy {
    /// Rule IDs to skip when scanning this path.  An empty list means
    /// all rules apply (no exemption).
    #[serde(default)]
    pub skip_rules: Vec<crate::taint::TaintRuleId>,
}

/// What the scanner does for a tool's argument paths NOT matched by any
/// entry in [`McpTaintToolPolicy::paths`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum McpTaintToolAction {
    /// Apply the full taint rule set to every argument leaf (current behaviour).
    #[default]
    Scan,
    /// Bypass scanning entirely for this tool. Even sensitive object keys are
    /// allowed through. Use as a tool-level kill switch when a tool's arguments
    /// are by-design opaque (browser tab handles, DB session IDs, etc.).
    Skip,
}

/// Per-tool taint policy for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct McpTaintToolPolicy {
    /// What to do for argument paths not matched by `paths`.
    /// Defaults to [`McpTaintToolAction::Scan`] (current behaviour).
    ///
    /// Set to [`McpTaintToolAction::Skip`] to bypass scanning for the whole
    /// tool with one line, instead of enumerating every argument path.
    #[serde(default)]
    pub default: McpTaintToolAction,
    /// Per-path exemptions.  The key is a minimal JSONPath expression
    /// (e.g. `$.tabId`, `$.headers.*`, `$.items[*]`).  Paths not
    /// listed here have all rules applied (subject to `default`).
    #[serde(default)]
    pub paths: HashMap<String, McpTaintPathPolicy>,
    /// Names of top-level `[[taint_rules]]` rule sets to apply to every
    /// argument leaf of this tool. Each referenced set's `action` controls
    /// whether the listed rules block, warn, or log when they fire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_sets: Vec<String>,
}

/// Per-server taint policy that lets operators disable specific taint
/// rules for known-safe fields rather than turning off all scanning.
///
/// Example config.toml:
/// ```toml
/// [mcp_servers.my_firefox.taint_policy.tools.navigate]
/// default = "skip"   # bypass scanning entirely for `navigate`
///
/// [mcp_servers.my_firefox.taint_policy.tools.read_file.paths]
/// "$.content"   = { skip_rules = ["opaque_token"] }
/// "$.metadata.*" = { skip_rules = ["sensitive_key_name"] }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct McpTaintPolicy {
    /// Per-tool exemptions.  The key is the tool name as it appears in
    /// the MCP server's tool list (without the `mcp_<server>_` prefix).
    #[serde(default)]
    pub tools: HashMap<String, McpTaintToolPolicy>,
}

/// Severity action for a [`NamedTaintRuleSet`] when one of its rules fires
/// during MCP argument scanning.
///
/// **Overlap resolution: most permissive wins.** When a tool's `rule_sets`
/// list references multiple sets that all cover the same `TaintRuleId`,
/// the scanner applies the *most permissive* action — `Log` > `Warn` >
/// `Block`. This is intentional (it lets a narrow `audit_only` set carve
/// out exceptions to a broad `Block` set without rewriting the broad set),
/// but it means **adding an audit-only set with `action = log` will
/// silently neutralise any `block` set that overlaps on the same rule**.
/// The dashboard surfaces a hint next to the `rule_sets` field; operators
/// authoring config by hand should keep this in mind.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum McpTaintRuleSetAction {
    /// Abort the MCP tool call and surface a violation error to the LLM
    /// (current scanner default).
    #[default]
    Block,
    /// Allow the call through, but emit a structured WARN-level tracing
    /// event so operators can see exemptions firing.
    Warn,
    /// Allow the call through and emit at INFO level. Useful for building
    /// an exemption baseline before flipping a rule set to `block`.
    Log,
}

/// A reusable, named group of taint rules with an associated severity action.
///
/// Defined as `[[taint_rules]]` in `config.toml` and referenced by
/// [`McpTaintToolPolicy::rule_sets`]:
///
/// ```toml
/// [[taint_rules]]
/// name = "browser_handles"
/// action = "warn"
/// rules = ["opaque_token"]
///
/// [mcp_servers.camofox.taint_policy.tools.navigate]
/// rule_sets = ["browser_handles"]
/// ```
///
/// `PartialEq + Eq` are derived so the kernel's reload-plan diff can
/// detect `[[taint_rules]]` changes and emit
/// `HotAction::ReloadTaintRules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NamedTaintRuleSet {
    /// Identifier referenced by [`McpTaintToolPolicy::rule_sets`]. Must be
    /// unique within a [`KernelConfig`]; duplicate names are resolved by
    /// last-wins ordering.
    pub name: String,
    /// What happens when one of `rules` fires during scanning.
    #[serde(default)]
    pub action: McpTaintRuleSetAction,
    /// `TaintRuleId` variants this set covers.
    #[serde(default)]
    pub rules: Vec<crate::taint::TaintRuleId>,
}

/// Configuration entry for an MCP server.
///
/// This is the config.toml representation. The runtime `McpServerConfig`
/// struct is constructed from this during kernel boot.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct McpServerConfigEntry {
    /// Display name for this server.
    pub name: String,
    /// Catalog template this server was installed from, if any.
    ///
    /// Set when the user installs a server via `POST /api/mcp/servers` with
    /// `{template_id, credentials}` or the CLI `librefang mcp add <id>` flow.
    /// Stays `None` for manually-authored entries. Used by the dashboard to
    /// render the catalog badge and by the migrator.
    // `skip_serializing_if = "Option::is_none"` mirrors the `oauth` field —
    // `upsert_mcp_server_config` round-trips through serde_json → TOML and
    // null values would serialize as `template_id = ""`, which fails to
    // deserialize back into `Option<String>` on reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    /// Transport configuration. Optional — entries without transport are skipped at boot.
    pub transport: Option<McpTransportEntry>,
    /// Request timeout in seconds.
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
    /// Environment variables to pass through (e.g., ["GITHUB_PERSONAL_ACCESS_TOKEN"]).
    #[serde(default)]
    pub env: Vec<String>,
    /// Extra HTTP headers for SSE / Streamable-HTTP transports.
    /// Each entry is `"Header-Name: value"` (e.g., `"Authorization: Bearer <token>"`).
    #[serde(default)]
    pub headers: Vec<String>,
    /// Optional OAuth configuration for this MCP server.
    // `skip_serializing_if` is load-bearing: `upsert_mcp_server_config` goes
    // serde_json → TOML, and the null round-trip writes `oauth = ""` which
    // fails to deserialize back into `Option<McpOAuthConfig>` on reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,
    /// Enable outbound taint scanning for this MCP server (default: true).
    ///
    /// Set to `false` to disable the credential/PII content heuristic for
    /// trusted local servers (e.g. browser automation, database adapters)
    /// whose tool results contain opaque session handles that would otherwise
    /// trip the scanner. Key-name blocking remains active regardless.
    #[serde(default = "default_taint_scanning")]
    pub taint_scanning: bool,
    /// Fine-grained taint exemptions per tool and per argument path.
    ///
    /// When `taint_scanning = true` (the default), specific rules can be
    /// disabled for known-safe fields here rather than disabling all scanning.
    /// When `taint_scanning = false`, this field is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taint_policy: Option<McpTaintPolicy>,
}

fn default_taint_scanning() -> bool {
    true
}

fn default_mcp_timeout() -> u64 {
    30
}

fn default_http_compat_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object"})
}

/// HTTP request method for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HttpCompatMethod {
    Get,
    #[default]
    Post,
    Put,
    Patch,
    Delete,
}

/// How tool arguments are mapped onto an outbound HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HttpCompatRequestMode {
    #[default]
    JsonBody,
    Query,
    None,
}

/// How the built-in HTTP compatibility transport formats responses.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HttpCompatResponseMode {
    #[default]
    Json,
    Text,
}

/// Header injection config for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct HttpCompatHeaderConfig {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub value_env: Option<String>,
}

/// Declarative tool mapping for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct HttpCompatToolConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub path: String,
    #[serde(default)]
    pub method: HttpCompatMethod,
    #[serde(default)]
    pub request_mode: HttpCompatRequestMode,
    #[serde(default)]
    pub response_mode: HttpCompatResponseMode,
    #[serde(default = "default_http_compat_input_schema")]
    pub input_schema: serde_json::Value,
}

/// Transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportEntry {
    /// Subprocess with JSON-RPC over stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// HTTP Server-Sent Events.
    Sse { url: String },
    /// Streamable HTTP transport (MCP 2025-03-26+).
    Http { url: String },
    /// Built-in compatibility adapter for plain HTTP/JSON tool backends.
    HttpCompat {
        base_url: String,
        #[serde(default)]
        headers: Vec<HttpCompatHeaderConfig>,
        #[serde(default)]
        tools: Vec<HttpCompatToolConfig>,
    },
}

/// Optional OAuth configuration for an MCP server.
///
/// Used as fallback when the server doesn't support `.well-known` discovery,
/// or to override specific values from discovery. All fields are optional —
/// discovery results fill gaps, config values take precedence.
///
/// # Example (config.toml)
///
/// ```toml
/// [[mcp_servers]]
/// name = "custom-server"
/// transport = { type = "http", url = "https://my-server.com/mcp" }
///
/// [mcp_servers.oauth]
/// auth_url = "https://my-server.com/oauth/authorize"
/// token_url = "https://my-server.com/oauth/token"
/// client_id = "my-client-id"
/// scopes = ["read", "write"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct McpOAuthConfig {
    #[serde(default)]
    pub auth_url: Option<String>,
    #[serde(default)]
    pub token_url: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Slack-style user scopes, appended to the authorization URL as
    /// `&user_scope=...`. Most OAuth servers don't use this.
    #[serde(default)]
    pub user_scopes: Vec<String>,
}

/// A2A (Agent-to-Agent) protocol configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct A2aConfig {
    /// Whether A2A is enabled.
    pub enabled: bool,
    /// Service-level display name for the well-known agent card.
    #[serde(default = "default_a2a_name")]
    pub name: String,
    /// Service-level description for the well-known agent card.
    #[serde(default)]
    pub description: String,
    /// Path to serve A2A endpoints (default: "/a2a").
    #[serde(default = "default_a2a_path")]
    pub listen_path: String,
    /// External A2A agents to connect to.
    #[serde(default)]
    pub external_agents: Vec<ExternalAgent>,
}

fn default_a2a_name() -> String {
    "LibreFang Agent OS".to_string()
}

fn default_a2a_path() -> String {
    "/a2a".to_string()
}

/// An external A2A agent to discover and interact with.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExternalAgent {
    /// Display name.
    pub name: String,
    /// Agent endpoint URL.
    pub url: String,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_true() -> bool {
    true
}

// ── Shared channel timeout defaults ────────────────────────────────

/// Default initial backoff in seconds for channels using exponential backoff (1s).
fn default_channel_initial_backoff_secs() -> u64 {
    1
}

/// Default maximum backoff in seconds for channels using exponential backoff (60s).
fn default_channel_max_backoff_secs() -> u64 {
    60
}

/// Default initial backoff for channels that default to 2s (WeChat, QQ, Feishu, etc.).
fn default_channel_initial_backoff_2s() -> u64 {
    2
}

/// Default poll interval for Signal (2s).
fn default_signal_poll_interval_secs() -> u64 {
    2
}

/// Default Telegram long-poll timeout (30s).
fn default_telegram_long_poll_timeout_secs() -> u64 {
    30
}

impl Default for KernelConfig {
    fn default() -> Self {
        let home_dir = librefang_home_dir();
        Self {
            config_version: super::version::CONFIG_VERSION,
            data_dir: home_dir.join("data"),
            home_dir,
            log_level: "info".to_string(),
            api_listen: DEFAULT_API_LISTEN.to_string(),
            network_enabled: false,
            agent_max_iterations: None,
            max_history_messages: None,
            default_routing: None,
            default_model: DefaultModelConfig::default(),
            memory: MemoryConfig::default(),
            memory_wiki: MemoryWikiConfig::default(),
            network: NetworkConfig::default(),
            channels: ChannelsConfig::default(),
            api_key: String::new(),
            require_auth_for_reads: None,
            trusted_manifest_signers: Vec::new(),
            dashboard_user: String::new(),
            dashboard_pass: String::new(),
            dashboard_pass_hash: String::new(),
            mode: KernelMode::default(),
            language: "en".to_string(),
            users: Vec::new(),
            channel_role_mapping: ChannelRoleMapping::default(),
            mcp_servers: Vec::new(),
            taint_rules: Vec::new(),
            a2a: None,
            usage_footer: UsageFooterMode::default(),
            stable_prefix_mode: false,
            web: WebConfig::default(),
            fallback_providers: Vec::new(),
            llm: LlmConfig::default(),
            browser: BrowserConfig::default(),
            extensions: ExtensionsConfig::default(),
            skills: SkillsConfig::default(),
            vault: VaultConfig::default(),
            workspaces_dir: None,
            log_dir: None,
            media: crate::media::MediaConfig::default(),
            links: crate::media::LinkConfig::default(),
            reload: ReloadConfig::default(),
            webhook_triggers: None,
            triggers: TriggersConfig::default(),
            approval: crate::approval::ApprovalPolicy::default(),
            notification: crate::approval::NotificationConfig::default(),
            max_cron_jobs: default_max_cron_jobs(),
            cron_session_max_tokens: None,
            cron_session_max_messages: None,
            cron_session_warn_fraction: default_cron_session_warn_fraction(),
            cron_session_warn_total_tokens: default_cron_session_warn_total_tokens(),
            cron_session_compaction_mode: CronCompactionMode::default(),
            cron_session_compaction_keep_recent: default_cron_session_compaction_keep_recent(),
            include: Vec::new(),
            exec_policy: ExecPolicy::default(),
            bindings: Vec::new(),
            broadcast: BroadcastConfig::default(),
            auto_reply: AutoReplyConfig::default(),
            canvas: CanvasConfig::default(),
            tts: TtsConfig::default(),
            docker: DockerSandboxConfig::default(),
            tool_exec: crate::tool_exec::ToolExecConfig::default(),
            pairing: PairingConfig::default(),
            auth_profiles: BTreeMap::new(),
            thinking: None,
            budget: BudgetConfig::default(),
            provider_urls: BTreeMap::new(),
            provider_proxy_urls: BTreeMap::new(),
            provider_request_timeout_secs: BTreeMap::new(),
            provider_regions: BTreeMap::new(),
            provider_api_keys: BTreeMap::new(),
            local_probe_interval_secs: default_local_probe_interval_secs(),
            vertex_ai: VertexAiConfig::default(),
            azure_openai: AzureOpenAiConfig::default(),
            oauth: OAuthConfig::default(),
            sidecar_channels: Vec::new(),
            proxy: ProxyConfig::default(),
            prompt_caching: default_prompt_caching(),
            session: SessionConfig::default(),
            compaction: CompactionTomlConfig::default(),
            queue: QueueConfig::default(),
            task_board: TaskBoardConfig::default(),
            external_auth: ExternalAuthConfig::default(),
            tool_policy: crate::tool_policy::ToolPolicy::default(),
            proactive_memory: crate::memory::ProactiveMemoryConfig::default(),
            auto_dream: AutoDreamConfig::default(),
            context_engine: ContextEngineTomlConfig::default(),
            audit: AuditConfig::default(),
            health_check: HealthCheckConfig::default(),
            heartbeat: HeartbeatTomlConfig::default(),
            plugins: PluginsConfig::default(),
            registry: RegistryConfig::default(),
            cors_origin: Vec::new(),
            trusted_hosts: Vec::new(),
            trusted_proxies: Vec::new(),
            trust_forwarded_for: false,
            allowed_mount_roots: Vec::new(),
            privacy: PrivacyConfig::default(),
            strict_config: false,
            qwen_code_path: None,
            sanitize: SanitizeConfig::default(),
            inbox: InboxConfig::default(),
            telemetry: TelemetryConfig::default(),
            prompt_intelligence: PromptIntelligenceConfig::default(),
            update_channel: UpdateChannel::default(),
            rate_limit: RateLimitConfig::default(),
            tool_timeout_secs: default_tool_timeout_secs(),
            tool_timeouts: std::collections::BTreeMap::new(),
            max_upload_size_bytes: default_max_upload_size_bytes(),
            max_concurrent_bg_llm: default_max_concurrent_bg_llm(),
            max_agent_call_depth: default_max_agent_call_depth(),
            max_request_body_bytes: default_max_request_body_bytes(),
            terminal: TerminalConfig::default(),
            uar: None,
            storage: StorageConfig::default(),
            tool_invoke: ToolInvokeConfig::default(),
            parallel_tools: ParallelToolsConfig::default(),
            tool_results: ToolResultsConfig::default(),
            workflow_stale_timeout_minutes: default_workflow_stale_timeout_minutes(),
            workflow_default_total_timeout_secs: None,
        }
    }
}

impl KernelConfig {
    /// Resolved workspaces root directory.
    pub fn effective_workspaces_dir(&self) -> PathBuf {
        self.workspaces_dir
            .clone()
            .unwrap_or_else(|| self.home_dir.join("workspaces"))
    }

    /// Resolved directory for standalone agent workspaces.
    pub fn effective_agent_workspaces_dir(&self) -> PathBuf {
        self.effective_workspaces_dir().join("agents")
    }

    /// Resolved directory for hand workspaces.
    pub fn effective_hands_workspaces_dir(&self) -> PathBuf {
        self.effective_workspaces_dir().join("hands")
    }

    /// Parse the TCP port number from `api_listen`.
    ///
    /// Returns `None` when the address string is malformed. Callers that rely
    /// on the port for security-relevant decisions (e.g. Origin validation)
    /// MUST fail closed in the `None` case rather than assume a default.
    pub fn listen_port(&self) -> Option<u16> {
        self.api_listen
            .rsplit(':')
            .next()
            .and_then(|s| s.parse::<u16>().ok())
    }

    /// Resolve the API key env var name for a provider.
    ///
    /// Checks: 1) explicit `provider_api_keys` mapping, 2) `auth_profiles` first entry,
    /// 3) convention `{PROVIDER_UPPER}_API_KEY`.
    pub fn resolve_api_key_env(&self, provider: &str) -> String {
        // 1. Explicit mapping in [provider_api_keys]
        if let Some(env_var) = self.provider_api_keys.get(provider) {
            return env_var.clone();
        }
        // 2. Auth profiles (first profile by priority)
        if let Some(profiles) = self.auth_profiles.get(provider) {
            let mut sorted: Vec<_> = profiles.iter().collect();
            sorted.sort_by_key(|p| p.priority);
            if let Some(best) = sorted.first() {
                return best.api_key_env.clone();
            }
        }
        // 3. Convention: NVIDIA → NVIDIA_API_KEY
        format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"))
    }
}

/// SECURITY: Custom Debug impl redacts sensitive fields (api_key).
impl std::fmt::Debug for KernelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelConfig")
            .field("home_dir", &self.home_dir)
            .field("data_dir", &self.data_dir)
            .field("log_level", &self.log_level)
            .field("api_listen", &self.api_listen)
            .field("network_enabled", &self.network_enabled)
            .field("default_model", &self.default_model)
            .field("memory", &self.memory)
            .field("network", &self.network)
            .field("channels", &self.channels)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("mode", &self.mode)
            .field("language", &self.language)
            .field("users", &format!("{} user(s)", self.users.len()))
            .field(
                "mcp_servers",
                &format!("{} server(s)", self.mcp_servers.len()),
            )
            .field("a2a", &self.a2a.as_ref().map(|a| a.enabled))
            .field("usage_footer", &self.usage_footer)
            .field("stable_prefix_mode", &self.stable_prefix_mode)
            .field("web", &self.web)
            .field(
                "fallback_providers",
                &format!("{} provider(s)", self.fallback_providers.len()),
            )
            .field("browser", &self.browser)
            .field("extensions", &self.extensions)
            .field("vault", &format!("enabled={}", self.vault.enabled))
            .field("workspaces_dir", &self.workspaces_dir)
            .field("log_dir", &self.log_dir)
            .field(
                "media",
                &format!(
                    "image={} audio={} video={}",
                    self.media.image_description,
                    self.media.audio_transcription,
                    self.media.video_description
                ),
            )
            .field("links", &format!("enabled={}", self.links.enabled))
            .field("reload", &self.reload.mode)
            .field(
                "webhook_triggers",
                &self.webhook_triggers.as_ref().map(|w| w.enabled),
            )
            .field(
                "approval",
                &format!("{} tool(s)", self.approval.require_approval.len()),
            )
            .field("max_cron_jobs", &self.max_cron_jobs)
            .field("include", &format!("{} file(s)", self.include.len()))
            .field("exec_policy", &self.exec_policy.mode)
            .field("bindings", &format!("{} binding(s)", self.bindings.len()))
            .field(
                "broadcast",
                &format!("{} route(s)", self.broadcast.routes.len()),
            )
            .field(
                "auto_reply",
                &format!("enabled={}", self.auto_reply.enabled),
            )
            .field("canvas", &format!("enabled={}", self.canvas.enabled))
            .field("tts", &format!("enabled={}", self.tts.enabled))
            .field("docker", &format!("enabled={}", self.docker.enabled))
            .field("pairing", &format!("enabled={}", self.pairing.enabled))
            .field(
                "auth_profiles",
                &format!("{} provider(s)", self.auth_profiles.len()),
            )
            .field("thinking", &self.thinking.is_some())
            .field(
                "provider_api_keys",
                &format!("{} mapping(s)", self.provider_api_keys.len()),
            )
            .field("session", &self.session)
            .field("queue", &self.queue)
            .field(
                "external_auth",
                &format!("enabled={}", self.external_auth.enabled),
            )
            .field("privacy", &format!("{:?}", self.privacy.mode))
            .field("strict_config", &self.strict_config)
            .field("qwen_code_path", &self.qwen_code_path)
            .finish()
    }
}

/// Resolve the LibreFang home directory.
///
/// Priority: `LIBREFANG_HOME` env var > `~/.librefang`.
fn librefang_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

/// Default LLM model configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DefaultModelConfig {
    /// Provider name (e.g., "anthropic", "openai").
    pub provider: String,
    /// Model identifier.
    pub model: String,
    /// Environment variable name for the API key.
    /// Defaults to `"{PROVIDER}_API_KEY"` pattern when omitted.
    #[serde(default)]
    pub api_key_env: String,
    /// Optional base URL override.
    pub base_url: Option<String>,
    /// Message timeout in seconds for CLI-based providers (e.g. Claude Code).
    /// The timeout is inactivity-based: the process is killed only after this
    /// many seconds of silence on stdout, not wall-clock time.
    #[serde(default = "default_message_timeout_secs")]
    pub message_timeout_secs: u64,
    /// Provider-specific extension parameters that are flattened directly
    /// into the API request body.
    #[serde(default, flatten)]
    pub extra_params: HashMap<String, serde_json::Value>,
    /// Claude Code CLI profile directories for token rotation.
    /// Each entry is a path to a `.claude/` config dir (e.g. `~/.claude-profiles/account-2`).
    /// When multiple profiles are configured, a TokenRotationDriver wraps them
    /// for automatic failover on rate limits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cli_profile_dirs: Vec<String>,
}

fn default_message_timeout_secs() -> u64 {
    300
}

impl Default for DefaultModelConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            model: String::new(),
            api_key_env: String::new(),
            base_url: None,
            message_timeout_secs: default_message_timeout_secs(),
            extra_params: HashMap::new(),
            cli_profile_dirs: Vec::new(),
        }
    }
}

/// Memory substrate configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MemoryConfig {
    /// Path to SQLite database file.
    pub sqlite_path: Option<PathBuf>,
    /// Embedding model for semantic search.
    pub embedding_model: String,
    /// Maximum memories before consolidation is triggered.
    pub consolidation_threshold: u64,
    /// Memory decay rate (0.0 = no decay, 1.0 = aggressive decay).
    pub decay_rate: f64,
    /// Embedding provider. Valid values: `"openai"`, `"groq"`, `"mistral"`,
    /// `"together"`, `"fireworks"`, `"cohere"`, `"ollama"`, `"bedrock"`,
    /// `"vllm"`, `"lmstudio"`, or `"auto"`.
    /// `None` or `"auto"` = probe API-key env vars across all cloud providers,
    /// then fall back to local Ollama.
    #[serde(default)]
    pub embedding_provider: Option<String>,
    /// Environment variable name for the embedding API key.
    #[serde(default)]
    pub embedding_api_key_env: Option<String>,
    /// Override embedding dimensions instead of auto-inferring from model name.
    #[serde(default)]
    pub embedding_dimensions: Option<usize>,
    /// How often to run memory consolidation (hours). 0 = disabled.
    #[serde(default = "default_consolidation_interval")]
    pub consolidation_interval_hours: u64,
    /// When true, use SQLite FTS5 full-text search instead of embedding-based
    /// vector similarity. Eliminates the need for an external embedding provider.
    #[serde(default)]
    pub fts_only: Option<bool>,
    /// Time-based memory decay configuration.
    #[serde(default)]
    pub decay: MemoryDecayConfig,
    /// Chunking configuration for long documents.
    #[serde(default)]
    pub chunking: ChunkConfig,
    /// Vector store backend selection.
    ///
    /// | Value | Behaviour |
    /// |-------|-----------|
    /// | `"auto"` (default / unset) | SurrealDB when the `surreal-backend` feature is compiled in; SQLite otherwise. |
    /// | `"surreal"` | Force SurrealDB HNSW+BM25 hybrid search via `SurrealSemanticBackend`. Requires `surreal-backend` feature. |
    /// | `"sqlite"` | Force SQLite cosine-similarity in-process store (`SqliteVectorStore`). |
    /// | `"http"` | Remote HTTP vector store (`HttpVectorStore`); see `vector_store_url`. |
    #[serde(default)]
    pub vector_backend: Option<String>,
    /// Base URL for the HTTP vector store (used when `vector_backend = "http"`).
    #[serde(default)]
    pub vector_store_url: Option<String>,
    /// How many days to keep soft-deleted memories (`deleted = 1`) before
    /// the periodic retention sweep hard-deletes them and reclaims their
    /// embedding BLOB. Default: 30. Set to 0 to disable hard-delete (rows
    /// stay forever, leaking embedding storage — see #3467).
    #[serde(default = "default_soft_delete_retention_days")]
    pub soft_delete_retention_days: u64,
    /// Maximum number of pooled SQLite connections served by the memory
    /// substrate (#3378 follow-up). The pre-pool design serialised every
    /// SQLite call through a single `Mutex<Connection>`; the r2d2 pool
    /// removes that bottleneck but introduces a new ceiling — too low and
    /// the trigger lane / channel bridges / cron / audit / idempotency
    /// callers contend on `pool.get()`, too high and per-connection page
    /// caches add up (each connection holds at most ~2 MiB via
    /// `PRAGMA cache_size=-2000`). The default of 8 matches
    /// `queue.concurrency.trigger_lane` so the lane semaphore, not the
    /// pool, is the limiting factor under typical workloads. Bump in
    /// lockstep with `trigger_lane` if you raise that, or lower for
    /// memory-constrained deployments. Pool exhaustion is surfaced via
    /// the `librefang_memory_pool_get_failed_total{store=...}` counter.
    #[serde(default = "default_memory_pool_size")]
    pub pool_size: u32,
}

fn default_soft_delete_retention_days() -> u64 {
    30
}

fn default_memory_pool_size() -> u32 {
    8
}

/// Configuration for splitting long documents into overlapping chunks.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ChunkConfig {
    /// Whether chunking is enabled. When false, text is stored as a single blob.
    pub enabled: bool,
    /// Maximum chunk size in characters.
    pub max_chunk_size: usize,
    /// Overlap between consecutive chunks in characters.
    pub overlap: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_chunk_size: 1500,
            overlap: 200,
        }
    }
}

fn default_consolidation_interval() -> u64 {
    24
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            sqlite_path: None,
            embedding_model: "text-embedding-3-small".to_string(),
            consolidation_threshold: 10_000,
            decay_rate: 0.1,
            embedding_provider: None,
            embedding_api_key_env: None,
            embedding_dimensions: None,
            consolidation_interval_hours: default_consolidation_interval(),
            fts_only: None,
            decay: MemoryDecayConfig::default(),
            chunking: ChunkConfig::default(),
            vector_backend: None,
            vector_store_url: None,
            soft_delete_retention_days: default_soft_delete_retention_days(),
            pool_size: default_memory_pool_size(),
        }
    }
}

/// Operating mode for the memory wiki (issue #3329).
///
/// * `Isolated` (default) — own vault under `vault_path`, populated only by
///   explicit `wiki_write` calls. No coupling to the memory substrate.
/// * `Bridge` — read shared artifacts from the memory substrate through the
///   public seams. Reserved for follow-up; v1 returns
///   `WikiError::ModeNotImplemented`.
/// * `UnsafeLocal` — same-machine escape hatch that points at an existing
///   filesystem path (e.g. an Obsidian vault). Reserved for follow-up.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWikiMode {
    #[default]
    Isolated,
    Bridge,
    UnsafeLocal,
}

/// Markdown render flavor for vault pages (issue #3329).
///
/// * `Native` — plain Markdown links: `[topic](topic.md)`.
/// * `Obsidian` — Obsidian / Logseq wiki-link syntax: `[[topic]]`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum MemoryWikiRenderMode {
    #[default]
    Native,
    Obsidian,
}

/// Which `wiki_write` calls actually land on disk.
///
/// * `Tagged` (default) — only writes that pass an explicit `topic` tag are
///   accepted, matching v1 acceptance criteria. Other writes are rejected
///   with `WikiError::InvalidTopic`.
/// * `All` — accept every write the kernel forwards. Useful in testing or
///   in `unsafe_local` mode where the agent already speaks vault semantics.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWikiIngestFilter {
    #[default]
    Tagged,
    All,
}

/// Memory wiki configuration (issue #3329). **Off by default** — set
/// `enabled = true` to opt in.
///
/// ```toml
/// [memory_wiki]
/// enabled = false
/// mode = "isolated"                    # isolated | bridge | unsafe_local
/// vault_path = "~/.librefang/wiki/main"
/// render_mode = "native"               # native | obsidian
/// ingest_filter = "tagged"             # tagged | all
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MemoryWikiConfig {
    /// Master switch. When `false` (default), the wiki is not constructed
    /// and the `wiki_*` builtin tools all return a `Disabled` error.
    pub enabled: bool,
    /// Operating mode (see `MemoryWikiMode`). v1 wires `Isolated`; the
    /// other variants return `ModeNotImplemented` until follow-up PRs.
    pub mode: MemoryWikiMode,
    /// Filesystem location of the vault root. Defaults to
    /// `<librefang_home>/wiki/main`. The `~` prefix is honoured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<PathBuf>,
    /// Markdown render flavor for cross-references (`[[topic]]` placeholders).
    pub render_mode: MemoryWikiRenderMode,
    /// Whether the vault accepts every `wiki_write` or only ones with an
    /// explicit topic tag.
    pub ingest_filter: MemoryWikiIngestFilter,
}

impl MemoryWikiConfig {
    /// Resolve the effective vault root against the kernel's home
    /// directory. Honours the leading `~` and `~/...` forms (current
    /// user's home directory) on `vault_path`; falls back to
    /// `<librefang_home>/wiki/main` when `vault_path` is unset, where
    /// `<librefang_home>` is the **caller-supplied** `home_dir` rather
    /// than the env-derived `LIBREFANG_HOME`. That matters for embedded
    /// or test profiles whose `KernelConfig.home_dir` deliberately
    /// points somewhere other than `~/.librefang` — the wiki must not
    /// leak data across profiles.
    ///
    /// `~user/...` (POSIX user-name expansion) is **not** honoured —
    /// only the bare `~` and `~/...` forms are. Set the path explicitly
    /// with no `~` prefix when targeting another user's home.
    pub fn resolved_vault_path(&self, home_dir: &std::path::Path) -> PathBuf {
        if let Some(path) = &self.vault_path {
            return expand_tilde(path);
        }
        home_dir.join("wiki").join("main")
    }
}

/// Expand a leading `~` or `~/...` to the current user's home directory.
/// Other forms (including `~user/...`) are returned unchanged — see
/// [`MemoryWikiConfig::resolved_vault_path`] for the rationale.
fn expand_tilde(path: &std::path::Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(rest);
    }
    if raw.as_ref() == "~" {
        return dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    }
    path.to_path_buf()
}

/// Time-based memory decay configuration.
///
/// When enabled, memories that have not been accessed within their scope's TTL
/// are automatically deleted during periodic decay runs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MemoryDecayConfig {
    /// Whether time-based decay is enabled.
    pub enabled: bool,
    /// SESSION-scope memories expire after this many days of no access.
    pub session_ttl_days: u32,
    /// AGENT-scope memories expire after this many days of no access.
    pub agent_ttl_days: u32,
    /// How often to run the decay sweep (hours).
    pub decay_interval_hours: u32,
}

impl Default for MemoryDecayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            session_ttl_days: 7,
            agent_ttl_days: 30,
            decay_interval_hours: 1,
        }
    }
}

/// Network layer configuration.
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NetworkConfig {
    /// libp2p listen addresses.
    pub listen_addresses: Vec<String>,
    /// Bootstrap peers for DHT.
    pub bootstrap_peers: Vec<String>,
    /// Enable mDNS for local discovery.
    pub mdns_enabled: bool,
    /// Maximum number of connected peers.
    pub max_peers: u32,
    /// Pre-shared secret for OFP HMAC authentication (required when network is enabled).
    pub shared_secret: String,
    /// SECURITY (#3876): Maximum number of  requests a single OFP
    /// peer may send per minute before being rate-limited.
    ///
    /// Each peer connection is tracked independently. Excess messages are
    /// rejected with a 429 error response; a  is emitted with the
    /// peer ID and current rate so operators can investigate abuse.
    ///
    /// Set to  to disable per-peer message rate limiting (not recommended
    /// for production federations). Default: 60.
    pub max_messages_per_peer_per_minute: u32,
    /// SECURITY (#3876): Optional cumulative LLM token budget per OFP peer per hour.
    ///
    /// When set, the node tracks how many tokens each peer's
    /// requests have consumed in the current hour window. If a peer exceeds
    /// this budget the request is rejected with a 429 error.
    ///
    ///  means no per-peer token cap (default). Set to a value like
    ///  to bound the LLM spend a single federated peer can force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_llm_tokens_per_peer_per_hour: Option<u64>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/0".to_string()],
            bootstrap_peers: vec![],
            mdns_enabled: true,
            max_peers: 50,
            shared_secret: String::new(),
            max_messages_per_peer_per_minute: 60,
            max_llm_tokens_per_peer_per_hour: None,
        }
    }
}

/// SECURITY: Custom Debug impl redacts sensitive fields (shared_secret).
impl std::fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("listen_addresses", &self.listen_addresses)
            .field("bootstrap_peers", &self.bootstrap_peers)
            .field("mdns_enabled", &self.mdns_enabled)
            .field("max_peers", &self.max_peers)
            .field(
                "shared_secret",
                &if self.shared_secret.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field(
                "max_messages_per_peer_per_minute",
                &self.max_messages_per_peer_per_minute,
            )
            .field(
                "max_llm_tokens_per_peer_per_hour",
                &self.max_llm_tokens_per_peer_per_hour,
            )
            .finish()
    }
}

/// Channel bridge configuration.
///
/// Each field uses `OneOrMany<T>` to support both single-instance (`[channels.telegram]`)
/// and multi-instance (`[[channels.telegram]]`) TOML syntax for multi-bot routing.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Telegram bot configuration(s).
    pub telegram: OneOrMany<TelegramConfig>,
    /// Discord bot configuration(s).
    pub discord: OneOrMany<DiscordConfig>,
    /// Slack bot configuration(s).
    pub slack: OneOrMany<SlackConfig>,
    /// WhatsApp Cloud API configuration(s).
    pub whatsapp: OneOrMany<WhatsAppConfig>,
    /// Signal (via signal-cli) configuration(s).
    pub signal: OneOrMany<SignalConfig>,
    /// Matrix protocol configuration(s).
    pub matrix: OneOrMany<MatrixConfig>,
    /// Email (IMAP/SMTP) configuration(s).
    pub email: OneOrMany<EmailConfig>,
    /// Microsoft Teams configuration(s).
    pub teams: OneOrMany<TeamsConfig>,
    /// Mattermost configuration(s).
    pub mattermost: OneOrMany<MattermostConfig>,
    /// IRC configuration(s).
    pub irc: OneOrMany<IrcConfig>,
    /// Google Chat configuration(s).
    pub google_chat: OneOrMany<GoogleChatConfig>,
    /// Twitch chat configuration(s).
    pub twitch: OneOrMany<TwitchConfig>,
    /// Rocket.Chat configuration(s).
    pub rocketchat: OneOrMany<RocketChatConfig>,
    /// Zulip configuration(s).
    pub zulip: OneOrMany<ZulipConfig>,
    /// XMPP/Jabber configuration(s).
    pub xmpp: OneOrMany<XmppConfig>,
    // Wave 3 — High-value channels
    /// LINE Messaging API configuration(s).
    pub line: OneOrMany<LineConfig>,
    /// Viber Bot API configuration(s).
    pub viber: OneOrMany<ViberConfig>,
    /// Facebook Messenger configuration(s).
    pub messenger: OneOrMany<MessengerConfig>,
    /// Reddit API configuration(s).
    pub reddit: OneOrMany<RedditConfig>,
    /// Mastodon Streaming API configuration(s).
    pub mastodon: OneOrMany<MastodonConfig>,
    /// Bluesky/AT Protocol configuration(s).
    pub bluesky: OneOrMany<BlueskyConfig>,
    /// Feishu/Lark Open Platform configuration(s).
    pub feishu: OneOrMany<FeishuConfig>,
    /// Revolt (Discord-like) configuration(s).
    pub revolt: OneOrMany<RevoltConfig>,
    // Wave 4 — Enterprise & community channels
    /// Nextcloud Talk configuration(s).
    pub nextcloud: OneOrMany<NextcloudConfig>,
    /// Guilded bot configuration(s).
    pub guilded: OneOrMany<GuildedConfig>,
    /// Keybase chat configuration(s).
    pub keybase: OneOrMany<KeybaseConfig>,
    /// Threema Gateway configuration(s).
    pub threema: OneOrMany<ThreemaConfig>,
    /// Nostr relay configuration(s).
    pub nostr: OneOrMany<NostrConfig>,
    /// Webex bot configuration(s).
    pub webex: OneOrMany<WebexConfig>,
    /// Pumble bot configuration(s).
    pub pumble: OneOrMany<PumbleConfig>,
    /// Flock bot configuration(s).
    pub flock: OneOrMany<FlockConfig>,
    /// Twist API configuration(s).
    pub twist: OneOrMany<TwistConfig>,
    // Wave 5 — Niche & differentiating channels
    /// Mumble text chat configuration(s).
    pub mumble: OneOrMany<MumbleConfig>,
    /// DingTalk robot configuration(s).
    pub dingtalk: OneOrMany<DingTalkConfig>,
    /// QQ Bot API v2 configuration(s).
    pub qq: OneOrMany<QqConfig>,
    /// Discourse forum configuration(s).
    pub discourse: OneOrMany<DiscourseConfig>,
    /// Gitter streaming configuration(s).
    pub gitter: OneOrMany<GitterConfig>,
    /// ntfy.sh pub/sub configuration(s).
    pub ntfy: OneOrMany<NtfyConfig>,
    /// Gotify notification configuration(s).
    pub gotify: OneOrMany<GotifyConfig>,
    /// Generic webhook configuration(s).
    pub webhook: OneOrMany<WebhookConfig>,
    /// Voice channel (WebSocket + STT/TTS) configuration(s).
    pub voice: OneOrMany<VoiceConfig>,
    /// LinkedIn messaging configuration(s).
    pub linkedin: OneOrMany<LinkedInConfig>,
    /// WeChat personal account (iLink) configuration(s).
    pub wechat: OneOrMany<WeChatConfig>,
    /// WeCom/WeChat Work configuration(s).
    pub wecom: OneOrMany<WeComConfig>,

    // --- Global file-download settings ---
    /// Maximum file size in bytes for channel file downloads (default: 50 MB).
    #[serde(default = "default_file_download_max_bytes")]
    pub file_download_max_bytes: u64,

    /// Directory to store downloaded files.
    /// When `None`, defaults to `std::env::temp_dir()/librefang_uploads`.
    #[serde(default)]
    pub file_download_dir: Option<String>,

    // --- Global file-upload settings ---
    /// Maximum file size in bytes for channel file uploads (bot → server),
    /// applied uniformly by the Matrix and Telegram adapters before sending
    /// outbound media (default: 50 MB).
    ///
    /// Distinct from `file_download_max_bytes` (inbound: server → agent →
    /// disk). An operator who wants a larger inbound budget but a smaller
    /// outbound budget — or vice versa — must set both independently. The
    /// 50 MiB default matches both the Matrix homeserver convention and
    /// Telegram's bot API ceiling, so omitting the field leaves behaviour
    /// unchanged from the pre-#4882 hardcoded constants.
    #[serde(default = "default_file_upload_max_bytes")]
    pub file_upload_max_bytes: u64,
}

/// Default max file download size: 50 MB.
fn default_file_download_max_bytes() -> u64 {
    50 * 1024 * 1024
}

/// Default max file upload size: 50 MB.
///
/// Same numeric value as the download default but a separate function so
/// the two can drift if the protocol ceilings ever diverge (Matrix's
/// theoretical event-size limit is much higher than Telegram's bot API
/// 50 MiB, but the conservative shared default protects operators from
/// surprises on the smaller-cap side).
fn default_file_upload_max_bytes() -> u64 {
    50 * 1024 * 1024
}

impl Default for ChannelsConfig {
    // Manual impl so `file_download_max_bytes` matches the
    // `#[serde(default = "default_file_download_max_bytes")]` value (50 MiB)
    // instead of `u64::default() == 0`. Without this, code paths that build
    // a `ChannelsConfig` programmatically (e.g. `KernelConfig::default()`,
    // tests, configs without a `[channels]` section) would silently set
    // `file_download_max_bytes = 0`, causing the bridge to reject every
    // channel attachment as oversized. See issue #4436.
    fn default() -> Self {
        Self {
            telegram: OneOrMany::default(),
            discord: OneOrMany::default(),
            slack: OneOrMany::default(),
            whatsapp: OneOrMany::default(),
            signal: OneOrMany::default(),
            matrix: OneOrMany::default(),
            email: OneOrMany::default(),
            teams: OneOrMany::default(),
            mattermost: OneOrMany::default(),
            irc: OneOrMany::default(),
            google_chat: OneOrMany::default(),
            twitch: OneOrMany::default(),
            rocketchat: OneOrMany::default(),
            zulip: OneOrMany::default(),
            xmpp: OneOrMany::default(),
            line: OneOrMany::default(),
            viber: OneOrMany::default(),
            messenger: OneOrMany::default(),
            reddit: OneOrMany::default(),
            mastodon: OneOrMany::default(),
            bluesky: OneOrMany::default(),
            feishu: OneOrMany::default(),
            revolt: OneOrMany::default(),
            nextcloud: OneOrMany::default(),
            guilded: OneOrMany::default(),
            keybase: OneOrMany::default(),
            threema: OneOrMany::default(),
            nostr: OneOrMany::default(),
            webex: OneOrMany::default(),
            pumble: OneOrMany::default(),
            flock: OneOrMany::default(),
            twist: OneOrMany::default(),
            mumble: OneOrMany::default(),
            dingtalk: OneOrMany::default(),
            qq: OneOrMany::default(),
            discourse: OneOrMany::default(),
            gitter: OneOrMany::default(),
            ntfy: OneOrMany::default(),
            gotify: OneOrMany::default(),
            webhook: OneOrMany::default(),
            voice: OneOrMany::default(),
            linkedin: OneOrMany::default(),
            wechat: OneOrMany::default(),
            wecom: OneOrMany::default(),
            file_download_max_bytes: default_file_download_max_bytes(),
            file_download_dir: None,
            file_upload_max_bytes: default_file_upload_max_bytes(),
        }
    }
}

impl ChannelsConfig {
    /// Resolve the effective directory for storing downloaded channel
    /// attachments (and any other code path that historically wrote into
    /// `<temp>/librefang_uploads`). Returns the operator-configured
    /// `[channels].file_download_dir` when set, otherwise the legacy
    /// `std::env::temp_dir()/librefang_uploads` default.
    ///
    /// This helper is the single source of truth — no other site in the
    /// codebase should hardcode the literal `"librefang_uploads"` so the
    /// kernel can hand the same path to the file-read sandbox and agents
    /// can actually open the files the bridge tells them about. See
    /// issues #4434 and #4435.
    pub fn effective_file_download_dir(&self) -> std::path::PathBuf {
        self.file_download_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("librefang_uploads"))
    }
}

/// Telegram channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TelegramConfig {
    /// Env var name holding the bot token (NOT the token itself).
    pub bot_token_env: String,
    /// Telegram user IDs allowed to interact (empty = allow all).
    /// Accepts strings for consistency; numeric TOML integers are coerced to strings.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Custom Telegram Bot API base URL for proxies or mirrors.
    /// Defaults to `https://api.telegram.org` when not set.
    #[serde(default)]
    pub api_url: Option<String>,
    /// Initial backoff in seconds on API failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on API failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Long-poll timeout in seconds sent to getUpdates (default: 30).
    #[serde(default = "default_telegram_long_poll_timeout_secs")]
    pub long_poll_timeout_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// Message coalescing window in milliseconds for Telegram-specific
    /// ergonomics (#4145). When set, messages from the same sender arriving
    /// within this window are buffered and dispatched to the agent as a
    /// single batched context. This prevents out-of-order processing in the
    /// common pattern where a user forwards a message and immediately follows
    /// up with a comment.
    ///
    /// This is a thin alias for [`ChannelOverrides::message_debounce_ms`]:
    /// if `overrides.message_debounce_ms` is non-zero it wins; otherwise the
    /// value here is applied. `Some(0)` explicitly disables coalescing.
    /// `None` (default) leaves behavior unchanged.
    #[serde(default)]
    pub message_coalesce_window_ms: Option<u64>,
    /// Thread-based agent routing for forum topics.
    ///
    /// Maps Telegram `message_thread_id` (as string) to an agent name.
    /// Messages in a matched thread are routed to that agent instead of
    /// the `default_agent`. Unmatched threads fall back to normal routing.
    ///
    /// ```toml
    /// [channels.telegram.thread_routes]
    /// "12345" = "research-agent"
    /// "67890" = "coding-agent"
    /// ```
    #[serde(default)]
    pub thread_routes: std::collections::HashMap<String, String>,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "TELEGRAM_BOT_TOKEN".to_string(),
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            poll_interval_secs: 1,
            api_url: None,
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            long_poll_timeout_secs: default_telegram_long_poll_timeout_secs(),
            overrides: ChannelOverrides::default(),
            message_coalesce_window_ms: None,
            thread_routes: std::collections::HashMap::new(),
        }
    }
}

impl TelegramConfig {
    /// Returns a [`ChannelOverrides`] with the Telegram-specific
    /// `message_coalesce_window_ms` alias applied to
    /// [`ChannelOverrides::message_debounce_ms`].
    ///
    /// Resolution rules (#4145):
    /// 1. If `overrides.message_debounce_ms` is non-zero, it wins.
    /// 2. Otherwise, if `message_coalesce_window_ms` is `Some`, that value
    ///    is copied into `message_debounce_ms` (including `Some(0)`, which
    ///    is a no-op since the existing field is already 0).
    /// 3. Otherwise the unmodified clone of `overrides` is returned.
    pub fn effective_overrides(&self) -> ChannelOverrides {
        let mut ov = self.overrides.clone();
        if ov.message_debounce_ms == 0 {
            if let Some(window) = self.message_coalesce_window_ms {
                ov.message_debounce_ms = window;
            }
        }
        ov
    }
}

/// Discord channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DiscordConfig {
    /// Env var name holding the bot token (NOT the token itself).
    pub bot_token_env: String,
    /// Guild (server) IDs allowed to interact (empty = allow all).
    /// Accepts strings for consistency with other channel configs.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_guilds: Vec<String>,
    /// User IDs allowed to interact (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Gateway intents bitmask (default: 37376 = GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT).
    pub intents: u64,
    /// Ignore messages from other bots (default: true).
    /// Set to false to allow bot-to-bot interactions in multi-agent setups.
    #[serde(default = "default_true")]
    pub ignore_bots: bool,
    /// Custom text patterns that trigger the bot (case-insensitive contains match).
    /// When any pattern matches the message content, the bot treats it as if it was mentioned.
    /// Example: `["hey bot", "!ask"]`
    #[serde(default)]
    pub mention_patterns: Vec<String>,
    /// Initial backoff in seconds on WebSocket failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on WebSocket failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "DISCORD_BOT_TOKEN".to_string(),
            allowed_guilds: vec![],
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            intents: 37376,
            ignore_bots: true,
            mention_patterns: vec![],
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Slack channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SlackConfig {
    /// Env var name holding the app-level token (xapp-) for Socket Mode.
    pub app_token_env: String,
    /// Env var name holding the bot token (xoxb-) for REST API.
    pub bot_token_env: String,
    /// Channel IDs allowed to interact (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Whether to disable link unfurling (preview expansion) in sent messages.
    /// When set to `false`, Slack will not expand link previews.
    /// When `None` (default), Slack uses its own default behavior.
    #[serde(default)]
    pub unfurl_links: Option<bool>,
    /// Initial backoff in seconds on WebSocket failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on WebSocket failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// When true, bot replies are posted as top-level channel messages instead
    /// of threaded replies. Defaults to `None` (i.e. use normal threading).
    #[serde(default)]
    pub force_flat_replies: Option<bool>,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            app_token_env: "SLACK_APP_TOKEN".to_string(),
            bot_token_env: "SLACK_BOT_TOKEN".to_string(),
            allowed_channels: vec![],
            account_id: None,
            default_agent: None,
            unfurl_links: None,
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
            force_flat_replies: None,
        }
    }
}

/// WhatsApp Cloud API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WhatsAppConfig {
    /// Env var name holding the access token (Cloud API mode).
    pub access_token_env: String,
    /// Env var name holding the webhook verify token (Cloud API mode).
    pub verify_token_env: String,
    /// WhatsApp Business phone number ID (Cloud API mode).
    pub phone_number_id: String,
    /// Port to listen for webhook callbacks (Cloud API mode).
    pub webhook_port: u16,
    /// Env var name holding the WhatsApp Web gateway URL (QR/Web mode).
    /// When set, outgoing messages are routed through the gateway instead of Cloud API.
    pub gateway_url_env: String,
    /// Allowed phone numbers (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Owner phone numbers for owner-routing mode (digits only, no '+' prefix).
    /// When set, messages from non-owner numbers are forwarded to the first
    /// owner number with sender context, and the sender receives an auto-ack.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub owner_numbers: Vec<String>,
    /// Conversation tracker TTL in hours (Web gateway mode).
    /// Active stranger conversations expire after this period of inactivity.
    #[serde(default = "default_conversation_ttl_hours")]
    pub conversation_ttl_hours: u32,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

fn default_conversation_ttl_hours() -> u32 {
    24
}

fn default_local_probe_interval_secs() -> u64 {
    60
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            access_token_env: "WHATSAPP_ACCESS_TOKEN".to_string(),
            verify_token_env: "WHATSAPP_VERIFY_TOKEN".to_string(),
            phone_number_id: String::new(),
            webhook_port: 8443,
            gateway_url_env: "WHATSAPP_WEB_GATEWAY_URL".to_string(),
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            owner_numbers: vec![],
            conversation_ttl_hours: default_conversation_ttl_hours(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Signal channel adapter configuration (via signal-cli REST API).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SignalConfig {
    /// URL of the signal-cli REST API (e.g., "http://localhost:8080").
    pub api_url: String,
    /// Registered phone number.
    pub phone_number: String,
    /// Allowed phone numbers (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Poll interval in seconds for checking new messages (default: 2).
    #[serde(default = "default_signal_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Optional API key sent as `Authorization: Bearer <api_key>` on every request.
    /// If absent, requests are sent without an Authorization header.
    #[serde(default)]
    pub api_key: Option<String>,
    /// When `true`, allow the `api_url` to point at loopback / RFC-1918 addresses.
    /// Defaults to `false`; set to `true` only when signal-cli runs on localhost.
    #[serde(default)]
    pub allow_local: bool,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:8080".to_string(),
            phone_number: String::new(),
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            poll_interval_secs: default_signal_poll_interval_secs(),
            api_key: None,
            allow_local: false,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Matrix protocol channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MatrixConfig {
    /// Matrix homeserver URL (e.g., `"https://matrix.org"`).
    pub homeserver_url: String,
    /// Bot user ID (e.g., "@librefang:matrix.org").
    pub user_id: String,
    /// Env var name holding the access token.
    pub access_token_env: String,
    /// Room IDs to listen in (empty = all joined rooms).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Whether to auto-accept room invites (default: false).
    #[serde(default)]
    pub auto_accept_invites: bool,
    /// Initial backoff in seconds on sync failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on sync failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver_url: "https://matrix.org".to_string(),
            user_id: String::new(),
            access_token_env: "MATRIX_ACCESS_TOKEN".to_string(),
            allowed_rooms: vec![],
            account_id: None,
            default_agent: None,
            auto_accept_invites: false,
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Email (IMAP/SMTP) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct EmailConfig {
    /// IMAP server host.
    pub imap_host: String,
    /// IMAP port (993 for TLS).
    pub imap_port: u16,
    /// SMTP server host.
    pub smtp_host: String,
    /// SMTP port (587 for STARTTLS).
    pub smtp_port: u16,
    /// Email address (used for both IMAP and SMTP).
    pub username: String,
    /// Env var name holding the password.
    pub password_env: String,
    /// IMAP-specific username (falls back to `username` if not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_username: Option<String>,
    /// Env var for IMAP-specific password (falls back to `password_env` if not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_password_env: Option<String>,
    /// SMTP-specific username (falls back to `username` if not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    /// Env var for SMTP-specific password (falls back to `password_env` if not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_password_env: Option<String>,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// IMAP folders to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub folders: Vec<String>,
    /// Only process emails from these senders (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_senders: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// Path to a PEM file containing one or more CA certificates to trust
    /// **in addition to** the system root store for the IMAP TLS connection
    /// (#4877). Use this for self-hosted IMAP behind a private CA — strictly
    /// preferred over [`Self::tls_accept_invalid_certs`] because hostname /
    /// expiry / signature validation remain ON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_root_ca_path: Option<String>,
    /// When `true`, the IMAP TLS connection accepts ANY server certificate —
    /// hostname, expiry, signature, and chain are all ignored. **This makes
    /// the connection vulnerable to MITM and exposes the entire mailbox.**
    /// Provided as a last-resort dev escape hatch (e.g., expired self-signed
    /// cert that cannot be renewed). Defaults to `false`. A WARN is logged
    /// on every IMAP connect attempt while this is enabled, so the risk
    /// stays visible in operator logs rather than being noticed once at
    /// startup and then forgotten (#4877).
    #[serde(default)]
    pub tls_accept_invalid_certs: bool,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 587,
            username: String::new(),
            password_env: "EMAIL_PASSWORD".to_string(),
            imap_username: None,
            imap_password_env: None,
            smtp_username: None,
            smtp_password_env: None,
            poll_interval_secs: 30,
            folders: vec!["INBOX".to_string()],
            allowed_senders: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
            tls_root_ca_path: None,
            tls_accept_invalid_certs: false,
        }
    }
}

/// Microsoft Teams (Bot Framework v3) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TeamsConfig {
    /// Azure Bot App ID.
    pub app_id: String,
    /// Env var name holding the app password.
    pub app_password_env: String,
    /// Env var name holding the outgoing webhook security token (base64-encoded).
    /// Used for HMAC-SHA256 verification of inbound webhook requests.
    /// Required by default; setting `signature_required = false` opts out (dev only).
    #[serde(default)]
    pub security_token_env: String,
    /// Reject adapter startup unless a security token is configured (default `true`).
    /// Setting to `false` is strongly discouraged — webhook becomes a public endpoint.
    #[serde(default = "default_true")]
    pub signature_required: bool,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Allowed tenant IDs (empty = allow all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_tenants: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TeamsConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_password_env: "TEAMS_APP_PASSWORD".to_string(),
            security_token_env: "TEAMS_SECURITY_TOKEN".to_string(),
            signature_required: true,
            webhook_port: 3978,
            allowed_tenants: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Mattermost channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MattermostConfig {
    /// Mattermost server URL (e.g., `"https://mattermost.example.com"`).
    pub server_url: String,
    /// Env var name holding the bot token.
    pub token_env: String,
    /// Allowed channel IDs (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Initial backoff in seconds on WebSocket failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on WebSocket failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "MATTERMOST_TOKEN".to_string(),
            allowed_channels: vec![],
            account_id: None,
            default_agent: None,
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// IRC channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct IrcConfig {
    /// IRC server hostname.
    pub server: String,
    /// IRC server port.
    pub port: u16,
    /// Bot nickname.
    pub nick: String,
    /// Env var name holding the server password (optional).
    pub password_env: Option<String>,
    /// Channels to join (e.g., `["#librefang", "#general"]`).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub channels: Vec<String>,
    /// Use TLS (requires tokio-native-tls).
    pub use_tls: bool,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Initial backoff in seconds on connection failures (default: 1).
    #[serde(default = "default_channel_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on connection failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for IrcConfig {
    fn default() -> Self {
        Self {
            server: "irc.libera.chat".to_string(),
            port: 6667,
            nick: "librefang".to_string(),
            password_env: None,
            channels: vec![],
            use_tls: false,
            account_id: None,
            default_agent: None,
            initial_backoff_secs: default_channel_initial_backoff_secs(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Google Chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GoogleChatConfig {
    /// Env var name holding the service account JSON key.
    pub service_account_env: String,
    /// Path to a Google service account JSON key file (alternative to env var).
    /// When set, JWT authentication is used to obtain OAuth2 access tokens.
    #[serde(default)]
    pub service_account_key_path: Option<String>,
    /// Space IDs to listen in.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub space_ids: Vec<String>,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GoogleChatConfig {
    fn default() -> Self {
        Self {
            service_account_env: "GOOGLE_CHAT_SERVICE_ACCOUNT".to_string(),
            service_account_key_path: None,
            space_ids: vec![],
            webhook_port: 8444,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Twitch chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TwitchConfig {
    /// Env var name holding the OAuth token.
    pub oauth_token_env: String,
    /// Twitch channels to join (without #).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub channels: Vec<String>,
    /// Bot nickname.
    pub nick: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TwitchConfig {
    fn default() -> Self {
        Self {
            oauth_token_env: "TWITCH_OAUTH_TOKEN".to_string(),
            channels: vec![],
            nick: "librefang".to_string(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Rocket.Chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RocketChatConfig {
    /// Rocket.Chat server URL.
    pub server_url: String,
    /// Env var name holding the auth token.
    pub token_env: String,
    /// User ID for the bot.
    pub user_id: String,
    /// Allowed channel IDs (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RocketChatConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "ROCKETCHAT_TOKEN".to_string(),
            user_id: String::new(),
            allowed_channels: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Zulip channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ZulipConfig {
    /// Zulip server URL.
    pub server_url: String,
    /// Bot email address.
    pub bot_email: String,
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Streams to listen in.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub streams: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ZulipConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            bot_email: String::new(),
            api_key_env: "ZULIP_API_KEY".to_string(),
            streams: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// XMPP/Jabber channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct XmppConfig {
    /// JID (e.g., "bot@jabber.org").
    pub jid: String,
    /// Env var name holding the password.
    pub password_env: String,
    /// XMPP server hostname (defaults to JID domain).
    pub server: String,
    /// XMPP server port.
    pub port: u16,
    /// MUC rooms to join.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub rooms: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for XmppConfig {
    fn default() -> Self {
        Self {
            jid: String::new(),
            password_env: "XMPP_PASSWORD".to_string(),
            server: String::new(),
            port: 5222,
            rooms: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 3 channel configs ─────────────────────────────────────────

/// LINE Messaging API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LineConfig {
    /// Env var name holding the channel secret.
    pub channel_secret_env: String,
    /// Env var name holding the channel access token.
    pub access_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self {
            channel_secret_env: "LINE_CHANNEL_SECRET".to_string(),
            access_token_env: "LINE_CHANNEL_ACCESS_TOKEN".to_string(),
            webhook_port: 8450,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Viber Bot API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ViberConfig {
    /// Env var name holding the auth token.
    pub auth_token_env: String,
    /// Webhook URL for receiving messages.
    pub webhook_url: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ViberConfig {
    fn default() -> Self {
        Self {
            auth_token_env: "VIBER_AUTH_TOKEN".to_string(),
            webhook_url: String::new(),
            webhook_port: 8451,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Facebook Messenger Platform channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MessengerConfig {
    /// Env var name holding the page access token.
    pub page_token_env: String,
    /// Env var name holding the webhook verify token.
    pub verify_token_env: String,
    /// Env var name holding the Facebook App Secret.
    /// Used for HMAC-SHA1 verification of incoming webhook POST requests
    /// via `X-Hub-Signature`. If absent or empty, verification is skipped
    /// with a warning (backwards compatibility).
    #[serde(default)]
    pub app_secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MessengerConfig {
    fn default() -> Self {
        Self {
            page_token_env: "MESSENGER_PAGE_TOKEN".to_string(),
            verify_token_env: "MESSENGER_VERIFY_TOKEN".to_string(),
            app_secret_env: "MESSENGER_APP_SECRET".to_string(),
            webhook_port: 8452,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Reddit API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RedditConfig {
    /// Reddit app client ID.
    pub client_id: String,
    /// Env var name holding the client secret.
    pub client_secret_env: String,
    /// Reddit bot username.
    pub username: String,
    /// Env var name holding the bot password.
    pub password_env: String,
    /// Subreddits to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub subreddits: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RedditConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret_env: "REDDIT_CLIENT_SECRET".to_string(),
            username: String::new(),
            password_env: "REDDIT_PASSWORD".to_string(),
            subreddits: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Mastodon Streaming API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MastodonConfig {
    /// Mastodon instance URL (e.g., `"https://mastodon.social"`).
    pub instance_url: String,
    /// Env var name holding the access token.
    pub access_token_env: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MastodonConfig {
    fn default() -> Self {
        Self {
            instance_url: String::new(),
            access_token_env: "MASTODON_ACCESS_TOKEN".to_string(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Bluesky/AT Protocol channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BlueskyConfig {
    /// Bluesky identifier (handle or DID).
    pub identifier: String,
    /// Env var name holding the app password.
    pub app_password_env: String,
    /// PDS service URL.
    pub service_url: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for BlueskyConfig {
    fn default() -> Self {
        Self {
            identifier: String::new(),
            app_password_env: "BLUESKY_APP_PASSWORD".to_string(),
            service_url: "https://bsky.social".to_string(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Feishu/Lark Open Platform channel adapter configuration.
///
/// Feishu (CN) and Lark (international) share the same API — set `region` to
/// `"intl"` for Lark or `"cn"` (default) for Feishu. The `receive_mode` field
/// controls whether the adapter uses a webhook HTTP server or a long-lived
/// WebSocket connection (default) to receive events.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct FeishuConfig {
    /// Feishu app ID.
    pub app_id: String,
    /// Env var name holding the app secret.
    pub app_secret_env: String,
    /// API region: `"cn"` for Feishu (default) or `"intl"` for Lark.
    #[serde(default)]
    pub region: String,
    /// How to receive inbound events: `"websocket"` (default) or `"webhook"`.
    #[serde(default = "default_receive_mode")]
    pub receive_mode: String,
    /// Port for the incoming webhook (only used when `receive_mode = "webhook"`).
    pub webhook_port: u16,
    /// Verification token for webhook event validation (webhook mode only).
    #[serde(default)]
    pub verification_token: Option<String>,
    /// Encrypt key for webhook event decryption (webhook mode only).
    #[serde(default)]
    pub encrypt_key: Option<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

fn default_receive_mode() -> String {
    "websocket".to_string()
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret_env: "FEISHU_APP_SECRET".to_string(),
            region: "cn".to_string(),
            receive_mode: "websocket".to_string(),
            webhook_port: 8453,
            verification_token: None,
            encrypt_key: None,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Connection mode for the WeCom intelligent bot adapter.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WeComMode {
    /// WebSocket long-connection (no public endpoint required).
    #[default]
    Websocket,
    /// URL callback (requires a publicly reachable HTTP endpoint).
    Callback,
}

/// WeCom intelligent bot adapter configuration.
///
/// Supports two connection modes:
/// - `websocket` (default): connects to `wss://openws.work.weixin.qq.com`
/// - `callback`: starts an HTTP server to receive message callbacks
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WeComConfig {
    /// Bot ID obtained from the WeCom admin console.
    pub bot_id: String,
    /// Env var name holding the bot secret.
    pub secret_env: String,
    /// Connection mode: "websocket" (default) or "callback".
    pub mode: WeComMode,
    /// Port for the callback HTTP server (only used in callback mode).
    pub webhook_port: u16,
    /// Env var name holding the callback verification token (callback mode only).
    pub token_env: Option<String>,
    /// Env var name holding the EncodingAESKey (callback mode only).
    pub encoding_aes_key_env: Option<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WeComConfig {
    fn default() -> Self {
        Self {
            bot_id: String::new(),
            secret_env: "WECOM_BOT_SECRET".to_string(),
            mode: WeComMode::default(),
            webhook_port: 8454,
            token_env: None,
            encoding_aes_key_env: None,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// WeChat personal account (iLink protocol) adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WeChatConfig {
    /// Env var name holding the bot token from a previous QR login session.
    /// If the env var is set and non-empty, the adapter skips QR login.
    pub bot_token_env: String,
    /// Allowed user IDs (empty = allow all). Format: `{hash}@im.wechat`.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Initial backoff in seconds on API failures (default: 2).
    #[serde(default = "default_channel_initial_backoff_2s")]
    pub initial_backoff_secs: u64,
    /// Maximum backoff in seconds on API failures (default: 60).
    #[serde(default = "default_channel_max_backoff_secs")]
    pub max_backoff_secs: u64,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WeChatConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "WECHAT_BOT_TOKEN".to_string(),
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            initial_backoff_secs: default_channel_initial_backoff_2s(),
            max_backoff_secs: default_channel_max_backoff_secs(),
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Revolt (Discord-like) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RevoltConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Revolt API URL (set to your self-hosted instance URL if not using revolt.chat).
    pub api_url: String,
    /// Revolt WebSocket URL (set to your self-hosted instance WS URL if not using revolt.chat).
    pub ws_url: String,
    /// Restrict to specific channel IDs (empty = all channels the bot is in).
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for RevoltConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "REVOLT_BOT_TOKEN".to_string(),
            api_url: "https://api.revolt.chat".to_string(),
            ws_url: "wss://ws.revolt.chat".to_string(),
            allowed_channels: Vec::new(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 4 channel configs ─────────────────────────────────────────

/// Nextcloud Talk channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NextcloudConfig {
    /// Nextcloud server URL.
    pub server_url: String,
    /// Env var name holding the auth token.
    pub token_env: String,
    /// Room tokens to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NextcloudConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token_env: "NEXTCLOUD_TOKEN".to_string(),
            allowed_rooms: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Guilded bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GuildedConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Server IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub server_ids: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GuildedConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "GUILDED_BOT_TOKEN".to_string(),
            server_ids: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Keybase chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct KeybaseConfig {
    /// Keybase username.
    pub username: String,
    /// Env var name holding the paper key.
    pub paperkey_env: String,
    /// Team names to listen in (empty = all DMs).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_teams: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for KeybaseConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            paperkey_env: "KEYBASE_PAPERKEY".to_string(),
            allowed_teams: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Threema Gateway channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ThreemaConfig {
    /// Threema Gateway ID.
    pub threema_id: String,
    /// Env var name holding the API secret.
    pub secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for ThreemaConfig {
    fn default() -> Self {
        Self {
            threema_id: String::new(),
            secret_env: "THREEMA_SECRET".to_string(),
            webhook_port: 8454,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Nostr relay channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NostrConfig {
    /// Env var name holding the private key (nsec or hex).
    pub private_key_env: String,
    /// Relay URLs to connect to.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub relays: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NostrConfig {
    fn default() -> Self {
        Self {
            private_key_env: "NOSTR_PRIVATE_KEY".to_string(),
            relays: vec!["wss://relay.damus.io".to_string()],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Webex bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WebexConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Room IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_rooms: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for WebexConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "WEBEX_BOT_TOKEN".to_string(),
            allowed_rooms: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Pumble bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PumbleConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for PumbleConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "PUMBLE_BOT_TOKEN".to_string(),
            webhook_port: 8455,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Flock bot channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct FlockConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for FlockConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "FLOCK_BOT_TOKEN".to_string(),
            webhook_port: 8456,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Twist API v3 channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TwistConfig {
    /// Env var name holding the API token.
    pub token_env: String,
    /// Workspace ID.
    pub workspace_id: String,
    /// Channel IDs to listen in (empty = all).
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub allowed_channels: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for TwistConfig {
    fn default() -> Self {
        Self {
            token_env: "TWIST_TOKEN".to_string(),
            workspace_id: String::new(),
            allowed_channels: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 5 channel configs ─────────────────────────────────────────

/// Mumble text chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MumbleConfig {
    /// Mumble server hostname.
    pub host: String,
    /// Mumble server port.
    pub port: u16,
    /// Bot username.
    pub username: String,
    /// Env var name holding the server password.
    pub password_env: String,
    /// Channel to join.
    pub channel: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for MumbleConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 64738,
            username: "librefang".to_string(),
            password_env: "MUMBLE_PASSWORD".to_string(),
            channel: String::new(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// How the DingTalk adapter receives inbound events.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DingTalkReceiveMode {
    /// HTTP webhook server (requires public IP / reverse proxy).
    Webhook,
    /// Long-lived WebSocket connection via DingTalk Stream protocol (default).
    #[default]
    Stream,
}

/// DingTalk Robot API channel adapter configuration.
///
/// Supports two receive modes:
/// - **Stream** (default): Uses `app_key` / `app_secret` to open a long-lived
///   WebSocket connection via the DingTalk Stream protocol. No public IP needed.
/// - **Webhook** (legacy): HTTP server that receives callback POST requests.
///   Requires `access_token` and `secret` for HMAC-SHA256 verification.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DingTalkConfig {
    /// How to receive inbound messages (stream or webhook).
    pub receive_mode: DingTalkReceiveMode,
    // -- Stream mode credentials --
    /// Env var name holding the DingTalk app key (stream mode).
    pub app_key_env: String,
    /// Env var name holding the DingTalk app secret (stream mode).
    pub app_secret_env: String,
    // -- Webhook mode credentials (legacy) --
    /// Env var name holding the webhook access token.
    pub access_token_env: String,
    /// Env var name holding the signing secret.
    pub secret_env: String,
    /// Port for the incoming webhook (webhook mode only).
    pub webhook_port: u16,
    /// Robot code for sending messages via the Open API (stream mode).
    /// If empty, falls back to app_key.
    #[serde(default)]
    pub robot_code: Option<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DingTalkConfig {
    fn default() -> Self {
        Self {
            receive_mode: DingTalkReceiveMode::default(),
            app_key_env: "DINGTALK_APP_KEY".to_string(),
            app_secret_env: "DINGTALK_APP_SECRET".to_string(),
            access_token_env: "DINGTALK_ACCESS_TOKEN".to_string(),
            secret_env: "DINGTALK_SECRET".to_string(),
            webhook_port: 8457,
            robot_code: None,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// QQ Bot API v2 channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct QqConfig {
    /// QQ Bot application ID.
    pub app_id: String,
    /// Env var name holding the app secret (NOT the secret itself).
    pub app_secret_env: String,
    /// QQ user IDs allowed to interact (empty = allow all).
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for QqConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret_env: "QQ_BOT_APP_SECRET".to_string(),
            allowed_users: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Discourse forum channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DiscourseConfig {
    /// Discourse base URL.
    pub base_url: String,
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// API username.
    pub api_username: String,
    /// Category slugs to monitor.
    #[serde(default, deserialize_with = "deserialize_string_or_int_vec")]
    pub categories: Vec<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for DiscourseConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key_env: "DISCOURSE_API_KEY".to_string(),
            api_username: "system".to_string(),
            categories: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Gitter Streaming API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GitterConfig {
    /// Env var name holding the auth token.
    pub token_env: String,
    /// Room ID to listen in.
    pub room_id: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GitterConfig {
    fn default() -> Self {
        Self {
            token_env: "GITTER_TOKEN".to_string(),
            room_id: String::new(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// ntfy.sh pub/sub channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NtfyConfig {
    /// ntfy server URL.
    pub server_url: String,
    /// Topic to subscribe/publish to.
    pub topic: String,
    /// Env var name holding the auth token (optional for public topics).
    pub token_env: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for NtfyConfig {
    fn default() -> Self {
        Self {
            server_url: "https://ntfy.sh".to_string(),
            topic: String::new(),
            token_env: "NTFY_TOKEN".to_string(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Gotify WebSocket channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GotifyConfig {
    /// Gotify server URL.
    pub server_url: String,
    /// Env var name holding the app token (for sending).
    pub app_token_env: String,
    /// Env var name holding the client token (for receiving).
    pub client_token_env: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for GotifyConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            app_token_env: "GOTIFY_APP_TOKEN".to_string(),
            client_token_env: "GOTIFY_CLIENT_TOKEN".to_string(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Generic webhook channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WebhookConfig {
    /// Env var name holding the HMAC signing secret.
    pub secret_env: String,
    /// Port to listen for incoming webhooks.
    pub listen_port: u16,
    /// URL to POST outgoing messages to.
    pub callback_url: Option<String>,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
    /// When true, incoming POST bodies are forwarded directly to the delivery
    /// target channel without invoking the LLM or any agent. Requires
    /// `deliver` to be set to a valid channel name (not "log").
    #[serde(default)]
    pub deliver_only: bool,
    /// Target channel name for direct delivery (e.g. "telegram", "discord").
    /// Required when `deliver_only` is true.
    #[serde(default)]
    pub deliver: Option<String>,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            secret_env: "WEBHOOK_SECRET".to_string(),
            listen_port: 8460,
            callback_url: None,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
            deliver_only: false,
            deliver: None,
        }
    }
}

/// Voice channel adapter configuration.
///
/// Runs a WebSocket server that accepts audio streams, transcribes via STT,
/// sends text to the agent, and returns synthesized speech via TTS.
///
/// ```toml
/// [channels.voice]
/// listen_port = 4546
/// api_key_env = "OPENAI_API_KEY"
/// stt_url = "https://api.openai.com"
/// tts_url = "https://api.openai.com"
/// tts_voice = "alloy"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct VoiceConfig {
    /// WebSocket server listen port (default: 4546).
    pub listen_port: u16,
    /// Env var name holding the API key for STT/TTS services.
    pub api_key_env: String,
    /// Base URL for the STT (Speech-to-Text) API.
    pub stt_url: String,
    /// Base URL for the TTS (Text-to-Speech) API.
    pub tts_url: String,
    /// TTS voice name (default: "alloy").
    pub tts_voice: String,
    /// Audio buffer threshold in bytes before triggering STT (default: 32768).
    pub buffer_threshold: usize,
    /// Unique identifier for this voice instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route voice messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            listen_port: 4546,
            api_key_env: "OPENAI_API_KEY".to_string(),
            stt_url: "https://api.openai.com".to_string(),
            tts_url: "https://api.openai.com".to_string(),
            tts_voice: "alloy".to_string(),
            buffer_threshold: 32768,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// LinkedIn Messaging API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LinkedInConfig {
    /// Env var name holding the OAuth2 access token.
    pub access_token_env: String,
    /// Organization ID for messaging.
    pub organization_id: String,
    /// Unique identifier for this bot instance (used for multi-bot routing).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Default agent name to route messages to.
    pub default_agent: Option<String>,
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
}

impl Default for LinkedInConfig {
    fn default() -> Self {
        Self {
            access_token_env: "LINKEDIN_ACCESS_TOKEN".to_string(),
            organization_id: String::new(),
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Terminal / CLI access control configuration.
///
/// Controls which clients may connect to the interactive terminal (WebSocket)
/// and how locality is determined.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TerminalConfig {
    /// Master switch — set to false to disable the terminal entirely.
    #[serde(default = "default_terminal_enabled")]
    pub enabled: bool,

    /// Additional allowed WebSocket origins beyond auto-detected localhost.
    /// Use when the dashboard is served from a custom domain (e.g. "https://my.domain.com").
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Allow terminal access from remote/proxied connections when no auth is configured.
    /// Default: false (local-only when unauthenticated).
    #[serde(default)]
    pub allow_remote: bool,

    /// When true, bare-loopback connections (127.0.0.1 / ::1 with no proxy
    /// headers) are rejected at auth time — only connections that arrived via
    /// a reverse proxy (carrying X-Forwarded-For / X-Real-IP) are considered
    /// "local". Enable only when running behind a reverse proxy that strips
    /// direct loopback access. Default: false.
    ///
    /// (Historically named `trust_proxy_headers`; the old name is still
    /// accepted for backward compatibility via `serde(alias)`.)
    #[serde(default, alias = "trust_proxy_headers")]
    pub require_proxy_headers: bool,

    /// Hard-override for the "remote + no authentication" combination.
    /// When `allow_remote` is true and no auth is configured, the terminal
    /// will still refuse every connection unless this flag is explicitly
    /// set to `true`. Intended as a foot-gun guard: enabling `allow_remote`
    /// alone is not enough to expose an unauthenticated shell to the network.
    /// Default: false.
    #[serde(default)]
    pub allow_unauthenticated_remote: bool,

    /// Enable tmux-backed multi-window terminal. Only effective when `tmux` binary is available.
    #[serde(default = "default_tmux_enabled")]
    pub tmux_enabled: bool,

    /// Maximum number of tmux windows that may exist simultaneously. Guards against DoS.
    #[serde(default = "default_max_windows")]
    pub max_windows: u32,

    /// Optional explicit path to the `tmux` binary. If None, resolve via PATH.
    #[serde(default)]
    pub tmux_binary_path: Option<String>,
}

fn default_terminal_enabled() -> bool {
    true
}

fn default_tmux_enabled() -> bool {
    true
}

fn default_max_windows() -> u32 {
    16
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_origins: Vec::new(),
            allow_remote: false,
            require_proxy_headers: false,
            allow_unauthenticated_remote: false,
            tmux_enabled: true,
            max_windows: 16,
            tmux_binary_path: None,
        }
    }
}

/// Configuration for `POST /api/tools/{name}/invoke`.
///
/// The direct-invoke endpoint bypasses the agent loop, so the usual
/// capability gate (agent manifest `tools` list) does not apply. To avoid
/// a situation where any holder of an API key can call any tool, this
/// endpoint is fail-closed: disabled by default, and — when enabled — only
/// the tools whose names match one of the glob patterns in `allowlist` may
/// be executed.
///
/// ```toml
/// [tool_invoke]
/// enabled = true
/// allowlist = ["web_search", "web_fetch", "file_read"]
/// ```
///
/// Pitfall: `allowlist = ["*"]` matches every tool and effectively turns
/// the endpoint into "give API-key holders the same power as the kernel".
/// Prefer narrow globs (`"file_*"`, `"web_*"`) — reserve `"*"` for
/// trusted single-tenant dev environments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ToolInvokeConfig {
    /// Master switch. When `false` (default) the endpoint rejects every
    /// request with 403 regardless of the allowlist.
    #[serde(default)]
    pub enabled: bool,

    /// Glob patterns of tool names that may be invoked via the REST
    /// endpoint (e.g. `"web_*"`, `"file_read"`). Empty list denies all
    /// invocations even when `enabled = true`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowlist: Vec<String>,
}

impl ToolInvokeConfig {
    /// Whether the endpoint is configured to accept `tool_name`.
    ///
    /// Returns `true` only when `enabled = true` AND at least one allowlist
    /// pattern matches. Patterns use the same glob semantics as agent
    /// capability grants (`*` wildcards).
    pub fn permits(&self, tool_name: &str) -> bool {
        self.enabled
            && self
                .allowlist
                .iter()
                .any(|pattern| crate::capability::glob_matches(pattern, tool_name))
    }
}

/// Configuration for the agent loop's parallel tool dispatcher.
///
/// PR-3 ships the schema only; the agent loop still runs tool calls
/// strictly sequentially. PR-4 wires the dispatcher into the runtime
/// and PR-5 flips `enabled` on by default.
///
/// ```toml
/// [parallel_tools]
/// enabled = false
/// max_concurrent = 4
/// mcp_default_safety = "write_shared"   # or "read_only"
/// mcp_readonly_allowlist = ["mcp__github__list_issues"]
/// ```
///
/// Falls back to fully sequential execution when `enabled = false`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct ParallelToolsConfig {
    /// Master switch. Default `false` so PR-3 ships with no behaviour
    /// change. PR-5 will flip the default to `true` after the streaming
    /// dispatcher integration lands.
    pub enabled: bool,

    /// Cap on concurrent tool calls within a single bucket. `0` =
    /// uncapped (use the bucket size). The dispatcher honours this
    /// when launching futures via `join_all`.
    pub max_concurrent: u32,

    /// Default `ParallelSafety` class assigned to MCP tools whose
    /// servers don't carry `readOnlyHint` annotations. Conservative
    /// default `"write_shared"` keeps unannotated MCP tools serialised
    /// (one per bucket) instead of optimistically parallelising.
    /// Accepted values: `"read_only"` | `"write_shared"`. PR-4 will
    /// promote this to a typed enum once the dispatcher consumes it.
    pub mcp_default_safety: String,

    /// Explicit allowlist of MCP tool names that should be treated as
    /// `ReadOnly` regardless of `mcp_default_safety`. Names match the
    /// fully-namespaced form (`mcp__server__name`).
    pub mcp_readonly_allowlist: Vec<String>,
}

impl Default for ParallelToolsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent: 4,
            mcp_default_safety: "write_shared".to_string(),
            mcp_readonly_allowlist: Vec::new(),
        }
    }
}

/// Tool-result context budget and artifact spill configuration.
///
/// Controls what happens when a tool returns a very large payload.  The primary
/// mechanism shipped in #3347 1/N is **artifact spill**: responses larger than
/// `spill_threshold_bytes` are written to `~/.librefang/data/artifacts/` and
/// the agent receives a compact stub with a handle it can pass to
/// `read_artifact` to retrieve the content in chunks.
///
/// `max_bytes_per_turn` enforces a per-turn cumulative byte cap (#3347 2/N).
/// `history_fold_after_turns` triggers tool-result history summarisation via
/// the aux-LLM channel (#3347 3/N) — falls back to byte truncation when no
/// aux-LLM is configured.
/// `artifact_max_age_days` evicts stale spill artifacts at daemon startup
/// (#3347 4/N).  Set to `0` to disable eviction entirely.
///
/// ```toml
/// [tool_results]
/// spill_threshold_bytes    = 16384        # 16 KB — spill to artifact store above this
/// max_artifact_bytes       = 67108864     # 64 MiB — per-artifact write cap
/// max_bytes_per_turn       = 50000        # cumulative byte cap across all tool results in one turn
/// history_fold_after_turns = 8            # fold stale tool results after this many turns
/// artifact_max_age_days    = 30           # evict spill artifacts older than this on startup; 0 disables
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(default)]
pub struct ToolResultsConfig {
    /// Spill threshold in bytes.  Tool results larger than this are written to
    /// the artifact store; the agent receives a stub with a `read_artifact`
    /// handle instead of the raw payload.  Default: 16 384 bytes (16 KB).
    #[serde(default = "default_spill_threshold_bytes")]
    pub spill_threshold_bytes: u64,
    /// Maximum bytes for a single artifact write.  Spill is skipped (falling
    /// back to truncation) when a tool result exceeds this cap, preventing a
    /// single oversized response from filling the artifact store.
    /// Default: 67 108 864 bytes (64 MiB).
    #[serde(default = "default_max_artifact_bytes")]
    pub max_artifact_bytes: u64,
    /// Cumulative byte cap across all tool results in a single LLM turn
    /// (#3347 2/N).  When the running total would exceed this, the next
    /// result is escalated to artifact spill (or tail truncation if spill
    /// fails).  Resets between assistant turns.  Default: 50 000 bytes.
    #[serde(default = "default_max_bytes_per_turn")]
    pub max_bytes_per_turn: u64,
    /// Fold (summarise via aux-LLM) stale tool results after this many turns
    /// (#3347 3/N).  Tool-result messages older than this threshold have
    /// each `ContentBlock::ToolResult.content` rewritten in place to a
    /// compact `[history-fold] <summary>` stub before the next LLM call.
    /// `tool_use_id` / `tool_name` / `is_error` / `status` are preserved so
    /// every assistant `tool_use` block keeps its matching `tool_result`
    /// (provider APIs reject mismatched ids with 400). Falls back to a
    /// static `[summarisation unavailable]` stub when no aux-LLM is
    /// configured or the aux call fails, so stale payload is always
    /// removed from context.  Default: 8 turns.
    #[serde(default = "default_history_fold_after_turns")]
    pub history_fold_after_turns: u32,
    /// Minimum number of newly-stale tool-result messages required to
    /// trigger a fold pass.  Without a batch threshold a long-running
    /// session would drag exactly one new message across the staleness
    /// boundary every turn and pay an aux-LLM round-trip per turn just to
    /// fold a single message.  Skipping until at least N have accumulated
    /// amortises that cost.  Set to `1` to fold every turn (no batching);
    /// `0` is treated as `1`.  Default: 4.
    #[serde(default = "default_fold_min_batch_size")]
    pub fold_min_batch_size: u32,
    /// Evict spill artifacts older than this many days at daemon startup
    /// (#3347 4/N).  The artifact store grows unbounded otherwise — every
    /// large tool result writes a content-addressed file under
    /// `~/.librefang/data/artifacts/` and the original
    /// `read_artifact` handle in the message history is the only thing
    /// pinning it.  After history compaction or a long agent lifetime
    /// those handles are no longer reachable, but the bytes remain on
    /// disk.  GC runs once per daemon boot, fire-and-forget.
    /// Set to `0` to disable eviction entirely.  Default: 30 days.
    #[serde(default = "default_artifact_max_age_days")]
    pub artifact_max_age_days: u32,
}

fn default_spill_threshold_bytes() -> u64 {
    16_384
}

fn default_max_artifact_bytes() -> u64 {
    64 * 1024 * 1024
}

fn default_max_bytes_per_turn() -> u64 {
    50_000
}

fn default_history_fold_after_turns() -> u32 {
    8
}

fn default_fold_min_batch_size() -> u32 {
    4
}

fn default_artifact_max_age_days() -> u32 {
    30
}

impl Default for ToolResultsConfig {
    fn default() -> Self {
        Self {
            spill_threshold_bytes: default_spill_threshold_bytes(),
            max_artifact_bytes: default_max_artifact_bytes(),
            max_bytes_per_turn: default_max_bytes_per_turn(),
            history_fold_after_turns: default_history_fold_after_turns(),
            fold_min_batch_size: default_fold_min_batch_size(),
            artifact_max_age_days: default_artifact_max_age_days(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_message_coalesce_window_default_is_none() {
        let tg = TelegramConfig::default();
        assert!(tg.message_coalesce_window_ms.is_none());
        // Backward compat: effective overrides leaves debounce disabled.
        assert_eq!(tg.effective_overrides().message_debounce_ms, 0);
    }

    #[test]
    fn telegram_message_coalesce_window_alias_fills_debounce() {
        let tg = TelegramConfig {
            message_coalesce_window_ms: Some(2000),
            ..Default::default()
        };
        assert_eq!(tg.effective_overrides().message_debounce_ms, 2000);
    }

    #[test]
    fn telegram_explicit_overrides_debounce_wins_over_alias() {
        let tg = TelegramConfig {
            message_coalesce_window_ms: Some(2000),
            overrides: ChannelOverrides {
                message_debounce_ms: 500,
                ..Default::default()
            },
            ..Default::default()
        };
        // The explicit `overrides.message_debounce_ms` is non-zero, so the
        // alias must NOT clobber it.
        assert_eq!(tg.effective_overrides().message_debounce_ms, 500);
    }

    #[test]
    fn telegram_message_coalesce_window_zero_keeps_disabled() {
        let tg = TelegramConfig {
            message_coalesce_window_ms: Some(0),
            ..Default::default()
        };
        assert_eq!(tg.effective_overrides().message_debounce_ms, 0);
    }

    #[test]
    fn telegram_message_coalesce_window_parses_from_toml() {
        let toml_str = r#"
            bot_token_env = "TG"
            message_coalesce_window_ms = 1500
        "#;
        let tg: TelegramConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(tg.message_coalesce_window_ms, Some(1500));
        assert_eq!(tg.effective_overrides().message_debounce_ms, 1500);
    }

    #[test]
    fn test_session_config_defaults_backward_compatible() {
        let sc = SessionConfig::default();
        assert!(sc.reset_prompt.is_none());
        assert!(sc.context_injection.is_empty());
        assert!(sc.on_session_start_script.is_none());
    }

    #[test]
    fn test_session_config_with_context_injection() {
        let toml_str = r#"
            reset_prompt = "Hello"

            [[context_injection]]
            name = "rules"
            content = "Follow the rules."
            position = "system"

            [[context_injection]]
            name = "prefs"
            content = "Be concise."
            position = "after_reset"
            condition = "agent.tags contains 'chat'"
        "#;
        let sc: SessionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.reset_prompt.as_deref(), Some("Hello"));
        assert_eq!(sc.context_injection.len(), 2);

        assert_eq!(sc.context_injection[0].name, "rules");
        assert_eq!(sc.context_injection[0].position, InjectionPosition::System);
        assert!(sc.context_injection[0].condition.is_none());

        assert_eq!(sc.context_injection[1].name, "prefs");
        assert_eq!(
            sc.context_injection[1].position,
            InjectionPosition::AfterReset
        );
        assert_eq!(
            sc.context_injection[1].condition.as_deref(),
            Some("agent.tags contains 'chat'")
        );
    }

    #[test]
    fn test_session_config_empty_injection_list() {
        let toml_str = r#"
            retention_days = 7
        "#;
        let sc: SessionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(sc.retention_days, 7);
        assert!(sc.context_injection.is_empty());
        assert!(sc.on_session_start_script.is_none());
    }

    #[test]
    fn test_injection_position_default() {
        assert_eq!(InjectionPosition::default(), InjectionPosition::System);
    }

    #[test]
    fn test_injection_position_deserialization() {
        #[derive(Deserialize)]
        struct Wrapper {
            pos: InjectionPosition,
        }
        let w: Wrapper = toml::from_str(r#"pos = "system""#).unwrap();
        assert_eq!(w.pos, InjectionPosition::System);

        let w: Wrapper = toml::from_str(r#"pos = "before_user""#).unwrap();
        assert_eq!(w.pos, InjectionPosition::BeforeUser);

        let w: Wrapper = toml::from_str(r#"pos = "after_reset""#).unwrap();
        assert_eq!(w.pos, InjectionPosition::AfterReset);
    }

    #[test]
    fn test_session_config_with_start_script() {
        let toml_str = r#"
            on_session_start_script = "/usr/local/bin/on_start.sh"
        "#;
        let sc: SessionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            sc.on_session_start_script.as_deref(),
            Some("/usr/local/bin/on_start.sh")
        );
    }

    // ---- ResponseFormat tests ----

    #[test]
    fn test_response_format_default_is_text() {
        assert_eq!(ResponseFormat::default(), ResponseFormat::Text);
    }

    #[test]
    fn test_response_format_text_roundtrip() {
        let rf = ResponseFormat::Text;
        let json = serde_json::to_string(&rf).unwrap();
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ResponseFormat::Text);
    }

    #[test]
    fn test_response_format_json_roundtrip() {
        let rf = ResponseFormat::Json;
        let json = serde_json::to_string(&rf).unwrap();
        assert!(json.contains(r#""type":"json""#));
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ResponseFormat::Json);
    }

    #[test]
    fn test_response_format_json_schema_roundtrip() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        });
        let rf = ResponseFormat::JsonSchema {
            name: "person".to_string(),
            schema: schema.clone(),
            strict: Some(true),
        };
        let json = serde_json::to_string(&rf).unwrap();
        assert!(json.contains(r#""type":"json_schema""#));
        assert!(json.contains(r#""name":"person""#));
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rf);
    }

    #[test]
    fn test_response_format_json_schema_strict_none() {
        let rf = ResponseFormat::JsonSchema {
            name: "test".to_string(),
            schema: serde_json::json!({}),
            strict: None,
        };
        let json = serde_json::to_string(&rf).unwrap();
        let back: ResponseFormat = serde_json::from_str(&json).unwrap();
        match back {
            ResponseFormat::JsonSchema { strict, .. } => assert_eq!(strict, None),
            _ => panic!("Expected JsonSchema variant"),
        }
    }

    #[test]
    fn test_response_format_toml_roundtrip() {
        // Simulate a TOML config fragment for json_schema
        let toml_str = r#"
type = "json_schema"
name = "weather"
strict = true

[schema]
type = "object"

[schema.properties.temp]
type = "number"
"#;
        let rf: ResponseFormat = toml::from_str(toml_str).unwrap();
        match &rf {
            ResponseFormat::JsonSchema { name, strict, .. } => {
                assert_eq!(name, "weather");
                assert_eq!(*strict, Some(true));
            }
            _ => panic!("Expected JsonSchema variant"),
        }
    }

    /// Compile-time guard: KernelConfig::default() must survive a TOML
    /// serialize → deserialize → serialize roundtrip.  If a field is added
    /// to the struct but omitted from the `Default` impl (or vice-versa),
    /// this test will fail.
    #[test]
    fn test_kernel_config_default_roundtrip() {
        let original = KernelConfig::default();

        // Serialize to TOML.
        let toml_str =
            toml::to_string(&original).expect("KernelConfig::default() must serialize to TOML");

        // Deserialize back.
        let restored: KernelConfig =
            toml::from_str(&toml_str).expect("KernelConfig TOML roundtrip deserialization failed");

        // Serialize again and compare — both TOML strings must be identical.
        let toml_str2 = toml::to_string(&restored).expect("KernelConfig re-serialization failed");

        assert_eq!(
            toml_str, toml_str2,
            "KernelConfig default roundtrip mismatch — a field may be missing from Default impl"
        );
    }

    /// Per-provider budget TOML roundtrip (issue #2316).
    #[test]
    fn test_budget_config_per_provider_roundtrip() {
        let toml_str = r#"
max_hourly_usd = 0.0
max_daily_usd = 10.0
max_monthly_usd = 0.0
alert_threshold = 0.8
default_max_llm_tokens_per_hour = 0

[providers.moonshot]
max_cost_per_day_usd = 2.0
max_tokens_per_hour = 500000

[providers.litellm]
# all zeros -> unlimited
"#;
        let cfg: BudgetConfig = toml::from_str(toml_str).expect("parse budget TOML");
        assert_eq!(cfg.providers.len(), 2);

        let moonshot = cfg.providers.get("moonshot").expect("moonshot entry");
        assert!((moonshot.max_cost_per_day_usd - 2.0).abs() < f64::EPSILON);
        assert_eq!(moonshot.max_tokens_per_hour, 500_000);
        // Unset fields default to 0 (unlimited).
        assert_eq!(moonshot.max_cost_per_hour_usd, 0.0);
        assert_eq!(moonshot.max_cost_per_month_usd, 0.0);

        let litellm = cfg.providers.get("litellm").expect("litellm entry");
        assert_eq!(*litellm, ProviderBudget::default());

        // Round-trip: serialize then re-parse, structs should match.
        let reserialized = toml::to_string(&cfg).expect("serialize budget");
        let cfg2: BudgetConfig = toml::from_str(&reserialized).expect("reparse budget");
        assert_eq!(cfg2.providers, cfg.providers);
    }

    #[test]
    fn test_budget_config_default_has_empty_providers() {
        let b = BudgetConfig::default();
        assert!(b.providers.is_empty());
        // An empty providers map must not appear in serialized output so that
        // users who never configured per-provider caps see a clean config.
        let s = toml::to_string(&b).expect("serialize");
        assert!(
            !s.contains("providers"),
            "empty providers map should be skipped: {s}"
        );
    }

    // ---- TerminalConfig tmux fields tests ----

    #[test]
    fn test_terminal_config_tmux_defaults() {
        let tc = TerminalConfig::default();
        assert!(tc.tmux_enabled, "tmux_enabled should default to true");
        assert_eq!(tc.max_windows, 16, "max_windows should default to 16");
        assert!(
            tc.tmux_binary_path.is_none(),
            "tmux_binary_path should default to None"
        );
    }

    #[test]
    fn test_terminal_config_empty_toml_uses_defaults() {
        let tc: TerminalConfig = toml::from_str("").unwrap();
        assert!(tc.tmux_enabled);
        assert_eq!(tc.max_windows, 16);
        assert!(tc.tmux_binary_path.is_none());
    }

    #[test]
    fn test_terminal_config_toml_roundtrip() {
        let toml_str = r#"
            tmux_enabled = false
            max_windows = 4
            tmux_binary_path = "/usr/bin/tmux"
        "#;
        let tc: TerminalConfig = toml::from_str(toml_str).unwrap();
        assert!(!tc.tmux_enabled);
        assert_eq!(tc.max_windows, 4);
        assert_eq!(tc.tmux_binary_path.as_deref(), Some("/usr/bin/tmux"));
    }

    // ---- ToolInvokeConfig tests ----

    #[test]
    fn test_tool_invoke_config_default_is_fail_closed() {
        let c = ToolInvokeConfig::default();
        assert!(!c.enabled, "tool_invoke must be disabled by default");
        assert!(c.allowlist.is_empty());
        assert!(
            !c.permits("web_search"),
            "default config must deny every tool"
        );
    }

    #[test]
    fn test_tool_invoke_config_enabled_without_allowlist_denies_all() {
        let c = ToolInvokeConfig {
            enabled: true,
            allowlist: Vec::new(),
        };
        assert!(
            !c.permits("web_search"),
            "empty allowlist denies all even when enabled"
        );
    }

    #[test]
    fn test_tool_invoke_config_allowlist_without_enabled_denies_all() {
        let c = ToolInvokeConfig {
            enabled: false,
            allowlist: vec!["web_search".to_string()],
        };
        assert!(
            !c.permits("web_search"),
            "disabled endpoint denies all regardless of allowlist"
        );
    }

    #[test]
    fn test_tool_invoke_config_exact_match_and_glob() {
        let c = ToolInvokeConfig {
            enabled: true,
            allowlist: vec!["web_search".to_string(), "file_*".to_string()],
        };
        assert!(c.permits("web_search"));
        assert!(c.permits("file_read"));
        assert!(c.permits("file_write"));
        assert!(!c.permits("shell_exec"));
        assert!(!c.permits("web_fetch"));
    }

    #[test]
    fn test_kernel_config_includes_tool_invoke_default() {
        let cfg = KernelConfig::default();
        assert!(!cfg.tool_invoke.enabled);
        assert!(cfg.tool_invoke.allowlist.is_empty());
    }

    #[test]
    fn test_tool_invoke_config_empty_toml_uses_defaults() {
        let c: ToolInvokeConfig = toml::from_str("").unwrap();
        assert!(!c.enabled);
        assert!(c.allowlist.is_empty());
    }

    #[test]
    fn test_tool_invoke_config_toml_roundtrip() {
        let toml_str = r#"
            enabled = true
            allowlist = ["web_search", "file_*"]
        "#;
        let c: ToolInvokeConfig = toml::from_str(toml_str).unwrap();
        assert!(c.enabled);
        assert_eq!(c.allowlist, vec!["web_search", "file_*"]);

        let back = toml::to_string(&c).unwrap();
        let again: ToolInvokeConfig = toml::from_str(&back).unwrap();
        assert_eq!(c, again);
    }

    // -------- ParallelToolsConfig (PR-3 schema only) --------

    #[test]
    fn parallel_tools_default_is_disabled() {
        let c = ParallelToolsConfig::default();
        assert!(!c.enabled, "PR-3 must ship with the dispatcher off");
        assert_eq!(c.max_concurrent, 4);
        assert_eq!(c.mcp_default_safety, "write_shared");
        assert!(c.mcp_readonly_allowlist.is_empty());

        // KernelConfig::default() must wire the field through.
        let cfg = KernelConfig::default();
        assert_eq!(cfg.parallel_tools, ParallelToolsConfig::default());
    }

    #[test]
    fn parallel_tools_serde_round_trip() {
        let original = ParallelToolsConfig {
            enabled: true,
            max_concurrent: 8,
            mcp_default_safety: "read_only".to_string(),
            mcp_readonly_allowlist: vec![
                "mcp__github__list_issues".to_string(),
                "mcp__fs__read_file".to_string(),
            ],
        };

        // TOML round-trip.
        let toml_str = toml::to_string(&original).unwrap();
        let from_toml: ParallelToolsConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(from_toml, original);

        // JSON round-trip.
        let json_str = serde_json::to_string(&original).unwrap();
        let from_json: ParallelToolsConfig = serde_json::from_str(&json_str).unwrap();
        assert_eq!(from_json, original);
    }

    #[test]
    fn parallel_tools_missing_in_kernel_config_uses_default() {
        // Old config.toml predating PR-3 has no [parallel_tools] section.
        // KernelConfig deserialisation must hydrate the field with Default.
        let toml_str = r#"
            log_level = "info"
            api_listen = "0.0.0.0:4545"
        "#;
        let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.parallel_tools, ParallelToolsConfig::default());
        assert!(!cfg.parallel_tools.enabled);
    }

    #[test]
    fn parallel_tools_partial_section_fills_remaining_with_default() {
        // User supplies only `enabled = true`; remaining fields fall back
        // to Default — verifies #[serde(default)] on the struct itself.
        let toml_str = r#"
            enabled = true
        "#;
        let c: ParallelToolsConfig = toml::from_str(toml_str).unwrap();
        assert!(c.enabled);
        assert_eq!(c.max_concurrent, 4);
        assert_eq!(c.mcp_default_safety, "write_shared");
        assert!(c.mcp_readonly_allowlist.is_empty());
    }

    // ── Issue #3050: granular MCP taint policy ─────────────────────────────

    #[test]
    fn mcp_taint_tool_policy_default_is_scan_and_omits_optional_fields() {
        // Backward compat: a bare `[tool_policy.tools.foo]` table must
        // deserialise into `default = Scan`, no paths, no rule_sets.
        let toml_str = "default = \"scan\"\n";
        let policy: McpTaintToolPolicy = toml::from_str(toml_str).unwrap();
        assert_eq!(policy.default, McpTaintToolAction::Scan);
        assert!(policy.paths.is_empty());
        assert!(policy.rule_sets.is_empty());

        let empty: McpTaintToolPolicy = toml::from_str("").unwrap();
        assert_eq!(empty.default, McpTaintToolAction::Scan);
    }

    #[test]
    fn mcp_taint_tool_action_skip_round_trips() {
        let mut tools = HashMap::new();
        tools.insert(
            "navigate".to_string(),
            McpTaintToolPolicy {
                default: McpTaintToolAction::Skip,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        let json = serde_json::to_string(&policy).unwrap();
        let back: McpTaintPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.tools.get("navigate").unwrap().default,
            McpTaintToolAction::Skip
        );

        // TOML round-trip — primary surface for operators.
        let toml_str = toml::to_string(&policy).unwrap();
        let back_toml: McpTaintPolicy = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            back_toml.tools.get("navigate").unwrap().default,
            McpTaintToolAction::Skip
        );
    }

    #[test]
    fn mcp_taint_path_policy_round_trips_with_wildcards() {
        let mut paths = HashMap::new();
        paths.insert(
            "$.metadata.*".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![crate::taint::TaintRuleId::SensitiveKeyName],
            },
        );
        paths.insert(
            "$.items[*]".to_string(),
            McpTaintPathPolicy {
                skip_rules: vec![crate::taint::TaintRuleId::OpaqueToken],
            },
        );
        let mut tools = HashMap::new();
        tools.insert(
            "read_file".to_string(),
            McpTaintToolPolicy {
                paths,
                ..Default::default()
            },
        );
        let policy = McpTaintPolicy { tools };

        let toml_str = toml::to_string(&policy).unwrap();
        let back: McpTaintPolicy = toml::from_str(&toml_str).unwrap();
        let read_paths = &back.tools.get("read_file").unwrap().paths;
        assert!(read_paths.contains_key("$.metadata.*"));
        assert!(read_paths.contains_key("$.items[*]"));
    }

    #[test]
    fn mcp_taint_rule_set_actions_round_trip() {
        // Inline TOML covers all three severity tiers.
        let toml_str = r#"
[[taint_rules]]
name = "browser_handles"
action = "warn"
rules = ["opaque_token"]

[[taint_rules]]
name = "pii_baseline"
action = "log"
rules = ["pii_email", "pii_phone"]

[[taint_rules]]
name = "strict_default"
rules = ["authorization_literal"]
"#;
        let cfg: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.taint_rules.len(), 3);
        assert_eq!(cfg.taint_rules[0].name, "browser_handles");
        assert_eq!(cfg.taint_rules[0].action, McpTaintRuleSetAction::Warn);
        assert_eq!(cfg.taint_rules[1].action, McpTaintRuleSetAction::Log);
        // Default action when omitted: Block.
        assert_eq!(cfg.taint_rules[2].action, McpTaintRuleSetAction::Block);
    }

    #[test]
    fn tool_policy_rule_sets_reference_round_trips() {
        // `McpTaintPolicy` has a single field `tools` — the test deserialises
        // the policy directly, not the surrounding `[mcp_servers.<name>.taint_policy]`
        // table. Using the un-prefixed `[tools.<name>]` shape keeps the test
        // focused on the policy struct.
        let toml_str = r#"
[tools.navigate]
default = "skip"
rule_sets = ["browser_handles"]

[tools.read_file]
rule_sets = ["browser_handles", "pii_baseline"]

[tools.read_file.paths]
"$.content" = { skip_rules = ["opaque_token"] }
"#;
        let policy: McpTaintPolicy = toml::from_str(toml_str).unwrap();
        let nav = policy.tools.get("navigate").unwrap();
        assert_eq!(nav.default, McpTaintToolAction::Skip);
        assert_eq!(nav.rule_sets, vec!["browser_handles"]);

        let rf = policy.tools.get("read_file").unwrap();
        assert_eq!(rf.default, McpTaintToolAction::Scan);
        assert_eq!(rf.rule_sets.len(), 2);
        assert!(rf.paths.contains_key("$.content"));
    }

    #[test]
    fn legacy_taint_policy_without_new_fields_still_loads() {
        // Pre-issue #3050 config.toml shape — must continue to deserialise
        // identically with `default = Scan`, empty `rule_sets`.
        let toml_str = r#"
[tools.navigate.paths]
"$.tabId" = { skip_rules = ["opaque_token"] }
"#;
        let policy: McpTaintPolicy = toml::from_str(toml_str).unwrap();
        let nav = policy.tools.get("navigate").unwrap();
        assert_eq!(nav.default, McpTaintToolAction::Scan);
        assert!(nav.rule_sets.is_empty());
        assert_eq!(nav.paths.len(), 1);
    }

    // Issue #3136 follow-up: PR #3170 made the bundled observability stack
    // opt-in but left `otlp_endpoint` defaulting to localhost:4317, so default
    // installs spammed `ConnectionRefused`. `otlp_export_disabled()` is the
    // gate that suppresses the exporter when no collector is reachable. The
    // gate takes the runtime fact `stack_running` rather than just the config
    // intent — `auto_start_observability_stack = true` only matters when the
    // stack actually came up, otherwise we'd still spam.
    #[test]
    fn otlp_export_disabled_for_default_localhost_without_managed_stack() {
        let cfg = TelemetryConfig::default();
        assert!(cfg.enabled, "default still enables tracing wiring");
        assert!(
            cfg.otlp_export_disabled(false),
            "default localhost endpoint with no running stack must skip exporter"
        );
    }

    #[test]
    fn otlp_export_enabled_when_managed_stack_runs() {
        let cfg = TelemetryConfig {
            auto_start_observability_stack: true,
            ..TelemetryConfig::default()
        };
        assert!(
            !cfg.otlp_export_disabled(true),
            "running stack on default endpoint; export must run"
        );
    }

    // Regression: operator opts in to auto_start but Docker is missing /
    // compose fails / port conflicts — without this gate, exporter would
    // still init and spam ConnectionRefused on every export interval.
    #[test]
    fn otlp_export_disabled_when_managed_stack_failed_to_start() {
        let cfg = TelemetryConfig {
            auto_start_observability_stack: true,
            ..TelemetryConfig::default()
        };
        assert!(
            cfg.otlp_export_disabled(false),
            "auto_start=true but stack startup failed; default endpoint is dead"
        );
    }

    #[test]
    fn otlp_export_enabled_for_custom_endpoint() {
        let cfg = TelemetryConfig {
            otlp_endpoint: "http://otel.internal:4317".to_string(),
            ..TelemetryConfig::default()
        };
        assert!(
            !cfg.otlp_export_disabled(false),
            "explicit non-default endpoint signals operator intent regardless of stack"
        );
    }

    #[test]
    fn otlp_export_disabled_for_empty_endpoint() {
        let cfg = TelemetryConfig {
            otlp_endpoint: String::new(),
            ..TelemetryConfig::default()
        };
        assert!(
            cfg.otlp_export_disabled(true),
            "empty endpoint is the explicit opt-out path even when stack is up"
        );
    }

    // ----- BudgetConfig::default_burst_ratio parse-time validation -----

    #[test]
    fn default_burst_ratio_accepts_zero_and_unit_range() {
        for v in [0.0_f32, 0.01, 0.2, 0.5, 1.0] {
            let toml_str = format!("default_burst_ratio = {v}");
            let cfg: BudgetConfig = toml::from_str(&toml_str)
                .unwrap_or_else(|e| panic!("expected accept for {v}: {e}"));
            assert!((cfg.default_burst_ratio - v).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn default_burst_ratio_rejects_negative_at_parse_time() {
        let err = toml::from_str::<BudgetConfig>("default_burst_ratio = -0.5")
            .expect_err("negative ratio must be rejected at parse time");
        let msg = err.to_string();
        assert!(
            msg.contains("default_burst_ratio") && msg.contains("[0.0, 1.0]"),
            "error must explain the constraint, got: {msg}"
        );
    }

    #[test]
    fn default_burst_ratio_rejects_above_one_at_parse_time() {
        let err = toml::from_str::<BudgetConfig>("default_burst_ratio = 2.5")
            .expect_err("ratio > 1.0 must be rejected at parse time");
        assert!(err.to_string().contains("default_burst_ratio"));
    }

    #[test]
    fn default_burst_ratio_rejects_nan_at_parse_time() {
        let err = toml::from_str::<BudgetConfig>("default_burst_ratio = nan")
            .expect_err("NaN must be rejected at parse time");
        let msg = err.to_string();
        assert!(
            msg.contains("default_burst_ratio") && msg.contains("finite"),
            "error must explain the constraint, got: {msg}"
        );
    }

    #[test]
    fn default_burst_ratio_rejects_infinity_at_parse_time() {
        let err = toml::from_str::<BudgetConfig>("default_burst_ratio = inf")
            .expect_err("infinity must be rejected at parse time");
        assert!(err.to_string().contains("finite"));
    }

    #[test]
    fn default_burst_ratio_defaults_to_zero_when_missing() {
        let cfg: BudgetConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.default_burst_ratio, 0.0);
    }
}
