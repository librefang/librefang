//! BossFang storage commands — inspect and manage the embedded SurrealDB store.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

#[allow(unused_imports)]
use crate::commands::prelude::*;

/// `librefang storage explore [--limit N] [--json]`
///
/// Read-only inspection of the `audit_entries` table in the embedded
/// SurrealDB store. Refuses to run while the daemon is up.
#[cfg(feature = "surreal-backend")]
pub(crate) fn cmd_audit_explore_surreal(config: Option<PathBuf>, limit: u32, json: bool) {
    use librefang_storage::{StorageConfig, SurrealConnectionPool};

    let kernel_config = match load_config(config.as_deref()) {
        Ok(cfg) => cfg,
        Err(_) => std::process::exit(1),
    };
    let storage_cfg: StorageConfig = kernel_config.storage.clone();

    // Refuse if the daemon is running — RocksDB holds an exclusive lock.
    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to open the embedded SurrealDB store"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ui::error(&format!("failed to start tokio runtime: {e}"));
            std::process::exit(1);
        }
    };

    let result: Result<Vec<serde_json::Value>, String> = rt.block_on(async move {
        let pool = SurrealConnectionPool::new();
        let session = pool
            .open(&storage_cfg)
            .await
            .map_err(|e| format!("open surreal: {e}"))?;
        // Apply schema migrations so a freshly-created store has the
        // `audit_entries` table even if the daemon never booted there.
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .map_err(|e| format!("apply schema migrations: {e}"))?;
        session
            .client()
            .query(
                "SELECT seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash \
                    FROM audit_entries ORDER BY seq ASC LIMIT $limit",
            )
            .bind(("limit", i64::from(limit)))
            .await
            .map_err(|e| format!("query audit_entries: {e}"))?
            .take::<Vec<serde_json::Value>>(0)
            .map_err(|e| format!("decode audit_entries: {e}"))
    });

    let rows = match result {
        Ok(r) => r,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };

    if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                ui::error(&format!("serialise rows: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }

    if rows.is_empty() {
        ui::hint("No audit entries in the embedded SurrealDB store.");
        return;
    }
    println!("{:<6} {:<25} {:<24} ACTION", "SEQ", "TIMESTAMP", "AGENT");
    println!("{}", "-".repeat(80));
    for row in &rows {
        let seq = row.get("seq").and_then(|v| v.as_i64()).unwrap_or(-1);
        let ts = row.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let agent = row.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
        let action = row.get("action").and_then(|v| v.as_str()).unwrap_or("");
        println!("{seq:<6} {ts:<25} {agent:<24} {action}");
    }
}

/// `librefang storage migrate --from sqlite --to surreal [--dry-run]`
///
/// Streams the legacy SQLite tables into the configured embedded or remote
/// SurrealDB instance. Idempotent via SurrealDB upserts.
#[cfg(all(feature = "sqlite-backend", feature = "surreal-backend"))]
fn cmd_storage_migrate(config: Option<PathBuf>, dry_run: bool) {
    use librefang_storage::migrate::{migrate_sqlite_to_surreal, plan_sqlite, MigrationOptions};
    use librefang_storage::SurrealConnectionPool;

    let kernel_config = match load_config(config.as_deref()) {
        Ok(cfg) => cfg,
        Err(_) => std::process::exit(1),
    };
    let sqlite_path = kernel_config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| kernel_config.data_dir.join("librefang.db"));

    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to migrate while the writer holds the database"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    if dry_run {
        match plan_sqlite(&sqlite_path) {
            Ok(plan) => {
                println!("Dry run — sqlite source: {}", sqlite_path.display());
                println!("{:<28} ROWS", "TABLE");
                println!("{}", "-".repeat(40));
                for (table, rows) in &plan.source_rows {
                    println!("{table:<28} {rows}");
                }
                println!("{}", "-".repeat(40));
                println!("{:<28} {}", "TOTAL", plan.total_rows());
            }
            Err(e) => {
                ui::error(&format!("plan failed: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }

    let receipts_dir = kernel_config.data_dir.join("migrations");
    let storage_cfg = kernel_config.storage.clone();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ui::error(&format!("failed to start tokio runtime: {e}"));
            std::process::exit(1);
        }
    };

    let outcome = rt.block_on(async {
        let pool = SurrealConnectionPool::new();
        let session = pool
            .open(&storage_cfg)
            .await
            .map_err(|e| format!("open surreal target: {e}"))?;
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .map_err(|e| format!("apply schema migrations: {e}"))?;
        let opts = MigrationOptions {
            dry_run: false,
            receipt_dir: Some(receipts_dir.clone()),
        };
        let session_for_blocking = session.clone();
        let sqlite_path_for_blocking = sqlite_path.clone();
        tokio::task::spawn_blocking(move || {
            migrate_sqlite_to_surreal(&sqlite_path_for_blocking, &session_for_blocking, &opts)
        })
        .await
        .map_err(|e| format!("join migration task: {e}"))?
        .map_err(|e| format!("migrate: {e}"))
    });

    match outcome {
        Ok(receipt) => {
            ui::success(&format!(
                "Migrated {} row(s) from {} to {}.",
                receipt.copied.values().sum::<u64>(),
                receipt.source,
                receipt.target,
            ));
            for (table, rows) in &receipt.copied {
                println!("  {table:<28} {rows}");
            }
            if !receipt.errors.is_empty() {
                ui::hint("Some tables reported errors:");
                for (table, err) in &receipt.errors {
                    println!("  {table:<28} {err}");
                }
            }
            ui::hint(&format!(
                "Receipt written under {}/",
                receipts_dir.display()
            ));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

/// Provision a UAR namespace + least-privilege user on a shared remote SurrealDB,
/// then write `[uar.remote]` (and optionally `share_librefang_storage`) to config.toml.
#[cfg(feature = "surreal-backend")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_storage_link_uar(
    remote_url: String,
    root_user: String,
    root_pass_env: String,
    namespace: String,
    database: String,
    app_user: String,
    app_pass_env: String,
    also_link_memory: bool,
) {
    use librefang_storage::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};

    let root_pass = std::env::var(&root_pass_env).unwrap_or_else(|_| {
        ui::error(&format!(
            "env var `{root_pass_env}` not set (required for root credentials)"
        ));
        std::process::exit(1);
    });
    let app_pass = std::env::var(&app_pass_env).unwrap_or_else(|_| {
        ui::error(&format!(
            "env var `{app_pass_env}` not set (required for app user password)"
        ));
        std::process::exit(1);
    });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            ui::error(&format!("tokio runtime: {e}"));
            std::process::exit(1);
        });

    let outcome: Result<(), String> = rt.block_on(async {
        let root_cfg = StorageConfig {
            backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                url: remote_url.clone(),
                namespace: namespace.clone(),
                database: database.clone(),
                username: root_user.clone(),
                password_env: root_pass_env.clone(),
                tls_skip_verify: false,
            }),
            namespace: namespace.clone(),
            database: database.clone(),
            legacy_sqlite_path: None,
        };

        // Set the env var so the pool's credential resolver can find the password.
        // SAFETY: single-threaded current_thread runtime; no concurrent env readers.
        #[allow(unused_unsafe)]
        unsafe { std::env::set_var(&root_pass_env, &root_pass) };

        let pool = librefang_storage::SurrealConnectionPool::new();
        let session = pool
            .open(&root_cfg)
            .await
            .map_err(|e| format!("connect to {remote_url}: {e}"))?;
        let db = session.client();

        ui::step(&format!(
            "Provisioning UAR namespace `{namespace}` on {remote_url} ..."
        ));

        // 1. Namespace
        db.query(format!("DEFINE NAMESPACE IF NOT EXISTS `{namespace}`;"))
            .await
            .map_err(|e| format!("DEFINE NAMESPACE: {e}"))?;

        // 2. Database
        db.query(format!(
            "USE NAMESPACE `{namespace}`; DEFINE DATABASE IF NOT EXISTS `{database}` ON NAMESPACE `{namespace}`;"
        ))
        .await
        .map_err(|e| format!("DEFINE DATABASE: {e}"))?;

        // 3. Application user scoped to the namespace
        let escaped_pass = app_pass.replace('\'', "\\'");
        db.query(format!(
            "USE NAMESPACE `{namespace}`; \
             DEFINE USER IF NOT EXISTS `{app_user}` ON NAMESPACE `{namespace}` \
             PASSWORD '{escaped_pass}' ROLES EDITOR;"
        ))
        .await
        .map_err(|e| format!("DEFINE USER: {e}"))?;

        Ok(())
    });

    if let Err(e) = outcome {
        ui::error(&e);
        std::process::exit(1);
    }

    ui::check_ok(&format!(
        "Provisioned namespace `{namespace}`, database `{database}`, user `{app_user}`"
    ));

    // Write [uar.remote] to config.toml
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(
            "config.toml not found",
            "run `librefang init` to initialise",
        );
        std::process::exit(1);
    }

    let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    if !doc.contains_table("uar") {
        doc.insert("uar", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let uar_tbl = doc["uar"].as_table_mut().expect("uar is a table");

    if also_link_memory {
        uar_tbl.insert("share_librefang_storage", toml_edit::value(true));
    }

    let mut remote_tbl = toml_edit::Table::new();
    remote_tbl.insert("url", toml_edit::value(&remote_url));
    remote_tbl.insert("namespace", toml_edit::value(&namespace));
    remote_tbl.insert("database", toml_edit::value(&database));
    remote_tbl.insert("username", toml_edit::value(&app_user));
    remote_tbl.insert("password_env", toml_edit::value(&app_pass_env));

    uar_tbl.insert("remote", toml_edit::Item::Table(remote_tbl));

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        ui::error(&format!("write config.toml: {e}"));
        std::process::exit(1);
    }

    ui::check_ok("Updated config.toml with [uar.remote]");

    if find_daemon().is_some() {
        ui::check_warn("Daemon is running — restart to activate: `librefang restart`");
    } else {
        ui::hint("Start the daemon to activate: `librefang start`");
    }
}

/// Remove `[uar.remote]` and `share_librefang_storage` from config.toml.
/// Optionally drops the application user from the remote instance.
#[cfg(feature = "surreal-backend")]
pub(crate) fn cmd_storage_unlink_uar(
    purge_user: bool,
    root_user: String,
    root_pass_env: Option<String>,
) {
    use librefang_storage::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};

    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(
            "config.toml not found",
            "run `librefang init` to initialise",
        );
        std::process::exit(1);
    }

    let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    let maybe_remote: Option<(String, String, String, String)> =
        doc.get("uar").and_then(|u| u.get("remote")).and_then(|r| {
            let url = r.get("url")?.as_str()?.to_string();
            let ns = r.get("namespace")?.as_str()?.to_string();
            let user = r.get("username")?.as_str()?.to_string();
            let pass_env = r.get("password_env")?.as_str()?.to_string();
            Some((url, ns, user, pass_env))
        });

    if purge_user {
        let Some((url, ns, app_user, _pass_env)) = maybe_remote.as_ref() else {
            ui::error("no [uar.remote] found in config.toml; cannot purge user");
            std::process::exit(1);
        };
        let Some(ref root_pass_env_name) = root_pass_env else {
            ui::error("--root-pass-env is required when --purge-user is set");
            std::process::exit(1);
        };
        let root_pass = std::env::var(root_pass_env_name).unwrap_or_else(|_| {
            ui::error(&format!("env var `{root_pass_env_name}` not set"));
            std::process::exit(1);
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                ui::error(&format!("tokio runtime: {e}"));
                std::process::exit(1);
            });

        let (url, ns, app_user, root_pass_env_name) = (
            url.clone(),
            ns.clone(),
            app_user.clone(),
            root_pass_env_name.clone(),
        );
        // SAFETY: single-threaded current_thread runtime; no concurrent env readers.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var(&root_pass_env_name, &root_pass)
        };

        let outcome: Result<(), String> = rt.block_on(async {
            let root_cfg = StorageConfig {
                backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                    url: url.clone(),
                    namespace: ns.clone(),
                    database: "main".to_string(),
                    username: root_user.clone(),
                    password_env: root_pass_env_name.clone(),
                    tls_skip_verify: false,
                }),
                namespace: ns.clone(),
                database: "main".to_string(),
                legacy_sqlite_path: None,
            };
            let pool = librefang_storage::SurrealConnectionPool::new();
            let session = pool
                .open(&root_cfg)
                .await
                .map_err(|e| format!("connect: {e}"))?;

            session
                .client()
                .query(format!(
                    "USE NAMESPACE `{ns}`; REMOVE USER IF EXISTS `{app_user}` ON NAMESPACE `{ns}`;"
                ))
                .await
                .map_err(|e| format!("REMOVE USER: {e}"))?;
            Ok(())
        });

        match outcome {
            Ok(()) => ui::check_ok(&format!("Removed user `{app_user}` from namespace `{ns}`")),
            Err(e) => {
                ui::check_warn(&format!(
                    "Could not drop user (config will still be cleared): {e}"
                ));
            }
        }
    }

    // Strip [uar.remote] and share_librefang_storage
    if let Some(uar_tbl) = doc.get_mut("uar").and_then(|u| u.as_table_mut()) {
        uar_tbl.remove("remote");
        uar_tbl.remove("share_librefang_storage");
    }

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        ui::error(&format!("write config.toml: {e}"));
        std::process::exit(1);
    }

    ui::check_ok("Removed [uar.remote] from config.toml");

    if find_daemon().is_some() {
        ui::check_warn("Daemon is running — restart to activate: `librefang restart`");
    } else {
        ui::hint("Changes take effect on next daemon start.");
    }
}
