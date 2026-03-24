//! All configuration struct and enum type definitions, including Default impls and associated helper functions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::serde_helpers::{deserialize_string_or_int_vec, OneOrMany};
use super::DEFAULT_API_LISTEN;

/// DM (direct message) policy for a channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Output format hint for channel-specific message formatting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Per-channel behavior overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelOverrides {
    /// Model override (uses agent's default if None).
    pub model: Option<String>,
    /// System prompt override.
    pub system_prompt: Option<String>,
    /// DM policy.
    pub dm_policy: DmPolicy,
    /// Group message policy.
    pub group_policy: GroupPolicy,
    /// Regex patterns that can trigger a reply in group chats when
    /// `group_policy` is `mention_only`.
    #[serde(default)]
    pub group_trigger_patterns: Vec<String>,
    /// Global rate limit for this channel (messages per minute, 0 = unlimited).
    pub rate_limit_per_minute: u32,
    /// Per-user rate limit (messages per minute, 0 = unlimited).
    pub rate_limit_per_user: u32,
    /// Enable thread replies.
    pub threading: bool,
    /// Output format override.
    pub output_format: Option<OutputFormat>,
    /// Usage footer mode override.
    pub usage_footer: Option<UsageFooterMode>,
    /// Typing indicator mode override.
    pub typing_mode: Option<TypingMode>,
}

/// Controls what usage info appears in response footers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

fn default_role() -> String {
    "user".to_string()
}

/// Web search provider selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    /// Brave Search API.
    Brave,
    /// Tavily AI-agent-native search.
    Tavily,
    /// Perplexity AI search.
    Perplexity,
    /// DuckDuckGo HTML (no API key needed).
    DuckDuckGo,
    /// Auto-select based on available API keys (Tavily → Brave → Perplexity → DuckDuckGo).
    #[default]
    Auto,
}

/// Web tools configuration (search + fetch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Which search provider to use.
    pub search_provider: SearchProvider,
    /// Cache TTL in minutes (0 = disabled).
    pub cache_ttl_minutes: u64,
    /// Brave Search configuration.
    pub brave: BraveSearchConfig,
    /// Tavily Search configuration.
    pub tavily: TavilySearchConfig,
    /// Perplexity Search configuration.
    pub perplexity: PerplexitySearchConfig,
    /// Web fetch configuration.
    pub fetch: WebFetchConfig,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: SearchProvider::default(),
            cache_ttl_minutes: 15,
            brave: BraveSearchConfig::default(),
            tavily: TavilySearchConfig::default(),
            perplexity: PerplexitySearchConfig::default(),
            fetch: WebFetchConfig::default(),
        }
    }
}

/// Brave Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Web fetch configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_chars: 50_000,
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            timeout_secs: 30,
            readability: true,
        }
    }
}

/// Browser automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
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
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            headless: true,
            viewport_width: 1280,
            viewport_height: 720,
            timeout_secs: 30,
            idle_timeout_secs: 300,
            max_sessions: 5,
            chromium_path: None,
        }
    }
}

/// Config hot-reload mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Webhook trigger authentication configuration.
///
/// Controls the `/hooks/wake` and `/hooks/agent` endpoints for external
/// systems to trigger agent actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Text-to-speech configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    /// Enable TTS. Default: false.
    pub enabled: bool,
    /// Default provider: "openai" or "elevenlabs".
    pub provider: Option<String>,
    /// OpenAI TTS settings.
    pub openai: TtsOpenAiConfig,
    /// ElevenLabs TTS settings.
    pub elevenlabs: TtsElevenLabsConfig,
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
            max_text_length: 4096,
            timeout_secs: 30,
        }
    }
}

/// OpenAI TTS settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Docker container sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PairingConfig {
    /// Enable device pairing. Default: false.
    pub enabled: bool,
    /// Max paired devices. Default: 10.
    pub max_devices: usize,
    /// Pairing token expiry in seconds. Default: 300 (5 min).
    pub token_expiry_secs: u64,
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
            push_provider: "none".to_string(),
            ntfy_url: None,
            ntfy_topic: None,
        }
    }
}

/// Extensions & integrations configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultConfig {
    /// Whether the vault is enabled (auto-detected if vault.enc exists).
    pub enabled: bool,
    /// Custom vault file path (default: ~/.librefang/vault.enc).
    pub path: Option<PathBuf>,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
        }
    }
}

/// Agent binding — routes specific channel/account/peer patterns to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    /// Target agent name or ID.
    pub agent: String,
    /// Match criteria (all specified fields must match).
    pub match_rule: BindingMatchRule,
}

/// Match rule for agent bindings. All specified (non-None) fields must match.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BroadcastConfig {
    /// Broadcast strategy.
    pub strategy: BroadcastStrategy,
    /// Map of peer_id -> list of agent names to receive the message.
    pub routes: HashMap<String, Vec<String>>,
}

/// Broadcast delivery strategy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BroadcastStrategy {
    /// Send to all agents simultaneously.
    #[default]
    Parallel,
    /// Send to agents one at a time in order.
    Sequential,
}

/// Auto-reply engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            otlp_endpoint: "http://localhost:4317".to_string(),
            service_name: "librefang".to_string(),
            sample_rate: 1.0,
            prometheus_enabled: true,
        }
    }
}

/// Canvas (Agent-to-UI) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecPolicy {
    /// Security mode: "deny" blocks all, "allowlist" only allows listed,
    /// "full" allows all (unsafe, dev only).
    pub mode: ExecSecurityMode,
    /// Commands that bypass allowlist (stdin-only utilities).
    pub safe_bins: Vec<String>,
    /// Global command allowlist (when mode = allowlist).
    pub allowed_commands: Vec<String>,
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
#[derive(Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

/// Where a context injection should be placed in the session message list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_depth_per_agent: 0,
            max_depth_global: 0,
            task_ttl_secs: 3600,
            concurrency: QueueConcurrencyConfig::default(),
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
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueConcurrencyConfig {
    /// Main lane concurrent limit (user messages).
    pub main_lane: usize,
    /// Cron lane concurrent limit (scheduled jobs).
    pub cron_lane: usize,
    /// Subagent lane concurrent limit (child agents).
    pub subagent_lane: usize,
}

impl Default for QueueConcurrencyConfig {
    fn default() -> Self {
        Self {
            main_lane: 3,
            cron_lane: 2,
            subagent_lane: 3,
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
#[derive(Clone, Default, Serialize, Deserialize)]
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

/// Top-level kernel configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KernelConfig {
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
    #[serde(default)]
    pub cors_origin: Vec<String>,
    /// Whether to enable the OFP network layer.
    pub network_enabled: bool,
    /// Default LLM provider configuration.
    pub default_model: DefaultModelConfig,
    /// Memory substrate configuration.
    pub memory: MemoryConfig,
    /// Network configuration.
    pub network: NetworkConfig,
    /// Channel bridge configuration (Telegram, etc.).
    pub channels: ChannelsConfig,
    /// API authentication key. When set, all API endpoints (except /api/health)
    /// require a `Authorization: Bearer <key>` header.
    /// If empty, the API is unauthenticated (local development only).
    pub api_key: String,
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
    #[serde(default)]
    pub users: Vec<UserConfig>,
    /// MCP server configurations for external tool integration.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfigEntry>,
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
    #[serde(default)]
    pub fallback_providers: Vec<FallbackProviderConfig>,
    /// Browser automation configuration.
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Extensions & integrations configuration.
    #[serde(default)]
    pub extensions: ExtensionsConfig,
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
    /// Execution approval policy.
    #[serde(default, alias = "approval_policy")]
    pub approval: crate::approval::ApprovalPolicy,
    /// Cron scheduler max total jobs across all agents. Default: 500.
    #[serde(default = "default_max_cron_jobs")]
    pub max_cron_jobs: usize,
    /// Config include files — loaded and deep-merged before the root config.
    /// Paths are relative to the root config file's directory.
    /// Security: absolute paths and `..` components are rejected.
    #[serde(default)]
    pub include: Vec<String>,
    /// Shell/exec security policy.
    #[serde(default)]
    pub exec_policy: ExecPolicy,
    /// Agent bindings for multi-account routing.
    #[serde(default)]
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
    /// Device pairing configuration.
    #[serde(default)]
    pub pairing: PairingConfig,
    /// Auth profiles for key rotation (provider name → profiles).
    #[serde(default)]
    pub auth_profiles: HashMap<String, Vec<AuthProfile>>,
    /// Extended thinking configuration.
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    /// Global spending budget configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Provider base URL overrides (provider ID → custom base URL).
    /// e.g. `ollama = "http://192.168.1.100:11434/v1"`
    #[serde(default)]
    pub provider_urls: HashMap<String, String>,
    /// Provider region selection (provider ID → region name).
    /// Selects a regional endpoint from the provider's `[provider.regions]` map.
    /// e.g. `qwen = "us"` to use the US endpoint instead of China mainland.
    #[serde(default)]
    pub provider_regions: HashMap<String, String>,
    /// Provider API key env var overrides (provider ID → env var name).
    /// For custom/unknown providers, maps the provider name to the environment
    /// variable holding the API key. e.g. `nvidia = "NVIDIA_API_KEY"`.
    /// If not set, the convention `{PROVIDER_UPPER}_API_KEY` is used automatically.
    #[serde(default)]
    pub provider_api_keys: HashMap<String, String>,
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
    #[serde(default)]
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
    /// Message queue configuration (depth limits, TTL, concurrency).
    #[serde(default)]
    pub queue: QueueConfig,
    /// External authentication provider configuration (OAuth2/OIDC).
    #[serde(default)]
    pub external_auth: ExternalAuthConfig,
    /// Tool policy configuration (global deny/allow rules, groups, depth limits).
    #[serde(default)]
    pub tool_policy: crate::tool_policy::ToolPolicy,
    /// Proactive memory (mem0-style) configuration.
    #[serde(default)]
    pub proactive_memory: crate::memory::ProactiveMemoryConfig,
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
    /// CLI update channel (stable, beta, rc).
    /// Controls which releases `librefang update` considers.
    #[serde(default)]
    pub update_channel: UpdateChannel,
}

/// Input sanitization mode for channel messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SanitizeMode {
    /// No checking — all messages pass through (default).
    #[default]
    Off,
    /// Log a warning but allow the message through.
    Warn,
    /// Reject the message and send an error to the user.
    Block,
}

/// Configuration for channel input sanitization / prompt-injection detection.
///
/// ```toml
/// [sanitize]
/// mode = "warn"           # off | warn | block
/// max_message_length = 32768
/// custom_block_patterns = ["(?i)secret\\s+code"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SanitizeConfig {
    /// Sanitization mode.
    pub mode: SanitizeMode,
    /// Maximum allowed message length in bytes (default: 32 768).
    pub max_message_length: usize,
    /// Additional regex patterns that should trigger a block/warn.
    pub custom_block_patterns: Vec<String>,
}

impl Default for SanitizeConfig {
    fn default() -> Self {
        Self {
            mode: SanitizeMode::Off,
            max_message_length: 32768,
            custom_block_patterns: Vec::new(),
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextEngineTomlConfig {
    /// Built-in engine name. Default: `"default"`.
    pub engine: String,
    /// Plugin name. Resolves to `~/.librefang/plugins/<name>/plugin.toml`.
    /// Takes precedence over manual `hooks` if set.
    pub plugin: Option<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Python script overrides for individual context engine lifecycle hooks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextEngineHooks {
    /// Python script for the `ingest` hook (called on new user message).
    /// Receives: `{"type": "ingest", "agent_id": "...", "message": "..."}`
    /// Returns: `{"type": "ingest_result", "memories": [{"content": "..."}]}`
    pub ingest: Option<String>,
    /// Python script for the `after_turn` hook (called after each turn).
    /// Receives: `{"type": "after_turn", "agent_id": "...", "messages": [...]}`
    /// Returns: `{"type": "ok"}` (acknowledgement)
    pub after_turn: Option<String>,
}

/// Plugin manifest — parsed from `~/.librefang/plugins/<name>/plugin.toml`.
///
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Python dependencies file (relative to plugin dir, default: `requirements.txt`).
    #[serde(default)]
    pub requirements: Option<String>,
}

/// client_secret_env = "GITHUB_OAUTH_CLIENT_SECRET"
/// scopes = ["read:user", "user:email"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Configuration for a single OIDC/OAuth2 provider.
///
/// Supports standard OIDC providers (Google, Azure AD, Keycloak) that use
/// `.well-known/openid-configuration` discovery, as well as non-OIDC OAuth2
/// providers (GitHub) where explicit `auth_url`, `token_url`, and `userinfo_url`
/// are specified.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Global spending budget configuration.
///
/// Set limits to 0.0 for unlimited. All limits apply across all agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_hourly_usd: 0.0,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
            default_max_llm_tokens_per_hour: 0,
        }
    }
}

fn default_max_cron_jobs() -> usize {
    500
}

/// Audit log configuration.
///
/// Configure in config.toml:
/// ```toml
/// [audit]
/// retention_days = 90
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    /// How many days to retain audit log entries. Default: 90. Set to 0 for unlimited.
    pub retention_days: u32,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self { retention_days: 90 }
    }
}

/// PII privacy mode for LLM context filtering.
///
/// Controls how personally identifiable information is handled before
/// messages are sent to LLM providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Plugin registry configuration.
///
/// Configure in config.toml:
/// ```toml
/// [plugins]
/// plugin_registries = ["librefang/plugin-registry"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    /// Additional GitHub `owner/repo` plugin registries to search.
    /// Merged with `context_engine.plugin_registries`.
    pub plugin_registries: Vec<String>,
}

fn default_prompt_caching() -> bool {
    true
}

/// Configuration entry for an MCP server.
///
/// This is the config.toml representation. The runtime `McpServerConfig`
/// struct is constructed from this during kernel boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfigEntry {
    /// Display name for this server.
    pub name: String,
    /// Transport configuration. Optional — entries without transport are skipped at boot.
    pub transport: Option<McpTransportEntry>,
    /// Request timeout in seconds.
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
    /// Environment variables to pass through (e.g., ["GITHUB_PERSONAL_ACCESS_TOKEN"]).
    #[serde(default)]
    pub env: Vec<String>,
}

fn default_mcp_timeout() -> u64 {
    30
}

fn default_http_compat_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object"})
}

/// HTTP request method for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HttpCompatRequestMode {
    #[default]
    JsonBody,
    Query,
    None,
}

/// How the built-in HTTP compatibility transport formats responses.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HttpCompatResponseMode {
    #[default]
    Json,
    Text,
}

/// Header injection config for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpCompatHeaderConfig {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub value_env: Option<String>,
}

/// Declarative tool mapping for the built-in HTTP compatibility transport.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Built-in compatibility adapter for plain HTTP/JSON tool backends.
    HttpCompat {
        base_url: String,
        #[serde(default)]
        headers: Vec<HttpCompatHeaderConfig>,
        #[serde(default)]
        tools: Vec<HttpCompatToolConfig>,
    },
}

/// A2A (Agent-to-Agent) protocol configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for KernelConfig {
    fn default() -> Self {
        let home_dir = librefang_home_dir();
        Self {
            data_dir: home_dir.join("data"),
            home_dir,
            log_level: "info".to_string(),
            api_listen: DEFAULT_API_LISTEN.to_string(),
            network_enabled: false,
            default_model: DefaultModelConfig::default(),
            memory: MemoryConfig::default(),
            network: NetworkConfig::default(),
            channels: ChannelsConfig::default(),
            api_key: String::new(),
            dashboard_user: String::new(),
            dashboard_pass: String::new(),
            dashboard_pass_hash: String::new(),
            mode: KernelMode::default(),
            language: "en".to_string(),
            users: Vec::new(),
            mcp_servers: Vec::new(),
            a2a: None,
            usage_footer: UsageFooterMode::default(),
            stable_prefix_mode: false,
            web: WebConfig::default(),
            fallback_providers: Vec::new(),
            browser: BrowserConfig::default(),
            extensions: ExtensionsConfig::default(),
            vault: VaultConfig::default(),
            workspaces_dir: None,
            log_dir: None,
            media: crate::media::MediaConfig::default(),
            links: crate::media::LinkConfig::default(),
            reload: ReloadConfig::default(),
            webhook_triggers: None,
            approval: crate::approval::ApprovalPolicy::default(),
            max_cron_jobs: default_max_cron_jobs(),
            include: Vec::new(),
            exec_policy: ExecPolicy::default(),
            bindings: Vec::new(),
            broadcast: BroadcastConfig::default(),
            auto_reply: AutoReplyConfig::default(),
            canvas: CanvasConfig::default(),
            tts: TtsConfig::default(),
            docker: DockerSandboxConfig::default(),
            pairing: PairingConfig::default(),
            auth_profiles: HashMap::new(),
            thinking: None,
            budget: BudgetConfig::default(),
            provider_urls: HashMap::new(),
            provider_regions: HashMap::new(),
            provider_api_keys: HashMap::new(),
            vertex_ai: VertexAiConfig::default(),
            azure_openai: AzureOpenAiConfig::default(),
            oauth: OAuthConfig::default(),
            sidecar_channels: Vec::new(),
            proxy: ProxyConfig::default(),
            prompt_caching: default_prompt_caching(),
            session: SessionConfig::default(),
            queue: QueueConfig::default(),
            external_auth: ExternalAuthConfig::default(),
            tool_policy: crate::tool_policy::ToolPolicy::default(),
            proactive_memory: crate::memory::ProactiveMemoryConfig::default(),
            context_engine: ContextEngineTomlConfig::default(),
            audit: AuditConfig::default(),
            health_check: HealthCheckConfig::default(),
            heartbeat: HeartbeatTomlConfig::default(),
            plugins: PluginsConfig::default(),
            cors_origin: Vec::new(),
            privacy: PrivacyConfig::default(),
            strict_config: false,
            qwen_code_path: None,
            sanitize: SanitizeConfig::default(),
            inbox: InboxConfig::default(),
            telemetry: TelemetryConfig::default(),
            update_channel: UpdateChannel::default(),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for DefaultModelConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: None,
        }
    }
}

/// Memory substrate configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Path to SQLite database file.
    pub sqlite_path: Option<PathBuf>,
    /// Embedding model for semantic search.
    pub embedding_model: String,
    /// Maximum memories before consolidation is triggered.
    pub consolidation_threshold: u64,
    /// Memory decay rate (0.0 = no decay, 1.0 = aggressive decay).
    pub decay_rate: f32,
    /// Embedding provider (e.g., "openai", "ollama"). None = auto-detect.
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
}

/// Configuration for splitting long documents into overlapping chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

/// Time-based memory decay configuration.
///
/// When enabled, memories that have not been accessed within their scope's TTL
/// are automatically deleted during periodic decay runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Clone, Serialize, Deserialize)]
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
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/0".to_string()],
            bootstrap_peers: vec![],
            mdns_enabled: true,
            max_peers: 50,
            shared_secret: String::new(),
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
            .finish()
    }
}

/// Channel bridge configuration.
///
/// Each field uses `OneOrMany<T>` to support both single-instance (`[channels.telegram]`)
/// and multi-instance (`[[channels.telegram]]`) TOML syntax for multi-bot routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// LinkedIn messaging configuration(s).
    pub linkedin: OneOrMany<LinkedInConfig>,
    /// WeChat personal account (iLink) configuration(s).
    pub wechat: OneOrMany<WeChatConfig>,
    /// WeCom/WeChat Work configuration(s).
    pub wecom: OneOrMany<WeComConfig>,
}

/// Telegram channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
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
            overrides: ChannelOverrides::default(),
            thread_routes: std::collections::HashMap::new(),
        }
    }
}

/// Discord channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Slack channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
            force_flat_replies: None,
        }
    }
}

/// WhatsApp Cloud API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Per-channel behavior overrides.
    #[serde(default)]
    pub overrides: ChannelOverrides,
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Signal channel adapter configuration (via signal-cli REST API).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Matrix protocol channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Email (IMAP/SMTP) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            poll_interval_secs: 30,
            folders: vec!["INBOX".to_string()],
            allowed_senders: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Microsoft Teams (Bot Framework v3) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamsConfig {
    /// Azure Bot App ID.
    pub app_id: String,
    /// Env var name holding the app password.
    pub app_password_env: String,
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
            webhook_port: 3978,
            allowed_tenants: vec![],
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Mattermost channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// IRC channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Google Chat channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MessengerConfig {
    /// Env var name holding the page access token.
    pub page_token_env: String,
    /// Env var name holding the webhook verify token.
    pub verify_token_env: String,
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
            webhook_port: 8452,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Reddit API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// WeCom/WeChat Work channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WeComConfig {
    /// WeCom corp ID.
    pub corp_id: String,
    /// WeCom application agent ID.
    pub agent_id: String,
    /// Env var name holding the application secret.
    pub secret_env: String,
    /// Port for the incoming webhook.
    pub webhook_port: u16,
    /// Env var name holding the callback verification token (optional).
    pub token_env: Option<String>,
    /// Env var name holding the encoding AES key (optional).
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
            corp_id: String::new(),
            agent_id: String::new(),
            secret_env: "WECOM_SECRET".to_string(),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            overrides: ChannelOverrides::default(),
        }
    }
}

/// Revolt (Discord-like) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RevoltConfig {
    /// Env var name holding the bot token.
    pub bot_token_env: String,
    /// Revolt API URL.
    pub api_url: String,
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
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

// ── Wave 4 channel configs ─────────────────────────────────────────

/// Nextcloud Talk channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// DingTalk Robot API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DingTalkConfig {
    /// Env var name holding the webhook access token.
    pub access_token_env: String,
    /// Env var name holding the signing secret.
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

impl Default for DingTalkConfig {
    fn default() -> Self {
        Self {
            access_token_env: "DINGTALK_ACCESS_TOKEN".to_string(),
            secret_env: "DINGTALK_SECRET".to_string(),
            webhook_port: 8457,
            account_id: None,
            default_agent: None,
            overrides: ChannelOverrides::default(),
        }
    }
}

/// QQ Bot API v2 channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

/// LinkedIn Messaging API channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
