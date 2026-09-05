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
    goals_storage_agent_id, Goal, GoalId, GoalRunPhase, GoalRunState, GoalStatus,
    DEFAULT_GOAL_TICK_INTERVAL_SECS, GOALS_STORAGE_KEY, MAX_GOAL_TICK_INTERVAL_SECS,
    MIN_GOAL_TICK_INTERVAL_SECS,
};

use crate::background::{classify_tick_error, TickOutcome};
use crate::KernelApi;

fn lock_goal_run_start_stop(lock: &std::sync::Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!("goal runner start/stop lock poisoned; recovering serialization");
            let guard = poisoned.into_inner();
            lock.clear_poison();
            guard
        }
    }
}

/// Consecutive provider rate-limit ticks before the loop gives up, mirroring
/// the background executor's circuit breaker (#5168) so a quota-exhausted
/// provider does not get hammered on every iteration.
const MAX_RATE_LIMIT_STREAK: u32 = 3;

/// Result of [`create_and_start_goal`]: the persisted goal id and whether a
/// run was scheduled for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalLaunch {
    pub goal_id: GoalId,
    pub started: bool,
}

impl GoalLaunch {
    /// Confirmation text for surfaces without a localized message catalog
    /// (channel adapters, dashboard chat WebSocket).
    pub fn message(&self, description: &str) -> String {
        let goal_id = self.goal_id;
        if self.started {
            format!("Goal created and started: {description} (ID: {goal_id})")
        } else {
            format!(
                "Goal created (ID: {goal_id}) but the run could not start — \
                 kernel self-handle unset. Restart the daemon and resume the goal."
            )
        }
    }
}

/// Persist a goal for `agent_id` and immediately start a run for it.
///
/// Every chat surface that exposes `/goal` — the channel bridge, the dashboard
/// chat WebSocket and the TUI chat runner — goes through this one function, so
/// a goal created from Telegram is identical in shape to one created from the
/// dashboard (upstream #3355).
pub fn create_and_start_goal(
    kernel: &dyn KernelApi,
    agent_id: AgentId,
    description: &str,
    loop_engineering: bool,
) -> Result<GoalLaunch, String> {
    if description.chars().count() > 4096 {
        return Err(format!(
            "Goal description too long ({} chars, max 4096)",
            description.chars().count()
        ));
    }

    let goal_id = GoalId::new();
    let now = Utc::now().to_rfc3339();
    let title: String = description.chars().take(256).collect();
    let entry = serde_json::json!({
        "id": goal_id.to_string(),
        "title": title,
        "description": description,
        "status": GoalStatus::Pending.to_string(),
        "progress": 0,
        "agent_id": agent_id.to_string(),
        "loop_engineering": loop_engineering,
        "created_at": now,
        "updated_at": now,
    });

    kernel
        .memory_substrate()
        .structured_modify(goals_storage_agent_id(), GOALS_STORAGE_KEY, |current| {
            let mut goals: Vec<serde_json::Value> = match current {
                Some(serde_json::Value::Array(arr)) => arr,
                _ => Vec::new(),
            };
            goals.push(entry.clone());
            Ok((serde_json::Value::Array(goals), ()))
        })
        .map_err(|e| format!("Failed to create goal: {e}"))?;

    let started = kernel.start_goal_run(goal_id, agent_id, None);
    Ok(GoalLaunch { goal_id, started })
}

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
        .map(|mut v| {
            // Goals written before the loop-engineering PR may store
            // UUID-typed Option fields as "" instead of null.  An empty
            // string fails UUID parsing and silently drops the whole
            // Goal via the downstream `.ok()`.  Normalise here.
            for key in ["verify_agent_id", "agent_id", "parent_id"] {
                if v.get(key).and_then(|s| s.as_str()) == Some("") {
                    v[key] = serde_json::Value::Null;
                }
            }
            v
        })
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

/// Shared-memory key holding a paused run's resume checkpoint.
///
/// ## Why not the `goal_runs` table
///
/// That table is the durable mirror of *active* runs, and its schema pins
/// `phase` with `CHECK (phase IN ('running','finished',
/// 'max_iterations_reached','rate_limited','stopped'))`, which SQLite cannot
/// alter in place to admit a `paused` value. A paused run is by definition not
/// an active run, so the mirror is the wrong home for it regardless — the
/// shared KV is where the goal-adjacent durable state already lives (the
/// goals array itself), and it holds the whole checkpoint as one value, which
/// makes the pause write atomic rather than a multi-column update that could
/// tear.
fn goal_pause_key(goal_id: GoalId) -> String {
    format!("goal_pause_{goal_id}")
}

/// The state a paused run hands to its successor.
struct ResumePoint {
    agent_id: AgentId,
    iteration: u32,
    max_iterations: u32,
    last_progress: u8,
}

/// Write the checkpoint a paused run resumes from.
fn persist_pause_checkpoint(substrate: &MemorySubstrate, goal_id: GoalId, state: &GoalRunState) {
    if let Err(e) = substrate.structured_set(
        goals_storage_agent_id(),
        &goal_pause_key(goal_id),
        serde_json::json!({
            "agent_id": state.agent_id.to_string(),
            "iteration": state.iteration,
            "max_iterations": state.max_iterations,
            "last_progress": state.last_progress,
            "paused_at": Utc::now().to_rfc3339(),
        }),
    ) {
        warn!(goal_id = %goal_id, error = %e,
              "Failed to persist goal pause checkpoint — resume will restart the goal");
    }
}

/// Read a paused run's checkpoint, if one is stored.
fn load_pause_checkpoint(substrate: &MemorySubstrate, goal_id: GoalId) -> Option<ResumePoint> {
    let value = substrate
        .structured_get(goals_storage_agent_id(), &goal_pause_key(goal_id))
        .ok()
        .flatten()?;
    Some(ResumePoint {
        agent_id: value
            .get("agent_id")
            .and_then(|v| v.as_str())?
            .parse()
            .ok()?,
        iteration: value.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        max_iterations: value
            .get("max_iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        last_progress: value
            .get("last_progress")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(100) as u8,
    })
}

/// Drop a pause checkpoint once it has been consumed or cancelled.
///
/// Load-bearing: a checkpoint that outlives its pause would silently seed the
/// *next* fresh start of the same goal with a stale iteration count.
fn clear_pause_checkpoint(substrate: &MemorySubstrate, goal_id: GoalId) {
    if let Err(e) = substrate.structured_delete(goals_storage_agent_id(), &goal_pause_key(goal_id))
    {
        warn!(goal_id = %goal_id, error = %e, "Failed to clear goal pause checkpoint");
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

/// Persist the first snapshot of a new run, replacing any durable predecessor
/// in one SQLite statement so a crash cannot land between delete and insert.
fn persist_new_run(store: &Option<GoalRunStore>, state: &GoalRunState) {
    let Some(store) = store else { return };
    if let Err(e) = store.start_run(&row_from_state(state)) {
        warn!(goal_id = %state.goal_id, "Failed to persist new goal run state: {e}");
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
    /// The spawned loop task.
    ///
    /// `None` in three cases, none of which have a task to abort: a terminal
    /// entry reconstructed at boot by [`GoalRunner::recover_stale_runs`] (that
    /// run's process already died); the brief window inside [`GoalRunner::start`]
    /// between registering the handle and backfilling the join handle; and a
    /// run whose loop finished before that backfill could happen.
    task: Option<JoinHandle<()>>,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    /// Cooperative pause flag. Distinct from `stop` because the two mean
    /// opposite things to the durable row: `stop` deletes it, `pause`
    /// checkpoints it.
    pause: Arc<AtomicBool>,
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
    /// Shared-memory handle for pause checkpoints.
    ///
    /// Held at construction rather than taken from `start`'s argument, because
    /// `stop`, `pause`, and `state` must reach a checkpoint written by a
    /// *previous* process — a goal paused before a daemon restart has to stay
    /// cancellable and observable without anyone calling `start` first.
    substrate: Option<Arc<MemorySubstrate>>,
    /// Serializes the compound `start()` / `stop()` sequences for one goal so a
    /// concurrent `start()` cannot observe an empty registry slot between an
    /// in-flight `start()`'s stop and its insert and spawn a second, orphaned
    /// loop. The per-generation self-cleanup guard only protects the sequential
    /// replace path; it does nothing for two `start()` calls racing on the same
    /// goal id. The guarded region is fully synchronous (no `.await`), so this
    /// std `Mutex` is never held across an await point.
    start_lock: std::sync::Mutex<()>,
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
            substrate: None,
            start_lock: std::sync::Mutex::new(()),
        }
    }

    /// Create a runner backed by a [`GoalRunStore`] so active runs survive a
    /// daemon restart. Boot wires this with the shared memory connection pool.
    pub fn new_with_store(
        shutdown_rx: watch::Receiver<bool>,
        store: GoalRunStore,
        substrate: Arc<MemorySubstrate>,
    ) -> Self {
        Self {
            runs: Arc::new(DashMap::new()),
            shutdown_rx,
            next_gen: Arc::new(AtomicU64::new(0)),
            store: Some(store),
            substrate: Some(substrate),
            start_lock: std::sync::Mutex::new(()),
        }
    }

    /// Snapshot the observable state of a goal's run, if one exists.
    ///
    /// Falls back to a persisted pause checkpoint when the registry has no
    /// live entry. A paused run's loop task exits and self-cleans its
    /// registry slot, so without this fallback pausing a goal would make it
    /// vanish from `GET /api/goals/{id}/run` entirely.
    pub fn state(&self, goal_id: GoalId) -> Option<GoalRunState> {
        if let Some(handle) = self.runs.get(&goal_id) {
            // try_lock: None → `running:false`; run_loop must never hold this lock across I/O.
            return handle.state.try_lock().ok().map(|s| s.clone());
        }
        let substrate = self.substrate.as_ref()?;
        let checkpoint = load_pause_checkpoint(substrate, goal_id)?;
        let now = Utc::now();
        Some(GoalRunState {
            goal_id,
            agent_id: checkpoint.agent_id,
            phase: GoalRunPhase::Paused,
            iteration: checkpoint.iteration,
            max_iterations: checkpoint.max_iterations,
            last_progress: checkpoint.last_progress,
            last_error: None,
            started_at: now,
            updated_at: now,
        })
    }

    /// Pause a goal's run, checkpointing its iteration count and progress.
    ///
    /// Returns whether a live run was signalled. The loop finishes the turn
    /// it is on, checkpoints, and exits in [`GoalRunPhase::Paused`]; a later
    /// [`GoalRunner::start`] picks up from that checkpoint.
    ///
    /// Deliberately does NOT abort the task the way [`GoalRunner::stop`]
    /// does — the loop needs to run to completion of its current turn to
    /// reach the checkpoint write.
    pub fn pause(&self, goal_id: GoalId) -> bool {
        let _guard = lock_goal_run_start_stop(&self.start_lock);
        let Some(handle) = self.runs.get(&goal_id) else {
            return false;
        };
        // A recovered terminal entry has no loop to signal.
        if handle.task.is_none() {
            return false;
        }
        handle.pause.store(true, Ordering::SeqCst);
        info!(goal_id = %goal_id, "Goal run pause requested");
        true
    }

    /// Stop a goal's run if active. Returns whether a run was stopped.
    ///
    /// An operator stop is a terminal boundary, so the durable mirror is
    /// dropped too — a stopped run must not be resurrected as "stale" at the
    /// next boot.
    pub fn stop(&self, goal_id: GoalId) -> bool {
        // Serialize against `start()` so the two never interleave on the same
        // goal id. The critical section is synchronous, so this std guard never
        // spans an await point. Recover the exclusion guard rather than panic.
        let _guard = lock_goal_run_start_stop(&self.start_lock);
        self.stop_locked(goal_id)
    }

    /// Stop body assuming the caller already holds `start_lock`. Split out so
    /// `start()` can run it inside its own critical section without re-locking
    /// the non-reentrant `start_lock` (which would deadlock).
    fn stop_locked(&self, goal_id: GoalId) -> bool {
        // Cancelling discards the resume checkpoint, whether or not a loop is
        // live: a goal paused before a daemon restart has no registry entry,
        // so the checkpoint is the ONLY thing cancel has to remove. Leaving
        // it would make the next start silently resume a run the operator
        // cancelled.
        let had_checkpoint = match self.substrate.as_ref() {
            Some(substrate) => {
                let existed = load_pause_checkpoint(substrate, goal_id).is_some();
                if existed {
                    clear_pause_checkpoint(substrate, goal_id);
                }
                existed
            }
            None => false,
        };

        if let Some((_, handle)) = self.runs.remove(&goal_id) {
            handle.stop.store(true, Ordering::SeqCst);
            // A recovered terminal entry has no live loop task to abort.
            if let Some(task) = handle.task {
                task.abort();
            }
            delete_persisted_run(&self.store, goal_id);
            true
        } else {
            had_checkpoint
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
    ) -> bool
    where
        F: Fn(AgentId, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        // Hold `start_lock` for the whole stop→gen→spawn→insert sequence so a
        // concurrent `start()` for the same goal cannot observe the empty slot
        // this creates between the stop and the insert and spawn a second,
        // orphaned loop. The sequence is synchronous (no `.await`), so this std
        // guard is never held across an await point; `tokio::spawn` only
        // enqueues the task and does not block.
        let _guard = lock_goal_run_start_stop(&self.start_lock);

        // Goal deletion commits before it calls `stop()`, which takes this same
        // lock. Consequently either this read observes the deletion and no run
        // is created, or deletion waits for the insertion and then removes it.
        // There is no window where a deleted goal can leave an orphaned loop.
        if load_goal(&substrate, goal_id).is_none() {
            return false;
        }

        // Read any pause checkpoint BEFORE `stop_locked`, which clears it —
        // reading after would make every start look like a fresh one and
        // silently reset a paused goal back to iteration 0.
        let resume = self
            .substrate
            .as_ref()
            .and_then(|s| load_pause_checkpoint(s, goal_id));
        if let Some(r) = resume.as_ref() {
            info!(
                goal_id = %goal_id,
                agent_id = %agent_id,
                from_iteration = r.iteration,
                last_progress = r.last_progress,
                "Resuming goal run from persisted checkpoint"
            );
        }

        // Replace any prior run for this goal. `stop_locked` (not `stop`)
        // because we already hold `start_lock`, which is non-reentrant.
        self.stop_locked(goal_id);
        let now = Utc::now();
        let initial = GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: resume.as_ref().map(|r| r.iteration).unwrap_or(0),
            max_iterations,
            last_progress: resume.as_ref().map(|r| r.last_progress).unwrap_or(0),
            last_error: None,
            started_at: now,
            updated_at: now,
        };
        // Persist the initial Running row before the first tick so a crash
        // mid-tick still leaves a recoverable record at the next boot. The
        // new-run upsert also atomically replaces a terminal predecessor's
        // start time if one survived an earlier daemon restart.
        persist_new_run(&self.store, &initial);
        let state = Arc::new(Mutex::new(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let generation = self.next_gen.fetch_add(1, Ordering::SeqCst);

        let runs = self.runs.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
        let loop_pause = pause.clone();
        let loop_store = self.store.clone();

        // Do not let the task reach self-cleanup before its entry is in the
        // map. `tokio::spawn` may run the loop to completion on another worker
        // before this thread reaches the insert below: the loop's `remove_if`
        // then finds nothing, the insert lands a handle for a run that has
        // already ended, and that entry is never collected — `state()` reports
        // the run forever and the registry grows by one per occurrence.
        //
        // The window is a few instructions wide, but the exits that fit inside
        // it are the fast ones: a pre-signalled shutdown, or a goal deleted
        // between the API's read and this call, both of which end the loop
        // before its first agent turn.
        //
        // Gating on a oneshot rather than reordering the insert is what
        // `background.rs` already does for the same race (`installed_tx` /
        // `installed_rx` there). It makes the ordering a property of a
        // primitive instead of the adjacency of two statements, and it keeps
        // `task` populated for every live run — an insert-first ordering has to
        // leave `task: None` until a backfill, which is a second window and a
        // second reason for `stop()` to find nothing to abort.
        let (installed_tx, installed_rx) = tokio::sync::oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            // The sender is dropped without a send only if `start()` unwound
            // between the spawn and the insert, in which case there is no
            // entry to clean up and nothing to run.
            if installed_rx.await.is_err() {
                return;
            }
            run_loop(
                goal_id,
                agent_id,
                max_iterations,
                substrate,
                send_message,
                loop_state,
                loop_stop,
                loop_pause,
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
                pause,
                generation,
            },
        );
        // Release the loop now that its entry is visible. Nothing between the
        // spawn and here can observe the run, which is the point.
        let _ = installed_tx.send(());
        info!(goal_id = %goal_id, agent_id = %agent_id, max_iterations, "Goal run started");
        true
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
                            pause: Arc::new(AtomicBool::new(false)),
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
    pause: Arc<AtomicBool>,
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
        // Checked before shutdown: a pause that lands as the daemon goes down
        // must still checkpoint, and a paused row is not auto-resumed at the
        // next boot the way a shutdown-interrupted run is.
        if pause.load(Ordering::SeqCst) {
            break GoalRunPhase::Paused;
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

        let tick_secs = goal
            .tick_interval_secs
            .unwrap_or(DEFAULT_GOAL_TICK_INTERVAL_SECS)
            .clamp(MIN_GOAL_TICK_INTERVAL_SECS, MAX_GOAL_TICK_INTERVAL_SECS);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(tick_secs)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    interrupted_by_shutdown = true;
                    break GoalRunPhase::Stopped;
                }
            }
        }
    };

    let snapshot = {
        let mut s = state.lock().await;
        s.phase = final_phase;
        s.updated_at = Utc::now();
        s.clone()
    };

    if final_phase == GoalRunPhase::Paused {
        // A pause is a checkpoint, not an ending. The `goal_runs` mirror
        // tracks *active* runs (and its schema's CHECK constraint does not
        // even admit a `paused` phase), so the paused run leaves it, exactly
        // as a cancelled one does — otherwise boot recovery would auto-resume
        // a goal the operator deliberately suspended.
        persist_pause_checkpoint(&substrate, goal_id, &snapshot);
        delete_persisted_run(&store, goal_id);
        info!(
            goal_id = %goal_id,
            iteration = snapshot.iteration,
            last_progress = snapshot.last_progress,
            "Goal run paused — state checkpointed for resume"
        );
        return;
    }

    // Any other exit settles the run, so a checkpoint from an earlier pause
    // of the same goal must not survive to seed a later fresh start.
    clear_pause_checkpoint(&substrate, goal_id);

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
    fn poisoned_goal_run_start_stop_lock_recovers_and_clears_poison() {
        let lock = std::sync::Mutex::new(());
        let poison = std::panic::catch_unwind(|| {
            let _guard = lock.lock().unwrap();
            panic!("poison goal runner start/stop lock");
        });

        assert!(poison.is_err());
        assert!(lock.is_poisoned());
        let recovered = lock_goal_run_start_stop(&lock);
        assert!(lock.try_lock().is_err());
        drop(recovered);
        assert!(!lock.is_poisoned());
        let ordinary_guard = lock.lock().unwrap();
        drop(ordinary_guard);
    }

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
            tick_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Goals written before UUID `Option` fields were reliably serialised as
    /// `null` may store `agent_id` / `parent_id` as `""` instead. An empty
    /// string fails UUID parsing inside `serde_json::from_value`, and the
    /// caller's `.ok()` used to swallow that error and drop the whole goal —
    /// turning every `start` / `pause` / `resume` on it into a bare 500.
    #[test]
    fn load_goal_sanitizes_empty_string_uuid_fields() {
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let goal_id = GoalId::new();
        let now = Utc::now().to_rfc3339();
        let raw = serde_json::json!([{
            "id": goal_id.to_string(),
            "title": "Legacy goal",
            "description": "",
            "status": "in_progress",
            "progress": 0,
            "agent_id": "",
            "parent_id": "",
            "created_at": now,
            "updated_at": now,
        }]);
        substrate
            .structured_set(goals_storage_agent_id(), GOALS_STORAGE_KEY, raw)
            .unwrap();

        let loaded = load_goal(&substrate, goal_id)
            .expect("empty-string UUID fields must not drop the goal");
        assert_eq!(loaded.agent_id, None);
        assert_eq!(loaded.parent_id, None);
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
            Arc::new(AtomicBool::new(false)),
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

    #[tokio::test]
    async fn start_replaces_terminal_row_with_a_fresh_started_at() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;
        let stale_started = Utc::now() - chrono::Duration::days(1);
        store
            .save_run(&GoalRunRow {
                goal_id: goal_id.to_string(),
                agent_id: agent_id.to_string(),
                phase: GoalRunPhase::Stopped.to_string(),
                iteration: 5,
                max_iterations: 25,
                last_progress: 50,
                last_error: Some("Interrupted by daemon restart".to_string()),
                started_at: stale_started.to_rfc3339(),
                updated_at: stale_started.to_rfc3339(),
            })
            .unwrap();

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
        runner.start(
            goal_id,
            agent_id,
            25,
            substrate,
            |_agent_id, _message| async move {
                std::future::pending::<Result<String, String>>().await
            },
        );

        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        let started_at = chrono::DateTime::parse_from_rfc3339(&row.started_at)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert!(
            started_at > stale_started,
            "a new run must not inherit the predecessor's started_at"
        );

        assert!(runner.stop(goal_id));
    }

    /// A loop that ends immediately must not leave its registry entry behind.
    ///
    /// `start()` used to spawn the loop and register its `RunHandle`
    /// afterwards. A loop that finished inside that window ran its self-cleanup
    /// `remove_if` against a registry that did not hold it yet: the removal
    /// found nothing, the registration then landed a handle for a run that was
    /// already over, and nothing ever collected it — `state()` reported the run
    /// forever and the map grew by one every time it happened.
    ///
    /// Shutdown is pre-signalled so the loop breaks on its first check, before
    /// any store read or agent turn — the shortest path from spawn to
    /// `remove_if`, and so the likeliest interleaving to expose the old
    /// ordering.
    ///
    /// This test is a smoke check, not the guarantee. Racing the old ordering
    /// deliberately is a poor detector: measured against a replica of it, a
    /// finished loop left an entry behind on roughly 0.09% of rounds, so
    /// catching it reliably needs thousands of rounds and seconds of CI time.
    /// What actually rules the ordering out is the `installed` oneshot in
    /// `start()`: the loop cannot reach `remove_if` before the entry exists,
    /// because it is parked on the channel until the insert has happened. That
    /// is a property of the primitive, and the reason this test does not need
    /// to win a race to be meaningful.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_run_that_ends_immediately_leaves_no_entry_behind() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());

        for round in 0..25 {
            // Per round, so the `Running` rows a shutdown-interrupted run
            // leaves behind cannot accumulate across rounds and change what
            // later rounds are reading.
            let store = store_from(&substrate);
            // `true` = shutdown already signalled.
            let (_tx, rx) = watch::channel(true);
            let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
            let agent_id = AgentId::new();
            let goal = test_goal(agent_id);
            seed_goal(&substrate, &goal);
            let goal_id = goal.id;

            // Asserted, not discarded: `start()` returns false when the goal is
            // missing from the store, and then nothing is inserted and nothing
            // spawned — under which the assertion below would pass having
            // exercised nothing at all.
            assert!(
                runner.start(
                    goal_id,
                    agent_id,
                    10,
                    substrate.clone(),
                    |_agent_id, _message| async move { Ok::<String, String>(String::new()) },
                ),
                "round {round}: start() rejected a seeded goal, so this round tested nothing"
            );

            // Probe the registry directly rather than through `state()`:
            // `state()` answers `None` both for "no entry" and for "the state
            // lock was momentarily held", and the run loop takes that lock on
            // its way out. Conflating the two would let a transient lock read
            // as a clean registry.
            //
            // The budget bounds how long the spawned task takes to be
            // scheduled, so exhausting it on a loaded machine is possible in
            // principle — generous here because this exit path does two atomic
            // loads and a `remove_if`, with no store write.
            for _ in 0..500 {
                if !runner.runs.contains_key(&goal_id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            assert!(
                !runner.runs.contains_key(&goal_id),
                "round {round}: a finished goal loop left its registry entry behind"
            );
        }
    }

    /// The loop does not begin before its registry entry is visible.
    ///
    /// This is the invariant the smoke test above can only sample. It is
    /// checked without racing anything: the loop's first act is a
    /// `send_message` call, so a closure that reports what the registry held at
    /// that moment answers the question directly. If the `installed` gate is
    /// removed, this fails deterministically rather than 9 runs out of 10.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_loop_does_not_run_before_its_entry_is_registered() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let (_tx, rx) = watch::channel(false);
        let runner = Arc::new(GoalRunner::new_with_store(
            rx,
            store.clone(),
            substrate.clone(),
        ));
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<bool>();
        let probe = runner.clone();
        assert!(runner.start(
            goal_id,
            agent_id,
            1,
            substrate.clone(),
            move |_agent_id, _message| {
                // Report registry visibility from inside the first turn, then
                // park: the loop must not finish while the assertion runs.
                let _ = seen_tx.send(probe.runs.contains_key(&goal_id));
                async move { std::future::pending::<Result<String, String>>().await }
            },
        ));

        let seen = seen_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the loop must reach its first turn");
        assert!(
            seen,
            "the loop ran before its RunHandle was registered: a fast exit here would self-clean against an empty registry and strand the entry the insert lands afterwards"
        );
        assert!(runner.stop(goal_id));
    }

    #[tokio::test]
    async fn start_rejects_a_goal_missing_from_the_shared_store() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();
        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

        let started =
            runner.start(
                goal_id,
                agent_id,
                25,
                substrate,
                |_agent_id, _message| async move {
                    std::future::pending::<Result<String, String>>().await
                },
            );

        assert!(!started);
        assert!(runner.state(goal_id).is_none());
        assert!(store.get_run(&goal_id.to_string()).unwrap().is_none());
    }

    #[test]
    fn recover_stale_run_marks_it_stopped_at_boot() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
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
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

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
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
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
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

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
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
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
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());
        let recovered = runner.recover_stale_runs(Duration::from_secs(600));
        assert!(recovered.is_empty(), "a fresh run must not be recovered");

        // Row stays Running, untouched.
        let row = store.get_run(&goal_id.to_string()).unwrap().unwrap();
        assert_eq!(row.phase, GoalRunPhase::Running.to_string());
        assert!(row.last_error.is_none());
    }

    // --- Concurrent-start atomicity (finding #8) ---

    /// Two `start()` calls racing on the same goal must never leave a second,
    /// orphaned loop running. Before the `start_lock` fix the non-atomic
    /// stop→spawn→insert let both racing calls pass their (no-op) stop while the
    /// slot was empty, spawn two loops, and have the second `insert` overwrite
    /// the first's handle — orphaning the first loop, which `stop()` could then
    /// never reach (it only aborts the currently-mapped generation) and which
    /// kept issuing agent turns invisibly.
    ///
    /// Detection: each turn registers its loop as "live" (an RAII guard that
    /// decrements on task abort) and then parks. After the racing starts settle,
    /// `stop()` cancels the single mapped run; if an orphan slipped through it is
    /// not in the map, so `stop()` cannot abort it and `live` never returns to
    /// zero. We do NOT assert a peak of one concurrent loop: `JoinHandle::abort`
    /// is asynchronous, so during a legitimate replace the outgoing loop can
    /// still be parked (live) when the incoming one registers — a transient the
    /// fix does not (and need not) eliminate. The load-bearing invariant is that
    /// no loop survives `stop()`. Repeated over many rounds because the race is
    /// timing-dependent; without the fix it manifests within a few rounds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_starts_never_leave_an_orphan_loop() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let runner = Arc::new(GoalRunner::new(rx));

        for round in 0..30 {
            // Fresh counter + gate per round so state never leaks between them.
            let live = Arc::new(AtomicU64::new(0));
            let gate = Arc::new(tokio::sync::Notify::new());

            // Each turn registers the loop as live and then blocks forever on
            // `gate` (simulating a long agent turn). The RAII `Dec` guard is
            // held across the await, so an aborted loop still decrements `live`.
            let send = {
                let live = live.clone();
                let gate = gate.clone();
                move |_a: AgentId, _p: String| {
                    let live = live.clone();
                    let gate = gate.clone();
                    async move {
                        struct Dec(Arc<AtomicU64>);
                        impl Drop for Dec {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                        live.fetch_add(1, Ordering::SeqCst);
                        let _dec = Dec(live.clone());
                        gate.notified().await;
                        Ok::<String, String>("GOAL_PROGRESS: 1".to_string())
                    }
                }
            };

            // Two genuinely-parallel starts (spawned, since `start()` is
            // synchronous — `join!` alone would run them sequentially).
            let r1 = runner.clone();
            let r2 = runner.clone();
            let s1 = send.clone();
            let s2 = send.clone();
            let sub1 = substrate.clone();
            let sub2 = substrate.clone();
            let h1 = tokio::spawn(async move {
                r1.start(goal_id, agent_id, 100, sub1, s1);
            });
            let h2 = tokio::spawn(async move {
                r2.start(goal_id, agent_id, 100, sub2, s2);
            });
            let _ = tokio::join!(h1, h2);

            // Wait for at least one loop to reach `send_message`, then give a
            // possible second (orphan) loop time to reach it too.
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while live.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;

            // Stop the (single, mapped) run. If an orphan exists it is not in
            // the map, so `stop()` cannot reach it and `live` never returns to 0.
            runner.stop(goal_id);
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while live.load(Ordering::SeqCst) != 0 && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            assert_eq!(
                live.load(Ordering::SeqCst),
                0,
                "round {round}: an orphaned goal loop survived stop()"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Pause / resume (#5744 follow-up)
    // -----------------------------------------------------------------------

    /// Pausing must checkpoint the iteration count and progress the loop had
    /// reached, and a later `start()` must resume from it rather than restart.
    #[tokio::test]
    async fn pause_checkpoints_and_start_resumes_from_it() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let mut goal = test_goal(agent_id);
        // Fastest allowed cadence so the test doesn't wait through the 2s
        // default while the loop sleeps between the tick landing and the
        // next top-of-loop pause check.
        goal.tick_interval_secs = Some(MIN_GOAL_TICK_INTERVAL_SECS);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store.clone(), substrate.clone());

        // Each turn bumps a counter; once two turns have landed, request a
        // pause so the loop checkpoints partway through.
        let turns = Arc::new(AtomicU64::new(0));
        let send = {
            let turns = turns.clone();
            move |_a: AgentId, _p: String| {
                let turns = turns.clone();
                async move {
                    turns.fetch_add(1, Ordering::SeqCst);
                    Ok("GOAL_PROGRESS: 40".to_string())
                }
            }
        };

        assert!(runner.start(goal_id, agent_id, 100, substrate.clone(), send));

        // Wait for at least one tick to land, then pause.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while turns.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(runner.pause(goal_id), "pause must signal the live run");

        // Wait for the loop to actually reach the Paused phase.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if runner
                .state(goal_id)
                .is_some_and(|s| s.phase == GoalRunPhase::Paused)
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "pause never landed");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let paused_iteration = runner.state(goal_id).unwrap().iteration;
        assert!(paused_iteration >= 1, "at least one turn must be counted");

        // The `goal_runs` mirror tracks active runs only, so a paused run
        // leaves no row there.
        assert!(store.get_run(&goal_id.to_string()).unwrap().is_none());

        // Resuming (start again) must continue from the checkpoint, not 0.
        let send_pending = |_a: AgentId, _m: String| async move {
            std::future::pending::<Result<String, String>>().await
        };
        assert!(runner.start(goal_id, agent_id, 100, substrate.clone(), send_pending));
        let resumed = runner.state(goal_id).unwrap();
        assert_eq!(resumed.phase, GoalRunPhase::Running);
        assert_eq!(resumed.iteration, paused_iteration);
        assert_eq!(resumed.last_progress, 40);

        assert!(runner.stop(goal_id));
    }

    /// `pause()` on a goal with no live run reports false rather than
    /// fabricating a checkpoint out of nothing.
    #[test]
    fn pause_on_an_idle_goal_reports_false() {
        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new(rx);
        assert!(!runner.pause(GoalId::new()));
    }

    /// Cancelling a paused goal must discard its checkpoint — the whole point
    /// of `stop` remaining the terminal verb.
    #[tokio::test]
    async fn stop_discards_a_pause_checkpoint_so_the_next_start_is_fresh() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let goal_id = goal.id;

        persist_pause_checkpoint(
            &substrate,
            goal_id,
            &GoalRunState {
                goal_id,
                agent_id,
                phase: GoalRunPhase::Paused,
                iteration: 7,
                max_iterations: 25,
                last_progress: 65,
                last_error: None,
                started_at: Utc::now(),
                updated_at: Utc::now(),
            },
        );

        let (_tx, rx) = watch::channel(false);
        let runner = GoalRunner::new_with_store(rx, store, substrate.clone());

        // The checkpoint is observable even with no live loop.
        let observed = runner.state(goal_id).expect("checkpoint must be visible");
        assert_eq!(observed.phase, GoalRunPhase::Paused);
        assert_eq!(observed.iteration, 7);

        // Cancel reaches the checkpoint even with no live task.
        assert!(
            runner.stop(goal_id),
            "cancelling a paused goal must report that it discarded something"
        );
        assert!(load_pause_checkpoint(&substrate, goal_id).is_none());
        assert!(runner.state(goal_id).is_none());
    }

    /// The loop must honour a per-goal `tick_interval_secs` override instead
    /// of the hard-wired default.
    #[tokio::test]
    async fn run_loop_waits_the_goals_configured_tick_interval() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let mut goal = test_goal(agent_id);
        // Single iteration, minimum cadence: one sleep of exactly
        // MIN_GOAL_TICK_INTERVAL_SECS, not the 2s DEFAULT_GOAL_TICK_INTERVAL_SECS
        // a missing override would fall back to.
        goal.tick_interval_secs = Some(MIN_GOAL_TICK_INTERVAL_SECS);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 1);

        let send = |_a: AgentId, _p: String| async move { Ok("GOAL_PROGRESS: 5".to_string()) };

        let began = std::time::Instant::now();
        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;
        let elapsed = began.elapsed();

        assert_eq!(state.lock().await.phase, GoalRunPhase::MaxIterationsReached);
        assert!(
            elapsed >= Duration::from_secs(MIN_GOAL_TICK_INTERVAL_SECS),
            "expected at least {MIN_GOAL_TICK_INTERVAL_SECS}s of tick sleep, took {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(DEFAULT_GOAL_TICK_INTERVAL_SECS),
            "took as long as the {DEFAULT_GOAL_TICK_INTERVAL_SECS}s default — the goal's \
             tick_interval_secs override was not honoured, took {elapsed:?}"
        );
    }
}
