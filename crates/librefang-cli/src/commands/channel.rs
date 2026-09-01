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
        println!("{}", i18n::t("channel-none-configured"));
        println!("{}", i18n::t("channel-use-setup-hint"));
        return;
    }
    let yes_str = i18n::t("label-yes");
    let no_str = i18n::t("label-no");
    let not_started_str = i18n::t("channel-liveness-not-started");
    let name_header = i18n::t("label-header-name");
    let kind_header = i18n::t("label-header-kind");
    let conf_header = i18n::t("label-header-configured");
    let token_header = i18n::t("label-header-token");
    let connected_header = i18n::t("channel-header-connected");
    let in_out_header = i18n::t("channel-header-in-out");
    let msgs_header = i18n::t("channel-header-msgs-24h-by-type");
    let mut t = crate::table::Table::new(&[
        &name_header,
        &kind_header,
        &conf_header,
        &token_header,
        &connected_header,
        &in_out_header,
        &msgs_header,
    ]);
    // Sticky per-instance errors are collected and printed under the table
    // rather than squeezed into a column — the supervisor's message is a full
    // sentence (spawn failure, circuit-break cause) and truncating it to a
    // cell width would drop the actionable part.
    let mut errors: Vec<(String, String)> = Vec::new();
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
        // Raw supervisor facts, not a derived health verdict — the state
        // mapping lives in one place (the dashboard) so the two surfaces
        // cannot drift into disagreeing about what "healthy" means (#6606).
        // `supervised` distinguishes "no adapter registered for this name"
        // from "adapter registered but its child is down".
        let supervised = ch
            .get("supervised")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let connected = ch
            .get("connected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let received = ch
            .get("messages_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let sent = ch
            .get("messages_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if let Some(err) = ch.get("last_error").and_then(|v| v.as_str()) {
            errors.push((name.to_string(), err.to_string()));
        }
        // Per channel TYPE, shared by every sidecar instance of that type;
        // per-instance traffic is the IN/OUT column above.
        let msgs = ch
            .get("msgs_24h_channel_type")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // Unconfigured catalog rows describe an adapter nobody has wired up,
        // so they have no instance to be alive or dead — leave their liveness
        // and traffic cells blank rather than reporting a definite "no".
        let dash = "-";
        let connected_cell: &str = if !configured {
            dash
        } else if !supervised {
            &not_started_str
        } else if connected {
            &yes_str
        } else {
            &no_str
        };
        let in_out_cell = if configured {
            format!("{received}/{sent}")
        } else {
            dash.to_string()
        };
        let msgs_cell = if configured {
            msgs.to_string()
        } else {
            dash.to_string()
        };
        t.add_row(&[
            name,
            kind,
            if configured { &yes_str } else { &no_str },
            if has_token { &yes_str } else { &no_str },
            connected_cell,
            &in_out_cell,
            &msgs_cell,
        ]);
    }
    t.print();
    if !errors.is_empty() {
        println!();
        println!("{}", i18n::t("channel-last-errors-heading"));
        for (name, err) in &errors {
            println!(
                "  {}",
                i18n::t_args(
                    "channel-last-error-entry",
                    &[("name", name.as_str()), ("error", err.as_str())]
                )
            );
        }
    }
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
    println!(
        "{}",
        i18n::t_args("channel-reloaded", &[("started", &started.to_string())])
    );
}

/// The adapter, instance name and current agent binding a `channel setup`
/// target implies.
///
/// `GET /api/channels` mixes two row shapes under one `items` array and reuses
/// `name` for two different things: on a configured row it is the
/// `[[sidecar_channels]].name` the operator chose, on a catalog row it is the
/// adapter key. `POST /api/channels/sidecar/{name}/configure` accepts only the
/// **adapter** in its path — it looks the segment up in the daemon's sidecar
/// catalog — so passing a configured row's name there 404s for every instance
/// whose name is not also an adapter key (`librefang channel setup slack-hr`).
/// That is the same identifier mix-up reported in #8055 / #8063.
///
/// The third element is the row's existing `agent`. It has to be sent back
/// because the endpoint treats an absent `agent` as "clear the binding"
/// (`write_form_managed` calls `block.remove("agent")` for `None`), so a setup
/// that omitted it would silently unbind an instance's default agent.
///
/// Returns `(adapter, instance_name, agent)`, where `instance_name` is `None`
/// when it coincides with the adapter — a first instance named after its own
/// adapter — matching the endpoint's own default for the field.
pub(crate) fn setup_target_identifiers(
    row: &serde_json::Value,
) -> (String, Option<String>, Option<String>) {
    let name = row
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    // A catalog row carries no `channel_type`, and a configured entry that
    // omits the key is treated by the daemon as having its name for a type.
    let adapter = row
        .get("channel_type")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or(name.as_str())
        .to_string();
    let instance_name = if adapter == name { None } else { Some(name) };
    let agent = row
        .get("agent")
        .and_then(|v| v.as_str())
        .filter(|a| !a.is_empty())
        .map(str::to_string);
    (adapter, instance_name, agent)
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
                println!("{}", i18n::t("channel-registry-empty"));
                println!("{}", i18n::t("channel-install-sdk-hint"));
                println!("{}", i18n::t("channel-install-sdk-cmd"));
                println!("{}", i18n::t("channel-rerun-setup-hint"));
                return;
            }
            let candidates: Vec<&serde_json::Value> = all
                .iter()
                .filter(|c| c.get("configured").and_then(|v| v.as_bool()) != Some(true))
                .collect();
            if candidates.is_empty() {
                println!("{}", i18n::t("channel-all-configured"));
                println!("{}", i18n::t("channel-see-list-hint"));
                println!("{}", i18n::t("channel-remove-entry-hint"));
                return;
            }
            println!("{}", i18n::t("channel-pick-setup"));
            for (i, ch) in candidates.iter().enumerate() {
                let n = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let d = ch.get("display_name").and_then(|v| v.as_str()).unwrap_or(n);
                println!("  {:>2}. {:<14} {}", i + 1, n, d);
            }
            let choice = prompt_input(&i18n::t("channel-choice-prompt"));
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
                &i18n::t_args("channel-unknown-error", &[("name", name.unwrap_or("?"))]),
                &i18n::t("channel-unknown-error-fix"),
            );
            std::process::exit(1);
        }
    };
    let chan_name = target
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // `chan_name` stays the operator-facing label; the request needs the
    // adapter for its path and the instance name for its body.
    let (adapter, instance_name, current_agent) = setup_target_identifiers(&target);
    let fields: Vec<serde_json::Value> = target
        .get("fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if fields.is_empty() {
        println!(
            "{}",
            i18n::t_args("channel-no-configurable-fields", &[("name", &chan_name)])
        );
        println!("{}", i18n::t("channel-hot-reload-manual-hint"));
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

        // Secret-typed + has_value=true: blank means "keep existing" — but
        // only for an optional field. The endpoint validates every `required`
        // schema field against the payload it receives and cannot read a
        // stored secret back, so omitting a required one is a 400, not a
        // no-op. Prompt for it as required instead of promising a keep that
        // the daemon will reject.
        let prompt = if ftype == "secret" && has_value && required {
            i18n::t_args(
                "channel-prompt-secret-retype",
                &[("label", label), ("key", key)],
            )
        } else if ftype == "secret" && has_value {
            i18n::t_args(
                "channel-prompt-secret-keep",
                &[("label", label), ("key", key)],
            )
        } else if !current.is_empty() {
            i18n::t_args(
                "channel-prompt-default",
                &[("label", label), ("key", key), ("current", current)],
            )
        } else if required {
            i18n::t_args("channel-prompt-required", &[("label", label), ("key", key)])
        } else {
            i18n::t_args("channel-prompt-optional", &[("label", label), ("key", key)])
        };
        let entered = prompt_input(&prompt);
        let val = entered.trim();
        if val.is_empty() {
            // `channel-prompt-default` shows the stored value in brackets, so a
            // bare Enter reads as "keep it" — but an omitted key is a removal,
            // not a no-op: `write_form_managed` drops every managed env key
            // absent from `values`. Resend the current value so accepting the
            // default keeps it instead of deleting it (and, for a required
            // field, instead of taking a 400 the operator did nothing to earn).
            // Secrets carry no readable current value, so there is nothing to
            // resend and blank keeps meaning "leave secrets.env alone".
            if ftype != "secret" && !current.is_empty() {
                values.insert(
                    key.to_string(),
                    serde_json::Value::String(current.to_string()),
                );
            }
            continue;
        }
        values.insert(key.to_string(), serde_json::Value::String(val.to_string()));
    }

    // Sidecar adapter names come from `SIDECAR_CATALOG` keys — short
    // alphanumeric (`telegram`, `ntfy`, …), URL-safe as-is. No need
    // for percent-encoding.
    let url = format!("{base}/api/channels/sidecar/{adapter}/configure");
    let payload = serde_json::json!({
        "values": values,
        "instance_name": instance_name,
        "agent": current_agent,
    });
    let body = daemon_json(client.post(&url).json(&payload).send());
    // `daemon_json` only logs 5xx; 4xx silently returns the error body.
    // Surface those by checking for the SidecarSaveResult shape. The
    // `ApiErrorResponse` envelope (see librefang-api types.rs:114-164)
    // serializes the human-readable message at both `error.message`
    // (nested, #3639 preferred shape) and `message` (top-level flat
    // alias kept for legacy callers); prefer the nested one, fall
    // through to the flat alias for older deployments.
    if body.get("status").and_then(|v| v.as_str()) != Some("saved") {
        let fallback_err = i18n::t("channel-error-save-failed-no-body");
        let err = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("message").and_then(|v| v.as_str()))
            .unwrap_or(&fallback_err);
        ui::error_with_fix(
            &i18n::t_args(
                "channel-save-rejected",
                &[("name", &chan_name), ("error", err)],
            ),
            &i18n::t("channel-save-rejected-fix"),
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
        println!(
            "{}",
            i18n::t_args("channel-saved-restart-required", &[("name", &chan_name)])
        );
    } else {
        println!(
            "{}",
            i18n::t_args("channel-saved-hot-reload", &[("name", &chan_name)])
        );
    }
    if !shadowed.is_empty() {
        let keys: Vec<String> = shadowed
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        eprintln!(
            "{}",
            i18n::t_args("channel-env-shadowing-warn", &[("keys", &keys.join(", "))])
        );
    }
}

pub(crate) fn cmd_channel_rm(name: &str) {
    // Strip the matching `[[sidecar_channels]]` entry from
    // ~/.librefang/config.toml in-place, then trigger a daemon reload
    // (best-effort: if no daemon is running, the file edit is enough
    // — the next daemon start will pick up the changed config).
    let path = cli_config_path();
    let original = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            ui::error_with_fix(
                &i18n::t_args(
                    "channel-config-read-fail",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
                &i18n::t("channel-config-read-fail-fix"),
            );
            std::process::exit(1);
        }
    };
    let mut doc: toml_edit::DocumentMut = match original.parse() {
        Ok(d) => d,
        Err(e) => {
            ui::error_with_fix(
                &i18n::t_args(
                    "channel-config-parse-fail",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
                &i18n::t("channel-config-parse-fail-fix"),
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
            println!("{}", i18n::t("channel-no-entries-to-remove"));
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
        println!(
            "{}",
            i18n::t_args("channel-no-entry-with-name", &[("name", name)])
        );
        return;
    }
    if let Err(e) = durable_atomic_write(&path, doc.to_string().as_bytes(), 0o600) {
        ui::error_with_fix(
            &i18n::t_args(
                "channel-config-write-fail",
                &[
                    ("path", &path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ),
            &i18n::t("channel-config-write-fail-fix"),
        );
        std::process::exit(1);
    }
    println!(
        "{}",
        i18n::t_args(
            "channel-removed-entries",
            &[("count", &removed.to_string()), ("name", name)]
        )
    );
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client
                .post(format!("{base}/api/channels/reload"))
                .json(&serde_json::json!({}))
                .send()
            {
                Ok(r) if r.status().is_success() => {
                    println!("{}", i18n::t("channel-hot-reloaded-daemon"));
                }
                Ok(r) => {
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "channel-reload-status-warn",
                            &[("status", &r.status().to_string())]
                        )
                    );
                }
                Err(e) => {
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "channel-reload-contact-fail-warn",
                            &[("error", &e.to_string())]
                        )
                    );
                }
            }
        }
        None => {
            println!("{}", i18n::t("channel-reload-daemon-offline"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::setup_target_identifiers;
    use serde_json::json;

    /// A catalog row's `name` *is* the adapter, so nothing is redirected.
    #[test]
    fn a_catalog_row_configures_its_own_adapter() {
        let row = json!({ "name": "telegram", "configured": false });
        let (adapter, instance, agent) = setup_target_identifiers(&row);
        assert_eq!(adapter, "telegram");
        assert_eq!(
            instance, None,
            "the default instance needs no explicit name"
        );
        assert_eq!(agent, None);
    }

    /// The regression this helper exists for: a second instance's name is not
    /// a catalog key, so it must not reach the configure path.
    #[test]
    fn a_named_instance_sends_its_adapter_in_the_path() {
        let row = json!({
            "name": "slack-hr",
            "channel_type": "slack",
            "configured": true,
            "agent": "hr-bot",
        });
        let (adapter, instance, agent) = setup_target_identifiers(&row);
        assert_eq!(
            adapter, "slack",
            "the path segment is looked up in the sidecar catalog"
        );
        assert_eq!(instance.as_deref(), Some("slack-hr"));
        assert_eq!(
            agent.as_deref(),
            Some("hr-bot"),
            "an omitted agent would clear the binding, so it is sent back"
        );
    }

    /// A configured entry may omit `channel_type`; the daemon then reads its
    /// name as the type, and so must we.
    #[test]
    fn a_configured_entry_without_a_channel_type_falls_back_to_its_name() {
        let row = json!({ "name": "ntfy", "configured": true });
        let (adapter, instance, _) = setup_target_identifiers(&row);
        assert_eq!(adapter, "ntfy");
        assert_eq!(instance, None);
    }

    #[test]
    fn a_blank_channel_type_is_treated_as_absent() {
        let row = json!({ "name": "ntfy", "channel_type": "", "configured": true });
        let (adapter, _, _) = setup_target_identifiers(&row);
        assert_eq!(adapter, "ntfy");
    }

    #[test]
    fn a_blank_agent_is_reported_as_no_binding() {
        let row = json!({ "name": "telegram", "agent": "" });
        let (_, _, agent) = setup_target_identifiers(&row);
        assert_eq!(agent, None);
    }
}
