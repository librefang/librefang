//! `goal` CLI command handler — create and run autonomous long-horizon goals.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via [`crate::commands::prelude`].

use crate::commands::prelude::*;

pub(crate) fn cmd_goal(
    description: &str,
    agent_id: Option<&str>,
    max_iterations: Option<u64>,
    loop_engineering: bool,
    watch: bool,
) {
    let base = require_daemon("goal");
    let client = daemon_client();

    // 1. Create the goal via POST /api/goals
    let mut payload = serde_json::json!({
        "title": description,
        "description": description,
        "status": "pending",
        "progress": 0,
        "loop_engineering": loop_engineering,
    });
    if let Some(aid) = agent_id {
        // `--agent` accepts a name or a UUID (see the command's own
        // long_about example `--agent my-agent`); resolve it the same way
        // every other command does, otherwise a name silently fails the
        // UUID parse in the goals route and the goal falls back to
        // auto-spawning an unrelated disposable agent.
        payload["agent_id"] = serde_json::json!(resolve_agent_id(&base, aid));
    }

    let create_body = daemon_json(
        client
            .post(format!("{base}/api/goals"))
            .json(&payload)
            .send(),
    );

    let goal_id = match create_body["id"].as_str() {
        Some(id) => id.to_string(),
        None => {
            let err_msg = create_body["error"].as_str().unwrap_or("Unknown error");
            eprintln!("Error creating goal: {err_msg}");
            std::process::exit(1);
        }
    };

    println!("Goal created: {goal_id}");

    // 2. Start the goal run via POST /api/goals/{id}/start
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
        let err_msg = start_body["error"].as_str().unwrap_or("Unknown error");
        eprintln!("Error starting goal run: {err_msg}");
        std::process::exit(1);
    }

    if !watch {
        // Fire-and-forget: print the goal ID and exit.
        println!("{goal_id}");
        return;
    }

    // 3. With --watch: poll GET /api/goals/{id}/run every 2s and print progress.
    eprintln!("Watching goal run (Ctrl+C to stop)...");
    loop {
        std::thread::sleep(Duration::from_secs(2));

        let run_body = daemon_json(client.get(format!("{base}/api/goals/{goal_id}/run")).send());

        let running = run_body["running"].as_bool().unwrap_or(false);
        let phase = run_body["run"]["phase"].as_str().unwrap_or("unknown");
        let iteration = run_body["run"]["iteration"].as_u64().unwrap_or(0);
        let max_it = run_body["run"]["max_iterations"].as_u64().unwrap_or(0);
        let progress = run_body["run"]["last_progress"].as_u64().unwrap_or(0);

        eprintln!(
            "  [{}/{}] phase={} progress={}%",
            iteration, max_it, phase, progress,
        );

        if !running {
            match phase {
                "finished" => eprintln!("  Goal finished successfully!"),
                "max_iterations_reached" => eprintln!("  Goal reached max iterations."),
                "rate_limited" => eprintln!("  Goal run rate-limited."),
                "stopped" => eprintln!("  Goal run stopped."),
                _ => {}
            }
            if let Some(err) = run_body["run"]["last_error"].as_str() {
                if !err.is_empty() {
                    eprintln!("  error: {err}");
                }
            }
            break;
        }
    }
}
