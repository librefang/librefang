//! Long-horizon autonomous goal execution (#5744).
//!
//! The Goals system (CRUD + dashboard) tracks objectives but, on its own, is
//! purely passive — nothing ever drives an agent toward a goal. The
//! [`GoalRunner`] closes that gap: starting a run for a goal with an assigned
//! agent spawns a bounded loop that repeatedly prompts the agent with the
//! goal's context and parses the agent's reply for progress / completion
//! markers, updating the goal in the shared memory store until the goal is
//! done, the iteration cap is hit, an operator stops it, or the kernel shuts
//! down.
//!
//! ## Why response markers instead of a tool
//!
//! The agent reports progress by ending its turn with structured lines:
//!
//! ```text
//! GOAL_PROGRESS: 60
//! GOAL_DONE          (optional — signals the goal is complete)
//! GOAL_BLOCKED       (optional — signals it cannot proceed without input)
//! ```
//!
//! This keeps the v1 runner entirely kernel-side: no new runtime tool, no
//! tool-registry / capability-permission surgery. The parsing is forgiving
//! (case-insensitive, last marker wins) so an agent that forgets the marker
//! simply keeps iterating to the cap rather than failing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use librefang_memory::MemorySubstrate;
use librefang_types::agent::AgentId;
use librefang_types::goal::{
    goals_storage_agent_id, Goal, GoalId, GoalRunPhase, GoalRunState, GoalStatus, GOALS_STORAGE_KEY,
};

use crate::background::{classify_tick_error, TickOutcome};

/// Pause between iterations. Short — the agent turn itself dominates wall-clock;
/// this just yields and lets shutdown / stop signals be observed promptly.
const TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Consecutive provider rate-limit ticks before the loop gives up, mirroring
/// the background executor's circuit breaker (#5168) so a quota-exhausted
/// provider does not get hammered on every iteration.
const MAX_RATE_LIMIT_STREAK: u32 = 3;

/// Result of parsing one agent reply for goal-control markers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedTick {
    /// Progress value (0-100) if the agent emitted `GOAL_PROGRESS:`.
    pub progress: Option<u8>,
    /// The agent signalled completion (`GOAL_DONE`).
    pub done: bool,
    /// The agent signalled it is blocked (`GOAL_BLOCKED`).
    pub blocked: bool,
}

/// Parse an agent reply for `GOAL_PROGRESS:` / `GOAL_DONE` / `GOAL_BLOCKED`
/// markers. Case-insensitive; the last `GOAL_PROGRESS` line wins.
pub fn parse_tick(reply: &str) -> ParsedTick {
    let mut out = ParsedTick::default();
    for line in reply.lines() {
        let t = line.trim();
        let upper = t.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("GOAL_PROGRESS:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                out.progress = Some(n.min(100) as u8);
            }
        } else if upper.starts_with("GOAL_DONE") || upper.starts_with("GOAL_COMPLETE") {
            out.done = true;
        } else if upper.starts_with("GOAL_BLOCKED") {
            out.blocked = true;
        }
    }
    out
}

/// Build the per-iteration prompt that frames the goal for the agent.
pub fn build_goal_prompt(goal: &Goal, iteration: u32, max_iterations: u32) -> String {
    format!(
        "[LONG-HORIZON GOAL] You are autonomously pursuing a goal across multiple turns.\n\
         Goal: {title}\n\
         Description: {description}\n\
         Current progress: {progress}%\n\
         Iteration: {iter} of {max}\n\n\
         Take the next concrete action toward completing this goal. When you finish a \
         step, end your reply with a line `GOAL_PROGRESS: <0-100>` reflecting overall \
         completion. Add a line `GOAL_DONE` once the goal is fully achieved, or \
         `GOAL_BLOCKED` if you cannot proceed without operator input.",
        title = goal.title,
        description = if goal.description.is_empty() {
            "(none)"
        } else {
            &goal.description
        },
        progress = goal.progress,
        iter = iteration + 1,
        max = max_iterations,
    )
}

/// Load the goal with `goal_id` from the shared goals store.
fn load_goal(substrate: &MemorySubstrate, goal_id: GoalId) -> Option<Goal> {
    let arr = match substrate.structured_get(goals_storage_agent_id(), GOALS_STORAGE_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        _ => return None,
    };
    let target = goal_id.to_string();
    arr.into_iter()
        .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
        .and_then(|v| serde_json::from_value(v).ok())
}

/// Atomically patch a goal's progress / status / `updated_at` in the shared
/// store. Uses `structured_modify` so concurrent writers (the API CRUD path)
/// never lose this update to a last-writer-wins race.
fn patch_goal(
    substrate: &MemorySubstrate,
    goal_id: GoalId,
    progress: Option<u8>,
    status: Option<GoalStatus>,
) {
    let target = goal_id.to_string();
    let res =
        substrate.structured_modify(goals_storage_agent_id(), GOALS_STORAGE_KEY, |existing| {
            let mut arr = match existing {
                Some(serde_json::Value::Array(arr)) => arr,
                _ => Vec::new(),
            };
            for g in arr.iter_mut() {
                if g.get("id").and_then(|v| v.as_str()) != Some(target.as_str()) {
                    continue;
                }
                if let Some(obj) = g.as_object_mut() {
                    if let Some(p) = progress {
                        obj.insert("progress".into(), serde_json::json!(p));
                    }
                    if let Some(s) = status {
                        obj.insert("status".into(), serde_json::json!(s.to_string()));
                    }
                    obj.insert("updated_at".into(), serde_json::json!(Utc::now()));
                }
                break;
            }
            Ok((serde_json::Value::Array(arr), ()))
        });
    if let Err(e) = res {
        warn!(goal_id = %goal_id, "Failed to persist goal update: {e}");
    }
}

/// A single in-flight goal run: the spawned loop task plus its observable state
/// and a cooperative stop flag.
struct RunHandle {
    task: JoinHandle<()>,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
}

/// Registry + driver for autonomous goal runs. One [`GoalRunner`] lives on the
/// kernel; it tracks at most one active run per goal.
pub struct GoalRunner {
    runs: Arc<DashMap<GoalId, RunHandle>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl GoalRunner {
    /// Create a runner wired to the kernel shutdown signal.
    pub fn new(shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
        }
    }

    /// Whether a run is currently active for `goal_id`.
    pub fn is_running(&self, goal_id: GoalId) -> bool {
        self.runs
            .get(&goal_id)
            .is_some_and(|h| !h.task.is_finished())
    }

    /// Snapshot the observable state of a goal's run, if one exists.
    pub fn state(&self, goal_id: GoalId) -> Option<GoalRunState> {
        let handle = self.runs.get(&goal_id)?;
        // try_lock avoids blocking the caller (an async HTTP handler) on a tick;
        // a momentary contention just yields no snapshot this call.
        handle.state.try_lock().ok().map(|s| s.clone())
    }

    /// Stop a goal's run if active. Returns whether a run was stopped.
    pub fn stop(&self, goal_id: GoalId) -> bool {
        if let Some((_, handle)) = self.runs.remove(&goal_id) {
            handle.stop.store(true, Ordering::SeqCst);
            handle.task.abort();
            true
        } else {
            false
        }
    }

    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// `send_message` performs one agent turn and yields the agent's reply text
    /// (or an error string). The loop owns iteration counting, marker parsing,
    /// goal persistence, and the rate-limit circuit breaker.
    ///
    /// Replaces any existing run for the same goal.
    pub fn start<F, Fut>(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: u32,
        substrate: Arc<MemorySubstrate>,
        send_message: F,
    ) where
        F: Fn(AgentId, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        // Replace any prior run for this goal.
        self.stop(goal_id);

        let now = Utc::now();
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            started_at: now,
            updated_at: now,
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let runs = self.runs.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();

        let task = tokio::spawn(async move {
            run_loop(
                goal_id,
                agent_id,
                max_iterations,
                substrate,
                send_message,
                loop_state,
                loop_stop,
                shutdown_rx,
            )
            .await;
            // Self-cleanup: drop the registry entry once the loop ends so a
            // stale handle does not linger (mirrors the background executor).
            runs.remove(&goal_id);
        });

        self.runs.insert(goal_id, RunHandle { task, state, stop });
        info!(goal_id = %goal_id, agent_id = %agent_id, max_iterations, "Goal run started");
    }
}

/// The run loop body. Extracted as a free function so tests can drive it with a
/// fake `send_message` and an in-memory substrate.
#[allow(clippy::too_many_arguments)]
async fn run_loop<F, Fut>(
    goal_id: GoalId,
    agent_id: AgentId,
    max_iterations: u32,
    substrate: Arc<MemorySubstrate>,
    send_message: F,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    F: Fn(AgentId, String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
{
    let mut iteration: u32 = 0;
    let mut rate_limit_streak: u32 = 0;
    let final_phase = loop {
        if stop.load(Ordering::SeqCst) {
            break GoalRunPhase::Stopped;
        }
        if *shutdown_rx.borrow() {
            break GoalRunPhase::Stopped;
        }

        let goal = match load_goal(&substrate, goal_id) {
            Some(g) => g,
            None => {
                warn!(goal_id = %goal_id, "Goal vanished from store; ending run");
                break GoalRunPhase::Finished;
            }
        };
        if matches!(goal.status, GoalStatus::Completed | GoalStatus::Cancelled)
            || goal.progress >= 100
        {
            break GoalRunPhase::Finished;
        }
        if iteration >= max_iterations {
            break GoalRunPhase::MaxIterationsReached;
        }

        let prompt = build_goal_prompt(&goal, iteration, max_iterations);
        debug!(goal_id = %goal_id, iteration, "Goal run: sending tick");

        match send_message(agent_id, prompt).await {
            Ok(reply) => {
                rate_limit_streak = 0;
                let parsed = parse_tick(&reply);
                let new_status = if parsed.done {
                    Some(GoalStatus::Completed)
                } else {
                    Some(GoalStatus::InProgress)
                };
                let new_progress = if parsed.done {
                    Some(100)
                } else {
                    parsed.progress
                };
                patch_goal(&substrate, goal_id, new_progress, new_status);

                {
                    let mut s = state.lock().await;
                    s.iteration = iteration + 1;
                    if let Some(p) = new_progress {
                        s.last_progress = p;
                    }
                    s.last_error = None;
                    s.updated_at = Utc::now();
                }

                if parsed.done {
                    break GoalRunPhase::Finished;
                }
                if parsed.blocked {
                    info!(goal_id = %goal_id, "Goal run: agent reported blocked; ending run");
                    break GoalRunPhase::Stopped;
                }
            }
            Err(e) => {
                match classify_tick_error(&e) {
                    TickOutcome::RateLimited => {
                        rate_limit_streak = rate_limit_streak.saturating_add(1);
                        warn!(
                            goal_id = %goal_id,
                            consecutive_rate_limits = rate_limit_streak,
                            "Goal run: tick failed on provider rate-limit",
                        );
                    }
                    TickOutcome::Ok => {
                        rate_limit_streak = 0;
                    }
                }
                {
                    let mut s = state.lock().await;
                    s.last_error = Some(e);
                    s.updated_at = Utc::now();
                }
                if rate_limit_streak >= MAX_RATE_LIMIT_STREAK {
                    break GoalRunPhase::RateLimited;
                }
            }
        }

        iteration += 1;

        tokio::select! {
            _ = tokio::time::sleep(TICK_INTERVAL) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break GoalRunPhase::Stopped;
                }
            }
        }
    };

    {
        let mut s = state.lock().await;
        s.phase = final_phase;
        s.updated_at = Utc::now();
    }
    info!(goal_id = %goal_id, phase = %final_phase, "Goal run ended");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tick_extracts_progress_done_blocked() {
        let p = parse_tick("working...\nGOAL_PROGRESS: 60\nmore text");
        assert_eq!(p.progress, Some(60));
        assert!(!p.done);

        let d = parse_tick("all set\ngoal_done");
        assert!(d.done);

        let b = parse_tick("stuck\nGOAL_BLOCKED: need a key");
        assert!(b.blocked);

        // Last progress wins; >100 clamps.
        let m = parse_tick("GOAL_PROGRESS: 30\nGOAL_PROGRESS: 250");
        assert_eq!(m.progress, Some(100));

        // No markers → all default.
        assert_eq!(parse_tick("just a normal reply"), ParsedTick::default());
    }

    fn seed_goal(substrate: &MemorySubstrate, goal: &Goal) {
        substrate
            .structured_set(
                goals_storage_agent_id(),
                GOALS_STORAGE_KEY,
                serde_json::json!([serde_json::to_value(goal).unwrap()]),
            )
            .unwrap();
    }

    fn test_goal(agent_id: AgentId) -> Goal {
        Goal {
            id: GoalId::new(),
            title: "Write a report".into(),
            description: String::new(),
            parent_id: None,
            status: GoalStatus::InProgress,
            progress: 0,
            agent_id: Some(agent_id),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn run_loop_stops_and_completes_on_goal_done() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 10,
            last_progress: 0,
            last_error: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        // Agent reports done on the first turn.
        let send = |_a: AgentId, _p: String| async move { Ok("done\nGOAL_DONE".to_string()) };

        run_loop(
            goal_id,
            agent_id,
            10,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Finished);
        let stored = load_goal(&substrate, goal_id).unwrap();
        assert_eq!(stored.status, GoalStatus::Completed);
        assert_eq!(stored.progress, 100);
    }

    #[tokio::test]
    async fn run_loop_honors_max_iterations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 2,
            last_progress: 0,
            last_error: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        // Agent never finishes — always reports partial progress.
        let send = |_a: AgentId, _p: String| async move { Ok("GOAL_PROGRESS: 10".to_string()) };

        run_loop(
            goal_id,
            agent_id,
            2,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::MaxIterationsReached);
        assert_eq!(s.iteration, 2);
        // Goal stays in progress, not completed.
        let stored = load_goal(&substrate, goal_id).unwrap();
        assert_eq!(stored.status, GoalStatus::InProgress);
    }
}
