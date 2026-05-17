//! Background agent executor — runs agents autonomously on schedules, timers, and conditions.
//!
//! Supports three autonomous modes:
//! - **Continuous**: Agent self-prompts on a fixed interval.
//! - **Periodic**: Agent wakes on a simplified cron schedule (e.g. "every 5m").
//! - **Proactive**: Agent wakes when matching events fire (via the trigger engine).

use crate::triggers::TriggerPattern;
use dashmap::DashMap;
use librefang_types::agent::{AgentId, ScheduleMode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Outer loop handle + any inner watcher handles spawned by that loop.
struct AgentTaskEntry {
    outer: JoinHandle<()>,
    /// Inner watcher tasks spawned by this agent's loop. These hold LLM permits
    /// and must be aborted when the agent stops so permits are released promptly.
    watchers: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
}

/// Maximum number of concurrent background LLM calls across all agents.
const MAX_CONCURRENT_BG_LLM: usize = 5;

/// Maximum number of *consecutive* ticks whose agent turn failed because the
/// LLM provider was rate-limited / quota-exhausted before the continuous /
/// periodic background loop stops re-firing the agent (issue #5168).
///
/// Without this cap, a hand agent that hits a long-lived provider limit
/// (e.g. an Ollama Cloud *weekly* quota) re-runs the agent loop on every
/// `check_interval_secs` tick forever: the in-loop `call_with_retry` does its
/// bounded 3 retries, fails with the deferred rate-limit error, the error is
/// logged and dropped, the `busy` flag clears, and the next tick fires the
/// exact same doomed call again. That burns the entire quota and the loop is
/// restarted on every daemon boot (`start_background_agents`), so the zombie
/// survives restarts. A single non-rate-limited successful (or
/// non-rate-limited *failed*) tick resets the counter, so transient blips do
/// not permanently park a healthy agent.
const MAX_CONSECUTIVE_RATE_LIMITS: u32 = 5;

/// Outcome of a single background tick, reported back from the inner watcher
/// task to the scheduling loop so the loop can stop hammering a rate-limited
/// provider (issue #5168).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// The agent turn completed, or failed for a reason other than a provider
    /// rate-limit / quota exhaustion. Resets the consecutive rate-limit count.
    Ok,
    /// The agent turn failed because the LLM provider rate-limited / exhausted
    /// quota (the runtime surfaced the `RATE_LIMIT_DEFER_MARKER`). Counts
    /// toward [`MAX_CONSECUTIVE_RATE_LIMITS`].
    RateLimited,
}

/// Classify a kernel `send_message_*` error string into a [`TickOutcome`].
///
/// The runtime appends [`librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER`]
/// (`[rate_limit_defer_ms]=<ms>`) to the error message *only* after the
/// in-loop retry budget for a rate-limit / overload error is exhausted, so
/// its presence is a precise, already-tested signal that this turn failed on
/// a provider limit rather than a one-off transient or a logic error.
pub fn classify_tick_error(err_msg: &str) -> TickOutcome {
    if err_msg.contains(librefang_channels::message_journal::RATE_LIMIT_DEFER_MARKER) {
        TickOutcome::RateLimited
    } else {
        TickOutcome::Ok
    }
}

/// RAII guard that clears the busy flag on drop, even if the task panics.
struct BusyGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// Manages background task loops for autonomous agents.
pub struct BackgroundExecutor {
    /// Running background task handles (outer loop + inner watcher list), keyed by agent ID.
    tasks: DashMap<AgentId, AgentTaskEntry>,
    /// Shutdown signal receiver (from Supervisor).
    shutdown_rx: watch::Receiver<bool>,
    /// SECURITY: Global semaphore to limit concurrent background LLM calls.
    llm_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-agent pause flags: when true, background ticks are skipped.
    pause_flags: DashMap<AgentId, Arc<AtomicBool>>,
}

impl BackgroundExecutor {
    /// Create a new executor bound to the supervisor's shutdown signal.
    ///
    /// `max_concurrent` overrides the default [`MAX_CONCURRENT_BG_LLM`] when
    /// provided (i.e. when it is > 0). Pass `0` to use the compiled default.
    pub fn new(shutdown_rx: watch::Receiver<bool>) -> Self {
        Self::with_concurrency(shutdown_rx, MAX_CONCURRENT_BG_LLM)
    }

    /// Create a new executor with a custom concurrency limit for background LLM calls.
    pub fn with_concurrency(shutdown_rx: watch::Receiver<bool>, max_concurrent: usize) -> Self {
        let effective = if max_concurrent == 0 {
            MAX_CONCURRENT_BG_LLM
        } else {
            max_concurrent
        };
        Self {
            tasks: DashMap::new(),
            shutdown_rx,
            llm_semaphore: Arc::new(tokio::sync::Semaphore::new(effective)),
            pause_flags: DashMap::new(),
        }
    }

    /// Pause an agent's background loop (ticks will be skipped until resumed).
    ///
    /// Safe to call before `start_agent` — pre-creates the pause flag so that
    /// when the loop does start it begins in the paused state.
    pub fn pause_agent(&self, agent_id: AgentId) {
        self.pause_flags
            .entry(agent_id)
            .or_insert_with(|| Arc::new(AtomicBool::new(true)))
            .store(true, Ordering::SeqCst);
        info!(id = %agent_id, "Background loop paused");
    }

    /// Resume a paused agent's background loop.
    pub fn resume_agent(&self, agent_id: AgentId) {
        if let Some(flag) = self.pause_flags.get(&agent_id) {
            flag.store(false, Ordering::SeqCst);
            info!(id = %agent_id, "Background loop resumed");
        }
    }

    /// Start a background loop for an agent based on its schedule mode.
    ///
    /// For `Continuous` and `Periodic` modes, spawns a tokio task that
    /// periodically sends a self-prompt message to the agent.
    /// For `Proactive` mode, registers triggers — no dedicated task needed.
    ///
    /// `send_message` is a closure that sends a message to the given agent
    /// and returns a join handle resolving to a [`TickOutcome`]. It captures
    /// an `Arc<LibreFangKernel>` from the caller. The outcome lets the
    /// scheduling loop stop re-firing an agent that is stuck on a provider
    /// rate-limit (issue #5168) instead of burning quota forever.
    pub fn start_agent<F>(
        &self,
        agent_id: AgentId,
        agent_name: &str,
        schedule: &ScheduleMode,
        send_message: F,
    ) where
        F: Fn(AgentId, String) -> tokio::task::JoinHandle<TickOutcome> + Send + Sync + 'static,
    {
        match schedule {
            ScheduleMode::Reactive => {} // nothing to do
            ScheduleMode::Continuous {
                check_interval_secs,
            } => {
                let interval = std::time::Duration::from_secs(*check_interval_secs);
                let name = agent_name.to_string();
                let mut shutdown = self.shutdown_rx.clone();
                let busy = Arc::new(AtomicBool::new(false));
                let semaphore = self.llm_semaphore.clone();
                // Reuse a pre-existing pause flag (set by pause_agent before loop start)
                // so hands paused before their loop begins stay paused.
                let paused = self
                    .pause_flags
                    .entry(agent_id)
                    .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                    .clone();

                info!(
                    agent = %name, id = %agent_id,
                    interval_secs = check_interval_secs,
                    "Starting continuous background loop"
                );

                let check_interval = *check_interval_secs;
                // Consecutive rate-limit tick counter (issue #5168). Updated by
                // the inner watcher from the tick outcome; read by the loop to
                // decide when to stop hammering a rate-limited provider.
                let rate_limit_streak = Arc::new(std::sync::atomic::AtomicU32::new(0));
                // Shared list of inner watcher handles so stop_agent can abort them.
                let watcher_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> =
                    Arc::new(std::sync::Mutex::new(Vec::new()));
                let watcher_handles_loop = watcher_handles.clone();

                let handle = tokio::spawn(async move {
                    // Stagger first tick: random jitter (0..interval) so agents
                    // don't all load sessions into memory simultaneously at boot.
                    let jitter_secs = rand::random::<u64>() % check_interval.max(1);
                    let jitter = std::time::Duration::from_secs(jitter_secs);
                    debug!(agent = %name, jitter_secs, "Continuous loop: initial jitter");
                    tokio::time::sleep(jitter).await;

                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(interval) => {}
                            _ = shutdown.changed() => {
                                info!(agent = %name, "Continuous loop: shutdown signal received");
                                break;
                            }
                        }

                        // Rate-limit circuit breaker (issue #5168): once the
                        // agent has failed N consecutive ticks on a provider
                        // rate-limit / quota exhaustion, stop re-firing it.
                        // Continuing would burn the (possibly weekly) quota on
                        // every tick forever with no chance of success until
                        // the window resets. Terminating the loop leaves the
                        // agent idle (the same terminal state as a normal
                        // stop); an operator restart / resume gets a fresh
                        // bounded budget rather than resuming an infinite loop.
                        let streak = rate_limit_streak.load(Ordering::SeqCst);
                        if streak >= MAX_CONSECUTIVE_RATE_LIMITS {
                            warn!(
                                agent = %name,
                                id = %agent_id,
                                consecutive_rate_limits = streak,
                                max = MAX_CONSECUTIVE_RATE_LIMITS,
                                "Continuous loop: provider rate-limited for {streak} consecutive \
                                 ticks — stopping the autonomous loop to stop burning quota. \
                                 Resolve the provider quota and resume / restart the agent.",
                            );
                            break;
                        }

                        // Skip tick if agent is paused (hand pause)
                        if paused.load(Ordering::SeqCst) {
                            debug!(agent = %name, "Continuous loop: skipping tick (paused)");
                            continue;
                        }

                        // Skip if previous tick is still running
                        if busy
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_err()
                        {
                            debug!(agent = %name, "Continuous loop: skipping tick (busy)");
                            continue;
                        }

                        // SECURITY: Acquire global LLM concurrency permit
                        let permit = match semaphore.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => {
                                busy.store(false, Ordering::SeqCst);
                                break; // Semaphore closed
                            }
                        };

                        let prompt = format!(
                            "[AUTONOMOUS TICK] You are running in continuous mode. \
                             Check your goals, review shared memory for pending tasks, \
                             and take any necessary actions. Agent: {name}"
                        );
                        debug!(agent = %name, "Continuous loop: sending self-prompt");
                        let busy_clone = busy.clone();
                        let watcher_name = name.clone();
                        let streak_clone = rate_limit_streak.clone();
                        let jh = (send_message)(agent_id, prompt);
                        // Spawn a watcher with RAII guard — busy flag clears even on panic.
                        // Track the handle so stop_agent can abort it and release the permit.
                        let watcher_jh = tokio::spawn(async move {
                            let _guard = BusyGuard { flag: busy_clone };
                            let _permit = permit; // drop permit when watcher exits
                            match jh.await {
                                Ok(TickOutcome::RateLimited) => {
                                    let n = streak_clone
                                        .fetch_add(1, Ordering::SeqCst)
                                        .saturating_add(1);
                                    warn!(
                                        agent = %watcher_name,
                                        id = %agent_id,
                                        consecutive_rate_limits = n,
                                        "Continuous loop: tick failed on provider rate-limit",
                                    );
                                }
                                Ok(TickOutcome::Ok) => {
                                    // A non-rate-limited tick (success or any
                                    // other failure) clears the streak so a
                                    // transient blip cannot permanently park
                                    // an otherwise-healthy agent.
                                    streak_clone.store(0, Ordering::SeqCst);
                                }
                                Err(e) => {
                                    warn!(
                                        agent = %watcher_name,
                                        id = %agent_id,
                                        error = %e,
                                        "Continuous loop: agent tick task panicked or was aborted",
                                    );
                                }
                            }
                        });
                        if let Ok(mut guards) = watcher_handles_loop.lock() {
                            guards.retain(|h| !h.is_finished());
                            guards.push(watcher_jh);
                        }
                    }
                });

                self.tasks.insert(
                    agent_id,
                    AgentTaskEntry {
                        outer: handle,
                        watchers: watcher_handles,
                    },
                );
            }
            ScheduleMode::Periodic { cron } => {
                let interval_secs = parse_cron_to_secs(cron);
                let interval = std::time::Duration::from_secs(interval_secs);
                let name = agent_name.to_string();
                let cron_owned = cron.clone();
                let mut shutdown = self.shutdown_rx.clone();
                let busy = Arc::new(AtomicBool::new(false));
                let semaphore = self.llm_semaphore.clone();
                // Reuse a pre-existing pause flag (set by pause_agent before loop start)
                // so hands paused before their loop begins stay paused.
                let paused = self
                    .pause_flags
                    .entry(agent_id)
                    .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                    .clone();

                info!(
                    agent = %name, id = %agent_id,
                    cron = %cron, interval_secs = interval_secs,
                    "Starting periodic background loop"
                );

                // Consecutive rate-limit tick counter (issue #5168) — same
                // circuit-breaker rationale as the continuous loop above.
                let rate_limit_streak = Arc::new(std::sync::atomic::AtomicU32::new(0));
                // Shared list of inner watcher handles so stop_agent can abort them.
                let watcher_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> =
                    Arc::new(std::sync::Mutex::new(Vec::new()));
                let watcher_handles_loop = watcher_handles.clone();

                let handle = tokio::spawn(async move {
                    // Stagger first tick: random jitter so agents don't spike memory together.
                    let jitter_secs = rand::random::<u64>() % interval_secs.max(1);
                    let jitter = std::time::Duration::from_secs(jitter_secs);
                    debug!(agent = %name, jitter_secs, "Periodic loop: initial jitter");
                    tokio::time::sleep(jitter).await;

                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(interval) => {}
                            _ = shutdown.changed() => {
                                info!(agent = %name, "Periodic loop: shutdown signal received");
                                break;
                            }
                        }

                        // Rate-limit circuit breaker (issue #5168): stop
                        // re-firing once the provider has rate-limited N
                        // consecutive ticks. See the continuous loop for the
                        // full rationale.
                        let streak = rate_limit_streak.load(Ordering::SeqCst);
                        if streak >= MAX_CONSECUTIVE_RATE_LIMITS {
                            warn!(
                                agent = %name,
                                id = %agent_id,
                                consecutive_rate_limits = streak,
                                max = MAX_CONSECUTIVE_RATE_LIMITS,
                                "Periodic loop: provider rate-limited for {streak} consecutive \
                                 ticks — stopping the scheduled loop to stop burning quota. \
                                 Resolve the provider quota and resume / restart the agent.",
                            );
                            break;
                        }

                        // Skip tick if agent is paused (hand pause)
                        if paused.load(Ordering::SeqCst) {
                            debug!(agent = %name, "Periodic loop: skipping tick (paused)");
                            continue;
                        }

                        if busy
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_err()
                        {
                            debug!(agent = %name, "Periodic loop: skipping tick (busy)");
                            continue;
                        }

                        // SECURITY: Acquire global LLM concurrency permit
                        let permit = match semaphore.clone().acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => {
                                busy.store(false, Ordering::SeqCst);
                                break; // Semaphore closed
                            }
                        };

                        let prompt = format!(
                            "[SCHEDULED TICK] You are running on a periodic schedule ({cron_owned}). \
                             Perform your routine duties. Agent: {name}"
                        );
                        debug!(agent = %name, "Periodic loop: sending scheduled prompt");
                        let busy_clone = busy.clone();
                        let watcher_name = name.clone();
                        let streak_clone = rate_limit_streak.clone();
                        let jh = (send_message)(agent_id, prompt);
                        // Spawn a watcher with RAII guard — busy flag clears even on panic.
                        // Track the handle so stop_agent can abort it and release the permit.
                        let watcher_jh = tokio::spawn(async move {
                            let _guard = BusyGuard { flag: busy_clone };
                            let _permit = permit; // drop permit when watcher exits
                            match jh.await {
                                Ok(TickOutcome::RateLimited) => {
                                    let n = streak_clone
                                        .fetch_add(1, Ordering::SeqCst)
                                        .saturating_add(1);
                                    warn!(
                                        agent = %watcher_name,
                                        id = %agent_id,
                                        consecutive_rate_limits = n,
                                        "Periodic loop: tick failed on provider rate-limit",
                                    );
                                }
                                Ok(TickOutcome::Ok) => {
                                    streak_clone.store(0, Ordering::SeqCst);
                                }
                                Err(e) => {
                                    warn!(
                                        agent = %watcher_name,
                                        id = %agent_id,
                                        error = %e,
                                        "Periodic loop: agent tick task panicked or was aborted",
                                    );
                                }
                            }
                        });
                        if let Ok(mut guards) = watcher_handles_loop.lock() {
                            guards.retain(|h| !h.is_finished());
                            guards.push(watcher_jh);
                        }
                    }
                });

                self.tasks.insert(
                    agent_id,
                    AgentTaskEntry {
                        outer: handle,
                        watchers: watcher_handles,
                    },
                );
            }
            ScheduleMode::Proactive { .. } => {
                // Proactive agents rely on triggers, not a dedicated loop.
                // Triggers are registered by the kernel during spawn_agent / start_background_agents.
                debug!(agent = %agent_name, "Proactive agent — triggers handle activation");
            }
        }
    }

    /// Stop the background loop for an agent, if one is running.
    ///
    /// Aborts both the outer scheduling loop and any in-flight inner watcher
    /// tasks so that LLM semaphore permits are released immediately.
    pub fn stop_agent(&self, agent_id: AgentId) {
        self.pause_flags.remove(&agent_id);
        if let Some((_, entry)) = self.tasks.remove(&agent_id) {
            entry.outer.abort();
            // Abort all tracked inner watcher tasks so they release LLM permits.
            if let Ok(mut guards) = entry.watchers.lock() {
                for watcher in guards.drain(..) {
                    watcher.abort();
                }
            }
            info!(id = %agent_id, "Background loop stopped");
        }
    }

    /// Number of actively running background loops.
    pub fn active_count(&self) -> usize {
        self.tasks.len()
    }
}

/// Parse a proactive condition string into a `TriggerPattern`.
///
/// Supported formats:
/// - `"event:agent_spawned"` → `TriggerPattern::AgentSpawned { name_pattern: "*" }`
/// - `"event:agent_terminated"` → `TriggerPattern::AgentTerminated`
/// - `"event:lifecycle"` → `TriggerPattern::Lifecycle`
/// - `"event:system"` → `TriggerPattern::System`
/// - `"memory:some_key"` → `TriggerPattern::MemoryKeyPattern { key_pattern: "some_key" }`
/// - `"all"` → `TriggerPattern::All`
pub fn parse_condition(condition: &str) -> Option<TriggerPattern> {
    let condition = condition.trim();

    if condition.eq_ignore_ascii_case("all") {
        return Some(TriggerPattern::All);
    }

    if let Some(event_kind) = condition.strip_prefix("event:") {
        let kind = event_kind.trim().to_lowercase();
        return match kind.as_str() {
            "agent_spawned" => Some(TriggerPattern::AgentSpawned {
                name_pattern: "*".to_string(),
            }),
            "agent_terminated" => Some(TriggerPattern::AgentTerminated),
            "lifecycle" => Some(TriggerPattern::Lifecycle),
            "system" => Some(TriggerPattern::System),
            "memory_update" => Some(TriggerPattern::MemoryUpdate),
            other => {
                warn!(condition = %condition, "Unknown event condition: {other}");
                None
            }
        };
    }

    if let Some(key) = condition.strip_prefix("memory:") {
        return Some(TriggerPattern::MemoryKeyPattern {
            key_pattern: key.trim().to_string(),
        });
    }

    warn!(condition = %condition, "Unrecognized proactive condition format");
    None
}

/// Parse a cron or interval expression into a polling interval in seconds.
///
/// Supported formats:
/// - `"every 30s"` → 30
/// - `"every 5m"` → 300
/// - `"every 1h"` → 3600
/// - `"every 2d"` → 172800
/// - Standard 5-field cron (e.g. `"*/15 * * * *"`) → interval derived from
///   the minute field: `*/N` → N*60, otherwise 60s for per-minute schedules.
///   Hour/day constraints are handled by the cron scheduler, not this function.
///
/// Falls back to 300 seconds (5 minutes) for unparseable expressions.
pub fn parse_cron_to_secs(cron: &str) -> u64 {
    let cron_lower = cron.trim().to_lowercase();

    // Try "every <N><unit>" format
    if let Some(rest) = cron_lower.strip_prefix("every ") {
        let rest = rest.trim();
        if let Some(num_str) = rest.strip_suffix('s') {
            if let Ok(n) = num_str.trim().parse::<u64>() {
                return n;
            }
        }
        if let Some(num_str) = rest.strip_suffix('m') {
            if let Ok(n) = num_str.trim().parse::<u64>() {
                return n * 60;
            }
        }
        if let Some(num_str) = rest.strip_suffix('h') {
            if let Ok(n) = num_str.trim().parse::<u64>() {
                return n * 3600;
            }
        }
        if let Some(num_str) = rest.strip_suffix('d') {
            if let Ok(n) = num_str.trim().parse::<u64>() {
                return n * 86400;
            }
        }
    }

    // Try standard 5-field cron: min hour dom month dow
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() == 5 {
        let min_field = parts[0];
        let hour_field = parts[1];

        // */N minute interval (e.g. */15 → 900s)
        if let Some(n_str) = min_field.strip_prefix("*/") {
            if let Ok(n) = n_str.parse::<u64>() {
                return n * 60;
            }
        }
        // Specific minute(s) with hour constraint → run once per hour window
        if min_field != "*" && hour_field != "*" {
            return 3600;
        }
        // Every minute
        if min_field == "*" {
            return 60;
        }
        // Specific minute, every hour
        return 3600;
    }

    warn!(cron = %cron, "Unparseable cron expression, defaulting to 300s");
    300
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cron_seconds() {
        assert_eq!(parse_cron_to_secs("every 30s"), 30);
        assert_eq!(parse_cron_to_secs("every 1s"), 1);
    }

    #[test]
    fn test_parse_cron_minutes() {
        assert_eq!(parse_cron_to_secs("every 5m"), 300);
        assert_eq!(parse_cron_to_secs("every 1m"), 60);
    }

    #[test]
    fn test_parse_cron_hours() {
        assert_eq!(parse_cron_to_secs("every 1h"), 3600);
        assert_eq!(parse_cron_to_secs("every 2h"), 7200);
    }

    #[test]
    fn test_parse_cron_days() {
        assert_eq!(parse_cron_to_secs("every 1d"), 86400);
    }

    #[test]
    fn test_parse_standard_cron() {
        assert_eq!(parse_cron_to_secs("*/5 * * * *"), 300);
        assert_eq!(parse_cron_to_secs("*/15 * * * *"), 900);
        assert_eq!(parse_cron_to_secs("*/15 21-23,0-5 * * 1-5"), 900);
        assert_eq!(parse_cron_to_secs("0 9 * * *"), 3600);
        assert_eq!(parse_cron_to_secs("* * * * *"), 60);
        assert_eq!(parse_cron_to_secs("30 * * * *"), 3600);
    }

    #[test]
    fn test_parse_cron_fallback() {
        assert_eq!(parse_cron_to_secs("gibberish"), 300);
    }

    #[test]
    fn test_parse_condition_events() {
        assert!(matches!(
            parse_condition("event:agent_spawned"),
            Some(TriggerPattern::AgentSpawned { .. })
        ));
        assert!(matches!(
            parse_condition("event:agent_terminated"),
            Some(TriggerPattern::AgentTerminated)
        ));
        assert!(matches!(
            parse_condition("event:lifecycle"),
            Some(TriggerPattern::Lifecycle)
        ));
        assert!(matches!(
            parse_condition("event:system"),
            Some(TriggerPattern::System)
        ));
        assert!(matches!(
            parse_condition("event:memory_update"),
            Some(TriggerPattern::MemoryUpdate)
        ));
    }

    #[test]
    fn test_parse_condition_memory() {
        match parse_condition("memory:agent.*.status") {
            Some(TriggerPattern::MemoryKeyPattern { key_pattern }) => {
                assert_eq!(key_pattern, "agent.*.status");
            }
            other => panic!(
                "Expected MemoryKeyPattern from parse_condition, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_parse_condition_all() {
        assert!(matches!(parse_condition("all"), Some(TriggerPattern::All)));
    }

    #[test]
    fn test_parse_condition_unknown() {
        assert!(parse_condition("event:unknown_thing").is_none());
        assert!(parse_condition("badprefix:foo").is_none());
    }

    #[tokio::test]
    async fn test_continuous_shutdown() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let executor = BackgroundExecutor::new(shutdown_rx);
        let agent_id = AgentId::new();

        let tick_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tick_clone = tick_count.clone();

        let schedule = ScheduleMode::Continuous {
            check_interval_secs: 1, // 1 second for fast test
        };

        executor.start_agent(agent_id, "test-agent", &schedule, move |_id, _msg| {
            let tc = tick_clone.clone();
            tokio::spawn(async move {
                tc.fetch_add(1, Ordering::SeqCst);
                TickOutcome::Ok
            })
        });

        assert_eq!(executor.active_count(), 1);

        // Wait for at least 1 tick
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(tick_count.load(Ordering::SeqCst) >= 1);

        // Shutdown
        let _ = shutdown_tx.send(true);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The loop should have exited (handle finished)
        // Active count still shows the entry until stop_agent is called
        executor.stop_agent(agent_id);
        assert_eq!(executor.active_count(), 0);
    }

    #[tokio::test]
    async fn test_skip_if_busy() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let executor = BackgroundExecutor::new(shutdown_rx);
        let agent_id = AgentId::new();

        let tick_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tick_clone = tick_count.clone();

        let schedule = ScheduleMode::Continuous {
            check_interval_secs: 1,
        };

        // Each tick takes 3 seconds — should cause subsequent ticks to be skipped
        executor.start_agent(agent_id, "slow-agent", &schedule, move |_id, _msg| {
            let tc = tick_clone.clone();
            tokio::spawn(async move {
                tc.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                TickOutcome::Ok
            })
        });

        // Wait 2.5 seconds: 1 tick should fire at t=1s, second at t=2s should be skipped (busy)
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        let ticks = tick_count.load(Ordering::SeqCst);
        // Should be exactly 1 because the first tick is still "busy" when the second arrives
        assert_eq!(ticks, 1, "Expected 1 tick (skip-if-busy), got {ticks}");

        executor.stop_agent(agent_id);
    }

    #[test]
    fn test_executor_active_count() {
        let (_tx, rx) = watch::channel(false);
        let executor = BackgroundExecutor::new(rx);
        assert_eq!(executor.active_count(), 0);

        // Reactive mode → no background task
        let id = AgentId::new();
        executor.start_agent(id, "reactive", &ScheduleMode::Reactive, |_id, _msg| {
            tokio::spawn(async { TickOutcome::Ok })
        });
        assert_eq!(executor.active_count(), 0);

        // Proactive mode → no dedicated task
        let id2 = AgentId::new();
        executor.start_agent(
            id2,
            "proactive",
            &ScheduleMode::Proactive {
                conditions: vec!["event:agent_spawned".to_string()],
            },
            |_id, _msg| tokio::spawn(async { TickOutcome::Ok }),
        );
        assert_eq!(executor.active_count(), 0);
    }

    #[test]
    fn test_classify_tick_error_detects_rate_limit_defer_marker() {
        // The runtime appends RATE_LIMIT_DEFER_MARKER only after the in-loop
        // retry budget is exhausted on a rate-limit / overload error. Its
        // presence (even wrapped in the LibreFangError Display prefix) must
        // classify as RateLimited; everything else is Ok.
        let exhausted = "LLM driver error: Rate limited after 3 retries \
                         [rate_limit_defer_ms]=300000";
        assert_eq!(classify_tick_error(exhausted), TickOutcome::RateLimited);

        let overloaded = "LLM driver error: Model overloaded after 3 retries \
                          [rate_limit_defer_ms]=60000";
        assert_eq!(classify_tick_error(overloaded), TickOutcome::RateLimited);

        // A plain rate-limit string WITHOUT the marker (e.g. an in-flight
        // retry log, or a one-off transient) must NOT trip the breaker —
        // only the exhaustion marker counts.
        assert_eq!(
            classify_tick_error("Rate limited, retrying after delay"),
            TickOutcome::Ok
        );
        assert_eq!(
            classify_tick_error("Tool execution failed: file_read — not found"),
            TickOutcome::Ok
        );
        assert_eq!(classify_tick_error(""), TickOutcome::Ok);
    }

    /// Regression for issue #5168: a hand agent whose every tick fails on a
    /// provider rate-limit (e.g. an Ollama Cloud *weekly* quota) must NOT
    /// re-fire forever. The continuous loop must stop after a bounded number
    /// of consecutive rate-limited ticks instead of burning quota until the
    /// (possibly week-long) window resets.
    #[tokio::test]
    async fn test_continuous_loop_stops_after_max_consecutive_rate_limits() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let executor = BackgroundExecutor::new(shutdown_rx);
        let agent_id = AgentId::new();

        let tick_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tick_clone = tick_count.clone();

        let schedule = ScheduleMode::Continuous {
            // Sub-second interval: without the fix this would fire dozens of
            // times in the test window; with the fix it caps out quickly.
            check_interval_secs: 1,
        };

        // Every tick reports RateLimited, exactly as the real closure does
        // once `send_message_with_sender_context` returns the deferred
        // rate-limit error.
        executor.start_agent(agent_id, "rl-hand", &schedule, move |_id, _msg| {
            let tc = tick_clone.clone();
            tokio::spawn(async move {
                tc.fetch_add(1, Ordering::SeqCst);
                TickOutcome::RateLimited
            })
        });

        // Give the loop generous wall-clock time to either self-terminate
        // (fixed) or keep hammering (the pre-fix infinite loop). With a 1s
        // interval, an unbounded loop would tick ~9 times in 10s; the bound
        // is MAX_CONSECUTIVE_RATE_LIMITS, so a few extra ticks for the
        // jitter + in-flight watcher are expected but it must plateau.
        tokio::time::sleep(std::time::Duration::from_millis(10_000)).await;
        let ticks_a = tick_count.load(Ordering::SeqCst);

        // It must NOT be unbounded. The breaker trips at
        // MAX_CONSECUTIVE_RATE_LIMITS; allow a small slop for the initial
        // jitter tick and the one in-flight tick whose outcome lands after
        // the loop already read the (sub-threshold) streak.
        let ceiling = u64::from(MAX_CONSECUTIVE_RATE_LIMITS) + 2;
        assert!(
            ticks_a >= 1 && ticks_a <= ceiling,
            "rate-limited continuous loop must terminate bounded: got {ticks_a} \
             ticks, expected 1..={ceiling}"
        );

        // The loop has terminated: no further ticks fire even after waiting
        // several more intervals. This is the "does not re-enter forever"
        // assertion — the pre-fix loop would keep climbing here.
        tokio::time::sleep(std::time::Duration::from_millis(4_000)).await;
        let ticks_b = tick_count.load(Ordering::SeqCst);
        assert_eq!(
            ticks_a, ticks_b,
            "loop must stay terminated (no new ticks after the breaker tripped): \
             before={ticks_a} after={ticks_b}"
        );

        executor.stop_agent(agent_id);
    }

    /// A single non-rate-limited tick must reset the consecutive-rate-limit
    /// streak so a transient blip cannot permanently park a healthy agent —
    /// and so the breaker only fires on a *sustained* limit, not a flap.
    #[tokio::test]
    async fn test_intermittent_rate_limit_does_not_trip_breaker() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let executor = BackgroundExecutor::new(shutdown_rx);
        let agent_id = AgentId::new();

        let tick_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tick_clone = tick_count.clone();

        let schedule = ScheduleMode::Continuous {
            check_interval_secs: 1,
        };

        // Alternate RateLimited / Ok forever: the streak never reaches
        // MAX_CONSECUTIVE_RATE_LIMITS, so the loop must keep running.
        executor.start_agent(agent_id, "flaky-hand", &schedule, move |_id, _msg| {
            let tc = tick_clone.clone();
            tokio::spawn(async move {
                let n = tc.fetch_add(1, Ordering::SeqCst);
                if n.is_multiple_of(2) {
                    TickOutcome::RateLimited
                } else {
                    TickOutcome::Ok
                }
            })
        });

        tokio::time::sleep(std::time::Duration::from_millis(8_000)).await;
        let ticks = tick_count.load(Ordering::SeqCst);
        // With a 1s interval over ~8s the loop should still be alive and have
        // ticked several times — well past MAX_CONSECUTIVE_RATE_LIMITS — proving
        // the Ok ticks reset the streak and the breaker did NOT fire.
        assert!(
            ticks > u64::from(MAX_CONSECUTIVE_RATE_LIMITS),
            "intermittent rate-limit must not trip the breaker: only {ticks} ticks \
             (expected > {})",
            MAX_CONSECUTIVE_RATE_LIMITS
        );

        executor.stop_agent(agent_id);
    }
}
