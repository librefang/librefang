//! `channel` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Channel commands (sidecar-aware). Replace the pre-#5463 in-process
// wizards: every channel now runs out-of-process, configuration goes
// through the surviving daemon endpoints (GET /api/channels for the
// list, GET /api/channels/registry + POST /api/channels/sidecar/{name}/
// configure for setup, POST /api/channels/reload to apply, plus a local
// `rm` that strips a [[sidecar_channels]] entry from config.toml).
// ---------------------------------------------------------------------------

pub(crate) fn cmd_channel_list() {
    let base = require_daemon("channel list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/channels")).send());
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        println!("No channels configured.");
        println!("Use `librefang channel setup` to add one.");
        return;
    }
    let mut t = crate::table::Table::new(&["NAME", "KIND", "CONFIGURED", "TOKEN", "24H MSGS"]);
    for ch in &items {
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = ch.get("category").and_then(|v| v.as_str()).unwrap_or("?");
        let configured = ch
            .get("configured")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_token = ch
            .get("has_token")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let msgs = ch.get("msgs_24h").and_then(|v| v.as_u64()).unwrap_or(0);
        t.add_row(&[
            name,
            kind,
            if configured { "yes" } else { "no" },
            if has_token { "yes" } else { "no" },
            &msgs.to_string(),
        ]);
    }
    t.print();
}

pub(crate) fn cmd_channel_reload() {
    let base = require_daemon("channel reload");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/channels/reload"))
            .json(&serde_json::json!({}))
            .send(),
    );
    let started = body
        .get("started")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    println!("Channels reloaded ({started} sidecar(s) started).");
}

pub(crate) fn cmd_channel_setup(name: Option<&str>) {
    let base = require_daemon("channel setup");
    let client = daemon_client();
    // `GET /api/channels` carries the full sidecar describe schema for
    // every discoverable adapter on `fields[]`, so we don't need a
    // separate /registry call for the picker — same list does both
    // jobs.
    let body = daemon_json(client.get(format!("{base}/api/channels")).send());
    let all: Vec<serde_json::Value> = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Resolve the target row: explicit `<NAME>` argument, or interactive
    // picker over unconfigured rows.
    let target = match name {
        Some(n) => all
            .iter()
            .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(n))
            .cloned(),
        None => {
            // Distinguish the two empty-picker cases so the operator
            // knows which is which:
            //  - `all.is_empty()`: daemon's `GET /api/channels` returned
            //    nothing at all — both `sidecar_channel_rows` and
            //    `sidecar_discovery_rows` are empty. That means there
            //    are no `[[sidecar_channels]]` entries AND nothing in
            //    the SIDECAR_CATALOG (the latter is normally only
            //    empty if the SDK wasn't installed alongside the
            //    daemon — fix is `pip install librefang-sdk`).
            //  - all non-empty but `candidates.is_empty()`: the
            //    operator has configured every adapter the catalog
            //    knows about. Use `librefang channel list` to see /
            //    `librefang channel rm <name>` to drop one.
            if all.is_empty() {
                println!("Daemon's channel registry is empty.");
                println!("Install the sidecar SDK so adapters appear in the catalog:");
                println!("  pip install librefang-sdk");
                println!("Then re-run `librefang channel setup`.");
                return;
            }
            let candidates: Vec<&serde_json::Value> = all
                .iter()
                .filter(|c| c.get("configured").and_then(|v| v.as_bool()) != Some(true))
                .collect();
            if candidates.is_empty() {
                println!("Every available channel is already configured.");
                println!("Use `librefang channel list` to see them, or");
                println!("`librefang channel rm <name>` to remove an entry first.");
                return;
            }
            println!("Pick a channel to set up:");
            for (i, ch) in candidates.iter().enumerate() {
                let n = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let d = ch.get("display_name").and_then(|v| v.as_str()).unwrap_or(n);
                println!("  {:>2}. {:<14} {}", i + 1, n, d);
            }
            let choice = prompt_input("Choice [1]: ");
            let idx = if choice.trim().is_empty() {
                0
            } else {
                choice
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(candidates.len() - 1)
            };
            Some(candidates[idx].clone())
        }
    };
    let target = match target {
        Some(t) => t,
        None => {
            ui::error_with_fix(
                &format!("Unknown channel: {}", name.unwrap_or("?")),
                "Run `librefang channel list` to see the available adapters.",
            );
            std::process::exit(1);
        }
    };
    let chan_name = target
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fields: Vec<serde_json::Value> = target
        .get("fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if fields.is_empty() {
        println!("`{chan_name}` exposes no configurable fields — nothing to prompt for.");
        println!("(Hot-reload anyway with `librefang channel reload` if you've already edited config.toml by hand.)");
        return;
    }

    let mut values = serde_json::Map::new();
    for f in &fields {
        let key = f.get("key").and_then(|v| v.as_str()).unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        let label = f.get("label").and_then(|v| v.as_str()).unwrap_or(key);
        let required = f.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
        let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        let has_value = f
            .get("has_value")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let current = f.get("value").and_then(|v| v.as_str()).unwrap_or("");

        // Secret-typed + has_value=true: blank means "keep existing".
        // Non-secret + has current value: show as default-in-brackets.
        let prompt = if ftype == "secret" && has_value {
            format!("  {label} ({key}) [set — leave blank to keep]: ")
        } else if !current.is_empty() {
            format!("  {label} ({key}) [{current}]: ")
        } else if required {
            format!("  {label} ({key}) *: ")
        } else {
            format!("  {label} ({key}): ")
        };
        let entered = prompt_input(&prompt);
        let val = entered.trim();
        if val.is_empty() {
            continue;
        }
        values.insert(key.to_string(), serde_json::Value::String(val.to_string()));
    }

    // Sidecar names come from `SIDECAR_CATALOG` keys — short
    // alphanumeric (`telegram`, `ntfy`, …), URL-safe as-is. No need
    // for percent-encoding.
    let url = format!("{base}/api/channels/sidecar/{chan_name}/configure");
    let payload = serde_json::json!({ "values": values });
    let body = daemon_json(client.post(&url).json(&payload).send());
    // `daemon_json` only logs 5xx; 4xx silently returns the error body.
    // Surface those by checking for the SidecarSaveResult shape. The
    // `ApiErrorResponse` envelope (see librefang-api types.rs:114-164)
    // serializes the human-readable message at both `error.message`
    // (nested, #3639 preferred shape) and `message` (top-level flat
    // alias kept for legacy callers); prefer the nested one, fall
    // through to the flat alias for older deployments.
    if body.get("status").and_then(|v| v.as_str()) != Some("saved") {
        let err = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("message").and_then(|v| v.as_str()))
            .unwrap_or("save failed (no error body)");
        ui::error_with_fix(
            &format!("Save for `{chan_name}` rejected: {err}"),
            "Re-run with corrected values, or check the daemon log for details.",
        );
        std::process::exit(1);
    }
    let restart_required = body
        .get("restart_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let shadowed = body
        .get("shadowed_secrets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if restart_required {
        println!("✓ Saved `{chan_name}` — restart the daemon for changes to apply.");
    } else {
        println!("✓ Saved `{chan_name}` — hot-reload applied.");
    }
    if !shadowed.is_empty() {
        let keys: Vec<String> = shadowed
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        eprintln!(
            "Warning: shell environment variables shadow these tokens — unset them and restart for the new value to take effect: {}",
            keys.join(", "),
        );
    }
}

pub(crate) fn cmd_channel_rm(name: &str) {
    // Strip the matching `[[sidecar_channels]]` entry from
    // ~/.librefang/config.toml in-place, then trigger a daemon reload
    // (best-effort: if no daemon is running, the file edit is enough
    // — the next daemon start will pick up the changed config).
    let home = cli_librefang_home();
    let path = home.join("config.toml");
    let original = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            ui::error_with_fix(
                &format!("Cannot read {}: {e}", path.display()),
                "Run `librefang init` to create the config file.",
            );
            std::process::exit(1);
        }
    };
    let mut doc: toml_edit::DocumentMut = match original.parse() {
        Ok(d) => d,
        Err(e) => {
            ui::error_with_fix(
                &format!("Cannot parse {}: {e}", path.display()),
                "Fix the TOML syntax and retry.",
            );
            std::process::exit(1);
        }
    };
    let arr = match doc
        .get_mut("sidecar_channels")
        .and_then(|v| v.as_array_of_tables_mut())
    {
        Some(a) => a,
        None => {
            println!("No [[sidecar_channels]] entries in config.toml — nothing to remove.");
            return;
        }
    };
    // `toml_edit::ArrayOfTables` has no `retain`; collect matching indices
    // then remove in reverse so earlier indices stay stable.
    let to_remove: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t.get("name").and_then(|v| v.as_str()) {
            Some(n) if n == name => Some(i),
            _ => None,
        })
        .collect();
    let removed = to_remove.len();
    for &i in to_remove.iter().rev() {
        arr.remove(i);
    }
    if removed == 0 {
        println!("No [[sidecar_channels]] entry with name=\"{name}\".");
        return;
    }
    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        ui::error_with_fix(
            &format!("Failed to write {}: {e}", path.display()),
            "Check filesystem permissions.",
        );
        std::process::exit(1);
    }
    println!("✓ Removed {removed} [[sidecar_channels]] entry/entries named `{name}`.");
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client
                .post(format!("{base}/api/channels/reload"))
                .json(&serde_json::json!({}))
                .send()
            {
                Ok(r) if r.status().is_success() => println!("  Hot-reloaded daemon."),
                Ok(r) => eprintln!(
                    "  Reload returned {}: change will apply on next daemon restart.",
                    r.status()
                ),
                Err(e) => eprintln!(
                    "  Could not contact daemon for reload ({e}); change will apply on next start."
                ),
            }
        }
        None => println!("  Daemon not running; change will apply on next start."),
    }
}
