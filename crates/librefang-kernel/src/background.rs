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

/// Maximum number of concurrent background LLM calls across all agents.
const MAX_CONCURRENT_BG_LLM: usize = 5;

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
    /// Running background task handles, keyed by agent ID.
    tasks: DashMap<AgentId, JoinHandle<()>>,
    /// Shutdown signal receiver (from Supervisor).
    shutdown_rx: watch::Receiver<bool>,
    /// SECURITY: Global semaphore to limit concurrent background LLM calls.
    llm_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-agent pause flags: when true, background ticks are skipped.
    pause_flags: DashMap<AgentId, Arc<AtomicBool>>,
}

impl BackgroundExecutor {
    /// Create a new executor bound to the supervisor's shutdown signal.
    pub fn new(shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            tasks: DashMap::new(),
            shutdown_rx,
            llm_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_BG_LLM)),
            pause_flags: DashMap::new(),
        }
    }

    /// Pause an agent's background loop (ticks will be skipped until resumed).
    pub fn pause_agent(&self, agent_id: AgentId) {
        if let Some(flag) = self.pause_flags.get(&agent_id) {
            flag.store(true, Ordering::SeqCst);
            info!(id = %agent_id, "Background loop paused");
        }
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
    /// and returns a result. It captures an `Arc<LibreFangKernel>` from the caller.
    pub fn start_agent<F>(
        &self,
        agent_id: AgentId,
        agent_name: &str,
        schedule: &ScheduleMode,
        send_message: F,
    ) where
        F: Fn(AgentId, String) -> tokio::task::JoinHandle<()> + Send + Sync + 'static,
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
                let paused = Arc::new(AtomicBool::new(false));
                self.pause_flags.insert(agent_id, paused.clone());

                info!(
                    agent = %name, id = %agent_id,
                    interval_secs = check_interval_secs,
                    "Starting continuous background loop"
                );

                let handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(interval) => {}
                            _ = shutdown.changed() => {
                                info!(agent = %name, "Continuous loop: shutdown signal received");
                                break;
                            }
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
                        let jh = (send_message)(agent_id, prompt);
                        // Spawn a watcher with RAII guard — busy flag clears even on panic
                        tokio::spawn(async move {
                            let _guard = BusyGuard { flag: busy_clone };
                            let _permit = permit; // drop permit when watcher exits
                            let _ = jh.await;
                        });
                    }
                });

                self.tasks.insert(agent_id, handle);
            }
            ScheduleMode::Periodic { cron } => {
                let interval_secs = parse_cron_to_secs(cron);
                let interval = std::time::Duration::from_secs(interval_secs);
                let name = agent_name.to_string();
                let cron_owned = cron.clone();
                let mut shutdown = self.shutdown_rx.clone();
                let busy = Arc::new(AtomicBool::new(false));
                let semaphore = self.llm_semaphore.clone();
                let paused = Arc::new(AtomicBool::new(false));
                self.pause_flags.insert(agent_id, paused.clone());

                info!(
                    agent = %name, id = %agent_id,
                    cron = %cron, interval_secs = interval_secs,
                    "Starting periodic background loop"
                );

                let handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(interval) => {}
                            _ = shutdown.changed() => {
                                info!(agent = %name, "Periodic loop: shutdown signal received");
                                break;
                            }
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
                        let jh = (send_message)(agent_id, prompt);
                        // Spawn a watcher with RAII guard — busy flag clears even on panic
                        tokio::spawn(async move {
                            let _guard = BusyGuard { flag: busy_clone };
                            let _permit = permit; // drop permit when watcher exits
                            let _ = jh.await;
                        });
                    }
                });

                self.tasks.insert(agent_id, handle);
            }
            ScheduleMode::Proactive { .. } => {
                // Proactive agents rely on triggers, not a dedicated loop.
                // Triggers are registered by the kernel during spawn_agent / start_background_agents.
                debug!(agent = %agent_name, "Proactive agent — triggers handle activation");
            }
        }
    }

    /// Stop the background loop for an agent, if one is running.
    pub fn stop_agent(&self, agent_id: AgentId) {
        self.pause_flags.remove(&agent_id);
        if let Some((_, handle)) = self.tasks.remove(&agent_id) {
            handle.abort();
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
            tokio::spawn(async {})
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
            |_id, _msg| tokio::spawn(async {}),
        );
        assert_eq!(executor.active_count(), 0);
    }
}
