//! `mcp_cmds` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// MCP server commands (librefang mcp {add,remove,list,catalog})
// ---------------------------------------------------------------------------

pub(crate) fn cmd_mcp_add(name: &str, key: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Check template exists
    let template = match catalog.get(name) {
        Some(t) => t.clone(),
        None => {
            ui::error(&i18n::t_args(
                "mcp-catalog-unknown-entry",
                &[("name", name)],
            ));
            println!("{}", i18n::t("mcp-catalog-available-header"));
            for t in catalog.list() {
                println!("  {} {} — {}", t.icon, t.id, t.description);
            }
            std::process::exit(1);
        }
    };

    // Reject re-install of an already-configured server by name/template_id.
    // The API path returns 409 here; the CLI was silently overwriting the
    // existing [[mcp_servers]] entry (including edited transport/env/oauth)
    // because upsert_mcp_server_local replaces by name. Users should remove
    // first if they want to re-install.
    let config_path = home.join("config.toml");
    if config_path.is_file() {
        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                ui::error(&i18n::t_args(
                    "mcp-failed-read-config",
                    &[
                        ("path", &config_path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ));
                std::process::exit(1);
            }
        };
        let parsed: toml::value::Table = match toml::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                ui::error(&i18n::t_args(
                    "mcp-invalid-toml",
                    &[
                        ("path", &config_path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ));
                std::process::exit(1);
            }
        };
        if let Some(toml::Value::Array(servers)) = parsed.get("mcp_servers") {
            let conflict = servers.iter().any(|v| {
                let t = match v.as_table() {
                    Some(t) => t,
                    None => return false,
                };
                let matches_field = |k: &str| t.get(k).and_then(|n| n.as_str()) == Some(name);
                matches_field("name") || matches_field("template_id")
            });
            if conflict {
                ui::error(&format!(
                    "MCP server '{name}' is already configured. Run \
                     `librefang mcp remove {name}` first if you want to re-install."
                ));
                std::process::exit(1);
            }
        }
    }

    // Set up credential resolver (vault + dotenv + interactive prompt fallback)
    let dotenv_path = home.join(".env");
    let vault_path = home.join("vault.enc");
    let vault = if vault_path.exists() {
        let mut v = librefang_extensions::vault::CredentialVault::new(vault_path);
        if v.unlock().is_ok() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    };
    let mut resolver =
        librefang_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path))
            .with_interactive(true);

    // Build provided keys map
    let mut provided_keys = std::collections::HashMap::new();
    if let Some(key_value) = key {
        // Auto-detect which env var to use (first required_env that's a secret)
        if let Some(env_var) = template.required_env.iter().find(|e| e.is_secret) {
            provided_keys.insert(env_var.name.clone(), key_value.to_string());
        }
    }

    let result = match librefang_extensions::installer::install_integration(
        &catalog,
        &mut resolver,
        name,
        &provided_keys,
    ) {
        Ok(r) => r,
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    };

    // Persist the new [[mcp_servers]] entry directly into config.toml.
    let config_path = home.join("config.toml");
    if let Err(e) = upsert_mcp_server_local(&config_path, &result.server) {
        ui::error(&i18n::t_args(
            "mcp-failed-write-config",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    match &result.status {
        librefang_types::mcp::McpStatus::Ready => ui::success(&result.message),
        librefang_types::mcp::McpStatus::Setup => {
            println!("{}", result.message.yellow());
            println!("{}", i18n::t("mcp-add-credentials-hint"));
            for env in &template.required_env {
                if env.is_secret {
                    println!("  librefang vault set {}  # {}", env.name, env.help);
                    if let Some(ref url) = env.get_url {
                        println!("{}", i18n::t_args("mcp-get-it-here", &[("url", url)]));
                    }
                }
            }
        }
        _ => println!("{}", result.message),
    }

    // If daemon is running, trigger hot-reload.
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

pub(crate) fn cmd_mcp_remove(name: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    // Resolve by template_id first, fall back to server name.
    let target_name: Option<String> = {
        let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
        let doc: toml::Value =
            toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
        doc.as_table()
            .and_then(|t| t.get("mcp_servers"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|entry| {
                    let tbl = entry.as_table()?;
                    let tid = tbl.get("template_id").and_then(|v| v.as_str());
                    let nm = tbl.get("name").and_then(|v| v.as_str())?;
                    if tid == Some(name) || nm == name {
                        Some(nm.to_string())
                    } else {
                        None
                    }
                })
            })
    };

    let target_name = match target_name {
        Some(n) => n,
        None => {
            ui::error(&i18n::t_args("mcp-not-configured", &[("name", name)]));
            std::process::exit(1);
        }
    };

    if let Err(e) = remove_mcp_server_local(&config_path, &target_name) {
        ui::error(&i18n::t_args(
            "mcp-failed-update-config",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    ui::success(&i18n::t_args(
        "mcp-removed-success",
        &[("name", &target_name)],
    ));

    // Hot-reload daemon
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

pub(crate) fn cmd_mcp_catalog(query: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Installed state comes from config.mcp_servers' template_id field.
    let installed_template_ids: std::collections::HashSet<String> = {
        let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
        toml::from_str::<toml::Value>(&raw)
            .ok()
            .and_then(|v| v.as_table().cloned())
            .and_then(|t| t.get("mcp_servers").cloned())
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| {
                        v.as_table()
                            .and_then(|t| t.get("template_id"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let entries: Vec<_> = if let Some(q) = query {
        catalog.search(q).into_iter().cloned().collect()
    } else {
        catalog.list().into_iter().cloned().collect()
    };

    if entries.is_empty() {
        if let Some(q) = query {
            println!("No MCP catalog entries matching '{q}'.");
        } else {
            println!("No MCP catalog entries available.");
        }
        return;
    }

    // Group by category
    let mut by_category: std::collections::BTreeMap<
        String,
        Vec<&librefang_types::mcp::McpCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for entry in &entries {
        by_category
            .entry(entry.category.to_string())
            .or_default()
            .push(entry);
    }

    for (category, items) in &by_category {
        println!("\n{}", format!("  {category}").bold());
        for item in items {
            let status_badge = if installed_template_ids.contains(&item.id) {
                "[Installed]".green().to_string()
            } else {
                "[Available]".dimmed().to_string()
            };
            println!(
                "    {} {:<20} {:<13} {}",
                item.icon, item.id, status_badge, item.description
            );
        }
    }
    println!();
    println!(
        "  {} catalog entries ({} installed)",
        entries.len(),
        entries
            .iter()
            .filter(|e| installed_template_ids.contains(&e.id))
            .count()
    );
    println!("  Use `librefang mcp add <id>` to install an MCP server.");
}

pub(crate) fn cmd_mcp_list() {
    let home = librefang_home();
    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let doc: toml::Value = toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
    let servers = doc
        .as_table()
        .and_then(|t| t.get("mcp_servers"))
        .and_then(|v| v.as_array());
    let Some(servers) = servers else {
        println!("No MCP servers configured.");
        return;
    };
    if servers.is_empty() {
        println!("No MCP servers configured.");
        return;
    }
    println!();
    println!(
        "  {:<28} {:<14} {:<18} details",
        "name", "template_id", "transport"
    );
    for entry in servers {
        let Some(tbl) = entry.as_table() else {
            continue;
        };
        let name = tbl.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let tid = tbl
            .get("template_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let (transport, detail) = match tbl.get("transport").and_then(|v| v.as_table()) {
            Some(t) => {
                let ttype = t.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                let detail = match ttype {
                    "stdio" => t
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    "sse" | "http" => t
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => String::new(),
                };
                (ttype.to_string(), detail)
            }
            None => ("-".to_string(), String::new()),
        };
        println!("  {name:<28} {tid:<14} {transport:<18} {detail}");
    }
    println!();
    println!("  Use `librefang mcp catalog` to list installable entries.");
}

/// Local upsert helper — mirrors the API's `upsert_mcp_server_config`.
pub(crate) fn upsert_mcp_server_local(
    config_path: &std::path::Path,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting. A
        // malformed config.toml would otherwise be overwritten as a new
        // near-empty file, wiping unrelated sections the user may want
        // to fix by hand.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        toml::value::Table::new()
    };

    let entry_json = serde_json::to_value(entry).map_err(|e| e.to_string())?;
    let entry_toml = json_to_toml_value_cli(&entry_json);

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));

    if let toml::Value::Array(ref mut arr) = servers {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != entry.name)
                .unwrap_or(true)
        });
        arr.push(entry_toml);
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn remove_mcp_server_local(
    config_path: &std::path::Path,
    name: &str,
) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        return Ok(());
    };
    if let Some(toml::Value::Array(ref mut arr)) = table.get_mut("mcp_servers") {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != name)
                .unwrap_or(true)
        });
    }
    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}
