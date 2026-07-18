//! Kernel-side wiring for the autonomous goal runner (#5744).
//!
//! Bridges the standalone [`crate::goal_runner::GoalRunner`] to the live agent
//! send path: each goal-run tick is an autonomous agent turn driven through
//! `send_message_with_sender_context` with the reserved `"autonomous"` channel
//! sentinel (same RBAC carve-out as the continuous / cron background loops).
//!
//! These are inherent helpers; the `KernelApi` trait methods (`start_goal_run`
//! etc.) delegate here so the HTTP layer can reach them through
//! `Arc<dyn KernelApi>`.

use librefang_channels::types::SenderContext;
use librefang_types::agent::{AgentId, AgentManifest, ModelConfig, SessionMode};
use librefang_types::goal::{GoalId, GoalRunState, DEFAULT_GOAL_MAX_ITERATIONS};

use super::{LibreFangKernel, SYSTEM_CHANNEL_AUTONOMOUS};
use crate::MemorySubsystemApi;

impl LibreFangKernel {
    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// Each tick is a full agent turn; the runner parses the agent's reply for
    /// `GOAL_PROGRESS:` / `GOAL_DONE` markers and updates the goal until it is
    /// complete, the iteration cap (`max_iterations`, default
    /// [`DEFAULT_GOAL_MAX_ITERATIONS`]) is reached, an operator stops it, or the
    /// kernel shuts down.
    pub fn goal_run_start(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
    ) {
        let max = max_iterations.unwrap_or(DEFAULT_GOAL_MAX_ITERATIONS).max(1);
        let substrate = self.substrate_ref().clone();

        // The tick closure drives a real agent turn, which needs an owned
        // `Arc<LibreFangKernel>`. Upgrade the self-handle (set right after the
        // kernel is wrapped in `Arc` at boot).
        let kernel = match self.self_handle.get().and_then(|w| w.upgrade()) {
            Some(k) => k,
            None => {
                tracing::warn!(%goal_id, "Cannot start goal run: kernel self-handle unset");
                return;
            }
        };

        let send_kernel = kernel.clone();
        let send = move |aid: AgentId, msg: String| {
            let k = send_kernel.clone();
            async move {
                let sender = SenderContext {
                    channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
                    user_id: aid.to_string(),
                    display_name: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
                    is_internal_system: true,
                    ..Default::default()
                };
                match k.send_message_with_sender_context(aid, &msg, &sender).await {
                    Ok(r) => Ok(r.response),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        // Sub-agent spawn closure for the loop. Gated on loop_engineering
        // inside the closure body (not via if/else) so both branches
        // return the same concrete type.
        let spawn_kernel = kernel.clone();
        let spawn_sub = move |task_name: String| {
            let k = spawn_kernel.clone();
            async move {
                if !loop_engineering {
                    return None;
                }
                let manifest = AgentManifest {
                    name: format!(
                        "goal-sub-{}",
                        uuid::Uuid::new_v4()
                            .to_string()
                            .split('-')
                            .next()
                            .unwrap_or("x")
                    ),
                    version: "0.1.0".into(),
                    description: format!("Auto-spawned sub-agent: {task_name}"),
                    author: "goal-runner".into(),
                    module: "builtin:chat".into(),
                    schedule: librefang_types::agent::ScheduleMode::Reactive,
                    session_mode: SessionMode::New,
                    model: ModelConfig {
                        provider: "deepseek".into(),
                        model: "deepseek-v4-pro".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                // spawn_agent is sync (not async).
                k.spawn_agent(manifest).ok()
            }
        };

        use librefang_skills::evolution::create_skill;

        // Clone before kernel is consumed by closures below.
        let goal_id_for_title = goal_id;
        let goal_title = {
            let substrate = self.substrate_ref();
            let arr = substrate
                .structured_get(
                    librefang_types::goal::goals_storage_agent_id(),
                    librefang_types::goal::GOALS_STORAGE_KEY,
                )
                .ok()
                .flatten()
                .unwrap_or(serde_json::Value::Array(vec![]));
            if let serde_json::Value::Array(arr) = arr {
                let target = goal_id_for_title.to_string();
                arr.into_iter()
                    .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
                    .and_then(|g| g.get("title").and_then(|v| v.as_str()).map(String::from))
                    .unwrap_or_else(|| format!("Goal {goal_id_for_title}"))
            } else {
                format!("Goal {goal_id_for_title}")
            }
        };

        // Evaluator closure: asks a cheap model whether the goal condition
        // has been met, mirroring Claude Code's /goal evaluator pattern.
        // Never trust the agent's self-reported GOAL_DONE alone.
        let eval_kernel = kernel.clone();
        let evaluate = move |goal_desc: String, agent_reply: String| {
            let k = eval_kernel.clone();
            async move {
                let prompt = format!(
                    "You are a goal evaluator. Read the goal and the agent's \
                     latest output. Answer ONLY 'YES' if the goal is fully \
                     achieved, or 'NO' if more work is needed.\n\n\
                     GOAL: {goal_desc}\n\n\
                     AGENT OUTPUT:\n{agent_reply}\n\n\
                     Is the goal achieved? (YES/NO):"
                );
                // Use agent itself for evaluation (same send path).
                // In a full implementation, this would use a separate
                // cheap evaluator model (Haiku) as Claude Code does.
                let sender = SenderContext {
                    channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
                    user_id: agent_id.to_string(),
                    display_name: "goal-evaluator".to_string(),
                    is_internal_system: true,
                    ..Default::default()
                };
                match k
                    .send_message_with_sender_context(agent_id, &prompt, &sender)
                    .await
                {
                    Ok(r) => {
                        let upper = r.response.to_ascii_uppercase();
                        Ok(upper.contains("YES") && !upper.contains("NO"))
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        // Learnings callback: persist captured knowledge as an auto-created
        // skill so the agent self-evolves. Only when loop_engineering is on.
        let learnings_agent_id = agent_id;
        let skills_dir = self.home_dir().join("skills");
        let on_learnings = move |learnings: Vec<String>| {
            if !loop_engineering || learnings.is_empty() {
                return;
            }
            let body = format!(
                "## Learnings from goal run\n\n{}\n\n## Usage\n\
                 These patterns were discovered during autonomous execution of \
                 goal `{title}`. Apply them when solving similar tasks.",
                learnings
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{}. {l}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n"),
                title = goal_title,
            );
            let skill_name = format!(
                "goal-learned-{}",
                goal_title
                    .to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric(), "-")
                    .trim_matches('-')
            );
            match create_skill(
                &skills_dir,
                &skill_name,
                &format!("Auto-discovered from goal: {goal_title}"),
                &body,
                vec!["goal-learned".into(), "auto-evolved".into()],
                Some("goal-runner"),
            ) {
                Ok(result) => {
                    tracing::info!(
                        agent = %learnings_agent_id,
                        goal_id = %goal_id,
                        count = learnings.len(),
                        skill = %result.skill_name,
                        "Goal runner: auto-created skill from captured learnings"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        agent = %learnings_agent_id,
                        goal_id = %goal_id,
                        error = %e,
                        "Goal runner: failed to auto-create skill from learnings"
                    );
                }
            }
        };

        self.workflows.goal_runner.start(
            goal_id,
            agent_id,
            max,
            substrate,
            send,
            spawn_sub,
            on_learnings,
            evaluate,
            loop_engineering,
            verify_agent_id,
            verify_max_retries,
        );
    }

    /// Stop an active goal run. Returns whether a run was stopped.
    pub fn goal_run_stop(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.stop(goal_id)
    }

    /// Snapshot the observable state of a goal's run, if one is active.
    pub fn goal_run_status(&self, goal_id: GoalId) -> Option<GoalRunState> {
        self.workflows.goal_runner.state(goal_id)
    }

    /// Recover goal runs interrupted by a prior crash or restart.
    ///
    /// Boot calls this once, mirroring the workflow stale-recovery sweep:
    /// persisted runs still in `Running` phase and older than `stale_timeout`
    /// are demoted to `Stopped` ("Interrupted by daemon restart"). Runs are not
    /// auto-resumed — an in-flight LLM call cannot be replayed. Returns the
    /// recovered goal ids.
    /// Returns (goal_id, agent_id) pairs for stale runs to auto-resume.
    /// Caller must call `goal_run_start` for each returned pair.
    pub fn recover_stale_goal_runs(
        &self,
        stale_timeout: std::time::Duration,
    ) -> Vec<(GoalId, AgentId)> {
        self.workflows.goal_runner.recover_stale_runs(stale_timeout)
    }
}
