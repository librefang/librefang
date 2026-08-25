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
        Some(workflows) if workflows.is_empty() => {
            println!("{}", i18n::t("automation-workflow-none"));
        }
        Some(workflows) => {
            let header_id = i18n::t("label-header-id");
            let header_name = i18n::t("label-header-name");
            let header_steps = i18n::t("label-header-steps");
            let header_created = i18n::t("label-header-created");
            let mut t = crate::table::Table::new(&[
                &header_id,
                &header_name,
                &header_steps,
                &header_created,
            ]);
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
        None => println!("{}", i18n::t("automation-workflow-none")),
    }
}

pub(crate) fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    if !file.exists() {
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-workflow-file-not-found",
                &[("path", &file.display().to_string())]
            )
        );
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-workflow-read-error",
                &[("error", &e.to_string())]
            )
        );
        std::process::exit(1);
    });
    let json_body: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-workflow-invalid-json",
                &[("error", &e.to_string())]
            )
        );
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
        println!("{}", i18n::t("automation-workflow-created"));
        println!(
            "{}",
            i18n::t_args("automation-workflow-created-id", &[("id", id)])
        );
    } else {
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-workflow-create-failed",
                &[("error", &err_localized)]
            )
        );
        std::process::exit(1);
    }
}

/// How long the daemon may hold `POST /api/workflows/{id}/run` open before it hands the run back as a background task.
///
/// `?wait=true` on its own selects the fully synchronous branch of `run_workflow`, whose own comment records that the run is owned by the request: the CLI's client gives up after 120 s (`daemon_client`) and a step defaults to a 120 s timeout of its own, so any multi-step workflow would disconnect mid-run and take the run with it.
/// Passing `timeout_ms` selects the branch that spawns the run as its own task, so a workflow that outlives the wait keeps going and we report its id instead.
/// 90 s leaves 30 s of the client budget for the response itself.
const WORKFLOW_RUN_WAIT_MS: u64 = 90_000;

/// The wait we ask the daemon for has to expire before our own client gives up, or the 202-with-`run_id` path is unreachable and a slow run comes back as a disconnect that the operator reads as a failure.
const _: () = assert!(
    WORKFLOW_RUN_WAIT_MS < 120_000,
    "daemon_client() times out at 120 s; a longer wait can never return 202"
);

/// What `librefang workflow run` should report for one response from `POST /api/workflows/{id}/run`.
///
/// Split out of [`cmd_workflow_run`] so the mapping from response shape to exit code is unit-testable — the command itself ends in `std::process::exit`, which is why the original inversion (every launch reported as a failure) had no test that could have caught it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorkflowRunOutcome<'a> {
    /// The run finished inside the wait window; carries its output.
    Completed { run_id: &'a str, output: &'a str },
    /// The daemon accepted the run and it is still going.
    /// A launch is not a failure, so this exits 0 and prints the id the operator can poll.
    Accepted { run_id: &'a str },
    /// The run, or the request, failed.
    /// Carries the most specific message the body offers, or an empty string when it offers none.
    Failed { error: &'a str },
}

/// Pick the most specific failure message a workflow-run body carries.
///
/// Three shapes reach here and only one of them puts the sentence in `error`.
/// The timed-wait branch answers 422 with `{"error":"workflow_failed","detail":"<why>"}`, where `error` is a machine code and `detail` is what an operator needs.
/// `ApiErrorResponse` (404, 400) serializes `error` as a nested `{code, message}` envelope and mirrors the sentence in a flat `message`.
/// Older hand-rolled bodies put it in a plain `error` string.
fn workflow_run_failure_detail(body: &serde_json::Value) -> &str {
    [
        &body["detail"],
        &body["message"],
        &body["error"]["message"],
        &body["error"],
    ]
    .into_iter()
    .filter_map(|value| value.as_str())
    .map(str::trim)
    .find(|s| !s.is_empty())
    .unwrap_or_default()
}

/// Classify a workflow-run response by its HTTP status first, and only then by the fields in its body.
///
/// Gating on `body["output"]` alone is what made every successful launch print "Unknown error" and exit 1: a 202 carries no `output` by design, because the run has not finished.
pub(crate) fn classify_workflow_run(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
) -> WorkflowRunOutcome<'_> {
    if !status.is_success() {
        return WorkflowRunOutcome::Failed {
            error: workflow_run_failure_detail(body),
        };
    }
    let run_id = body["run_id"].as_str().unwrap_or("?");
    match body["output"].as_str() {
        Some(output) => WorkflowRunOutcome::Completed { run_id, output },
        None => WorkflowRunOutcome::Accepted { run_id },
    }
}

pub(crate) fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let (status, body) = daemon_json_checked(
        client
            .post(format!(
                "{base}/api/workflows/{workflow_id}/run?wait=true&timeout_ms={WORKFLOW_RUN_WAIT_MS}"
            ))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    match classify_workflow_run(status, &body) {
        WorkflowRunOutcome::Completed { run_id, output } => {
            println!("{}", i18n::t("automation-workflow-completed"));
            println!(
                "{}",
                i18n::t_args("automation-workflow-run-id", &[("id", run_id)])
            );
            println!("  Output:\n{output}");
        }
        WorkflowRunOutcome::Accepted { run_id } => {
            println!("{}", i18n::t("automation-workflow-still-running"));
            println!(
                "{}",
                i18n::t_args("automation-workflow-run-id", &[("id", run_id)])
            );
        }
        WorkflowRunOutcome::Failed { error } => {
            let err_localized = if error.is_empty() {
                i18n::t("error-unknown")
            } else {
                error.to_string()
            };
            eprintln!(
                "{}",
                i18n::t_args("automation-workflow-failed", &[("error", &err_localized)])
            );
            std::process::exit(1);
        }
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
        Some(triggers) if triggers.is_empty() => {
            println!("{}", i18n::t("automation-trigger-none"));
        }
        Some(triggers) => {
            let header_trigger_id = i18n::t("label-header-trigger-id");
            let header_agent_id = i18n::t("label-header-agent-id");
            let header_enabled = i18n::t("label-header-enabled");
            let header_fires = i18n::t("label-header-fires");
            let header_pattern = i18n::t("label-header-pattern");
            let yes_str = i18n::t("label-yes");
            let no_str = i18n::t("label-no");

            let mut tbl = crate::table::Table::new(&[
                &header_trigger_id,
                &header_agent_id,
                &header_enabled,
                &header_fires,
                &header_pattern,
            ]);
            for t in triggers {
                tbl.add_row(&[
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    if t["enabled"].as_bool().unwrap_or(false) {
                        &yes_str
                    } else {
                        &no_str
                    },
                    &t["fire_count"].as_u64().unwrap_or(0).to_string(),
                    t["pattern"].as_str().unwrap_or("?"),
                ]);
            }
            tbl.print();
        }
        None => println!("{}", i18n::t("automation-trigger-none")),
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
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-invalid-pattern",
                &[("error", &e.to_string())]
            )
        );
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
        println!("{}", i18n::t("automation-trigger-created"));
        println!(
            "{}",
            i18n::t_args("automation-trigger-created-id", &[("id", id)])
        );
        println!(
            "{}",
            i18n::t_args(
                "automation-trigger-created-agent",
                &[("agent_id", &agent_id)]
            )
        );
        if let Some(t) = target_agent {
            println!(
                "{}",
                i18n::t_args("automation-trigger-created-target", &[("target", t)])
            );
        }
    } else {
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-create-failed",
                &[("error", &err_localized)]
            )
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
        println!(
            "{}",
            i18n::t_args("automation-trigger-deleted", &[("id", trigger_id)])
        );
    } else {
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-delete-failed",
                &[("error", &err_localized)]
            )
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
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-get-failed",
                &[("error", &err_localized)]
            )
        );
        std::process::exit(1);
    }

    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-id",
            &[("id", body["id"].as_str().unwrap_or("-"))]
        )
    );
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-agent",
            &[("id", body["agent_id"].as_str().unwrap_or("-"))]
        )
    );
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-pattern",
            &[("pattern", &body["pattern"].to_string())]
        )
    );
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-prompt",
            &[("prompt", body["prompt_template"].as_str().unwrap_or("-"))]
        )
    );

    let yes_str = i18n::t("label-yes");
    let no_str = i18n::t("label-no");
    let enabled_str = if body["enabled"].as_bool().unwrap_or(false) {
        &yes_str
    } else {
        &no_str
    };
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-enabled",
            &[("enabled", enabled_str)]
        )
    );
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-fires",
            &[(
                "count",
                &body["fire_count"].as_u64().unwrap_or(0).to_string()
            )]
        )
    );

    let max_fires_str = body["max_fires"]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| i18n::t("automation-unlimited"));
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-info-max-fires",
            &[("count", &max_fires_str)]
        )
    );

    if let Some(t) = body["target_agent_id"].as_str() {
        println!(
            "{}",
            i18n::t_args("automation-trigger-info-target", &[("agent", t)])
        );
    }
    if let Some(c) = body["cooldown_secs"].as_u64() {
        println!(
            "{}",
            i18n::t_args(
                "automation-trigger-info-cooldown",
                &[("secs", &c.to_string())]
            )
        );
    }
    if let Some(m) = body["session_mode"].as_str() {
        println!(
            "{}",
            i18n::t_args("automation-trigger-info-session", &[("mode", m)])
        );
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
            eprintln!(
                "{}",
                i18n::t_args(
                    "automation-trigger-invalid-pattern",
                    &[("error", &e.to_string())]
                )
            );
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
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-update-failed",
                &[("error", &err_localized)]
            )
        );
        std::process::exit(1);
    }
    println!(
        "{}",
        i18n::t_args("automation-trigger-updated", &[("id", trigger_id)])
    );
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

    let action = if enabled { "enable" } else { "disable" };
    if body.get("error").is_some() {
        let err_msg = body["error"].as_str().unwrap_or("Unknown error");
        let err_localized = if err_msg == "Unknown error" {
            i18n::t("error-unknown")
        } else {
            err_msg.to_string()
        };
        eprintln!(
            "{}",
            i18n::t_args(
                "automation-trigger-toggle-failed",
                &[("action", action), ("error", &err_localized)]
            )
        );
        std::process::exit(1);
    }
    println!(
        "{}",
        i18n::t_args(
            "automation-trigger-toggled",
            &[("id", trigger_id), ("action", action)]
        )
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
            println!("{}", i18n::t("automation-cron-none"));
            return;
        }
        let header_id = i18n::t("label-header-id");
        let header_agent = i18n::t("label-header-agent");
        let header_schedule = i18n::t("label-header-schedule");
        let header_enabled = i18n::t("label-header-enabled");
        let header_prompt = i18n::t("label-header-prompt");
        let yes_str = i18n::t("label-yes");
        let no_str = i18n::t("label-no");

        let mut t = crate::table::Table::new(&[
            &header_id,
            &header_agent,
            &header_schedule,
            &header_enabled,
            &header_prompt,
        ]);
        for j in arr {
            t.add_row(&[
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["schedule"]["expr"]
                    .as_str()
                    .or_else(|| j["cron_expr"].as_str())
                    .unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    &yes_str
                } else {
                    &no_str
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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    /// The bug in #7825: the default arm of `run_workflow` answers 202 with a body carrying only `run_id`, and the command read `output` from it.
    /// Every successful launch printed "Unknown error" and exited 1, so any script gating on the exit code saw a failure on every run.
    #[test]
    fn an_accepted_run_is_not_a_failure() {
        let body = serde_json::json!({"run_id": "3f1c-run"});

        assert_eq!(
            classify_workflow_run(StatusCode::ACCEPTED, &body),
            WorkflowRunOutcome::Accepted { run_id: "3f1c-run" },
            "a 202 must exit 0 and name the run, not report a failure"
        );
    }

    #[test]
    fn a_finished_run_reports_its_output() {
        let body =
            serde_json::json!({"run_id": "3f1c-run", "output": "hello", "status": "completed"});

        assert_eq!(
            classify_workflow_run(StatusCode::OK, &body),
            WorkflowRunOutcome::Completed {
                run_id: "3f1c-run",
                output: "hello",
            }
        );
    }

    /// The mirror-image half: a run that really failed must exit non-zero, and must show the reason rather than the `workflow_failed` machine code the route puts in `error`.
    #[test]
    fn a_failed_run_surfaces_its_detail_not_the_machine_code() {
        let body = serde_json::json!({
            "error": "workflow_failed",
            "detail": "step 'draft': agent 'writer' not found",
        });

        assert_eq!(
            classify_workflow_run(StatusCode::UNPROCESSABLE_ENTITY, &body),
            WorkflowRunOutcome::Failed {
                error: "step 'draft': agent 'writer' not found",
            }
        );
    }

    #[test]
    fn a_generic_client_error_falls_back_to_the_error_field() {
        let body = serde_json::json!({"error": "Workflow 'nope' not found"});

        assert_eq!(
            classify_workflow_run(StatusCode::NOT_FOUND, &body),
            WorkflowRunOutcome::Failed {
                error: "Workflow 'nope' not found",
            }
        );
    }

    /// `ApiErrorResponse` (a 404 for an unknown workflow, a 400 for a bad id) serializes `error` as a `{code, message}` envelope, so reading `error` as a string finds nothing and the operator would be told "Unknown error" about a workflow the daemon named precisely.
    #[test]
    fn a_nested_error_envelope_still_yields_its_message() {
        let body = serde_json::json!({
            "error": {"code": "not_found", "message": "Workflow 'nope' not found"},
            "message": "Workflow 'nope' not found",
        });

        assert_eq!(
            classify_workflow_run(StatusCode::NOT_FOUND, &body),
            WorkflowRunOutcome::Failed {
                error: "Workflow 'nope' not found",
            }
        );
    }

    /// A 5xx with an unparseable body still has to exit non-zero; the empty message is what `cmd_workflow_run` replaces with the localized `error-unknown`.
    #[test]
    fn a_bodyless_server_error_is_still_a_failure() {
        let body = serde_json::Value::Null;

        assert_eq!(
            classify_workflow_run(StatusCode::INTERNAL_SERVER_ERROR, &body),
            WorkflowRunOutcome::Failed { error: "" }
        );
    }
}
