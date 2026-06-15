//! `automation` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Workflow commands
// ---------------------------------------------------------------------------

pub(crate) fn cmd_workflow_list() {
    let base = require_daemon("workflow list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/workflows")).send());

    match body.as_array() {
        Some(workflows) if workflows.is_empty() => println!("No workflows registered."),
        Some(workflows) => {
            let mut t = crate::table::Table::new(&["ID", "NAME", "STEPS", "CREATED"]);
            for w in workflows {
                t.add_row(&[
                    w["id"].as_str().unwrap_or("?"),
                    w["name"].as_str().unwrap_or("?"),
                    &w["steps"].as_u64().unwrap_or(0).to_string(),
                    w["created_at"].as_str().unwrap_or("?"),
                ]);
            }
            t.print();
        }
        None => println!("No workflows registered."),
    }
}

pub(crate) fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    if !file.exists() {
        eprintln!("Workflow file not found: {}", file.display());
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("Error reading workflow file: {e}");
        std::process::exit(1);
    });
    let json_body: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("Invalid JSON: {e}");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows"))
            .json(&json_body)
            .send(),
    );

    if let Some(id) = body["workflow_id"].as_str() {
        println!("Workflow created successfully!");
        println!("  ID: {id}");
    } else {
        eprintln!(
            "Failed to create workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows/{workflow_id}/run"))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    if let Some(output) = body["output"].as_str() {
        println!("Workflow completed!");
        println!("  Run ID: {}", body["run_id"].as_str().unwrap_or("?"));
        println!("  Output:\n{output}");
    } else {
        eprintln!(
            "Workflow failed: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Trigger commands
// ---------------------------------------------------------------------------

pub(crate) fn cmd_trigger_list(agent_id: Option<&str>) {
    let base = require_daemon("trigger list");
    let client = daemon_client();

    let url = match agent_id {
        Some(id) => format!("{base}/api/triggers?agent_id={id}"),
        None => format!("{base}/api/triggers"),
    };
    let body = daemon_json(client.get(&url).send());

    let arr = body["triggers"].as_array().or_else(|| body.as_array());
    match arr {
        Some(triggers) if triggers.is_empty() => println!("No triggers registered."),
        Some(triggers) => {
            let mut tbl = crate::table::Table::new(&[
                "TRIGGER ID",
                "AGENT ID",
                "ENABLED",
                "FIRES",
                "PATTERN",
            ]);
            for t in triggers {
                tbl.add_row(&[
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    &t["enabled"].as_bool().unwrap_or(false).to_string(),
                    &t["fire_count"].as_u64().unwrap_or(0).to_string(),
                    t["pattern"].as_str().unwrap_or("?"),
                ]);
            }
            tbl.print();
        }
        None => println!("No triggers registered."),
    }
}

pub(crate) fn cmd_trigger_create(
    agent_id: &str,
    pattern_json: &str,
    prompt: &str,
    max_fires: u64,
    target_agent: Option<&str>,
    cooldown: Option<u64>,
    session_mode: Option<&str>,
) {
    let base = require_daemon("trigger create");
    let agent_id = resolve_agent_id(&base, agent_id);
    let pattern: serde_json::Value = serde_json::from_str(pattern_json).unwrap_or_else(|e| {
        eprintln!("Invalid pattern JSON: {e}");
        eprintln!("Examples:");
        eprintln!("  '\"lifecycle\"'");
        eprintln!("  '{{\"agent_spawned\":{{\"name_pattern\":\"*\"}}}}'");
        eprintln!("  '\"agent_terminated\"'");
        eprintln!("  '\"all\"'");
        std::process::exit(1);
    });

    let mut payload = serde_json::json!({
        "agent_id": agent_id,
        "pattern": pattern,
        "prompt_template": prompt,
        "max_fires": max_fires,
    });
    if let Some(t) = target_agent {
        payload["target_agent_id"] = serde_json::json!(t);
    }
    if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&payload)
            .send(),
    );

    if let Some(id) = body["trigger_id"].as_str() {
        println!("Trigger created successfully!");
        println!("  Trigger ID: {id}");
        println!("  Agent ID:   {agent_id}");
        if let Some(t) = target_agent {
            println!("  Target:     {t}");
        }
    } else {
        eprintln!(
            "Failed to create trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_trigger_delete(trigger_id: &str) {
    let base = require_daemon("trigger delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("status").is_some() {
        println!("Trigger {trigger_id} deleted.");
    } else {
        eprintln!(
            "Failed to delete trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_trigger_get(trigger_id: &str) {
    let base = require_daemon("trigger get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to get trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }

    println!("Trigger ID:    {}", body["id"].as_str().unwrap_or("-"));
    println!(
        "Agent ID:      {}",
        body["agent_id"].as_str().unwrap_or("-")
    );
    println!("Pattern:       {}", body["pattern"]);
    println!(
        "Prompt:        {}",
        body["prompt_template"].as_str().unwrap_or("-")
    );
    println!(
        "Enabled:       {}",
        body["enabled"].as_bool().unwrap_or(false)
    );
    println!(
        "Fire count:    {}",
        body["fire_count"].as_u64().unwrap_or(0)
    );
    println!(
        "Max fires:     {}",
        body["max_fires"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unlimited".to_string())
    );
    if let Some(t) = body["target_agent_id"].as_str() {
        println!("Target agent:  {t}");
    }
    if let Some(c) = body["cooldown_secs"].as_u64() {
        println!("Cooldown:      {c}s");
    }
    if let Some(m) = body["session_mode"].as_str() {
        println!("Session mode:  {m}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_trigger_update(
    trigger_id: &str,
    pattern: Option<&str>,
    prompt: Option<&str>,
    enabled: Option<bool>,
    max_fires: Option<u64>,
    cooldown: Option<u64>,
    clear_cooldown: bool,
    session_mode: Option<&str>,
    clear_session_mode: bool,
    target_agent: Option<&str>,
    clear_target_agent: bool,
) {
    let base = require_daemon("trigger update");
    let client = daemon_client();

    let mut payload = serde_json::json!({});
    if let Some(p) = pattern {
        let parsed: serde_json::Value = serde_json::from_str(p).unwrap_or_else(|e| {
            eprintln!("Invalid pattern JSON: {e}");
            std::process::exit(1);
        });
        payload["pattern"] = parsed;
    }
    if let Some(t) = prompt {
        payload["prompt_template"] = serde_json::json!(t);
    }
    if let Some(e) = enabled {
        payload["enabled"] = serde_json::json!(e);
    }
    if let Some(m) = max_fires {
        payload["max_fires"] = serde_json::json!(m);
    }
    if clear_cooldown {
        payload["cooldown_secs"] = serde_json::Value::Null;
    } else if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if clear_session_mode {
        payload["session_mode"] = serde_json::Value::Null;
    } else if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }
    if clear_target_agent {
        payload["target_agent_id"] = serde_json::Value::Null;
    } else if let Some(a) = target_agent {
        payload["target_agent_id"] = serde_json::json!(a);
    }

    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to update trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!("Trigger {trigger_id} updated.");
}

pub(crate) fn cmd_trigger_set_enabled(trigger_id: &str, enabled: bool) {
    let base = require_daemon(if enabled {
        "trigger enable"
    } else {
        "trigger disable"
    });
    let client = daemon_client();
    let payload = serde_json::json!({ "enabled": enabled });
    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to {} trigger: {}",
            if enabled { "enable" } else { "disable" },
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!(
        "Trigger {trigger_id} {}.",
        if enabled { "enabled" } else { "disabled" }
    );
}

pub(crate) fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("jobs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "AGENT", "SCHEDULE", "ENABLED", "PROMPT"]);
        for j in arr {
            t.add_row(&[
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["schedule"]["expr"]
                    .as_str()
                    .or_else(|| j["cron_expr"].as_str())
                    .unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                &j["action"]["message"]
                    .as_str()
                    .or_else(|| j["prompt"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(40)
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

pub(crate) fn cmd_cron_create(agent: &str, spec: &str, prompt: &str, explicit_name: Option<&str>) {
    let base = require_daemon("cron create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Use explicit name if provided, otherwise derive from agent + prompt
    let name = if let Some(n) = explicit_name {
        n.to_string()
    } else {
        let short_prompt: String = prompt
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        format!(
            "{}-{}",
            agent,
            if short_prompt.is_empty() {
                "job"
            } else {
                &short_prompt
            }
        )
    };

    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "name": name,
                "schedule": {
                    "kind": "cron",
                    "expr": spec
                },
                "action": {
                    "kind": "agent_turn",
                    "message": prompt
                }
            }))
            .send(),
    );
    if let Some(id) = body["job_id"].as_str().or_else(|| body["id"].as_str()) {
        ui::success(&i18n::t_args("cron-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "cron-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

pub(crate) fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("cron-deleted", &[("id", id)]));
    }
}

pub(crate) fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    // The daemon exposes a single `PUT /api/cron/jobs/{id}/enable` route that
    // toggles in either direction via the `enabled` bool in the request body —
    // there is no `/disable` route. `endpoint` is only the action label used in
    // user-facing messages below.
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .put(format!("{base}/api/cron/jobs/{id}/enable"))
            .json(&serde_json::json!({ "enabled": enable }))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-toggle-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "cron-toggled",
            &[("id", id), ("action", endpoint)],
        ));
    }
}
