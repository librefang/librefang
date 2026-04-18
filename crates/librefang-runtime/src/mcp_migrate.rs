//! One-time migration from the legacy two-store MCP layout into the unified
//! `config.toml`-only layout.
//!
//! Legacy layout (pre-unification):
//!   - `~/.librefang/integrations/*.toml` — catalog templates (read-only)
//!   - `~/.librefang/integrations.toml`   — installed state + credentials
//!
//! Unified layout:
//!   - `~/.librefang/mcp/catalog/*.toml`  — catalog templates (read-only)
//!   - `~/.librefang/config.toml`         — `[[mcp_servers]]` with an
//!     optional `template_id` field
//!
//! Migration steps (run at most once per home dir):
//! 1. If `integrations/` still exists and `mcp/catalog/` does not, rename.
//! 2. If `integrations.toml` exists, read it, synthesize `[[mcp_servers]]`
//!    entries pointing back to their template_id, upsert into `config.toml`,
//!    then back the old file up to `integrations.toml.bak.<unix_ts>`.
//!
//! After a successful migration, `registry_sync` takes over for the catalog
//! cache and the installer writes directly to `config.toml`.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Represents an entry in the legacy `integrations.toml`.
#[derive(Debug, Clone, Deserialize)]
struct LegacyInstalledIntegration {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    installed_at: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    #[allow(dead_code)]
    oauth_provider: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    config: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyIntegrationsFile {
    #[serde(default)]
    installed: Vec<LegacyInstalledIntegration>,
}

/// Transport copy of the legacy template format so we don't need to depend
/// on the extensions crate (which would create a circular dependency with
/// the kernel).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LegacyTransport {
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

#[derive(Debug, Clone, Deserialize)]
struct LegacyRequiredEnv {
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyOAuth {
    #[serde(default)]
    auth_url: Option<String>,
    #[serde(default)]
    token_url: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyTemplate {
    #[serde(default)]
    id: Option<String>,
    transport: LegacyTransport,
    #[serde(default)]
    required_env: Vec<LegacyRequiredEnv>,
    #[serde(default)]
    oauth: Option<LegacyOAuth>,
}

/// Run the migration if a legacy layout is detected.
///
/// Returns `Ok(Some(summary))` when any work was performed, `Ok(None)` when
/// nothing needed migrating, and `Err(_)` only for unexpected I/O errors
/// during the migration itself (missing source files are not errors).
pub fn migrate_if_needed(home_dir: &Path) -> Result<Option<String>, String> {
    let legacy_dir = home_dir.join("integrations");
    let new_dir = home_dir.join("mcp").join("catalog");
    let legacy_file = home_dir.join("integrations.toml");

    let dir_needs_migrate = legacy_dir.is_dir() && !new_dir.exists();
    let file_needs_migrate = legacy_file.is_file();

    if !dir_needs_migrate && !file_needs_migrate {
        return Ok(None);
    }

    let mut parts: Vec<String> = Vec::new();

    // Step 1: rename directory if catalog dir doesn't exist yet.
    if dir_needs_migrate {
        if let Some(parent) = new_dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Could not create {}: {e}", parent.display());
            }
        }
        match std::fs::rename(&legacy_dir, &new_dir) {
            Ok(()) => {
                parts.push(format!(
                    "renamed {} → {}",
                    legacy_dir.display(),
                    new_dir.display()
                ));
            }
            Err(e) => {
                // Fall back to copy-then-remove to handle cross-device or
                // permissions issues.
                warn!(
                    "Could not rename {}: {e}; falling back to copy",
                    legacy_dir.display()
                );
                if let Err(e2) = copy_dir_recursive(&legacy_dir, &new_dir) {
                    return Err(format!(
                        "migration: failed to copy {}: {e2}",
                        legacy_dir.display()
                    ));
                }
                let _ = std::fs::remove_dir_all(&legacy_dir);
                parts.push(format!(
                    "copied {} → {}",
                    legacy_dir.display(),
                    new_dir.display()
                ));
            }
        }
    }

    // Step 2: synthesize `[[mcp_servers]]` entries from integrations.toml.
    if file_needs_migrate {
        let raw = match std::fs::read_to_string(&legacy_file) {
            Ok(s) => s,
            Err(e) => {
                return Err(format!(
                    "migration: failed to read {}: {e}",
                    legacy_file.display()
                ));
            }
        };
        let parsed: LegacyIntegrationsFile = match toml::from_str(&raw) {
            Ok(p) => p,
            Err(e) => {
                return Err(format!(
                    "migration: failed to parse {}: {e}",
                    legacy_file.display()
                ));
            }
        };

        let config_path = home_dir.join("config.toml");
        let mut synth_count = 0usize;
        let mut skipped_count = 0usize;
        for inst in &parsed.installed {
            // Look up the template that this install record pointed at. It
            // lives either at the new catalog dir (if step 1 just renamed)
            // or in the legacy directory (if no dir migration was needed).
            let template_path = {
                let p_new = new_dir.join(format!("{}.toml", inst.id));
                if p_new.is_file() {
                    p_new
                } else {
                    legacy_dir.join(format!("{}.toml", inst.id))
                }
            };
            let template = match std::fs::read_to_string(&template_path) {
                Ok(s) => match toml::from_str::<LegacyTemplate>(&s) {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(
                            "migration: could not parse template {}: {e}; skipping install record",
                            template_path.display()
                        );
                        skipped_count += 1;
                        continue;
                    }
                },
                Err(_) => {
                    warn!(
                        "migration: template file for '{}' not found at {}; skipping install record",
                        inst.id,
                        template_path.display()
                    );
                    skipped_count += 1;
                    continue;
                }
            };
            if let Err(e) = upsert_mcp_server_from_template(&config_path, inst, &template) {
                warn!("migration: could not upsert '{}': {e}", inst.id);
                skipped_count += 1;
                continue;
            }
            synth_count += 1;
        }

        // Back up integrations.toml so we don't re-run this branch on every boot.
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = home_dir.join(format!("integrations.toml.bak.{ts}"));
        match std::fs::rename(&legacy_file, &backup) {
            Ok(()) => {
                parts.push(format!(
                    "synthesized {synth_count} mcp_servers entries (skipped {skipped_count}); \
                     backed up {} → {}",
                    legacy_file.display(),
                    backup.display()
                ));
            }
            Err(e) => {
                // Don't leave the file in place — at worst, try to copy it.
                warn!(
                    "migration: could not rename {} → {}: {e}; leaving old file in place",
                    legacy_file.display(),
                    backup.display()
                );
                parts.push(format!(
                    "synthesized {synth_count} mcp_servers entries (skipped {skipped_count})"
                ));
            }
        }
    }

    if parts.is_empty() {
        Ok(None)
    } else {
        let summary = parts.join("; ");
        info!("MCP migration completed: {summary}");
        Ok(Some(summary))
    }
}

fn upsert_mcp_server_from_template(
    config_path: &Path,
    install: &LegacyInstalledIntegration,
    template: &LegacyTemplate,
) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting. Writing back
        // a near-empty table would drop every unrelated section the user had
        // and turn a recoverable malformed-file into destructive data loss.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        toml::value::Table::new()
    };

    // Build a TOML table for the synthesized [[mcp_servers]] entry so that
    // we don't need to import librefang-types here (runtime → types is fine,
    // but a plain inline TOML keeps us independent of the exact derive
    // configuration on `McpServerConfigEntry`).
    let mut entry = toml::value::Table::new();
    entry.insert("name".to_string(), toml::Value::String(install.id.clone()));
    entry.insert(
        "template_id".to_string(),
        toml::Value::String(template.id.clone().unwrap_or_else(|| install.id.clone())),
    );
    entry.insert("timeout_secs".to_string(), toml::Value::Integer(30));

    let env_list: Vec<toml::Value> = template
        .required_env
        .iter()
        .map(|e| toml::Value::String(e.name.clone()))
        .collect();
    entry.insert("env".to_string(), toml::Value::Array(env_list));
    entry.insert("headers".to_string(), toml::Value::Array(Vec::new()));
    entry.insert("taint_scanning".to_string(), toml::Value::Boolean(true));

    let transport = match &template.transport {
        LegacyTransport::Stdio { command, args } => {
            let mut t = toml::value::Table::new();
            t.insert("type".to_string(), toml::Value::String("stdio".to_string()));
            t.insert("command".to_string(), toml::Value::String(command.clone()));
            let arr: Vec<toml::Value> = args
                .iter()
                .map(|a| toml::Value::String(a.clone()))
                .collect();
            t.insert("args".to_string(), toml::Value::Array(arr));
            toml::Value::Table(t)
        }
        LegacyTransport::Sse { url } => {
            let mut t = toml::value::Table::new();
            t.insert("type".to_string(), toml::Value::String("sse".to_string()));
            t.insert("url".to_string(), toml::Value::String(url.clone()));
            toml::Value::Table(t)
        }
        LegacyTransport::Http { url } => {
            let mut t = toml::value::Table::new();
            t.insert("type".to_string(), toml::Value::String("http".to_string()));
            t.insert("url".to_string(), toml::Value::String(url.clone()));
            toml::Value::Table(t)
        }
    };
    entry.insert("transport".to_string(), transport);

    if let Some(o) = &template.oauth {
        let mut oauth = toml::value::Table::new();
        if let Some(auth_url) = &o.auth_url {
            oauth.insert(
                "auth_url".to_string(),
                toml::Value::String(auth_url.clone()),
            );
        }
        if let Some(token_url) = &o.token_url {
            oauth.insert(
                "token_url".to_string(),
                toml::Value::String(token_url.clone()),
            );
        }
        let scopes: Vec<toml::Value> = o
            .scopes
            .iter()
            .map(|s| toml::Value::String(s.clone()))
            .collect();
        oauth.insert("scopes".to_string(), toml::Value::Array(scopes));
        entry.insert("oauth".to_string(), toml::Value::Table(oauth));
    }

    // Respect the `enabled = false` legacy flag by simply not migrating
    // disabled entries — a disabled install in the old layout had no effect
    // on connections anyway.
    if !install.enabled {
        debug!(
            "migration: legacy integration '{}' was disabled; skipping",
            install.id
        );
        return Ok(());
    }

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    if let toml::Value::Array(ref mut arr) = servers {
        // Don't duplicate an entry the user already has in config.toml.
        let already = arr.iter().any(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                == Some(install.id.as_str())
        });
        if already {
            debug!(
                "migration: '{}' already present in config.toml; leaving manual entry intact",
                install.id
            );
            return Ok(());
        }
        arr.push(toml::Value::Table(entry));
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path: PathBuf = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn no_legacy_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let result = migrate_if_needed(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn rename_integrations_dir_to_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("integrations");
        std::fs::create_dir_all(&old).unwrap();
        write(
            &old.join("github.toml"),
            "id = \"github\"\n[transport]\ntype = \"stdio\"\ncommand = \"echo\"\n",
        );
        let result = migrate_if_needed(tmp.path()).unwrap();
        assert!(result.is_some(), "expected migration to report work");
        assert!(!old.exists(), "legacy integrations dir should be gone");
        assert!(tmp
            .path()
            .join("mcp")
            .join("catalog")
            .join("github.toml")
            .exists());
    }

    #[test]
    fn install_records_become_mcp_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let old_dir = tmp.path().join("integrations");
        std::fs::create_dir_all(&old_dir).unwrap();
        write(
            &old_dir.join("github.toml"),
            r#"
id = "github"
name = "GitHub"
description = "GH"
category = "devtools"

[transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[[required_env]]
name = "GITHUB_PERSONAL_ACCESS_TOKEN"
label = "PAT"
help = "pat"
is_secret = true
"#,
        );
        write(
            &tmp.path().join("integrations.toml"),
            r#"
[[installed]]
id = "github"
installed_at = "2026-02-23T10:00:00Z"
enabled = true
"#,
        );

        let result = migrate_if_needed(tmp.path()).unwrap();
        assert!(result.is_some(), "expected migration to report work");
        // Old file backed up, new catalog present, config.toml has entry.
        assert!(
            !tmp.path().join("integrations.toml").exists(),
            "integrations.toml should be renamed away"
        );
        let backup_exists = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("integrations.toml.bak.")
            });
        assert!(backup_exists, "expected integrations.toml.bak.<ts>");

        let config = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            config.contains("name = \"github\""),
            "config.toml missing mcp_servers entry for github:\n{config}"
        );
        assert!(
            config.contains("template_id = \"github\""),
            "config.toml missing template_id for github:\n{config}"
        );
    }

    #[test]
    fn disabled_entries_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let old_dir = tmp.path().join("integrations");
        std::fs::create_dir_all(&old_dir).unwrap();
        write(
            &old_dir.join("slack.toml"),
            r#"
id = "slack"
name = "Slack"
description = "Slack"
category = "communication"

[transport]
type = "stdio"
command = "slack-mcp"
"#,
        );
        write(
            &tmp.path().join("integrations.toml"),
            r#"
[[installed]]
id = "slack"
installed_at = "2026-02-23T10:00:00Z"
enabled = false
"#,
        );
        migrate_if_needed(tmp.path()).unwrap();
        let config_path = tmp.path().join("config.toml");
        let config = std::fs::read_to_string(&config_path).unwrap_or_default();
        assert!(
            !config.contains("name = \"slack\""),
            "disabled entry should not be written to config.toml"
        );
    }

    #[test]
    fn does_not_clobber_existing_manual_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let old_dir = tmp.path().join("integrations");
        std::fs::create_dir_all(&old_dir).unwrap();
        write(
            &old_dir.join("github.toml"),
            r#"
id = "github"
name = "GitHub"
description = "GH"
category = "devtools"

[transport]
type = "stdio"
command = "npx"
args = []
"#,
        );
        write(
            &tmp.path().join("integrations.toml"),
            r#"
[[installed]]
id = "github"
installed_at = "2026-02-23T10:00:00Z"
enabled = true
"#,
        );
        write(
            &tmp.path().join("config.toml"),
            r#"
[[mcp_servers]]
name = "github"
timeout_secs = 120

[mcp_servers.transport]
type = "http"
url = "https://manual.example.com/mcp"
"#,
        );

        migrate_if_needed(tmp.path()).unwrap();

        let config = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        // Manual entry must survive — we only detect by name.
        assert!(
            config.contains("https://manual.example.com/mcp"),
            "manual config.toml entry must not be clobbered:\n{config}"
        );
    }
}
