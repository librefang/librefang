//! `monitoring` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

pub(crate) fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section(&i18n::t("section-security-status"));
    ui::blank();
    ui::kv(&i18n::t("label-audit-trail"), &i18n::t("value-audit-trail"));
    ui::kv(
        &i18n::t("label-taint-tracking"),
        &i18n::t("value-taint-tracking"),
    );
    ui::kv(
        &i18n::t("label-wasm-sandbox"),
        &i18n::t("value-wasm-sandbox"),
    );
    ui::kv(
        &i18n::t("label-wire-protocol"),
        &i18n::t("value-wire-protocol"),
    );
    ui::kv(&i18n::t("label-api-keys"), &i18n::t("value-api-keys"));
    ui::kv(&i18n::t("label-manifests"), &i18n::t("value-manifests"));
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv(&i18n::t("label-active-agents"), &agents.to_string());
    }
}

pub(crate) fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("entries")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        let mut t = crate::table::Table::new(&["TIMESTAMP", "AGENT", "TYPE", "EVENT"]);
        for entry in arr {
            let agent_id = entry["agent_id"].as_str().unwrap_or("");
            let agent_col = if agent_id.len() > 16 {
                &agent_id[..16]
            } else if agent_id.is_empty() {
                entry["agent_name"].as_str().unwrap_or("?")
            } else {
                agent_id
            };
            t.add_row(&[
                entry["timestamp"].as_str().unwrap_or("?"),
                agent_col,
                entry["action"]
                    .as_str()
                    .or_else(|| entry["event_type"].as_str())
                    .unwrap_or("?"),
                entry["detail"]
                    .as_str()
                    .or_else(|| entry["description"].as_str())
                    .unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t("audit-verified"));
    } else {
        ui::error(&i18n::t("audit-failed"));
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

/// Destructively reset the local audit trail.
///
/// Truncates `audit_entries` in SQLite and removes the anchor file so the
/// next daemon boot seeds a fresh Merkle chain. Refuses to run while the
/// daemon holds the DB (SQLite WAL mode + writer lock) and without
/// `--confirm`.
pub(crate) fn cmd_audit_reset(config: Option<PathBuf>, confirm: bool) {
    let daemon = daemon_config_context(config.as_deref());
    // `load_config` already eprintln!s the underlying parse / deserialize
    // error (see #5186); printing it again here would double the message.
    let kernel_config = match load_config(config.as_deref()) {
        Ok(cfg) => cfg,
        Err(_) => std::process::exit(1),
    };

    let db_path = kernel_config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| kernel_config.data_dir.join("librefang.db"));

    let anchor_path = match kernel_config.audit.anchor_path.as_ref() {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => kernel_config.data_dir.join(p),
        None => kernel_config.data_dir.join("audit.anchor"),
    };

    if !confirm {
        ui::error(&i18n::t("monitoring-audit-reset-destructive"));
        ui::blank();
        println!("{}", i18n::t("monitoring-audit-reset-would-header"));
        println!(
            "{}",
            i18n::t_args(
                "monitoring-audit-reset-would-delete",
                &[("path", &db_path.display().to_string())]
            )
        );
        println!(
            "{}",
            i18n::t_args(
                "monitoring-audit-reset-would-remove-anchor",
                &[("path", &anchor_path.display().to_string())]
            )
        );
        println!("{}", i18n::t("monitoring-audit-reset-would-restart"));
        std::process::exit(1);
    }

    // Refuse if daemon is running — SQLite writer lock would block or corrupt.
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &i18n::t_args("monitoring-daemon-running-error", &[("url", &base)]),
            &i18n::t("monitoring-daemon-running-error-fix"),
        );
        std::process::exit(1);
    }

    if !db_path.exists() {
        ui::error(&i18n::t_args(
            "monitoring-db-not-found",
            &[("path", &db_path.display().to_string())],
        ));
        std::process::exit(1);
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            ui::error(&i18n::t_args(
                "monitoring-db-open-failed",
                &[
                    ("path", &db_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            std::process::exit(1);
        }
    };

    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_entries", [], |r| r.get(0))
        .unwrap_or(0);

    // Remove the anchor FIRST. If the subsequent DB truncation then fails,
    // the next daemon boot sees `read_anchor = None` and re-seeds from the
    // current DB tip — a consistent (if still broken) state the user can
    // retry. The reverse order (DB first, anchor second) would instead
    // leave an empty table alongside a stale anchor, which produces a
    // fresh MISMATCH error the user didn't have before calling reset.
    let anchor_removed = if anchor_path.exists() {
        match std::fs::remove_file(&anchor_path) {
            Ok(()) => true,
            Err(e) => {
                ui::error(&i18n::t_args(
                    "monitoring-anchor-remove-failed",
                    &[
                        ("path", &anchor_path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ));
                std::process::exit(1);
            }
        }
    } else {
        false
    };

    if let Err(e) = conn.execute("DELETE FROM audit_entries", []) {
        ui::error(&i18n::t_args(
            "monitoring-db-truncate-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }
    drop(conn);
    // `seq` is `INTEGER PRIMARY KEY` without AUTOINCREMENT, so the next
    // insert after an empty table naturally gets seq = 1. No sqlite_sequence
    // fiddling needed.

    let anchor_detail = if anchor_removed {
        i18n::t_args(
            "monitoring-audit-reset-anchor-deleted",
            &[("path", &anchor_path.display().to_string())],
        )
    } else {
        i18n::t("monitoring-audit-reset-anchor-none")
    };
    ui::success(&i18n::t_args(
        "monitoring-audit-reset-success",
        &[
            ("count", &rows_before.to_string()),
            ("anchor_detail", &anchor_detail),
        ],
    ));
    ui::hint(&i18n::t("monitoring-audit-reset-seed-fresh"));
}

pub(crate) fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("kv_pairs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        let mut t = crate::table::Table::new(&["KEY", "VALUE"]);
        for kv in arr {
            t.add_row(&[
                kv["key"].as_str().unwrap_or("?"),
                &kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect::<String>(),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-set",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

pub(crate) fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-deleted",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

pub(crate) fn cmd_devices_list(json: bool) {
    let base = require_daemon("devices list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/pairing/devices")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No paired devices.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "LAST SEEN"]);
        for d in arr {
            t.add_row(&[
                d["id"].as_str().unwrap_or("?"),
                d["name"].as_str().unwrap_or("?"),
                d["last_seen"].as_str().unwrap_or("?"),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_devices_pair() {
    let base = require_daemon("qr");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/pairing/request")).send());
    if let Some(qr) = body["qr_data"].as_str() {
        ui::section(&i18n::t("section-device-pairing"));
        ui::blank();
        // Render a simple text-based QR representation
        println!("  {}", i18n::t("device-scan-qr"));
        ui::blank();
        println!("  {qr}");
        ui::blank();
        if let Some(code) = body["pairing_code"].as_str() {
            ui::kv(&i18n::t("label-pairing-code"), code);
        }
        if let Some(expires) = body["expires_at"].as_str() {
            ui::kv(&i18n::t("label-expires"), expires);
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_devices_remove(id: &str) {
    let base = require_daemon("devices remove");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/pairing/devices/{id}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "device-remove-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("device-removed", &[("id", id)]));
    }
}

pub(crate) fn cmd_webhooks_list(json: bool) {
    let base = require_daemon("webhooks list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/webhooks")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("webhooks")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No webhooks configured.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "ENABLED", "URL"]);
        for w in arr {
            t.add_row(&[
                w["id"].as_str().unwrap_or("?"),
                w["name"].as_str().unwrap_or("?"),
                if w["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                w["url"].as_str().unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_webhooks_create(agent: &str, url: &str) {
    let base = require_daemon("webhooks create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Derive a name from the URL hostname
    let name = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "webhook".to_string());

    let body = daemon_json(
        client
            .post(format!("{base}/api/webhooks"))
            .json(&serde_json::json!({
                "name": format!("{agent}-{name}"),
                "url": url,
                "events": ["all"],
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&i18n::t_args("webhook-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

pub(crate) fn cmd_webhooks_delete(id: &str) {
    let base = require_daemon("webhooks delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/webhooks/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "webhook-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("webhook-deleted", &[("id", id)]));
    }
}

pub(crate) fn cmd_webhooks_test(id: &str) {
    let base = require_daemon("webhooks test");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/webhooks/{id}/test")).send());
    if body["success"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t_args("webhook-test-ok", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-test-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}
