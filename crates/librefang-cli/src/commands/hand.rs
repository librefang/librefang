//! `hand` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Channel commands
// ---------------------------------------------------------------------------

// maybe_write_channel_config / notify_daemon_restart removed — they
// supported the interactive in-process channel onboarding flow whose
// callers were dropped when channels moved to sidecars, leaving both
// helpers orphaned.

// ---------------------------------------------------------------------------
// Hand commands
// ---------------------------------------------------------------------------

pub(crate) fn cmd_hand_install(path: &str) {
    let base = require_daemon("hand install");
    let dir = std::path::Path::new(path);
    let toml_path = dir.join("HAND.toml");
    let skill_path = dir.join("SKILL.md");

    if !toml_path.exists() {
        eprintln!(
            "Error: No HAND.toml found in {}",
            dir.canonicalize()
                .unwrap_or_else(|_| dir.to_path_buf())
                .display()
        );
        std::process::exit(1);
    }

    let toml_content = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", toml_path.display());
        std::process::exit(1);
    });
    let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_default();

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/install"))
            .json(&serde_json::json!({
                "toml_content": toml_content,
                "skill_content": skill_content,
            }))
            .send(),
    );

    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    println!(
        "Installed hand: {} ({})",
        body["name"].as_str().unwrap_or("?"),
        body["id"].as_str().unwrap_or("?"),
    );
    println!(
        "Use `librefang hand activate {}` to start it.",
        body["id"].as_str().unwrap_or("?")
    );
}

pub(crate) fn cmd_hand_list() {
    let base = require_daemon("hand list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands")).send());
    // API returns {"hands": [...]} or a bare array
    let arr_val;
    if let Some(arr) = body.get("hands").and_then(|v| v.as_array()) {
        arr_val = arr.clone();
    } else if let Some(arr) = body.as_array() {
        arr_val = arr.clone();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = Some(&arr_val) {
        if arr.is_empty() {
            println!("No hands available.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "CATEGORY", "DESCRIPTION"]);
        for h in arr {
            t.add_row(&[
                h["id"].as_str().unwrap_or("?"),
                h["name"].as_str().unwrap_or("?"),
                h["category"].as_str().unwrap_or("?"),
                &h["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            ]);
        }
        t.print();
        println!("\nUse `librefang hand activate <id>` to activate a hand.");
    }
}

pub(crate) fn cmd_hand_active() {
    let base = require_daemon("hand active");
    let client = daemon_client();
    let arr = fetch_active_hand_instances(&base, &client);
    if arr.is_empty() {
        println!("No active hands.");
        return;
    }
    let mut t = crate::table::Table::new(&["INSTANCE", "HAND", "STATUS", "AGENT"]);
    for i in &arr {
        t.add_row(&[
            i["instance_id"].as_str().unwrap_or("?"),
            i["hand_id"].as_str().unwrap_or("?"),
            i["status"].as_str().unwrap_or("?"),
            i["agent_name"].as_str().unwrap_or("?"),
        ]);
    }
    t.print();
}

pub(crate) fn cmd_hand_status(id: Option<&str>) {
    if id.is_none() {
        cmd_hand_active();
        return;
    }

    let id = id.unwrap_or_default();
    let base = require_daemon("hand status");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);

    if let Some(instance) = resolve_hand_instance(&active, id) {
        let hand_id = instance["hand_id"].as_str().unwrap_or(id);
        let hand_body = daemon_json(client.get(format!("{base}/api/hands/{hand_id}")).send());
        let name = hand_body["name"].as_str().unwrap_or(hand_id);
        let status = instance["status"].as_str().unwrap_or("unknown");
        let instance_id = instance["instance_id"].as_str().unwrap_or("?");
        let agent_name = instance["agent_name"].as_str().unwrap_or("?");

        ui::section(&i18n::t("hand-status-title"));
        ui::kv(&i18n::t("label-hand"), hand_id);
        ui::kv(&i18n::t("label-name"), name);
        ui::kv(&i18n::t("label-instance"), instance_id);
        ui::kv(&i18n::t("label-status"), status);
        ui::kv(&i18n::t("label-agent"), agent_name);
        return;
    }

    let hand_body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if hand_body.get("error").is_some() {
        ui::error(&i18n::t_args("hand-not-found", &[("id", id)]));
        std::process::exit(1);
    }

    ui::section(&i18n::t("hand-status-title"));
    ui::kv(
        &i18n::t("label-hand"),
        hand_body["id"].as_str().unwrap_or(id),
    );
    ui::kv(
        &i18n::t("label-name"),
        hand_body["name"].as_str().unwrap_or(id),
    );
    ui::kv(&i18n::t("label-status"), &i18n::t("label-status-inactive"));
    if let Some(description) = hand_body["description"].as_str() {
        if !description.is_empty() {
            ui::kv(&i18n::t("label-description"), description);
        }
    }
}

pub(crate) fn cmd_hand_activate(id: &str) {
    let base = require_daemon("hand activate");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/activate"))
            .header("content-type", "application/json")
            .body("{}")
            .send(),
    );
    if body.get("instance_id").is_some() {
        println!(
            "Hand '{}' activated (instance: {}, agent: {})",
            id,
            body["instance_id"].as_str().unwrap_or("?"),
            body["agent_name"].as_str().unwrap_or("?"),
        );
    } else {
        eprintln!(
            "Failed to activate hand '{}': {}",
            id,
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_hand_deactivate(id: &str) {
    let base = require_daemon("hand deactivate");
    let client = daemon_client();
    // First find the instance ID for this hand
    let arr = fetch_active_hand_instances(&base, &client);
    let instance_id = arr.iter().find_map(|i| {
        if i["hand_id"].as_str() == Some(id) {
            i["instance_id"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    });

    match instance_id {
        Some(iid) => {
            let body = daemon_json(
                client
                    .delete(format!("{base}/api/hands/instances/{iid}"))
                    .send(),
            );
            if body.get("status").is_some() {
                println!("Hand '{id}' deactivated.");
            } else {
                eprintln!(
                    "Failed: {}",
                    body["error"].as_str().unwrap_or("Unknown error")
                );
                std::process::exit(1);
            }
        }
        None => {
            eprintln!("No active instance found for hand '{id}'.");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_hand_info(id: &str) {
    let base = require_daemon("hand info");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if body.get("error").is_some() {
        eprintln!("Hand not found: {}", body["error"].as_str().unwrap_or(id));
        std::process::exit(1);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
}

pub(crate) fn cmd_hand_check_deps(id: &str) {
    let base = require_daemon("hand check-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/check-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_hand_install_deps(id: &str) {
    let base = require_daemon("hand install-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/install-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&i18n::t_args("hand-install-deps-success", &[("id", id)]));
        if let Some(results) = body.get("results") {
            println!(
                "{}",
                serde_json::to_string_pretty(results).unwrap_or_default()
            );
        }
    }
}

pub(crate) fn cmd_hand_pause(id: &str) {
    let base = require_daemon("hand pause");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/pause"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-paused",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

pub(crate) fn cmd_hand_resume(id: &str) {
    let base = require_daemon("hand resume");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/resume"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-resumed",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

pub(crate) fn cmd_hand_settings(id: &str) {
    let base = require_daemon("hand settings");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}/settings")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    if let Some(config) = body.get("config").and_then(|c| c.as_object()) {
        if config.is_empty() {
            ui::step(&i18n::t_args("hand-no-settings", &[("id", id)]));
        } else {
            ui::section(&i18n::t_args("hand-settings-title", &[("id", id)]));
            for (k, v) in config {
                println!("  {}: {}", k.bold(), v);
            }
        }
    } else {
        ui::step(&i18n::t_args("hand-no-settings", &[("id", id)]));
    }
}

pub(crate) fn cmd_hand_set(id: &str, key: &str, value: &str) {
    let base = require_daemon("hand set");
    let client = daemon_client();
    let mut config = serde_json::Map::new();
    config.insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    let body = daemon_json(
        client
            .put(format!("{base}/api/hands/{id}/settings"))
            .json(&serde_json::json!({ "config": config }))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "label-failed-reason",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        std::process::exit(1);
    }
    ui::success(&i18n::t_args(
        "hand-set-setting-success",
        &[("key", key), ("value", value), ("id", id)],
    ));
}

pub(crate) fn cmd_hand_reload() {
    let base = require_daemon("hand reload");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/hands/reload")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "label-failed-reason",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
        std::process::exit(1);
    }
    let added = body["added"].as_u64().unwrap_or(0);
    let updated = body["updated"].as_u64().unwrap_or(0);
    let total = body["total"].as_u64().unwrap_or(0);
    ui::success(&i18n::t_args(
        "hand-reloaded-summary",
        &[
            ("added", &added.to_string()),
            ("updated", &updated.to_string()),
            ("total", &total.to_string()),
        ],
    ));
}

pub(crate) fn cmd_hand_chat(id: &str) {
    let base = require_daemon("hand chat");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = match resolve_hand_instance(&active, id) {
        Some(instance) => instance,
        None => {
            ui::error(&i18n::t_args("hand-no-active-instance", &[("id", id)]));
            ui::hint(&i18n::t("hand-list-activate-hint"));
            std::process::exit(1);
        }
    };
    let instance_id = resolved["instance_id"]
        .as_str()
        .expect("instance_id missing");
    let hand_id = resolved["hand_id"].as_str().unwrap_or(id);
    let hand_name = resolved["hand_name"]
        .as_str()
        .or_else(|| resolved["name"].as_str())
        .unwrap_or(hand_id);

    install_ctrlc_handler();

    println!(
        "{} {} {}",
        "Chat with".bold(),
        hand_name.cyan().bold(),
        "(type /quit to exit)".dimmed()
    );
    println!();

    loop {
        print!("{} ", "you >".green().bold());
        io::stdout().flush().unwrap();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let msg = line.trim();
        if msg.is_empty() {
            continue;
        }
        if msg == "/quit" || msg == "/exit" || msg == "/q" {
            break;
        }

        let resp = client
            .post(format!("{base}/api/hands/instances/{instance_id}/message"))
            .json(&serde_json::json!({"message": msg}))
            .send();

        let body = daemon_json(resp);
        if let Some(err) = body["error"].as_str() {
            ui::error(err);
            continue;
        }
        let reply = body["response"]
            .as_str()
            .or_else(|| body["reply"].as_str())
            .unwrap_or("[no response]");
        println!("{} {}\n", format!("{hand_name} >").cyan().bold(), reply);
    }
}

pub(crate) fn fetch_active_hand_instances(
    base: &str,
    client: &reqwest::blocking::Client,
) -> Vec<serde_json::Value> {
    let body = daemon_json(client.get(format!("{base}/api/hands/active")).send());
    body.get("instances")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn resolve_hand_instance(
    active_instances: &[serde_json::Value],
    id_or_hand: &str,
) -> Option<serde_json::Value> {
    active_instances
        .iter()
        .find(|instance| {
            instance["instance_id"].as_str() == Some(id_or_hand)
                || instance["hand_id"].as_str() == Some(id_or_hand)
        })
        .cloned()
}
