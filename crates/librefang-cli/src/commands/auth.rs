//! `auth` CLI command handlers (vault, `auth chatgpt`, credential pools,
//! hash-password), split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

/// Outcome of starting the ChatGPT device-auth flow: either continue with the
/// device prompt, or fall back to browser auth (with a reason).
pub(crate) enum DeviceAuthNextStep {
    ContinueDevice(librefang_runtime::chatgpt_oauth::DeviceAuthPrompt),
    FallbackToBrowser(String),
}

pub(crate) fn resolve_device_auth_start(
    result: Result<
        librefang_runtime::chatgpt_oauth::DeviceAuthPrompt,
        librefang_runtime::chatgpt_oauth::DeviceAuthFlowError,
    >,
) -> Result<DeviceAuthNextStep, String> {
    match result {
        Ok(prompt) => Ok(DeviceAuthNextStep::ContinueDevice(prompt)),
        Err(librefang_runtime::chatgpt_oauth::DeviceAuthFlowError::BrowserFallback { message }) => {
            Ok(DeviceAuthNextStep::FallbackToBrowser(message))
        }
        Err(err) => Err(err.to_string()),
    }
}

pub(crate) async fn authenticate_chatgpt(
    device_auth: bool,
) -> Result<librefang_runtime::chatgpt_oauth::ChatGptAuthResult, String> {
    use librefang_runtime::chatgpt_oauth;

    if device_auth {
        match resolve_device_auth_start(chatgpt_oauth::start_device_auth_flow().await)? {
            DeviceAuthNextStep::ContinueDevice(prompt) => {
                println!("{}", i18n::t("auth-chatgpt-device-requested"));
                println!(
                    "{}",
                    i18n::t_args(
                        "auth-chatgpt-device-open-url",
                        &[("url", chatgpt_oauth::DEVICE_AUTH_URL)]
                    )
                );
                println!(
                    "{}",
                    i18n::t_args(
                        "auth-chatgpt-device-one-time-code",
                        &[("code", &prompt.user_code)]
                    )
                );
                println!("{}", i18n::t("auth-chatgpt-device-do-not-share"));
                println!("{}", i18n::t("auth-chatgpt-device-waiting"));
                return chatgpt_oauth::poll_device_auth_flow(&prompt).await;
            }
            DeviceAuthNextStep::FallbackToBrowser(message) => {
                println!("{message}");
                println!("{}", i18n::t("auth-chatgpt-switching-browser"));
            }
        }
    }

    let (auth_url, port, code_verifier, state) = chatgpt_oauth::start_oauth_flow().await?;

    println!("{}", i18n::t("auth-chatgpt-opening-browser"));
    println!(
        "{}",
        i18n::t_args("auth-chatgpt-open-manually-hint", &[("url", &auth_url)])
    );

    if let Err(e) = open::that(&auth_url) {
        eprintln!(
            "{}",
            i18n::t_args(
                "auth-chatgpt-open-browser-failed",
                &[("error", &e.to_string())]
            )
        );
        eprintln!(
            "{}",
            i18n::t_args("auth-chatgpt-open-manually", &[("url", &auth_url)])
        );
    }

    let code = chatgpt_oauth::run_oauth_callback_server(port, &state).await?;
    chatgpt_oauth::exchange_code_for_tokens(&code, &code_verifier, port).await
}

pub(crate) async fn persist_chatgpt_auth(
    auth_result: librefang_runtime::chatgpt_oauth::ChatGptAuthResult,
) -> Result<(), String> {
    use librefang_runtime::chatgpt_oauth;

    let home = librefang_home();
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("Failed to create LibreFang home directory: {e}"))?;

    let access_token = auth_result.access_token;
    let refresh_token = auth_result.refresh_token;
    let secrets_path = write_chatgpt_secrets(
        &home,
        access_token.as_str(),
        refresh_token.as_ref().map(|rt| rt.as_str()),
    )?;

    println!(
        "{}",
        i18n::t_args(
            "auth-chatgpt-tokens-saved",
            &[("path", &secrets_path.display().to_string())]
        )
    );

    println!("{}", i18n::t("auth-chatgpt-detecting-model"));
    let best_model = chatgpt_oauth::fetch_best_codex_model(&access_token).await;
    println!(
        "{}",
        i18n::t_args("auth-chatgpt-selected-model", &[("model", &best_model)])
    );

    update_chatgpt_config(&home, &best_model)?;

    println!(
        "{}",
        i18n::t_args("auth-chatgpt-config-updated", &[("model", &best_model)])
    );
    Ok(())
}

pub(crate) fn write_chatgpt_secrets(
    home: &std::path::Path,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let secrets_path = home.join("secrets.env");
    let mut env_vars: Vec<(String, String)> = vec![(
        "CHATGPT_SESSION_TOKEN".to_string(),
        access_token.to_string(),
    )];
    if let Some(rt) = refresh_token {
        env_vars.push(("CHATGPT_REFRESH_TOKEN".to_string(), rt.to_string()));
    }

    let existing = std::fs::read_to_string(&secrets_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            !l.starts_with("CHATGPT_SESSION_TOKEN=") && !l.starts_with("CHATGPT_REFRESH_TOKEN=")
        })
        .map(|l| l.to_string())
        .collect();

    for (key, val) in &env_vars {
        lines.push(format!("{key}={val}"));
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }

    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("Failed to write secrets.env: {e}"))?;

    Ok(secrets_path)
}

pub(crate) fn update_chatgpt_config(
    home: &std::path::Path,
    best_model: &str,
) -> Result<(), String> {
    let config_path = home.join("config.toml");
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if config_str.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        config_str
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("Failed to parse config.toml: {e}"))?
    };

    let dm = doc
        .entry("default_model")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("default_model is not a table")?;
    dm.insert("provider", toml_edit::value("chatgpt"));
    dm.insert("api_key_env", toml_edit::value("CHATGPT_SESSION_TOKEN"));
    dm.insert("model", toml_edit::value(best_model));
    dm.insert(
        "base_url",
        toml_edit::value(librefang_runtime::chatgpt_oauth::CHATGPT_BASE_URL),
    );

    std::fs::write(&config_path, doc.to_string())
        .map_err(|e| format!("Failed to write config.toml: {e}"))?;

    Ok(())
}

pub(crate) fn cmd_auth_chatgpt(device_auth: bool) {
    println!("{}", i18n::t("auth-chatgpt-starting-flow"));

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let result: Result<(), String> = rt.block_on(async {
        let auth_result = authenticate_chatgpt(device_auth).await?;
        persist_chatgpt_auth(auth_result).await
    });

    match result {
        Ok(()) => ui::success(&i18n::t("auth-chatgpt-complete")),
        Err(e) => {
            ui::error(&i18n::t_args(
                "auth-chatgpt-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

// ─── Credential pool commands (#4965) ───────────────────────────────────────

/// Resolve the active config.toml path. `--config <path>` overrides; else
/// `$LIBREFANG_HOME/config.toml` (or `~/.librefang/config.toml`).
pub(crate) fn pool_config_path(config_override: Option<PathBuf>) -> PathBuf {
    config_override.unwrap_or_else(|| librefang_home().join("config.toml"))
}

/// Parse config.toml into a `toml_edit::DocumentMut` so comments, blank
/// lines, key ordering, and unrelated sections are preserved through any
/// mutation. Exits with a friendly message on missing-file / parse errors.
/// Shared by all three mutating pool commands so the same diagnostic appears
/// for each entry point.
pub(crate) fn pool_load_doc_or_exit(path: &std::path::Path) -> toml_edit::DocumentMut {
    if !path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    if content.trim().is_empty() {
        return toml_edit::DocumentMut::new();
    }
    content
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|e| {
            ui::error_with_fix(
                &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
                &i18n::t("config-parse-fix-alt"),
            );
            std::process::exit(1);
        })
}

pub(crate) fn pool_write_doc_or_exit(path: &std::path::Path, doc: &toml_edit::DocumentMut) {
    std::fs::write(path, doc.to_string()).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "auth-write-failed",
            &[
                ("path", &path.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
        std::process::exit(1);
    });
}

pub(crate) fn pool_strategy_canon(input: &str) -> Option<&'static str> {
    match input.to_ascii_lowercase().replace('-', "_").as_str() {
        "fill_first" | "fillfirst" => Some("fill_first"),
        "round_robin" | "roundrobin" => Some("round_robin"),
        "random" => Some("random"),
        "least_used" | "leastused" => Some("least_used"),
        _ => None,
    }
}

/// Locate the `[[credential_pools]]` entry whose `provider` matches
/// `provider_name`, creating the surrounding `ArrayOfTables` if it does not
/// exist yet. Returns `(array, Some(idx))` on hit and `(array, None)` on miss
/// so the caller can decide whether to append or report an error.
pub(crate) fn pool_lookup_doc_mut<'d>(
    doc: &'d mut toml_edit::DocumentMut,
    provider_name: &str,
) -> (&'d mut toml_edit::ArrayOfTables, Option<usize>) {
    // Insert an empty `[[credential_pools]]` if missing. We use
    // `or_insert(Item::ArrayOfTables(...))` so the rendered output retains
    // the canonical TOML form even when the section was absent in the
    // original file.
    let item = doc
        .entry("credential_pools")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ));
    let arr = match item.as_array_of_tables_mut() {
        Some(a) => a,
        None => {
            ui::error(&i18n::t("auth-pool-config-not-array"));
            std::process::exit(1);
        }
    };
    let idx = arr.iter().position(|t| {
        t.get("provider")
            .and_then(|v| v.as_str())
            .map(|n| n.eq_ignore_ascii_case(provider_name))
            .unwrap_or(false)
    });
    (arr, idx)
}

pub(crate) fn cmd_auth_pool_list(config: Option<PathBuf>, json: bool) {
    // Prefer the running daemon — its snapshot includes live request_count
    // and cooldown telemetry that config.toml alone cannot provide.
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base_url}/api/credential-pools");
        let resp = client.get(&url).send();
        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().unwrap_or_default();
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                    return;
                }
                print_pool_summary_human(&body);
                return;
            }
            Ok(r) => {
                ui::check_warn(&i18n::t_args(
                    "auth-pool-daemon-error-fallback",
                    &[("status", &r.status().to_string())],
                ));
            }
            Err(e) => {
                ui::check_warn(&i18n::t_args(
                    "auth-pool-daemon-connect-fallback",
                    &[("url", &url), ("error", &e.to_string())],
                ));
            }
        }
    }

    // Offline path: render the static config view (no live telemetry).
    let path = pool_config_path(config);
    if !path.exists() {
        if json {
            println!("[]");
        } else {
            ui::check_warn(&i18n::t_args(
                "auth-pool-no-config-offline",
                &[("path", &path.display().to_string())],
            ));
        }
        return;
    }
    let cfg = load_config(Some(&path)).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "auth-pool-config-load-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    let mut pools: Vec<serde_json::Value> = cfg
        .credential_pools
        .iter()
        .map(|p| {
            let strategy = match p.strategy {
                librefang_types::config::CredentialPoolStrategy::FillFirst => "fill_first",
                librefang_types::config::CredentialPoolStrategy::RoundRobin => "round_robin",
                librefang_types::config::CredentialPoolStrategy::Random => "random",
                librefang_types::config::CredentialPoolStrategy::LeastUsed => "least_used",
            };
            let mut keys: Vec<&librefang_types::config::CredentialPoolKeyConfig> =
                p.keys.iter().collect();
            keys.sort_by_key(|k| std::cmp::Reverse(k.priority));
            let creds: Vec<serde_json::Value> = keys
                .iter()
                .map(|k| {
                    let resolved = std::env::var(&k.api_key_env).is_ok();
                    serde_json::json!({
                        "label": k.label,
                        "env_var": k.api_key_env,
                        "priority": k.priority,
                        "env_resolved": resolved,
                    })
                })
                .collect();
            serde_json::json!({
                "provider": p.provider,
                "strategy": strategy,
                "total_count": p.keys.len(),
                "credentials": creds,
            })
        })
        .collect();
    // Deterministic alphabetical ordering (matches the HTTP endpoint).
    pools.sort_by(|a, b| {
        a["provider"]
            .as_str()
            .unwrap_or("")
            .cmp(b["provider"].as_str().unwrap_or(""))
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&pools).unwrap_or_default()
        );
    } else {
        print_pool_summary_human(&serde_json::Value::Array(pools));
    }
}

pub(crate) fn print_pool_summary_human(body: &serde_json::Value) {
    let pools = match body.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => {
            println!("{}", i18n::t("auth-pool-none-configured").dimmed());
            println!();
            println!("Add one with:");
            println!(
                "  librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10"
            );
            return;
        }
    };
    for pool in pools {
        let provider = pool["provider"].as_str().unwrap_or("");
        let strategy = pool["strategy"].as_str().unwrap_or("");
        let total = pool["total_count"].as_u64().unwrap_or(0);
        let available = pool["available_count"].as_u64().unwrap_or(total);
        let header = format!("{provider}  ({strategy})");
        println!("{}", header.bold());
        println!(
            "  keys: {}/{} available",
            available.to_string().bold(),
            total
        );
        if let Some(creds) = pool["credentials"].as_array() {
            for c in creds {
                let label = c["label"].as_str().unwrap_or("");
                let hint = c["key_hint"].as_str().unwrap_or("");
                let env_var = c["env_var"].as_str().unwrap_or("");
                let key_display = if hint.is_empty() { env_var } else { hint };
                let pri = c["priority"].as_u64().unwrap_or(0);
                let reqs = c["request_count"].as_u64();
                let exhausted = c["is_exhausted"].as_bool().unwrap_or(false);
                let env_resolved = c["env_resolved"].as_bool();
                let cooldown = c.get("cooldown_remaining_secs");

                let status: String = if exhausted {
                    if let Some(serde_json::Value::String(s)) = cooldown {
                        if s == "permanent" {
                            "invalid".red().to_string()
                        } else {
                            "exhausted".yellow().to_string()
                        }
                    } else if let Some(serde_json::Value::Number(n)) = cooldown {
                        format!(
                            "{} {}",
                            "cooldown".yellow(),
                            format!("({}s left)", n).dimmed()
                        )
                    } else {
                        "exhausted".yellow().to_string()
                    }
                } else if env_resolved == Some(false) {
                    "env-missing".red().to_string()
                } else {
                    "healthy".green().to_string()
                };

                let reqs_str = reqs.map(|r| format!(" requests={r}")).unwrap_or_default();
                println!(
                    "    - [{label}] {key_display}  priority={pri}{reqs_str}  status={status}"
                );
            }
        }
        println!();
    }
}

/// Best-effort env-var name sanity check used by `auth pool add`. POSIX
/// env-var names are `[A-Z_][A-Z0-9_]*`; reject obvious nonsense (spaces,
/// punctuation, leading digit) at config-time so the operator finds out
/// here instead of seeing "pool has no resolvable keys" from the daemon
/// on next boot.
pub(crate) fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn cmd_auth_pool_add(
    config: Option<PathBuf>,
    provider: &str,
    env_var: &str,
    label: &str,
    priority: u32,
) {
    if !is_valid_env_var_name(env_var) {
        ui::error(&i18n::t_args(
            "auth-pool-invalid-env-name",
            &[("env_var", env_var)],
        ));
        std::process::exit(1);
    }
    // Validate the env var is actually set at add time. Without this the
    // operator can stage a typo into config.toml and only find out at the
    // next daemon boot via a "Credential pool key env var not set — skipping"
    // warning that may go unnoticed. Treat empty/whitespace as unset too —
    // an env var set to "" cannot drive a real provider call.
    match std::env::var(env_var) {
        Ok(v) if !v.trim().is_empty() => {}
        Ok(_) => {
            ui::error_with_fix(
                &i18n::t_args("auth-pool-env-empty", &[("env_var", env_var)]),
                &i18n::t_args("auth-pool-env-empty-fix", &[("env_var", env_var)]),
            );
            std::process::exit(1);
        }
        Err(_) => {
            ui::error_with_fix(
                &i18n::t_args("auth-pool-env-not-set", &[("env_var", env_var)]),
                &i18n::t_args("auth-pool-env-not-set-fix", &[("env_var", env_var)]),
            );
            std::process::exit(1);
        }
    }

    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);

        match idx {
            Some(i) => {
                // Append to existing pool's keys array-of-tables.
                let pool_tbl = arr.get_mut(i).expect("idx within bounds");
                let keys_item = pool_tbl
                    .entry("keys")
                    .or_insert(toml_edit::Item::ArrayOfTables(
                        toml_edit::ArrayOfTables::new(),
                    ));
                let keys_arr = match keys_item.as_array_of_tables_mut() {
                    Some(a) => a,
                    None => {
                        ui::error(&i18n::t_args(
                            "auth-pool-keys-not-array",
                            &[("provider", provider)],
                        ));
                        std::process::exit(1);
                    }
                };
                // Duplicate guard: same env_var on the same provider is an error.
                let dup = keys_arr.iter().any(|k| {
                    k.get("api_key_env")
                        .and_then(|v| v.as_str())
                        .map(|e| e == env_var)
                        .unwrap_or(false)
                });
                if dup {
                    ui::error(&i18n::t_args(
                        "auth-pool-key-duplicate",
                        &[("env_var", env_var), ("provider", provider)],
                    ));
                    std::process::exit(1);
                }
                let mut new_key_tbl = toml_edit::Table::new();
                new_key_tbl["api_key_env"] = toml_edit::value(env_var);
                new_key_tbl["label"] = toml_edit::value(label);
                new_key_tbl["priority"] = toml_edit::value(priority as i64);
                keys_arr.push(new_key_tbl);
            }
            None => {
                // Create the pool with default strategy = fill_first.
                let mut pool_tbl = toml_edit::Table::new();
                pool_tbl["provider"] = toml_edit::value(provider);
                pool_tbl["strategy"] = toml_edit::value("fill_first");
                let mut keys_arr = toml_edit::ArrayOfTables::new();
                let mut new_key_tbl = toml_edit::Table::new();
                new_key_tbl["api_key_env"] = toml_edit::value(env_var);
                new_key_tbl["label"] = toml_edit::value(label);
                new_key_tbl["priority"] = toml_edit::value(priority as i64);
                keys_arr.push(new_key_tbl);
                pool_tbl.insert("keys", toml_edit::Item::ArrayOfTables(keys_arr));
                arr.push(pool_tbl);
            }
        }
    }

    pool_write_doc_or_exit(&path, &doc);
    ui::success(&i18n::t_args(
        "auth-pool-key-added",
        &[
            ("label", label),
            ("env_var", env_var),
            ("priority", &priority.to_string()),
            ("provider", provider),
        ],
    ));
}

pub(crate) fn cmd_auth_pool_remove(config: Option<PathBuf>, provider: &str, env_var: &str) {
    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    let mut empty_pool_removed = false;
    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);
        let Some(i) = idx else {
            ui::error(&i18n::t_args(
                "auth-pool-not-configured",
                &[("provider", provider)],
            ));
            std::process::exit(1);
        };

        let pool_tbl = arr.get_mut(i).expect("idx within bounds");
        let Some(keys_item) = pool_tbl.get_mut("keys") else {
            ui::error(&i18n::t_args(
                "auth-pool-no-keys-field",
                &[("provider", provider)],
            ));
            std::process::exit(1);
        };
        let Some(keys_arr) = keys_item.as_array_of_tables_mut() else {
            ui::error(&i18n::t_args(
                "auth-pool-keys-not-array",
                &[("provider", provider)],
            ));
            std::process::exit(1);
        };
        let before = keys_arr.len();
        // ArrayOfTables has no `retain` — walk indices backwards and remove
        // matching entries one by one so index shifts don't skip neighbors.
        for j in (0..keys_arr.len()).rev() {
            let matches = keys_arr
                .get(j)
                .and_then(|t| t.get("api_key_env"))
                .and_then(|v| v.as_str())
                .map(|e| e == env_var)
                .unwrap_or(false);
            if matches {
                keys_arr.remove(j);
            }
        }
        if keys_arr.len() == before {
            ui::error(&i18n::t_args(
                "auth-pool-key-not-found",
                &[("env_var", env_var), ("provider", provider)],
            ));
            std::process::exit(1);
        }
        if keys_arr.is_empty() {
            arr.remove(i);
            empty_pool_removed = true;
        }
    }

    pool_write_doc_or_exit(&path, &doc);
    if empty_pool_removed {
        ui::success(&i18n::t_args(
            "auth-pool-key-removed-pool-empty",
            &[("env_var", env_var), ("provider", provider)],
        ));
    } else {
        ui::success(&i18n::t_args(
            "auth-pool-key-removed",
            &[("env_var", env_var), ("provider", provider)],
        ));
    }
}

pub(crate) fn cmd_auth_pool_strategy(config: Option<PathBuf>, provider: &str, strategy: &str) {
    let Some(canon) = pool_strategy_canon(strategy) else {
        ui::error(&i18n::t_args(
            "auth-pool-unknown-strategy",
            &[("strategy", strategy)],
        ));
        std::process::exit(1);
    };

    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);
        let Some(i) = idx else {
            ui::error(&i18n::t_args(
                "auth-pool-not-configured",
                &[("provider", provider)],
            ));
            std::process::exit(1);
        };
        let pool_tbl = arr.get_mut(i).expect("idx within bounds");
        pool_tbl["strategy"] = toml_edit::value(canon);
    }

    pool_write_doc_or_exit(&path, &doc);
    ui::success(&i18n::t_args(
        "auth-pool-strategy-set",
        &[("provider", provider), ("strategy", canon)],
    ));
}

// ---------------------------------------------------------------------------
// Vault commands (librefang vault init/set/list/remove)
// ---------------------------------------------------------------------------

pub(crate) fn cmd_vault_init() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    match vault.init() {
        Ok(()) => ui::success(&i18n::t("vault-initialized")),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_vault_set(key: &str) {
    use zeroize::Zeroizing;

    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-init-run"));
        std::process::exit(1);
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let value = prompt_input(&format!("Enter value for {key}: "));
    if value.is_empty() {
        ui::error(&i18n::t("vault-empty-value"));
        std::process::exit(1);
    }

    match vault.set(key.to_string(), Zeroizing::new(value)) {
        Ok(()) => ui::success(&i18n::t_args("vault-stored", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-store-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_vault_list() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        println!("{}", i18n::t("vault-not-init-run"));
        return;
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let keys = vault.list_keys();
    if keys.is_empty() {
        println!("{}", i18n::t("vault-empty"));
    } else {
        println!(
            "{}",
            i18n::t_args("vault-stored-count", &[("count", &keys.len().to_string())])
        );
        for key in keys {
            println!("  {key}");
        }
    }
}

pub(crate) fn cmd_vault_remove(key: &str) {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-initialized"));
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    match vault.remove(key) {
        Ok(true) => ui::success(&i18n::t_args("vault-removed", &[("key", key)])),
        Ok(false) => println!("{}", i18n::t_args("vault-key-not-found", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-remove-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

/// Rotate the vault master key by re-encrypting every entry under a fresh
/// 32-byte key. Issue #3651.
///
/// Source of the keys (in order):
///   - OLD: env var `LIBREFANG_VAULT_KEY_OLD` (REQUIRED)
///   - NEW: env var `LIBREFANG_VAULT_KEY_NEW` unless `--from-stdin` is set,
///     in which case stdin is read until EOF and trimmed.
///
/// Both must be base64 of exactly 32 raw bytes (`openssl rand -base64 32`,
/// matches `LIBREFANG_VAULT_KEY` in production). Any other length is
/// rejected up-front before any vault state is touched.
///
/// On success the vault file is atomically replaced (vault.rs's `save()`
/// already writes to `<path>.tmp` and `rename`s — re-using it gives us the
/// atomic-swap-on-disk guarantee for free) and prints the new key fingerprint
/// so the operator has a non-secret confirmation that the rotation took.
pub(crate) fn cmd_vault_rotate_key(from_stdin: bool) {
    use std::io::Read as _;
    use zeroize::Zeroizing;

    let home = librefang_home();
    let vault_path = home.join("vault.enc");

    // Pre-flight: vault must already exist. Refuse on missing file rather
    // than silently `init()` — rotating a vault that was never created is
    // a no-op masking an operator error.
    if !vault_path.exists() {
        ui::error(&i18n::t("vault-rotate-no-vault"));
        std::process::exit(1);
    }

    // Read OLD key from env. Always required.
    let old_key_b64 = match std::env::var("LIBREFANG_VAULT_KEY_OLD") {
        Ok(s) if !s.is_empty() => Zeroizing::new(s),
        _ => {
            ui::error(&i18n::t("vault-rotate-old-key-missing"));
            std::process::exit(1);
        }
    };

    // Read NEW key from stdin or env, depending on the flag. stdin wins
    // when `--from-stdin` is set so a key in env can't accidentally
    // override an explicit stdin pipe.
    let new_key_b64 = if from_stdin {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            ui::error(&i18n::t_args(
                "vault-rotate-stdin-read-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
        let trimmed = buf.trim().to_string();
        if trimmed.is_empty() {
            ui::error(&i18n::t("vault-rotate-stdin-empty"));
            std::process::exit(1);
        }
        Zeroizing::new(trimmed)
    } else {
        match std::env::var("LIBREFANG_VAULT_KEY_NEW") {
            Ok(s) if !s.is_empty() => Zeroizing::new(s),
            _ => {
                ui::error(&i18n::t("vault-rotate-new-key-missing"));
                std::process::exit(1);
            }
        }
    };

    // Reject identical OLD/NEW up-front — silently no-op rotations are a
    // footgun. (`Zeroizing<String>` derefs to `&str` so direct comparison
    // is safe and constant-time on equal-length strings is unnecessary
    // here: this is a configuration check, not a credential check.)
    if old_key_b64.as_str() == new_key_b64.as_str() {
        ui::error(&i18n::t("vault-rotate-same-key"));
        std::process::exit(1);
    }

    // Decode both keys via the same parser the production daemon uses so
    // any rejection here matches what the daemon will reject at boot.
    let old_key_bytes = match librefang_extensions::vault::decode_master_key(&old_key_b64) {
        Ok(k) => k,
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-rotate-old-key-invalid",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };
    let new_key_bytes = match librefang_extensions::vault::decode_master_key(&new_key_b64) {
        Ok(k) => k,
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-rotate-new-key-invalid",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };

    // Open + unlock with OLD key. Use `unlock_with_key` so the rotation
    // doesn't accidentally pick up a stale env / keyring value — we want
    // the rotation to fail loudly if `LIBREFANG_VAULT_KEY_OLD` doesn't
    // match the on-disk vault.
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path.clone());
    if let Err(e) = vault.unlock_with_key(old_key_bytes) {
        ui::error(&i18n::t_args(
            "vault-rotate-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    // Verify (or backfill) the sentinel under the OLD key BEFORE rotating.
    // This catches "OLD key decrypted noise" and ensures legacy vaults
    // gain a sentinel during rotation rather than after.
    if let Err(e) = vault.verify_or_install_sentinel() {
        ui::error(&i18n::t_args(
            "vault-rotate-sentinel-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let entry_count = vault.list_keys().len();

    // Re-encrypt the entire vault under the NEW key. `rewrap_with_new_key`
    // re-uses the proven atomic save path inside vault.rs (write to
    // `<path>.tmp`, fsync, rename) — no separate code path to maintain.
    if let Err(e) = vault.rewrap_with_new_key(new_key_bytes) {
        ui::error(&i18n::t_args(
            "vault-rotate-rewrap-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    ui::success(&i18n::t_args(
        "vault-rotate-success",
        &[("count", &entry_count.to_string())],
    ));
    println!("{}", i18n::t("vault-rotate-next-step"));
}

// ---------------------------------------------------------------------------
// hash-password command
// ---------------------------------------------------------------------------

pub(crate) fn cmd_hash_password(password: Option<String>) {
    let pass = match password {
        Some(p) => p,
        None => {
            let p1 = prompt_input(&i18n::t("auth-enter-password-prompt"));
            if p1.is_empty() {
                ui::error(&i18n::t("auth-password-empty"));
                std::process::exit(1);
            }
            let p2 = prompt_input(&i18n::t("auth-confirm-password-prompt"));
            if p1 != p2 {
                ui::error(&i18n::t("auth-passwords-mismatch"));
                std::process::exit(1);
            }
            p1
        }
    };

    match librefang_api::password_hash::hash_password(&pass) {
        Ok(hash) => {
            println!("\n{hash}\n");
            println!("Add to config.toml:");
            println!("  dashboard_pass_hash = \"{hash}\"");
        }
        Err(e) => {
            ui::error(&i18n::t_args(
                "auth-password-hash-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}
