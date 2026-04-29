//! OpenClaw workspace parser and migration engine.
//!
//! Real OpenClaw installations use a **single JSON5 config file** at
//! `~/.openclaw/openclaw.json` that contains everything: global config,
//! agents, channels, models, tools, cron, hooks, and more.
//!
//! ```text
//! ~/.openclaw/                          (or legacy: ~/.clawdbot, ~/.moldbot, ~/.moltbot)
//! ├── openclaw.json                     # JSON5 — THE config (everything lives here)
//! ├── auth-profiles.json                # Auth credentials
//! ├── sessions/                         # JSONL conversation logs per session key
//! │   ├── main.jsonl
//! │   └── agent:coder:main.jsonl
//! ├── memory/                           # Per-agent MEMORY.md files
//! │   ├── default/MEMORY.md
//! │   └── coder/MEMORY.md
//! ├── memory-search/                    # SQLite vector index
//! ├── skills/                           # Installed skills
//! ├── cron/                             # Cron run state
//! ├── hooks/                            # Webhook hook modules
//! └── workspaces/                       # Per-agent working directories
//! ```

use crate::report::{ItemKind, MigrateItem, MigrationReport, SkippedItem};
use crate::{MigrateError, MigrateOptions};
use librefang_types::config::{CONFIG_VERSION, DEFAULT_API_LISTEN};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Security helpers
// ---------------------------------------------------------------------------

/// #3794 — Reject ids that contain path-traversal components (e.g. `../`,
/// absolute paths, NUL bytes). Only ids consisting of a single normal path
/// component are accepted.
fn validate_migration_id(id: &str) -> Result<(), crate::MigrateError> {
    if id.is_empty() {
        return Err(crate::MigrateError::InvalidId("id is empty".to_string()));
    }
    if id.contains('\0') {
        return Err(crate::MigrateError::InvalidId(
            "id contains NUL byte".to_string(),
        ));
    }
    for component in std::path::Path::new(id).components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(crate::MigrateError::InvalidId(format!(
                    "id contains illegal path component: {id:?}"
                )))
            }
        }
    }
    Ok(())
}

/// #3798 — Write `content` to `path` atomically: write to a sibling `.tmp`
/// file first, then rename into place. Prevents torn writes from leaving a
/// half-written config file if the process is interrupted.
fn atomic_write(path: &std::path::Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Marker written after a successful migration; present = re-runs are no-ops.
const MIGRATION_MARKER_FILENAME: &str = ".openclaw_migrated";

/// Renames `path` to a `.bak.<timestamp>` sibling; returns the new path or `None` if absent.
fn backup_existing(path: &std::path::Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let original = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let backup_name = format!("{original}.bak.{stamp}");
    let backup_path = path.with_file_name(backup_name);
    // If by some collision the backup already exists, fall back to nanosecond
    // precision so we never silently drop the previous backup.
    let backup_path = if backup_path.exists() {
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        path.with_file_name(format!("{original}.bak.{stamp}.{nanos}"))
    } else {
        backup_path
    };
    std::fs::rename(path, &backup_path)?;
    Ok(Some(backup_path))
}

/// Backs up any existing `dest` to a `.bak.*` sibling then atomically writes `content`.
fn write_with_backup(
    dest: &std::path::Path,
    content: &str,
    report: &mut MigrationReport,
) -> std::io::Result<()> {
    if let Some(backup) = backup_existing(dest)? {
        warn!(
            "Backed up existing {} -> {} before overwriting",
            dest.display(),
            backup.display()
        );
        report.warnings.push(format!(
            "Existing {} was backed up to {} before overwrite",
            dest.display(),
            backup.display()
        ));
    }
    atomic_write(dest, content)
}

// ---------------------------------------------------------------------------
// OpenClaw JSON5 input types
// ---------------------------------------------------------------------------

/// Schema versions this migrator can handle.
const SUPPORTED_OPENCLAW_VERSIONS: &[u32] = &[1, 2];

/// Top-level openclaw.json structure.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawRoot {
    /// #3797 — schema/format version declared by openclaw.json.
    #[serde(alias = "schemaVersion")]
    version: Option<u32>,
    auth: Option<OpenClawAuth>,
    models: Option<OpenClawModels>,
    agents: Option<OpenClawAgents>,
    tools: Option<OpenClawRootTools>,
    channels: Option<OpenClawChannels>,
    cron: Option<serde_json::Value>,
    hooks: Option<serde_json::Value>,
    skills: Option<OpenClawSkills>,
    memory: Option<serde_json::Value>,
    session: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAuth {
    profiles: Option<serde_json::Value>,
    order: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawModels {
    providers: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawRootTools {
    #[allow(dead_code)]
    profile: Option<serde_json::Value>,
    #[allow(dead_code)]
    allow: Option<serde_json::Value>,
    #[allow(dead_code)]
    deny: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAgents {
    defaults: Option<OpenClawAgentDefaults>,
    list: Vec<OpenClawAgentEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAgentDefaults {
    model: Option<OpenClawAgentModel>,
    workspace: Option<String>,
    tools: Option<OpenClawAgentTools>,
    identity: Option<OpenClawIdentity>,
}

/// Agent model reference — either `"provider/model"` or `{ primary, fallbacks }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OpenClawAgentModel {
    Simple(String),
    Detailed(OpenClawAgentModelDetailed),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAgentModelDetailed {
    primary: Option<String>,
    fallbacks: Vec<String>,
}

/// Agent identity/system prompt reference — either a raw string or a structured object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OpenClawIdentity {
    Text(String),
    Structured(serde_json::Value),
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAgentEntry {
    id: String,
    name: Option<String>,
    model: Option<OpenClawAgentModel>,
    tools: Option<OpenClawAgentTools>,
    workspace: Option<String>,
    skills: Option<serde_json::Value>,
    identity: Option<OpenClawIdentity>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawAgentTools {
    profile: Option<serde_json::Value>,
    allow: Option<serde_json::Value>,
    deny: Option<serde_json::Value>,
    also_allow: Option<serde_json::Value>,
}

/// Extract a profile name from a Value (string or {name: "..."}  object).
fn extract_profile(val: &serde_json::Value) -> Option<String> {
    val.as_str().map(|s| s.to_string()).or_else(|| {
        val.get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

/// Extract a list of strings from a Value (array of strings, single string, or object keys).
fn extract_string_list(val: &serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect(),
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        _ => vec![],
    }
}

/// Extract a prompt string from OpenClaw's `identity` field.
///
/// Recent OpenClaw configs may store identity as a structured object instead of a
/// raw string. We accept both and look for common prompt-bearing keys without
/// failing the whole migration when the shape differs.
fn extract_identity_prompt(identity: &OpenClawIdentity) -> Option<String> {
    match identity {
        OpenClawIdentity::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        OpenClawIdentity::Structured(value) => extract_identity_prompt_value(value),
    }
}

fn extract_identity_prompt_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(extract_identity_prompt_value)
                .collect();
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "systemPrompt",
                "system_prompt",
                "prompt",
                "instructions",
                "instruction",
                "content",
                "text",
                "value",
                "persona",
                "identity",
                "description",
            ] {
                if let Some(prompt) = map.get(key).and_then(extract_identity_prompt_value) {
                    return Some(prompt);
                }
            }

            for nested in map.values().filter(|v| v.is_object() || v.is_array()) {
                if let Some(prompt) = extract_identity_prompt_value(nested) {
                    return Some(prompt);
                }
            }

            None
        }
        _ => None,
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawChannels {
    telegram: Option<OpenClawTelegramConfig>,
    discord: Option<OpenClawDiscordConfig>,
    slack: Option<OpenClawSlackConfig>,
    whatsapp: Option<OpenClawWhatsAppConfig>,
    signal: Option<OpenClawSignalConfig>,
    matrix: Option<OpenClawMatrixConfig>,
    #[serde(alias = "googlechat", alias = "googleChat")]
    google_chat: Option<OpenClawGoogleChatConfig>,
    #[serde(alias = "msteams", alias = "msTeams")]
    teams: Option<OpenClawTeamsConfig>,
    irc: Option<OpenClawIrcConfig>,
    mattermost: Option<OpenClawMattermostConfig>,
    feishu: Option<OpenClawFeishuConfig>,
    imessage: Option<OpenClawIMessageConfig>,
    bluebubbles: Option<OpenClawBlueBubblesConfig>,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawTelegramConfig {
    bot_token: Option<String>,
    allow_from: Option<serde_json::Value>,
    group_policy: Option<String>,
    dm_policy: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawDiscordConfig {
    token: Option<String>,
    guilds: Option<serde_json::Value>,
    dm_policy: Option<String>,
    group_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawSlackConfig {
    bot_token: Option<String>,
    app_token: Option<String>,
    dm_policy: Option<String>,
    group_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawWhatsAppConfig {
    auth_dir: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    group_policy: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawSignalConfig {
    http_url: Option<String>,
    http_host: Option<String>,
    http_port: Option<u16>,
    account: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawMatrixConfig {
    homeserver: Option<String>,
    user_id: Option<String>,
    access_token: Option<String>,
    rooms: Option<serde_json::Value>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawGoogleChatConfig {
    service_account_file: Option<String>,
    webhook_path: Option<String>,
    bot_user: Option<String>,
    dm_policy: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawTeamsConfig {
    app_id: Option<String>,
    app_password: Option<String>,
    tenant_id: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawIrcConfig {
    host: Option<String>,
    port: Option<u16>,
    tls: Option<bool>,
    nick: Option<String>,
    password: Option<String>,
    channels: Option<serde_json::Value>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawMattermostConfig {
    bot_token: Option<String>,
    base_url: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawFeishuConfig {
    app_id: Option<String>,
    app_secret: Option<String>,
    domain: Option<String>,
    dm_policy: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawIMessageConfig {
    cli_path: Option<String>,
    db_path: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawBlueBubblesConfig {
    server_url: Option<String>,
    password: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<serde_json::Value>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct OpenClawSkills {
    entries: Option<serde_json::Map<String, serde_json::Value>>,
    load: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Legacy YAML input types (backward compat for very old installs)
// ---------------------------------------------------------------------------

/// OpenClaw's legacy config.yaml structure.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct LegacyYamlConfig {
    provider: String,
    model: String,
    api_key_env: Option<String>,
    base_url: Option<String>,
    #[allow(dead_code)]
    temperature: Option<f32>,
    #[allow(dead_code)]
    max_tokens: Option<u32>,
    memory: Option<LegacyYamlMemoryConfig>,
}

impl Default for LegacyYamlConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key_env: None,
            base_url: None,
            temperature: None,
            max_tokens: None,
            memory: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyYamlMemoryConfig {
    decay_rate: Option<f32>,
}

/// OpenClaw's legacy agent.yaml structure.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct LegacyYamlAgent {
    name: String,
    description: String,
    model: Option<String>,
    provider: Option<String>,
    system_prompt: Option<String>,
    tools: Vec<String>,
    tool_profile: Option<String>,
    api_key_env: Option<String>,
    base_url: Option<String>,
    tags: Vec<String>,
}

impl Default for LegacyYamlAgent {
    fn default() -> Self {
        Self {
            name: "unnamed".to_string(),
            description: String::new(),
            model: None,
            provider: None,
            system_prompt: None,
            tools: vec![],
            tool_profile: None,
            api_key_env: None,
            base_url: None,
            tags: vec![],
        }
    }
}

/// OpenClaw's legacy channel config structure.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyYamlChannelConfig {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    channel_type: String,
    bot_token_env: Option<String>,
    app_token_env: Option<String>,
    #[allow(dead_code)]
    phone_number_id_env: Option<String>,
    #[allow(dead_code)]
    access_token_env: Option<String>,
    #[allow(dead_code)]
    verify_token_env: Option<String>,
    #[allow(dead_code)]
    webhook_port: Option<u16>,
    allowed_users: Vec<String>,
    default_agent: Option<String>,
}

// ---------------------------------------------------------------------------
// LibreFang output types (TOML)
// ---------------------------------------------------------------------------

/// LibreFang config.toml structure for serialization.
///
/// This is a minimal subset of `librefang_types::config::KernelConfig` — the
/// kernel's `#[serde(default)]` on every field means any LibreFang struct field
/// we omit will simply take its default value at load time. We only emit the
/// fields carried over from OpenClaw plus `config_version` so the kernel
/// recognises this as an up-to-date file and skips the versioned-migration step.
#[derive(Serialize)]
struct LibreFangConfig {
    config_version: u32,
    api_listen: String,
    default_model: LibreFangModelConfig,
    memory: LibreFangMemorySection,
    #[serde(skip_serializing_if = "Option::is_none")]
    channels: Option<toml::Value>,
}

#[derive(Serialize)]
struct LibreFangModelConfig {
    provider: String,
    model: String,
    api_key_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

#[derive(Serialize)]
struct LibreFangMemorySection {
    decay_rate: f32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Secrets & policy helpers
// ---------------------------------------------------------------------------

/// Write or update a key in a secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
fn write_secret_env(path: &Path, key: &str, value: &str) -> Result<(), std::io::Error> {
    if key.contains('\n') || key.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret key must not contain newline characters",
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value must not contain newline characters",
        ));
    }
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Upsert
    let prefix = format!("{key}=");
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, lines.join("\n") + "\n")?;

    // SECURITY: Restrict file permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Map OpenClaw DM policy to LibreFang DM policy string.
fn map_dm_policy(oc: &str) -> &'static str {
    match oc.to_lowercase().as_str() {
        "open" => "respond",
        "allowlist" | "allow_list" => "allowed_only",
        "pairing" | "disabled" => "ignore",
        _ => "respond",
    }
}

/// Map OpenClaw group policy to LibreFang group policy string.
///
/// LibreFang `GroupPolicy` variants: `all | mention_only | commands_only | ignore`.
fn map_group_policy(oc: &str) -> &'static str {
    match oc.to_lowercase().as_str() {
        "open" | "all" => "all",
        "mention" | "mention_only" => "mention_only",
        "commands" | "commands_only" | "slash_only" => "commands_only",
        "disabled" | "ignore" => "ignore",
        _ => "mention_only",
    }
}

/// Build a TOML table for a channel with the given fields and optional overrides.
///
/// The returned table has the shape:
/// ```toml
/// { ...fields, overrides = { dm_policy, group_policy } }
/// ```
///
/// Allow-lists must be written by the caller into the channel-specific
/// top-level field (e.g. `allowed_users`, `allowed_guilds`, `allowed_channels`),
/// because `ChannelOverrides` has no `allowed_users` field.
fn build_channel_table(
    fields: Vec<(&str, toml::Value)>,
    dm_policy: Option<&str>,
    group_policy: Option<&str>,
) -> toml::Value {
    let mut table = toml::map::Map::new();
    for (key, val) in fields {
        table.insert(key.to_string(), val);
    }

    let has_overrides = dm_policy.is_some() || group_policy.is_some();
    if has_overrides {
        let mut overrides = toml::map::Map::new();
        if let Some(dp) = dm_policy {
            overrides.insert(
                "dm_policy".to_string(),
                toml::Value::String(map_dm_policy(dp).to_string()),
            );
        }
        if let Some(gp) = group_policy {
            overrides.insert(
                "group_policy".to_string(),
                toml::Value::String(map_group_policy(gp).to_string()),
            );
        }
        table.insert("overrides".to_string(), toml::Value::Table(overrides));
    }

    toml::Value::Table(table)
}

/// Convert an OpenClaw `allow_from` list into a TOML array of strings.
/// Returns `None` if the list is empty or not present.
fn allow_from_to_toml_array(allow_from: Option<&serde_json::Value>) -> Option<toml::Value> {
    let list = allow_from.map(extract_string_list).unwrap_or_default();
    if list.is_empty() {
        return None;
    }
    let arr: Vec<toml::Value> = list.into_iter().map(toml::Value::String).collect();
    Some(toml::Value::Array(arr))
}

/// Split an OpenClaw model reference like `"provider/model"` into `(provider, model)`.
/// If there's no slash, returns `("anthropic", input)` as a fallback.
fn split_model_ref(model_ref: &str) -> (String, String) {
    if let Some(pos) = model_ref.find('/') {
        let provider = &model_ref[..pos];
        let model = &model_ref[pos + 1..];
        (map_provider(provider), model.to_string())
    } else {
        ("anthropic".to_string(), model_ref.to_string())
    }
}

/// Extract the primary model string from an agent entry, falling back to defaults.
fn extract_primary_model(
    agent: &OpenClawAgentEntry,
    defaults: Option<&OpenClawAgentDefaults>,
) -> Option<String> {
    // Try agent-level model first
    if let Some(ref m) = agent.model {
        match m {
            OpenClawAgentModel::Simple(s) => return Some(s.clone()),
            OpenClawAgentModel::Detailed(d) => {
                if let Some(ref p) = d.primary {
                    return Some(p.clone());
                }
            }
        }
    }
    // Fall back to defaults
    if let Some(defs) = defaults {
        if let Some(ref m) = defs.model {
            match m {
                OpenClawAgentModel::Simple(s) => return Some(s.clone()),
                OpenClawAgentModel::Detailed(d) => return d.primary.clone(),
            }
        }
    }
    None
}

/// Extract fallback model strings from an agent entry.
fn extract_fallback_models(
    agent: &OpenClawAgentEntry,
    defaults: Option<&OpenClawAgentDefaults>,
) -> Vec<String> {
    // Try agent-level
    if let Some(OpenClawAgentModel::Detailed(ref d)) = agent.model {
        if !d.fallbacks.is_empty() {
            return d.fallbacks.clone();
        }
    }
    // Fall back to defaults
    if let Some(defs) = defaults {
        if let Some(OpenClawAgentModel::Detailed(ref d)) = defs.model {
            if !d.fallbacks.is_empty() {
                return d.fallbacks.clone();
            }
        }
    }
    vec![]
}

/// Which config file does this dir contain? Returns the path if found.
fn find_config_file(dir: &Path) -> Option<PathBuf> {
    // Prefer JSON5 config (modern OpenClaw)
    for name in &[
        "openclaw.json",
        "clawdbot.json",
        "moldbot.json",
        "moltbot.json",
    ] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to YAML (very old installs)
    let yaml = dir.join("config.yaml");
    if yaml.exists() {
        return Some(yaml);
    }
    None
}

// Tool name mapping and recognition are shared with the skill system.
use librefang_types::tool_compat::{is_known_librefang_tool, map_tool_name};

/// Map OpenClaw tool profile to LibreFang capability tool list.
/// Delegates to `ToolProfile` so the migration and kernel use identical definitions.
fn tools_for_profile(profile: &str) -> Vec<String> {
    use librefang_types::agent::ToolProfile;
    let p = match profile {
        "minimal" => ToolProfile::Minimal,
        "coding" => ToolProfile::Coding,
        "research" => ToolProfile::Research,
        "messaging" => ToolProfile::Messaging,
        "automation" => ToolProfile::Automation,
        _ => ToolProfile::Full,
    };
    p.tools()
}

/// Map OpenClaw provider name to LibreFang provider name.
fn map_provider(openclaw_provider: &str) -> String {
    match openclaw_provider.to_lowercase().as_str() {
        "anthropic" | "claude" => "anthropic".to_string(),
        "openai" | "gpt" => "openai".to_string(),
        "groq" => "groq".to_string(),
        "ollama" => "ollama".to_string(),
        "openrouter" => "openrouter".to_string(),
        "deepseek" => "deepseek".to_string(),
        "together" => "together".to_string(),
        "mistral" => "mistral".to_string(),
        "fireworks" => "fireworks".to_string(),
        "google" | "gemini" => "google".to_string(),
        "xai" | "grok" => "xai".to_string(),
        "cerebras" => "cerebras".to_string(),
        "sambanova" => "sambanova".to_string(),
        other => other.to_string(),
    }
}

/// Map OpenClaw provider to its default API key env var.
fn default_api_key_env(provider: &str) -> String {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "groq" => "GROQ_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "cerebras" => "CEREBRAS_API_KEY".to_string(),
        "sambanova" => "SAMBANOVA_API_KEY".to_string(),
        "ollama" => String::new(), // Ollama doesn't need an API key
        _ => format!("{}_API_KEY", provider.to_uppercase()),
    }
}

/// Derive capability grants from the tool list.
fn derive_capabilities(tools: &[String]) -> AgentCapabilities {
    let mut caps = AgentCapabilities::default();

    for tool in tools {
        match tool.as_str() {
            "*" => {
                caps.shell = vec!["*".to_string()];
                caps.network = vec!["*".to_string()];
                caps.agent_message = vec!["*".to_string()];
                caps.agent_spawn = true;
            }
            "shell_exec" => {
                caps.shell = vec!["*".to_string()];
            }
            "web_fetch" | "web_search" | "browser_navigate" if caps.network.is_empty() => {
                caps.network = vec!["*".to_string()];
            }
            "agent_send" | "agent_list" => {
                if caps.agent_message.is_empty() {
                    caps.agent_message = vec!["*".to_string()];
                }
                caps.agent_spawn = true;
            }
            _ => {}
        }
    }

    caps
}

#[derive(Default)]
struct AgentCapabilities {
    shell: Vec<String>,
    network: Vec<String>,
    agent_message: Vec<String>,
    agent_spawn: bool,
}

// ---------------------------------------------------------------------------
// Auto-detection
// ---------------------------------------------------------------------------

/// Try to find the OpenClaw home directory.
pub fn detect_openclaw_home() -> Option<PathBuf> {
    // Check env override first
    if let Ok(dir) = std::env::var("OPENCLAW_STATE_DIR") {
        let p = PathBuf::from(dir);
        if p.exists() && p.is_dir() {
            return Some(p);
        }
    }

    // Standard locations + legacy dir names
    let home = dirs::home_dir();
    let mut candidates: Vec<Option<PathBuf>> = vec![
        home.as_ref().map(|h| h.join(".openclaw")),
        home.as_ref().map(|h| h.join(".clawdbot")),
        home.as_ref().map(|h| h.join(".moldbot")),
        home.as_ref().map(|h| h.join(".moltbot")),
        home.as_ref().map(|h| h.join("openclaw")),
        home.as_ref().map(|h| h.join(".config").join("openclaw")),
    ];

    // Windows-specific paths
    if let Ok(p) = std::env::var("APPDATA") {
        candidates.push(Some(PathBuf::from(p).join("openclaw")));
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        candidates.push(Some(PathBuf::from(p).join("openclaw")));
    }

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() && candidate.is_dir() {
            // Verify it looks like an OpenClaw workspace
            if find_config_file(&candidate).is_some() {
                return Some(candidate);
            }
            // Also accept if it has agents or sessions dirs
            if candidate.join("sessions").exists() || candidate.join("memory").exists() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Scan an OpenClaw workspace and return what's available for migration.
pub fn scan_openclaw_workspace(path: &Path) -> ScanResult {
    let config_file = find_config_file(path);
    let is_json5 = config_file
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "json"));

    let mut result = ScanResult {
        path: path.display().to_string(),
        has_config: config_file.is_some(),
        agents: vec![],
        channels: vec![],
        skills: vec![],
        has_memory: false,
    };

    if let (true, Some(ref cf)) = (is_json5, &config_file) {
        scan_from_json5(path, cf, &mut result);
    } else {
        scan_from_legacy_yaml(path, &mut result);
    }

    result
}

fn scan_from_json5(base: &Path, config_path: &Path, result: &mut ScanResult) {
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let root: OpenClawRoot = match json5::from_str(&content) {
        Ok(r) => r,
        Err(_) => return,
    };

    // Agents from JSON config
    if let Some(ref agents) = root.agents {
        for entry in &agents.list {
            let id = entry.id.clone();
            let name = entry.name.clone().unwrap_or_else(|| id.clone());

            let (provider, model) = extract_primary_model(entry, agents.defaults.as_ref())
                .map(|m| split_model_ref(&m))
                .unwrap_or_else(|| ("anthropic".to_string(), String::new()));

            let tool_count = entry
                .tools
                .as_ref()
                .and_then(|t| t.allow.as_ref())
                .map(|a| extract_string_list(a).len())
                .or_else(|| {
                    entry
                        .tools
                        .as_ref()
                        .and_then(|t| t.profile.as_ref())
                        .and_then(extract_profile)
                        .map(|p| tools_for_profile(&p).len())
                })
                .unwrap_or(3);

            // Check physical memory dirs
            let has_memory = base.join("memory").join(&id).join("MEMORY.md").exists();
            let has_sessions = base.join("sessions").exists();
            let has_workspace = base.join("workspaces").join(&id).exists();

            if has_memory {
                result.has_memory = true;
            }

            result.agents.push(ScannedAgent {
                name,
                description: String::new(),
                provider,
                model,
                tool_count,
                has_memory,
                has_sessions,
                has_workspace,
            });
        }
    }

    // Channels from JSON config — scan all 13 typed fields + catch-all
    if let Some(ref channels) = root.channels {
        if channels.telegram.is_some() {
            result.channels.push("telegram".to_string());
        }
        if channels.discord.is_some() {
            result.channels.push("discord".to_string());
        }
        if channels.slack.is_some() {
            result.channels.push("slack".to_string());
        }
        if channels.whatsapp.is_some() {
            result.channels.push("whatsapp".to_string());
        }
        if channels.signal.is_some() {
            result.channels.push("signal".to_string());
        }
        if channels.matrix.is_some() {
            result.channels.push("matrix".to_string());
        }
        if channels.google_chat.is_some() {
            result.channels.push("google_chat".to_string());
        }
        if channels.teams.is_some() {
            result.channels.push("teams".to_string());
        }
        if channels.irc.is_some() {
            result.channels.push("irc".to_string());
        }
        if channels.mattermost.is_some() {
            result.channels.push("mattermost".to_string());
        }
        if channels.feishu.is_some() {
            result.channels.push("feishu".to_string());
        }
        if channels.imessage.is_some() {
            result.channels.push("imessage".to_string());
        }
        if channels.bluebubbles.is_some() {
            result.channels.push("bluebubbles".to_string());
        }
        for key in channels.other.keys() {
            result.channels.push(key.clone());
        }
    }

    // Skills from JSON config
    if let Some(ref skills) = root.skills {
        if let Some(ref entries) = skills.entries {
            for key in entries.keys() {
                result.skills.push(key.clone());
            }
        }
    }

    // Also check physical memory dir
    let memory_dir = base.join("memory");
    if memory_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() && entry.path().join("MEMORY.md").exists() {
                    result.has_memory = true;
                    break;
                }
            }
        }
    }
}

fn scan_from_legacy_yaml(path: &Path, result: &mut ScanResult) {
    // Scan agents from agents/ dir
    let agents_dir = path.join("agents");
    if agents_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let agent_path = entry.path();
                if !agent_path.is_dir() {
                    continue;
                }
                let agent_yaml = agent_path.join("agent.yaml");
                if !agent_yaml.exists() {
                    continue;
                }

                let name = agent_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let has_memory = agent_path.join("MEMORY.md").exists();
                let has_sessions = agent_path.join("sessions").exists();
                let has_workspace = agent_path.join("workspace").exists();

                if has_memory {
                    result.has_memory = true;
                }

                let mut description = String::new();
                let mut provider = String::new();
                let mut model = String::new();
                let mut tool_count = 0;

                if let Ok(yaml_str) = std::fs::read_to_string(&agent_yaml) {
                    if let Ok(oc) = serde_yaml::from_str::<LegacyYamlAgent>(&yaml_str) {
                        description = oc.description.clone();
                        provider = oc.provider.unwrap_or_default();
                        model = oc.model.unwrap_or_default();
                        tool_count = if !oc.tools.is_empty() {
                            oc.tools.len()
                        } else if oc.tool_profile.is_some() {
                            tools_for_profile(oc.tool_profile.as_deref().unwrap_or("")).len()
                        } else {
                            3
                        };
                    }
                }

                result.agents.push(ScannedAgent {
                    name,
                    description,
                    provider,
                    model,
                    tool_count,
                    has_memory,
                    has_sessions,
                    has_workspace,
                });
            }
        }
    }

    // Scan channels from messaging/ dir — all 13 possible channels
    let messaging_dir = path.join("messaging");
    if messaging_dir.exists() {
        for name in &[
            "telegram",
            "discord",
            "slack",
            "whatsapp",
            "signal",
            "matrix",
            "irc",
            "mattermost",
            "feishu",
            "googlechat",
            "msteams",
            "imessage",
            "bluebubbles",
            "email",
        ] {
            if messaging_dir.join(format!("{name}.yaml")).exists() {
                result.channels.push(name.to_string());
            }
        }
    }

    // Scan skills
    let skills_dir = path.join("skills");
    if skills_dir.exists() {
        for subdir in &["community", "custom"] {
            let sub = skills_dir.join(subdir);
            if let Ok(entries) = std::fs::read_dir(&sub) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let name = entry
                            .path()
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !name.is_empty() {
                            result.skills.push(name);
                        }
                    }
                }
            }
        }
    }
}

/// Result of scanning an OpenClaw workspace.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub path: String,
    pub has_config: bool,
    pub agents: Vec<ScannedAgent>,
    pub channels: Vec<String>,
    pub skills: Vec<String>,
    pub has_memory: bool,
}

/// An agent found during scanning.
#[derive(Debug, Clone, Serialize)]
pub struct ScannedAgent {
    pub name: String,
    pub description: String,
    pub provider: String,
    pub model: String,
    pub tool_count: usize,
    pub has_memory: bool,
    pub has_sessions: bool,
    pub has_workspace: bool,
}

// ---------------------------------------------------------------------------
// Migration entry point
// ---------------------------------------------------------------------------

/// Run the OpenClaw migration.
pub fn migrate(options: &MigrateOptions) -> Result<MigrationReport, MigrateError> {
    let source = &options.source_dir;
    let real_target = &options.target_dir;

    if !source.exists() {
        return Err(MigrateError::SourceNotFound(source.clone()));
    }

    info!("Migrating from OpenClaw: {}", source.display());

    let mut report = MigrationReport {
        source: "OpenClaw".to_string(),
        dry_run: options.dry_run,
        ..Default::default()
    };

    // Refuse to re-run if the marker file is present; user edits since the
    // first import must not be overwritten.
    let marker_path = real_target.join(MIGRATION_MARKER_FILENAME);
    if marker_path.exists() && !options.dry_run {
        warn!(
            "OpenClaw migration already completed (marker {} present); skipping re-run to preserve user edits",
            marker_path.display()
        );
        report.warnings.push(format!(
            "Migration already completed — marker {} present. Delete it to force a re-import; existing files will be backed up to .bak.<timestamp> siblings.",
            marker_path.display()
        ));
        return Ok(report);
    }

    // Determine config format
    let config_file = find_config_file(source);
    let is_json5 = config_file
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "json"));

    if options.dry_run {
        // Dry-run never touches disk — no staging needed.
        if is_json5 {
            migrate_from_json5(source, real_target, true, &mut report)?;
        } else {
            migrate_from_legacy_yaml(source, real_target, true, &mut report)?;
        }
        return Ok(report);
    }

    // #3798 — Workspace-level atomicity. All writes go to a sibling staging
    // directory first; only after the entire migration succeeds is the
    // staging tree promoted into the real target via per-entry same-fs
    // renames. If anything fails partway, the staging directory is left
    // behind so the user can inspect / retry without their `~/.librefang`
    // ending up half-written.
    let staging = staging_dir_for(real_target);
    if staging.exists() {
        // Stale staging from a previous failed run. Refuse rather than
        // silently overwrite; require explicit cleanup.
        return Err(MigrateError::StagingExists(staging));
    }
    std::fs::create_dir_all(&staging)?;

    let inner = if is_json5 {
        migrate_from_json5(source, &staging, false, &mut report)
    } else {
        migrate_from_legacy_yaml(source, &staging, false, &mut report)
    };

    if let Err(e) = inner {
        warn!(
            "Migration failed; staging directory left in place for inspection: {}",
            staging.display()
        );
        return Err(e);
    }

    // Write the report and marker inside staging so they get promoted with
    // the rest in the same atomic step.
    let report_md = report.to_markdown();
    let _ = std::fs::write(staging.join("migration_report.md"), &report_md);
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let marker_body = format!(
        "OpenClaw migration completed at {stamp}.\nDelete this file to force a re-import; existing files will be moved to .bak.<timestamp> siblings rather than overwritten.\n"
    );
    if let Err(e) = std::fs::write(staging.join(MIGRATION_MARKER_FILENAME), marker_body) {
        warn!(
            "Failed to write migration marker into staging: {} (re-runs will not be detected)",
            e
        );
    }

    // Promote staging → real target. Existing files in the real target are
    // never clobbered (matches #3795 semantics).
    if let Err(e) = promote_staging(&staging, real_target, &mut report) {
        warn!(
            "Promotion partially failed; staging directory left in place: {} ({e})",
            staging.display()
        );
        return Err(e);
    }

    // Best-effort cleanup; a leftover empty staging is harmless.
    let _ = std::fs::remove_dir_all(&staging);

    Ok(report)
}

/// #3798 — Compute the sibling staging directory used during atomic migration.
fn staging_dir_for(target: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let leaf = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(".librefang");
    parent.join(format!("{leaf}.migrate-staging-{stamp}"))
}

/// #3798 — Recursively move entries from `staging` into `target`, never
/// overwriting existing files. Imported destinations in `report` are
/// rewritten so they point at the real target instead of staging.
fn promote_staging(
    staging: &Path,
    target: &Path,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    std::fs::create_dir_all(target)?;
    promote_dir(staging, target, target, report)?;

    let staging_str = staging.display().to_string();
    let target_str = target.display().to_string();
    for item in &mut report.imported {
        if item.destination.starts_with(&staging_str) {
            item.destination = item.destination.replacen(&staging_str, &target_str, 1);
        }
    }

    Ok(())
}

fn promote_dir(
    src: &Path,
    dst: &Path,
    real_target_root: &Path,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;

        if ft.is_dir() {
            // Recurse so we can merge into existing target subdirs without
            // refusing the whole tree.
            promote_dir(&from, &to, real_target_root, report)?;
            // Prune the now-empty staging subdir.
            let _ = std::fs::remove_dir(&from);
        } else if to.exists() {
            // #3795 — never clobber. Drop the staged copy.
            report.warnings.push(format!(
                "Skipped promoting {} — destination already exists",
                to.strip_prefix(real_target_root).unwrap_or(&to).display()
            ));
            let _ = std::fs::remove_file(&from);
        } else {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Same-filesystem rename — atomic per-entry. If for some reason
            // the rename fails (e.g. cross-device), fall back to copy+remove.
            if std::fs::rename(&from, &to).is_err() {
                std::fs::copy(&from, &to)?;
                let _ = std::fs::remove_file(&from);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON5 migration flow (modern OpenClaw)
// ---------------------------------------------------------------------------

fn migrate_from_json5(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let config_path = find_config_file(source).ok_or_else(|| {
        MigrateError::ConfigParse("No openclaw.json found in workspace".to_string())
    })?;

    let content = std::fs::read_to_string(&config_path)?;
    let root: OpenClawRoot = json5::from_str(&content)
        .map_err(|e| MigrateError::Json5Parse(format!("{}: {e}", config_path.display())))?;

    // #3797 — Reject configs that declare a schema version we don't support.
    match root.version {
        None => warn!("openclaw.json has no version field — assuming compatible format"),
        Some(v) if SUPPORTED_OPENCLAW_VERSIONS.contains(&v) => {}
        Some(v) => return Err(MigrateError::UnsupportedVersion(v)),
    }

    // 1. Migrate config
    migrate_config_from_json(&root, target, dry_run, report)?;

    // 2. Migrate agents
    migrate_agents_from_json(&root, target, dry_run, report)?;

    // 3. Migrate memory files
    migrate_memory_files(source, &root, target, dry_run, report)?;

    // 4. Migrate workspace dirs
    migrate_workspace_dirs(source, &root, target, dry_run, report)?;

    // 5. Migrate sessions
    migrate_sessions(source, target, dry_run, report)?;

    // 6. Report skipped features
    report_skipped_features(&root, source, report);

    info!("JSON5 migration complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Config migration from JSON5
// ---------------------------------------------------------------------------

fn migrate_config_from_json(
    root: &OpenClawRoot,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    // Extract default model from agents.defaults.model
    let (provider, model) = root
        .agents
        .as_ref()
        .and_then(|a| a.defaults.as_ref())
        .and_then(|d| d.model.as_ref())
        .and_then(|m| match m {
            OpenClawAgentModel::Simple(s) => Some(s.clone()),
            OpenClawAgentModel::Detailed(d) => d.primary.clone(),
        })
        .map(|m| split_model_ref(&m))
        .unwrap_or_else(|| {
            (
                "anthropic".to_string(),
                "claude-sonnet-4-20250514".to_string(),
            )
        });

    let api_key_env = default_api_key_env(&provider);

    // Extract channels (writes secrets.env)
    let channels = migrate_channels_from_json(root, target, dry_run, report);

    let of_config = LibreFangConfig {
        config_version: CONFIG_VERSION,
        api_listen: DEFAULT_API_LISTEN.to_string(),
        default_model: LibreFangModelConfig {
            provider,
            model,
            api_key_env,
            base_url: None,
        },
        memory: LibreFangMemorySection { decay_rate: 0.05 },
        channels,
    };

    let toml_str = toml::to_string_pretty(&of_config)?;

    let config_content = format!(
        "# LibreFang Agent OS configuration\n\
         # Migrated from OpenClaw on {}\n\n\
         {toml_str}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    let dest = target.join("config.toml");

    if !dry_run {
        std::fs::create_dir_all(target)?;
        // Back up any existing config.toml before overwriting.
        write_with_backup(&dest, &config_content, report)?;
    }

    report.imported.push(MigrateItem {
        kind: ItemKind::Config,
        name: "openclaw.json".to_string(),
        destination: dest.display().to_string(),
    });

    info!("Migrated openclaw.json -> config.toml");
    Ok(())
}

// ---------------------------------------------------------------------------
// Channel migration from JSON5
// ---------------------------------------------------------------------------

fn migrate_channels_from_json(
    root: &OpenClawRoot,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Option<toml::Value> {
    let oc_channels = root.channels.as_ref()?;

    let mut channels_table = toml::map::Map::new();
    let secrets_path = target.join("secrets.env");

    /// Helper: write a secret and report it.
    fn emit_secret(
        path: &Path,
        dry_run: bool,
        key: &str,
        value: &str,
        report: &mut MigrationReport,
    ) {
        if value.is_empty() {
            return;
        }
        if !dry_run {
            if let Err(e) = write_secret_env(path, key, value) {
                report
                    .warnings
                    .push(format!("Failed to write {key} to secrets.env: {e}"));
                return;
            }
        }
        report.imported.push(MigrateItem {
            kind: ItemKind::Secret,
            name: key.to_string(),
            destination: "secrets.env".to_string(),
        });
    }

    // --- Telegram ---
    if let Some(ref tg) = oc_channels.telegram {
        if tg.enabled.unwrap_or(true) {
            if let Some(ref token) = tg.bot_token {
                emit_secret(&secrets_path, dry_run, "TELEGRAM_BOT_TOKEN", token, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "bot_token_env",
                toml::Value::String("TELEGRAM_BOT_TOKEN".into()),
            )];
            if let Some(arr) = allow_from_to_toml_array(tg.allow_from.as_ref()) {
                fields.push(("allowed_users", arr));
            }
            channels_table.insert(
                "telegram".to_string(),
                build_channel_table(fields, tg.dm_policy.as_deref(), tg.group_policy.as_deref()),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "telegram".to_string(),
                destination: "config.toml [channels.telegram]".to_string(),
            });
        }
    }

    // --- Discord ---
    if let Some(ref dc) = oc_channels.discord {
        if dc.enabled.unwrap_or(true) {
            if let Some(ref token) = dc.token {
                emit_secret(&secrets_path, dry_run, "DISCORD_BOT_TOKEN", token, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "bot_token_env",
                toml::Value::String("DISCORD_BOT_TOKEN".into()),
            )];
            if let Some(arr) = allow_from_to_toml_array(dc.allow_from.as_ref()) {
                fields.push(("allowed_users", arr));
            }
            channels_table.insert(
                "discord".to_string(),
                build_channel_table(fields, dc.dm_policy.as_deref(), dc.group_policy.as_deref()),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "discord".to_string(),
                destination: "config.toml [channels.discord]".to_string(),
            });
        }
    }

    // --- Slack ---
    if let Some(ref sl) = oc_channels.slack {
        if sl.enabled.unwrap_or(true) {
            if let Some(ref token) = sl.bot_token {
                emit_secret(&secrets_path, dry_run, "SLACK_BOT_TOKEN", token, report);
            }
            if let Some(ref token) = sl.app_token {
                emit_secret(&secrets_path, dry_run, "SLACK_APP_TOKEN", token, report);
            }
            let fields: Vec<(&str, toml::Value)> = vec![
                (
                    "bot_token_env",
                    toml::Value::String("SLACK_BOT_TOKEN".into()),
                ),
                (
                    "app_token_env",
                    toml::Value::String("SLACK_APP_TOKEN".into()),
                ),
            ];
            if allow_from_to_toml_array(sl.allow_from.as_ref()).is_some() {
                report.warnings.push(
                    "Slack: OpenClaw 'allow_from' (per-user allowlist) could not be \
                     auto-mapped — SlackConfig has no per-user allowlist, only \
                     'allowed_channels' (channel IDs)."
                        .to_string(),
                );
            }
            channels_table.insert(
                "slack".to_string(),
                build_channel_table(fields, sl.dm_policy.as_deref(), sl.group_policy.as_deref()),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "slack".to_string(),
                destination: "config.toml [channels.slack]".to_string(),
            });
        }
    }

    // --- WhatsApp ---
    if let Some(ref wa) = oc_channels.whatsapp {
        if wa.enabled.unwrap_or(true) {
            // WhatsApp uses Baileys credential dir — copy it, warn user
            if let Some(ref auth_dir) = wa.auth_dir {
                let src_path = PathBuf::from(auth_dir);
                if src_path.exists() {
                    let dest_creds = target.join("credentials").join("whatsapp");
                    if !dry_run {
                        if let Err(e) = copy_dir_recursive(&src_path, &dest_creds) {
                            report
                                .warnings
                                .push(format!("Failed to copy WhatsApp credentials: {e}"));
                        }
                    }
                    report.imported.push(MigrateItem {
                        kind: ItemKind::Secret,
                        name: "whatsapp/credentials".to_string(),
                        destination: dest_creds.display().to_string(),
                    });
                    report.warnings.push(
                        "WhatsApp Baileys credentials copied — you may need to re-authenticate"
                            .to_string(),
                    );
                }
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "access_token_env",
                toml::Value::String("WHATSAPP_ACCESS_TOKEN".into()),
            )];
            if let Some(arr) = allow_from_to_toml_array(wa.allow_from.as_ref()) {
                fields.push(("allowed_users", arr));
            }
            channels_table.insert(
                "whatsapp".to_string(),
                build_channel_table(fields, wa.dm_policy.as_deref(), wa.group_policy.as_deref()),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "whatsapp".to_string(),
                destination: "config.toml [channels.whatsapp]".to_string(),
            });
        }
    }

    // --- Signal ---
    if let Some(ref sig) = oc_channels.signal {
        if sig.enabled.unwrap_or(true) {
            // Construct API URL from host+port or use http_url directly
            let api_url = sig.http_url.clone().unwrap_or_else(|| {
                let host = sig.http_host.as_deref().unwrap_or("localhost");
                let port = sig.http_port.unwrap_or(8080);
                format!("http://{host}:{port}")
            });
            let mut fields: Vec<(&str, toml::Value)> =
                vec![("api_url", toml::Value::String(api_url))];
            if let Some(ref account) = sig.account {
                fields.push(("phone_number", toml::Value::String(account.clone())));
            }
            if let Some(arr) = allow_from_to_toml_array(sig.allow_from.as_ref()) {
                fields.push(("allowed_users", arr));
            }
            channels_table.insert(
                "signal".to_string(),
                build_channel_table(fields, sig.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "signal".to_string(),
                destination: "config.toml [channels.signal]".to_string(),
            });
        }
    }

    // --- Matrix ---
    if let Some(ref mx) = oc_channels.matrix {
        if mx.enabled.unwrap_or(true) {
            if let Some(ref token) = mx.access_token {
                emit_secret(&secrets_path, dry_run, "MATRIX_ACCESS_TOKEN", token, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "access_token_env",
                toml::Value::String("MATRIX_ACCESS_TOKEN".into()),
            )];
            if let Some(ref hs) = mx.homeserver {
                fields.push(("homeserver_url", toml::Value::String(hs.clone())));
            }
            if let Some(ref uid) = mx.user_id {
                fields.push(("user_id", toml::Value::String(uid.clone())));
            }
            if let Some(arr) = allow_from_to_toml_array(mx.rooms.as_ref()) {
                fields.push(("allowed_rooms", arr));
            }
            if allow_from_to_toml_array(mx.allow_from.as_ref()).is_some() {
                report.warnings.push(
                    "Matrix: OpenClaw 'allow_from' could not be auto-mapped — \
                     MatrixConfig has no per-user allowlist, only 'allowed_rooms' (room IDs)."
                        .to_string(),
                );
            }
            channels_table.insert(
                "matrix".to_string(),
                build_channel_table(fields, mx.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "matrix".to_string(),
                destination: "config.toml [channels.matrix]".to_string(),
            });
        }
    }

    // --- Google Chat ---
    if let Some(ref gc) = oc_channels.google_chat {
        if gc.enabled.unwrap_or(true) {
            // Copy service account file if it exists
            if let Some(ref sa_file) = gc.service_account_file {
                let src_sa = PathBuf::from(sa_file);
                if src_sa.exists() {
                    let dest_sa = target.join("credentials").join("google_chat_sa.json");
                    if !dry_run {
                        if let Some(parent) = dest_sa.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::copy(&src_sa, &dest_sa) {
                            report
                                .warnings
                                .push(format!("Failed to copy Google Chat SA file: {e}"));
                        }
                    }
                    report.imported.push(MigrateItem {
                        kind: ItemKind::Secret,
                        name: "google_chat/service_account".to_string(),
                        destination: dest_sa.display().to_string(),
                    });
                }
            }
            let fields: Vec<(&str, toml::Value)> = vec![(
                "service_account_env",
                toml::Value::String("GOOGLE_CHAT_SA_FILE".into()),
            )];
            channels_table.insert(
                "google_chat".to_string(),
                build_channel_table(fields, gc.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "google_chat".to_string(),
                destination: "config.toml [channels.google_chat]".to_string(),
            });
        }
    }

    // --- Teams ---
    if let Some(ref tm) = oc_channels.teams {
        if tm.enabled.unwrap_or(true) {
            if let Some(ref pw) = tm.app_password {
                emit_secret(&secrets_path, dry_run, "TEAMS_APP_PASSWORD", pw, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "app_password_env",
                toml::Value::String("TEAMS_APP_PASSWORD".into()),
            )];
            if let Some(ref id) = tm.app_id {
                fields.push(("app_id", toml::Value::String(id.clone())));
            }
            if let Some(ref tenant) = tm.tenant_id {
                fields.push((
                    "allowed_tenants",
                    toml::Value::Array(vec![toml::Value::String(tenant.clone())]),
                ));
            }
            if allow_from_to_toml_array(tm.allow_from.as_ref()).is_some() {
                report.warnings.push(
                    "Teams: OpenClaw 'allow_from' could not be auto-mapped — \
                     TeamsConfig has no per-user allowlist, only 'allowed_tenants' (tenant IDs)."
                        .to_string(),
                );
            }
            channels_table.insert(
                "teams".to_string(),
                build_channel_table(fields, tm.dm_policy.as_deref(), None),
            );
            report.warnings.push(
                "Teams: signature_required defaults to true. Set TEAMS_SECURITY_TOKEN \
                 (base64 outgoing-webhook token from the Teams portal) before \
                 starting the daemon, or the adapter will refuse to register."
                    .to_string(),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "teams".to_string(),
                destination: "config.toml [channels.teams]".to_string(),
            });
        }
    }

    // --- IRC ---
    if let Some(ref irc) = oc_channels.irc {
        if irc.enabled.unwrap_or(true) {
            if let Some(ref pw) = irc.password {
                emit_secret(&secrets_path, dry_run, "IRC_PASSWORD", pw, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = Vec::new();
            if let Some(ref host) = irc.host {
                fields.push(("server", toml::Value::String(host.clone())));
            }
            if let Some(port) = irc.port {
                fields.push(("port", toml::Value::Integer(port as i64)));
            }
            if let Some(ref nick) = irc.nick {
                fields.push(("nick", toml::Value::String(nick.clone())));
            }
            if let Some(tls) = irc.tls {
                fields.push(("use_tls", toml::Value::Boolean(tls)));
            }
            if irc.password.is_some() {
                fields.push(("password_env", toml::Value::String("IRC_PASSWORD".into())));
            }
            if let Some(ref chans_val) = irc.channels {
                let chans = extract_string_list(chans_val);
                if !chans.is_empty() {
                    let arr: Vec<toml::Value> = chans
                        .iter()
                        .map(|c| toml::Value::String(c.clone()))
                        .collect();
                    fields.push(("channels", toml::Value::Array(arr)));
                }
            }
            if allow_from_to_toml_array(irc.allow_from.as_ref()).is_some() {
                report.warnings.push(
                    "IRC: OpenClaw 'allow_from' could not be auto-mapped — \
                     IrcConfig has no per-user allowlist field."
                        .to_string(),
                );
            }
            channels_table.insert(
                "irc".to_string(),
                build_channel_table(fields, irc.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "irc".to_string(),
                destination: "config.toml [channels.irc]".to_string(),
            });
        }
    }

    // --- Mattermost ---
    if let Some(ref mm) = oc_channels.mattermost {
        if mm.enabled.unwrap_or(true) {
            if let Some(ref token) = mm.bot_token {
                emit_secret(&secrets_path, dry_run, "MATTERMOST_TOKEN", token, report);
            }
            let mut fields: Vec<(&str, toml::Value)> =
                vec![("token_env", toml::Value::String("MATTERMOST_TOKEN".into()))];
            if let Some(ref url) = mm.base_url {
                fields.push(("server_url", toml::Value::String(url.clone())));
            }
            if allow_from_to_toml_array(mm.allow_from.as_ref()).is_some() {
                report.warnings.push(
                    "Mattermost: OpenClaw 'allow_from' could not be auto-mapped — \
                     MattermostConfig has no per-user allowlist, only 'allowed_channels' (channel IDs)."
                        .to_string(),
                );
            }
            channels_table.insert(
                "mattermost".to_string(),
                build_channel_table(fields, mm.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "mattermost".to_string(),
                destination: "config.toml [channels.mattermost]".to_string(),
            });
        }
    }

    // --- Feishu ---
    if let Some(ref fs) = oc_channels.feishu {
        if fs.enabled.unwrap_or(true) {
            if let Some(ref secret) = fs.app_secret {
                emit_secret(&secrets_path, dry_run, "FEISHU_APP_SECRET", secret, report);
            }
            let mut fields: Vec<(&str, toml::Value)> = vec![(
                "app_secret_env",
                toml::Value::String("FEISHU_APP_SECRET".into()),
            )];
            if let Some(ref id) = fs.app_id {
                fields.push(("app_id", toml::Value::String(id.clone())));
            }
            if let Some(ref domain) = fs.domain {
                // Map OpenClaw 'domain' to LibreFang 'region': any non-CN domain
                // (e.g. lark.com / larksuite.com) → "intl", otherwise "cn".
                let region = if domain.contains("lark") || domain.contains("intl") {
                    "intl"
                } else {
                    "cn"
                };
                fields.push(("region", toml::Value::String(region.to_string())));
            }
            channels_table.insert(
                "feishu".to_string(),
                build_channel_table(fields, fs.dm_policy.as_deref(), None),
            );
            report.imported.push(MigrateItem {
                kind: ItemKind::Channel,
                name: "feishu".to_string(),
                destination: "config.toml [channels.feishu]".to_string(),
            });
        }
    }

    // --- iMessage (skip — macOS-only, manual setup) ---
    if oc_channels.imessage.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Channel,
            name: "imessage".to_string(),
            reason: "macOS-only channel — requires manual setup on the target Mac".to_string(),
        });
    }

    // --- BlueBubbles (skip — no LibreFang adapter) ---
    if oc_channels.bluebubbles.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Channel,
            name: "bluebubbles".to_string(),
            reason: "No LibreFang adapter available — consider using the iMessage channel instead"
                .to_string(),
        });
    }

    // --- Unknown channels from the catch-all ---
    for key in oc_channels.other.keys() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Channel,
            name: key.clone(),
            reason: format!("Unknown channel '{key}' — not mapped to any LibreFang adapter"),
        });
    }

    if channels_table.is_empty() {
        None
    } else {
        Some(toml::Value::Table(channels_table))
    }
}

// ---------------------------------------------------------------------------
// Agent migration from JSON5
// ---------------------------------------------------------------------------

fn migrate_agents_from_json(
    root: &OpenClawRoot,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let agents = match root.agents.as_ref() {
        Some(a) => a,
        None => {
            report
                .warnings
                .push("No agents section found in openclaw.json".to_string());
            return Ok(());
        }
    };

    let defaults = agents.defaults.as_ref();

    for entry in &agents.list {
        let id = &entry.id;
        if id.is_empty() {
            continue;
        }

        // #3794 — Reject ids with path-traversal components.
        if let Err(e) = validate_migration_id(id) {
            warn!("Skipping agent with unsafe id {id:?}: {e}");
            report.skipped.push(SkippedItem {
                kind: ItemKind::Agent,
                name: id.clone(),
                reason: e.to_string(),
            });
            continue;
        }

        match convert_agent_from_json(entry, defaults) {
            Ok((toml_str, unmapped_tools)) => {
                let dest_dir = target.join("agents").join(id);
                let dest_file = dest_dir.join("agent.toml");

                if !dry_run {
                    std::fs::create_dir_all(&dest_dir)?;
                    // Back up any existing agent.toml before overwriting.
                    write_with_backup(&dest_file, &toml_str, report)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Agent,
                    name: id.clone(),
                    destination: dest_file.display().to_string(),
                });

                for tool in &unmapped_tools {
                    report.warnings.push(format!(
                        "Agent '{id}': tool '{tool}' has no LibreFang equivalent and was skipped"
                    ));
                }

                info!("Migrated agent: {id}");
            }
            Err(e) => {
                warn!("Failed to migrate agent {id}: {e}");
                report.skipped.push(SkippedItem {
                    kind: ItemKind::Agent,
                    name: id.clone(),
                    reason: e.to_string(),
                });
            }
        }
    }

    Ok(())
}

fn convert_agent_from_json(
    entry: &OpenClawAgentEntry,
    defaults: Option<&OpenClawAgentDefaults>,
) -> Result<(String, Vec<String>), MigrateError> {
    let id = &entry.id;
    let display_name = entry.name.clone().unwrap_or_else(|| id.clone());

    // Resolve model
    let primary_ref = extract_primary_model(entry, defaults)
        .unwrap_or_else(|| "anthropic/claude-sonnet-4-20250514".to_string());
    let (provider, model) = split_model_ref(&primary_ref);

    // Resolve fallback models
    let fallbacks = extract_fallback_models(entry, defaults);

    // Resolve tools
    let mut unmapped_tools = Vec::new();
    // Also capture deny list — previously #[allow(dead_code)] and silently
    // dropped, which widened the agent's tool access after migration.
    let tool_blocklist: Vec<String> = entry
        .tools
        .as_ref()
        .and_then(|t| t.deny.as_ref())
        .map(extract_string_list)
        .map(|names| {
            names
                .into_iter()
                .map(|t| {
                    if is_known_librefang_tool(&t) {
                        t
                    } else if let Some(of_name) = map_tool_name(&t) {
                        of_name.to_string()
                    } else {
                        t
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let tools: Vec<String> = if let Some(ref agent_tools) = entry.tools {
        if let Some(ref allow_val) = agent_tools.allow {
            let allow = extract_string_list(allow_val);
            let mut mapped = Vec::new();
            for t in &allow {
                if is_known_librefang_tool(t) {
                    mapped.push(t.clone());
                } else if let Some(of_name) = map_tool_name(t) {
                    mapped.push(of_name.to_string());
                } else {
                    unmapped_tools.push(t.clone());
                }
            }
            // also_allow
            if let Some(ref also_val) = agent_tools.also_allow {
                let also = extract_string_list(also_val);
                for t in &also {
                    if is_known_librefang_tool(t) {
                        mapped.push(t.clone());
                    } else if let Some(of_name) = map_tool_name(t) {
                        mapped.push(of_name.to_string());
                    } else {
                        unmapped_tools.push(t.clone());
                    }
                }
            }
            mapped
        } else if let Some(ref profile_val) = agent_tools.profile {
            let profile_name = extract_profile(profile_val).unwrap_or_default();
            tools_for_profile(&profile_name)
        } else {
            resolve_default_tools(defaults)
        }
    } else {
        resolve_default_tools(defaults)
    };

    // Derive capabilities
    let caps = derive_capabilities(&tools);

    let api_key_env = {
        let env = default_api_key_env(&provider);
        if env.is_empty() {
            None
        } else {
            Some(env)
        }
    };

    // System prompt from identity
    let system_prompt = entry
        .identity
        .as_ref()
        .and_then(extract_identity_prompt)
        .or_else(|| {
            defaults
                .and_then(|d| d.identity.as_ref())
                .and_then(extract_identity_prompt)
        })
        .unwrap_or_else(|| {
            format!(
                "You are {display_name}, an AI agent running on the LibreFang Agent OS. You are helpful, concise, and accurate."
            )
        });

    // Resolve profile name to a valid LibreFang ToolProfile variant (snake_case).
    // Must be written BEFORE any [section] header so it lands at the top level
    // of the agent manifest, not inside a section.
    let profile_name: Option<&'static str> = entry
        .tools
        .as_ref()
        .and_then(|t| t.profile.as_ref())
        .and_then(extract_profile)
        .map(|p| map_profile_to_librefang(&p));

    // Build agent TOML
    let mut toml_str = String::new();
    toml_str.push_str(&format!(
        "# LibreFang agent manifest\n# Migrated from OpenClaw agent '{id}'\n\n"
    ));
    toml_str.push_str(&format!(
        "name = \"{}\"\n",
        display_name.replace('"', "\\\"")
    ));
    toml_str.push_str(&format!("version = \"{}\"\n", librefang_types::VERSION));
    toml_str.push_str(&format!(
        "description = \"Migrated from OpenClaw agent '{id}'\"\n"
    ));
    toml_str.push_str("author = \"librefang\"\n");
    toml_str.push_str("module = \"builtin:chat\"\n");
    if let Some(p) = profile_name {
        toml_str.push_str(&format!("profile = \"{p}\"\n"));
    }

    // Per-agent skill allowlist (previously silently dropped during migration).
    if let Some(ref skills_val) = entry.skills {
        let skill_names = extract_string_list(skills_val);
        if !skill_names.is_empty() {
            let skills_toml: Vec<String> = skill_names.iter().map(|s| format!("\"{s}\"")).collect();
            toml_str.push_str(&format!("skills = [{}]\n", skills_toml.join(", ")));
        }
    }

    // Tool blocklist from OpenClaw's tools.deny list — previously dropped,
    // which widened the agent's tool access relative to the source config.
    if !tool_blocklist.is_empty() {
        let blocklist_toml: Vec<String> =
            tool_blocklist.iter().map(|t| format!("\"{t}\"")).collect();
        toml_str.push_str(&format!(
            "tool_blocklist = [{}]\n",
            blocklist_toml.join(", ")
        ));
    }

    // Custom workspace path (previously dropped — agents reverted to default).
    if let Some(ref workspace) = entry.workspace {
        if !workspace.is_empty() {
            toml_str.push_str(&format!(
                "workspace = \"{}\"\n",
                workspace.replace('"', "\\\"")
            ));
        }
    }

    toml_str.push_str("\n[model]\n");
    toml_str.push_str(&format!("provider = \"{provider}\"\n"));
    toml_str.push_str(&format!("model = \"{model}\"\n"));
    toml_str.push_str(&format!(
        "system_prompt = \"\"\"\n{system_prompt}\n\"\"\"\n"
    ));

    if let Some(ref api_key) = api_key_env {
        toml_str.push_str(&format!("api_key_env = \"{api_key}\"\n"));
    }

    // Fallback models
    for fb in &fallbacks {
        let (fb_provider, fb_model) = split_model_ref(fb);
        let fb_api_key = default_api_key_env(&fb_provider);
        toml_str.push_str("\n[[fallback_models]]\n");
        toml_str.push_str(&format!("provider = \"{fb_provider}\"\n"));
        toml_str.push_str(&format!("model = \"{fb_model}\"\n"));
        if !fb_api_key.is_empty() {
            toml_str.push_str(&format!("api_key_env = \"{fb_api_key}\"\n"));
        }
    }

    // Capabilities section
    toml_str.push_str("\n[capabilities]\n");
    let tools_str: Vec<String> = tools.iter().map(|t| format!("\"{t}\"")).collect();
    toml_str.push_str(&format!("tools = [{}]\n", tools_str.join(", ")));
    toml_str.push_str("memory_read = [\"*\"]\n");
    toml_str.push_str("memory_write = [\"self.*\"]\n");

    if !caps.network.is_empty() {
        let net_str: Vec<String> = caps.network.iter().map(|n| format!("\"{n}\"")).collect();
        toml_str.push_str(&format!("network = [{}]\n", net_str.join(", ")));
    }
    if !caps.shell.is_empty() {
        let shell_str: Vec<String> = caps.shell.iter().map(|s| format!("\"{s}\"")).collect();
        toml_str.push_str(&format!("shell = [{}]\n", shell_str.join(", ")));
    }
    if !caps.agent_message.is_empty() {
        let msg_str: Vec<String> = caps
            .agent_message
            .iter()
            .map(|m| format!("\"{m}\""))
            .collect();
        toml_str.push_str(&format!("agent_message = [{}]\n", msg_str.join(", ")));
    }
    if caps.agent_spawn {
        toml_str.push_str("agent_spawn = true\n");
    }

    Ok((toml_str, unmapped_tools))
}

/// Map an OpenClaw tool-profile name to the snake_case string LibreFang
/// expects for the `profile` field on an agent manifest. Unknown names map
/// to `"full"` (the LibreFang `ToolProfile::Full` default).
fn map_profile_to_librefang(openclaw_profile: &str) -> &'static str {
    match openclaw_profile.to_lowercase().as_str() {
        "minimal" => "minimal",
        "coding" | "coder" | "developer" | "dev" => "coding",
        "research" | "researcher" => "research",
        "messaging" | "chat" | "messenger" => "messaging",
        "automation" | "automator" => "automation",
        "custom" => "custom",
        _ => "full",
    }
}

fn resolve_default_tools(defaults: Option<&OpenClawAgentDefaults>) -> Vec<String> {
    if let Some(defs) = defaults {
        if let Some(ref tools) = defs.tools {
            if let Some(ref profile_val) = tools.profile {
                if let Some(profile) = extract_profile(profile_val) {
                    return tools_for_profile(&profile);
                }
            }
            if let Some(ref allow_val) = tools.allow {
                let allow = extract_string_list(allow_val);
                let mut mapped = Vec::new();
                for t in &allow {
                    if is_known_librefang_tool(t) {
                        mapped.push(t.clone());
                    } else if let Some(of_name) = map_tool_name(t) {
                        mapped.push(of_name.to_string());
                    }
                }
                if !mapped.is_empty() {
                    return mapped;
                }
            }
        }
    }
    vec!["file_read".into(), "file_list".into(), "web_fetch".into()]
}

// ---------------------------------------------------------------------------
// Memory migration
// ---------------------------------------------------------------------------

fn migrate_memory_files(
    source: &Path,
    root: &OpenClawRoot,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    // Collect agent IDs from the config
    let agent_ids: Vec<String> = root
        .agents
        .as_ref()
        .map(|a| a.list.iter().map(|e| e.id.clone()).collect())
        .unwrap_or_default();

    // Check both memory layouts:
    // Layout 1: memory/<agent>/MEMORY.md
    // Layout 2: agents/<agent>/MEMORY.md (legacy)
    let mut migrated = std::collections::HashSet::new();

    let memory_dir = source.join("memory");
    if memory_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let memory_md = path.join("MEMORY.md");
                if !memory_md.exists() {
                    continue;
                }

                let agent_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let content = std::fs::read_to_string(&memory_md)?;
                if content.trim().is_empty() {
                    continue;
                }

                let dest_dir = target.join("agents").join(&agent_name);
                let dest_file = dest_dir.join("imported_memory.md");

                if !dry_run {
                    std::fs::create_dir_all(&dest_dir)?;
                    // Back up any existing imported_memory.md before overwriting.
                    write_with_backup(&dest_file, &content, report)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Memory,
                    name: format!("{agent_name}/MEMORY.md"),
                    destination: dest_file.display().to_string(),
                });

                migrated.insert(agent_name);
            }
        }
    }

    // Layout 2: agents/<agent>/MEMORY.md (legacy layout)
    let agents_dir = source.join("agents");
    if agents_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let agent_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                if migrated.contains(&agent_name) {
                    continue;
                }

                let memory_md = path.join("MEMORY.md");
                if !memory_md.exists() {
                    continue;
                }

                let content = std::fs::read_to_string(&memory_md)?;
                if content.trim().is_empty() {
                    continue;
                }

                let dest_dir = target.join("agents").join(&agent_name);
                let dest_file = dest_dir.join("imported_memory.md");

                if !dry_run {
                    std::fs::create_dir_all(&dest_dir)?;
                    // Back up any existing imported_memory.md before overwriting.
                    write_with_backup(&dest_file, &content, report)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Memory,
                    name: format!("{agent_name}/MEMORY.md"),
                    destination: dest_file.display().to_string(),
                });
            }
        }
    }

    // Warn about agents with no memory found
    for id in &agent_ids {
        if !migrated.contains(id) {
            let has_in_agents = source.join("agents").join(id).join("MEMORY.md").exists();
            if !has_in_agents {
                // not an error, just informational
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Workspace directory migration
// ---------------------------------------------------------------------------

fn migrate_workspace_dirs(
    source: &Path,
    root: &OpenClawRoot,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    // OpenClaw stores workspaces in workspaces/<agent>/
    let workspaces_dir = source.join("workspaces");
    if workspaces_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&workspaces_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let agent_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let file_count = walkdir::WalkDir::new(&path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .count();

                if file_count == 0 {
                    continue;
                }

                let dest_dir = target.join("agents").join(&agent_name).join("workspace");

                if !dry_run {
                    copy_dir_recursive(&path, &dest_dir)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Session, // reuse for workspace
                    name: format!("{agent_name}/workspace ({file_count} files)"),
                    destination: dest_dir.display().to_string(),
                });
            }
        }
    }

    // Also check legacy agents/<agent>/workspace/ layout
    let _ = root; // used for agent IDs if needed
    let agents_dir = source.join("agents");
    if agents_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let workspace_dir = path.join("workspace");
                if !workspace_dir.exists() || !workspace_dir.is_dir() {
                    continue;
                }

                let agent_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Skip if already migrated from workspaces/ dir
                let dest_dir = target.join("agents").join(&agent_name).join("workspace");
                if dest_dir.exists() {
                    continue;
                }

                let file_count = walkdir::WalkDir::new(&workspace_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .count();

                if file_count == 0 {
                    continue;
                }

                if !dry_run {
                    copy_dir_recursive(&workspace_dir, &dest_dir)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Session,
                    name: format!("{agent_name}/workspace ({file_count} files)"),
                    destination: dest_dir.display().to_string(),
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Session migration
// ---------------------------------------------------------------------------

fn migrate_sessions(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let sessions_dir = source.join("sessions");
    if !sessions_dir.exists() {
        return Ok(());
    }

    let dest_dir = target.join("imported_sessions");
    let mut count = 0;

    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Only copy .jsonl files
            let ext = path.extension().and_then(|e| e.to_str());
            if ext != Some("jsonl") {
                continue;
            }

            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if !dry_run {
                std::fs::create_dir_all(&dest_dir)?;
                std::fs::copy(&path, dest_dir.join(&file_name))?;
            }

            count += 1;
        }
    }

    if count > 0 {
        report.imported.push(MigrateItem {
            kind: ItemKind::Session,
            name: format!("{count} session files"),
            destination: dest_dir.display().to_string(),
        });
        info!("Migrated {count} session files");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Report non-migratable features
// ---------------------------------------------------------------------------

fn report_skipped_features(root: &OpenClawRoot, source: &Path, report: &mut MigrationReport) {
    // Cron jobs
    if root.cron.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "cron".to_string(),
            reason: "Cron job scheduling not yet supported — use LibreFang's ScheduleMode::Periodic instead".to_string(),
        });
    }

    // Hooks
    if root.hooks.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "hooks".to_string(),
            reason: "Webhook hooks not supported — use LibreFang's event system instead"
                .to_string(),
        });
    }

    // Auth profiles
    if let Some(ref auth) = root.auth {
        if auth.profiles.is_some() {
            report.skipped.push(SkippedItem {
                kind: ItemKind::Config,
                name: "auth-profiles".to_string(),
                reason: "Auth profiles (API keys, OAuth tokens) not migrated for security — set env vars manually".to_string(),
            });
        }
    }

    // Skills entries
    if let Some(ref skills) = root.skills {
        if let Some(ref entries) = skills.entries {
            if !entries.is_empty() {
                report.skipped.push(SkippedItem {
                    kind: ItemKind::Skill,
                    name: format!("{} skill entries", entries.len()),
                    reason: "Skills must be reinstalled via `librefang skill install`".to_string(),
                });
            }
        }
    }

    // Cron state file
    if source.join("cron").join("cron-store.json").exists() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "cron-store.json".to_string(),
            reason: "Cron run state not portable".to_string(),
        });
    }

    // Vector index
    if source.join("memory-search").join("index.db").exists() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Memory,
            name: "memory-search/index.db".to_string(),
            reason: "SQLite vector index not portable — LibreFang will rebuild embeddings"
                .to_string(),
        });
    }

    // Auth profiles file
    if source.join("auth-profiles.json").exists() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "auth-profiles.json".to_string(),
            reason: "Credential file not migrated for security — set API keys as env vars"
                .to_string(),
        });
    }

    // Session config
    if root.session.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "session".to_string(),
            reason: "Session scope config differs — LibreFang uses per-agent sessions by default"
                .to_string(),
        });
    }

    // Memory backend config
    if root.memory.is_some() {
        report.skipped.push(SkippedItem {
            kind: ItemKind::Config,
            name: "memory".to_string(),
            reason:
                "Memory backend config not migrated — LibreFang uses SQLite with vector embeddings"
                    .to_string(),
        });
    }
}

// ---------------------------------------------------------------------------
// Legacy YAML migration (backward compat)
// ---------------------------------------------------------------------------

fn migrate_from_legacy_yaml(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    // Channel parsing
    let channels = parse_legacy_channels(source, target, dry_run, report)?;

    // Config migration
    migrate_legacy_config(source, target, dry_run, channels, report)?;

    // Agent migration
    migrate_legacy_agents(source, target, dry_run, report)?;

    // Memory migration
    migrate_legacy_memory(source, target, dry_run, report)?;

    // Workspace migration
    migrate_legacy_workspaces(source, target, dry_run, report)?;

    // Skill scanning
    scan_legacy_skills(source, report);

    info!("Legacy YAML migration complete");
    Ok(())
}

fn migrate_legacy_config(
    source: &Path,
    target: &Path,
    dry_run: bool,
    channels: Option<toml::Value>,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let config_path = source.join("config.yaml");
    if !config_path.exists() {
        report
            .warnings
            .push("No config.yaml found in OpenClaw workspace".to_string());
        return Ok(());
    }

    let yaml_str = std::fs::read_to_string(&config_path)?;
    let oc_config: LegacyYamlConfig = serde_yaml::from_str(&yaml_str)
        .map_err(|e| MigrateError::ConfigParse(format!("config.yaml: {e}")))?;

    let provider = map_provider(&oc_config.provider);
    let api_key_env = oc_config
        .api_key_env
        .unwrap_or_else(|| default_api_key_env(&provider));

    let of_config = LibreFangConfig {
        config_version: CONFIG_VERSION,
        api_listen: DEFAULT_API_LISTEN.to_string(),
        default_model: LibreFangModelConfig {
            provider,
            model: oc_config.model,
            api_key_env,
            base_url: oc_config.base_url,
        },
        memory: LibreFangMemorySection {
            decay_rate: oc_config
                .memory
                .as_ref()
                .and_then(|m| m.decay_rate)
                .unwrap_or(0.05),
        },
        channels,
    };

    let toml_str = toml::to_string_pretty(&of_config)?;

    let config_content = format!(
        "# LibreFang Agent OS configuration\n\
         # Migrated from OpenClaw on {}\n\n\
         {toml_str}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    let dest = target.join("config.toml");

    if !dry_run {
        std::fs::create_dir_all(target)?;
        // Back up any existing config.toml before overwriting.
        write_with_backup(&dest, &config_content, report)?;
    }

    report.imported.push(MigrateItem {
        kind: ItemKind::Config,
        name: "config.yaml".to_string(),
        destination: dest.display().to_string(),
    });

    info!("Migrated config.yaml -> config.toml");
    Ok(())
}

fn parse_legacy_channels(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<Option<toml::Value>, MigrateError> {
    let messaging_dir = source.join("messaging");
    if !messaging_dir.exists() {
        return Ok(None);
    }

    let mut channels_table = toml::map::Map::new();
    // Note: Legacy YAML channels use env var names (bot_token_env), not raw tokens,
    // so no secrets extraction needed. target/dry_run reserved for future use.
    let _ = (target, dry_run);

    for name in &[
        "telegram",
        "discord",
        "slack",
        "whatsapp",
        "signal",
        "matrix",
        "irc",
        "mattermost",
        "feishu",
        "googlechat",
        "msteams",
        "imessage",
        "bluebubbles",
    ] {
        let yaml_path = messaging_dir.join(format!("{name}.yaml"));
        if !yaml_path.exists() {
            continue;
        }

        let yaml_str = std::fs::read_to_string(&yaml_path)?;
        let ch: LegacyYamlChannelConfig = serde_yaml::from_str(&yaml_str).unwrap_or_default();

        match *name {
            "telegram" => {
                let token_env = ch
                    .bot_token_env
                    .unwrap_or_else(|| "TELEGRAM_BOT_TOKEN".to_string());
                let mut fields: Vec<(&str, toml::Value)> =
                    vec![("bot_token_env", toml::Value::String(token_env))];
                if !ch.allowed_users.is_empty() {
                    let arr: Vec<toml::Value> = ch
                        .allowed_users
                        .iter()
                        .map(|u| toml::Value::String(u.clone()))
                        .collect();
                    fields.push(("allowed_users", toml::Value::Array(arr)));
                }
                if let Some(ref da) = ch.default_agent {
                    fields.push(("default_agent", toml::Value::String(da.clone())));
                }
                channels_table.insert(
                    "telegram".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "telegram".to_string(),
                    destination: "config.toml [channels.telegram]".to_string(),
                });
            }
            "discord" => {
                let token_env = ch
                    .bot_token_env
                    .unwrap_or_else(|| "DISCORD_BOT_TOKEN".to_string());
                let mut fields: Vec<(&str, toml::Value)> =
                    vec![("bot_token_env", toml::Value::String(token_env))];
                if let Some(ref da) = ch.default_agent {
                    fields.push(("default_agent", toml::Value::String(da.clone())));
                }
                channels_table.insert(
                    "discord".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "discord".to_string(),
                    destination: "config.toml [channels.discord]".to_string(),
                });
            }
            "slack" => {
                let token_env = ch
                    .bot_token_env
                    .unwrap_or_else(|| "SLACK_BOT_TOKEN".to_string());
                let mut fields: Vec<(&str, toml::Value)> =
                    vec![("bot_token_env", toml::Value::String(token_env))];
                if let Some(ref app_tok) = ch.app_token_env {
                    fields.push(("app_token_env", toml::Value::String(app_tok.clone())));
                }
                if let Some(ref da) = ch.default_agent {
                    fields.push(("default_agent", toml::Value::String(da.clone())));
                }
                channels_table.insert("slack".to_string(), build_channel_table(fields, None, None));
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "slack".to_string(),
                    destination: "config.toml [channels.slack]".to_string(),
                });
            }
            "whatsapp" => {
                let token_env = ch
                    .access_token_env
                    .clone()
                    .unwrap_or_else(|| "WHATSAPP_ACCESS_TOKEN".to_string());
                let fields: Vec<(&str, toml::Value)> =
                    vec![("access_token_env", toml::Value::String(token_env))];
                channels_table.insert(
                    "whatsapp".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "whatsapp".to_string(),
                    destination: "config.toml [channels.whatsapp]".to_string(),
                });
            }
            "signal" => {
                let fields: Vec<(&str, toml::Value)> = vec![(
                    "api_url",
                    toml::Value::String("http://localhost:8080".into()),
                )];
                channels_table.insert(
                    "signal".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "signal".to_string(),
                    destination: "config.toml [channels.signal]".to_string(),
                });
            }
            "matrix" => {
                let token_env = ch
                    .access_token_env
                    .clone()
                    .unwrap_or_else(|| "MATRIX_ACCESS_TOKEN".to_string());
                let fields: Vec<(&str, toml::Value)> =
                    vec![("access_token_env", toml::Value::String(token_env))];
                channels_table.insert(
                    "matrix".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "matrix".to_string(),
                    destination: "config.toml [channels.matrix]".to_string(),
                });
            }
            "irc" => {
                let mut fields: Vec<(&str, toml::Value)> = Vec::new();
                if let Some(ref tok) = ch.bot_token_env {
                    fields.push(("password_env", toml::Value::String(tok.clone())));
                }
                channels_table.insert("irc".to_string(), build_channel_table(fields, None, None));
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "irc".to_string(),
                    destination: "config.toml [channels.irc]".to_string(),
                });
            }
            "mattermost" => {
                let token_env = ch
                    .bot_token_env
                    .unwrap_or_else(|| "MATTERMOST_TOKEN".to_string());
                let fields: Vec<(&str, toml::Value)> =
                    vec![("token_env", toml::Value::String(token_env))];
                channels_table.insert(
                    "mattermost".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "mattermost".to_string(),
                    destination: "config.toml [channels.mattermost]".to_string(),
                });
            }
            "feishu" => {
                let fields: Vec<(&str, toml::Value)> = vec![(
                    "app_secret_env",
                    toml::Value::String("FEISHU_APP_SECRET".into()),
                )];
                channels_table.insert(
                    "feishu".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "feishu".to_string(),
                    destination: "config.toml [channels.feishu]".to_string(),
                });
            }
            "googlechat" => {
                let fields: Vec<(&str, toml::Value)> = vec![(
                    "service_account_env",
                    toml::Value::String("GOOGLE_CHAT_SA_FILE".into()),
                )];
                channels_table.insert(
                    "google_chat".to_string(),
                    build_channel_table(fields, None, None),
                );
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "google_chat".to_string(),
                    destination: "config.toml [channels.google_chat]".to_string(),
                });
            }
            "msteams" => {
                let fields: Vec<(&str, toml::Value)> = vec![(
                    "app_password_env",
                    toml::Value::String("TEAMS_APP_PASSWORD".into()),
                )];
                channels_table.insert("teams".to_string(), build_channel_table(fields, None, None));
                report.imported.push(MigrateItem {
                    kind: ItemKind::Channel,
                    name: "teams".to_string(),
                    destination: "config.toml [channels.teams]".to_string(),
                });
            }
            "imessage" => {
                report.skipped.push(SkippedItem {
                    kind: ItemKind::Channel,
                    name: "imessage".to_string(),
                    reason: "macOS-only channel — requires manual setup on the target Mac"
                        .to_string(),
                });
            }
            "bluebubbles" => {
                report.skipped.push(SkippedItem {
                    kind: ItemKind::Channel,
                    name: "bluebubbles".to_string(),
                    reason: "No LibreFang adapter available — consider using the iMessage channel instead".to_string(),
                });
            }
            _ => {}
        }
    }

    if channels_table.is_empty() {
        Ok(None)
    } else {
        Ok(Some(toml::Value::Table(channels_table)))
    }
}

fn migrate_legacy_agents(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let agents_dir = source.join("agents");
    if !agents_dir.exists() {
        report
            .warnings
            .push("No agents/ directory found".to_string());
        return Ok(());
    }

    let entries = std::fs::read_dir(&agents_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let agent_yaml = path.join("agent.yaml");
        if !agent_yaml.exists() {
            continue;
        }

        let agent_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // #3794 — Validate agent name derived from the filesystem path.
        if let Err(e) = validate_migration_id(&agent_name) {
            warn!("Skipping agent with unsafe name {agent_name:?}: {e}");
            report.skipped.push(SkippedItem {
                kind: ItemKind::Agent,
                name: agent_name,
                reason: e.to_string(),
            });
            continue;
        }

        match convert_legacy_agent(&agent_yaml, &agent_name) {
            Ok((toml_str, unmapped_tools)) => {
                let dest_dir = target.join("agents").join(&agent_name);
                let dest_file = dest_dir.join("agent.toml");

                if !dry_run {
                    std::fs::create_dir_all(&dest_dir)?;
                    // Back up any existing agent.toml before overwriting.
                    write_with_backup(&dest_file, &toml_str, report)?;
                }

                report.imported.push(MigrateItem {
                    kind: ItemKind::Agent,
                    name: agent_name.clone(),
                    destination: dest_file.display().to_string(),
                });

                for tool in &unmapped_tools {
                    report.warnings.push(format!(
                        "Agent '{agent_name}': tool '{tool}' has no LibreFang equivalent and was skipped"
                    ));
                }

                info!("Migrated agent: {agent_name}");
            }
            Err(e) => {
                warn!("Failed to migrate agent {agent_name}: {e}");
                report.skipped.push(SkippedItem {
                    kind: ItemKind::Agent,
                    name: agent_name,
                    reason: e.to_string(),
                });
            }
        }
    }

    Ok(())
}

fn convert_legacy_agent(
    yaml_path: &Path,
    name: &str,
) -> Result<(String, Vec<String>), MigrateError> {
    let yaml_str = std::fs::read_to_string(yaml_path)?;
    let oc: LegacyYamlAgent = serde_yaml::from_str(&yaml_str)
        .map_err(|e| MigrateError::AgentParse(format!("{name}: {e}")))?;

    // Map tools
    let mut unmapped_tools = Vec::new();
    let tools: Vec<String> = if !oc.tools.is_empty() {
        let mut mapped = Vec::new();
        for t in &oc.tools {
            if is_known_librefang_tool(t) {
                mapped.push(t.clone());
            } else if let Some(of_name) = map_tool_name(t) {
                mapped.push(of_name.to_string());
            } else {
                unmapped_tools.push(t.clone());
            }
        }
        mapped
    } else if let Some(ref profile) = oc.tool_profile {
        tools_for_profile(profile)
    } else {
        vec!["file_read".into(), "file_list".into(), "web_fetch".into()]
    };

    let caps = derive_capabilities(&tools);

    let provider = oc
        .provider
        .map(|p| map_provider(&p))
        .unwrap_or_else(|| "anthropic".to_string());

    let model = oc
        .model
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

    let system_prompt = oc.system_prompt.unwrap_or_else(|| {
        format!(
            "You are {}, an AI agent running on the LibreFang Agent OS. {}",
            oc.name,
            if oc.description.is_empty() {
                "You are helpful, concise, and accurate.".to_string()
            } else {
                oc.description.clone()
            }
        )
    });

    let api_key_env = oc.api_key_env.or_else(|| {
        let env = default_api_key_env(&provider);
        if env.is_empty() {
            None
        } else {
            Some(env)
        }
    });

    let mut toml_str = String::new();
    toml_str.push_str(&format!(
        "# LibreFang agent manifest\n# Migrated from OpenClaw agent '{}'\n\n",
        oc.name
    ));
    toml_str.push_str(&format!("name = \"{}\"\n", oc.name));
    toml_str.push_str(&format!("version = \"{}\"\n", librefang_types::VERSION));
    toml_str.push_str(&format!(
        "description = \"{}\"\n",
        oc.description.replace('"', "\\\"")
    ));
    toml_str.push_str("author = \"librefang\"\n");
    toml_str.push_str("module = \"builtin:chat\"\n");

    if !oc.tags.is_empty() {
        let tags_str: Vec<String> = oc.tags.iter().map(|t| format!("\"{t}\"")).collect();
        toml_str.push_str(&format!("tags = [{}]\n", tags_str.join(", ")));
    }

    toml_str.push_str("\n[model]\n");
    toml_str.push_str(&format!("provider = \"{provider}\"\n"));
    toml_str.push_str(&format!("model = \"{model}\"\n"));
    toml_str.push_str(&format!(
        "system_prompt = \"\"\"\n{system_prompt}\n\"\"\"\n"
    ));

    if let Some(ref api_key) = api_key_env {
        toml_str.push_str(&format!("api_key_env = \"{api_key}\"\n"));
    }
    if let Some(base_url) = oc.base_url {
        toml_str.push_str(&format!("base_url = \"{base_url}\"\n"));
    }

    toml_str.push_str("\n[capabilities]\n");
    let tools_str: Vec<String> = tools.iter().map(|t| format!("\"{t}\"")).collect();
    toml_str.push_str(&format!("tools = [{}]\n", tools_str.join(", ")));
    toml_str.push_str("memory_read = [\"*\"]\n");
    toml_str.push_str("memory_write = [\"self.*\"]\n");

    if !caps.network.is_empty() {
        let net_str: Vec<String> = caps.network.iter().map(|n| format!("\"{n}\"")).collect();
        toml_str.push_str(&format!("network = [{}]\n", net_str.join(", ")));
    }
    if !caps.shell.is_empty() {
        let shell_str: Vec<String> = caps.shell.iter().map(|s| format!("\"{s}\"")).collect();
        toml_str.push_str(&format!("shell = [{}]\n", shell_str.join(", ")));
    }
    if !caps.agent_message.is_empty() {
        let msg_str: Vec<String> = caps
            .agent_message
            .iter()
            .map(|m| format!("\"{m}\""))
            .collect();
        toml_str.push_str(&format!("agent_message = [{}]\n", msg_str.join(", ")));
    }
    if caps.agent_spawn {
        toml_str.push_str("agent_spawn = true\n");
    }

    Ok((toml_str, unmapped_tools))
}

fn migrate_legacy_memory(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let agents_dir = source.join("agents");
    if !agents_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(&agents_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let memory_md = path.join("MEMORY.md");
        if !memory_md.exists() {
            continue;
        }

        let agent_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let content = std::fs::read_to_string(&memory_md)?;
        if content.trim().is_empty() {
            continue;
        }

        let dest_dir = target.join("agents").join(&agent_name);
        let dest_file = dest_dir.join("imported_memory.md");

        if !dry_run {
            std::fs::create_dir_all(&dest_dir)?;
            // Back up any existing imported_memory.md before overwriting.
            write_with_backup(&dest_file, &content, report)?;
        }

        report.imported.push(MigrateItem {
            kind: ItemKind::Memory,
            name: format!("{agent_name}/MEMORY.md"),
            destination: dest_file.display().to_string(),
        });
    }

    Ok(())
}

fn migrate_legacy_workspaces(
    source: &Path,
    target: &Path,
    dry_run: bool,
    report: &mut MigrationReport,
) -> Result<(), MigrateError> {
    let agents_dir = source.join("agents");
    if !agents_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(&agents_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let workspace_dir = path.join("workspace");
        if !workspace_dir.exists() || !workspace_dir.is_dir() {
            continue;
        }

        let agent_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let file_count = walkdir::WalkDir::new(&workspace_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .count();

        if file_count == 0 {
            continue;
        }

        let dest_dir = target.join("agents").join(&agent_name).join("workspace");

        if !dry_run {
            copy_dir_recursive(&workspace_dir, &dest_dir)?;
        }

        report.imported.push(MigrateItem {
            kind: ItemKind::Session,
            name: format!("{agent_name}/workspace ({file_count} files)"),
            destination: dest_dir.display().to_string(),
        });
    }

    Ok(())
}

fn scan_legacy_skills(source: &Path, report: &mut MigrationReport) {
    let skills_dir = source.join("skills");
    if !skills_dir.exists() {
        return;
    }

    let mut scan_subdir = |subdir: &Path| {
        if let Ok(entries) = std::fs::read_dir(subdir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let has_package_json = path.join("package.json").exists();
                let has_index = path.join("index.ts").exists() || path.join("index.js").exists();

                if has_package_json && has_index {
                    report.skipped.push(SkippedItem {
                        kind: ItemKind::Skill,
                        name: name.clone(),
                        reason:
                            "Node.js skill — run with `librefang skill install` after migration"
                                .to_string(),
                    });
                } else {
                    report.skipped.push(SkippedItem {
                        kind: ItemKind::Skill,
                        name,
                        reason: "Unknown skill format".to_string(),
                    });
                }
            }
        }
    };

    scan_subdir(&skills_dir.join("community"));
    scan_subdir(&skills_dir.join("custom"));
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ===== Helper: create legacy YAML workspace =====

    fn create_legacy_yaml_workspace(dir: &Path) {
        // config.yaml
        std::fs::write(
            dir.join("config.yaml"),
            "provider: anthropic\nmodel: canonical-id-one\napi_key_env: ANTHROPIC_API_KEY\n",
        )
        .unwrap();

        // agents/coder/agent.yaml
        let agent_dir = dir.join("agents").join("coder");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("agent.yaml"),
            "name: coder\ndescription: A coding assistant\ntools:\n  - read_file\n  - write_file\n  - execute_command\ntags:\n  - coding\n  - dev\n",
        ).unwrap();

        // agents/coder/MEMORY.md
        std::fs::write(
            agent_dir.join("MEMORY.md"),
            "## Project Context\n- Working on a Rust project\n- Uses async/await\n",
        )
        .unwrap();

        // messaging/telegram.yaml
        let msg_dir = dir.join("messaging");
        std::fs::create_dir_all(&msg_dir).unwrap();
        std::fs::write(
            msg_dir.join("telegram.yaml"),
            "type: telegram\nbot_token_env: TELEGRAM_BOT_TOKEN\ndefault_agent: coder\n",
        )
        .unwrap();
    }

    // ===== Helper: create JSON5 workspace =====

    fn create_json5_workspace(dir: &Path) {
        let json5_content = r##"{
  agents: {
    defaults: {
      model: "provider-a/canonical-id-one",
      tools: { profile: "coding" }
    },
    list: [
      {
        id: "coder",
        name: "Coder",
        model: {
          primary: "deepseek/deepseek-chat",
          fallbacks: ["groq/llama-3.3-70b-versatile", "anthropic/claude-haiku-4-5-20251001"]
        },
        tools: { allow: ["Read", "Write", "Bash", "WebSearch"] },
        identity: "You are an expert software engineer."
      },
      {
        id: "researcher",
        model: "google/gemini-2.5-flash",
        tools: { profile: "research" }
      }
    ]
  },
  channels: {
    telegram: {
      botToken: "123:ABC",
      allowFrom: ["user1", "user2"],
      groupPolicy: "open",
      dmPolicy: "allowlist"
    },
    discord: {
      token: "discord-token-here",
      enabled: true,
      dmPolicy: "open"
    },
    slack: {
      botToken: "xoxb-slack",
      appToken: "xapp-slack"
    },
    whatsapp: {
      dmPolicy: "open",
      allowFrom: ["phone1"],
      groupPolicy: "disabled"
    },
    signal: {
      httpHost: "signal-api.local",
      httpPort: 9090,
      account: "+15551234567"
    },
    matrix: {
      homeserver: "https://matrix.example.com",
      userId: "@bot:example.com",
      accessToken: "syt_matrix_token_xyz"
    },
    irc: {
      host: "irc.libera.chat",
      port: 6697,
      tls: true,
      nick: "librefang-bot",
      password: "irc-secret-pw",
      channels: ["#dev", "#general"]
    },
    mattermost: {
      botToken: "mm-token-abc",
      baseUrl: "https://mm.example.com"
    },
    feishu: {
      appId: "cli_feishu123",
      appSecret: "feishu-secret-xyz",
      domain: "example.feishu.cn"
    },
    googlechat: {
      webhookPath: "/webhook/gchat",
      dmPolicy: "open"
    },
    msteams: {
      appId: "teams-app-id-123",
      appPassword: "teams-pw-secret",
      tenantId: "tenant-uuid"
    },
    imessage: {
      cliPath: "/usr/local/bin/imessage-cli"
    },
    bluebubbles: {
      serverUrl: "http://localhost:1234",
      password: "bb-pw"
    }
  },
  cron: { enabled: true },
  hooks: { enabled: true, mappings: [] },
  skills: {
    entries: {
      "web-scraper": {},
      "pdf-reader": {}
    }
  },
  auth: {
    profiles: { "default": { apiKey: "sk-xxx" } }
  },
  memory: { backend: "builtin" },
  session: { scope: "per-sender" }
}"##;

        std::fs::write(dir.join("openclaw.json"), json5_content).unwrap();

        // Physical memory dirs
        let mem_coder = dir.join("memory").join("coder");
        std::fs::create_dir_all(&mem_coder).unwrap();
        std::fs::write(
            mem_coder.join("MEMORY.md"),
            "## Coder Memory\n- Prefers Rust\n",
        )
        .unwrap();

        let mem_researcher = dir.join("memory").join("researcher");
        std::fs::create_dir_all(&mem_researcher).unwrap();
        std::fs::write(
            mem_researcher.join("MEMORY.md"),
            "## Researcher Memory\n- Uses academic sources\n",
        )
        .unwrap();

        // Sessions
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(
            sessions_dir.join("main.jsonl"),
            "{\"role\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        std::fs::write(
            sessions_dir.join("agent_coder_main.jsonl"),
            "{\"role\":\"user\",\"content\":\"write code\"}\n",
        )
        .unwrap();

        // Workspaces
        let ws_coder = dir.join("workspaces").join("coder");
        std::fs::create_dir_all(&ws_coder).unwrap();
        std::fs::write(ws_coder.join("main.rs"), "fn main() {}").unwrap();
    }

    // ================================================================
    // JSON5 tests (new)
    // ================================================================

    #[test]
    fn test_json5_full_migration() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // Config imported
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Config));
        assert!(target.path().join("config.toml").exists());

        // Agents imported
        let agent_items: Vec<_> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Agent)
            .collect();
        assert_eq!(agent_items.len(), 2);
        assert!(target.path().join("agents/coder/agent.toml").exists());
        assert!(target.path().join("agents/researcher/agent.toml").exists());

        // Channels imported (11 supported channels from fixture)
        let channel_items: Vec<_> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Channel)
            .collect();
        assert_eq!(channel_items.len(), 11); // 13 - imessage - bluebubbles

        let config_toml = std::fs::read_to_string(target.path().join("config.toml")).unwrap();
        assert!(config_toml.contains("[channels.telegram]"));
        assert!(config_toml.contains("[channels.discord]"));
        assert!(config_toml.contains("[channels.slack]"));
        assert!(config_toml.contains("[channels.whatsapp]"));
        assert!(config_toml.contains("[channels.signal]"));
        assert!(config_toml.contains("[channels.matrix]"));
        assert!(config_toml.contains("[channels.irc]"));
        assert!(config_toml.contains("[channels.mattermost]"));
        assert!(config_toml.contains("[channels.feishu]"));
        assert!(config_toml.contains("[channels.teams]"));
        assert!(
            config_toml.contains("[channels.google_chat]"),
            "missing google_chat in config: {config_toml}"
        );

        // Secrets extracted
        let secret_items: Vec<_> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Secret)
            .collect();
        assert!(
            secret_items.len() >= 7,
            "expected >=7 secrets, got {}",
            secret_items.len()
        );
        assert!(target.path().join("secrets.env").exists());

        let secrets = std::fs::read_to_string(target.path().join("secrets.env")).unwrap();
        assert!(secrets.contains("TELEGRAM_BOT_TOKEN=123:ABC"));
        assert!(secrets.contains("DISCORD_BOT_TOKEN=discord-token-here"));
        assert!(secrets.contains("SLACK_BOT_TOKEN=xoxb-slack"));
        assert!(secrets.contains("MATRIX_ACCESS_TOKEN=syt_matrix_token_xyz"));
        assert!(secrets.contains("IRC_PASSWORD=irc-secret-pw"));
        assert!(secrets.contains("MATTERMOST_TOKEN=mm-token-abc"));
        assert!(secrets.contains("FEISHU_APP_SECRET=feishu-secret-xyz"));
        assert!(secrets.contains("TEAMS_APP_PASSWORD=teams-pw-secret"));

        // NO raw tokens in config.toml
        assert!(
            !config_toml.contains("123:ABC"),
            "raw token leaked into config.toml"
        );
        assert!(
            !config_toml.contains("discord-token-here"),
            "raw token leaked into config.toml"
        );
        assert!(
            !config_toml.contains("xoxb-slack"),
            "raw token leaked into config.toml"
        );
        assert!(
            !config_toml.contains("syt_matrix_token_xyz"),
            "raw token leaked into config.toml"
        );

        // Skipped channels reported
        assert!(report.skipped.iter().any(|s| s.name == "imessage"));
        assert!(report.skipped.iter().any(|s| s.name == "bluebubbles"));

        // Memory imported
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Memory));
        assert!(target
            .path()
            .join("agents/coder/imported_memory.md")
            .exists());
        assert!(target
            .path()
            .join("agents/researcher/imported_memory.md")
            .exists());

        // Sessions imported
        assert!(report
            .imported
            .iter()
            .any(|i| i.kind == ItemKind::Session && i.name.contains("session")));
        assert!(target.path().join("imported_sessions/main.jsonl").exists());

        // Workspace imported
        assert!(report
            .imported
            .iter()
            .any(|i| i.kind == ItemKind::Session && i.name.contains("workspace")));

        // Skipped features reported
        assert!(report.skipped.iter().any(|s| s.name == "cron"));
        assert!(report.skipped.iter().any(|s| s.name == "hooks"));
        assert!(report.skipped.iter().any(|s| s.name == "auth-profiles"));
        assert!(report.skipped.iter().any(|s| s.name.contains("skill")));

        // Report file
        assert!(target.path().join("migration_report.md").exists());
    }

    /// Round-trip: the migrated `config.toml` and `agent.toml` must parse
    /// cleanly into the real `KernelConfig` / `AgentManifest` types from
    /// `librefang-types`. If any field we emit has drifted from the real
    /// schema (wrong name, wrong type, removed field), this test fails —
    /// that's the whole point. It's the structural guardrail that the
    /// manual-string-building tests don't provide.
    #[test]
    fn test_roundtrip_migrate_output_into_real_structs() {
        use librefang_types::agent::AgentManifest;
        use librefang_types::config::KernelConfig;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        // Minimal OpenClaw JSON5 workspace — exercises every channel code
        // path plus one agent with identity + tool profile.
        // (r##"..."## used because the fixture contains `"#test"` for IRC.)
        let json5 = r##"{
  channels: {
    telegram: {
      botToken: "tg-token",
      dmPolicy: "allowlist",
      groupPolicy: "open",
      allowFrom: ["123", "456"]
    },
    discord: { token: "dc-token", allowFrom: ["user-1"] },
    slack: { botToken: "xoxb", appToken: "xapp" },
    whatsapp: { allowFrom: ["+1555"] },
    signal: { httpHost: "localhost", httpPort: 8080, account: "+1555", allowFrom: ["+1556"] },
    matrix: { homeserver: "https://matrix.org", userId: "@bot:matrix.org", accessToken: "mx-token", rooms: ["!room:matrix.org"] },
    irc: { host: "irc.libera.chat", port: 6697, nick: "bot", tls: true, password: "pw", channels: ["#test"] },
    mattermost: { botToken: "mm-token", baseUrl: "https://mm.example" },
    feishu: { appId: "app1", appSecret: "sec1", domain: "lark.com" },
    teams: { appId: "teams-app", appPassword: "teams-pw", tenantId: "tenant-xyz" },
    googleChat: { serviceAccountFile: "/nonexistent/sa.json" }
  },
  agents: {
    list: [
      {
        id: "coder",
        name: "Coder",
        model: "provider-a/canonical-id-one",
        tools: { profile: "coding" },
        identity: "You are a coding assistant."
      }
    ]
  }
}"##;
        std::fs::write(source.path().join("openclaw.json"), json5).unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };
        let _ = migrate(&options).unwrap();

        // ---- config.toml round-trip ----
        let config_str = std::fs::read_to_string(target.path().join("config.toml")).unwrap();

        // 1. Parse as raw TOML to detect unknown top-level fields.
        let raw: toml::Value = toml::from_str(&config_str).unwrap_or_else(|e| {
            panic!("migrated config.toml is not valid TOML: {e}\n\n{config_str}")
        });
        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert!(
            unknown.is_empty(),
            "migrate wrote unknown top-level fields to config.toml: {unknown:?}\n\n{config_str}"
        );

        // 2. Parse into the real KernelConfig — fails on type mismatches.
        let cfg: KernelConfig = toml::from_str(&config_str).unwrap_or_else(|e| {
            panic!(
                "migrated config.toml does not deserialize into KernelConfig: {e}\n\n{config_str}"
            )
        });

        // 3. api_listen went to the right place.
        assert!(!cfg.api_listen.is_empty(), "api_listen must be populated");

        // 4. config_version is current (no best-effort migration needed).
        assert_eq!(
            cfg.config_version,
            librefang_types::config::CONFIG_VERSION,
            "migrate must stamp the current CONFIG_VERSION"
        );

        // 5. Channel top-level allowlists are populated (not stuffed into overrides).
        let tg = cfg
            .channels
            .telegram
            .iter()
            .next()
            .expect("telegram configured");
        assert_eq!(tg.allowed_users, vec!["123".to_string(), "456".to_string()]);
        let dc = cfg
            .channels
            .discord
            .iter()
            .next()
            .expect("discord configured");
        assert_eq!(dc.allowed_users, vec!["user-1".to_string()]);
        let wa = cfg
            .channels
            .whatsapp
            .iter()
            .next()
            .expect("whatsapp configured");
        assert_eq!(wa.allowed_users, vec!["+1555".to_string()]);
        let sig = cfg
            .channels
            .signal
            .iter()
            .next()
            .expect("signal configured");
        assert_eq!(sig.allowed_users, vec!["+1556".to_string()]);
        let mx = cfg
            .channels
            .matrix
            .iter()
            .next()
            .expect("matrix configured");
        assert_eq!(mx.allowed_rooms, vec!["!room:matrix.org".to_string()]);

        // 6. Policy mappings land on valid enum variants (no "respond" on group_policy).
        use librefang_types::config::{DmPolicy, GroupPolicy};
        assert_eq!(tg.overrides.dm_policy, DmPolicy::AllowedOnly);
        assert_eq!(tg.overrides.group_policy, GroupPolicy::All);

        // 7. Per-struct field-name corrections.
        let irc = cfg.channels.irc.iter().next().expect("irc configured");
        assert_eq!(irc.nick, "bot"); // was "nickname" before the fix
        let mm = cfg
            .channels
            .mattermost
            .iter()
            .next()
            .expect("mattermost configured");
        assert_eq!(mm.token_env, "MATTERMOST_TOKEN"); // was "bot_token_env" before
        let teams = cfg.channels.teams.iter().next().expect("teams configured");
        assert_eq!(teams.allowed_tenants, vec!["tenant-xyz".to_string()]); // was "tenant_id" scalar before
        let feishu = cfg
            .channels
            .feishu
            .iter()
            .next()
            .expect("feishu configured");
        assert_eq!(feishu.region, "intl"); // domain "lark.com" → region "intl"

        // ---- agent.toml round-trip ----
        let agent_str =
            std::fs::read_to_string(target.path().join("agents/coder/agent.toml")).unwrap();
        let manifest: AgentManifest = toml::from_str(&agent_str).unwrap_or_else(|e| {
            panic!(
                "migrated agent.toml does not deserialize into AgentManifest: {e}\n\n{agent_str}"
            )
        });

        // profile is a root-level field, not inside [capabilities].
        assert!(
            manifest.profile.is_some(),
            "agent.toml 'profile' must be at root, not buried inside [capabilities]\n\n{agent_str}"
        );
        assert_eq!(manifest.name, "Coder");
        assert!(manifest
            .capabilities
            .tools
            .contains(&"file_read".to_string()));
    }

    #[test]
    fn test_json5_agent_model_parsing() {
        // Pure parser tests — model ids are placeholders so the assertions
        // don't track which Sonnet / Gemini / DeepSeek id is canonical
        // in the registry this week. The "no slash fallback" case still
        // pins the provider to "anthropic" because that's the documented
        // default-provider behaviour of split_model_ref, not a catalog fact.
        let (p, m) = split_model_ref("provider-a/canonical-id-one");
        assert_eq!(p, "provider-a");
        assert_eq!(m, "canonical-id-one");

        let (p, m) = split_model_ref("provider-b/canonical-id-two");
        assert_eq!(p, "provider-b");
        assert_eq!(m, "canonical-id-two");

        // No slash fallback
        let (p, m) = split_model_ref("bare-id");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "bare-id");

        // Detailed model
        let json_str =
            r#"{ "primary": "provider-c/primary-id", "fallbacks": ["provider-d/fallback-id"] }"#;
        let model: OpenClawAgentModel = serde_json::from_str(json_str).unwrap();
        match model {
            OpenClawAgentModel::Detailed(d) => {
                assert_eq!(d.primary.unwrap(), "provider-c/primary-id");
                assert_eq!(d.fallbacks.len(), 1);
            }
            _ => panic!("Expected Detailed variant"),
        }

        // Simple model (string)
        let json_str = r#""provider-a/canonical-id-one""#;
        let model: OpenClawAgentModel = serde_json::from_str(json_str).unwrap();
        match model {
            OpenClawAgentModel::Simple(s) => {
                assert_eq!(s, "provider-a/canonical-id-one");
            }
            _ => panic!("Expected Simple variant"),
        }
    }

    #[test]
    fn test_json5_channel_extraction() {
        let target = TempDir::new().unwrap();
        let json5_content = r#"{
  channels: {
    telegram: { botToken: "123", allowFrom: ["alice"], enabled: true },
    discord: { token: "abc", enabled: true },
    slack: { botToken: "xoxb", appToken: "xapp" }
  }
}"#;
        let root: OpenClawRoot = json5::from_str(json5_content).unwrap();
        let mut report = MigrationReport::default();

        let channels = migrate_channels_from_json(&root, target.path(), false, &mut report);
        assert!(channels.is_some());
        let ch = channels.unwrap();
        let ch_table = ch.as_table().unwrap();
        assert!(ch_table.contains_key("telegram"));
        assert!(ch_table.contains_key("discord"));
        assert!(ch_table.contains_key("slack"));

        // Check telegram has allowed_users and bot_token_env
        let tg = ch_table["telegram"].as_table().unwrap();
        assert_eq!(tg["bot_token_env"].as_str().unwrap(), "TELEGRAM_BOT_TOKEN");
        let users = tg["allowed_users"].as_array().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].as_str().unwrap(), "alice");

        // 3 channel imports
        assert_eq!(
            report
                .imported
                .iter()
                .filter(|i| i.kind == ItemKind::Channel)
                .count(),
            3
        );

        // 4 secrets extracted (telegram + discord + slack bot + slack app)
        assert_eq!(
            report
                .imported
                .iter()
                .filter(|i| i.kind == ItemKind::Secret)
                .count(),
            4
        );

        // Secrets file written
        let secrets = std::fs::read_to_string(target.path().join("secrets.env")).unwrap();
        assert!(secrets.contains("TELEGRAM_BOT_TOKEN=123"));
        assert!(secrets.contains("DISCORD_BOT_TOKEN=abc"));
        assert!(secrets.contains("SLACK_BOT_TOKEN=xoxb"));
    }

    #[test]
    fn test_json5_fallback_models() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        migrate(&options).unwrap();

        let coder_toml =
            std::fs::read_to_string(target.path().join("agents/coder/agent.toml")).unwrap();

        // Primary model should be deepseek
        assert!(coder_toml.contains("provider = \"deepseek\""));
        assert!(coder_toml.contains("model = \"deepseek-chat\""));

        // Should have fallback models
        assert!(coder_toml.contains("[[fallback_models]]"));
        assert!(coder_toml.contains("provider = \"groq\""));
        assert!(coder_toml.contains("model = \"llama-3.3-70b-versatile\""));
        assert!(coder_toml.contains("provider = \"anthropic\""));
        assert!(coder_toml.contains("model = \"claude-haiku-4-5-20251001\""));
    }

    #[test]
    fn test_json5_tool_profile_resolution() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        migrate(&options).unwrap();

        // researcher uses profile = "research", should get research tools
        let researcher_toml =
            std::fs::read_to_string(target.path().join("agents/researcher/agent.toml")).unwrap();
        assert!(researcher_toml.contains("web_fetch"));
        assert!(researcher_toml.contains("web_search"));
        assert!(researcher_toml.contains("profile = \"research\""));
    }

    #[test]
    fn test_json5_legacy_yaml_fallback() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_legacy_yaml_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // Should still work with YAML fallback
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Config));
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Agent));
        assert!(target.path().join("config.toml").exists());
        assert!(target.path().join("agents/coder/agent.toml").exists());
    }

    #[test]
    fn test_json5_detect_home() {
        let dir = TempDir::new().unwrap();

        // No config file = should not detect
        assert!(find_config_file(dir.path()).is_none());

        // With openclaw.json
        std::fs::write(dir.path().join("openclaw.json"), "{}").unwrap();
        let found = find_config_file(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("openclaw.json"));

        // Legacy clawdbot.json
        let dir2 = TempDir::new().unwrap();
        std::fs::write(dir2.path().join("clawdbot.json"), "{}").unwrap();
        let found = find_config_file(dir2.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("clawdbot.json"));

        // config.yaml (legacy)
        let dir3 = TempDir::new().unwrap();
        std::fs::write(dir3.path().join("config.yaml"), "provider: anthropic\n").unwrap();
        let found = find_config_file(dir3.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("config.yaml"));
    }

    #[test]
    fn test_json5_session_migration() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        migrate(&options).unwrap();

        let imported_dir = target.path().join("imported_sessions");
        assert!(imported_dir.exists());
        assert!(imported_dir.join("main.jsonl").exists());
        assert!(imported_dir.join("agent_coder_main.jsonl").exists());

        // Verify content preserved
        let content = std::fs::read_to_string(imported_dir.join("main.jsonl")).unwrap();
        assert!(content.contains("hello"));
    }

    #[test]
    fn test_json5_memory_both_layouts() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        // Create JSON5 config with agents
        let json5_content = r#"{
  agents: {
    list: [
      { id: "agent1" },
      { id: "agent2" }
    ]
  }
}"#;
        std::fs::write(source.path().join("openclaw.json"), json5_content).unwrap();

        // Layout 1: memory/<agent>/MEMORY.md
        let mem1 = source.path().join("memory").join("agent1");
        std::fs::create_dir_all(&mem1).unwrap();
        std::fs::write(mem1.join("MEMORY.md"), "Memory from layout 1").unwrap();

        // Layout 2: agents/<agent>/MEMORY.md (legacy)
        let mem2 = source.path().join("agents").join("agent2");
        std::fs::create_dir_all(&mem2).unwrap();
        std::fs::write(mem2.join("MEMORY.md"), "Memory from layout 2").unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        let memory_items: Vec<_> = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Memory)
            .collect();
        assert_eq!(memory_items.len(), 2);

        assert!(target
            .path()
            .join("agents/agent1/imported_memory.md")
            .exists());
        assert!(target
            .path()
            .join("agents/agent2/imported_memory.md")
            .exists());

        let c1 = std::fs::read_to_string(target.path().join("agents/agent1/imported_memory.md"))
            .unwrap();
        assert!(c1.contains("layout 1"));

        let c2 = std::fs::read_to_string(target.path().join("agents/agent2/imported_memory.md"))
            .unwrap();
        assert!(c2.contains("layout 2"));
    }

    #[test]
    fn test_json5_skipped_features() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        let json5_content = r#"{
  cron: { enabled: true },
  hooks: { enabled: true },
  auth: { profiles: { "default": {} } },
  skills: { entries: { "a": {}, "b": {} } },
  memory: { backend: "builtin" },
  session: { scope: "per-sender" }
}"#;
        std::fs::write(source.path().join("openclaw.json"), json5_content).unwrap();

        // Physical files that get skipped
        let cron_dir = source.path().join("cron");
        std::fs::create_dir_all(&cron_dir).unwrap();
        std::fs::write(cron_dir.join("cron-store.json"), "{}").unwrap();

        let mem_search = source.path().join("memory-search");
        std::fs::create_dir_all(&mem_search).unwrap();
        std::fs::write(mem_search.join("index.db"), "sqlite").unwrap();

        std::fs::write(source.path().join("auth-profiles.json"), "{}").unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // All should be in skipped
        assert!(report.skipped.iter().any(|s| s.name == "cron"));
        assert!(report.skipped.iter().any(|s| s.name == "hooks"));
        assert!(report.skipped.iter().any(|s| s.name == "auth-profiles"));
        assert!(report.skipped.iter().any(|s| s.name.contains("skill")));
        assert!(report.skipped.iter().any(|s| s.name == "cron-store.json"));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.name.contains("memory-search")));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.name == "auth-profiles.json"));
        assert!(report.skipped.iter().any(|s| s.name == "session"));
        assert!(report.skipped.iter().any(|s| s.name == "memory"));
    }

    #[test]
    fn test_json5_dry_run() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: true,
        };

        let report = migrate(&options).unwrap();
        assert!(report.dry_run);
        assert!(!report.imported.is_empty());

        // No files created
        assert!(!target.path().join("config.toml").exists());
        assert!(!target.path().join("agents").exists());
        assert!(!target.path().join("imported_sessions").exists());
    }

    #[test]
    fn test_json5_empty_config() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        std::fs::write(source.path().join("openclaw.json"), "{}").unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // Should still produce a config
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Config));
        assert!(target.path().join("config.toml").exists());

        // No agents should be an info, not crash
        assert!(report.warnings.iter().any(|w| w.contains("No agents")));
    }

    #[test]
    fn test_model_ref_split() {
        // Pure split test — provider/model ids are placeholders to keep the
        // assertions from drifting whenever the registry retires a specific
        // model id.
        let (p, m) = split_model_ref("provider-a/canonical-id-one");
        assert_eq!(p, "provider-a");
        assert_eq!(m, "canonical-id-one");

        let (p, m) = split_model_ref("provider-b/canonical-id-two");
        assert_eq!(p, "provider-b");
        assert_eq!(m, "canonical-id-two");

        let (p, m) = split_model_ref("provider-c/canonical-id-three");
        assert_eq!(p, "provider-c");
        assert_eq!(m, "canonical-id-three");

        let (p, m) = split_model_ref("provider-d/canonical-id-four");
        assert_eq!(p, "provider-d");
        assert_eq!(m, "canonical-id-four");

        // No slash
        let (p, m) = split_model_ref("some-model");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "some-model");

        // Empty
        let (p, m) = split_model_ref("");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "");
    }

    #[test]
    fn test_json5_unknown_provider_passthrough() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        let json5_content = r#"{
  agents: {
    list: [
      { id: "test-agent", model: "mycompany/custom-llm-v3" }
    ]
  }
}"#;
        std::fs::write(source.path().join("openclaw.json"), json5_content).unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Agent));

        let agent_toml =
            std::fs::read_to_string(target.path().join("agents/test-agent/agent.toml")).unwrap();
        assert!(agent_toml.contains("provider = \"mycompany\""));
        assert!(agent_toml.contains("model = \"custom-llm-v3\""));
        assert!(agent_toml.contains("api_key_env = \"MYCOMPANY_API_KEY\""));
    }

    #[test]
    fn test_json5_identity_object_parsing() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        let json5_content = r#"{
  agents: {
    defaults: {
      model: "provider-a/canonical-id-one"
    },
    list: [
      {
        id: "admin",
        name: "Admin",
        identity: {
          prompt: {
            text: "You are the admin agent. Keep control-plane changes explicit."
          }
        }
      }
    ]
  }
}"#;
        std::fs::write(source.path().join("openclaw.json"), json5_content).unwrap();

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Agent));

        let agent_toml =
            std::fs::read_to_string(target.path().join("agents/admin/agent.toml")).unwrap();
        assert!(agent_toml.contains("You are the admin agent."));
        assert!(agent_toml.contains("Keep control-plane changes explicit."));
    }

    // ================================================================
    // Existing tests (kept — now test YAML legacy path + shared utils)
    // ================================================================

    #[test]
    fn test_full_migration() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_legacy_yaml_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        assert!(!report.imported.is_empty());
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Config));
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Agent));
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Memory));
        assert!(report.imported.iter().any(|i| i.kind == ItemKind::Channel));

        assert!(target.path().join("config.toml").exists());
        assert!(target.path().join("agents/coder/agent.toml").exists());
        assert!(target
            .path()
            .join("agents/coder/imported_memory.md")
            .exists());

        let agent_toml =
            std::fs::read_to_string(target.path().join("agents/coder/agent.toml")).unwrap();
        assert!(
            agent_toml.contains("shell = [\"*\"]"),
            "shell_exec should derive shell capability"
        );
        assert!(agent_toml.contains("file_read"));
        assert!(agent_toml.contains("file_write"));
        assert!(agent_toml.contains("shell_exec"));

        let config_toml = std::fs::read_to_string(target.path().join("config.toml")).unwrap();
        assert!(config_toml.contains("[channels.telegram]"));
        assert!(!target.path().join("channels_import.toml").exists());

        assert!(target.path().join("migration_report.md").exists());
    }

    /// Round-trip for the **legacy YAML** migration path — parallel to
    /// `test_roundtrip_migrate_output_into_real_structs` which covers the
    /// JSON5 path. `convert_legacy_agent` and `parse_legacy_channels` write
    /// their own TOML by hand, so they need the same structural guardrail.
    #[test]
    fn test_roundtrip_legacy_yaml_output_into_real_structs() {
        use librefang_types::agent::AgentManifest;
        use librefang_types::config::KernelConfig;

        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_legacy_yaml_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };
        let _ = migrate(&options).unwrap();

        // config.toml round-trip
        let config_str = std::fs::read_to_string(target.path().join("config.toml")).unwrap();
        let raw: toml::Value = toml::from_str(&config_str).unwrap_or_else(|e| {
            panic!("legacy config.toml is not valid TOML: {e}\n\n{config_str}")
        });
        let unknown = KernelConfig::detect_unknown_fields(&raw);
        assert!(
            unknown.is_empty(),
            "legacy YAML path wrote unknown top-level fields: {unknown:?}\n\n{config_str}"
        );
        let cfg: KernelConfig = toml::from_str(&config_str).unwrap_or_else(|e| {
            panic!("legacy config.toml does not deserialize into KernelConfig: {e}\n\n{config_str}")
        });
        assert_eq!(cfg.config_version, librefang_types::config::CONFIG_VERSION);
        assert!(!cfg.api_listen.is_empty());

        // agent.toml round-trip — legacy YAML writes `tags` at top level +
        // `base_url` inside [model] (neither is written by the JSON5 path,
        // so this is fresh coverage).
        let agent_str =
            std::fs::read_to_string(target.path().join("agents/coder/agent.toml")).unwrap();
        let manifest: AgentManifest = toml::from_str(&agent_str).unwrap_or_else(|e| {
            panic!("legacy agent.toml does not deserialize into AgentManifest: {e}\n\n{agent_str}")
        });
        assert_eq!(manifest.name, "coder");
        assert_eq!(manifest.module, "builtin:chat");
    }

    #[test]
    fn test_dry_run() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_legacy_yaml_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: true,
        };

        let report = migrate(&options).unwrap();
        assert!(report.dry_run);
        assert!(!report.imported.is_empty());

        assert!(!target.path().join("config.toml").exists());
    }

    #[test]
    fn test_source_not_found() {
        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: "/nonexistent/path".into(),
            target_dir: std::env::temp_dir().join("test_migrate_not_found"),
            dry_run: false,
        };

        let result = migrate(&options);
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_mapping() {
        assert_eq!(map_tool_name("read_file"), Some("file_read"));
        assert_eq!(map_tool_name("write_file"), Some("file_write"));
        assert_eq!(map_tool_name("execute_command"), Some("shell_exec"));
        assert_eq!(map_tool_name("fetch_url"), Some("web_fetch"));
        assert_eq!(map_tool_name("memory_search"), Some("memory_recall"));
        assert_eq!(map_tool_name("unknown_tool"), None);
        // New Claude-style mappings
        assert_eq!(map_tool_name("Read"), Some("file_read"));
        assert_eq!(map_tool_name("Write"), Some("file_write"));
        assert_eq!(map_tool_name("Bash"), Some("shell_exec"));
        assert_eq!(map_tool_name("Glob"), Some("file_list"));
        assert_eq!(map_tool_name("Grep"), Some("file_list"));
        assert_eq!(map_tool_name("WebSearch"), Some("web_search"));
        assert_eq!(map_tool_name("WebFetch"), Some("web_fetch"));
        assert_eq!(map_tool_name("sessions_send"), Some("agent_send"));
        assert_eq!(map_tool_name("sessions_spawn"), Some("agent_send"));
    }

    #[test]
    fn test_provider_mapping() {
        assert_eq!(map_provider("anthropic"), "anthropic");
        assert_eq!(map_provider("claude"), "anthropic");
        assert_eq!(map_provider("openai"), "openai");
        assert_eq!(map_provider("gpt"), "openai");
        assert_eq!(map_provider("groq"), "groq");
        assert_eq!(map_provider("custom"), "custom");
        assert_eq!(map_provider("google"), "google");
        assert_eq!(map_provider("gemini"), "google");
        assert_eq!(map_provider("xai"), "xai");
        assert_eq!(map_provider("grok"), "xai");
    }

    #[test]
    fn test_tools_for_profile() {
        let minimal = tools_for_profile("minimal");
        assert_eq!(minimal.len(), 2);
        assert!(minimal.contains(&"file_read".to_string()));

        let coding = tools_for_profile("coding");
        assert!(coding.contains(&"shell_exec".to_string()));

        let full = tools_for_profile("full");
        assert!(full.contains(&"*".to_string()));

        let automation = tools_for_profile("automation");
        assert!(automation.len() >= 10);
        assert!(automation.contains(&"shell_exec".to_string()));
        assert!(automation.contains(&"web_fetch".to_string()));
    }

    #[test]
    fn test_convert_agent() {
        let dir = TempDir::new().unwrap();
        let yaml_path = dir.path().join("agent.yaml");
        std::fs::write(
            &yaml_path,
            "name: test-agent\ndescription: Test\ntools:\n  - read_file\n  - web_search\n",
        )
        .unwrap();

        let (toml_str, unmapped) = convert_legacy_agent(&yaml_path, "test-agent").unwrap();
        assert!(toml_str.contains("name = \"test-agent\""));
        assert!(toml_str.contains("file_read"));
        assert!(toml_str.contains("web_search"));
        assert!(
            toml_str.contains("network = [\"*\"]"),
            "web_search should derive network capability"
        );
        assert!(unmapped.is_empty());
    }

    #[test]
    fn test_capability_derivation() {
        let tools = vec!["shell_exec".into(), "web_fetch".into(), "agent_send".into()];
        let caps = derive_capabilities(&tools);
        assert_eq!(caps.shell, vec!["*".to_string()]);
        assert_eq!(caps.network, vec!["*".to_string()]);
        assert_eq!(caps.agent_message, vec!["*".to_string()]);
        assert!(caps.agent_spawn);
    }

    #[test]
    fn test_unmapped_tools_reported() {
        let dir = TempDir::new().unwrap();
        let yaml_path = dir.path().join("agent.yaml");
        std::fs::write(
            &yaml_path,
            "name: test\ntools:\n  - read_file\n  - some_custom_tool\n  - another_unknown\n",
        )
        .unwrap();

        let (toml_str, unmapped) = convert_legacy_agent(&yaml_path, "test").unwrap();
        assert!(toml_str.contains("file_read"));
        assert!(!toml_str.contains("some_custom_tool"));
        assert_eq!(unmapped.len(), 2);
        assert!(unmapped.contains(&"some_custom_tool".to_string()));
        assert!(unmapped.contains(&"another_unknown".to_string()));
    }

    #[test]
    fn test_scan_workspace() {
        let source = TempDir::new().unwrap();
        create_legacy_yaml_workspace(source.path());

        let result = scan_openclaw_workspace(source.path());
        assert!(result.has_config);
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].name, "coder");
        assert!(result.agents[0].has_memory);
        assert_eq!(result.channels.len(), 1);
        assert!(result.channels.contains(&"telegram".to_string()));
    }

    #[test]
    fn test_scan_json5_workspace() {
        let source = TempDir::new().unwrap();
        create_json5_workspace(source.path());

        let result = scan_openclaw_workspace(source.path());
        assert!(result.has_config);
        assert_eq!(result.agents.len(), 2);
        assert!(result.agents.iter().any(|a| a.name == "Coder"));
        assert!(result.agents.iter().any(|a| a.name == "researcher"));
        // All 13 channels detected by scanner
        assert_eq!(
            result.channels.len(),
            13,
            "expected 13 channels, got {:?}",
            result.channels
        );
        assert!(result.channels.contains(&"telegram".to_string()));
        assert!(result.channels.contains(&"discord".to_string()));
        assert!(result.channels.contains(&"slack".to_string()));
        assert!(result.channels.contains(&"whatsapp".to_string()));
        assert!(result.channels.contains(&"signal".to_string()));
        assert!(result.channels.contains(&"matrix".to_string()));
        assert!(result.channels.contains(&"irc".to_string()));
        assert!(result.channels.contains(&"mattermost".to_string()));
        assert!(result.channels.contains(&"feishu".to_string()));
        assert!(result.channels.contains(&"teams".to_string()));
        assert!(result.channels.contains(&"imessage".to_string()));
        assert!(result.channels.contains(&"bluebubbles".to_string()));
        assert!(result.has_memory);
    }

    #[test]
    fn test_is_known_librefang_tool() {
        assert!(is_known_librefang_tool("file_read"));
        assert!(is_known_librefang_tool("shell_exec"));
        assert!(is_known_librefang_tool("web_fetch"));
        assert!(!is_known_librefang_tool("Read"));
        assert!(!is_known_librefang_tool("unknown"));
    }

    #[test]
    fn test_secrets_migration() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        let report = migrate(&options).unwrap();

        // secrets.env must exist and contain all extracted tokens
        let secrets_path = target.path().join("secrets.env");
        assert!(secrets_path.exists(), "secrets.env not created");
        let secrets = std::fs::read_to_string(&secrets_path).unwrap();

        // Verify each token is in secrets.env
        assert!(secrets.contains("TELEGRAM_BOT_TOKEN=123:ABC"));
        assert!(secrets.contains("DISCORD_BOT_TOKEN=discord-token-here"));
        assert!(secrets.contains("SLACK_BOT_TOKEN=xoxb-slack"));
        assert!(secrets.contains("SLACK_APP_TOKEN=xapp-slack"));
        assert!(secrets.contains("MATRIX_ACCESS_TOKEN=syt_matrix_token_xyz"));
        assert!(secrets.contains("IRC_PASSWORD=irc-secret-pw"));
        assert!(secrets.contains("MATTERMOST_TOKEN=mm-token-abc"));
        assert!(secrets.contains("FEISHU_APP_SECRET=feishu-secret-xyz"));
        assert!(secrets.contains("TEAMS_APP_PASSWORD=teams-pw-secret"));

        // config.toml must NOT contain any raw secrets
        let config_toml = std::fs::read_to_string(target.path().join("config.toml")).unwrap();
        for secret in &[
            "123:ABC",
            "discord-token-here",
            "xoxb-slack",
            "xapp-slack",
            "syt_matrix_token_xyz",
            "irc-secret-pw",
            "mm-token-abc",
            "feishu-secret-xyz",
            "teams-pw-secret",
        ] {
            assert!(
                !config_toml.contains(secret),
                "Raw secret '{secret}' leaked into config.toml"
            );
        }

        // Secret items in report
        let secret_count = report
            .imported
            .iter()
            .filter(|i| i.kind == ItemKind::Secret)
            .count();
        assert!(
            secret_count >= 9,
            "expected >=9 Secret items, got {secret_count}"
        );
    }

    #[test]
    fn test_policy_migration() {
        let target = TempDir::new().unwrap();
        let json5_content = r#"{
  channels: {
    telegram: {
      botToken: "tok",
      dmPolicy: "allowlist",
      groupPolicy: "open",
      allowFrom: ["alice", "bob"]
    },
    discord: {
      token: "tok2",
      dmPolicy: "disabled"
    }
  }
}"#;
        let root: OpenClawRoot = json5::from_str(json5_content).unwrap();
        let mut report = MigrationReport::default();

        let channels = migrate_channels_from_json(&root, target.path(), false, &mut report);
        assert!(channels.is_some());
        let ch_table = channels.unwrap();
        let table = ch_table.as_table().unwrap();

        // Telegram should have overrides with mapped policies.
        // OpenClaw group_policy "open" → LibreFang "all" (was incorrectly
        // mapped to "respond" before the 2026-04 sync fix).
        let tg = table["telegram"].as_table().unwrap();
        let overrides = tg["overrides"].as_table().unwrap();
        assert_eq!(overrides["dm_policy"].as_str().unwrap(), "allowed_only");
        assert_eq!(overrides["group_policy"].as_str().unwrap(), "all");
        // allowed_users lives at the top level of the channel table, not inside
        // overrides (ChannelOverrides has no allowed_users field).
        assert!(
            !overrides.contains_key("allowed_users"),
            "allowed_users must not be written inside overrides — it's not a \
             ChannelOverrides field"
        );
        let users = tg["allowed_users"].as_array().unwrap();
        assert_eq!(users.len(), 2);

        // Discord should have overrides with mapped dm_policy
        let dc = table["discord"].as_table().unwrap();
        let dc_overrides = dc["overrides"].as_table().unwrap();
        assert_eq!(dc_overrides["dm_policy"].as_str().unwrap(), "ignore");
    }

    #[test]
    fn test_idempotent_migration() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        // Run migration twice
        migrate(&options).unwrap();

        // Marker file must be present after a successful run.
        let marker = target.path().join(MIGRATION_MARKER_FILENAME);
        assert!(marker.exists(), "marker file not written");

        // Second run must be a no-op (no imported entries) so user edits to
        // config.toml / agent.toml between runs are preserved.
        let report2 = migrate(&options).unwrap();
        assert!(
            report2.imported.is_empty(),
            "second run should short-circuit on marker file"
        );

        // secrets.env should not have duplicate keys (re-run is no-op).
        let secrets = std::fs::read_to_string(target.path().join("secrets.env")).unwrap();
        let tg_count = secrets
            .lines()
            .filter(|l| l.starts_with("TELEGRAM_BOT_TOKEN="))
            .count();
        assert_eq!(tg_count, 1, "Duplicate TELEGRAM_BOT_TOKEN in secrets.env");

        let dc_count = secrets
            .lines()
            .filter(|l| l.starts_with("DISCORD_BOT_TOKEN="))
            .count();
        assert_eq!(dc_count, 1, "Duplicate DISCORD_BOT_TOKEN in secrets.env");
    }

    /// Forced re-run (marker deleted) must NEVER clobber existing user-edited
    /// files. The atomic staging-promotion path (this PR) preserves the user
    /// edit as-is — no backup needed because nothing is overwritten.
    /// This is the #3795 semantics; the earlier #4091 "backup + overwrite"
    /// behaviour was reversed by the staging-dir promotion.
    #[test]
    fn test_rerun_preserves_user_edits() {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        create_json5_workspace(source.path());

        let options = MigrateOptions {
            source: crate::MigrateSource::OpenClaw,
            source_dir: source.path().to_path_buf(),
            target_dir: target.path().to_path_buf(),
            dry_run: false,
        };

        // Initial migration writes config.toml and the migration marker.
        migrate(&options).unwrap();

        let config_path = target.path().join("config.toml");
        assert!(config_path.exists());

        // Simulate user editing the config after the first import.
        let user_marker = "# user edited line: do not lose me\n";
        let original = std::fs::read_to_string(&config_path).unwrap();
        std::fs::write(&config_path, format!("{user_marker}{original}")).unwrap();

        // Force a re-run by deleting the marker.
        std::fs::remove_file(target.path().join(MIGRATION_MARKER_FILENAME)).unwrap();

        migrate(&options).unwrap();

        // User edit is preserved verbatim — promote_staging refuses to
        // clobber existing files.
        let new_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            new_content.contains(user_marker),
            "user edit must be preserved (never-clobber semantics, #3795)"
        );

        // No `.bak.*` sibling should be created — never-clobber means we
        // never need to back up because we never overwrite.
        let backups: Vec<_> = std::fs::read_dir(target.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("config.toml.bak.")
            })
            .collect();
        assert!(
            backups.is_empty(),
            "no backup expected (never-clobber semantics), got {} backup(s)",
            backups.len()
        );
    }

    #[test]
    fn test_google_chat_channel_alias() {
        // Verify that "googlechat" (camelCase variant) is parsed correctly
        let target = TempDir::new().unwrap();
        let json5_content = r#"{
  channels: {
    googlechat: {
      webhookPath: "/webhook/gchat"
    }
  }
}"#;
        let root: OpenClawRoot = json5::from_str(json5_content).unwrap();
        let mut report = MigrationReport::default();

        let channels = migrate_channels_from_json(&root, target.path(), false, &mut report);
        assert!(channels.is_some());
        let ch_table = channels.unwrap();
        let table = ch_table.as_table().unwrap();
        assert!(
            table.contains_key("google_chat"),
            "googlechat should map to google_chat"
        );
    }

    #[test]
    fn test_signal_url_construction() {
        let target = TempDir::new().unwrap();
        let json5_content = r#"{
  channels: {
    signal: {
      httpHost: "signal-api.local",
      httpPort: 9090,
      account: "+15551234567"
    }
  }
}"#;
        let root: OpenClawRoot = json5::from_str(json5_content).unwrap();
        let mut report = MigrationReport::default();

        let channels = migrate_channels_from_json(&root, target.path(), false, &mut report);
        assert!(channels.is_some());
        let ch_table = channels.unwrap();
        let table = ch_table.as_table().unwrap();
        let sig = table["signal"].as_table().unwrap();
        assert_eq!(
            sig["api_url"].as_str().unwrap(),
            "http://signal-api.local:9090"
        );
        assert_eq!(sig["phone_number"].as_str().unwrap(), "+15551234567");
    }
}
