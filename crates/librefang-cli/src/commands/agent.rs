//! `agent` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

/// A parsed-and-validated agent manifest ready to spawn, with the raw TOML and
/// a human-readable source label for previews. Internal to the agent group.
pub(crate) struct PreparedAgentManifest {
    manifest: AgentManifest,
    manifest_toml: String,
    source_label: String,
}

pub(crate) fn cmd_agent_spawn(
    config: Option<PathBuf>,
    manifest_path: PathBuf,
    name_override: Option<String>,
    dry_run: bool,
) {
    let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
    if dry_run {
        preview_agent_manifest(&prepared);
        return;
    }
    spawn_prepared_agent(config, prepared);
}

pub(crate) fn cmd_spawn_alias(
    config: Option<PathBuf>,
    target: Option<String>,
    template_path: Option<PathBuf>,
    name_override: Option<String>,
    dry_run: bool,
) {
    if template_path.is_some() && target.is_some() {
        ui::error_with_fix(
            &i18n::t("agent-spawn-choose-target-or-template"),
            &i18n::t("agent-spawn-choose-target-or-template-fix"),
        );
        std::process::exit(1);
    }

    if target.is_none() && template_path.is_none() {
        if name_override.is_some() {
            ui::error_with_fix(
                &i18n::t("agent-spawn-name-requires-template"),
                &i18n::t("agent-spawn-name-requires-template-fix"),
            );
            std::process::exit(1);
        }
        if dry_run {
            ui::error_with_fix(
                &i18n::t("agent-spawn-dry-run-requires-template"),
                &i18n::t("agent-spawn-dry-run-requires-template-fix"),
            );
            std::process::exit(1);
        }
        cmd_agent_new(config, None);
        return;
    }

    if let Some(path) = template_path {
        let prepared = prepared_agent_manifest_from_path(&path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let target = target.expect("target checked above");
    let manifest_path = PathBuf::from(&target);
    if manifest_path.exists() {
        let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let templates = templates::load_all_templates();
    let template = templates
        .iter()
        .find(|t| t.name == target)
        .unwrap_or_else(|| {
            ui::error_with_fix(
                &i18n::t_args(
                    "agent-spawn-template-or-path-not-found",
                    &[("target", &target)],
                ),
                &i18n::t("agent-spawn-template-or-path-not-found-fix"),
            );
            std::process::exit(1);
        });
    if dry_run {
        let prepared = prepared_agent_manifest_from_template(template, name_override.as_deref());
        preview_agent_manifest(&prepared);
    } else {
        spawn_template_agent(config, template, name_override.as_deref());
    }
}

pub(crate) fn prepared_agent_manifest_from_path(
    manifest_path: &std::path::Path,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    if !manifest_path.exists() {
        ui::error_with_fix(
            &i18n::t_args(
                "manifest-not-found",
                &[("path", &manifest_path.display().to_string())],
            ),
            &i18n::t("manifest-not-found-fix"),
        );
        std::process::exit(1);
    }

    let contents = std::fs::read_to_string(manifest_path).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args("error-reading-manifest", &[("error", &e.to_string())])
        );
        std::process::exit(1);
    });

    prepared_agent_manifest_from_contents(
        &contents,
        manifest_path.display().to_string(),
        name_override,
    )
}

pub(crate) fn prepared_agent_manifest_from_template(
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    prepared_agent_manifest_from_contents(
        &template.content,
        format!("template:{}", template.name),
        name_override,
    )
}

pub(crate) fn prepared_agent_manifest_from_contents(
    contents: &str,
    source_label: String,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    let mut manifest: AgentManifest = toml::from_str(contents).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args(
                "agent-manifest-parse-failed",
                &[("source", &source_label), ("error", &e.to_string())],
            ),
            &i18n::t("agent-manifest-parse-failed-fix"),
        );
        std::process::exit(1);
    });

    if let Some(name) = name_override {
        manifest.name = name.to_string();
    }

    let manifest_toml = if name_override.is_some() {
        toml::to_string_pretty(&manifest).unwrap_or_else(|e| {
            ui::error(&i18n::t_args(
                "agent-manifest-serialize-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        })
    } else {
        contents.to_string()
    };

    PreparedAgentManifest {
        manifest,
        manifest_toml,
        source_label,
    }
}

pub(crate) fn preview_agent_manifest(prepared: &PreparedAgentManifest) {
    ui::section(&i18n::t("agent-dry-run-title"));
    ui::kv(&i18n::t("label-source"), &prepared.source_label);
    ui::kv(&i18n::t("label-name"), &prepared.manifest.name);
    ui::kv(&i18n::t("label-version"), &prepared.manifest.version);
    ui::kv(&i18n::t("label-module"), &prepared.manifest.module);
    ui::kv(
        &i18n::t("label-model"),
        &format!(
            "{}/{}",
            prepared.manifest.model.provider, prepared.manifest.model.model
        ),
    );
    ui::kv(
        &i18n::t("label-tools"),
        &prepared.manifest.capabilities.tools.len().to_string(),
    );
    ui::kv(
        &i18n::t("label-skills"),
        &prepared.manifest.skills.len().to_string(),
    );
    if !prepared.manifest.tags.is_empty() {
        ui::kv(&i18n::t("label-tags"), &prepared.manifest.tags.join(", "));
    }
    if !prepared.manifest.description.is_empty() {
        ui::kv(
            &i18n::t("label-description"),
            &prepared.manifest.description,
        );
    }
    ui::success(&i18n::t("agent-dry-run-success"));
}

pub(crate) fn spawn_prepared_agent(config: Option<PathBuf>, prepared: PreparedAgentManifest) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if body.get("agent_id").is_some() {
            println!("{}", i18n::t("agent-spawn-success"));
            println!(
                "{}",
                i18n::t_args(
                    "agent-spawn-id-label",
                    &[("id", body["agent_id"].as_str().unwrap_or("?"))]
                )
            );
            println!(
                "{}",
                i18n::t_args(
                    "agent-spawn-name-label",
                    &[(
                        "name",
                        body["name"]
                            .as_str()
                            .unwrap_or(prepared.manifest.name.as_str())
                    )]
                )
            );
        } else {
            let err_fallback = i18n::t("error-unknown");
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", body["error"].as_str().unwrap_or(&err_fallback))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_name = prepared.manifest.name.clone();
        let kernel = boot_kernel(config);
        match kernel.spawn_agent_with_source(prepared.manifest, None) {
            Ok(id) => {
                println!("{}", i18n::t("agent-spawn-inprocess-mode"));
                println!(
                    "{}",
                    i18n::t_args("agent-spawn-id-label", &[("id", &id.to_string())])
                );
                println!(
                    "{}",
                    i18n::t_args("agent-spawn-name-label", &[("name", &agent_name)])
                );
                println!();
                println!("  {}", i18n::t("agent-note-lost"));
                println!("  {}", i18n::t("agent-note-persistent"));
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-spawn-agent-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

pub(crate) fn cmd_agent_list(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/agents")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        let agents = body
            .get("items")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array());

        match agents {
            Some(agents) if agents.is_empty() => println!("{}", i18n::t("agent-no-agents")),
            Some(agents) => {
                // Render via the shared Table builder so column widths
                // self-size to the actual content (instead of hard-coded
                // {:<38} which truncates / over-pads), and so piped output
                // automatically falls back to ASCII (#3306).
                let header_id = i18n::t("label-header-id");
                let header_name = i18n::t("label-header-name");
                let header_state = i18n::t("label-header-state");
                let header_provider = i18n::t("label-header-provider");
                let header_model = i18n::t("label-header-model");
                let mut t = crate::table::Table::new(&[
                    &header_id,
                    &header_name,
                    &header_state,
                    &header_provider,
                    &header_model,
                ]);
                for a in agents {
                    t.add_row(&[
                        a["id"].as_str().unwrap_or("?"),
                        a["name"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["model_provider"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    ]);
                }
                t.print();
            }
            None => println!("{}", i18n::t("agent-no-agents")),
        }
    } else {
        let kernel = boot_kernel(config);
        let agents = kernel.agent_registry_ref().list();

        if json {
            let list: Vec<serde_json::Value> = agents
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&list).unwrap_or_default()
            );
            return;
        }

        if agents.is_empty() {
            println!("{}", i18n::t("agent-no-agents"));
            return;
        }

        let header_id = i18n::t("label-header-id");
        let header_name = i18n::t("label-header-name");
        let header_state = i18n::t("label-header-state");
        let header_created = i18n::t("label-header-created");
        let mut t =
            crate::table::Table::new(&[&header_id, &header_name, &header_state, &header_created]);
        for entry in agents {
            let id = entry.id.to_string();
            let state = format!("{:?}", entry.state);
            let created = entry.created_at.format("%Y-%m-%d %H:%M").to_string();
            t.add_row(&[
                id.as_str(),
                entry.name.as_str(),
                state.as_str(),
                created.as_str(),
            ]);
        }
        t.print();
    }
}

pub(crate) fn cmd_agent_chat(config: Option<PathBuf>, agent_id_str: &str) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, Some(agent_id_str.to_string()));
}

pub(crate) fn cmd_agent_kill(config: Option<PathBuf>, agent_id_str: &str) {
    if let Some(base) = find_daemon() {
        let agent_id = resolve_agent_id(&base, agent_id_str);
        let client = daemon_client();
        // Refs #4614: explicit `librefang agent kill <id>` IS the user's
        // confirmation. The API requires `?confirm=true` on DELETE so the
        // canonical UUID is purged on the kill (matching the issue's
        // "explicit delete" semantics). Internal lifecycle resets call
        // `kernel.kill_agent` directly and skip this path.
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{agent_id}?confirm=true"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("{}", i18n::t_args("agent-killed", &[("id", &agent_id)]));
        } else {
            let err_fallback = i18n::t("error-unknown");
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-kill-failed",
                    &[("error", body["error"].as_str().unwrap_or(&err_fallback))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_id: AgentId = agent_id_str.parse().unwrap_or_else(|_| {
            eprintln!(
                "{}",
                i18n::t_args("agent-invalid-id", &[("id", agent_id_str)])
            );
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        // Direct-kernel path (no daemon): mirror the API's confirmed-delete
        // semantics so behavior matches whether the daemon is running or not.
        match kernel.kill_agent_with_purge(agent_id, true) {
            Ok(()) => println!(
                "{}",
                i18n::t_args("agent-killed", &[("id", &agent_id.to_string())])
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-kill-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent delete <name>` with confirmation prompt.
///
/// Looks up the canonical UUID for `name` via `GET /api/agents/identities`
/// (or directly from the kernel registry when no daemon is running),
/// prints the destructive-action warning, and either prompts `[y/N]` or
/// proceeds immediately when `--yes` is set. Then issues the confirmed
/// DELETE. This is the long-form companion to `librefang agent kill <id>`
/// — useful when the operator only knows the agent's name.
pub(crate) fn cmd_agent_delete(config: Option<PathBuf>, name: &str, yes: bool) {
    eprintln!(
        "{}",
        i18n::t_args("agent-delete-warning-text", &[("name", name)])
    );
    if !yes && !prompt_yes_no(&i18n::t("label-confirm-prompt"), false) {
        eprintln!("{}", i18n::t("label-aborted"));
        std::process::exit(1);
    }

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        // Resolve name → UUID via the identity registry endpoint.
        let canonical_uuid = match lookup_canonical_uuid(&base, name) {
            Some(id) => id,
            None => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-delete-no-uuid", &[("name", name)])
                );
                std::process::exit(1);
            }
        };
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{canonical_uuid}?confirm=true"))
                .send(),
        );
        if body.get("status").is_some() {
            println!(
                "{}",
                i18n::t_args("agent-deleted-success", &[("name", name)])
            );
        } else {
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-delete-failed-with-reason",
                    &[(
                        "error",
                        body["error"].as_str().unwrap_or(&i18n::t("error-unknown"))
                    )]
                )
            );
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        let canonical_uuid = match kernel.identities_ref().get(name) {
            Some(id) => id,
            None => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-delete-no-uuid", &[("name", name)])
                );
                std::process::exit(1);
            }
        };
        match kernel.kill_agent_with_purge(canonical_uuid, true) {
            Ok(()) => println!(
                "{}",
                i18n::t_args("agent-deleted-success", &[("name", name)])
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args(
                        "agent-delete-failed-with-reason",
                        &[("error", &e.to_string())]
                    )
                );
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent reset-uuid <name>` with confirmation.
///
/// Drops the canonical UUID binding without killing a running agent. The
/// next spawn under `name` re-derives a fresh UUID and registers it as
/// the new canonical binding; prior sessions / memories tied to the old
/// UUID are orphaned. `--yes` skips the prompt.
pub(crate) fn cmd_agent_reset_uuid(config: Option<PathBuf>, name: &str, yes: bool) {
    eprintln!(
        "{}",
        i18n::t_args("agent-reset-uuid-warning-text", &[("name", name)])
    );
    if !yes && !prompt_yes_no(&i18n::t("label-confirm-prompt"), false) {
        eprintln!("{}", i18n::t("label-aborted"));
        std::process::exit(1);
    }

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!(
                    "{base}/api/agents/identities/{}/reset",
                    percent_encode_path_segment(name)
                ))
                .query(&[("confirm", "true")])
                .send(),
        );
        if body.get("status").is_some() {
            let prev_fallback = i18n::t("label-unknown");
            let prev = body["previous_canonical_uuid"]
                .as_str()
                .unwrap_or(&prev_fallback);
            println!(
                "{}",
                i18n::t_args(
                    "agent-reset-uuid-success",
                    &[("name", name), ("previous", prev)]
                )
            );
        } else {
            let err_fallback = i18n::t("error-unknown");
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-reset-uuid-failed-with-reason",
                    &[("error", body["error"].as_str().unwrap_or(&err_fallback))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        match kernel.identities_ref().purge(name) {
            Some(prev) => println!(
                "{}",
                i18n::t_args(
                    "agent-reset-uuid-success",
                    &[("name", name), ("previous", &prev.to_string())]
                )
            ),
            None => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-reset-uuid-not-found", &[("name", name)])
                );
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent merge-history` placeholder.
///
/// The cross-table reassignment is not yet implemented — see the
/// long_about on `AgentCommands::MergeHistory` for the rationale (deep
/// memory-substrate surgery across 10+ tables under one transaction).
pub(crate) fn cmd_agent_merge_history(name: &str, from: &str) {
    eprintln!(
        "{}",
        i18n::t_args(
            "agent-merge-history-not-implemented",
            &[("from", from), ("name", name)]
        )
    );
    std::process::exit(2);
}

/// Look up the canonical UUID for `name` via the identity-registry
/// endpoint. Returns `None` if no entry exists (or on any HTTP error —
/// the caller surfaces a friendly message).
pub(crate) fn lookup_canonical_uuid(base: &str, name: &str) -> Option<String> {
    let client = daemon_client();
    let resp = client
        .get(format!("{base}/api/agents/identities"))
        .send()
        .ok()?;
    let entries: serde_json::Value = resp.json().ok()?;
    let arr = entries.as_array()?;
    for entry in arr {
        if entry["name"].as_str() == Some(name) {
            return entry["canonical_uuid"].as_str().map(String::from);
        }
    }
    None
}

pub(crate) fn cmd_agent_set(agent_id_str: &str, field: &str, value: &str) {
    match field {
        "model" => {
            if let Some(base) = find_daemon() {
                let agent_id = resolve_agent_id(&base, agent_id_str);
                let client = daemon_client();
                let body = daemon_json(
                    client
                        .put(format!("{base}/api/agents/{agent_id}/model"))
                        .json(&serde_json::json!({"model": value}))
                        .send(),
                );
                if body.get("status").is_some() {
                    println!(
                        "{}",
                        i18n::t_args(
                            "agent-set-model-success",
                            &[("id", &agent_id), ("value", value)]
                        )
                    );
                } else {
                    let err_fallback = i18n::t("error-unknown");
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "agent-set-model-failed-with-reason",
                            &[("error", body["error"].as_str().unwrap_or(&err_fallback))]
                        )
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("{}", i18n::t("agent-set-no-daemon"));
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "{}",
                i18n::t_args("agent-set-unknown-field", &[("field", field)])
            );
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_agent_new(config: Option<PathBuf>, template_name: Option<String>) {
    let all_templates = templates::load_all_templates();
    if all_templates.is_empty() {
        ui::error_with_fix(
            &i18n::t("agent-new-no-templates"),
            &i18n::t("agent-new-no-templates-fix"),
        );
        std::process::exit(1);
    }

    // Resolve template: by name or interactive picker
    let chosen = match template_name {
        Some(ref name) => match all_templates.iter().find(|t| t.name == *name) {
            Some(t) => t,
            None => {
                ui::error_with_fix(
                    &i18n::t_args("agent-new-template-not-found", &[("name", name)]),
                    &i18n::t("agent-new-template-not-found-fix"),
                );
                std::process::exit(1);
            }
        },
        None => {
            ui::section(&i18n::t("section-agent-templates"));
            ui::blank();
            for (i, t) in all_templates.iter().enumerate() {
                let desc = if t.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", t.description)
                };
                println!(
                    "    {:>2}. {:<22}{}",
                    i + 1,
                    t.name,
                    colored::Colorize::dimmed(desc.as_str())
                );
            }
            ui::blank();
            let choice = prompt_input(&i18n::t("agent-new-choose-template-prompt"));
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(all_templates.len() - 1)
            };
            &all_templates[idx]
        }
    };

    // Spawn the agent
    spawn_template_agent(config, chosen, None);
}

/// Spawn an agent from a template, via daemon or in-process.
pub(crate) fn spawn_template_agent(
    config: Option<PathBuf>,
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) {
    let prepared = prepared_agent_manifest_from_template(template, name_override);
    let agent_name = prepared.manifest.name.clone();

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if let Some(id) = body["agent_id"].as_str() {
            ui::blank();
            ui::success(&i18n::t_args("agent-spawned", &[("name", &agent_name)]));
            ui::kv(&i18n::t("label-id"), id);
            if let Some(model) = body["model_name"].as_str() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                ui::kv(&i18n::t("label-model"), &format!("{provider}/{model}"));
            }
            ui::blank();
            ui::hint(&i18n::t_args(
                "hint-chat-with-agent",
                &[("name", &agent_name)],
            ));
        } else {
            let err_fallback = i18n::t("error-unknown");
            ui::error(&i18n::t_args(
                "agent-spawn-failed",
                &[("error", body["error"].as_str().unwrap_or(&err_fallback))],
            ));
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(prepared.manifest) {
            Ok(id) => {
                ui::blank();
                ui::success(&i18n::t_args(
                    "agent-spawned-inprocess",
                    &[("name", &agent_name)],
                ));
                ui::kv(&i18n::t("label-id"), &id.to_string());
                ui::blank();
                ui::hint(&i18n::t_args(
                    "hint-chat-with-agent",
                    &[("name", &agent_name)],
                ));
                ui::hint(&i18n::t("hint-agent-lost-on-exit"));
                ui::hint(&i18n::t("hint-persistent-agents"));
            }
            Err(e) => {
                ui::error(&i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", &e.to_string())],
                ));
                std::process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Quick chat (OpenClaw alias)
// ---------------------------------------------------------------------------

pub(crate) fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, agent);
}

pub(crate) fn cmd_sessions(agent: Option<&str>, json: bool, active_only: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let url = match agent {
        Some(a) => format!("{base}/api/sessions?agent={a}"),
        None => format!("{base}/api/sessions"),
    };
    let body = daemon_json(client.get(&url).send());

    // Build a (agent_id -> set<session_id>) map of currently-running sessions.
    // Walks the unique agent ids in the listing once and asks the per-agent
    // runtime endpoint added in #3172. Cheap on dev-scale agent counts; if
    // this ever becomes a hotspot we can add a single-call /api/runtime.
    let session_arr_owned: Option<Vec<serde_json::Value>> = body
        .get("sessions")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| body.as_array().cloned());
    let mut active_sessions: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    if let Some(arr) = session_arr_owned.as_ref() {
        let agent_ids: std::collections::HashSet<String> = arr
            .iter()
            .filter_map(|s| s["agent_id"].as_str().map(|id| id.to_string()))
            .collect();
        for aid in agent_ids {
            let runtime_url = format!("{base}/api/agents/{aid}/runtime");
            if let Ok(resp) = client.get(&runtime_url).send() {
                if let Ok(items) = resp.json::<Vec<serde_json::Value>>() {
                    let sids: std::collections::HashSet<String> = items
                        .iter()
                        .filter_map(|v| v["session_id"].as_str().map(|s| s.to_string()))
                        .collect();
                    active_sessions.insert(aid, sids);
                }
            }
        }
    }

    let is_running = |s: &serde_json::Value| -> bool {
        let aid = match s["agent_id"].as_str() {
            Some(a) => a,
            None => return false,
        };
        let sid = match s["session_id"].as_str().or_else(|| s["id"].as_str()) {
            Some(s) => s,
            None => return false,
        };
        active_sessions
            .get(aid)
            .is_some_and(|set| set.contains(sid))
    };

    if json {
        // Annotate each session with `state` so JSON consumers see the same
        // signal as the table renderer.
        if let Some(arr) = session_arr_owned.as_ref() {
            let annotated: Vec<serde_json::Value> = arr
                .iter()
                .filter(|s| !active_only || is_running(s))
                .map(|s| {
                    let mut out = s.clone();
                    out["state"] = serde_json::Value::String(
                        if is_running(s) { "running" } else { "idle" }.into(),
                    );
                    out
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&annotated).unwrap_or_default()
            );
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
        return;
    }
    if let Some(arr) = session_arr_owned.as_ref() {
        let filtered: Vec<&serde_json::Value> = arr
            .iter()
            .filter(|s| !active_only || is_running(s))
            .collect();
        if filtered.is_empty() {
            if active_only {
                println!("{}", i18n::t("agent-sessions-none-active"));
            } else {
                println!("{}", i18n::t("agent-sessions-none-found"));
            }
            return;
        }
        let header_id = i18n::t("label-header-id");
        let header_agent = i18n::t("label-header-agent");
        let header_msgs = i18n::t("label-header-msgs");
        let header_state = i18n::t("label-header-state");
        let header_last_active = i18n::t("label-header-last-active");
        let mut t = crate::table::Table::new(&[
            &header_id,
            &header_agent,
            &header_msgs,
            &header_state,
            &header_last_active,
        ]);
        for s in filtered {
            let state_str = if is_running(s) {
                i18n::t("label-session-state-running")
            } else {
                i18n::t("label-session-state-idle")
            };
            let agent_id = s["agent_id"].as_str().unwrap_or("");
            let agent_col = if agent_id.len() > 16 {
                &agent_id[..16]
            } else if agent_id.is_empty() {
                s["agent_name"].as_str().unwrap_or("?")
            } else {
                agent_id
            };
            t.add_row(&[
                s["session_id"]
                    .as_str()
                    .or_else(|| s["id"].as_str())
                    .unwrap_or("?"),
                agent_col,
                &s["message_count"].as_u64().unwrap_or(0).to_string(),
                &state_str,
                s["created_at"]
                    .as_str()
                    .or_else(|| s["last_active"].as_str())
                    .unwrap_or("?"),
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

pub(crate) fn cmd_message(agent: &str, text: &str, json: bool, incognito: bool) {
    let base = require_daemon("message");
    let agent_id = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/message"))
            .json(&serde_json::json!({"message": text, "incognito": incognito}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}
