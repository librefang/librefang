//! `init` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub(crate) fn cmd_init(quick: bool) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error(&i18n::t("error-home-dir"));
            std::process::exit(1);
        }
    };

    let librefang_dir = cli_librefang_home();

    // When an existing config is detected in interactive mode, redirect to the
    // upgrade path so user settings (channels, keys, etc.) are preserved.
    // The interactive wizard unconditionally overwrites config.toml, which
    // would silently delete channels and custom configuration (#1862).
    if !quick && librefang_dir.join("config.toml").exists() {
        ui::hint(&i18n::t("init-upgrade-existing"));
        ui::hint(&i18n::t("init-upgrade-fresh-hint"));
        cmd_init_upgrade();
        return;
    }

    // --- Ensure directories exist ---
    if !librefang_dir.exists() {
        std::fs::create_dir_all(&librefang_dir).unwrap_or_else(|e| {
            ui::error_with_fix(
                &i18n::t_args(
                    "error-create-dir",
                    &[("path", &librefang_dir.display().to_string())],
                ),
                &i18n::t_args(
                    "error-create-dir-fix",
                    &[("path", &home.display().to_string())],
                ),
            );
            eprintln!("  {e}");
            std::process::exit(1);
        });
        restrict_dir_permissions(&librefang_dir);
    }

    let data_dir = librefang_dir.join("data");
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
            eprintln!("Error creating data dir: {e}");
            std::process::exit(1);
        });
    }

    // Sync registry content (downloads to registry/, pre-installs providers/integrations/assistant)
    librefang_runtime::registry_sync::sync_registry(
        &librefang_dir,
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
        None,
    );

    // Initialize vault if not already initialized
    init_vault_if_missing(&librefang_dir);

    // Initialize git repo for config version control
    init_git_if_missing(&librefang_dir);

    if quick {
        cmd_init_quick(&librefang_dir);
    } else if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        ui::hint(&i18n::t("hint-non-interactive"));
        ui::hint(&i18n::t("hint-non-interactive-wizard"));
        cmd_init_quick(&librefang_dir);
    } else {
        cmd_init_interactive(&librefang_dir);
    }

    // Fallback: ensure config.toml exists even if wizard was cancelled/failed
    let config_path = librefang_dir.join("config.toml");
    if !config_path.exists() {
        let (provider, api_key_env, model) = detect_best_provider();
        write_config_if_missing(&librefang_dir, &provider, &model, &api_key_env);
    }
}

/// Upgrade an existing LibreFang installation: backup config, sync registry, merge new defaults.
pub(crate) fn cmd_init_upgrade() {
    let librefang_dir = cli_librefang_home();
    let config_path = librefang_dir.join("config.toml");

    // 1. Must have an existing installation
    if !config_path.exists() {
        ui::error(&i18n::t("init-upgrade-no-config"));
        std::process::exit(1);
    }

    ui::banner();
    ui::blank();
    ui::section(&i18n::t("init-upgrade-title"));

    // Four upgrade steps: backup, registry sync, vault/git, config merge.
    let mut p = progress::auto(&i18n::t("init-upgrade-progress-label"), Some(4));

    // 2. Backup existing config under backups/ (keep last 3)
    p.set_message(&i18n::t("init-upgrade-backing-up"));
    let backups_dir = librefang_dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        p.finish_with_failure(&i18n::t_args(
            "init-upgrade-failed-create-backups-dir",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }
    let backup_name = format!("config-{}.toml", format_local_timestamp());
    let backup_path = backups_dir.join(&backup_name);
    if let Err(e) = std::fs::copy(&config_path, &backup_path) {
        p.finish_with_failure(&i18n::t_args(
            "init-upgrade-failed-backup-config",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }
    restrict_file_permissions(&backup_path);
    prune_old_config_backups(&backups_dir, 3);
    p.tick(1);
    ui::success(&i18n::t_args(
        "init-upgrade-backup-success",
        &[("name", &backup_name)],
    ));

    // 3. Sync registry (TTL=0 forces refresh regardless of last sync time)
    p.set_message(&i18n::t("init-upgrade-syncing-registry"));
    if librefang_runtime::registry_sync::sync_registry(&librefang_dir, 0, "", None) {
        p.tick(1);
        ui::success(&i18n::t("init-upgrade-registry-synced"));
    } else {
        p.tick(1);
        ui::hint(&i18n::t("init-upgrade-registry-failed"));
    }

    // 4. Ensure data dir, vault, and git exist
    p.set_message(&i18n::t("init-upgrade-initializing-vault-git"));
    let data_dir = librefang_dir.join("data");
    if !data_dir.exists() {
        let _ = std::fs::create_dir_all(&data_dir);
    }
    init_vault_if_missing(&librefang_dir);
    init_git_if_missing(&librefang_dir);

    // Ensure .gitignore excludes the backups/ directory (may be missing in older installations)
    let gitignore = librefang_dir.join(".gitignore");
    if gitignore.exists() {
        if let Ok(content) = std::fs::read_to_string(&gitignore) {
            if !content.lines().any(|l| l.trim() == "backups/") {
                let _ = std::fs::write(&gitignore, format!("{content}backups/\n"));
            }
        }
    }
    p.tick(1);

    // 5. Merge new default config fields
    p.set_message(&i18n::t("init-upgrade-merging-config"));
    let existing_raw = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) => {
            p.finish_with_failure(&i18n::t_args(
                "init-upgrade-failed-read",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };

    let existing: toml::Value = match toml::from_str(&existing_raw) {
        Ok(v) => v,
        Err(e) => {
            p.finish_with_failure(&i18n::t_args(
                "init-upgrade-failed-parse",
                &[("error", &e.to_string())],
            ));
            ui::hint(&i18n::t_args(
                "init-upgrade-backup-saved-hint",
                &[("name", &backup_name)],
            ));
            std::process::exit(1);
        }
    };

    let (provider, api_key_env, model) = detect_best_provider();
    let default_config_str = render_init_default_config(&provider, &model, &api_key_env);
    let defaults: toml::Value = match toml::from_str(&default_config_str) {
        Ok(v) => v,
        Err(e) => {
            p.finish_with_failure(&i18n::t_args(
                "init-upgrade-failed-parse-template",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };

    // Find top-level keys/sections missing from user config and append them
    // as TOML fragments. This preserves the original file's comments and formatting.
    let added = find_missing_toplevel_keys(&existing, &defaults);

    if added.is_empty() {
        ui::success(&i18n::t("init-upgrade-config-up-to-date"));
    } else {
        // Partition into scalars (must stay in TOML root scope) and tables.
        // Scalars appended after a [table] header would be absorbed into that
        // table's scope, potentially colliding with same-named sub-keys (#2021).
        let (scalar_keys, table_keys): (Vec<_>, Vec<_>) = added
            .iter()
            .partition(|k| defaults.get(*k).is_none_or(|v| !v.is_table()));

        let mut content = existing_raw.clone();

        // Insert scalar keys before the first [table] header so they remain
        // top-level in the TOML document.
        if !scalar_keys.is_empty() {
            let mut scalar_snippet = String::new();
            for key in &scalar_keys {
                if let Some(val) = defaults.get(*key) {
                    let mut fragment = toml::map::Map::new();
                    fragment.insert((*key).clone(), val.clone());
                    if let Ok(s) = toml::to_string_pretty(&toml::Value::Table(fragment)) {
                        scalar_snippet.push_str(&s);
                    }
                }
            }
            // Find the first line that starts with '[' (a table header).
            // We search for "\n[" then insert just before the '['.
            if let Some(pos) = content.find("\n[").map(|p| p + 1) {
                content.insert_str(pos, &format!("{scalar_snippet}\n"));
            } else {
                // No table headers in file — appending is safe.
                content.push('\n');
                content.push_str(&scalar_snippet);
            }
        }

        // Append table sections at the end of the file.
        if !table_keys.is_empty() {
            content.push_str("\n# ── Added by upgrade ────────────────────────────────────\n");
            for key in &table_keys {
                if let Some(val) = defaults.get(*key) {
                    let mut fragment = toml::map::Map::new();
                    fragment.insert((*key).clone(), val.clone());
                    if let Ok(snippet) = toml::to_string_pretty(&toml::Value::Table(fragment)) {
                        content.push('\n');
                        content.push_str(&snippet);
                    }
                }
            }
        }

        if let Err(e) = std::fs::write(&config_path, &content) {
            p.finish_with_failure(&i18n::t_args(
                "init-upgrade-failed-write",
                &[("error", &e.to_string())],
            ));
            ui::hint(&i18n::t_args(
                "init-upgrade-backup-saved-hint",
                &[("name", &backup_name)],
            ));
            std::process::exit(1);
        }
        restrict_file_permissions(&config_path);
        ui::success(&i18n::t_args(
            "init-upgrade-sections-added",
            &[("count", &added.len().to_string())],
        ));
        for key in &added {
            ui::kv("  +", key);
        }
    }
    p.tick(1);
    p.finish(&i18n::t("init-upgrade-steps-complete"));

    // 6. Check for legacy ~/.openclaw installation
    if let Some(home) = dirs::home_dir() {
        let openclaw_dir = home.join(".openclaw");
        if openclaw_dir.exists() {
            ui::blank();
            ui::hint(&i18n::t("init-upgrade-legacy-openclaw"));
            ui::hint(&i18n::t("init-upgrade-legacy-openclaw-hint"));
        }
    }

    // 7. Warn users whose require_approval list predates the file_write default (#1861).
    // The default was expanded to include file_write and file_delete, but users who
    // had an explicit `require_approval = [...]` entry in their config won't pick up
    // the new default automatically.
    let approval_needs_update = existing
        .get("approval")
        .and_then(|a| a.get("require_approval"))
        .and_then(|r| r.as_array())
        .is_some_and(|list| {
            let has_shell = list.iter().any(|v| v.as_str() == Some("shell_exec"));
            let missing_new = ["file_write", "file_delete", "apply_patch"]
                .iter()
                .any(|tool| !list.iter().any(|v| v.as_str() == Some(*tool)));
            has_shell && missing_new
        });
    if approval_needs_update {
        ui::blank();
        ui::hint(&i18n::t("init-upgrade-approval-warning"));
        ui::hint(&i18n::t("init-upgrade-approval-hint"));
    }

    // 8. Summary
    ui::blank();
    ui::success(&i18n::t("init-upgrade-success-summary"));
    ui::kv(&i18n::t("label-backup"), &format!("backups/{backup_name}"));
    if !added.is_empty() {
        ui::kv(&i18n::t("label-new-fields"), &added.len().to_string());
    }
    ui::blank();
}

/// Keep only the `keep` most recent `config-*.toml` backups under `backups_dir`.
/// The embedded `YYYYMMDD-HHMMSS` timestamp sorts lexicographically, so a
/// filename sort gives the same order as a chronological sort.
pub(crate) fn prune_old_config_backups(backups_dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return;
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            if name.starts_with("config-") && name.ends_with(".toml") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    if files.len() > keep {
        for old in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Find top-level keys in `defaults` that are missing from `existing`.
/// Only checks top-level — does not recurse into sub-tables to avoid
/// injecting partial sections the user intentionally omitted.
pub(crate) fn find_missing_toplevel_keys(
    existing: &toml::Value,
    defaults: &toml::Value,
) -> Vec<String> {
    let (Some(existing_table), Some(defaults_table)) = (existing.as_table(), defaults.as_table())
    else {
        return Vec::new();
    };
    defaults_table
        .keys()
        .filter(|k| !existing_table.contains_key(*k))
        .cloned()
        .collect()
}

/// Initialize vault if it doesn't exist yet (silent no-op if already initialized).
pub(crate) fn init_vault_if_missing(librefang_dir: &std::path::Path) {
    let vault_path = librefang_dir.join("vault.enc");
    if vault_path.exists() {
        return; // Already initialized
    }

    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
    if let Err(e) = vault.init() {
        // Silently skip vault init on failure - it's optional
        tracing::debug!("vault init skipped: {e}");
    }
}

/// Initialize a git repo in ~/.librefang/ for config version control.
pub(crate) fn init_git_if_missing(librefang_dir: &std::path::Path) {
    if librefang_dir.join(".git").exists() {
        return;
    }

    let Ok(status) = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(librefang_dir)
        .status()
    else {
        tracing::debug!("git not available, skipping repo init");
        return;
    };
    if !status.success() {
        tracing::debug!("git init failed");
        return;
    }

    // Write .gitignore for sensitive/temporary files
    let gitignore = librefang_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(
            &gitignore,
            "secrets.env\nvault.enc\ndaemon.json\nlogs/\ncache/\nregistry/\ndata/\nbackups/\n*.db\n*.db-shm\n*.db-wal\n",
        );
    }

    // Initial commit
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(librefang_dir)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-q", "-m", "chore: initial librefang config"])
        .current_dir(librefang_dir)
        .status();
}

/// Quick init: no prompts, auto-detect, write config + .env, print next steps.
pub(crate) fn cmd_init_quick(librefang_dir: &std::path::Path) {
    ui::banner();
    ui::blank();

    let (provider, api_key_env, model) = detect_best_provider();

    write_config_if_missing(librefang_dir, &provider, &model, &api_key_env);

    ui::blank();
    ui::success(&i18n::t("init-quick-success"));
    ui::kv(&i18n::t("label-provider"), &provider);
    ui::kv(&i18n::t("label-model"), &model);
    ui::blank();
    ui::next_steps(&[&i18n::t("init-next-start"), &i18n::t("init-next-chat")]);
}

/// Interactive 5-step onboarding wizard (ratatui TUI).
pub(crate) fn cmd_init_interactive(librefang_dir: &std::path::Path) {
    use tui::screens::init_wizard::{self, InitResult, LaunchChoice};

    match init_wizard::run() {
        InitResult::Completed {
            provider,
            model,
            daemon_started,
            launch,
        } => {
            // Print summary after TUI restores terminal
            ui::blank();
            ui::success(&i18n::t("init-interactive-success"));
            ui::kv(&i18n::t("label-provider"), &provider);
            ui::kv(&i18n::t("label-model"), &model);

            if daemon_started {
                ui::kv_ok(&i18n::t("label-daemon"), "running");
            }
            ui::blank();

            // Execute the user's chosen launch action.
            match launch {
                LaunchChoice::Desktop => {
                    launch_desktop_app(librefang_dir);
                }
                LaunchChoice::Dashboard => {
                    if let Some(base) = find_daemon() {
                        let url = format!("{base}/");
                        ui::success(&i18n::t_args("dashboard-opening", &[("url", &url)]));
                        if !open_in_browser(&url) {
                            ui::hint(&i18n::t_args(
                                "hint-could-not-open-browser-visit",
                                &[("url", &url)],
                            ));
                        }
                    } else {
                        ui::error(&i18n::t("daemon-not-running-start"));
                    }
                }
                LaunchChoice::Chat => {
                    ui::hint(&i18n::t("hint-starting-chat"));
                    ui::blank();
                    // Note: tracing was initialized for stderr (init is a CLI
                    // subcommand).  The chat TUI takes over the terminal with
                    // raw mode so stderr output is suppressed.  We can't
                    // reinitialize tracing (global subscriber is set once).
                    cmd_quick_chat(None, None);
                }
            }
        }
        InitResult::Cancelled => {
            println!("  {}", i18n::t("init-cancelled"));
        }
    }
}

/// Launch the librefang-desktop Tauri app, connecting to the running daemon.
pub(crate) fn launch_desktop_app(_librefang_dir: &std::path::Path) {
    if let Some(path) = desktop_install::find_desktop_binary() {
        desktop_install::launch(&path);
        return;
    }

    // Not installed — offer to download
    if let Some(installed) = desktop_install::prompt_and_install() {
        desktop_install::launch(&installed);
    }
}

/// Auto-detect the best available provider.
///
/// Delegates to the runtime's `detect_available_provider()` which probes 13+
/// providers (OpenAI, Anthropic, Gemini, Groq, DeepSeek, OpenRouter, Mistral,
/// Together, Fireworks, xAI, Perplexity, Cohere, Azure OpenAI) plus the
/// GOOGLE_API_KEY alias.  Falls back to local Ollama, then the interactive
/// free-provider TUI guide.
pub(crate) fn detect_best_provider() -> (String, String, String) {
    // 1. Check all cloud provider API keys via the runtime registry
    if let Some((provider, _model, env_var)) =
        librefang_runtime::drivers::detect_available_provider()
    {
        // Capitalize provider name for display (e.g. "groq" → "Groq")
        let display_name = {
            let mut c = provider.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + c.as_str(),
            }
        };
        // CLI-backed providers return an empty env_var (auth via OAuth token
        // or keychain, not an env variable). Display a readable placeholder
        // so the i18n message doesn't end with an empty parenthetical.
        let auth_display = if env_var.is_empty() {
            "CLI login"
        } else {
            env_var
        };
        ui::success(&i18n::t_args(
            "detected-provider",
            &[("display", &display_name), ("env_var", auth_display)],
        ));
        return (
            provider.to_string(),
            env_var.to_string(),
            default_model_for_provider(provider),
        );
    }

    // 2. Check if Ollama is running locally (no API key needed)
    if check_ollama_available() {
        ui::success(&i18n::t("detected-ollama"));
        return (
            "ollama".to_string(),
            "OLLAMA_API_KEY".to_string(),
            default_model_for_provider("ollama"),
        );
    }

    // 3. No API key found — launch TUI guide to pick a free provider
    {
        if let Some(result) = guide_free_provider_setup() {
            return result;
        }
    }

    // 4. Non-interactive fallback: just print hints
    ui::hint(&i18n::t("hint-no-api-keys"));
    ui::hint(&i18n::t("hint-groq-free"));
    ui::hint(&i18n::t("hint-gemini-free"));
    ui::hint(&i18n::t("hint-deepseek-free"));
    ui::hint(&i18n::t("hint-ollama-local"));
    (
        "groq".to_string(),
        "GROQ_API_KEY".to_string(),
        default_model_for_provider("groq"),
    )
}

/// Interactive TUI guide: help user pick a free LLM provider and set up an API key.
/// Returns `Some((provider, env_var, model))` on success, `None` if user cancels.
pub(crate) fn guide_free_provider_setup() -> Option<(String, String, String)> {
    use tui::screens::free_provider_guide::{self, GuideResult};

    match free_provider_guide::run() {
        GuideResult::Completed { provider, env_var } => {
            ui::success(&i18n::t_args("config-saved-key", &[("env_var", &env_var)]));
            let model = default_model_for_provider(&provider);
            Some((provider, env_var, model))
        }
        GuideResult::Skipped => None,
    }
}

/// Quick probe to check if Ollama is running on localhost.
pub(crate) fn check_ollama_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 11434)),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

pub(crate) fn render_init_default_config(provider: &str, model: &str, api_key_env: &str) -> String {
    INIT_DEFAULT_CONFIG_TEMPLATE
        .replace("{{provider}}", provider)
        .replace("{{model}}", model)
        .replace("{{api_key_env}}", api_key_env)
}

pub(crate) fn default_model_for_provider(provider: &str) -> String {
    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    catalog
        .default_model_for_provider(provider)
        .unwrap_or_else(|| "local-model".to_string())
}

/// Write config.toml if it doesn't already exist.
pub(crate) fn write_config_if_missing(
    librefang_dir: &std::path::Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    let config_path = librefang_dir.join("config.toml");
    if config_path.exists() {
        ui::check_ok(&i18n::t_args(
            "error-config-exists",
            &[("path", &config_path.display().to_string())],
        ));
    } else {
        let default_config = render_init_default_config(provider, model, api_key_env);
        std::fs::write(&config_path, &default_config).unwrap_or_else(|e| {
            ui::error_with_fix(&i18n::t("error-write-config"), &e.to_string());
            std::process::exit(1);
        });
        restrict_file_permissions(&config_path);
        ui::success(&i18n::t_args(
            "error-config-created",
            &[("path", &config_path.display().to_string())],
        ));
    }

    // Write config.example.toml with the full annotated template for reference
    let example_path = librefang_dir.join("config.example.toml");
    if !example_path.exists() {
        if let Err(e) = std::fs::write(&example_path, INIT_DEFAULT_CONFIG_TEMPLATE) {
            ui::hint(&i18n::t_args(
                "init-error-write-config-example",
                &[("error", &e.to_string())],
            ));
        }
    }
}
