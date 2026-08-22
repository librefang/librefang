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
//! GOAL_LEARNED: ...  (optional — a reusable lesson worth keeping)
//! ```
//!
//! This keeps the v1 runner entirely kernel-side: no new runtime tool, no
//! tool-registry / capability-permission surgery. The parsing is forgiving
//! (case-insensitive, last marker wins) so an agent that forgets the marker
//! simply keeps iterating to the cap rather than failing.
//!
//! ## Loop engineering (opt-in, per goal)
//!
//! `GOAL_DONE` is the agent's own opinion of its own work, and a loop that
//! stops the moment the worker says it is finished has no independent check
//! in it at all. Setting `loop_engineering` on the goal adds two:
//!
//! - **A verifier agent** (`Goal::verify_agent_id`). Each iteration's output
//!   goes to it for a `VERDICT: PASS|FAIL|NEEDS_REWORK` judgement. On a
//!   rejection the generator is asked to rework the output *with the
//!   verifier's stated reason*, up to `verify_max_retries` times. Until the
//!   verifier passes it, `GOAL_DONE` does not end the run.
//! - **An evaluator model** (`Goal::evaluator_model`). A single cheap
//!   yes/no read of the goal against the latest output, which can conclude
//!   the goal is met even when the agent never emitted `GOAL_DONE`.
//!
//! Both are optional and both are inert unless `loop_engineering` is set, so
//! a goal that does not ask for them runs exactly the loop it ran before —
//! same prompt, same number of LLM calls.
//!
//! Sub-agents are delegated, not conjured: the prompt tells the agent to use
//! its own `agent_spawn` / `agent_send` tools, which run under the agent's
//! own capability grants. The runner never provisions an agent behind the
//! operator's back.

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

/// Consecutive non-rate-limit tick failures before the loop gives up.
///
/// The rate-limit breaker above only catches a quota-exhausted provider. A
/// deleted agent, a revoked API key or a downed network fails every tick with
/// the same error forever, and without a second breaker the loop spends the
/// whole `max_iterations` budget rediscovering it. Kept separate from
/// `MAX_RATE_LIMIT_STREAK` so a transient rate-limit does not also count
/// toward this one.
const MAX_ERROR_STREAK: u32 = 5;

/// Marker prefix for a reusable lesson the agent wants to keep.
const LEARNED_MARKER: &str = "GOAL_LEARNED:";

/// How many captured learnings are replayed into the next iteration's prompt.
/// Recent ones are the relevant ones, and the whole list would grow without
/// bound across a long run.
const LEARNINGS_IN_PROMPT: usize = 6;

/// Rework rounds allowed per iteration when the caller does not pick a number.
/// Each round is a verifier turn plus a generator turn, so the default stays
/// small.
const DEFAULT_VERIFY_MAX_RETRIES: u32 = 3;

/// Structured-memory key prefix under which a run's captured learnings are
/// stored, alongside the goals document itself.
const LEARNINGS_KEY_PREFIX: &str = "goal_learnings_";

/// Result of parsing one agent reply for goal-control markers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedTick {
    /// Progress value (0-100) if the agent emitted `GOAL_PROGRESS:`.
    pub progress: Option<u8>,
    /// The agent signalled completion (`GOAL_DONE`).
    pub done: bool,
    /// The agent signalled it is blocked (`GOAL_BLOCKED`).
    pub blocked: bool,
    /// Lessons the agent captured with `GOAL_LEARNED: <text>`, in the order
    /// they appeared.
    pub learnings: Vec<String>,
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
        } else if upper.starts_with(LEARNED_MARKER) {
            // Slice the ORIGINAL line, not the uppercased copy: the lesson is
            // prose that gets replayed into later prompts and written into a
            // skill, and SHOUTING IT BACK loses the agent's own wording.
            // `to_ascii_uppercase` is byte-length preserving and the marker is
            // ASCII, so the marker's length is a valid boundary in `t` too.
            let learning = t[LEARNED_MARKER.len()..].trim();
            if !learning.is_empty() {
                out.learnings.push(learning.to_string());
            }
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
///
/// `loop_engineering` adds the sections that only make sense when the extra
/// machinery is switched on; with it off the prompt is byte-identical to the
/// plain loop's, so an existing goal's prompt cache is not invalidated.
/// `has_verifier` tells the agent its output will be judged, which is worth
/// saying explicitly — it changes how much it should claim.
pub fn build_goal_prompt(
    goal: &Goal,
    iteration: u32,
    max_iterations: u32,
    loop_engineering: bool,
    has_verifier: bool,
    learnings: &[String],
) -> String {
    let mut extra = String::new();
    if loop_engineering {
        if !learnings.is_empty() {
            extra.push_str("\n\n## What earlier iterations learned\n");
            // Chronological within the window: the agent reads them as a
            // sequence, and the order is stable for a given run so the prompt
            // prefix stays cacheable across iterations that add nothing.
            let skip = learnings.len().saturating_sub(LEARNINGS_IN_PROMPT);
            for l in &learnings[skip..] {
                extra.push_str("- ");
                extra.push_str(l);
                extra.push('\n');
            }
        }
        extra.push_str(if has_verifier {
            "\n\n## Loop engineering\n\
             A separate verifier agent judges this output before it counts. If it \
             rejects the work you will be asked to rework it with the reason given, \
             so claiming more than you did costs you an extra round rather than \
             buying you one.\n\
             Delegate genuinely separable work to sub-agents with your `agent_spawn` \
             and `agent_send` tools.\n\
             When you learn something reusable — a pattern, a pitfall, a technique — \
             record it as `GOAL_LEARNED: <one sentence>` so later iterations start \
             from it."
        } else {
            "\n\n## Loop engineering\n\
             Delegate genuinely separable work to sub-agents with your `agent_spawn` \
             and `agent_send` tools.\n\
             When you learn something reusable — a pattern, a pitfall, a technique — \
             record it as `GOAL_LEARNED: <one sentence>` so later iterations start \
             from it."
        });
    }
    format!(
        "[LONG-HORIZON GOAL] You are autonomously pursuing a goal across multiple turns.\n\
         Goal: {title}\n\
         Description: {description}\n\
         Current progress: {progress}%\n\
         Iteration: {iter} of {max}{extra}\n\n\
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

/// Build the prompt that asks the verifier for a verdict on `output`.
fn build_verdict_prompt(goal: &Goal, output: &str) -> String {
    format!(
        "[GOAL VERIFICATION] Judge the work below against the goal. You are the \
         independent check on it — do not restate it approvingly, decide whether it \
         actually advances the goal.\n\n\
         Goal: {title}\n\
         Description: {description}\n\n\
         Reply with exactly two lines:\n\
         VERDICT: PASS|FAIL|NEEDS_REWORK\n\
         REASON: <one sentence>\n\n\
         Work to judge:\n{output}",
        title = goal.title,
        description = if goal.description.is_empty() {
            "(none)"
        } else {
            &goal.description
        },
    )
}

/// Build the prompt that sends the verifier's rejection back to the generator.
fn build_rework_prompt(goal: &Goal, verdict: &str) -> String {
    format!(
        "[GOAL REWORK] The verifier rejected your last output on goal \"{title}\". \
         Its verdict was:\n\n{verdict}\n\n\
         Address the stated reason and produce the corrected work. Emit the same \
         `GOAL_PROGRESS:` / `GOAL_DONE` / `GOAL_BLOCKED` markers as usual — the \
         reworked reply replaces the rejected one.",
        title = goal.title,
        verdict = verdict.trim(),
    )
}

/// Read a verifier reply as pass / not-pass.
///
/// Only an explicit `VERDICT: PASS` passes. Anything else — `FAIL`,
/// `NEEDS_REWORK`, a refusal, an empty string, or prose that never reaches a
/// verdict — is a rejection, because the gate exists to be closed by default.
///
/// The verdict is compared as a whole token rather than a prefix, so a model
/// that echoes the instruction line back verbatim
/// (`VERDICT: PASS|FAIL|NEEDS_REWORK`) is not read as having chosen `PASS`.
fn verdict_is_pass(verdict: &str) -> bool {
    verdict.to_ascii_uppercase().lines().any(|line| {
        let Some(rest) = line.trim().strip_prefix("VERDICT:") else {
            return false;
        };
        rest.split_whitespace().next() == Some("PASS")
    })
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
            start_lock: std::sync::Mutex::new(()),
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
            start_lock: std::sync::Mutex::new(()),
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
        // Serialize against `start()` so the two never interleave on the same
        // goal id. The critical section is synchronous, so this std guard never
        // spans an await point. Poison is irrelevant for a `Mutex<()>` used
        // purely for mutual exclusion — recover the guard rather than panic.
        let _guard = self
            .start_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.stop_locked(goal_id)
    }

    /// Stop body assuming the caller already holds `start_lock`. Split out so
    /// `start()` can run it inside its own critical section without re-locking
    /// the non-reentrant `start_lock` (which would deadlock).
    fn stop_locked(&self, goal_id: GoalId) -> bool {
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
    /// The remaining arguments configure loop engineering and are inert when
    /// `loop_engineering` is false: `evaluate_goal` judges whether the goal
    /// condition is met (only called when `evaluator_model` is set),
    /// `on_learnings_captured` receives the run's `GOAL_LEARNED:` lessons once
    /// the loop ends, and `verify_agent_id` / `verify_max_retries` configure
    /// the verifier gate.
    ///
    /// Replaces any existing run for the same goal.
    #[allow(clippy::too_many_arguments)]
    pub fn start<F, Fut, L, E, Efut>(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: u32,
        substrate: Arc<MemorySubstrate>,
        send_message: F,
        on_learnings_captured: L,
        evaluate_goal: E,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
    ) where
        F: Fn(AgentId, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
        L: FnOnce(Vec<String>) + Send + 'static,
        E: Fn(String, String) -> Efut + Send + Sync + 'static,
        Efut: std::future::Future<Output = Result<bool, String>> + Send + 'static,
    {
        // Hold `start_lock` for the whole stop→gen→spawn→insert sequence so a
        // concurrent `start()` for the same goal cannot observe the empty slot
        // this creates between the stop and the insert and spawn a second,
        // orphaned loop. The sequence is synchronous (no `.await`), so this std
        // guard is never held across an await point; `tokio::spawn` only
        // enqueues the task and does not block.
        let _guard = self
            .start_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Replace any prior run for this goal. `stop_locked` (not `stop`)
        // because we already hold `start_lock`, which is non-reentrant.
        self.stop_locked(goal_id);
        let now = Utc::now();
        let initial = GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            // The verifier is only ever consulted under loop engineering, so
            // do not record one on a run that is not using it — a stored
            // verifier the loop ignores reads as a live gate on the run API.
            verify_agent_id: if loop_engineering {
                verify_agent_id
            } else {
                None
            },
            verify_max_retries: if loop_engineering {
                verify_max_retries
                    .unwrap_or(DEFAULT_VERIFY_MAX_RETRIES)
                    .max(1)
            } else {
                0
            },
            evaluator_model: if loop_engineering {
                evaluator_model
            } else {
                None
            },
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
        let generation = self.next_gen.fetch_add(1, Ordering::SeqCst);

        let runs = self.runs.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
        let loop_store = self.store.clone();

        // Register the handle BEFORE spawning, so the spawned task's
        // self-cleanup always has an entry to find.
        //
        // Inserting afterwards leaves a window in which the loop can run to
        // completion on another worker thread and execute its `remove_if`
        // before this thread reaches the insert. The removal then finds
        // nothing, the insert lands a handle for a loop that has already
        // ended, and that entry is never collected: `state()` reports the run
        // forever and the registry grows by one per occurrence. The window is
        // small but the exits that fit inside it are the fast ones — a
        // pre-signalled shutdown, or a goal deleted between the API's read and
        // this call, both of which end the loop before its first agent turn.
        self.runs.insert(
            goal_id,
            RunHandle {
                task: None,
                state,
                stop,
                generation,
            },
        );

        let task = tokio::spawn(async move {
            run_loop(
                goal_id,
                agent_id,
                max_iterations,
                substrate,
                send_message,
                on_learnings_captured,
                evaluate_goal,
                loop_engineering,
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

        // Backfill the join handle so `stop()` can abort the task.
        //
        // The entry is gone when the loop already finished and cleaned up,
        // which is exactly the case the insert-first ordering exists to handle:
        // dropping `task` there detaches a `JoinHandle` whose task has already
        // returned, which is correct — there is nothing left to abort. The
        // generation check keeps a replacement run's handle from being
        // overwritten. `stop()` cannot interleave here: it takes `start_lock`,
        // which this function holds for the whole sequence.
        if let Some(mut entry) = self.runs.get_mut(&goal_id) {
            if entry.generation == generation {
                entry.task = Some(task);
            }
        }
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
                        // The placeholder is terminal — no loop is running, so
                        // there is no verifier or evaluator to describe. The
                        // persisted row never carried them either: the
                        // loop-engineering configuration lives on the goal,
                        // and a resumed run would read it back from there.
                        verify_agent_id: None,
                        verify_max_retries: 0,
                        evaluator_model: None,
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
async fn run_loop<F, Fut, L, E, Efut>(
    goal_id: GoalId,
    agent_id: AgentId,
    max_iterations: u32,
    substrate: Arc<MemorySubstrate>,
    send_message: F,
    on_learnings_captured: L,
    evaluate_goal: E,
    loop_engineering: bool,
    state: Arc<Mutex<GoalRunState>>,
    stop: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
    store: Option<GoalRunStore>,
) where
    F: Fn(AgentId, String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
    L: FnOnce(Vec<String>) + Send,
    E: Fn(String, String) -> Efut + Send + Sync,
    Efut: std::future::Future<Output = Result<bool, String>> + Send,
{
    // Read the loop-engineering configuration once. It is fixed for the run —
    // `start()` writes it before spawning and nothing mutates it afterwards —
    // so re-locking per iteration would only add contention with `state()`,
    // whose `try_lock` reports `running: false` whenever it loses the race.
    let (verify_agent_id, verify_max_retries, has_evaluator) = {
        let s = state.lock().await;
        (
            s.verify_agent_id,
            s.verify_max_retries.max(1),
            s.evaluator_model.is_some(),
        )
    };

    let mut iteration: u32 = 0;
    let mut rate_limit_streak: u32 = 0;
    let mut error_streak: u32 = 0;
    // Lessons captured via `GOAL_LEARNED:` across the whole run. Replayed into
    // each iteration's prompt and handed to `on_learnings_captured` at the end.
    let mut learnings: Vec<String> = Vec::new();
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

        let prompt = build_goal_prompt(
            &goal,
            iteration,
            max_iterations,
            loop_engineering,
            verify_agent_id.is_some(),
            &learnings,
        );
        debug!(goal_id = %goal_id, iteration, "Goal run: sending tick");

        match send_message(agent_id, prompt).await {
            Ok(reply) => {
                rate_limit_streak = 0;
                error_streak = 0;
                let mut output = reply;
                let mut parsed = parse_tick(&output);
                if loop_engineering {
                    learnings.append(&mut parsed.learnings);
                }

                // Verifier gate. Each rejection sends the work back to the
                // generator WITH the verifier's reason: re-asking the same
                // verifier about the same unchanged text would only replay the
                // same verdict, so a retry that does not regenerate is not a
                // retry. The reworked reply carries its own markers and
                // replaces the rejected one outright.
                let mut verified = true;
                let mut rejection: Option<String> = None;
                if let Some(verifier) = verify_agent_id {
                    let mut round: u32 = 0;
                    loop {
                        let verdict = match send_message(
                            verifier,
                            build_verdict_prompt(&goal, &output),
                        )
                        .await
                        {
                            Ok(v) => v,
                            Err(e) => {
                                // An unreachable verifier is an open gate, and
                                // an open gate is the failure mode this whole
                                // mechanism exists to prevent. Treat it as a
                                // rejection rather than waving the work through.
                                warn!(goal_id = %goal_id, verifier = %verifier, error = %e,
                                      "Goal run: verifier call failed; treating as a rejection");
                                round = round.saturating_add(1);
                                if round >= verify_max_retries {
                                    verified = false;
                                    rejection = Some(format!("verifier unreachable: {e}"));
                                    break;
                                }
                                continue;
                            }
                        };
                        if verdict_is_pass(&verdict) {
                            info!(goal_id = %goal_id, iteration, rework_rounds = round,
                                  "Goal run: verifier passed the iteration");
                            break;
                        }
                        round = round.saturating_add(1);
                        if round >= verify_max_retries {
                            warn!(goal_id = %goal_id, iteration, rework_rounds = round,
                                  verdict = %verdict.trim(),
                                  "Goal run: verifier still rejecting after the retry budget");
                            verified = false;
                            rejection = Some(verdict.trim().to_string());
                            break;
                        }
                        info!(goal_id = %goal_id, iteration, rework_rounds = round,
                              verdict = %verdict.trim(), "Goal run: verifier rejected; reworking");
                        match send_message(agent_id, build_rework_prompt(&goal, &verdict)).await {
                            Ok(reworked) => {
                                output = reworked;
                                parsed = parse_tick(&output);
                                if loop_engineering {
                                    learnings.append(&mut parsed.learnings);
                                }
                            }
                            Err(e) => {
                                warn!(goal_id = %goal_id, error = %e,
                                      "Goal run: rework turn failed; keeping the rejected output");
                                verified = false;
                                rejection = Some(format!("rework turn failed: {e}"));
                                break;
                            }
                        }
                    }
                }

                // Independent completion check. Only consulted when the goal
                // asked for one, so the plain loop keeps costing exactly one
                // LLM call per iteration.
                let evaluator_done = if verified && loop_engineering && has_evaluator {
                    match evaluate_goal(goal.description.clone(), output.clone()).await {
                        Ok(done) => {
                            if done {
                                info!(goal_id = %goal_id, iteration,
                                      "Goal run: evaluator judged the goal met");
                            }
                            done
                        }
                        Err(e) => {
                            // Fall back to the agent's own marker rather than
                            // stalling the run on an evaluator outage.
                            warn!(goal_id = %goal_id, error = %e,
                                  "Goal run: evaluator call failed; falling back to the agent's marker");
                            false
                        }
                    }
                } else {
                    false
                };

                // `GOAL_DONE` only ends the run once the verifier has passed
                // the work it is attached to. Without this the gate would be
                // decorative: the agent could close its own goal by asserting
                // completion in an output the verifier had just rejected.
                let done = verified && (parsed.done || evaluator_done);
                let new_status = if done {
                    Some(GoalStatus::Completed)
                } else {
                    Some(GoalStatus::InProgress)
                };
                let new_progress = if done { Some(100) } else { parsed.progress };
                patch_goal(&substrate, goal_id, new_progress, new_status);

                // Release before persist_run: state()'s try_lock returns None (→ running:false) while held.
                let snapshot = {
                    let mut s = state.lock().await;
                    s.iteration = iteration + 1;
                    if let Some(p) = new_progress {
                        s.last_progress = p;
                    }
                    // Surface an exhausted verifier budget on the run API. The
                    // tick itself succeeded, so leaving `last_error` clear
                    // would show an operator a healthy run making no progress
                    // with nothing to explain why.
                    s.last_error = rejection.as_ref().map(|r| {
                        format!("Iteration {} did not pass verification: {r}", iteration + 1)
                    });
                    s.updated_at = Utc::now();
                    s.clone()
                };
                // Mirror the post-iteration state to the durable store so a
                // crash before the next tick still leaves a recoverable row.
                persist_run(&store, &snapshot);

                if done {
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
                        error_streak = error_streak.saturating_add(1);
                        warn!(
                            goal_id = %goal_id,
                            consecutive_errors = error_streak,
                            "Goal run: tick failed",
                        );
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
                if error_streak >= MAX_ERROR_STREAK {
                    warn!(
                        goal_id = %goal_id,
                        consecutive_errors = error_streak,
                        "Goal run: giving up after repeated tick failures",
                    );
                    break GoalRunPhase::Stopped;
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
    // Lessons outlive the run that produced them — that is the whole point of
    // capturing them. Write them to the shared store first (durable, queryable,
    // independent of what the caller does with them), then hand them to the
    // caller's hook.
    if loop_engineering && !learnings.is_empty() {
        let key = format!("{LEARNINGS_KEY_PREFIX}{goal_id}");
        let count = learnings.len();
        let payload = serde_json::json!({
            "goal_id": goal_id.to_string(),
            "learnings": learnings.clone(),
            "captured_at": Utc::now().to_rfc3339(),
        });
        match substrate.structured_set(goals_storage_agent_id(), &key, payload) {
            Ok(()) => info!(goal_id = %goal_id, count, "Goal run: persisted captured learnings"),
            Err(e) => warn!(goal_id = %goal_id, error = %e,
                            "Goal run: failed to persist captured learnings"),
        }
        on_learnings_captured(learnings);
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

    /// `on_learnings_captured` for a run that is not using loop engineering.
    /// The loop never calls it, so it exists only to satisfy the type.
    fn no_learnings_hook(_learnings: Vec<String>) {}

    /// `evaluate_goal` for a run that has no evaluator model. The loop skips
    /// the call entirely; `Ok(false)` makes an accidental call visible as
    /// "not done" rather than silently completing the goal.
    async fn no_evaluator(_goal: String, _output: String) -> Result<bool, String> {
        Ok(false)
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
            loop_engineering: false,
            verify_agent_id: None,
            evaluator_model: None,
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
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            verify_agent_id: None,
            verify_max_retries: 0,
            evaluator_model: None,
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    /// A run state configured for loop engineering with a verifier attached.
    fn mk_verified_state(
        goal_id: GoalId,
        agent_id: AgentId,
        verifier: AgentId,
        max_iterations: u32,
        verify_max_retries: u32,
    ) -> Arc<Mutex<GoalRunState>> {
        Arc::new(Mutex::new(GoalRunState {
            goal_id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations,
            last_progress: 0,
            last_error: None,
            verify_agent_id: Some(verifier),
            verify_max_retries,
            evaluator_model: None,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
            no_learnings_hook,
            no_evaluator,
            false,
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
                verify_agent_id: None,
                verify_max_retries: 0,
                evaluator_model: None,
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
            no_learnings_hook,
            no_evaluator,
            false,
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

    #[tokio::test]
    async fn start_replaces_terminal_row_with_a_fresh_started_at() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let store = store_from(&substrate);
        let goal_id = GoalId::new();
        let agent_id = AgentId::new();
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
        let runner = GoalRunner::new_with_store(rx, store.clone());
        runner.start(
            goal_id,
            agent_id,
            25,
            substrate,
            |_agent_id, _message| async move {
                std::future::pending::<Result<String, String>>().await
            },
            no_learnings_hook,
            no_evaluator,
            false,
            None,
            None,
            None,
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
                r1.start(
                    goal_id,
                    agent_id,
                    100,
                    sub1,
                    s1,
                    no_learnings_hook,
                    no_evaluator,
                    false,
                    None,
                    None,
                    None,
                );
            });
            let h2 = tokio::spawn(async move {
                r2.start(
                    goal_id,
                    agent_id,
                    100,
                    sub2,
                    s2,
                    no_learnings_hook,
                    no_evaluator,
                    false,
                    None,
                    None,
                    None,
                );
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

    // -----------------------------------------------------------------
    // Loop engineering
    // -----------------------------------------------------------------

    #[test]
    fn parse_tick_keeps_a_learning_in_the_agents_own_words() {
        // The marker is matched case-insensitively like every other marker,
        // but the lesson itself is prose that gets replayed into later prompts
        // and written into a skill. Uppercasing it would corrupt both.
        let p = parse_tick("goal_learned: Retry the API with backoff, not immediately");
        assert_eq!(
            p.learnings,
            vec!["Retry the API with backoff, not immediately".to_string()]
        );
    }

    #[test]
    fn parse_tick_collects_every_learning_and_ignores_empty_ones() {
        let p = parse_tick(
            "GOAL_LEARNED: first lesson\n\
             GOAL_LEARNED:   \n\
             working…\n\
             GOAL_LEARNED: second lesson\n\
             GOAL_PROGRESS: 40",
        );
        assert_eq!(p.learnings, vec!["first lesson", "second lesson"]);
        assert_eq!(p.progress, Some(40));
    }

    #[test]
    fn verdict_is_pass_only_on_an_explicit_pass() {
        assert!(verdict_is_pass("VERDICT: PASS\nREASON: it works"));
        assert!(verdict_is_pass("verdict: pass"));
        assert!(verdict_is_pass("Some preamble.\nVERDICT: PASS"));

        assert!(!verdict_is_pass("VERDICT: FAIL\nREASON: no tests"));
        assert!(!verdict_is_pass("VERDICT: NEEDS_REWORK"));
        assert!(!verdict_is_pass(""));
        assert!(!verdict_is_pass("Looks good to me!"));
        // A model that parrots the instruction line has not chosen anything.
        assert!(!verdict_is_pass("VERDICT: PASS|FAIL|NEEDS_REWORK"));
    }

    #[test]
    fn plain_goal_prompt_is_unchanged_by_the_new_sections() {
        let goal = test_goal(AgentId::new());
        let plain = build_goal_prompt(&goal, 0, 10, false, false, &[]);
        // Even with learnings on hand and a verifier configured, a goal that
        // did not opt in must get the exact prompt it got before, or every
        // existing goal's provider-side prompt cache is invalidated for free.
        let with_ignored_extras =
            build_goal_prompt(&goal, 0, 10, false, true, &["a lesson".to_string()]);
        assert_eq!(plain, with_ignored_extras);
        assert!(!plain.contains("Loop engineering"));
        assert!(!plain.contains("GOAL_LEARNED"));
    }

    #[test]
    fn loop_engineering_prompt_announces_the_verifier_and_replays_learnings() {
        let goal = test_goal(AgentId::new());
        let lessons: Vec<String> = (1..=8).map(|i| format!("lesson {i}")).collect();

        let with_verifier = build_goal_prompt(&goal, 0, 10, true, true, &lessons);
        assert!(with_verifier.contains("verifier agent judges this output"));
        assert!(with_verifier.contains("GOAL_LEARNED"));
        // Only the most recent window is replayed, oldest first.
        assert!(!with_verifier.contains("lesson 2"));
        assert!(with_verifier.contains("lesson 3"));
        assert!(with_verifier.contains("lesson 8"));

        let without_verifier = build_goal_prompt(&goal, 0, 10, true, false, &lessons);
        assert!(!without_verifier.contains("verifier agent judges this output"));
        assert!(without_verifier.contains("GOAL_LEARNED"));
    }

    /// The gate has to be able to say no. An agent that claims `GOAL_DONE`
    /// while the verifier keeps rejecting the work must not close its own
    /// goal — that is the single failure mode the verifier exists to stop.
    #[tokio::test(start_paused = true)]
    async fn verifier_rejection_blocks_the_agents_own_goal_done() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 2, 2);

        let send = move |target: AgentId, _p: String| async move {
            if target == verifier {
                Ok("VERDICT: FAIL\nREASON: nothing was actually produced".to_string())
            } else {
                Ok("all finished\nGOAL_DONE".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            2,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(
            s.phase,
            GoalRunPhase::MaxIterationsReached,
            "a rejected iteration must not finish the run"
        );
        // The operator needs both halves: that verification is what blocked
        // the iteration, and the verifier's own stated reason — a bare "did
        // not pass" would leave them with a healthy-looking run making no
        // progress and nothing to act on.
        let last_error = s.last_error.as_deref().unwrap_or_default();
        assert!(
            last_error.contains("did not pass verification"),
            "the exhausted verifier budget must be visible to an operator, got {:?}",
            s.last_error
        );
        assert!(
            last_error.contains("nothing was actually produced"),
            "the verifier's reason must reach the operator, got {:?}",
            s.last_error
        );
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(stored.status, GoalStatus::InProgress);
        assert_ne!(stored.progress, 100);
    }

    #[tokio::test]
    async fn verifier_pass_lets_goal_done_finish_the_run() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 5, 2);

        let send = move |target: AgentId, _p: String| async move {
            if target == verifier {
                Ok("VERDICT: PASS\nREASON: the report is complete".to_string())
            } else {
                Ok("report written\nGOAL_DONE".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            5,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Finished);
        assert_eq!(s.last_error, None);
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(stored.status, GoalStatus::Completed);
        assert_eq!(stored.progress, 100);
    }

    /// A "retry" that re-asks the same verifier about the same unchanged text
    /// just replays the same verdict. The rejection has to go back to the
    /// generator, carrying the verifier's reason, and the reworked reply has
    /// to be what the verifier sees next.
    #[tokio::test]
    async fn verifier_rejection_sends_the_work_back_to_the_generator() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 1, 3);

        // The verifier rejects the first submission and accepts the reworked
        // one; the generator only emits GOAL_DONE after being asked to rework.
        let verifier_calls = Arc::new(AtomicU64::new(0));
        let rework_prompts = Arc::new(AtomicU64::new(0));
        let vc = verifier_calls.clone();
        let rp = rework_prompts.clone();
        let send = move |target: AgentId, prompt: String| {
            let vc = vc.clone();
            let rp = rp.clone();
            async move {
                if target == verifier {
                    let n = vc.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        assert!(
                            prompt.contains("first draft"),
                            "the verifier must judge the generator's output"
                        );
                        Ok("VERDICT: NEEDS_REWORK\nREASON: cite a source".to_string())
                    } else {
                        assert!(
                            prompt.contains("second draft"),
                            "the verifier must re-judge the REWORKED output, not the rejected one"
                        );
                        Ok("VERDICT: PASS\nREASON: sourced now".to_string())
                    }
                } else if prompt.contains("[GOAL REWORK]") {
                    assert!(
                        prompt.contains("cite a source"),
                        "the generator must be told WHY it was rejected"
                    );
                    rp.fetch_add(1, Ordering::SeqCst);
                    Ok("second draft\nGOAL_DONE".to_string())
                } else {
                    Ok("first draft".to_string())
                }
            }
        };

        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(
            rework_prompts.load(Ordering::SeqCst),
            1,
            "the generator must be re-prompted after a rejection"
        );
        assert_eq!(verifier_calls.load(Ordering::SeqCst), 2);
        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Finished);
    }

    /// A verifier that cannot be reached is an open gate, and an open gate is
    /// exactly what this mechanism exists to prevent. Its failure must count
    /// as a rejection, not as a pass.
    #[tokio::test(start_paused = true)]
    async fn an_unreachable_verifier_does_not_wave_the_work_through() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 1, 2);

        let send = move |target: AgentId, _p: String| async move {
            if target == verifier {
                Err("verifier agent not found".to_string())
            } else {
                Ok("done and dusted\nGOAL_DONE".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_ne!(s.phase, GoalRunPhase::Finished);
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(stored.status, GoalStatus::InProgress);
    }

    /// Loop engineering is opt-in, and the reason it can afford to be is that
    /// switching it off costs nothing: no verifier turn, no evaluator turn,
    /// one LLM call per iteration exactly as before.
    #[tokio::test(start_paused = true)]
    async fn a_plain_run_makes_no_extra_llm_calls() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 3);

        let turns = Arc::new(AtomicU64::new(0));
        let evaluations = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let send = move |_a: AgentId, _p: String| {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                Ok("GOAL_PROGRESS: 10".to_string())
            }
        };
        let e = evaluations.clone();
        let evaluate = move |_g: String, _o: String| {
            let e = e.clone();
            async move {
                e.fetch_add(1, Ordering::SeqCst);
                Ok::<bool, String>(true)
            }
        };

        run_loop(
            goal.id,
            agent_id,
            3,
            substrate.clone(),
            send,
            no_learnings_hook,
            evaluate,
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(turns.load(Ordering::SeqCst), 3, "one turn per iteration");
        assert_eq!(
            evaluations.load(Ordering::SeqCst),
            0,
            "a goal that did not ask for an evaluator must never be billed for one"
        );
    }

    /// The evaluator can conclude the goal is met even when the agent never
    /// says so — that is the point of having a judge that is not the worker.
    #[tokio::test]
    async fn the_evaluator_can_finish_a_goal_the_agent_never_claimed() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id: goal.id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 5,
            last_progress: 0,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 1,
            evaluator_model: Some("haiku".into()),
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        let send = |_a: AgentId, _p: String| async move { Ok("GOAL_PROGRESS: 30".to_string()) };
        let evaluate = |_g: String, _o: String| async move { Ok::<bool, String>(true) };

        run_loop(
            goal.id,
            agent_id,
            5,
            substrate.clone(),
            send,
            no_learnings_hook,
            evaluate,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Finished);
        assert_eq!(s.iteration, 1, "the first evaluated turn ends the run");
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(stored.status, GoalStatus::Completed);
    }

    /// An evaluator outage must not stall a run that the agent itself has
    /// already reported complete.
    #[tokio::test]
    async fn an_evaluator_failure_falls_back_to_the_agents_marker() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(GoalRunState {
            goal_id: goal.id,
            agent_id,
            phase: GoalRunPhase::Running,
            iteration: 0,
            max_iterations: 5,
            last_progress: 0,
            last_error: None,
            verify_agent_id: None,
            verify_max_retries: 1,
            evaluator_model: Some("haiku".into()),
            started_at: Utc::now(),
            updated_at: Utc::now(),
        }));

        let send = |_a: AgentId, _p: String| async move { Ok("all set\nGOAL_DONE".to_string()) };
        let evaluate = |_g: String, _o: String| async move {
            Err::<bool, String>("evaluator model unavailable".to_string())
        };

        run_loop(
            goal.id,
            agent_id,
            5,
            substrate.clone(),
            send,
            no_learnings_hook,
            evaluate,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Finished);
    }

    /// Lessons are only worth capturing if they outlive the run. They must
    /// reach the durable store AND the caller's hook.
    #[tokio::test]
    async fn captured_learnings_are_persisted_and_handed_to_the_caller() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 1);

        let send = |_a: AgentId, _p: String| async move {
            Ok("GOAL_LEARNED: Backoff beats retrying immediately\nGOAL_DONE".to_string())
        };
        let (tx_hook, rx_hook) = std::sync::mpsc::channel::<Vec<String>>();

        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send,
            move |l: Vec<String>| {
                let _ = tx_hook.send(l);
            },
            no_evaluator,
            true,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(
            rx_hook.try_recv().unwrap(),
            vec!["Backoff beats retrying immediately".to_string()],
            "the caller hook must receive the run's lessons"
        );
        let stored = substrate
            .structured_get(
                goals_storage_agent_id(),
                &format!("{LEARNINGS_KEY_PREFIX}{}", goal.id),
            )
            .unwrap()
            .expect("learnings must be persisted under the goal's key");
        assert_eq!(
            stored["learnings"][0].as_str(),
            Some("Backoff beats retrying immediately")
        );
    }

    #[tokio::test]
    async fn a_plain_run_persists_no_learnings() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 1);

        let send = |_a: AgentId, _p: String| async move {
            Ok("GOAL_LEARNED: ignored without loop engineering\nGOAL_DONE".to_string())
        };

        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert!(substrate
            .structured_get(
                goals_storage_agent_id(),
                &format!("{LEARNINGS_KEY_PREFIX}{}", goal.id),
            )
            .unwrap()
            .is_none());
    }

    /// A permanently broken condition — deleted agent, revoked key, network
    /// down — fails identically on every tick. Without a breaker the loop
    /// spends its whole iteration budget rediscovering that.
    #[tokio::test(start_paused = true)]
    async fn repeated_tick_failures_stop_the_run_before_the_iteration_cap() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 100);

        let turns = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let send = move |_a: AgentId, _p: String| {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                Err::<String, String>("agent 'ghost' not found".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            100,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            false,
            state.clone(),
            Arc::new(AtomicBool::new(false)),
            rx,
            None,
        )
        .await;

        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
        assert_eq!(
            turns.load(Ordering::SeqCst) as u32,
            MAX_ERROR_STREAK,
            "the breaker must fire at the streak limit, not at the iteration cap"
        );
    }

    /// The registry must never retain an entry for a loop that has ended.
    ///
    /// `start()` used to spawn the loop and register its handle afterwards. A
    /// loop that finishes inside that window runs its self-cleanup `remove_if`
    /// against a registry that does not hold it yet; the removal finds
    /// nothing, the registration then lands a handle for a run that is already
    /// over, and nothing ever collects it — `state()` reports the run forever
    /// and the map grows by one every time it happens.
    ///
    /// Shutdown is pre-signalled here so the loop breaks on its very first
    /// check, before any store read or agent turn: the shortest path from
    /// `tokio::spawn` to `remove_if`, and so the widest that window ever gets.
    ///
    /// Hitting the race is probabilistic, which is why this runs many rounds
    /// on a multi-threaded runtime. The invariant it asserts is not: with the
    /// handle registered before the spawn there is no ordering in which a
    /// finished loop leaves an entry behind, so this test cannot fail
    /// spuriously — only when the ordering regresses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_run_that_ends_immediately_leaves_no_entry_behind() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());

        for round in 0..100 {
            let (_tx, rx) = watch::channel(true);
            let runner = GoalRunner::new(rx);
            let goal_id = GoalId::new();
            let agent_id = AgentId::new();

            runner.start(
                goal_id,
                agent_id,
                10,
                substrate.clone(),
                |_a: AgentId, _p: String| async move { Ok::<String, String>(String::new()) },
                no_learnings_hook,
                no_evaluator,
                false,
                None,
                None,
                None,
            );

            // Probe the registry directly rather than through `state()`:
            // `state()` answers `None` both for "no entry" and for "the state
            // lock was momentarily held", and the run loop takes that lock on
            // its way out. Conflating the two would let a transient lock read
            // as a clean registry.
            //
            // A stale entry is never collected, so exhausting this budget is a
            // real failure rather than a slow machine.
            for _ in 0..200 {
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
}
