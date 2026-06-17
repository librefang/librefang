//! `doctor_cmd` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

pub(crate) fn cmd_doctor(json: bool, repair: bool) {
    // BrokenPipe protection for the WHOLE command, not just the --json
    // branch. `librefang doctor | head -5` and similar pipelines drop the
    // reader after a few lines, which on the next stdout write turns into a
    // panic — Rust ignores SIGPIPE by default and translates EPIPE into an
    // io::Error that `println!` unwraps.
    //
    // The pre-existing `write_stdout_safe` helper only covered the
    // `--json` final emission. Hundreds of `ui::*` and bare `println!`
    // calls between the start of cmd_doctor and that emission were still
    // unprotected. Restoring the default SIGPIPE handler for the duration
    // of this command makes the kernel terminate the process cleanly on
    // pipe close instead, covering every print path in this function and
    // the `ui::*` helpers it calls.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let mut checks: Vec<serde_json::Value> = Vec::new();
    let mut all_ok = true;
    let mut repaired = false;

    if !json {
        ui::step(&i18n::t("doctor-title"));
        println!();
    }

    let home = dirs::home_dir();
    if let Some(_h) = &home {
        let librefang_dir = cli_librefang_home();

        // --- Check 1: LibreFang directory ---
        if librefang_dir.exists() {
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-librefang-dir-ok",
                    &[("path", &librefang_dir.display().to_string())],
                ));
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "ok", "path": librefang_dir.display().to_string()}));
        } else if repair {
            if !json {
                ui::check_fail(&i18n::t("doctor-check-librefang-dir-fail"));
            }
            let answer = prompt_input(&i18n::t("doctor-prompt-create-dir"));
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                if std::fs::create_dir_all(&librefang_dir).is_ok() {
                    restrict_dir_permissions(&librefang_dir);
                    let _ = std::fs::create_dir_all(librefang_dir.join("data"));
                    let _ =
                        std::fs::create_dir_all(librefang_dir.join("workspaces").join("agents"));
                    if !json {
                        ui::check_ok(&i18n::t("doctor-check-librefang-dir-created"));
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail(&i18n::t("doctor-check-librefang-dir-create-fail"));
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail(&i18n::t("doctor-check-librefang-dir-not-found-init"));
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "fail"}));
            all_ok = false;
        }

        // --- Check 2: .env file exists + permissions ---
        let env_path = librefang_dir.join(".env");
        if env_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&env_path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode == 0o600 {
                        if !json {
                            ui::check_ok(&i18n::t("doctor-check-env-ok"));
                        }
                    } else if repair {
                        let _ = std::fs::set_permissions(
                            &env_path,
                            std::fs::Permissions::from_mode(0o600),
                        );
                        if !json {
                            ui::check_ok(&i18n::t("doctor-check-env-fixed"));
                        }
                        repaired = true;
                    } else if !json {
                        ui::check_warn(&i18n::t_args(
                            "doctor-check-env-loose-warn",
                            &[("mode", &format!("{:o}", mode))],
                        ));
                    }
                } else if !json {
                    ui::check_ok(&i18n::t("doctor-check-env-ok-generic"));
                }
            }
            #[cfg(not(unix))]
            {
                if !json {
                    ui::check_ok(&i18n::t("doctor-check-env-ok-generic"));
                }
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "ok"}));
        } else {
            if !json {
                ui::check_warn(&i18n::t("doctor-check-env-not-found-warn"));
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "warn"}));
        }

        // --- Check 3: Config TOML syntax validation ---
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<toml::Value>(&config_content) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&i18n::t_args(
                            "doctor-check-config-ok",
                            &[("path", &config_path.display().to_string())],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "config_file", "status": "ok"}));
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&i18n::t_args(
                            "doctor-check-config-syntax-fail",
                            &[("error", &e.to_string())],
                        ));
                        ui::hint(&i18n::t("hint-config-edit"));
                    }
                    checks.push(serde_json::json!({"check": "config_syntax", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        } else if repair {
            if !json {
                ui::check_fail(&i18n::t("doctor-check-config-not-found"));
            }
            let answer = prompt_input(&i18n::t("doctor-prompt-create-config"));
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                let (provider, api_key_env, model) = detect_best_provider();
                let default_config = render_init_default_config(&provider, &model, &api_key_env);
                let _ = std::fs::create_dir_all(&librefang_dir);
                if std::fs::write(&config_path, default_config).is_ok() {
                    restrict_file_permissions(&config_path);
                    if !json {
                        ui::check_ok(&i18n::t("doctor-check-config-created"));
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail(&i18n::t("doctor-check-config-create-fail"));
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "config_file", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail(&i18n::t("doctor-check-config-not-found"));
            }
            checks.push(serde_json::json!({"check": "config_file", "status": "fail"}));
            all_ok = false;
        }

        // --- Check: Version update ---
        {
            let current_version = env!("CARGO_PKG_VERSION");
            let update_channel = load_update_channel_from_config().unwrap_or_default();
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-cli-version",
                    &[
                        ("version", current_version),
                        ("channel", &update_channel.to_string()),
                    ],
                ));
            }
            checks.push(serde_json::json!({"check": "cli_version", "status": "ok", "version": current_version, "channel": update_channel.to_string()}));

            // Try to fetch latest release for the configured channel (best-effort)
            match fetch_latest_release_tag(update_channel) {
                Ok(tag) => {
                    let latest = tag.strip_prefix('v').unwrap_or(&tag);
                    if latest != current_version {
                        if !json {
                            ui::check_warn(&i18n::t_args(
                                "doctor-check-update-available-warn",
                                &[("current", current_version), ("latest", latest)],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "warn", "current": current_version, "latest": latest}));
                    } else {
                        if !json {
                            ui::check_ok(&i18n::t("doctor-check-cli-up-to-date"));
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "ok"}));
                    }
                }
                Err(_) => {
                    if !json {
                        ui::check_warn(&i18n::t("doctor-check-update-fail-warn"));
                    }
                    checks.push(serde_json::json!({"check": "version_update", "status": "warn", "reason": "network_error"}));
                }
            }
        }

        // --- Check 4: Port availability ---
        // Read api_listen from config (default: 127.0.0.1:4545)
        let api_listen = {
            let cfg_path = librefang_dir.join("config.toml");
            if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|s| toml::from_str::<librefang_types::config::KernelConfig>(&s).ok())
                    .map(|c| c.api_listen)
                    .unwrap_or_else(|| librefang_types::config::DEFAULT_API_LISTEN.to_string())
            } else {
                librefang_types::config::DEFAULT_API_LISTEN.to_string()
            }
        };
        if !json {
            println!();
        }
        let daemon_running = find_daemon();
        if let Some(ref base) = daemon_running {
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-daemon-running",
                    &[("url", base)],
                ));
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "ok", "url": base}));
        } else {
            if !json {
                ui::check_warn(&i18n::t("doctor-check-daemon-not-running-warn"));
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "warn"}));

            // Check if the configured port is available
            let bind_addr = if api_listen.starts_with("0.0.0.0") {
                api_listen.replacen("0.0.0.0", "127.0.0.1", 1)
            } else {
                api_listen.clone()
            };
            match std::net::TcpListener::bind(&bind_addr) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&i18n::t_args(
                            "doctor-check-port-available",
                            &[("address", &api_listen)],
                        ));
                    }
                    checks.push(
                        serde_json::json!({"check": "port", "status": "ok", "address": api_listen}),
                    );
                }
                Err(_) => {
                    if !json {
                        ui::check_warn(&i18n::t_args(
                            "doctor-check-port-in-use-warn",
                            &[("address", &api_listen)],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "port", "status": "warn", "address": api_listen}));
                }
            }
        }

        // --- Check 5: Stale daemon.json ---
        let daemon_json_path = librefang_dir.join("daemon.json");
        if daemon_json_path.exists() && daemon_running.is_none() {
            if repair {
                let _ = std::fs::remove_file(&daemon_json_path);
                if !json {
                    ui::check_ok(&i18n::t("doctor-check-stale-daemon-json-removed"));
                }
                repaired = true;
            } else if !json {
                ui::check_warn(&i18n::t("doctor-check-stale-daemon-json-warn"));
            }
            checks.push(serde_json::json!({"check": "stale_daemon_json", "status": if repair { "repaired" } else { "warn" }}));
        }

        // --- Check 6: Database file ---
        let db_path = librefang_dir.join("data").join("librefang.db");
        if db_path.exists() {
            // Quick SQLite magic bytes check
            if let Ok(bytes) = std::fs::read(&db_path) {
                if bytes.len() >= 16 && bytes.starts_with(b"SQLite format 3") {
                    if !json {
                        ui::check_ok(&i18n::t("doctor-check-db-ok"));
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "ok"}));
                } else {
                    if !json {
                        ui::check_fail(&i18n::t("doctor-check-db-invalid-fail"));
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "fail"}));
                    all_ok = false;
                }
            }
        } else {
            if !json {
                ui::check_warn(&i18n::t("doctor-check-db-not-found-warn"));
            }
            checks.push(serde_json::json!({"check": "database", "status": "warn"}));
        }

        // --- Check 7: Disk space ---
        #[cfg(unix)]
        {
            if let Ok(output) = std::process::Command::new("df")
                .args(["-m", &librefang_dir.display().to_string()])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Parse the available MB from df output (4th column of 2nd line)
                if let Some(line) = stdout.lines().nth(1) {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() >= 4 {
                        if let Ok(available_mb) = cols[3].parse::<u64>() {
                            if available_mb < 100 {
                                if !json {
                                    ui::check_warn(&i18n::t_args(
                                        "doctor-check-disk-space-low-warn",
                                        &[("count", &available_mb.to_string())],
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "warn", "available_mb": available_mb}));
                            } else {
                                if !json {
                                    ui::check_ok(&i18n::t_args(
                                        "doctor-check-disk-space-ok",
                                        &[("count", &available_mb.to_string())],
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "ok", "available_mb": available_mb}));
                            }
                        }
                    }
                }
            }
        }

        // --- Check 8: Agent manifests parse correctly ---
        let agents_dir = librefang_dir.join("workspaces").join("agents");
        if agents_dir.exists() {
            let mut agent_errors = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&agents_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Err(e) = toml::from_str::<AgentManifest>(&content) {
                                agent_errors.push((
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    e.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            if agent_errors.is_empty() {
                if !json {
                    ui::check_ok(&i18n::t("doctor-check-manifests-ok"));
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "ok"}));
            } else {
                for (file, err) in &agent_errors {
                    if !json {
                        ui::check_fail(&i18n::t_args(
                            "doctor-check-manifest-invalid-fail",
                            &[("file", file), ("error", err)],
                        ));
                    }
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "fail", "errors": agent_errors.len()}));
                all_ok = false;
            }
        }
    } else {
        if !json {
            ui::check_fail(&i18n::t("doctor-check-home-dir-fail"));
        }
        checks.push(serde_json::json!({"check": "home_dir", "status": "fail"}));
        all_ok = false;
    }

    // --- LLM providers ---
    if !json {
        println!("{}", i18n::t("doctor-section-providers"));
    }
    // Pretty display names for known provider IDs. Anything not listed
    // here falls back to a Title-Case derivation of the raw provider id
    // (e.g. `xiaomi` → `Xiaomi`). Adding a new provider to
    // `PROVIDER_REGISTRY` automatically picks up the fallback so the
    // check loop never silently misses a key — only the cosmetic name
    // needs editing here, not the list of providers checked.
    fn display_name(provider_id: &str) -> String {
        match provider_id {
            "openai" => "OpenAI".to_string(),
            "openrouter" => "OpenRouter".to_string(),
            "deepseek" => "DeepSeek".to_string(),
            "deepinfra" => "DeepInfra".to_string(),
            "byteplus" => "BytePlus".to_string(),
            "azure-openai" => "Azure OpenAI".to_string(),
            "github-copilot" => "GitHub Copilot".to_string(),
            "huggingface" => "Hugging Face".to_string(),
            "openai-codex" => "OpenAI Codex".to_string(),
            "claude-code" => "Claude Code".to_string(),
            "vertex-ai" => "Vertex AI".to_string(),
            "nvidia-nim" => "NVIDIA NIM".to_string(),
            "z.ai" | "zai" => "Z.ai".to_string(),
            "kimi-coding" | "kimi_coding" => "Kimi Coding".to_string(),
            "alibaba-coding-plan" => "Alibaba Coding Plan".to_string(),
            other => {
                // Title-case fallback for unlisted providers so `xiaomi` →
                // `Xiaomi` instead of leaking the raw lowercase id.
                let mut chars = other.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        }
    }

    // Drive doctor off PROVIDER_REGISTRY so adding a provider to the
    // driver layer never requires a parallel edit here. `GOOGLE_API_KEY`
    // (gemini's alt env) and similar aliases come through automatically.
    // This subsumes the previous hardcoded array (including the byteplus
    // entry from #3274 — now provided automatically by the registry).
    let provider_specs = librefang_runtime::drivers::cloud_provider_key_specs();
    let provider_keys: Vec<(&str, String, &str)> = provider_specs
        .iter()
        .map(|(env_var, provider_id)| (*env_var, display_name(provider_id), *provider_id))
        .collect();

    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // --- Check 9: Live key validation ---
            let valid = test_api_key(provider_id, &std::env::var(env_var).unwrap_or_default());
            if valid {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&i18n::t_args(
                    "doctor-check-provider-key-rejected-warn",
                    &[("name", name), ("env_var", env_var)],
                ));
            }
            any_key_set = true;
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": !valid}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    if !any_key_set {
        if !json {
            println!();
            ui::check_fail(&i18n::t("doctor-no-api-keys"));
            ui::blank();
            ui::section(&i18n::t("section-getting-api-key"));
            ui::suggest_cmd("Groq:", &i18n::t("doctor-suggest-groq"));
            ui::suggest_cmd("Gemini:", &i18n::t("doctor-suggest-gemini"));
            ui::suggest_cmd("DeepSeek:", &i18n::t("doctor-suggest-deepseek"));
            ui::blank();
            ui::hint(&i18n::t("hint-set-key"));
        }
        all_ok = false;
    }

    // --- Check: Network connectivity to configured LLM provider endpoints ---
    {
        let provider_endpoints: &[(&str, &str, &str)] = &[
            ("OPENAI_API_KEY", "OpenAI", "api.openai.com:443"),
            ("ANTHROPIC_API_KEY", "Anthropic", "api.anthropic.com:443"),
            ("GROQ_API_KEY", "Groq", "api.groq.com:443"),
            ("DEEPSEEK_API_KEY", "DeepSeek", "api.deepseek.com:443"),
            (
                "GEMINI_API_KEY",
                "Gemini",
                "generativelanguage.googleapis.com:443",
            ),
            (
                "GOOGLE_API_KEY",
                "Google",
                "generativelanguage.googleapis.com:443",
            ),
            ("OPENROUTER_API_KEY", "OpenRouter", "openrouter.ai:443"),
            ("TOGETHER_API_KEY", "Together", "api.together.xyz:443"),
            ("MISTRAL_API_KEY", "Mistral", "api.mistral.ai:443"),
            ("FIREWORKS_API_KEY", "Fireworks", "api.fireworks.ai:443"),
        ];

        let configured: Vec<_> = provider_endpoints
            .iter()
            .filter(|(env_var, _, _)| std::env::var(env_var).is_ok())
            .collect();

        if !configured.is_empty() {
            if !json {
                println!("{}", i18n::t("doctor-section-connectivity"));
            }
            for (env_var, name, endpoint) in &configured {
                use std::net::{TcpStream, ToSocketAddrs};
                let reachable = endpoint
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addrs| addrs.next())
                    .map(|addr| {
                        TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)).is_ok()
                    })
                    .unwrap_or(false);

                if reachable {
                    if !json {
                        ui::check_ok(&i18n::t_args(
                            "doctor-check-endpoint-reachable",
                            &[("name", name), ("endpoint", endpoint)],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "ok"}));
                } else {
                    if !json {
                        ui::check_warn(&i18n::t_args(
                            "doctor-check-endpoint-unreachable-warn",
                            &[("name", name), ("endpoint", endpoint)],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "warn"}));
                }
            }
        }
    }

    // --- Check 10: Channel token format validation ---
    if !json {
        println!("{}", i18n::t("doctor-section-channels"));
    }
    let channel_keys = [
        ("TELEGRAM_BOT_TOKEN", "Telegram"),
        ("DISCORD_BOT_TOKEN", "Discord"),
        ("SLACK_APP_TOKEN", "Slack App"),
        ("SLACK_BOT_TOKEN", "Slack Bot"),
    ];
    for (env_var, name) in &channel_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // Format validation
            let val = std::env::var(env_var).unwrap_or_default();
            let format_ok = match *env_var {
                "TELEGRAM_BOT_TOKEN" => val.contains(':'), // Telegram tokens have format "123456:ABC-DEF..."
                "DISCORD_BOT_TOKEN" => val.len() > 50,     // Discord tokens are typically 59+ chars
                "SLACK_APP_TOKEN" => val.starts_with("xapp-"),
                "SLACK_BOT_TOKEN" => val.starts_with("xoxb-"),
                _ => true,
            };
            if format_ok {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&i18n::t_args(
                    "doctor-check-channel-token-format-warn",
                    &[("name", name), ("env_var", env_var)],
                ));
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": if format_ok { "ok" } else { "warn" }}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    // --- Check 11: .env keys vs config api_key_env consistency ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
            // Look for api_key_env references in config
            for line in config_str.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("api_key_env") {
                    if let Some(val_part) = rest.strip_prefix('=') {
                        let val = val_part.trim().trim_matches('"');
                        if !val.is_empty() && std::env::var(val).is_err() {
                            if !json {
                                ui::check_warn(&i18n::t_args(
                                    "doctor-check-config-env-missing-warn",
                                    &[("env_var", val)],
                                ));
                            }
                            checks.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
                        }
                    }
                }
            }
        }
    }

    // --- Check 12: Config deserialization into KernelConfig ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            if !json {
                println!("{}", i18n::t("doctor-section-config-val"));
            }
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<librefang_types::config::KernelConfig>(&config_content) {
                Ok(cfg) => {
                    if !json {
                        ui::check_ok(&i18n::t("doctor-check-config-deser-ok"));
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "ok"}));

                    // Check exec policy
                    let mode = format!("{:?}", cfg.exec_policy.mode);
                    let safe_bins_count = cfg.exec_policy.safe_bins.len();
                    if !json {
                        ui::check_ok(&i18n::t_args(
                            "doctor-check-exec-policy",
                            &[("mode", &mode), ("count", &safe_bins_count.to_string())],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "exec_policy", "status": "ok", "mode": mode, "safe_bins": safe_bins_count}));

                    // Check includes
                    if !cfg.include.is_empty() {
                        let mut include_ok = true;
                        for inc in &cfg.include {
                            let inc_path = librefang_dir.join(inc);
                            if inc_path.exists() {
                                if !json {
                                    ui::check_ok(&i18n::t_args(
                                        "doctor-check-include-file-ok",
                                        &[("path", inc)],
                                    ));
                                }
                            } else if repair {
                                if !json {
                                    ui::check_warn(&i18n::t_args(
                                        "doctor-check-include-file-missing-warn",
                                        &[("path", inc)],
                                    ));
                                }
                                include_ok = false;
                            } else {
                                if !json {
                                    ui::check_fail(&i18n::t_args(
                                        "doctor-check-include-file-not-found-fail",
                                        &[("path", inc)],
                                    ));
                                }
                                include_ok = false;
                                all_ok = false;
                            }
                        }
                        checks.push(serde_json::json!({"check": "config_includes", "status": if include_ok { "ok" } else { "fail" }, "count": cfg.include.len()}));
                    }

                    // Check MCP server configs
                    if !cfg.mcp_servers.is_empty() {
                        let mcp_count = cfg.mcp_servers.len();
                        if !json {
                            ui::check_ok(&i18n::t_args(
                                "doctor-check-mcp-servers-count",
                                &[("count", &mcp_count.to_string())],
                            ));
                        }
                        for server in &cfg.mcp_servers {
                            // Validate transport config
                            let Some(ref transport) = server.transport else {
                                continue;
                            };
                            match transport {
                                librefang_types::config::McpTransportEntry::Stdio {
                                    command,
                                    ..
                                } => {
                                    if command.is_empty() {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-empty-command-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::Sse { url }
                                | librefang_types::config::McpTransportEntry::Http { url } => {
                                    if url.is_empty() {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-empty-url-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::HttpCompat {
                                    base_url,
                                    headers,
                                    tools,
                                } => {
                                    if base_url.is_empty() {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-empty-base-url-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.is_empty() {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-no-compat-tools-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| h.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-compat-header-empty-name-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| {
                                        h.value.as_ref().is_none_or(|value| value.trim().is_empty())
                                            && h.value_env
                                                .as_ref()
                                                .is_none_or(|value| value.trim().is_empty())
                                    }) {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-compat-header-no-value-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-compat-tool-empty-name-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.path.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&i18n::t_args(
                                                "doctor-check-mcp-compat-tool-empty-path-warn",
                                                &[("name", &server.name)],
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                            }
                        }
                        checks.push(serde_json::json!({"check": "mcp_servers", "status": "ok", "count": mcp_count}));
                    }
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&i18n::t_args(
                            "doctor-check-config-deser-fail",
                            &[("error", &e.to_string())],
                        ));
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        }
    }

    // --- Check 13: Skill registry health ---
    {
        if !json {
            println!("{}", i18n::t("doctor-section-skills"));
        }
        let skills_dir = cli_librefang_home().join("skills");
        let mut skill_reg = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
        match skill_reg.load_all() {
            Ok(count) => {
                if !json {
                    ui::check_ok(&i18n::t_args(
                        "doctor-check-skills-loaded",
                        &[("count", &count.to_string())],
                    ));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "ok", "count": count}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&i18n::t_args(
                        "doctor-check-skills-load-fail-warn",
                        &[("error", &e.to_string())],
                    ));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check for prompt injection issues in skill definitions.
        // Only flag Critical-severity warnings.
        let skills = skill_reg.list();
        let mut injection_warnings = 0;
        for skill in &skills {
            if let Some(ref prompt) = skill.manifest.prompt_context {
                let warnings = librefang_skills::verify::SkillVerifier::scan_prompt_content(prompt);
                let has_critical = warnings.iter().any(|w| {
                    matches!(
                        w.severity,
                        librefang_skills::verify::WarningSeverity::Critical
                    )
                });
                if has_critical {
                    injection_warnings += 1;
                    if !json {
                        ui::check_warn(&i18n::t_args(
                            "doctor-check-skills-injection-warn",
                            &[("name", &skill.manifest.skill.name)],
                        ));
                    }
                }
            }
        }
        if injection_warnings > 0 {
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "warn", "warnings": injection_warnings}));
        } else {
            if !json {
                ui::check_ok(&i18n::t("doctor-check-skills-injection-ok"));
            }
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "ok"}));
        }
    }

    // --- Check 14: MCP catalog + configured servers ---
    {
        if !json {
            println!("\n{}", i18n::t("doctor-section-mcp-servers"));
        }
        let librefang_dir = cli_librefang_home();
        let mut catalog = librefang_extensions::catalog::McpCatalog::new(&librefang_dir);
        catalog.load(&librefang_runtime::registry_sync::resolve_home_dir_for_tests());
        let template_count = catalog.len();

        // Count configured [[mcp_servers]] entries in config.toml (if any).
        let configured_count = {
            let config_path = librefang_dir.join("config.toml");
            if config_path.is_file() {
                let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
                toml::from_str::<toml::Value>(&raw)
                    .ok()
                    .and_then(|v| v.as_table().cloned())
                    .and_then(|t| t.get("mcp_servers").cloned())
                    .and_then(|v| v.as_array().cloned())
                    .map(|a| a.len())
                    .unwrap_or(0)
            } else {
                0
            }
        };
        if !json {
            ui::check_ok(&i18n::t_args(
                "doctor-check-mcp-catalog-templates",
                &[("templates", &template_count.to_string())],
            ));
            ui::check_ok(&i18n::t_args(
                "doctor-check-mcp-configured-servers",
                &[("configured", &configured_count.to_string())],
            ));
        }
        checks.push(
            serde_json::json!({"check": "mcp_catalog", "status": "ok", "count": template_count}),
        );
        checks.push(serde_json::json!({"check": "mcp_servers_configured", "status": "ok", "count": configured_count}));
    }

    // --- Check 15: Daemon health detail (if running) ---
    if let Some(ref base) = find_daemon() {
        if !json {
            println!("\n{}", i18n::t("doctor-section-daemon-health"));
        }
        let client = daemon_client();
        match client.get(format!("{base}/api/health/detail")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
                        if !json {
                            ui::check_ok(&i18n::t_args(
                                "doctor-check-running-agents",
                                &[("count", &agents.to_string())],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_agents", "status": "ok", "count": agents}));
                    }
                    if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                        let hours = uptime / 3600;
                        let mins = (uptime % 3600) / 60;
                        if !json {
                            ui::check_ok(&i18n::t_args(
                                "doctor-check-daemon-uptime",
                                &[("hours", &hours.to_string()), ("mins", &mins.to_string())],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_uptime", "status": "ok", "secs": uptime}));
                    }
                    if let Some(db_status) = body.get("database").and_then(|v| v.as_str()) {
                        if db_status == "connected" || db_status == "ok" {
                            if !json {
                                ui::check_ok(&i18n::t("doctor-check-db-connectivity-ok"));
                            }
                        } else {
                            if !json {
                                ui::check_fail(&i18n::t_args(
                                    "doctor-check-db-status-fail",
                                    &[("status", db_status)],
                                ));
                            }
                            all_ok = false;
                        }
                        checks.push(serde_json::json!({"check": "daemon_db", "status": db_status}));
                    }
                }
            }
            Ok(resp) => {
                if !json {
                    ui::check_warn(&i18n::t_args(
                        "doctor-check-health-detail-status-warn",
                        &[("status", &resp.status().to_string())],
                    ));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn"}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&i18n::t_args(
                        "doctor-check-health-detail-fail-warn",
                        &[("error", &e.to_string())],
                    ));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check skills endpoint
        match client.get(format!("{base}/api/skills")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("skills")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        if !json {
                            ui::check_ok(&i18n::t_args(
                                "doctor-check-skills-loaded-daemon",
                                &[("count", &arr.len().to_string())],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_skills", "status": "ok", "count": arr.len()}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP servers endpoint
        match client.get(format!("{base}/api/mcp/servers")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("configured")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        let connected = arr
                            .iter()
                            .filter(|s| {
                                s.get("connected")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                            })
                            .count();
                        if !json {
                            ui::check_ok(&i18n::t_args(
                                "doctor-check-daemon-mcp-status",
                                &[
                                    ("configured", &arr.len().to_string()),
                                    ("connected", &connected.to_string()),
                                ],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_mcp", "status": "ok", "configured": arr.len(), "connected": connected}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP health endpoint
        match client.get(format!("{base}/api/mcp/health")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries = body.get("health").and_then(|h| h.as_array());
                    if let Some(arr) = entries {
                        let healthy = arr
                            .iter()
                            .filter(|v| {
                                v.get("status")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s.eq_ignore_ascii_case("ready"))
                                    .unwrap_or(false)
                            })
                            .count();
                        let total = arr.len();
                        if healthy == total {
                            if !json {
                                ui::check_ok(&i18n::t_args(
                                    "doctor-check-daemon-mcp-health",
                                    &[
                                        ("healthy", &healthy.to_string()),
                                        ("total", &total.to_string()),
                                    ],
                                ));
                            }
                        } else if !json {
                            ui::check_warn(&i18n::t_args(
                                "doctor-check-daemon-mcp-health",
                                &[
                                    ("healthy", &healthy.to_string()),
                                    ("total", &total.to_string()),
                                ],
                            ));
                        }
                        checks.push(serde_json::json!({"check": "mcp_health", "status": if healthy == total { "ok" } else { "warn" }, "healthy": healthy, "total": total}));
                    }
                }
            }
            _ => {}
        }
    }

    if !json {
        println!();
    }
    match std::process::Command::new("rustc")
        .arg("--version")
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-rust-version",
                    &[("version", &version)],
                ));
            }
            checks.push(serde_json::json!({"check": "rust", "status": "ok", "version": version}));
        }
        Err(_) => {
            if !json {
                ui::check_fail(&i18n::t("doctor-check-rust-not-found-fail"));
            }
            checks.push(serde_json::json!({"check": "rust", "status": "fail"}));
            all_ok = false;
        }
    }

    // Python runtime check
    match std::process::Command::new("python3")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-python-version",
                    &[("version", &version)],
                ));
            }
            checks.push(serde_json::json!({"check": "python", "status": "ok", "version": version}));
        }
        _ => {
            // Try `python` instead
            match std::process::Command::new("python")
                .arg("--version")
                .output()
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !json {
                        ui::check_ok(&i18n::t_args(
                            "doctor-check-python-version",
                            &[("version", &version)],
                        ));
                    }
                    checks.push(
                        serde_json::json!({"check": "python", "status": "ok", "version": version}),
                    );
                }
                _ => {
                    if !json {
                        ui::check_warn(&i18n::t("doctor-check-python-not-found-warn"));
                    }
                    checks.push(serde_json::json!({"check": "python", "status": "warn"}));
                }
            }
        }
    }

    // Node.js runtime check
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&i18n::t_args(
                    "doctor-check-node-version",
                    &[("version", &version)],
                ));
            }
            checks.push(serde_json::json!({"check": "node", "status": "ok", "version": version}));
        }
        _ => {
            if !json {
                ui::check_warn(&i18n::t("doctor-check-node-not-found-warn"));
            }
            checks.push(serde_json::json!({"check": "node", "status": "warn"}));
        }
    }

    // Framework-based audit checks (see crates/librefang-cli/src/doctor.rs).
    // Each check is its own struct, registered in `doctor::registered_checks`.
    // Migrating the legacy inline checks above into this framework can happen
    // incrementally — adding a new check is one struct + one registry entry,
    // no edits to this function.
    {
        let ctx = doctor::AuditContext {
            librefang_home: cli_librefang_home(),
        };
        for result in doctor::run_all(&ctx) {
            if !json {
                match result.severity {
                    doctor::Severity::Pass | doctor::Severity::Info => {
                        ui::check_ok(&result.summary);
                    }
                    doctor::Severity::Warn => {
                        ui::check_warn(&result.summary);
                        if let Some(hint) = &result.hint {
                            ui::hint(hint);
                        }
                    }
                    doctor::Severity::Error => {
                        ui::check_fail(&result.summary);
                        if let Some(hint) = &result.hint {
                            ui::hint(hint);
                        }
                    }
                }
            }
            let mut entry = serde_json::json!({
                "check": result.name,
                "status": result.severity.as_str(),
                "summary": result.summary,
            });
            if let Some(h) = &result.hint {
                entry["hint"] = serde_json::Value::String(h.clone());
            }
            checks.push(entry);
            if matches!(result.severity, doctor::Severity::Error) {
                all_ok = false;
            }
        }
    }

    if json {
        write_stdout_safe(
            &serde_json::to_string_pretty(&serde_json::json!({
                "all_ok": all_ok,
                "checks": checks,
            }))
            .unwrap_or_default(),
        );
    } else {
        println!();
        if all_ok {
            ui::success(&i18n::t("doctor-all-passed"));
            ui::hint(&i18n::t("hint-start-daemon-cmd"));
        } else if repaired {
            ui::success(&i18n::t("doctor-repairs-applied"));
        } else {
            ui::error(&i18n::t("doctor-some-failed"));
            if !repair {
                ui::hint(&i18n::t("hint-doctor-repair"));
            }
        }
    }
}
