//! LibreFang Hands — curated autonomous capability packages.
//!
//! A Hand is a pre-built, domain-complete agent configuration that users activate
//! from a marketplace. Unlike regular agents (you chat with them), Hands work for
//! you (you check in on them).

pub mod registry;

use chrono::{DateTime, Utc};
use librefang_types::agent::{AgentId, AgentManifest, AutonomousConfig, ModelConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use uuid::Uuid;

// ─── Error types ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum HandError {
    #[error("Hand not found: {0}")]
    NotFound(String),
    #[error("Hand already active: {0}")]
    AlreadyActive(String),
    #[error("Hand instance not found: {0}")]
    InstanceNotFound(Uuid),
    #[error("Activation failed: {0}")]
    ActivationFailed(String),
    #[error("TOML parse error: {0}")]
    TomlParse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Config error: {0}")]
    Config(String),
}

pub type HandResult<T> = Result<T, HandError>;

// ─── Core types ──────────────────────────────────────────────────────────────

/// Category of a Hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandCategory {
    Content,
    Security,
    Productivity,
    Development,
    Communication,
    Data,
    Finance,
    #[serde(other)]
    Other,
}

impl std::fmt::Display for HandCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Content => write!(f, "Content"),
            Self::Security => write!(f, "Security"),
            Self::Productivity => write!(f, "Productivity"),
            Self::Development => write!(f, "Development"),
            Self::Communication => write!(f, "Communication"),
            Self::Data => write!(f, "Data"),
            Self::Finance => write!(f, "Finance"),
            Self::Other => write!(f, "Other"),
        }
    }
}

/// Type of requirement check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementType {
    /// A binary must exist on PATH.
    Binary,
    /// An environment variable must be set.
    EnvVar,
    /// An API key env var must be set.
    ApiKey,
    /// Any one of several env vars must be set (comma-separated in check_value).
    AnyEnvVar,
}

/// Platform-specific install commands and guides for a requirement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandInstallInfo {
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
    #[serde(default)]
    pub linux_apt: Option<String>,
    #[serde(default)]
    pub linux_dnf: Option<String>,
    #[serde(default)]
    pub linux_pacman: Option<String>,
    #[serde(default)]
    pub pip: Option<String>,
    #[serde(default)]
    pub signup_url: Option<String>,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub env_example: Option<String>,
    #[serde(default)]
    pub manual_url: Option<String>,
    #[serde(default)]
    pub estimated_time: Option<String>,
    #[serde(default)]
    pub steps: Vec<String>,
}

/// A single requirement the user must satisfy to use a Hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandRequirement {
    /// Unique key for this requirement.
    pub key: String,
    /// Human-readable label.
    pub label: String,
    /// What kind of check to perform.
    pub requirement_type: RequirementType,
    /// The value to check (binary name, env var name, etc.).
    pub check_value: String,
    /// Human-readable description of why this is needed.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this requirement is optional (non-critical).
    ///
    /// Optional requirements do not block activation. When an active hand has
    /// unmet optional requirements it is reported as "degraded" rather than
    /// "requirements not met".
    #[serde(default)]
    pub optional: bool,
    /// Platform-specific installation instructions.
    #[serde(default)]
    pub install: Option<HandInstallInfo>,
}

/// A metric displayed on the Hand dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandMetric {
    /// Display label.
    pub label: String,
    /// Memory key to read from agent's structured memory.
    pub memory_key: String,
    /// Display format (e.g. "number", "duration", "bytes").
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "number".to_string()
}

// ─── Hand settings types ────────────────────────────────────────────────────

/// Type of a hand setting control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandSettingType {
    Select,
    Text,
    Toggle,
}

/// A single option within a Select-type setting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandSettingOption {
    pub value: String,
    pub label: String,
    /// Env var to check for "Ready" badge (e.g. `GROQ_API_KEY`).
    #[serde(default)]
    pub provider_env: Option<String>,
    /// Binary to check on PATH for "Ready" badge (e.g. `whisper`).
    #[serde(default)]
    pub binary: Option<String>,
}

/// A configurable setting declared in HAND.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandSetting {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub setting_type: HandSettingType,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub options: Vec<HandSettingOption>,
    /// Env var name to expose when a text-type setting has a value
    /// (e.g. `ELEVENLABS_API_KEY` for an API key text field).
    #[serde(default)]
    pub env_var: Option<String>,
}

/// Result of resolving user-chosen settings against the schema.
pub struct ResolvedSettings {
    /// Markdown block to append to the system prompt (e.g. `## User Configuration\n- STT: Groq...`).
    pub prompt_block: String,
    /// Env var names the agent's subprocess should have access to.
    pub env_vars: Vec<String>,
}

/// Resolve user config values against a hand's settings schema.
///
/// For each setting, looks up the user's choice in `config` (falling back to
/// `setting.default`). For Select-type settings, finds the matching option and
/// collects its `provider_env` if present. Builds a prompt block summarising
/// the user's configuration.
pub fn resolve_settings(
    settings: &[HandSetting],
    config: &HashMap<String, serde_json::Value>,
) -> ResolvedSettings {
    let mut lines: Vec<String> = Vec::new();
    let mut env_vars: Vec<String> = Vec::new();

    for setting in settings {
        let chosen_value = config
            .get(&setting.key)
            .and_then(|v| v.as_str())
            .unwrap_or(&setting.default);

        match setting.setting_type {
            HandSettingType::Select => {
                let matched = setting.options.iter().find(|o| o.value == chosen_value);
                let display = matched.map(|o| o.label.as_str()).unwrap_or(chosen_value);
                lines.push(format!(
                    "- {}: {} ({})",
                    setting.label, display, chosen_value
                ));

                if let Some(opt) = matched {
                    if let Some(ref env) = opt.provider_env {
                        env_vars.push(env.clone());
                    }
                }
            }
            HandSettingType::Toggle => {
                let enabled = chosen_value == "true" || chosen_value == "1";
                lines.push(format!(
                    "- {}: {}",
                    setting.label,
                    if enabled { "Enabled" } else { "Disabled" }
                ));
            }
            HandSettingType::Text => {
                if !chosen_value.is_empty() {
                    lines.push(format!("- {}: {}", setting.label, chosen_value));
                    if let Some(ref env) = setting.env_var {
                        env_vars.push(env.clone());
                    }
                }
            }
        }
    }

    let prompt_block = if lines.is_empty() {
        String::new()
    } else {
        format!("## User Configuration\n\n{}", lines.join("\n"))
    };

    ResolvedSettings {
        prompt_block,
        env_vars,
    }
}

/// Dashboard schema for a Hand's metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandDashboard {
    pub metrics: Vec<HandMetric>,
}

/// Legacy flat agent config from HAND.toml — kept for backward compatibility.
///
/// New HAND.toml files can use the full `[agent]` / `[agent.model]` nested format
/// from `AgentManifest`.  Legacy files with flat fields (provider, model, max_tokens,
/// temperature, system_prompt at the top level of `[agent]`) are auto-converted.
#[derive(Debug, Clone, Deserialize)]
struct LegacyHandAgentConfig {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_module")]
    module: String,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
    #[serde(default = "default_temperature")]
    temperature: f32,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    max_iterations: Option<u32>,
}

impl From<LegacyHandAgentConfig> for AgentManifest {
    fn from(legacy: LegacyHandAgentConfig) -> Self {
        AgentManifest {
            name: legacy.name,
            description: legacy.description,
            module: legacy.module,
            model: ModelConfig {
                provider: legacy.provider,
                model: legacy.model,
                max_tokens: legacy.max_tokens,
                temperature: legacy.temperature,
                system_prompt: legacy.system_prompt,
                api_key_env: legacy.api_key_env,
                base_url: legacy.base_url,
            },
            autonomous: legacy.max_iterations.map(|max_iter| AutonomousConfig {
                max_iterations: max_iter,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

/// A single agent within a multi-agent hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandAgentManifest {
    /// Whether this agent is the coordinator (receives user messages).
    /// If no agent is marked coordinator, the first agent (sorted by role) is used.
    #[serde(default)]
    pub coordinator: bool,
    /// Hint for other agents on when/how to invoke this agent.
    /// Injected into the coordinator's system prompt as a dispatch guide.
    #[serde(default)]
    pub invoke_hint: Option<String>,
    /// Reference to an agent template from the agents/ registry.
    /// If set, the template's AgentManifest is loaded as a base,
    /// and fields explicitly set in this hand agent override it.
    #[serde(default)]
    pub base: Option<String>,
    /// The underlying agent manifest (flattened so TOML fields sit alongside).
    #[serde(flatten)]
    pub manifest: AgentManifest,
}

/// Parse a single `[agent]` section (toml::Value) into an AgentManifest.
fn parse_single_agent_section(value: &toml::Value) -> Result<AgentManifest, String> {
    // Deserialize directly from toml::Value (avoid to_string() which produces
    // inline table format that toml::from_str cannot parse).
    //
    // Heuristic: if [agent] contains a `model` sub-table (nested ModelConfig),
    // parse as full AgentManifest. Otherwise parse as legacy flat format where
    // `provider`, `model`, `system_prompt` etc. are top-level fields.
    let has_model_table = value
        .as_table()
        .and_then(|t| t.get("model"))
        .map(|v| v.is_table())
        .unwrap_or(false);

    if has_model_table {
        AgentManifest::deserialize(value.clone())
            .or_else(|_| LegacyHandAgentConfig::deserialize(value.clone()).map(AgentManifest::from))
            .map_err(|e| format!("Failed to parse [agent] section: {e}"))
    } else {
        LegacyHandAgentConfig::deserialize(value.clone())
            .map(AgentManifest::from)
            .or_else(|_| AgentManifest::deserialize(value.clone()))
            .map_err(|e| format!("Failed to parse [agent] section: {e}"))
    }
}

fn default_version() -> String {
    "0.0.0".to_string()
}

/// Deep-merge two TOML values. `overlay` fields win over `base` fields.
/// Tables are merged recursively; scalars and arrays in overlay replace base.
fn deep_merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                if let Some(base_value) = base_table.get_mut(key) {
                    deep_merge_toml(base_value, value);
                } else {
                    base_table.insert(key.clone(), value.clone());
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Parse a single entry from `[agents.*]` into a `HandAgentManifest`.
///
/// Extracts `coordinator`, `invoke_hint`, and `base` from the raw TOML value.
/// If `base` is set, loads the referenced agent template from the agents
/// registry directory and deep-merges the hand's overrides on top.
/// This gives multi-agent entries the same legacy flat-field fallback that
/// single-agent `[agent]` already has.
pub(crate) fn parse_multi_agent_entry(
    role: &str,
    value: &toml::Value,
    agents_dir: Option<&std::path::Path>,
) -> Result<HandAgentManifest, String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("[agents.{role}] must be a table"))?;

    let coordinator = table
        .get("coordinator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let invoke_hint = table
        .get("invoke_hint")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let base_ref = table
        .get("base")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let manifest = if let Some(ref template_name) = base_ref {
        // Validate template name: must be a simple directory name without path
        // separators or parent-directory references to prevent path traversal.
        if template_name.contains("..")
            || template_name.contains('/')
            || template_name.contains('\\')
        {
            return Err(format!(
                "[agents.{role}]: invalid base template name '{template_name}': \
                 must be a simple name without path separators"
            ));
        }
        // Load the base agent template and merge hand overrides on top.
        let agents_dir = agents_dir.ok_or_else(|| {
            format!(
                "[agents.{role}]: `base = \"{template_name}\"` requires agents registry directory"
            )
        })?;
        let template_path = agents_dir.join(template_name).join("agent.toml");
        let template_toml = std::fs::read_to_string(&template_path).map_err(|e| {
            format!(
                "[agents.{role}]: failed to read base template '{}': {e}",
                template_path.display()
            )
        })?;
        let mut base_value: toml::Value = toml::from_str(&template_toml).map_err(|e| {
            format!(
                "[agents.{role}]: failed to parse base template '{}': {e}",
                template_path.display()
            )
        })?;

        // Deep-merge: hand agent fields override base template fields.
        // Remove hand-only fields before merge (they're not part of AgentManifest).
        let mut overlay = value.clone();
        if let Some(t) = overlay.as_table_mut() {
            t.remove("coordinator");
            t.remove("invoke_hint");
            t.remove("base");
        }
        deep_merge_toml(&mut base_value, &overlay);

        parse_single_agent_section(&base_value)
            .map_err(|e| format!("[agents.{role}] (merged with base '{template_name}'): {e}"))?
    } else {
        parse_single_agent_section(value).map_err(|e| format!("[agents.{role}]: {e}"))?
    };

    Ok(HandAgentManifest {
        coordinator,
        invoke_hint,
        base: base_ref,
        manifest,
    })
}

fn default_module() -> String {
    "builtin:chat".to_string()
}
fn default_provider() -> String {
    "anthropic".to_string()
}
fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}
fn default_max_tokens() -> u32 {
    4096
}
fn default_temperature() -> f32 {
    0.7
}

/// Localized label/description for a single setting (optional).
///
/// If omitted, the original English label/description from `[[settings]]` is used.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandSettingI18n {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Localized strings for a single agent within a Hand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandAgentI18n {
    /// Localized agent display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Localized agent description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Localized strings for a Hand definition.
///
/// All fields are optional — HAND.toml files work without any `[i18n.*]` section.
/// When present, only the provided fields override their English defaults.
///
/// ## Example (HAND.toml)
///
/// ```toml
/// [i18n.zh]
/// name = "线索生成 Hand"
/// description = "自主线索生成"
///
/// # Optional: translate individual agent names/descriptions.
/// [i18n.zh.agents.main]
/// name = "主协调器"
/// description = "协调各个子智能体完成任务"
///
/// # Optional: translate individual settings. Omit to keep English labels.
/// [i18n.zh.settings.target_industry]
/// label = "目标行业"
/// description = "聚焦的行业领域"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandI18n {
    /// Localized name.
    #[serde(default)]
    pub name: Option<String>,
    /// Localized description.
    #[serde(default)]
    pub description: Option<String>,
    /// Localized category display name.
    #[serde(default)]
    pub category: Option<String>,
    /// Localized agent names/descriptions, keyed by role name.
    /// Optional — agents without translations fall back to English.
    #[serde(default)]
    pub agents: HashMap<String, HandAgentI18n>,
    /// Localized setting labels/descriptions, keyed by setting key.
    /// Optional — settings without translations fall back to English.
    #[serde(default)]
    pub settings: HashMap<String, HandSettingI18n>,
}

/// Complete Hand definition — parsed from HAND.toml.
///
/// Supports two agent formats:
/// - **Single-agent** (`[agent]`): auto-converted to `{"main": ...}` with coordinator=true
/// - **Multi-agent** (`[agents.role1]`, `[agents.role2]`, ...): each role gets its own agent
#[derive(Debug, Clone, Serialize)]
pub struct HandDefinition {
    /// Unique hand identifier (e.g. "clip").
    pub id: String,
    /// Semantic version from HAND.toml (e.g. "1.2.0"). Defaults to "0.0.0".
    #[serde(default = "default_version")]
    pub version: String,
    /// Human-readable name.
    pub name: String,
    /// What this Hand does.
    pub description: String,
    /// Category for marketplace browsing.
    pub category: HandCategory,
    /// Icon (emoji).
    pub icon: String,
    /// Tools all agents need access to.
    pub tools: Vec<String>,
    /// Skill allowlist for the spawned agents (empty = all).
    pub skills: Vec<String>,
    /// MCP server allowlist for the spawned agents (empty = all).
    pub mcp_servers: Vec<String>,
    /// Plugin allowlist for the spawned agents (empty = all).
    #[serde(default)]
    pub allowed_plugins: Vec<String>,
    /// Requirements that must be satisfied before activation.
    pub requires: Vec<HandRequirement>,
    /// Configurable settings (shown in activation modal).
    pub settings: Vec<HandSetting>,
    /// Agent manifests keyed by role name.
    /// Single-agent hands are stored as `{"main": ...}`.
    pub agents: BTreeMap<String, HandAgentManifest>,
    /// Dashboard metrics schema.
    pub dashboard: HandDashboard,
    /// Routing keywords for hand selection.
    pub routing: HandRouting,
    /// Bundled skill content (populated at load time, not in TOML).
    /// Shared across all agents unless overridden by `agent_skill_content`.
    #[serde(skip)]
    pub skill_content: Option<String>,
    /// Per-role skill content overrides, keyed by role name (e.g. "pm", "qa").
    /// Populated from `SKILL-{role}.md` files at load time.
    /// When present for a role, takes precedence over the shared `skill_content`.
    #[serde(skip)]
    pub agent_skill_content: HashMap<String, String>,
    /// Token consumption and activation metadata.
    pub metadata: Option<HandMetadata>,
    /// Localized strings keyed by language code (e.g. "zh", "ja").
    pub i18n: HashMap<String, HandI18n>,
}

impl HandDefinition {
    /// Get the coordinator agent manifest (the one that receives user messages).
    /// Falls back to the first agent by role name if none is marked coordinator.
    pub fn coordinator(&self) -> Option<(&str, &HandAgentManifest)> {
        // Explicit coordinator
        for (role, agent) in &self.agents {
            if agent.coordinator {
                return Some((role, agent));
            }
        }
        // Fallback: first entry (BTreeMap is sorted)
        self.agents.iter().next().map(|(r, a)| (r.as_str(), a))
    }

    /// Backward-compatible accessor: returns the single/coordinator agent manifest.
    ///
    /// Returns `None` if the hand has no agents (should never happen in practice).
    pub fn agent(&self) -> Option<&AgentManifest> {
        self.coordinator().map(|(_, a)| &a.manifest)
    }

    /// Whether this hand has multiple agents.
    pub fn is_multi_agent(&self) -> bool {
        self.agents.len() > 1
    }
}

/// Raw intermediate struct for TOML deserialization — supports both formats.
#[derive(Deserialize)]
struct HandDefinitionRaw {
    id: String,
    #[serde(default = "default_version")]
    version: String,
    name: String,
    description: String,
    category: HandCategory,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<String>,
    #[serde(default)]
    allowed_plugins: Vec<String>,
    #[serde(default)]
    requires: Vec<HandRequirement>,
    #[serde(default)]
    settings: Vec<HandSetting>,
    /// Single-agent format: `[agent]`
    #[serde(default)]
    agent: Option<toml::Value>,
    /// Multi-agent format: `[agents.role1]`, `[agents.role2]`, ...
    /// Deserialized as raw TOML values so we can apply legacy fallback per entry.
    #[serde(default)]
    agents: Option<BTreeMap<String, toml::Value>>,
    #[serde(default)]
    dashboard: HandDashboard,
    #[serde(default)]
    routing: HandRouting,
    #[serde(default)]
    metadata: Option<HandMetadata>,
    #[serde(default)]
    i18n: HashMap<String, HandI18n>,
}

impl<'de> Deserialize<'de> for HandDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = HandDefinitionRaw::deserialize(deserializer)?;

        let agents = if let Some(raw_agents) = raw.agents {
            // Multi-agent format: [agents.*] — parse each entry with legacy fallback
            if raw_agents.is_empty() {
                return Err(serde::de::Error::custom(
                    "Hand must define at least one agent in [agents.*]",
                ));
            }
            let mut agents_map = BTreeMap::new();
            for (role, value) in &raw_agents {
                let agent =
                    parse_multi_agent_entry(role, value, None).map_err(serde::de::Error::custom)?;
                agents_map.insert(role.clone(), agent);
            }
            agents_map
        } else if let Some(agent_value) = raw.agent {
            // Single-agent format: [agent] → convert to {"main": ...}
            let manifest =
                parse_single_agent_section(&agent_value).map_err(serde::de::Error::custom)?;
            let mut map = BTreeMap::new();
            map.insert(
                "main".to_string(),
                HandAgentManifest {
                    coordinator: true,
                    invoke_hint: None,
                    base: None,
                    manifest,
                },
            );
            map
        } else {
            return Err(serde::de::Error::custom(
                "Hand must define either [agent] or [agents.*]",
            ));
        };

        Ok(HandDefinition {
            id: raw.id,
            version: raw.version,
            name: raw.name,
            description: raw.description,
            category: raw.category,
            icon: raw.icon,
            tools: raw.tools,
            skills: raw.skills,
            mcp_servers: raw.mcp_servers,
            allowed_plugins: raw.allowed_plugins,
            requires: raw.requires,
            settings: raw.settings,
            agents,
            dashboard: raw.dashboard,
            routing: raw.routing,
            skill_content: None,
            agent_skill_content: HashMap::new(),
            metadata: raw.metadata,
            i18n: raw.i18n,
        })
    }
}

/// Parse a HAND.toml string into a `HandDefinition`, resolving `base` agent
/// templates when `agents_dir` is provided.
///
/// This bypasses the `Deserialize` impl (which cannot do filesystem I/O) and
/// manually constructs agents via `parse_multi_agent_entry` so that base
/// template resolution has access to the agents registry directory.
pub(crate) fn parse_hand_definition(
    toml_content: &str,
    agents_dir: Option<&Path>,
) -> Result<HandDefinition, String> {
    let raw: HandDefinitionRaw =
        toml::from_str(toml_content).map_err(|e| format!("Failed to parse HAND.toml: {e}"))?;

    let agents = if let Some(raw_agents) = raw.agents {
        if raw_agents.is_empty() {
            return Err("Hand must define at least one agent in [agents.*]".to_string());
        }
        let mut agents_map = BTreeMap::new();
        for (role, value) in &raw_agents {
            let agent = parse_multi_agent_entry(role, value, agents_dir)?;
            agents_map.insert(role.clone(), agent);
        }
        agents_map
    } else if let Some(agent_value) = raw.agent {
        let manifest = parse_single_agent_section(&agent_value)?;
        let mut map = BTreeMap::new();
        map.insert(
            "main".to_string(),
            HandAgentManifest {
                coordinator: true,
                invoke_hint: None,
                base: None,
                manifest,
            },
        );
        map
    } else {
        return Err("Hand must define either [agent] or [agents.*]".to_string());
    };

    Ok(HandDefinition {
        id: raw.id,
        version: raw.version,
        name: raw.name,
        description: raw.description,
        category: raw.category,
        icon: raw.icon,
        tools: raw.tools,
        skills: raw.skills,
        mcp_servers: raw.mcp_servers,
        allowed_plugins: raw.allowed_plugins,
        requires: raw.requires,
        settings: raw.settings,
        agents,
        dashboard: raw.dashboard,
        routing: raw.routing,
        skill_content: None,
        agent_skill_content: HashMap::new(),
        metadata: raw.metadata,
        i18n: raw.i18n,
    })
}

/// Token consumption and activation metadata for user awareness.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandMetadata {
    /// How often the hand runs: continuous, periodic, on_demand
    #[serde(default)]
    pub frequency: String,
    /// Relative token consumption: low, medium, high
    #[serde(default)]
    pub token_consumption: String,
    /// Whether this hand is included in default activation
    #[serde(default)]
    pub default_active: bool,
    /// Warning message shown when activating
    #[serde(default)]
    pub activation_warning: String,
}

/// Routing keywords for deterministic hand selection.
///
/// Keywords are English-only. Cross-lingual matching is handled by
/// semantic embedding fallback, not by translating keywords.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandRouting {
    /// Strong aliases — high-confidence intent signals (score ×3 each).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Weak aliases — supporting signals (score ×1 each).
    #[serde(default)]
    pub weak_aliases: Vec<String>,
}

/// Runtime status of a Hand instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandStatus {
    Active,
    Paused,
    Error(String),
    Inactive,
}

impl std::fmt::Display for HandStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Paused => write!(f, "Paused"),
            Self::Error(msg) => write!(f, "Error: {msg}"),
            Self::Inactive => write!(f, "Inactive"),
        }
    }
}

/// A running Hand instance — links a HandDefinition to its spawned agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandInstance {
    /// Unique instance identifier.
    pub instance_id: Uuid,
    /// Which hand definition this is an instance of.
    pub hand_id: String,
    /// Current status.
    pub status: HandStatus,
    /// Spawned agents keyed by role name → AgentId.
    /// Empty until agents are spawned by the kernel.
    #[serde(default)]
    pub agent_ids: BTreeMap<String, AgentId>,
    /// Role name of the coordinator agent that receives user messages.
    /// Persisted explicitly so runtime routes do not have to guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator_role: Option<String>,
    /// User-provided configuration overrides.
    pub config: HashMap<String, serde_json::Value>,
    /// When activated.
    pub activated_at: DateTime<Utc>,
    /// Last status change.
    pub updated_at: DateTime<Utc>,
}

impl HandInstance {
    /// Create a new pending instance.
    ///
    /// If `instance_id` is `Some`, that UUID is reused (e.g. when restoring
    /// a persisted instance across daemon restarts).  Otherwise a fresh
    /// random UUID is generated.
    pub fn new(
        hand_id: &str,
        config: HashMap<String, serde_json::Value>,
        instance_id: Option<Uuid>,
    ) -> Self {
        let now = Utc::now();
        Self {
            instance_id: instance_id.unwrap_or_else(Uuid::new_v4),
            hand_id: hand_id.to_string(),
            status: HandStatus::Active,
            agent_ids: BTreeMap::new(),
            coordinator_role: None,
            config,
            activated_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn normalize_coordinator_role(
        agent_ids: &BTreeMap<String, AgentId>,
        coordinator_role: Option<&str>,
    ) -> Option<String> {
        if let Some(role) = coordinator_role.filter(|role| agent_ids.contains_key(*role)) {
            return Some(role.to_string());
        }
        if agent_ids.len() == 1 {
            return agent_ids.keys().next().cloned();
        }
        if agent_ids.contains_key("main") {
            return Some("main".to_string());
        }
        agent_ids.keys().next().cloned()
    }

    /// Get the coordinator role name (if agents have been spawned).
    pub fn coordinator_role(&self) -> Option<String> {
        Self::normalize_coordinator_role(&self.agent_ids, self.coordinator_role.as_deref())
    }

    /// Get the coordinator agent ID (if agents have been spawned).
    pub fn coordinator_agent_id(&self) -> Option<AgentId> {
        self.coordinator_role()
            .and_then(|role| self.agent_ids.get(&role).copied())
    }

    /// Backward-compatible accessor: returns the single/first agent ID.
    pub fn agent_id(&self) -> Option<AgentId> {
        self.coordinator_agent_id()
    }

    /// Backward-compatible accessor: returns the agent name (coordinator role).
    pub fn agent_name(&self) -> String {
        self.coordinator_role().unwrap_or_default()
    }
}

/// Request to activate a hand.
#[derive(Debug, Deserialize)]
pub struct ActivateHandRequest {
    /// Optional configuration overrides.
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_category_display() {
        assert_eq!(HandCategory::Content.to_string(), "Content");
        assert_eq!(HandCategory::Security.to_string(), "Security");
        assert_eq!(HandCategory::Data.to_string(), "Data");
    }

    #[test]
    fn hand_status_display() {
        assert_eq!(HandStatus::Active.to_string(), "Active");
        assert_eq!(HandStatus::Paused.to_string(), "Paused");
        assert_eq!(
            HandStatus::Error("ffmpeg not found".to_string()).to_string(),
            "Error: ffmpeg not found"
        );
    }

    #[test]
    fn hand_instance_new() {
        let instance = HandInstance::new("clip", HashMap::new(), None);
        assert_eq!(instance.hand_id, "clip");
        assert_eq!(instance.status, HandStatus::Active);
        assert!(instance.agent_ids.is_empty());
        assert!(instance.coordinator_role.is_none());
    }

    #[test]
    fn hand_instance_prefers_explicit_coordinator_role() {
        let mut instance = HandInstance::new("research", HashMap::new(), None);
        instance
            .agent_ids
            .insert("analyst".to_string(), AgentId::new());
        let planner_id = AgentId::new();
        instance.agent_ids.insert("planner".to_string(), planner_id);
        instance.coordinator_role = Some("planner".to_string());

        assert_eq!(instance.coordinator_role(), Some("planner".to_string()));
        assert_eq!(instance.coordinator_agent_id(), Some(planner_id));
        assert_eq!(instance.agent_name(), "planner");
    }

    #[test]
    fn hand_error_display() {
        let err = HandError::NotFound("clip".to_string());
        assert!(err.to_string().contains("clip"));

        let err = HandError::AlreadyActive("clip".to_string());
        assert!(err.to_string().contains("already"));
    }

    #[test]
    fn hand_definition_roundtrip() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test hand"
category = "content"
icon = "T"
tools = ["shell_exec"]

[[requires]]
key = "test_bin"
label = "test must be installed"
requirement_type = "binary"
check_value = "test"

[agent]
name = "test-hand"
description = "Test agent"
system_prompt = "You are a test agent."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.id, "test");
        assert_eq!(def.category, HandCategory::Content);
        assert_eq!(def.requires.len(), 1);
        assert_eq!(def.agent().unwrap().name, "test-hand");
    }

    #[test]
    fn hand_definition_with_settings() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test"
category = "content"
tools = []

[[settings]]
key = "stt_provider"
label = "STT Provider"
description = "Speech-to-text engine"
setting_type = "select"
default = "auto"

[[settings.options]]
value = "auto"
label = "Auto-detect"

[[settings.options]]
value = "groq"
label = "Groq Whisper"
provider_env = "GROQ_API_KEY"

[[settings.options]]
value = "local"
label = "Local Whisper"
binary = "whisper"

[agent]
name = "test-hand"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.settings.len(), 1);
        assert_eq!(def.settings[0].key, "stt_provider");
        assert_eq!(def.settings[0].setting_type, HandSettingType::Select);
        assert_eq!(def.settings[0].options.len(), 3);
        assert_eq!(
            def.settings[0].options[1].provider_env.as_deref(),
            Some("GROQ_API_KEY")
        );
        assert_eq!(
            def.settings[0].options[2].binary.as_deref(),
            Some("whisper")
        );
    }

    #[test]
    fn resolve_settings_with_config() {
        let settings = vec![HandSetting {
            key: "stt".to_string(),
            label: "STT Provider".to_string(),
            description: String::new(),
            setting_type: HandSettingType::Select,
            default: "auto".to_string(),
            options: vec![
                HandSettingOption {
                    value: "auto".to_string(),
                    label: "Auto".to_string(),
                    provider_env: None,
                    binary: None,
                },
                HandSettingOption {
                    value: "groq".to_string(),
                    label: "Groq Whisper".to_string(),
                    provider_env: Some("GROQ_API_KEY".to_string()),
                    binary: None,
                },
                HandSettingOption {
                    value: "openai".to_string(),
                    label: "OpenAI Whisper".to_string(),
                    provider_env: Some("OPENAI_API_KEY".to_string()),
                    binary: None,
                },
            ],
            env_var: None,
        }];

        // User picks groq
        let mut config = HashMap::new();
        config.insert("stt".to_string(), serde_json::json!("groq"));
        let resolved = resolve_settings(&settings, &config);
        assert!(resolved.prompt_block.contains("STT Provider"));
        assert!(resolved.prompt_block.contains("Groq Whisper"));
        assert_eq!(resolved.env_vars, vec!["GROQ_API_KEY"]);
    }

    #[test]
    fn resolve_settings_defaults() {
        let settings = vec![HandSetting {
            key: "stt".to_string(),
            label: "STT".to_string(),
            description: String::new(),
            setting_type: HandSettingType::Select,
            default: "auto".to_string(),
            options: vec![
                HandSettingOption {
                    value: "auto".to_string(),
                    label: "Auto".to_string(),
                    provider_env: None,
                    binary: None,
                },
                HandSettingOption {
                    value: "groq".to_string(),
                    label: "Groq".to_string(),
                    provider_env: Some("GROQ_API_KEY".to_string()),
                    binary: None,
                },
            ],
            env_var: None,
        }];

        // Empty config → uses default "auto"
        let resolved = resolve_settings(&settings, &HashMap::new());
        assert!(resolved.prompt_block.contains("Auto"));
        assert!(
            resolved.env_vars.is_empty(),
            "only selected option env var should be collected"
        );
    }

    #[test]
    fn resolve_settings_toggle_and_text() {
        let settings = vec![
            HandSetting {
                key: "tts_enabled".to_string(),
                label: "TTS".to_string(),
                description: String::new(),
                setting_type: HandSettingType::Toggle,
                default: "false".to_string(),
                options: vec![],
                env_var: None,
            },
            HandSetting {
                key: "custom_model".to_string(),
                label: "Model".to_string(),
                description: String::new(),
                setting_type: HandSettingType::Text,
                default: String::new(),
                options: vec![],
                env_var: None,
            },
        ];

        let mut config = HashMap::new();
        config.insert("tts_enabled".to_string(), serde_json::json!("true"));
        config.insert("custom_model".to_string(), serde_json::json!("large-v3"));
        let resolved = resolve_settings(&settings, &config);
        assert!(resolved.prompt_block.contains("Enabled"));
        assert!(resolved.prompt_block.contains("large-v3"));
    }

    #[test]
    fn hand_requirement_with_install_info() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test hand"
category = "content"
tools = []

[[requires]]
key = "ffmpeg"
label = "FFmpeg must be installed"
requirement_type = "binary"
check_value = "ffmpeg"
description = "FFmpeg is the core video processing engine."

[requires.install]
macos = "brew install ffmpeg"
windows = "winget install Gyan.FFmpeg"
linux_apt = "sudo apt install ffmpeg"
linux_dnf = "sudo dnf install ffmpeg-free"
linux_pacman = "sudo pacman -S ffmpeg"
manual_url = "https://ffmpeg.org/download.html"
estimated_time = "2-5 min"

[agent]
name = "test-hand"
description = "Test agent"
system_prompt = "You are a test agent."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.requires.len(), 1);
        let req = &def.requires[0];
        assert_eq!(
            req.description.as_deref(),
            Some("FFmpeg is the core video processing engine.")
        );
        let install = req.install.as_ref().unwrap();
        assert_eq!(install.macos.as_deref(), Some("brew install ffmpeg"));
        assert_eq!(
            install.windows.as_deref(),
            Some("winget install Gyan.FFmpeg")
        );
        assert_eq!(
            install.linux_apt.as_deref(),
            Some("sudo apt install ffmpeg")
        );
        assert_eq!(
            install.linux_dnf.as_deref(),
            Some("sudo dnf install ffmpeg-free")
        );
        assert_eq!(
            install.linux_pacman.as_deref(),
            Some("sudo pacman -S ffmpeg")
        );
        assert_eq!(
            install.manual_url.as_deref(),
            Some("https://ffmpeg.org/download.html")
        );
        assert_eq!(install.estimated_time.as_deref(), Some("2-5 min"));
        assert!(install.pip.is_none());
        assert!(install.signup_url.is_none());
        assert!(install.steps.is_empty());
    }

    #[test]
    fn hand_requirement_without_install_info_backward_compat() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test"
category = "content"
tools = []

[[requires]]
key = "test_bin"
label = "test must be installed"
requirement_type = "binary"
check_value = "test"

[agent]
name = "test-hand"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.requires.len(), 1);
        assert!(def.requires[0].description.is_none());
        assert!(def.requires[0].install.is_none());
    }

    #[test]
    fn multi_agent_flat_model_format() {
        let toml_str = r#"
id = "research"
version = "2.0.0"
name = "Research Hand"
description = "Multi-agent research"
category = "content"
tools = []

[agents.planner]
coordinator = true
invoke_hint = "Use planner for task decomposition"
name = "planner-agent"
description = "Plans research tasks"
model = "default"
system_prompt = "You plan research."

[agents.analyst]
name = "analyst-agent"
description = "Analyzes data"
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You analyze data."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.id, "research");
        assert_eq!(def.version, "2.0.0");
        assert!(def.is_multi_agent());
        assert_eq!(def.agents.len(), 2);

        let (coord_role, coord) = def.coordinator().unwrap();
        assert_eq!(coord_role, "planner");
        assert!(coord.coordinator);
        assert_eq!(
            coord.invoke_hint.as_deref(),
            Some("Use planner for task decomposition")
        );
        assert_eq!(coord.manifest.name, "planner-agent");

        let analyst = &def.agents["analyst"];
        assert!(!analyst.coordinator);
        assert_eq!(analyst.manifest.name, "analyst-agent");
        assert_eq!(analyst.manifest.model.provider, "groq");
        assert_eq!(analyst.manifest.model.model, "llama-3.3-70b-versatile");
    }

    #[test]
    fn multi_agent_nested_model_format() {
        let toml_str = r#"
id = "research"
name = "Research Hand"
description = "Multi-agent research"
category = "content"
tools = []

[agents.planner]
coordinator = true
name = "planner-agent"
description = "Plans research tasks"

[agents.planner.model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
max_tokens = 8192
temperature = 0.5
system_prompt = "You plan research."

[agents.analyst]
name = "analyst-agent"
description = "Analyzes data"

[agents.analyst.model]
provider = "groq"
model = "llama-3.3-70b-versatile"
max_tokens = 4096
temperature = 0.3
system_prompt = "You analyze data."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.agents.len(), 2);

        let planner = &def.agents["planner"];
        assert!(planner.coordinator);
        assert_eq!(planner.manifest.model.provider, "anthropic");
        assert_eq!(planner.manifest.model.max_tokens, 8192);

        let analyst = &def.agents["analyst"];
        assert_eq!(analyst.manifest.model.provider, "groq");
        assert_eq!(analyst.manifest.model.temperature, 0.3);
    }

    #[test]
    fn hand_version_defaults_to_zero() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test"
category = "content"
tools = []

[agent]
name = "test-hand"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.version, "0.0.0");
    }

    #[test]
    fn api_key_requirement_with_steps() {
        let toml_str = r#"
id = "test"
name = "Test Hand"
description = "A test"
category = "communication"
tools = []

[[requires]]
key = "API_TOKEN"
label = "API Token"
requirement_type = "api_key"
check_value = "API_TOKEN"
description = "A token from the service."

[requires.install]
signup_url = "https://example.com/signup"
docs_url = "https://example.com/docs"
env_example = "API_TOKEN=your_token_here"
estimated_time = "5-10 min"
steps = [
    "Go to example.com and sign up",
    "Navigate to API settings",
    "Generate a new token",
    "Set it as an environment variable",
]

[agent]
name = "test-hand"
description = "Test"
system_prompt = "Test."

[dashboard]
metrics = []
"#;
        let def: HandDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(def.requires.len(), 1);
        let req = &def.requires[0];
        let install = req.install.as_ref().unwrap();
        assert_eq!(
            install.signup_url.as_deref(),
            Some("https://example.com/signup")
        );
        assert_eq!(
            install.docs_url.as_deref(),
            Some("https://example.com/docs")
        );
        assert_eq!(
            install.env_example.as_deref(),
            Some("API_TOKEN=your_token_here")
        );
        assert_eq!(install.estimated_time.as_deref(), Some("5-10 min"));
        assert_eq!(install.steps.len(), 4);
        assert_eq!(install.steps[0], "Go to example.com and sign up");
        assert!(install.macos.is_none());
        assert!(install.windows.is_none());
    }

    #[test]
    fn base_template_reuse() {
        // Set up a temporary agents registry directory with a template.
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        let template_dir = agents_dir.join("my-writer");
        std::fs::create_dir_all(&template_dir).unwrap();
        std::fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "writer-base"
description = "A base writer agent"
module = "builtin:chat"

[model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
max_tokens = 4096
temperature = 0.7
system_prompt = "You are a writer."
"#,
        )
        .unwrap();

        let hand_toml = r#"
id = "content-hand"
name = "Content Hand"
description = "Creates content using a writer template"
category = "content"
tools = []

[agents.writer]
coordinator = true
base = "my-writer"
# Override just the system prompt — everything else comes from the base.

[agents.writer.model]
system_prompt = "You are a blog post writer."

[dashboard]
metrics = []
"#;

        let def = parse_hand_definition(hand_toml, Some(agents_dir.as_path())).unwrap();
        assert_eq!(def.id, "content-hand");
        assert_eq!(def.agents.len(), 1);

        let writer = &def.agents["writer"];
        assert!(writer.coordinator);
        assert_eq!(writer.base.as_deref(), Some("my-writer"));
        // Name comes from the base template.
        assert_eq!(writer.manifest.name, "writer-base");
        // Provider and model come from base.
        assert_eq!(writer.manifest.model.provider, "anthropic");
        assert_eq!(writer.manifest.model.model, "claude-sonnet-4-20250514");
        assert_eq!(writer.manifest.model.max_tokens, 4096);
        // System prompt is overridden by the hand.
        assert_eq!(
            writer.manifest.model.system_prompt,
            "You are a blog post writer."
        );
    }

    #[test]
    fn base_template_with_scalar_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        let template_dir = agents_dir.join("generic-chat");
        std::fs::create_dir_all(&template_dir).unwrap();
        std::fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "generic-chat"
description = "Generic chat agent"

[model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
max_tokens = 4096
temperature = 0.7
system_prompt = "You are a helpful assistant."
"#,
        )
        .unwrap();

        let hand_toml = r#"
id = "custom-hand"
name = "Custom Hand"
description = "A hand that overrides scalar fields"
category = "development"
tools = []

[agents.main]
coordinator = true
base = "generic-chat"
name = "custom-agent"
description = "Overridden description"

[agents.main.model]
provider = "groq"
model = "llama-3.3-70b-versatile"
temperature = 0.3

[dashboard]
metrics = []
"#;

        let def = parse_hand_definition(hand_toml, Some(agents_dir.as_path())).unwrap();
        let agent = &def.agents["main"];
        assert_eq!(agent.base.as_deref(), Some("generic-chat"));
        // Overridden fields.
        assert_eq!(agent.manifest.name, "custom-agent");
        assert_eq!(agent.manifest.description, "Overridden description");
        assert_eq!(agent.manifest.model.provider, "groq");
        assert_eq!(agent.manifest.model.model, "llama-3.3-70b-versatile");
        assert_eq!(agent.manifest.model.temperature, 0.3);
        // Preserved from base (not overridden).
        assert_eq!(agent.manifest.model.max_tokens, 4096);
        assert_eq!(
            agent.manifest.model.system_prompt,
            "You are a helpful assistant."
        );
    }

    #[test]
    fn base_template_missing_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        let hand_toml = r#"
id = "broken-hand"
name = "Broken Hand"
description = "References a non-existent template"
category = "content"
tools = []

[agents.main]
coordinator = true
base = "does-not-exist"

[dashboard]
metrics = []
"#;

        let result = parse_hand_definition(hand_toml, Some(agents_dir.as_path()));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("does-not-exist"),
            "Error should mention the missing template name: {err}"
        );
    }

    #[test]
    fn base_template_without_agents_dir_errors() {
        let hand_toml = r#"
id = "no-dir-hand"
name = "No Dir Hand"
description = "Uses base without agents_dir"
category = "content"
tools = []

[agents.main]
coordinator = true
base = "some-template"

[dashboard]
metrics = []
"#;

        // Without agents_dir, base resolution should fail.
        let result = parse_hand_definition(hand_toml, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("agents registry directory"),
            "Error should mention missing agents dir: {err}"
        );
    }

    #[test]
    fn no_base_backward_compatible() {
        // Agents without `base` should parse identically to before.
        let hand_toml = r#"
id = "plain-hand"
name = "Plain Hand"
description = "No base used"
category = "content"
tools = []

[agents.worker]
coordinator = true
name = "worker-agent"
description = "Just a worker"
system_prompt = "You work."

[dashboard]
metrics = []
"#;
        // Should succeed even without agents_dir.
        let def = parse_hand_definition(hand_toml, None).unwrap();
        assert_eq!(def.agents.len(), 1);
        let worker = &def.agents["worker"];
        assert!(worker.base.is_none());
        assert_eq!(worker.manifest.name, "worker-agent");
    }

    #[test]
    fn deep_merge_preserves_base_fields_and_overrides_hand_fields() {
        use toml::Value;

        let mut base = toml::from_str::<Value>(
            r#"
            name = "base-agent"
            description = "Base description"
            [model]
            provider = "anthropic"
            model = "claude"
            max_tokens = 4096
            system_prompt = "Base prompt"
            api_key_env = "BASE_KEY"
            [capabilities]
            network = ["api.example.com"]
            shell = ["cargo *"]
            tools = ["file_read"]
            "#,
        )
        .unwrap();

        let overlay = toml::from_str::<Value>(
            r#"
            name = "hand-agent"
            [model]
            max_tokens = 8192
            system_prompt = "Hand prompt"
            [capabilities]
            shell = ["npm *", "git *"]
            "#,
        )
        .unwrap();

        super::deep_merge_toml(&mut base, &overlay);

        let table = base.as_table().unwrap();
        // Hand overrides
        assert_eq!(table["name"].as_str().unwrap(), "hand-agent");
        let model = table["model"].as_table().unwrap();
        assert_eq!(model["max_tokens"].as_integer().unwrap(), 8192);
        assert_eq!(model["system_prompt"].as_str().unwrap(), "Hand prompt");
        // Base preserved (not in hand overlay)
        assert_eq!(table["description"].as_str().unwrap(), "Base description");
        assert_eq!(model["provider"].as_str().unwrap(), "anthropic");
        assert_eq!(model["model"].as_str().unwrap(), "claude");
        assert_eq!(model["api_key_env"].as_str().unwrap(), "BASE_KEY");
        // Capabilities: shell replaced by hand, network preserved from base
        let caps = table["capabilities"].as_table().unwrap();
        let shell: Vec<&str> = caps["shell"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(shell, vec!["npm *", "git *"]); // hand wins
        let network: Vec<&str> = caps["network"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(network, vec!["api.example.com"]); // base preserved
    }

    #[test]
    fn base_template_name_override() {
        // When hand sets name, it should override the base template's name
        let agents_dir = tempfile::tempdir().unwrap();
        let template_dir = agents_dir.path().join("my-template");
        std::fs::create_dir_all(&template_dir).unwrap();
        std::fs::write(
            template_dir.join("agent.toml"),
            r#"
name = "template-name"
description = "Template desc"
module = "builtin:chat"

[model]
provider = "default"
model = "default"
system_prompt = "Template prompt"
"#,
        )
        .unwrap();

        let hand_toml = r#"
id = "name-test"
name = "Name Test"
description = "Test name override"
category = "content"
tools = []

[agents.main]
coordinator = true
base = "my-template"
name = "custom-name"
description = "Custom description"

[agents.main.model]
system_prompt = "Custom prompt"

[dashboard]
metrics = []
"#;
        let def = parse_hand_definition(hand_toml, Some(agents_dir.path())).unwrap();
        let agent = &def.agents["main"];
        assert_eq!(agent.manifest.name, "custom-name");
        assert_eq!(agent.manifest.description, "Custom description");
        assert_eq!(agent.manifest.model.system_prompt, "Custom prompt");
        assert_eq!(agent.manifest.module, "builtin:chat"); // from base
    }
}
