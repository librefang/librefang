//! Installer — pure transforms from MCP catalog entries to
//! `McpServerConfigEntry` values that can be persisted into `config.toml`.
//!
//! No side effects — callers (API / CLI) decide when to store the returned
//! server config and credentials. The old "integrations.toml" file is gone;
//! all MCP server state lives in `config.toml` under `[[mcp_servers]]`.

use crate::catalog::McpCatalog;
use crate::credentials::CredentialResolver;
use crate::{ExtensionError, ExtensionResult};
use librefang_types::config::{McpOAuthConfig, McpServerConfigEntry, McpTransportEntry};
use librefang_types::mcp::{McpCatalogEntry, McpCatalogTransport, McpStatus};
use librefang_types::oauth::OAuthTemplate;
use std::collections::HashMap;
use tracing::{info, warn};
use zeroize::Zeroizing;

/// Result of an installation attempt.
#[derive(Debug)]
pub struct InstallResult {
    /// MCP server id (matches the new `McpServerConfigEntry.name`).
    pub id: String,
    /// The `[[mcp_servers]]` entry the caller should persist into config.toml.
    pub server: McpServerConfigEntry,
    /// Final status.
    pub status: McpStatus,
    /// Names of required env vars that still have no credential.
    pub missing_credentials: Vec<String>,
    /// Message to display to the user.
    pub message: String,
}

/// Resolve a catalog entry + provided credentials into a new
/// `McpServerConfigEntry`.
///
/// This is a pure transform:
/// 1. Look up the catalog template by id.
/// 2. Optionally store provided creds in the vault.
/// 3. Check which required env vars still have no credential.
/// 4. Map the template transport + required env into a `McpServerConfigEntry`.
///
/// The caller is responsible for writing the returned entry to config.toml
/// and triggering a kernel reload.
pub fn install_integration(
    catalog: &McpCatalog,
    resolver: &mut CredentialResolver,
    id: &str,
    provided_keys: &HashMap<String, String>,
) -> ExtensionResult<InstallResult> {
    // 1. Look up template
    let template = catalog
        .get(id)
        .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?
        .clone();

    // 2. Store provided keys in vault (best effort)
    for (key, value) in provided_keys {
        if let Err(e) = resolver.store_in_vault(key, Zeroizing::new(value.clone())) {
            warn!("Could not store {} in vault: {}", key, e);
            // Fall through — the key is still in the provided_keys map
        }
    }

    // 3. Check all required credentials
    let required_keys: Vec<&str> = template
        .required_env
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    let missing = resolver.missing_credentials(&required_keys);
    let actually_missing: Vec<String> = missing
        .into_iter()
        .filter(|k| !provided_keys.contains_key(k))
        .collect();

    let status = if actually_missing.is_empty() {
        McpStatus::Ready
    } else {
        McpStatus::Setup
    };

    // 4. Build the McpServerConfigEntry
    let server = catalog_entry_to_mcp_server(&template);

    // 5. Build result message
    let message = match &status {
        McpStatus::Ready => {
            format!(
                "{} added. MCP tools will be available as mcp_{}_*.",
                template.name, id
            )
        }
        McpStatus::Setup => {
            let missing_labels: Vec<String> = actually_missing
                .iter()
                .filter_map(|key| {
                    template
                        .required_env
                        .iter()
                        .find(|e| e.name == *key)
                        .map(|e| format!("{} ({})", e.label, e.name))
                })
                .collect();
            format!(
                "{} installed but needs credentials: {}",
                template.name,
                missing_labels.join(", ")
            )
        }
        _ => format!("{} installed.", template.name),
    };

    info!("{}", message);

    Ok(InstallResult {
        id: id.to_string(),
        server,
        status,
        missing_credentials: actually_missing,
        message,
    })
}

/// Convert a catalog entry into a fresh `McpServerConfigEntry`.
///
/// The resulting entry has `template_id` set to the catalog id so the
/// kernel / dashboard can tell which entries came from the catalog.
pub fn catalog_entry_to_mcp_server(entry: &McpCatalogEntry) -> McpServerConfigEntry {
    let transport = match &entry.transport {
        McpCatalogTransport::Stdio { command, args } => McpTransportEntry::Stdio {
            command: command.clone(),
            args: args.clone(),
        },
        McpCatalogTransport::Sse { url } => McpTransportEntry::Sse { url: url.clone() },
        McpCatalogTransport::Http { url } => McpTransportEntry::Http { url: url.clone() },
    };
    let env: Vec<String> = entry.required_env.iter().map(|e| e.name.clone()).collect();
    let oauth = entry.oauth.as_ref().map(oauth_template_to_config);
    McpServerConfigEntry {
        name: entry.id.clone(),
        template_id: Some(entry.id.clone()),
        transport: Some(transport),
        timeout_secs: 30,
        env,
        headers: Vec::new(),
        oauth,
        taint_scanning: true,
        taint_policy: None,
    }
}

fn oauth_template_to_config(t: &OAuthTemplate) -> McpOAuthConfig {
    McpOAuthConfig {
        auth_url: Some(t.auth_url.clone()),
        token_url: Some(t.token_url.clone()),
        client_id: None,
        scopes: t.scopes.clone(),
        user_scopes: Vec::new(),
    }
}

/// Generate scaffold files for a new custom MCP server template.
pub fn scaffold_integration(dir: &std::path::Path) -> ExtensionResult<String> {
    let template = r#"# Custom MCP Server Template
# Place this in ~/.librefang/mcp/catalog/ to make it available as a catalog entry.

id = "my-mcp"
name = "My MCP Server"
description = "A custom MCP server template"
category = "devtools"
icon = "🔧"
tags = ["custom"]

[transport]
type = "stdio"
command = "npx"
args = ["my-mcp-server"]

[[required_env]]
name = "MY_API_KEY"
label = "API Key"
help = "Get your API key from https://example.com/api-keys"
is_secret = true

[health_check]
interval_secs = 60
unhealthy_threshold = 3

setup_instructions = """
1. Install the MCP server: npm install -g my-mcp-server
2. Get your API key from https://example.com/api-keys
3. Run: librefang mcp add my-mcp --key=<your-key>
"""
"#;
    let path = dir.join("mcp.toml");
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, template)?;
    Ok(format!("MCP server template created at {}", path.display()))
}

/// Generate scaffold files for a new skill.
pub fn scaffold_skill(dir: &std::path::Path) -> ExtensionResult<String> {
    let skill_toml = format!(
        r#"name = "my-skill"
description = "A custom skill"
version = "{version}"
runtime = "prompt_only"
"#,
        version = librefang_types::VERSION,
    );
    let skill_md = format!(
        r#"---
name: my-skill
description: A custom skill
version: {version}
runtime: prompt_only
---

# My Skill

You are an expert at [domain]. When the user asks about [topic], provide [behavior].

## Guidelines

- Be concise and accurate
- Cite sources when possible
"#,
        version = librefang_types::VERSION,
    );
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("skill.toml"), skill_toml)?;
    std::fs::write(dir.join("SKILL.md"), skill_md)?;
    Ok(format!("Skill scaffold created at {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::McpCatalog;

    fn ensure_registry() {
        let _ = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
    }

    #[test]
    fn install_github_returns_mcp_server_entry() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut catalog = McpCatalog::new(&home);
        catalog.load(&home);

        let mut resolver = CredentialResolver::new(None, None);
        let result = install_integration(&catalog, &mut resolver, "github", &HashMap::new())
            .expect("install_integration failed");
        assert_eq!(result.id, "github");
        assert_eq!(result.server.name, "github");
        assert_eq!(result.server.template_id.as_deref(), Some("github"));
        // Status depends on whether GITHUB_PERSONAL_ACCESS_TOKEN is in env
        assert!(result.status == McpStatus::Ready || result.status == McpStatus::Setup);
    }

    #[test]
    fn install_unknown_id_errors() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut catalog = McpCatalog::new(&home);
        catalog.load(&home);
        let mut resolver = CredentialResolver::new(None, None);
        let err = install_integration(&catalog, &mut resolver, "does-not-exist", &HashMap::new())
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NotFound(_)));
    }

    #[test]
    fn install_stdio_template_produces_stdio_transport() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut catalog = McpCatalog::new(&home);
        catalog.load(&home);

        let mut resolver = CredentialResolver::new(None, None);
        let result =
            install_integration(&catalog, &mut resolver, "github", &HashMap::new()).unwrap();
        match &result.server.transport {
            Some(McpTransportEntry::Stdio { command, .. }) => {
                assert!(!command.is_empty());
            }
            other => panic!("expected stdio transport, got {other:?}"),
        }
    }

    #[test]
    fn scaffold_integration_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my-mcp");
        let msg = scaffold_integration(&sub).unwrap();
        assert!(sub.join("mcp.toml").exists());
        assert!(msg.contains("mcp.toml"));
    }

    #[test]
    fn scaffold_skill_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my-skill");
        let msg = scaffold_skill(&sub).unwrap();
        assert!(sub.join("skill.toml").exists());
        assert!(sub.join("SKILL.md").exists());
        assert!(msg.contains("my-skill"));
    }
}
