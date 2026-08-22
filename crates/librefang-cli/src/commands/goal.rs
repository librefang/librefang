//! `goal` CLI command handler — create and run an autonomous long-horizon goal.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

/// How often `--watch` polls the run endpoint.
const WATCH_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) fn cmd_goal(
    description: &str,
    agent: Option<&str>,
    max_iterations: Option<u64>,
    watch: bool,
) {
    let base = require_daemon("goal");
    let client = daemon_client();

    // Resolve the agent before creating anything.
    //
    // `POST /api/goals/{id}/start` refuses a goal with no agent assigned, so
    // creating first and discovering that afterwards would leave an unrunnable
    // goal behind on every mistyped invocation.
    let Some(agent) = agent else {
        ui::error_with_fix(
            &i18n::t("cmd-goal-agent-required"),
            &i18n::t("cmd-goal-agent-required-fix"),
        );
        std::process::exit(1);
    };
    let agent_id = resolve_agent_id(&base, agent);
    if uuid::Uuid::try_parse(&agent_id).is_err() {
        ui::error_with_fix(
            &i18n::t_args("cmd-goal-agent-unknown", &[("agent", agent)]),
            &i18n::t("cmd-goal-agent-required-fix"),
        );
        std::process::exit(1);
    }

    // 1. Create the goal via POST /api/goals.
    let payload = serde_json::json!({
        "title": description,
        "description": description,
        "status": "pending",
        "progress": 0,
        "agent_id": agent_id,
    });

    let create_body = daemon_json(
        client
            .post(format!("{base}/api/goals"))
            .json(&payload)
            .send(),
    );

    let Some(goal_id) = create_body["id"].as_str().map(str::to_string) else {
        eprintln!(
            "{}",
            i18n::t_args(
                "cmd-goal-create-error",
                &[("error", &api_error(&create_body))]
            )
        );
        std::process::exit(1);
    };

    println!("{}", i18n::t_args("cmd-goal-created", &[("id", &goal_id)]));

    // 2. Start the run via POST /api/goals/{id}/start.
    let mut start_payload = serde_json::json!({});
    if let Some(mi) = max_iterations {
        start_payload["max_iterations"] = serde_json::json!(mi);
    }

    let start_body = daemon_json(
        client
            .post(format!("{base}/api/goals/{goal_id}/start"))
            .json(&start_payload)
            .send(),
    );

    if start_body.get("error").is_some() {
        eprintln!(
            "{}",
            i18n::t_args(
                "cmd-goal-start-error",
                &[("error", &api_error(&start_body))]
            )
        );
        std::process::exit(1);
    }

    if !watch {
        // Fire-and-forget: leave the id on stdout so it can be piped onward.
        println!("{goal_id}");
        return;
    }

    // 3. Poll GET /api/goals/{id}/run until the loop leaves the running phase.
    eprintln!("{}", i18n::t("cmd-goal-watching"));
    loop {
        std::thread::sleep(WATCH_POLL_INTERVAL);

        let run_body = daemon_json(client.get(format!("{base}/api/goals/{goal_id}/run")).send());

        let running = run_body["running"].as_bool().unwrap_or(false);
        let run = &run_body["run"];
        let phase = run["phase"].as_str().unwrap_or_default();

        eprintln!(
            "{}",
            i18n::t_args(
                "cmd-goal-progress",
                &[
                    (
                        "current",
                        &run["iteration"].as_u64().unwrap_or(0).to_string()
                    ),
                    (
                        "total",
                        &run["max_iterations"].as_u64().unwrap_or(0).to_string()
                    ),
                    ("phase", &crate::tui::screens::goals::translate_phase(phase)),
                    (
                        "progress",
                        &run["last_progress"].as_u64().unwrap_or(0).to_string()
                    ),
                ],
            ),
        );

        if !running {
            if let Some(key) = terminal_phase_message(phase) {
                eprintln!("{}", i18n::t(key));
            }
            if let Some(err) = run["last_error"].as_str().filter(|e| !e.is_empty()) {
                eprintln!("{}", i18n::t_args("cmd-goal-error", &[("error", err)]));
            }
            // A run that ends anywhere but `finished` did not achieve the goal;
            // reporting success there would break `librefang goal … --watch &&
            // next-step` in a script.
            if phase != "finished" {
                std::process::exit(1);
            }
            break;
        }
    }
}

/// The `error` field of a daemon response, or a generic stand-in.
fn api_error(body: &serde_json::Value) -> String {
    body["error"]
        .as_str()
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| i18n::t("cmd-goal-unknown-error"))
}

/// Locale key summarising a terminal run phase, or `None` while still running.
fn terminal_phase_message(phase: &str) -> Option<&'static str> {
    match phase {
        "finished" => Some("cmd-goal-finished"),
        "max_iterations_reached" => Some("cmd-goal-max-iterations"),
        "rate_limited" => Some("cmd-goal-rate-limited"),
        "stopped" => Some("cmd-goal-stopped"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_prefers_the_daemon_message() {
        let body = serde_json::json!({ "error": "Title too long (max 256 chars)" });
        assert_eq!(api_error(&body), "Title too long (max 256 chars)");
    }

    #[test]
    fn api_error_falls_back_when_the_field_is_missing_or_blank() {
        let fallback = i18n::t("cmd-goal-unknown-error");
        assert_eq!(api_error(&serde_json::json!({})), fallback);
        assert_eq!(api_error(&serde_json::json!({ "error": "" })), fallback);
    }

    #[test]
    fn every_terminal_phase_has_a_summary() {
        // Mirrors `GoalRunPhase` in librefang-types minus `Running`, which is
        // the one phase that is not terminal.
        for phase in [
            "finished",
            "max_iterations_reached",
            "rate_limited",
            "stopped",
        ] {
            assert!(
                terminal_phase_message(phase).is_some(),
                "terminal phase '{phase}' has no summary message"
            );
        }
        assert!(terminal_phase_message("running").is_none());
    }
}
