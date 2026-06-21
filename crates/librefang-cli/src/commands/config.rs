//! `config` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

pub(crate) fn cmd_config_show() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!(
            "{}",
            i18n::t_args(
                "config-no-file-found",
                &[("path", &config_path.display().to_string())]
            )
        );
        println!("{}", i18n::t("config-run-init-hint"));
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args("config-read-error", &[("error", &e.to_string())])
        );
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

pub(crate) fn cmd_config_edit() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "{}",
                i18n::t_args("config-editor-exit", &[("status", &s.to_string())])
            );
        }
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args(
                    "config-editor-open-fail",
                    &[("editor", &editor), ("error", &e.to_string())]
                )
            );
            eprintln!("{}", i18n::t("config-editor-env-hint"));
        }
    }
}

pub(crate) fn cmd_config_get(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix"),
        );
        std::process::exit(1);
    });

    // Navigate dotted path
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
                std::process::exit(1);
            }
        }
    }

    // Print value
    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

/// Parse a string as a TOML integer, rejecting values outside i64 range.
/// TOML integers are i64; we never silently truncate `u64 > i64::MAX` into
/// negative numbers (#3461).
pub(crate) fn parse_toml_integer(raw: &str) -> Result<toml::Value, String> {
    if let Ok(v) = raw.parse::<i64>() {
        return Ok(toml::Value::Integer(v));
    }
    if let Ok(v) = raw.parse::<u64>() {
        return match i64::try_from(v) {
            Ok(v) => Ok(toml::Value::Integer(v)),
            Err(_) => Err(i18n::t_args(
                "config-val-exceeds-i64",
                &[("value", &v.to_string()), ("max", &i64::MAX.to_string())],
            )),
        };
    }
    Err(i18n::t_args("config-invalid-integer", &[("raw", raw)]))
}

pub(crate) fn cmd_config_set(key: &str, value: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent and set key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];

    // Validate: single-part keys must be known scalar fields, not sections.
    // Writing a section name as a scalar silently breaks config deserialization.
    if parts.len() == 1 {
        let known_scalars = [
            "home_dir",
            "data_dir",
            "log_level",
            "api_listen",
            "network_enabled",
            "api_key",
            "language",
            "max_cron_jobs",
            "usage_footer",
            "workspaces_dir",
        ];
        if !known_scalars.contains(&last_key) {
            ui::error_with_fix(
                &i18n::t_args("config-section-not-scalar", &[("key", last_key)]),
                &i18n::t_args("config-section-not-scalar-fix", &[("key", last_key)]),
            );
            std::process::exit(1);
        }
    }

    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    // Try to preserve type: if the existing value is an integer, parse as int, etc.
    let new_value = if let Some(existing) = tbl.get(last_key) {
        match existing {
            toml::Value::Integer(_) => match parse_toml_integer(value) {
                Ok(v) => v,
                Err(msg) => {
                    ui::error(&msg);
                    std::process::exit(1);
                }
            },
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        }
    } else {
        // No existing value — infer type from the string content
        if let Ok(b) = value.parse::<bool>() {
            toml::Value::Boolean(b)
        } else if let Ok(v) = parse_toml_integer(value) {
            v
        } else if let Ok(f) = value.parse::<f64>() {
            toml::Value::Float(f)
        } else {
            toml::Value::String(value.to_string())
        }
    };

    tbl.insert(last_key.to_string(), new_value);

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args(
        "config-set-kv",
        &[("key", key), ("value", value)],
    ));
}

pub(crate) fn cmd_config_unset(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent table and remove the final key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    if tbl.remove(last_key).is_none() {
        ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
        std::process::exit(1);
    }

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args("config-removed-key", &[("key", key)]));
}

pub(crate) fn cmd_config_set_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    let key = prompt_input(&i18n::t_args(
        "config-paste-api-key-prompt",
        &[("provider", provider)],
    ));
    if key.is_empty() {
        ui::error(&i18n::t("config-no-key"));
        return;
    }

    match dotenv::save_env_key(&env_var, &key) {
        Ok(()) => {
            ui::success(&i18n::t_args("config-saved-key", &[("env_var", &env_var)]));
            // Test the key
            print!("{}", i18n::t("config-testing-key"));
            io::stdout().flush().unwrap();
            if test_api_key(provider, &key) {
                println!("{}", i18n::t("config-test-ok").bright_green());
            } else {
                println!("{}", i18n::t("config-test-unverified").bright_yellow());
            }
        }
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-save-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_delete_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    match dotenv::remove_env_key(&env_var) {
        Ok(()) => ui::success(&i18n::t_args(
            "config-removed-env",
            &[("env_var", &env_var)],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-remove-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_test_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    if std::env::var(&env_var).is_err() {
        ui::error(&i18n::t_args(
            "config-env-not-set",
            &[("env_var", &env_var)],
        ));
        ui::hint(&i18n::t_args(
            "config-set-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }

    print!(
        "{}",
        i18n::t_args(
            "config-testing-provider-key",
            &[("provider", provider), ("env_var", &env_var)]
        )
    );
    io::stdout().flush().unwrap();
    if test_api_key(provider, &std::env::var(&env_var).unwrap_or_default()) {
        println!("{}", i18n::t("config-test-ok").bright_green());
    } else {
        println!("{}", i18n::t("config-test-failed").bright_red());
        ui::hint(&i18n::t_args(
            "config-update-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }
}
