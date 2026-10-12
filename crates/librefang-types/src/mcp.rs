//! MCP catalog and runtime-status schema.
//!
//! Lives here because the `librefang-types` crate is the schema spine — every
//! cross-crate type belongs here, with implementation living in the consuming
//! crate. The behaviours that *use* these types (catalog loading, installer
//! transforms, health monitoring) stay in `librefang-extensions`.
//!
//! Distinct from [`crate::config::McpServerConfigEntry`]: catalog entries are
//! read-only *templates* shipped with the registry (`~/.librefang/mcp/catalog/
//! *.toml`) that get transformed into a config-side `McpServerConfigEntry`
//! when the user installs an MCP server.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
/// Parallels [`crate::config::McpTransportEntry`] but without the
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

/// An HTTP header an MCP catalog entry sends to its SSE / Streamable-HTTP server.
///
/// `value` is a template: every `${NAME}` placeholder must name one of the entry's `required_env` credentials and is replaced with that credential when the connection is opened, never earlier.
/// The installer copies the template — not a resolved secret — into the installed server's `headers`, so the persisted config carries a reference and the secret itself stays in the vault / environment.
/// A value without placeholders is a fixed header and is sent verbatim.
///
/// ```toml
/// [[headers]]
/// name = "Authorization"
/// value = "Bearer ${EXAMPLE_API_KEY}"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCatalogHeader {
    /// Header name (e.g., "Authorization").
    pub name: String,
    /// Header value template (e.g., "Bearer ${EXAMPLE_API_KEY}").
    pub value: String,
}

/// Render a header value template, replacing each `${NAME}` placeholder with `lookup(NAME)`.
///
/// Only the braced form is a placeholder: a bare `$` is literal text, so fixed header values that happen to contain `$` keep their meaning.
/// A placeholder name is one or more ASCII letters, digits or underscores and does not start with a digit.
/// An unterminated `${`, an empty name or an invalid name is an error, as is any error `lookup` returns.
/// Error messages name the placeholder but never include a resolved value.
pub fn render_header_template(
    template: &str,
    mut lookup: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err("unterminated '${' placeholder in header value".to_string());
        };
        let name = &after[..end];
        if !is_valid_placeholder_name(name) {
            return Err(format!("invalid placeholder '${{{name}}}' in header value"));
        }
        out.push_str(&lookup(name)?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The `${NAME}` placeholder names a header value template references, in order of appearance.
pub fn header_template_placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    render_header_template(template, |name| {
        names.push(name.to_string());
        Ok(String::new())
    })?;
    Ok(names)
}

fn is_valid_placeholder_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name` is a valid HTTP header field name (an RFC 9110 `token`).
fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Health check tuning for an MCP catalog entry.
///
/// Renamed from `HealthCheckConfig` during the move into `librefang-types`
/// to disambiguate from [`crate::config::HealthCheckConfig`] (the global
/// LLM-provider health-check tuning), which already lives in this crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpHealthCheckConfig {
    /// How often to check health (seconds).
    pub interval_secs: u64,
    /// Consider unhealthy after this many consecutive failures.
    pub unhealthy_threshold: u32,
}

impl Default for McpHealthCheckConfig {
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
    /// HTTP headers for an SSE / Streamable-HTTP transport, typically mapping a `required_env` credential to an `Authorization` header.
    /// See [`McpCatalogHeader`]; checked by [`McpCatalogEntry::validate`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<McpCatalogHeader>,
    /// OAuth configuration (None = API key only).
    #[serde(default)]
    pub oauth: Option<crate::oauth::OAuthTemplate>,
    /// Searchable tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Setup instructions (displayed in TUI detail view).
    #[serde(default)]
    pub setup_instructions: String,
    /// Health check configuration.
    #[serde(default)]
    pub health_check: McpHealthCheckConfig,
    /// Per-language translation overrides for `name`, `description`, and
    /// `setup_instructions`. Keyed by BCP-47 tag (`zh`, `zh-TW`, …).
    /// API routes resolve `Accept-Language` against this table and fall
    /// back to the top-level English fields when no entry matches.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub i18n: HashMap<String, McpCatalogI18n>,
}

impl McpCatalogEntry {
    /// Check the entry's header bindings, so a broken template is rejected when the catalog loads instead of when a server built from it first connects.
    ///
    /// Headers are only meaningful on the `sse` and `http` transports, each header name must be a valid, unique (case-insensitive) HTTP field name, a value must not contain CR, LF or NUL, and every `${NAME}` placeholder must name one of this entry's `required_env` credentials.
    /// The last rule is what keeps a template from reading an arbitrary daemon secret: the runtime only substitutes variables the installed server declares in its `env` list, which the installer fills from `required_env`.
    pub fn validate(&self) -> Result<(), String> {
        if self.headers.is_empty() {
            return Ok(());
        }
        if matches!(self.transport, McpCatalogTransport::Stdio { .. }) {
            return Err("headers are only supported on the sse and http transports".to_string());
        }
        let mut seen = std::collections::HashSet::new();
        for header in &self.headers {
            if !is_valid_header_name(&header.name) {
                return Err(format!("invalid header name '{}'", header.name));
            }
            if !seen.insert(header.name.to_ascii_lowercase()) {
                return Err(format!("duplicate header '{}'", header.name));
            }
            if header
                .value
                .chars()
                .any(|c| c == '\r' || c == '\n' || c == '\0')
            {
                return Err(format!(
                    "header '{}' value contains a control character",
                    header.name
                ));
            }
            let placeholders = header_template_placeholders(&header.value)
                .map_err(|e| format!("header '{}': {e}", header.name))?;
            for placeholder in placeholders {
                if !self.required_env.iter().any(|env| env.name == placeholder) {
                    return Err(format!(
                        "header '{}' references '${{{placeholder}}}', which is not one of this entry's required_env names",
                        header.name
                    ));
                }
            }
        }
        Ok(())
    }
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

    fn http_entry_with_headers(headers: &str) -> McpCatalogEntry {
        let toml_str = format!(
            r#"
id = "remote"
name = "Remote"
description = "Remote MCP over Streamable HTTP"
category = "ai"

[transport]
type = "http"
url = "https://mcp.example.com/mcp"

[[required_env]]
name = "REMOTE_API_KEY"
label = "API Key"
help = "Create one in the account settings"

{headers}
"#
        );
        toml::from_str(&toml_str).expect("fixture parses")
    }

    /// #8578: a catalog entry can bind a credential to a header, and the binding survives a JSON round-trip (the shape the catalog API serves).
    #[test]
    fn catalog_entry_header_binding_parses_and_validates() {
        let entry = http_entry_with_headers(
            r#"
[[headers]]
name = "Authorization"
value = "Bearer ${REMOTE_API_KEY}"

[[headers]]
name = "X-Client"
value = "librefang"
"#,
        );
        assert_eq!(entry.headers.len(), 2);
        assert_eq!(entry.headers[0].value, "Bearer ${REMOTE_API_KEY}");
        entry.validate().expect("valid binding");

        let json = serde_json::to_value(&entry).unwrap();
        let back: McpCatalogEntry = serde_json::from_value(json).unwrap();
        assert_eq!(back.headers, entry.headers);
    }

    /// Entries without `[[headers]]` (every entry that predates #8578) still parse, validate, and serialize without a `headers` key.
    #[test]
    fn catalog_entry_without_headers_is_unchanged() {
        let entry = http_entry_with_headers("");
        assert!(entry.headers.is_empty());
        entry.validate().expect("no headers is valid");
        let json = serde_json::to_value(&entry).unwrap();
        assert!(json.get("headers").is_none());
    }

    #[test]
    fn catalog_entry_rejects_placeholder_outside_required_env() {
        let entry = http_entry_with_headers(
            r#"
[[headers]]
name = "Authorization"
value = "Bearer ${ANTHROPIC_API_KEY}"
"#,
        );
        let err = entry.validate().unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY"), "{err}");
    }

    #[test]
    fn catalog_entry_rejects_malformed_header_bindings() {
        for (headers, needle) in [
            (
                "[[headers]]\nname = \"Authorization\"\nvalue = \"Bearer ${REMOTE_API_KEY\"",
                "unterminated",
            ),
            (
                "[[headers]]\nname = \"Authorization\"\nvalue = \"Bearer ${}\"",
                "invalid placeholder",
            ),
            (
                "[[headers]]\nname = \"Bad Name\"\nvalue = \"x\"",
                "invalid header name",
            ),
            (
                "[[headers]]\nname = \"X-Key\"\nvalue = \"a\\nb\"",
                "control character",
            ),
            (
                "[[headers]]\nname = \"X-Key\"\nvalue = \"a\"\n[[headers]]\nname = \"x-key\"\nvalue = \"b\"",
                "duplicate header",
            ),
        ] {
            let entry = http_entry_with_headers(headers);
            let err = entry.validate().unwrap_err();
            assert!(err.contains(needle), "expected '{needle}' in '{err}'");
        }
    }

    #[test]
    fn catalog_entry_rejects_headers_on_stdio_transport() {
        let toml_str = r#"
id = "local"
name = "Local"
description = "stdio"
category = "devtools"

[transport]
type = "stdio"
command = "local-mcp"

[[required_env]]
name = "LOCAL_KEY"
label = "Key"
help = "h"

[[headers]]
name = "Authorization"
value = "Bearer ${LOCAL_KEY}"
"#;
        let entry: McpCatalogEntry = toml::from_str(toml_str).unwrap();
        let err = entry.validate().unwrap_err();
        assert!(err.contains("sse and http"), "{err}");
    }

    #[test]
    fn render_header_template_substitutes_only_braced_placeholders() {
        let rendered =
            render_header_template("Bearer ${A}-$B-${C_2}", |name| Ok(format!("<{name}>")))
                .unwrap();
        assert_eq!(rendered, "Bearer <A>-$B-<C_2>");
        assert_eq!(
            header_template_placeholders("x ${ONE} y ${TWO}").unwrap(),
            vec!["ONE".to_string(), "TWO".to_string()]
        );
        assert!(header_template_placeholders("${1ABC}").is_err());
        let err =
            render_header_template("${MISSING}", |name| Err(format!("{name} unset"))).unwrap_err();
        assert_eq!(err, "MISSING unset");
    }
}
