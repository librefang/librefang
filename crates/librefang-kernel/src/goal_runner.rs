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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use librefang_memory::{GoalRunRow, GoalRunStore, MemorySubstrate};
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
        } else if marker_present(&upper, "GOAL_DONE") || marker_present(&upper, "GOAL_COMPLETE") {
            out.done = true;
        } else if marker_present(&upper, "GOAL_BLOCKED") {
            out.blocked = true;
        }
    }
    out
}

/// Match `marker` as a standalone token at the start of `line`, not a bare
/// prefix. The marker counts only when the line begins with it AND the byte
/// immediately after is a word boundary — end-of-line, or any character that is
/// not a word-continuation char (i.e. not alphanumeric and not `_`). That
/// admits the bare form (`GOAL_DONE`), trailing punctuation the model tends to
/// add (`GOAL_DONE!`, `GOAL_DONE.`), and the trailing-note form the prompt
/// suggests (`GOAL_BLOCKED: need a key`), while still rejecting a longer
/// identifier that merely starts with the token (`GOAL_DONE_CRITERIA`,
/// `GOAL_DONENESS`). `line` is expected to be already uppercased.
fn marker_present(line: &str, marker: &str) -> bool {
    match line.strip_prefix(marker) {
        Some(rest) => rest
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_'),
        None => false,
    }
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

/// Flatten a `GoalRunState` into the `goal_runs` row shape the store persists.
fn row_from_state(state: &GoalRunState) -> GoalRunRow {
    GoalRunRow {
        goal_id: state.goal_id.to_string(),
        agent_id: state.agent_id.to_string(),
        phase: state.phase.to_string(),
        iteration: state.iteration as i64,
        max_iterations: state.max_iterations as i64,
        last_progress: state.last_progress as i64,
        last_error: state.last_error.clone(),
        started_at: state.started_at.to_rfc3339(),
        updated_at: state.updated_at.to_rfc3339(),
    }
}

/// Mirror the live run state into the durable store. A persistence failure is
/// logged and swallowed — the in-memory DashMap stays the hot path, so a
/// transient DB hiccup must never abort or stall the run loop.
fn persist_run(store: &Option<GoalRunStore>, state: &GoalRunState) {
    let Some(store) = store else { return };
    if let Err(e) = store.save_run(&row_from_state(state)) {
        warn!(goal_id = %state.goal_id, "Failed to persist goal run state: {e}");
    }
}

/// Drop the durable mirror once a run ends. Same failure policy as
/// [`persist_run`]: log and swallow.
fn delete_persisted_run(store: &Option<GoalRunStore>, goal_id: GoalId) {
    let Some(store) = store else { return };
    if let Err(e) = store.delete_run(&goal_id.to_string()) {
        warn!(goal_id = %goal_id, "Failed to delete persisted goal run: {e}");
    }
}

/// A single goal run entry: the spawned loop task plus its observable state
/// and a cooperative stop flag.
struct RunHandle {
    /// The spawned loop task. `None` for a terminal entry reconstructed at boot
    /// by [`GoalRunner::recover_stale_runs`] — that run's process already died,
    /// so there is no live loop to abort; the entry exists only so the demoted
    /// `Stopped` state stays observable via [`GoalRunner::state`].
    task: Option<JoinHandle<()>>,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    /// Monotonic id for this run, used by the task's self-cleanup so it only
    /// removes its OWN registry entry — never a newer run that replaced it.
    generation: u64,
}

/// Registry + driver for autonomous goal runs. One [`GoalRunner`] lives on the
/// kernel; it tracks at most one active run per goal.
pub struct GoalRunner {
    runs: Arc<DashMap<GoalId, RunHandle>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Source of monotonic run generations (see [`RunHandle::generation`]).
    next_gen: Arc<AtomicU64>,
    /// Durable mirror of active run state (#5744 follow-up). `None` when the
    /// runner is constructed without persistence (e.g. unit tests that drive
    /// `run_loop` directly); the in-memory DashMap remains the hot path either
    /// way.
    store: Option<GoalRunStore>,
}

impl GoalRunner {
    /// Create a runner wired to the kernel shutdown signal, without durable
    /// persistence. Used where no memory substrate is available.
    pub fn new(shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
            next_gen: Arc::new(AtomicU64::new(0)),
            store: None,
        }
    }

    /// Create a runner backed by a [`GoalRunStore`] so active runs survive a
    /// daemon restart. Boot wires this with the shared memory connection pool.
    pub fn new_with_store(shutdown_rx: watch::Receiver<bool>, store: GoalRunStore) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
            next_gen: Arc::new(AtomicU64::new(0)),
            store: Some(store),
        }
    }

    /// Snapshot the observable state of a goal's run, if one exists.
    pub fn state(&self, goal_id: GoalId) -> Option<GoalRunState> {
        let handle = self.runs.get(&goal_id)?;
        // try_lock: None → `running:false`; run_loop must never hold this lock across I/O.
        handle.state.try_lock().ok().map(|s| s.clone())
    }

    /// Stop a goal's run if active. Returns whether a run was stopped.
    ///
    /// An operator stop is a terminal boundary, so the durable mirror is
    /// dropped too — a stopped run must not be resurrected as "stale" at the
    /// next boot.
    pub fn stop(&self, goal_id: GoalId) -> bool {
        if let Some((_, handle)) = self.runs.remove(&goal_id) {
            handle.stop.store(true, Ordering::SeqCst);
            // A recovered terminal entry has no live loop task to abort.
            if let Some(task) = handle.task {
                task.abort();
            }
            delete_persisted_run(&self.store, goal_id);
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
        let initial = GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            started_at: now,
            updated_at: now,
        };
        // Persist the initial Running row before the first tick so a crash
        // mid-tick still leaves a recoverable record at the next boot.
        persist_run(&self.store, &initial);
        let state = Arc::new(Mutex::new(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let generation = self.next_gen.fetch_add(1, Ordering::SeqCst);

        let runs = self.runs.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
        let loop_store = self.store.clone();

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
                loop_store,
            )
            .await;
            // Self-cleanup: drop the registry entry once the loop ends so a
            // stale handle does not linger (mirrors the background executor).
            // Guard on generation: if a concurrent `start()` already replaced
            // this run, the entry now belongs to the NEW run — removing it
            // unconditionally would orphan a live loop (unstoppable + invisible
            // until it self-terminates at the iteration cap). `remove_if` only
            // drops the entry when it is still ours.
            runs.remove_if(&goal_id, |_, h| h.generation == generation);
        });

        self.runs.insert(
            goal_id,
            RunHandle {
                task: Some(task),
                state,
                stop,
                generation,
            },
        );
        info!(goal_id = %goal_id, agent_id = %agent_id, max_iterations, "Goal run started");
    }

    /// Recover goal runs left in `Running` phase by a prior crash or restart.
    ///
    /// Called once at boot, mirroring `WorkflowEngine::recover_stale_running_runs`.
    /// Only persisted rows still in `Running` phase are candidates — any
    /// terminal-phase row was already deleted when its run ended, so the only
    /// `Running` rows on disk are ones whose process died mid-run. For each such
    /// row older than `stale_timeout`, demote it to `Stopped` with the same
    /// `"Interrupted by daemon restart"` marker workflow recovery uses, persist
    /// that, and checkpoint the WAL so the transition is durable. The run is
    /// **not** auto-resumed — an in-flight LLM call cannot be replayed, so the
    /// policy matches workflow: surface the interrupted run as failed/stopped
    /// rather than silently restarting it. Returns the recovered goal ids.
    pub fn recover_stale_runs(&self, stale_timeout: Duration) -> Vec<GoalId> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        if stale_timeout.is_zero() {
            return Vec::new();
        }
        let rows = match store.load_all_runs() {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Failed to load persisted goal runs for recovery: {e}");
                return Vec::new();
            }
        };

        let now = Utc::now();
        let stale_secs = stale_timeout.as_secs() as i64;
        let mut recovered: Vec<GoalId> = Vec::new();
        for row in rows {
            // Terminal-phase rows are settled; only `Running` rows are stale
            // candidates. (Belt-and-braces: the run loop deletes terminal rows,
            // so a non-running row on disk would be a bug elsewhere.)
            if row.phase != GoalRunPhase::Running.to_string() {
                continue;
            }
            let Ok(goal_id) = row.goal_id.parse::<GoalId>() else {
                warn!(goal_id = %row.goal_id, "Skipping goal run with unparseable id during recovery");
                continue;
            };
            let started_at = match chrono::DateTime::parse_from_rfc3339(&row.started_at) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(e) => {
                    warn!(goal_id = %goal_id, "Skipping goal run with unparseable started_at during recovery: {e}");
                    continue;
                }
            };
            let age = now.signed_duration_since(started_at).num_seconds();
            // Wall-clock skew guard, identical to the workflow sweep (#5114):
            // `Utc::now()` is not monotonic, so a backwards NTP step makes `age`
            // negative. Treat a negative age as "fresh" rather than silently
            // masking a real stale row, and warn so operators see the skew.
            if age < 0 {
                warn!(
                    goal_id = %goal_id,
                    now = %now,
                    started_at = %started_at,
                    age_secs = age,
                    "Negative goal run age — wall-clock moved backwards; \
                     treating run as fresh, not stale"
                );
                continue;
            }
            if age < stale_secs {
                continue;
            }
            warn!(
                goal_id = %goal_id,
                started_at = %started_at,
                age_secs = age,
                "Recovering stale goal run interrupted by daemon restart"
            );
            let recovered_row = GoalRunRow {
                phase: GoalRunPhase::Stopped.to_string(),
                last_error: Some("Interrupted by daemon restart".to_string()),
                updated_at: now.to_rfc3339(),
                ..row
            };
            if let Err(e) = store.save_run(&recovered_row) {
                warn!(goal_id = %goal_id, "Failed to persist recovered goal run: {e}");
                continue;
            }
            // Load the demoted row back into the in-memory registry so the
            // runtime read path (`state` → `goal_run_status` → GET
            // /goals/{id}/run) surfaces "stopped — interrupted by daemon
            // restart" after a restart, instead of returning `None` for a row
            // that exists only on disk. Mirrors `WorkflowEngine::load_runs`,
            // which loads persisted rows back into memory before the stale
            // sweep so demoted runs stay observable. The entry carries no live
            // task (`task: None`) and is purely a terminal placeholder — the
            // run is **not** resumed or re-executed.
            match recovered_row.agent_id.parse::<AgentId>() {
                Ok(agent_id) => {
                    let state = GoalRunState {
                        goal_id,
                        agent_id,
                        phase: GoalRunPhase::Stopped,
                        iteration: recovered_row.iteration.max(0) as u32,
                        max_iterations: recovered_row.max_iterations.max(0) as u32,
                        last_progress: recovered_row.last_progress.clamp(0, 100) as u8,
                        last_error: recovered_row.last_error.clone(),
                        started_at,
                        updated_at: now,
                    };
                    self.runs.insert(
                        goal_id,
                        RunHandle {
                            task: None,
                            state: Arc::new(Mutex::new(state)),
                            stop: Arc::new(AtomicBool::new(true)),
                            generation: self.next_gen.fetch_add(1, Ordering::SeqCst),
                        },
                    );
                }
                Err(_) => {
                    // The row was demoted on disk; only the in-memory surfacing
                    // is skipped. Operators still see the corrected DB row.
                    warn!(
                        goal_id = %goal_id,
                        agent_id = %recovered_row.agent_id,
                        "Recovered goal run has unparseable agent id; demoted on \
                         disk but not surfaced via the runtime read path"
                    );
                }
            }
            recovered.push(goal_id);
        }
        if !recovered.is_empty() {
            if let Err(e) = store.wal_checkpoint() {
                warn!("Goal run recovery WAL checkpoint failed: {e}");
            }
        }
        recovered
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
    store: Option<GoalRunStore>,
) where
    F: Fn(AgentId, String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
{
    let mut iteration: u32 = 0;
    let mut rate_limit_streak: u32 = 0;
    // True when the loop ends because the kernel is shutting down (vs. an
    // operator stop, completion, or cap). On shutdown the durable row is left
    // in its last persisted `Running` shape so the next boot's stale-recovery
    // sweep can demote it — mirroring how workflow runs survive a restart.
    let mut interrupted_by_shutdown = false;
    let final_phase = loop {
        if stop.load(Ordering::SeqCst) {
            break GoalRunPhase::Stopped;
        }
        if *shutdown_rx.borrow() {
            interrupted_by_shutdown = true;
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

                // Release before persist_run: state()'s try_lock returns None (→ running:false) while held.
                let snapshot = {
                    let mut s = state.lock().await;
                    s.iteration = iteration + 1;
                    if let Some(p) = new_progress {
                        s.last_progress = p;
                    }
                    s.last_error = None;
                    s.updated_at = Utc::now();
                    s.clone()
                };
                // Mirror the post-iteration state to the durable store so a
                // crash before the next tick still leaves a recoverable row.
                persist_run(&store, &snapshot);

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
                // Same lock discipline as success path: release before persist_run.
                let snapshot = {
                    let mut s = state.lock().await;
                    s.last_error = Some(e);
                    s.updated_at = Utc::now();
                    s.clone()
                };
                persist_run(&store, &snapshot);
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
                    interrupted_by_shutdown = true;
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
    // A run that reaches a natural terminal phase (completed, capped, rate-
    // limited, agent-blocked, or an operator stop) is settled — drop its
    // durable row so it is never resurfaced as "stale" at the next boot. A
    // shutdown-interrupted run is the exception: leave its last `Running` row
    // in place so boot recovery demotes it, exactly as workflow runs do.
    if !interrupted_by_shutdown {
        delete_persisted_run(&store, goal_id);
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

    #[test]
    fn parse_tick_requires_marker_token_boundary() {
        // Substrings that merely start with a control marker must NOT trip it.
        assert!(!parse_tick("GOAL_DONENESS: not yet").done);
        assert!(!parse_tick("GOAL_DONE_CRITERIA: ship it").done);
        assert!(!parse_tick("GOAL_COMPLETENESS: 40%").done);
        assert!(!parse_tick("GOAL_BLOCKEDNESS is low").blocked);

        // Bare and boundary-delimited forms still register, including the
        // trailing punctuation the model commonly appends.
        assert!(parse_tick("GOAL_DONE").done);
        assert!(parse_tick("GOAL_DONE now").done);
        assert!(parse_tick("GOAL_DONE.").done);
        assert!(parse_tick("GOAL_DONE!").done);
        assert!(parse_tick("GOAL_DONE - shipped the report").done);
        assert!(parse_tick("GOAL_COMPLETE").done);
        assert!(parse_tick("GOAL_BLOCKED").blocked);
        assert!(parse_tick("GOAL_BLOCKED! waiting on a key").blocked);
        assert!(parse_tick("GOAL_BLOCKED: need a key").blocked);
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
            None,
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
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::MaxIterationsReached);
        assert_eq!(s.iteration, 2);
        // Goal stays in progress, not completed.
        let stored = load_goal(&substrate, goal_id).unwrap();
        assert_eq!(stored.status, GoalStatus::InProgress);
    }

    fn mk_state(
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: u32,
    ) -> Arc<Mutex<GoalRunState>> {
        Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    #[tokio::test]
    async fn run_loop_stops_when_agent_reports_blocked() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);

        let send = |_a: AgentId, _p: String| async move {
            Ok("stuck\nGOAL_BLOCKED: need a key".to_string())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
        // Blocked must NOT mark the goal completed.
        assert_eq!(
            load_goal(&substrate, goal.id).unwrap().status,
            GoalStatus::InProgress
        );
    }

    #[tokio::test]
    async fn run_loop_stops_immediately_when_stop_flag_preset() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);

        // Operator stop is observed at the top of the loop before any tick.
        let send = |_a: AgentId, _p: String| async move {
            panic!("send_message must not be called once the stop flag is set");
            #[allow(unreachable_code)]
            Ok(String::new())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(true)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Stopped);
        assert_eq!(s.iteration, 0, "no tick should run");
    }

    #[tokio::test]
    async fn run_loop_stops_immediately_on_shutdown_signal() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        // Shutdown already signalled.
        let (_tx, rx) = watch::channel(true);
        let state = mk_state(goal.id, agent_id, 10);

        let send = |_a: AgentId, _p: String| async move {
            panic!("send_message must not be called during shutdown");
            #[allow(unreachable_code)]
            Ok(String::new())
        };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
    }

    #[tokio::test(start_paused = true)]
    async fn run_loop_breaks_after_consecutive_rate_limits() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 100);

        // Every tick fails with the rate-limit marker; the circuit breaker must
        // trip at MAX_RATE_LIMIT_STREAK rather than burning all 100 iterations.
        // start_paused auto-advances the inter-tick sleeps so this is instant.
        let send = |_a: AgentId, _p: String| async move {
            Err(format!(
                "provider quota exhausted {}",
                librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER
            ))
        };
        run_loop(
            goal.id,
            agent_id,
            100,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::RateLimited);
        assert!(
            s.iteration < 100,
            "must trip the breaker, not run to the cap"
        );
    }

    // --- Persistence + boot recovery (#5744 follow-up) ---

    /// Build a goal-run store sharing the substrate's SQLite pool. The
    /// substrate has already run migrations, so the `goal_runs` table exists.
    fn store_from(substrate: &MemorySubstrate) -> GoalRunStore {
        GoalRunStore::new(substrate.pool())
    }

    #[tokio::test(start_paused = true)]
    async fn run_loop_persists_state_across_iterations() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let store = store_from(&substrate);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 3);

        // Capture the persisted row after the second iteration, before the run
        // reaches the cap and deletes the row. A oneshot fires from inside the
        // fake send_message on the third call.
        let counter = Arc::new(AtomicU64::new(0));
        let probe_store = store.clone();
        let probe_id = goal.id.to_string();
        let captured: Arc<Mutex<Option<GoalRunRow>>> = Arc::new(Mutex::new(None));
        let probe_captured = captured.clone();
        let send = move |_a: AgentId, _p: String| {
            let counter = counter.clone();
            let probe_store = probe_store.clone();
            let probe_id = probe_id.clone();
            let probe_captured = probe_captured.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                // On the third call (n == 2), two iterations have already
                // persisted; snapshot the row before the loop ends.
                if n == 2 {
                    let row = probe_store.get_run(&probe_id).unwrap();
                    *probe_captured.lock().await = row;
                }
                Ok("GOAL_PROGRESS: 40".to_string())
            }
        };
        run_loop(
            goal.id,
            agent_id,
            3,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            Some(store.clone()),
        )
        .await;

        let row = captured
            .lock()
            .await
            .clone()
            .expect("a Running row must have been persisted mid-run");
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert_eq!(row.goal_id, goal.id.to_string());
        assert!(
            row.iteration >= 2,
            "iterations must accumulate in the store"
        );
        assert_eq!(row.last_progress, 40);
    }

    #[tokio::test]
    async fn completed_run_is_deleted_from_store() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let store = store_from(&substrate);
        // Pre-seed a Running row as `start()` would.
        store
            .save_run(&row_from_state(&GoalRunState {
                goal_id: goal.id,
                agent_id,
                phase: GoalRunPhase::Running,
                iteration: 0,
                max_iterations: 10,
                last_progress: 0,
                last_error: None,
                started_at: Utc::now(),
                updated_at: Utc::now(),
            }))
            .unwrap();
        assert!(store.get_run(&goal.id.to_string()).unwrap().is_some());

        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 10);
        let send = |_a: AgentId, _p: String| async move { Ok("done\nGOAL_DONE".to_string()) };
        run_loop(
            goal.id,
            agent_id,
            10,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            Some(store.clone()),
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Finished);
        assert!(
            store.get_run(&goal.id.to_string()).unwrap().is_none(),
            "a completed run must be removed from the durable store"
        );
    }

    #[test]
    fn recover_stale_run_marks_it_stopped_at_boot() {
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        // A Running row whose process died an hour ago.
        let stale_started = Utc::now() - chrono::Duration::seconds(3600);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: None,
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone());

        // 10-minute staleness window → the hour-old run is recovered.
        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert_eq!(recovered, vec![goal_id]);

        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        assert_eq!(row.phase, GoalRunPhase::Stopped.to_string());
        assert_eq!(
            row.last_error,
            Some("Interrupted by daemon restart".to_string())
        );
    }

    #[test]
    fn recovered_stale_run_is_observable_via_runtime_read_path() {
        // Regression: a stale `Running` row demoted to `Stopped` at boot must
        // also be loaded back into the in-memory registry so `state()` — the
        // runtime read path behind `goal_run_status` and GET /goals/{id}/run —
        // surfaces it, instead of returning `None` for a row that exists only
        // on disk (write-only invisibility). Mirrors WorkflowEngine, which
        // loads persisted rows back into memory before the stale sweep.
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        let stale_started = Utc::now() - chrono::Duration::seconds(3600);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: None,
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone());

        // Before recovery the registry is empty — nothing observable yet.
        assert!(runner.state(goal_id).is_none());

        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert_eq!(recovered, vec![goal_id]);

        // The demoted run is now visible through the runtime read path, not
        // just present in the DB, and carries the interrupted marker.
        let observed = runner
            .state(goal_id)
            .expect("recovered run must be observable via the runtime read path");
        assert_eq!(observed.phase, GoalRunPhase::Stopped);
        assert_eq!(observed.agent_id, agent_id);
        assert_eq!(observed.iteration, 5);
        assert_eq!(observed.max_iterations, 25);
        assert_eq!(observed.last_progress, 50);
        assert_eq!(
            observed.last_error,
            Some("Interrupted by daemon restart".to_string())
        );

        // The terminal placeholder must not shadow a future live run: an
        // operator stop clears it (start() calls stop() before inserting the
        // new run), restoring the empty-registry invariant.
        assert!(runner.stop(goal_id), "stop() removes the recovered entry");
        assert!(runner.state(goal_id).is_none());
    }

    #[test]
    fn recover_skips_fresh_running_run() {
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();

        // A Running row that started just now — not stale.
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Running.to_string(),
                iteration: 1,
                max_iterations: 25,
                last_progress: 10,
                last_error: None,
                started_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone());
        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert!(recovered.is_empty(), "a fresh run must not be recovered");

        // Row stays Running, untouched.
        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert!(row.last_error.is_none());
    }
}
