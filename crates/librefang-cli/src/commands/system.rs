//! `system` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Dashboard command
// ---------------------------------------------------------------------------

pub(crate) fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint(&i18n::t("daemon-no-running-auto"));
        match start_daemon_background() {
            Ok(url) => {
                ui::success(&i18n::t("daemon-started"));
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &i18n::t_args("daemon-start-fail", &[("error", &e.to_string())]),
                    &i18n::t("daemon-start-fail-fix"),
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/");
    ui::success(&i18n::t_args("dashboard-opening", &[("url", &url)]));
    if copy_to_clipboard(&url) {
        ui::hint(&i18n::t("hint-url-copied"));
    }
    if !open_in_browser(&url) {
        ui::hint(&i18n::t_args(
            "hint-could-not-open-browser-visit",
            &[("url", &url)],
        ));
    }
}

// ---------------------------------------------------------------------------
// Shell completion command
// ---------------------------------------------------------------------------

pub(crate) fn cmd_completion(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "librefang", &mut std::io::stdout());
}

// ---------------------------------------------------------------------------
// Migrate command
// ---------------------------------------------------------------------------

pub(crate) fn cmd_migrate(args: MigrateArgs) {
    let source = match args.from {
        MigrateSourceArg::Openclaw => librefang_import::MigrateSource::OpenClaw,
        MigrateSourceArg::Langchain => librefang_import::MigrateSource::LangChain,
        MigrateSourceArg::Autogpt => librefang_import::MigrateSource::AutoGpt,
        MigrateSourceArg::Openfang => librefang_import::MigrateSource::OpenFang,
    };

    let source_dir = args.source_dir.unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| {
            eprintln!("{}", i18n::t("migrate-error-home-dir"));
            std::process::exit(1);
        });
        match source {
            librefang_import::MigrateSource::OpenClaw => home.join(".openclaw"),
            librefang_import::MigrateSource::LangChain => home.join(".langchain"),
            librefang_import::MigrateSource::AutoGpt => home.join("Auto-GPT"),
            librefang_import::MigrateSource::OpenFang => home.join(".openfang"),
        }
    });

    let target_dir = cli_librefang_home();

    println!(
        "{}",
        i18n::t_args(
            "migrate-start-msg",
            &[
                ("source", &source.to_string()),
                ("path", &source_dir.display().to_string())
            ]
        )
    );
    if args.dry_run {
        println!("{}\n", i18n::t("migrate-dry-run-hint"));
    }

    let options = librefang_import::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: args.dry_run,
    };

    let mut sp = progress::auto(&i18n::t("migrate-progress-label"), None);
    match librefang_import::run_migration(&options) {
        Ok(report) => {
            sp.finish(&i18n::t("migrate-complete-msg"));
            report.print_summary();

            // Save migration report
            if !args.dry_run {
                let report_path = options.target_dir.join("migration_report.md");
                if let Err(e) = std::fs::write(&report_path, report.to_markdown()) {
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "migrate-warn-report-save-failed",
                            &[("error", &e.to_string())]
                        )
                    );
                } else {
                    println!(
                        "{}",
                        i18n::t_args(
                            "migrate-report-saved",
                            &[("path", &report_path.display().to_string())]
                        )
                    );
                }
            }
        }
        Err(e) => {
            sp.finish_with_failure(&i18n::t_args(
                "migrate-failed-msg",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Scaffold commands (librefang new skill/integration)
// ---------------------------------------------------------------------------

pub(crate) fn cmd_scaffold(kind: ScaffoldKind) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = match kind {
        ScaffoldKind::Skill => {
            librefang_extensions::installer::scaffold_skill(&cwd.join("my-skill"))
        }
        ScaffoldKind::Mcp => {
            librefang_extensions::installer::scaffold_integration(&cwd.join("my-mcp"))
        }
    };
    match result {
        Ok(msg) => ui::success(&msg),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if json {
            let mut data = body.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "version".to_string(),
                    serde_json::json!(env!("CARGO_PKG_VERSION")),
                );
                obj.insert("api_url".to_string(), serde_json::json!(base));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv(
            &i18n::t("label-status"),
            body["status"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-agents"),
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv(
            &i18n::t("label-provider"),
            body["default_provider"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-model"),
            body["default_model"].as_str().unwrap_or("?"),
        );
        ui::kv(&i18n::t("label-api"), &base);
        ui::kv(
            &i18n::t("label-data-dir"),
            body["data_dir"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-uptime"),
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );
    } else {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "daemon": "not_running",
                })
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv_warn(
            &i18n::t("label-daemon"),
            &i18n::t("label-daemon-not-running"),
        );
        ui::hint(&i18n::t("hint-start-daemon"));
    }
}

pub(crate) fn cmd_system_version(json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")})
        );
        return;
    }
    println!("librefang {}", env!("CARGO_PKG_VERSION"));
}
