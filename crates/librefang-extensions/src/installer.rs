//! Installer — resolves MCP catalog entries and credentials into
//! `McpServerConfigEntry` values that can be persisted into `config.toml`.
//!
//! Provided credentials are persisted to the configured vault before a
//! successful result is returned. Callers (API / CLI) decide when to persist
//! the returned server config. The old "integrations.toml" file is gone; all
//! MCP server state lives in `config.toml` under `[[mcp_servers]]`.

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

/// Bridge the extensions-crate `InstallResult` to the dependency-free
/// [`librefang_types::integration::IntegrationOutcome`] that the kernel's
/// HTTP-layer `install_integration` façade now returns on its `Ok` side.
/// Keeping the conversion here lets the real kernel impl `.map(Into::into)`
/// cleanly while reimplementers of the trait (mocks, alternate kernels)
/// construct `IntegrationOutcome` directly without ever touching this crate.
///
/// The mapping is field-for-field — all field types already live in
/// `librefang-types` — so the success payload the API layer reads (`server`,
/// `id`, …) is unchanged.
impl From<InstallResult> for librefang_types::integration::IntegrationOutcome {
    fn from(result: InstallResult) -> Self {
        librefang_types::integration::IntegrationOutcome {
            id: result.id,
            server: result.server,
            status: result.status,
            missing_credentials: result.missing_credentials,
            message: result.message,
        }
    }
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

    let required_keys: Vec<&str> = template
        .required_env
        .iter()
        .map(|e| e.name.as_str())
        .collect();

    // 2. Store provided keys in the vault. A success response must not claim
    // the integration is ready unless every supplied credential is durable.
    // Only the template's own `required_env` names are accepted: vault entries are injected into the process env at boot with priority over `.env`, so a stray key (e.g. `OPENAI_API_KEY`) would silently overwrite an unrelated secret.
    // Blank values are skipped rather than stored, so they neither clobber an existing credential nor count as present.
    // An empty or whitespace-only value is not a credential: storing it would mark the requirement satisfied and, for a header binding, send `Authorization: Bearer ` (#8578).
    let mut stored: Vec<&str> = Vec::new();
    for (key, value) in provided_keys {
        if !required_keys.contains(&key.as_str()) {
            warn!(
                integration = id,
                key = %key,
                "Ignoring credential that is not a required_env of this integration"
            );
            continue;
        }
        if value.trim().is_empty() {
            warn!(integration = id, key = %key, "Ignoring empty credential value");
            continue;
        }
        resolver.store_in_vault(key, Zeroizing::new(value.clone()))?;
        stored.push(key.as_str());
    }

    // 3. Check all required credentials
    let missing = resolver.missing_credentials(&required_keys);
    let actually_missing: Vec<String> = missing
        .into_iter()
        .filter(|k| !stored.contains(&k.as_str()))
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
    // Header templates are copied verbatim: `${NAME}` placeholders stay unresolved in the persisted config and are substituted from the declared `env` names only when the runtime opens the connection (#8578).
    let headers: Vec<String> = entry
        .headers
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect();
    let oauth = entry.oauth.as_ref().map(oauth_template_to_config);
    McpServerConfigEntry {
        name: entry.id.clone(),
        template_id: Some(entry.id.clone()),
        transport: Some(transport),
        timeout_secs: 30,
        env,
        headers,
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
        client_secret_env: None,
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
    fn install_fails_when_provided_credential_cannot_be_persisted() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut catalog = McpCatalog::new(&home);
        catalog.load(&home);
        let credential = catalog
            .get("github")
            .and_then(|entry| entry.required_env.first())
            .expect("github fixture should require a credential")
            .name
            .clone();
        let provided = HashMap::from([(credential, "secret".to_string())]);
        let mut resolver = CredentialResolver::new(None, None);

        let error = install_integration(&catalog, &mut resolver, "github", &provided)
            .expect_err("missing vault must fail instead of reporting Ready");

        assert!(matches!(error, ExtensionError::Vault(_)));
    }

    #[test]
    fn install_ignores_unrelated_and_empty_credentials() {
        ensure_registry();
        let home = librefang_runtime::registry_sync::resolve_home_dir_for_tests();
        let mut catalog = McpCatalog::new(&home);
        catalog.load(&home);
        let credential = catalog
            .get("github")
            .and_then(|entry| entry.required_env.first())
            .expect("github fixture should require a credential")
            .name
            .clone();
        let provided = HashMap::from([
            (credential.clone(), "   ".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-test".to_string()),
        ]);
        // No vault: any attempt to store either key would fail with ExtensionError::Vault.
        let mut resolver = CredentialResolver::new(None, None);

        let result = install_integration(&catalog, &mut resolver, "github", &provided)
            .expect("unrelated and blank credentials must be skipped, not stored");

        if std::env::var(&credential).is_err() {
            assert_eq!(result.status, McpStatus::Setup);
            assert!(result.missing_credentials.contains(&credential));
        }
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

    const REMOTE_TEMPLATE: &str = r#"
id = "remote-http"
name = "Remote HTTP"
description = "Secret-backed Streamable HTTP fixture"
category = "ai"

[transport]
type = "http"
url = "https://mcp.example.com/mcp"

[[required_env]]
name = "LIBREFANG_TEST_8578_REMOTE_KEY"
label = "API Key"
help = "h"

[[headers]]
name = "Authorization"
value = "Bearer ${LIBREFANG_TEST_8578_REMOTE_KEY}"

[[headers]]
name = "X-Client"
value = "librefang"
"#;

    /// #8578: an HTTP catalog entry's header bindings reach the installed server as unresolved templates, next to the `env` declaration the runtime resolves them against.
    #[test]
    fn http_template_headers_are_installed_as_unresolved_references() {
        let entry: McpCatalogEntry = toml::from_str(REMOTE_TEMPLATE).unwrap();
        let server = catalog_entry_to_mcp_server(&entry);
        assert_eq!(
            server.headers,
            vec![
                "Authorization: Bearer ${LIBREFANG_TEST_8578_REMOTE_KEY}".to_string(),
                "X-Client: librefang".to_string(),
            ]
        );
        assert_eq!(server.env, vec!["LIBREFANG_TEST_8578_REMOTE_KEY"]);
        assert!(matches!(
            server.transport,
            Some(McpTransportEntry::Http { ref url }) if url == "https://mcp.example.com/mcp"
        ));
    }

    /// Entries without header bindings keep installing with no headers.
    #[test]
    fn template_without_headers_installs_no_headers() {
        let entry: McpCatalogEntry = toml::from_str(
            r#"
id = "plain"
name = "Plain"
description = "d"
category = "devtools"

[transport]
type = "http"
url = "https://mcp.example.com/mcp"
"#,
        )
        .unwrap();
        assert!(catalog_entry_to_mcp_server(&entry).headers.is_empty());
    }

    /// #8578: an empty credential is not a credential — it must not be stored or count as provided, so the install reports `Setup` instead of a `Ready` server that sends `Authorization: Bearer `.
    #[test]
    fn empty_provided_credential_is_treated_as_missing() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("mcp").join("catalog");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("remote-http.toml"), REMOTE_TEMPLATE).unwrap();
        let mut catalog = McpCatalog::new(home.path());
        assert_eq!(catalog.load(home.path()), 1);

        // No vault: storing anything would fail with `ExtensionError::Vault`, so reaching `Ok` proves the blank value was never stored.
        let mut resolver = CredentialResolver::new(None, None);
        let provided = HashMap::from([(
            "LIBREFANG_TEST_8578_REMOTE_KEY".to_string(),
            "   ".to_string(),
        )]);
        let result = install_integration(&catalog, &mut resolver, "remote-http", &provided)
            .expect("blank credential must not be written to the vault");
        assert_eq!(result.status, McpStatus::Setup);
        assert_eq!(
            result.missing_credentials,
            vec!["LIBREFANG_TEST_8578_REMOTE_KEY".to_string()]
        );
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
