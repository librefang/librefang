//! LibreFang Extensions — MCP server catalog, credential vault, OAuth, health.
//!
//! This crate provides:
//! - **MCP Catalog**: read-only set of MCP server templates (GitHub, Slack, ...)
//!   cached at `~/.librefang/mcp/catalog/*.toml` and refreshed by `registry_sync`.
//! - **Credential Vault**: AES-256-GCM encrypted storage with OS keyring support
//! - **OAuth2 PKCE**: Localhost callback flows for Google/GitHub/Microsoft/Slack
//! - **Health Monitor**: Auto-reconnect with exponential backoff
//! - **Installer**: Pure transforms from a catalog entry to a new
//!   `McpServerConfigEntry` that the kernel can wire up.
//!
//! Installed MCP servers no longer live in a separate `integrations.toml`;
//! every configured server is an `[[mcp_servers]]` entry in
//! `~/.librefang/config.toml`. An optional `template_id` field records the
//! catalog entry it was installed from.

pub mod catalog;
pub mod credentials;
pub mod dotenv;
pub mod health;
pub mod http_client;
pub mod installer;
pub mod oauth;
pub mod vault;

// Backwards-compatible module alias so downstream crates can still reach the
// catalog via the old `registry` path during the rename. Prefer `catalog`.

use serde::{Deserialize, Serialize};

// ─── Error types ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("MCP catalog entry not found: {0}")]
    NotFound(String),
    #[error("MCP server already configured: {0}")]
    AlreadyInstalled(String),
    #[error("MCP server not configured: {0}")]
    NotInstalled(String),
    #[error("Credential not found: {0}")]
    CredentialNotFound(String),
    #[error("Vault error: {0}")]
    Vault(String),
    #[error("Vault locked — unlock with vault key or LIBREFANG_VAULT_KEY env var")]
    VaultLocked,
    /// The vault was opened with a key that does not match the key it was
    /// encrypted with. Surfaced from #3651: pre-fix the daemon would silently
    /// boot, then every subsequent vault read would error with a generic
    /// "Decryption failed" log line — the operator never learned the root
    /// cause was a mismatched `LIBREFANG_VAULT_KEY`.
    ///
    /// `hint` carries the recovery instruction for the operator (typically
    /// "restore the original env var, or rebuild from backup"). The
    /// boot-path translates this into a `KernelError::BootFailed` so the
    /// daemon refuses to start instead of corrupting downstream state.
    #[error("Vault key mismatch: {hint}")]
    VaultKeyMismatch { hint: String },
    #[error("OAuth error: {0}")]
    OAuth(String),
    #[error("TOML parse error: {0}")]
    TomlParse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Health check failed: {0}")]
    HealthCheck(String),
}

pub type ExtensionResult<T> = Result<T, ExtensionError>;

// ─── Core types ──────────────────────────────────────────────────────────────

/// Category of an MCP catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpCategory {
    DevTools,
    Productivity,
    Communication,
    Data,
    Cloud,
    AI,
}

impl std::fmt::Display for McpCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DevTools => write!(f, "Dev Tools"),
            Self::Productivity => write!(f, "Productivity"),
            Self::Communication => write!(f, "Communication"),
            Self::Data => write!(f, "Data"),
            Self::Cloud => write!(f, "Cloud"),
            Self::AI => write!(f, "AI & Search"),
        }
    }
}

/// MCP transport template — how to launch the server.
///
/// Parallels [`librefang_types::config::McpTransportEntry`] but without the
/// `HttpCompat` variant, which is a user-authored power-user transport and
/// doesn't ship as a catalog template. The catalog entry's transport is
/// converted into a `McpTransportEntry` when the user installs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpCatalogTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Sse {
        url: String,
    },
    Http {
        url: String,
    },
}

/// An environment variable required by an MCP catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCatalogRequiredEnv {
    /// Env var name (e.g., "GITHUB_PERSONAL_ACCESS_TOKEN").
    pub name: String,
    /// Human-readable label (e.g., "Personal Access Token").
    pub label: String,
    /// How to obtain this credential.
    pub help: String,
    /// Whether this is a secret (should be stored in vault).
    #[serde(default = "default_true")]
    pub is_secret: bool,
    /// URL where the user can create the key.
    #[serde(default)]
    pub get_url: Option<String>,
}

fn default_true() -> bool {
    true
}

/// OAuth provider configuration template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTemplate {
    /// OAuth provider (google, github, microsoft, slack).
    pub provider: String,
    /// OAuth scopes required.
    pub scopes: Vec<String>,
    /// Authorization URL.
    pub auth_url: String,
    /// Token exchange URL.
    pub token_url: String,
}

/// Health check configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthCheckConfig {
    /// How often to check health (seconds).
    pub interval_secs: u64,
    /// Consider unhealthy after this many consecutive failures.
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            unhealthy_threshold: 3,
        }
    }
}

/// A bundled MCP catalog entry — describes how to configure an MCP server.
///
/// Catalog entries live under `~/.librefang/mcp/catalog/*.toml` and are
/// refreshed from the upstream registry by `librefang-runtime::registry_sync`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCatalogEntry {
    /// Unique identifier (e.g., "github").
    pub id: String,
    /// Human-readable name (e.g., "GitHub").
    pub name: String,
    /// Short description.
    pub description: String,
    /// Category for browsing.
    pub category: McpCategory,
    /// Icon (emoji).
    #[serde(default)]
    pub icon: String,
    /// MCP transport configuration.
    pub transport: McpCatalogTransport,
    /// Required credentials.
    #[serde(default)]
    pub required_env: Vec<McpCatalogRequiredEnv>,
    /// OAuth configuration (None = API key only).
    #[serde(default)]
    pub oauth: Option<OAuthTemplate>,
    /// Searchable tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Setup instructions (displayed in TUI detail view).
    #[serde(default)]
    pub setup_instructions: String,
    /// Health check configuration.
    #[serde(default)]
    pub health_check: HealthCheckConfig,
    /// Per-language translation overrides for `name`, `description`, and
    /// `setup_instructions`. Keyed by BCP-47 tag (`zh`, `zh-TW`, …).
    /// API routes resolve `Accept-Language` against this table and fall
    /// back to the top-level English fields when no entry matches.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub i18n: std::collections::HashMap<String, McpCatalogI18n>,
}

/// Per-language overrides for an MCP catalog entry's user-facing strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpCatalogI18n {
    /// Localized name. Falls back to the top-level `name`.
    #[serde(default)]
    pub name: Option<String>,
    /// Localized description. Falls back to the top-level `description`.
    #[serde(default)]
    pub description: Option<String>,
    /// Localized setup instructions. Falls back to the top-level
    /// `setup_instructions`.
    #[serde(default)]
    pub setup_instructions: Option<String>,
}

/// Status of an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpStatus {
    /// Configured and MCP server running.
    Ready,
    /// Configured but credentials missing.
    Setup,
    /// Not yet configured (catalog entry only).
    Available,
    /// MCP server errored.
    Error(String),
    /// Disabled by user.
    Disabled,
}

impl std::fmt::Display for McpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "Ready"),
            Self::Setup => write!(f, "Setup"),
            Self::Available => write!(f, "Available"),
            Self::Error(msg) => write!(f, "Error: {msg}"),
            Self::Disabled => write!(f, "Disabled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_display() {
        assert_eq!(McpCategory::DevTools.to_string(), "Dev Tools");
        assert_eq!(McpCategory::Productivity.to_string(), "Productivity");
        assert_eq!(McpCategory::AI.to_string(), "AI & Search");
    }

    #[test]
    fn status_display() {
        assert_eq!(McpStatus::Ready.to_string(), "Ready");
        assert_eq!(McpStatus::Setup.to_string(), "Setup");
        assert_eq!(
            McpStatus::Error("timeout".to_string()).to_string(),
            "Error: timeout"
        );
    }

    #[test]
    fn catalog_entry_roundtrip() {
        let toml_str = r#"
id = "test"
name = "Test Integration"
description = "A test"
category = "devtools"
icon = "T"
tags = ["test"]
setup_instructions = "Just test it."

[transport]
type = "stdio"
command = "test-server"
args = ["--flag"]

[[required_env]]
name = "TEST_KEY"
label = "Test Key"
help = "Get it from test.com"
is_secret = true
get_url = "https://test.com/keys"

[health_check]
interval_secs = 30
unhealthy_threshold = 5
"#;
        let entry: McpCatalogEntry = toml::from_str(toml_str).unwrap();
        assert_eq!(entry.id, "test");
        assert_eq!(entry.category, McpCategory::DevTools);
        assert_eq!(entry.required_env.len(), 1);
        assert!(entry.required_env[0].is_secret);
        assert_eq!(entry.health_check.interval_secs, 30);
    }

    #[test]
    fn error_display() {
        let err = ExtensionError::NotFound("github".to_string());
        assert!(err.to_string().contains("github"));
        let err = ExtensionError::VaultLocked;
        assert!(err.to_string().contains("vault"));
    }

    /// Catalog entries with `[i18n.<lang>]` blocks deserialize all three
    /// localizable fields and survive a JSON round-trip. Catches future
    /// regressions where someone reorders / renames a field on
    /// `McpCatalogI18n` without updating the parser side.
    #[test]
    fn catalog_entry_i18n_roundtrip() {
        let toml_str = r#"
id = "aws"
name = "AWS"
description = "Manage Amazon Web Services resources via MCP."
category = "cloud"
icon = "lucide:cloud"
tags = ["cloud", "aws"]
setup_instructions = "Set AWS_* env vars."

[transport]
type = "stdio"
command = "npx"
args = ["-y", "@aws-mcp/server-aws"]

[i18n.zh]
name = "AWS"
description = "通过 MCP 管理亚马逊云资源。"
setup_instructions = "请配置 AWS_* 环境变量。"

[i18n.zh-TW]
name = "AWS"
description = "透過 MCP 管理亞馬遜雲端資源。"

[i18n.de]
description = "Verwaltet AWS-Ressourcen über MCP."
"#;
        let entry: McpCatalogEntry = toml::from_str(toml_str).unwrap();

        // All three locales are present.
        assert_eq!(entry.i18n.len(), 3);

        // zh: name + description + setup_instructions all set.
        let zh = &entry.i18n["zh"];
        assert_eq!(zh.name.as_deref(), Some("AWS"));
        assert_eq!(
            zh.description.as_deref(),
            Some("通过 MCP 管理亚马逊云资源。")
        );
        assert_eq!(
            zh.setup_instructions.as_deref(),
            Some("请配置 AWS_* 环境变量。")
        );

        // zh-TW: name + description but no setup_instructions → field stays
        // None so render_catalog_entry will fall back to the English value.
        let zh_tw = &entry.i18n["zh-TW"];
        assert_eq!(zh_tw.name.as_deref(), Some("AWS"));
        assert!(zh_tw.setup_instructions.is_none());

        // de: only description; name + setup_instructions remain None and
        // the resolver will fall through to English for those.
        let de = &entry.i18n["de"];
        assert!(de.name.is_none());
        assert!(de.setup_instructions.is_none());
        assert_eq!(
            de.description.as_deref(),
            Some("Verwaltet AWS-Ressourcen über MCP.")
        );
    }

    /// `[i18n.*]`-free entries still deserialize cleanly — the field is
    /// `#[serde(default)]` so existing single-language catalogs keep working.
    #[test]
    fn catalog_entry_without_i18n_block() {
        let toml_str = r#"
id = "no-i18n"
name = "No I18n"
description = "single language only"
category = "devtools"

[transport]
type = "http"
url = "https://example.com"
"#;
        let entry: McpCatalogEntry = toml::from_str(toml_str).unwrap();
        assert!(entry.i18n.is_empty());
    }
}
