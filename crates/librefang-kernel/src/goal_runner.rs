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

/// Consecutive dispatch failures on one leg of the verifier gate, split by
/// kind the same way the generator's own `rate_limit_streak` / `error_streak`
/// pair is.
///
/// Two things this deliberately does NOT do, each because the obvious
/// shortcut produces a counter that can never trip (#7785 re-review):
///
/// - It is not shared between the verdict leg and the rework leg. A verdict
///   dispatch that succeeds is no evidence the rework dispatch works, so a
///   shared counter would be reset by the other leg's success on every
///   iteration — the same trap that kept the generator's `error_streak` from
///   ever noticing a dead verifier.
/// - It is not the generator's `error_streak`, even for the rework turn,
///   which is a generator dispatch. That counter is reset at the top of every
///   successful iteration (`error_streak = 0`) and is only *checked* inside
///   the arm where the opening turn failed, so a rework that fails every
///   iteration while the opening turn keeps succeeding would increment
///   nothing that is ever read.
#[derive(Default)]
struct GateStreak {
    rate_limits: u32,
    errors: u32,
}

impl GateStreak {
    /// Record one dispatch failure. Returns the phase the run should end in
    /// once this leg has failed too many times in a row, `None` while it is
    /// still under the threshold.
    ///
    /// A throttled dependency is busy, not broken: routing the error through
    /// `classify_tick_error` first is what lets a rate-limited verifier end
    /// the run as `RateLimited` — the phase the dashboard renders as the
    /// retry-later signal — instead of being reported as unreachable and
    /// sending the operator looking for an agent that was never deleted.
    fn record(&mut self, error: &str) -> Option<GoalRunPhase> {
        match classify_tick_error(error) {
            TickOutcome::RateLimited => {
                self.rate_limits = self.rate_limits.saturating_add(1);
                (self.rate_limits >= MAX_RATE_LIMIT_STREAK).then_some(GoalRunPhase::RateLimited)
            }
            TickOutcome::Ok => {
                self.errors = self.errors.saturating_add(1);
                (self.errors >= MAX_ERROR_STREAK).then_some(GoalRunPhase::Stopped)
            }
        }
    }

    /// A dispatch that got a reply — whatever the reply said — means this leg
    /// is up. Only *consecutive* failures indicate a dead dependency; a flaky
    /// one that recovers between rejections must not have its errors
    /// accumulate across iterations toward a trip it never earned.
    fn reset(&mut self) {
        self.rate_limits = 0;
        self.errors = 0;
    }
}

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

    let started =
        kernel.start_goal_run(goal_id, agent_id, None, loop_engineering, None, None, None);
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

/// The cooperative stop signal for one run, plus who raised it.
///
/// Both of the operator's stop paths land here, and the runner has to tell
/// them apart when it decides whether the iteration in flight may still write
/// the goal document (#7785 re-review):
///
/// - `POST /api/goals/{id}/stop` writes nothing. The iteration in flight still
///   holds the freshest accounting anyone has, so it must land — otherwise the
///   goal keeps the previous iteration's progress while the run row reports the
///   higher iteration count those turns were paid for.
/// - A terminal `PUT /api/goals/{id}` wrote the document *before* stopping, and
///   the write it made is exactly the one this iteration would revert: the
///   runner's own `new_status` is `InProgress` for every iteration that did not
///   pass verification, and the `PUT` carries `progress` too (the dashboard's
///   edit form always sends both). Neither field may be written over it.
#[derive(Debug, Default)]
struct StopFlag {
    stopped: AtomicBool,
    wrote_goal: AtomicBool,
}

impl StopFlag {
    /// A flag that is already raised, by a stopper that wrote nothing.
    fn raised() -> Self {
        let flag = Self::default();
        flag.raise(false);
        flag
    }

    /// Raise the flag. `wrote_goal` is set first so that any thread which
    /// observes `stopped` also observes the stopper's ownership of the
    /// document — the reverse order would leave a window where the runner
    /// sees the stop but not who caused it, which is the exact write this
    /// distinction exists to prevent.
    fn raise(&self, wrote_goal: bool) {
        if wrote_goal {
            self.wrote_goal.store(true, Ordering::SeqCst);
        }
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn is_raised(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Whether the caller that stopped this run had already written the goal
    /// document itself, and therefore owns it from that point on.
    fn stopper_wrote_goal(&self) -> bool {
        self.wrote_goal.load(Ordering::SeqCst)
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
    stop: Arc<StopFlag>,
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
        // spans an await point. Recover the exclusion guard rather than panic.
        let _guard = lock_goal_run_start_stop(&self.start_lock);
        self.stop_locked(goal_id, false)
    }

    /// Stop a goal's run on behalf of a caller that has **already written the
    /// goal document** — the terminal-status `PUT` (#7785 re-review).
    ///
    /// Same stop, but the iteration in flight is barred from writing the goal
    /// afterwards; see [`StopFlag`] for why the plain stop is not.
    pub fn stop_after_goal_write(&self, goal_id: GoalId) -> bool {
        let _guard = lock_goal_run_start_stop(&self.start_lock);
        self.stop_locked(goal_id, true)
    }

    /// Stop body assuming the caller already holds `start_lock`. Split out so
    /// `start()` can run it inside its own critical section without re-locking
    /// the non-reentrant `start_lock` (which would deadlock).
    fn stop_locked(&self, goal_id: GoalId, stopper_wrote_goal: bool) -> bool {
        if let Some((_, handle)) = self.runs.remove(&goal_id) {
            handle.stop.raise(stopper_wrote_goal);
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
    ) -> bool
    where
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
        let _guard = lock_goal_run_start_stop(&self.start_lock);

        // Goal deletion commits before it calls `stop()`, which takes this same
        // lock. Consequently either this read observes the deletion and no run
        // is created, or deletion waits for the insertion and then removes it.
        // There is no window where a deleted goal can leave an orphaned loop.
        if load_goal(&substrate, goal_id).is_none() {
            return false;
        }

        // Replace any prior run for this goal. `stop_locked` (not `stop`)
        // because we already hold `start_lock`, which is non-reentrant. The
        // predecessor's stopper wrote no goal document, and this call is about
        // to overwrite the run state anyway.
        self.stop_locked(goal_id, false);
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
        let stop = Arc::new(StopFlag::default());
        let generation = self.next_gen.fetch_add(1, Ordering::SeqCst);

        let runs = self.runs.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let loop_state = state.clone();
        let loop_stop = stop.clone();
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

        self.runs.insert(
            goal_id,
            RunHandle {
                task: Some(task),
                state,
                stop,
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
                            stop: Arc::new(StopFlag::raised()),
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
    stop: Arc<StopFlag>,
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
    let (verify_agent_id, verify_max_retries, has_evaluator, started_at) = {
        let s = state.lock().await;
        (
            s.verify_agent_id,
            s.verify_max_retries.max(1),
            s.evaluator_model.is_some(),
            s.started_at,
        )
    };

    let mut iteration: u32 = 0;
    let mut rate_limit_streak: u32 = 0;
    let mut error_streak: u32 = 0;
    // #7785 review: the generator's streak resets on every successful tick,
    // so a dead verifier (deleted agent, revoked key) never trips it — the
    // generator keeps succeeding while the gate stays open. Each leg of the
    // gate carries its own streak so a permanently unreachable one ends the
    // run with the cause on last_error, instead of paying for every
    // remaining iteration plus a dead dispatch per rework round.
    let mut verdict_streak = GateStreak::default();
    let mut rework_streak = GateStreak::default();
    // Set once a gate leg has failed often enough to end the run, and read
    // after this iteration's bookkeeping has landed.
    let mut gate_break: Option<GoalRunPhase> = None;
    // Lessons captured via `GOAL_LEARNED:` across the whole run. Replayed into
    // each iteration's prompt and handed to `on_learnings_captured` at the end.
    let mut learnings: Vec<String> = Vec::new();
    // True when the loop ends because the kernel is shutting down (vs. an
    // operator stop, completion, or cap). On shutdown the durable row is left
    // in its last persisted `Running` shape so the next boot's stale-recovery
    // sweep can demote it — mirroring how workflow runs survive a restart.
    let mut interrupted_by_shutdown = false;
    let final_phase = loop {
        if stop.is_raised() {
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
        // #7785 review: `goal.progress` AND `goal.status` both have a second
        // writer — the `goal_update` tool, which the agent's own system
        // prompt tells it to call, and which patches the same shared
        // document this read comes from (progress: goal_control.rs, status:
        // same handler, `"completed"` is a valid enum value). Neither write
        // goes through `parse_tick`, so the verifier gate below never sees
        // them, and a rejected iteration could still close the run one tick
        // later through this check alone. Both are only a completion signal
        // when there is no verifier configured to bypass — a gated run needs
        // the runner's own `done` branch to have actually run, which is what
        // an unconditioned `Cancelled` still allows: a cancellation is a
        // legitimate stop order regardless of what the run's own gate thinks.
        if goal.status == GoalStatus::Cancelled
            || (verify_agent_id.is_none()
                && (goal.status == GoalStatus::Completed || goal.progress >= 100))
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
                // #7785 re-review: this iteration's lessons are held aside and
                // REPLACED by each rework round, exactly like `parsed` itself
                // (`:1095`), because the rework prompt promises the agent that
                // "the reworked reply replaces the rejected one". Appending
                // straight into `learnings` here instead stored the rejected
                // draft's lesson AND the corrected one — up to
                // `verify_max_retries` copies of a single lesson, which then
                // crowd genuinely distinct earlier ones out of the 6-entry
                // `LEARNINGS_IN_PROMPT` replay window and pad the numbered
                // list a human reads in `librefang skill pending show`.
                let mut iteration_learnings = std::mem::take(&mut parsed.learnings);

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
                            Ok(v) => {
                                verdict_streak.reset();
                                v
                            }
                            Err(e) => {
                                // An unreachable verifier is an open gate, and
                                // an open gate is the failure mode this whole
                                // mechanism exists to prevent. Treat it as a
                                // rejection rather than waving the work through
                                // — and do NOT `continue` back into the loop:
                                // a dead verifier does not heal between
                                // back-to-back dispatches of the same prompt,
                                // so the retry loop's own rule (a retry that
                                // does not regenerate is not a retry) says
                                // this arm is waste, not recovery. One
                                // dispatch failure rejects the iteration, and
                                // `GateStreak` decides whether a permanently
                                // failing verifier ends the run (#7785 review).
                                warn!(goal_id = %goal_id, verifier = %verifier, error = %e,
                                      "Goal run: verifier call failed; treating as a rejection");
                                if let Some(phase) = verdict_streak.record(&e) {
                                    warn!(goal_id = %goal_id, verifier = %verifier, %phase,
                                          "Goal run: verifier failing across iterations; ending the run");
                                    gate_break = Some(phase);
                                }
                                verified = false;
                                rejection = Some(format!("verifier unreachable: {e}"));
                                break;
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
                                rework_streak.reset();
                                output = reworked;
                                parsed = parse_tick(&output);
                                iteration_learnings = std::mem::take(&mut parsed.learnings);
                            }
                            Err(e) => {
                                // #7785 re-review: the third leg of the loop
                                // needs its own breaker for the same reason
                                // the other two have one. The rework prompt
                                // is dispatched into a session one turn longer
                                // than the opening one that just succeeded, so
                                // a context-length or request-size rejection
                                // surfaces here first and then repeats every
                                // iteration while the shorter opening prompt
                                // keeps succeeding — burning the whole budget
                                // and ending in `MaxIterationsReached` with no
                                // cause attributed.
                                warn!(goal_id = %goal_id, error = %e,
                                      "Goal run: rework turn failed; keeping the rejected output");
                                if let Some(phase) = rework_streak.record(&e) {
                                    warn!(goal_id = %goal_id, %phase,
                                          "Goal run: rework turn failing across iterations; ending the run");
                                    gate_break = Some(phase);
                                }
                                verified = false;
                                rejection = Some(format!("rework turn failed: {e}"));
                                break;
                            }
                        }
                    }
                }

                // Fold this iteration's surviving lessons into the run's list,
                // skipping any text already captured. Deliberately NOT gated
                // on `verified`, for the same reason `GOAL_BLOCKED` below is
                // not: `GOAL_LEARNED` closes nothing and asserts nothing about
                // the work the verifier grades, so it cannot cross the
                // boundary the gate on `done` exists to protect — and a lesson
                // drawn from an attempt that did not land is exactly the kind
                // a human reviewing the draft skill wants to see.
                // Linear scan: the list is bounded by iterations × retries.
                if loop_engineering {
                    for lesson in iteration_learnings {
                        if !learnings.iter().any(|k| k == &lesson) {
                            learnings.push(lesson);
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
                // #7785 review: only a REJECTED iteration's progress needs
                // clamping — `parsed.progress` there is an assertion from
                // work the verifier just refused. Clamping unconditionally
                // also caught the plain no-verifier path, wrongly pinning a
                // legitimately-reported 100 at 99 and burning the rest of
                // the iteration budget instead of letting the top-of-loop
                // `progress >= 100` check end the run right away. For a
                // VERIFIED run the unclamped write is merely harmless, not
                // an early exit: that check no longer fires while a
                // verifier is configured, so a verified-but-undeclared-done
                // iteration's 100 just sits on the document until an
                // explicit `GOAL_DONE` closes it.
                let new_progress = if done {
                    Some(100)
                } else if verified {
                    parsed.progress
                } else {
                    parsed.progress.map(|p| p.min(99))
                };
                // #7785 re-review: an operator's `PUT /api/goals/{id}` that
                // sets a terminal status raises this run's stop flag
                // (`routes/goals.rs`, the interlock `delete_goal` already
                // had). Writing the run's own status on top of that would
                // revert the operator's choice — this iteration's
                // `new_status` is `InProgress` whenever the work was not
                // passed, which is exactly the case an operator marking the
                // goal `completed` is overriding. Once THAT stop lands the
                // goal document belongs to whoever wrote it.
                //
                // A plain `POST /stop` is not that: it writes nothing, so
                // there is no operator choice on the document to protect and
                // skipping the write would just throw away the accounting
                // this iteration already paid for — the goal would keep the
                // previous iteration's progress while the run row reported
                // the higher iteration count. Hence the flag carries who
                // raised it (`StopFlag`), not merely that it is up.
                //
                // The run row below is persisted in every case for the same
                // reason: those turns happened, and hiding them would leave
                // the run API reporting a stale iteration count.
                if !stop.stopper_wrote_goal() {
                    patch_goal(&substrate, goal_id, new_progress, new_status);
                }

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

                // A permanently failing gate leg ends the run here, AFTER the
                // same bookkeeping every other outcome gets above (iteration
                // count, progress, the persisted row) — breaking before it
                // left the run API reporting a stale iteration count for
                // turns the daemon had already paid for (#7785 review).
                if let Some(phase) = gate_break {
                    break phase;
                }
                if done {
                    break GoalRunPhase::Finished;
                }
                // #7785 review: deliberately NOT gated by `verified`, unlike
                // `done`. `GOAL_BLOCKED` is a claim about the AGENT's own
                // situation (missing credential, unreachable dependency),
                // not a claim about the task the verifier judges — asking
                // the same verifier to approve "I am stuck" conflates two
                // different questions. Gating it also has no completion risk
                // to justify the cost: a false claim only spends the run's
                // own budget one iteration early and ends in `Stopped`, never
                // `Completed`, so it cannot cross the boundary the gate on
                // `done` exists to protect. A genuinely blocked agent in a
                // verified run needs to be able to say so.
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
        // #7785 review: key the entry to the RUN, not the goal. A per-goal key
        // let a second run of the same goal overwrite the first run's lessons
        // (structured_set replaces the document), losing work the operator may
        // never have reviewed. The run's started_at timestamp is the identity
        // the run state already carries; millisecond precision cannot collide
        // for one goal because `start()` replaces any predecessor.
        let run_ident = started_at.timestamp_millis();
        let key = format!("{LEARNINGS_KEY_PREFIX}{goal_id}_{run_ident}");
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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

    /// The store key a run's learnings land under — derived from the run
    /// state's `started_at` exactly as `run_loop` derives it.
    fn learnings_key_for(state: &Arc<Mutex<GoalRunState>>) -> String {
        let s = state.try_lock().unwrap();
        format!(
            "{LEARNINGS_KEY_PREFIX}{}_{}",
            s.goal_id,
            s.started_at.timestamp_millis()
        )
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
            Arc::new(StopFlag::default()),
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

    /// #7785 re-review: `GOAL_BLOCKED` is a claim about the agent's own
    /// situation, not about the task the verifier judges, so it is
    /// deliberately not gated by `verified` the way `GOAL_DONE` is — a
    /// genuinely blocked agent in a verified run must still be able to stop
    /// the run rather than burn its whole iteration budget repeating a
    /// rejected claim the verifier has no way to confirm either way.
    #[tokio::test(start_paused = true)]
    async fn a_rejected_iterations_blocked_marker_still_stops_the_run() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 5, 1);

        let send = move |target: AgentId, _p: String| async move {
            if target == verifier {
                Ok("VERDICT: FAIL\nREASON: not really stuck".to_string())
            } else {
                Ok("stuck\nGOAL_BLOCKED: need a key".to_string())
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
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            state.lock().await.phase,
            GoalRunPhase::Stopped,
            "a blocked claim must stop the run on the first iteration, verified or not"
        );
        // Blocked must NOT mark the goal completed, verified or not.
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
            Arc::new(StopFlag::raised()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            let runner = GoalRunner::new_with_store(rx, store.clone());
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
                    no_learnings_hook,
                    no_evaluator,
                    false,
                    None,
                    None,
                    None,
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
        let runner = Arc::new(GoalRunner::new_with_store(rx, store.clone()));
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
            no_learnings_hook,
            no_evaluator,
            false,
            None,
            None,
            None,
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
        let runner = GoalRunner::new_with_store(rx, store.clone());

        let started =
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

        assert!(!started);
        assert!(runner.state(goal_id).is_none());
        assert!(store.get_run(&goal_id.to_string()).unwrap().is_none());
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
            Arc::new(StopFlag::default()),
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
    async fn a_rejected_iterations_progress_cannot_cross_the_completion_boundary() {
        // #7785 review: the verifier gate only blocks GOAL_DONE. An agent
        // whose rejected reply carries `GOAL_PROGRESS: 100` used to persist
        // 100 anyway, and the pre-existing `progress >= 100` top-of-loop
        // check finished the run one iteration later — leaving the
        // incoherent `progress: 100` + `status: in_progress`. The rejected
        // progress must be clamped below the boundary.
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 2, 2);

        let send = move |target: AgentId, _p: String| async move {
            if target == verifier {
                Ok("VERDICT: FAIL\nREASON: the claimed progress is not real".to_string())
            } else {
                Ok("still working on it\nGOAL_PROGRESS: 100".to_string())
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
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_ne!(
            stored.progress, 100,
            "a rejected iteration's progress must not cross the completion boundary"
        );
        assert_eq!(
            stored.status,
            GoalStatus::InProgress,
            "the goal cannot be finished by the work its own verifier rejected"
        );
        let s = state.lock().await;
        assert_ne!(
            s.phase,
            GoalRunPhase::Finished,
            "the run must not end through the progress check on rejected work"
        );
    }

    /// #7785 review: `parsed.progress` is only one of two writers of
    /// `goal.progress`. The `goal_update` tool (the agent's own system
    /// prompt tells it to call this) patches the same shared document
    /// directly, bypassing `parse_tick` entirely — so clamping the text
    /// marker alone does not close the back door. Simulated here by calling
    /// `patch_goal` from the tick closure: not the tool's own handler (that
    /// is `apply_goal_update` in `goal_control.rs`, a different function),
    /// but the same document, the same key, and the same JSON shape, both
    /// under `structured_modify`.
    #[tokio::test(start_paused = true)]
    async fn a_rejected_iterations_tool_written_progress_cannot_cross_the_completion_boundary() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        let goal_id = goal.id;
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal_id, agent_id, verifier, 3, 1);

        let turns = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let sub = substrate.clone();
        let send = move |target: AgentId, _p: String| {
            let t = t.clone();
            let sub = sub.clone();
            async move {
                if target == verifier {
                    Ok("VERDICT: FAIL\nREASON: not actually done".to_string())
                } else {
                    t.fetch_add(1, Ordering::SeqCst);
                    // No `GOAL_PROGRESS:` marker in the text — the agent
                    // called the tool instead, which writes straight to the
                    // store the runner reads at the top of the loop.
                    patch_goal(&sub, goal_id, Some(100), None);
                    Ok("still working on it".to_string())
                }
            }
        };

        run_loop(
            goal_id,
            agent_id,
            3,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            turns.load(Ordering::SeqCst),
            3,
            "a tool-written progress of 100 must not let the top-of-loop check \
             short-circuit the run after the first rejected iteration"
        );
        let s = state.lock().await;
        assert_eq!(
            s.phase,
            GoalRunPhase::MaxIterationsReached,
            "the run must spend its full budget, not finish through progress \
             the verifier never saw"
        );
    }

    /// #7785 re-review: `goal_update`'s `status` field is the same shape of
    /// bypass as its `progress` field — `"completed"` is a valid enum value
    /// (`definitions.rs`), written to the same document, never seen by
    /// `parse_tick` or the verifier. The top-of-loop check must ignore it
    /// exactly the same way it now ignores tool-written progress.
    ///
    /// Unlike progress, this bypass only lives in the tick's `Err` arm: a
    /// successful tick always writes `new_status = Some(InProgress)` at the
    /// end of the same iteration (`done` is false on a rejection), which
    /// overwrites a tool-written `Completed` before the header ever reads it
    /// back. Only a turn that calls the tool and THEN fails leaves the
    /// write standing — the `Err` arm never touches the goal document at
    /// all — so the first tick here fails after writing, to exercise the
    /// window that actually survives to the next header check.
    #[tokio::test(start_paused = true)]
    async fn a_rejected_iterations_tool_written_completed_status_cannot_finish_the_run() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        let goal_id = goal.id;
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal_id, agent_id, verifier, 3, 1);

        let turns = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let sub = substrate.clone();
        let send = move |target: AgentId, _p: String| {
            let t = t.clone();
            let sub = sub.clone();
            async move {
                if target == verifier {
                    Ok("VERDICT: FAIL\nREASON: not actually done".to_string())
                } else {
                    let n = t.fetch_add(1, Ordering::SeqCst) + 1;
                    // No `GOAL_DONE` marker — the agent called the tool with
                    // status="completed" instead.
                    patch_goal(&sub, goal_id, None, Some(GoalStatus::Completed));
                    if n == 1 {
                        // The `Err` arm never touches the goal document, so
                        // this is the only shape of turn that leaves the
                        // tool's `Completed` write standing for the next
                        // header check to see.
                        return Err("provider hiccup".to_string());
                    }
                    Ok("still working on it".to_string())
                }
            }
        };

        run_loop(
            goal_id,
            agent_id,
            3,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            turns.load(Ordering::SeqCst),
            3,
            "a tool-written Completed status must not let the top-of-loop check \
             short-circuit the run after the first rejected iteration"
        );
        assert_eq!(
            state.lock().await.phase,
            GoalRunPhase::MaxIterationsReached,
            "the run must spend its full budget, not finish through a status \
             the verifier never saw"
        );
    }

    /// A cancellation is a legitimate stop order regardless of whether the
    /// run has a verifier — unlike `Completed` / bare progress, it is not
    /// something the runner's own gate produces, so it must not be gated.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_goal_still_stops_a_verified_run() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let mut goal = test_goal(agent_id);
        goal.status = GoalStatus::Cancelled;
        let goal_id = goal.id;
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal_id, agent_id, verifier, 3, 1);

        let turns = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let send = move |_a: AgentId, _p: String| {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                Ok("should never run".to_string())
            }
        };

        run_loop(
            goal_id,
            agent_id,
            3,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            turns.load(Ordering::SeqCst),
            0,
            "a cancelled goal must never tick"
        );
        assert_eq!(state.lock().await.phase, GoalRunPhase::Finished);
    }

    /// #7785 review: the clamp on rejected progress must not reach the plain
    /// no-verifier path. There is no gate to bypass there, so a `100` an
    /// agent reports without an explicit `GOAL_DONE` (the two markers are
    /// independent) must still let the top-of-loop `progress >= 100` check
    /// end the run — clamping it unconditionally pinned it at 99 and burned
    /// the rest of the iteration budget instead.
    #[tokio::test(start_paused = true)]
    async fn progress_100_without_goal_done_still_finishes_with_no_verifier_configured() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 5);

        let turns = Arc::new(AtomicU64::new(0));
        let t = turns.clone();
        let send = move |_a: AgentId, _p: String| {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                Ok("GOAL_PROGRESS: 100".to_string())
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
            false,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            turns.load(Ordering::SeqCst),
            1,
            "an unverified 100 must end the run on the next check, not burn the whole budget"
        );
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(stored.progress, 100);
        assert_eq!(state.lock().await.phase, GoalRunPhase::Finished);
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            Arc::new(StopFlag::default()),
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
            .structured_get(goals_storage_agent_id(), &learnings_key_for(&state))
            .unwrap()
            .expect("learnings must be persisted under the run's key");
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
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert!(substrate
            .structured_get(goals_storage_agent_id(), &learnings_key_for(&state),)
            .unwrap()
            .is_none());
    }

    /// #7785 review: the learnings key is per-RUN, not per-goal. A second run
    /// of the same goal must not overwrite the first run's lessons — the
    /// operator may never have read them.
    #[tokio::test]
    async fn a_second_run_of_the_same_goal_does_not_delete_the_first_runs_learnings() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);

        let state = mk_state(goal.id, agent_id, 1);
        state.try_lock().unwrap().started_at = Utc::now() - chrono::Duration::hours(1);
        let send = |_a: AgentId, _p: String| async move {
            // No GOAL_DONE marker: completing the goal would end the second
            // run before its first tick (the loop breaks on Completed status).
            Ok("GOAL_LEARNED: first run lesson".to_string())
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
            Arc::new(StopFlag::default()),
            rx.clone(),
            None,
        )
        .await;
        let first_key = learnings_key_for(&state);

        // Simulate the operator re-running the same goal: a fresh run state
        // (new started_at) against the same goal id.
        let state2 = mk_state(goal.id, agent_id, 1);
        state2.try_lock().unwrap().started_at = Utc::now() + chrono::Duration::hours(1);
        let send2 = |_a: AgentId, _p: String| async move {
            Ok("GOAL_LEARNED: second run lesson\nGOAL_DONE".to_string())
        };
        run_loop(
            goal.id,
            agent_id,
            1,
            substrate.clone(),
            send2,
            no_learnings_hook,
            no_evaluator,
            true,
            state2.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;
        let second_key = learnings_key_for(&state2);

        assert_ne!(
            first_key, second_key,
            "distinct runs must use distinct keys"
        );
        let first = substrate
            .structured_get(goals_storage_agent_id(), &first_key)
            .unwrap()
            .expect("the first run's learnings must survive the second run");
        assert_eq!(first["learnings"][0].as_str(), Some("first run lesson"));
        let second = substrate
            .structured_get(goals_storage_agent_id(), &second_key)
            .unwrap()
            .expect("the second run's learnings must be stored under its own key");
        assert_eq!(second["learnings"][0].as_str(), Some("second run lesson"));
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
            Arc::new(StopFlag::default()),
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

    /// #7785 review: an unreachable verifier fed no circuit breaker, so a
    /// permanently dead one burned the whole iteration budget — every
    /// generator turn succeeding, every verifier dispatch failing, and
    /// nothing ever tripping `error_streak` because that counter belongs to
    /// the generator leg. The verifier needs its own streak so a dead one
    /// ends the run in `Stopped` at the streak limit, not at the cap.
    #[tokio::test(start_paused = true)]
    async fn repeated_verifier_failures_stop_the_run_before_the_iteration_cap() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 100, 1);

        let verifier_calls = Arc::new(AtomicU64::new(0));
        let vc = verifier_calls.clone();
        let send = move |target: AgentId, _p: String| {
            let vc = vc.clone();
            async move {
                if target == verifier {
                    vc.fetch_add(1, Ordering::SeqCst);
                    Err::<String, String>("verifier agent not found".to_string())
                } else {
                    Ok("still working".to_string())
                }
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
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(s.phase, GoalRunPhase::Stopped);
        assert_eq!(
            s.last_error.as_deref(),
            Some("Iteration 5 did not pass verification: verifier unreachable: verifier agent not found")
        );
        assert_eq!(
            verifier_calls.load(Ordering::SeqCst) as u32,
            MAX_ERROR_STREAK,
            "the breaker must fire at the streak limit, not at the iteration cap"
        );
        // #7785 review: the dead-verifier break used to happen before the
        // iteration count and progress got their usual post-tick update, so
        // the run API reported one fewer turn than the daemon actually paid
        // for. The break now happens after that bookkeeping, same as every
        // other terminal outcome.
        assert_eq!(
            s.iteration, MAX_ERROR_STREAK,
            "the recorded iteration count must match the turns actually paid for"
        );
    }

    /// #7785 re-review: gating `Completed` on `verify_agent_id.is_none()`
    /// closed the `goal_update` tool's bypass but also took away the only
    /// path by which an OPERATOR's completion ended a gated run — and the
    /// iteration already in flight then wrote the operator's `completed`
    /// back to `in_progress`, leaving the incoherent `progress: 100` +
    /// `status: in_progress` pair and buying the rest of the budget.
    ///
    /// The fix is not to guess which writer produced the value: it is to
    /// give the operator the run's real control channel (`stop_goal_run`,
    /// now called by `update_goal_by_id` the way `delete_goal` always has)
    /// and to stop the run overwriting a document it no longer owns. This
    /// simulates the operator's `PUT` landing mid-iteration, which is the
    /// window the clobber lived in. The sibling below covers the other stop
    /// path — the one that wrote nothing and therefore bars nothing.
    #[tokio::test(start_paused = true)]
    async fn an_operator_stop_landing_mid_iteration_keeps_the_status_it_wrote() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 25, 1);
        let stop = Arc::new(StopFlag::default());

        let sub = substrate.clone();
        let stop_flag = stop.clone();
        let goal_id = goal.id;
        let send = move |target: AgentId, _p: String| {
            let sub = sub.clone();
            let stop_flag = stop_flag.clone();
            async move {
                if target == verifier {
                    // The work is not passed, so the runner's own
                    // `new_status` for this iteration is `InProgress` — the
                    // value that used to clobber the operator's choice.
                    return Ok::<String, String>("VERDICT: FAIL\nREASON: not yet".to_string());
                }
                // The operator's `PUT /api/goals/{id}` landing while the
                // generator turn is in flight: the document is written and
                // the run's stop flag goes up, both before this iteration's
                // bookkeeping runs.
                patch_goal(&sub, goal_id, Some(100), Some(GoalStatus::Completed));
                stop_flag.raise(true);
                Ok("still working".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            25,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            stop,
            rx,
            None,
        )
        .await;

        let stored = load_goal(&substrate, goal.id).expect("goal must still exist");
        assert_eq!(
            stored.status,
            GoalStatus::Completed,
            "the run must not write its own status over the one an operator chose"
        );
        assert_eq!(
            stored.progress, 100,
            "nor its own progress, which would leave the 100/in_progress pair incoherent"
        );
        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
        assert_eq!(
            state.lock().await.iteration,
            1,
            "the turn was paid for, so the run row still records it"
        );
    }

    /// #7785 review (M2): the interlock above must NOT fire for the plain
    /// `POST /api/goals/{id}/stop`, which raises the same flag but writes
    /// nothing to the goal document.
    ///
    /// There is no operator write to protect on that path, so skipping the
    /// end-of-iteration write only throws away the accounting the run already
    /// paid for: the goal would keep iteration N-1's progress while the run
    /// row — persisted either way, deliberately — reports iteration N. The
    /// sibling test above always pairs the flag with a document write, so it
    /// passes whether the runner keys on "stopped" or on "stopped by someone
    /// who wrote the goal"; this one only passes for the latter.
    #[tokio::test(start_paused = true)]
    async fn a_bare_operator_stop_still_lands_the_progress_of_the_iteration_it_interrupted() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_state(goal.id, agent_id, 25);
        let stop = Arc::new(StopFlag::default());

        let stop_flag = stop.clone();
        let send = move |_target: AgentId, _p: String| {
            let stop_flag = stop_flag.clone();
            async move {
                // The operator hitting Stop while this turn is in flight: the
                // flag goes up, and the goal document is left exactly as the
                // previous iteration left it.
                stop_flag.raise(false);
                Ok::<String, String>("progress so far\nGOAL_PROGRESS: 60".to_string())
            }
        };

        run_loop(
            goal.id,
            agent_id,
            25,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            false,
            state.clone(),
            stop,
            rx,
            None,
        )
        .await;

        let stored = load_goal(&substrate, goal.id).expect("goal must still exist");
        assert_eq!(
            stored.progress, 60,
            "a stop that wrote nothing must not cost the operator the progress \
             of the turn it interrupted"
        );
        assert_eq!(
            stored.status,
            GoalStatus::InProgress,
            "and the run's own status write is not overriding anyone here"
        );
        assert_eq!(state.lock().await.phase, GoalRunPhase::Stopped);
        assert_eq!(
            state.lock().await.iteration,
            1,
            "the run row records the iteration the goal document now agrees with"
        );
    }

    /// #7785 re-review: a verifier dispatch failure never reached
    /// `classify_tick_error`, so a THROTTLED verifier was counted as a DEAD
    /// one. The operator was told the verifier did not exist when it was
    /// only busy, and `RateLimited` — the phase the dashboard renders as the
    /// retry-later signal — never fired for the verifier leg.
    #[tokio::test(start_paused = true)]
    async fn a_rate_limited_verifier_ends_the_run_as_rate_limited_not_unreachable() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 100, 1);

        let verifier_calls = Arc::new(AtomicU64::new(0));
        let vc = verifier_calls.clone();
        let send = move |target: AgentId, _p: String| {
            let vc = vc.clone();
            async move {
                if target == verifier {
                    vc.fetch_add(1, Ordering::SeqCst);
                    Err::<String, String>(format!(
                        "provider throttled {} 30000",
                        librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER
                    ))
                } else {
                    Ok("still working".to_string())
                }
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
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(
            s.phase,
            GoalRunPhase::RateLimited,
            "a throttled verifier is busy, not missing"
        );
        assert_eq!(
            verifier_calls.load(Ordering::SeqCst) as u32,
            MAX_RATE_LIMIT_STREAK,
            "and it must trip the rate-limit breaker's shorter streak, not the error one"
        );
    }

    /// #7785 re-review: the rework turn's failure fed no streak counter, so
    /// it was the one leg of the loop that could still burn the whole
    /// iteration budget on a permanently failing dispatch — the
    /// "healthy-looking exhausted budget" the two sibling breakers exist to
    /// remove.
    ///
    /// It cannot be counted toward the generator's `error_streak`, which is
    /// reset at the top of every successful iteration and only *checked* in
    /// the arm where the opening turn failed: the opening turn keeps
    /// succeeding here, so that counter would be zeroed every iteration and
    /// never read. Hence the rework leg's own `GateStreak`.
    #[tokio::test(start_paused = true)]
    async fn a_rework_turn_that_always_fails_stops_the_run_before_the_iteration_cap() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        // Two rework rounds allowed, so a FAIL verdict dispatches a rework
        // turn rather than rejecting the iteration outright.
        let state = mk_verified_state(goal.id, agent_id, verifier, 100, 2);

        let rework_calls = Arc::new(AtomicU64::new(0));
        let rc = rework_calls.clone();
        let send = move |target: AgentId, prompt: String| {
            let rc = rc.clone();
            async move {
                if target == verifier {
                    return Ok::<String, String>("VERDICT: FAIL\nREASON: not yet".to_string());
                }
                if prompt.starts_with("[GOAL REWORK]") {
                    // The longer prompt is the one the provider rejects —
                    // a context-length or request-size limit the shorter
                    // opening prompt stays under.
                    rc.fetch_add(1, Ordering::SeqCst);
                    return Err("request too large for the model's context".to_string());
                }
                Ok("still working".to_string())
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
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_eq!(
            s.phase,
            GoalRunPhase::Stopped,
            "a permanently failing rework dispatch must end the run, not exhaust the budget"
        );
        assert_eq!(
            rework_calls.load(Ordering::SeqCst) as u32,
            MAX_ERROR_STREAK,
            "the breaker must fire at the streak limit, not at the iteration cap"
        );
        assert_eq!(
            s.last_error.as_deref(),
            Some(
                "Iteration 5 did not pass verification: rework turn failed: \
                 request too large for the model's context"
            ),
            "and the cause must be attributed rather than left blank"
        );
    }

    /// #7785 re-review: learnings were appended before the gate and again per
    /// rework round with no dedup at any layer, so one lesson re-stated in
    /// the corrected reply was stored once per round. Those copies filled the
    /// 6-entry `LEARNINGS_IN_PROMPT` replay window and padded the numbered
    /// list a human reads in `librefang skill pending show`, pushing
    /// genuinely distinct earlier lessons out.
    ///
    /// `parsed` is already REPLACED by each rework round because the rework
    /// prompt promises "the reworked reply replaces the rejected one";
    /// learnings now follow the same rule, and the run-level list refuses a
    /// text it already holds.
    #[tokio::test(start_paused = true)]
    async fn a_lesson_restated_in_the_reworked_reply_is_stored_once() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        let state = mk_verified_state(goal.id, agent_id, verifier, 1, 2);

        let send = move |target: AgentId, prompt: String| async move {
            if target == verifier {
                return Ok::<String, String>("VERDICT: FAIL\nREASON: not yet".to_string());
            }
            if prompt.starts_with("[GOAL REWORK]") {
                // The corrected reply restates the same lesson and adds one.
                return Ok("GOAL_LEARNED: Backoff beats retrying immediately\n\
                           GOAL_LEARNED: Read the quota header first"
                    .to_string());
            }
            Ok("GOAL_LEARNED: Backoff beats retrying immediately".to_string())
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
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            rx_hook.try_recv().unwrap(),
            vec![
                "Backoff beats retrying immediately".to_string(),
                "Read the quota header first".to_string(),
            ],
            "the restated lesson must be kept once, and the new one must survive with it"
        );
    }

    /// #7785 review (m3): the dedup has two layers, and the test above only
    /// exercises one of them.
    ///
    /// Within an iteration, `iteration_learnings` is REPLACED by each rework
    /// round, so a lesson restated in a corrected reply is stored once even
    /// with the run-level `any()` guard deleted — that test passes against
    /// either version. Across iterations there is no replacement to lean on:
    /// each round's surviving lessons are folded into a list that outlives it,
    /// and only the run-level guard stops an agent that keeps restating the
    /// same lesson from filling the 6-entry `LEARNINGS_IN_PROMPT` replay
    /// window with copies of it. This is the case that fails without the
    /// guard, with a duplicate at index 1.
    #[tokio::test(start_paused = true)]
    async fn a_lesson_repeated_in_a_later_iteration_is_stored_once() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        // No verifier: the gate is not what is under test here, and without it
        // each iteration is exactly one generator turn.
        let state = mk_state(goal.id, agent_id, 2);

        let turn = Arc::new(AtomicU64::new(0));
        let t = turn.clone();
        let send = move |_target: AgentId, _p: String| {
            let t = t.clone();
            async move {
                if t.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok::<String, String>(
                        "GOAL_LEARNED: Backoff beats retrying immediately".to_string(),
                    );
                }
                // A second iteration that re-states the first one's lesson and
                // adds one of its own.
                Ok("GOAL_LEARNED: Backoff beats retrying immediately\n\
                    GOAL_LEARNED: Read the quota header first"
                    .to_string())
            }
        };
        let (tx_hook, rx_hook) = std::sync::mpsc::channel::<Vec<String>>();

        run_loop(
            goal.id,
            agent_id,
            2,
            substrate.clone(),
            send,
            move |l: Vec<String>| {
                let _ = tx_hook.send(l);
            },
            no_evaluator,
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        assert_eq!(
            turn.load(Ordering::SeqCst),
            2,
            "the fixture only means anything if both iterations actually ran"
        );
        assert_eq!(
            rx_hook.try_recv().unwrap(),
            vec![
                "Backoff beats retrying immediately".to_string(),
                "Read the quota header first".to_string(),
            ],
            "a lesson already captured in an earlier iteration must not be stored again"
        );
    }

    /// The streak counts *consecutive* verifier failures. A flaky verifier
    /// that recovers between rejections must not have its failures
    /// accumulate across the whole run toward a trip it never earned — the
    /// same way the generator's own `error_streak` resets on every
    /// successful tick rather than only when it stops failing for good.
    #[tokio::test(start_paused = true)]
    async fn a_flaky_verifier_that_recovers_never_trips_the_breaker() {
        let substrate = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let agent_id = AgentId::new();
        let verifier = AgentId::new();
        let goal = test_goal(agent_id);
        seed_goal(&substrate, &goal);
        let (_tx, rx) = watch::channel(false);
        // verify_max_retries: 1 — a single FAIL verdict rejects the
        // iteration outright, so each iteration costs exactly one verifier
        // dispatch, matching the failing calls below one-for-one.
        let state = mk_verified_state(goal.id, agent_id, verifier, 20, 1);

        let call = Arc::new(AtomicU64::new(0));
        let c = call.clone();
        let send = move |target: AgentId, _p: String| {
            let c = c.clone();
            async move {
                if target == verifier {
                    // Four consecutive failures (below the streak limit of
                    // 5), then one successful-but-rejecting dispatch that
                    // must reset the streak, then four more failures that
                    // would trip the breaker on their own if the earlier
                    // streak had leaked through.
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    match n {
                        1..=4 | 6..=9 => {
                            Err::<String, String>("verifier agent not found".to_string())
                        }
                        5 => Ok("VERDICT: FAIL\nREASON: not yet".to_string()),
                        _ => Ok("VERDICT: PASS".to_string()),
                    }
                } else {
                    Ok("still working\nGOAL_DONE".to_string())
                }
            }
        };

        run_loop(
            goal.id,
            agent_id,
            20,
            substrate.clone(),
            send,
            no_learnings_hook,
            no_evaluator,
            true,
            state.clone(),
            Arc::new(StopFlag::default()),
            rx,
            None,
        )
        .await;

        let s = state.lock().await;
        assert_ne!(
            s.phase,
            GoalRunPhase::Stopped,
            "non-consecutive failures below the streak limit must not stop the run"
        );
        let stored = load_goal(&substrate, goal.id).unwrap();
        assert_eq!(
            stored.status,
            GoalStatus::Completed,
            "the run must reach the verifier's eventual pass, not die on the flakiness"
        );
    }
}
