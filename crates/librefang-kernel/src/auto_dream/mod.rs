//! Auto-dream: periodic per-agent background memory consolidation.
//!
//! Port of libre-code's `autoDream` — a time-gated background task that asks
//! each opt-in agent to reflect on its own memory and consolidate recent
//! signal via a 4-phase prompt (Orient / Gather / Consolidate / Prune).
//!
//! # Gates (cheapest first, per agent)
//!
//! 1. **Global enabled** — `config.auto_dream.enabled` must be true.
//! 2. **Per-agent opt-in** — the agent's manifest must have
//!    `auto_dream_enabled = true`.
//! 3. **Time** — at least `min_hours` must have elapsed since the last
//!    recorded consolidation for *that agent* (mtime of its lock file).
//! 4. **Session count** — at least `min_sessions` of that agent's sessions
//!    must have been touched since the last consolidation. Prevents
//!    consolidating an idle agent. Set `min_sessions = 0` to disable.
//! 5. **Lock** — per-agent filesystem lock with PID-staleness detection
//!    prevents two daemons on the same data directory from double-firing.
//!
//! A failed or timed-out dream rolls back the lock mtime so the time gate
//! reopens on the next tick (via `utimes` on Unix; best-effort elsewhere).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use librefang_channels::types::SenderContext;
use librefang_types::agent::AgentId;

use crate::kernel::LibreFangKernel;

pub mod lock;
pub mod prompt;

pub use lock::ConsolidationLock;

/// Channel name used for auto-dream invocations, so session scoping and
/// auditing distinguish dream turns from cron/user/channel turns.
pub const AUTO_DREAM_CHANNEL: &str = "auto_dream";

/// Default subdirectory under `data_dir` holding per-agent lock files.
const DEFAULT_LOCK_DIR: &str = "auto_dream";

/// Resolve the directory holding per-agent lock files. Honours
/// `config.auto_dream.lock_dir` when set, otherwise defaults to
/// `<data_dir>/auto_dream/`.
fn lock_dir_for_kernel(kernel: &LibreFangKernel) -> PathBuf {
    let cfg = kernel.config_snapshot();
    if cfg.auto_dream.lock_dir.is_empty() {
        kernel.data_dir().join(DEFAULT_LOCK_DIR)
    } else {
        PathBuf::from(&cfg.auto_dream.lock_dir)
    }
}

/// Build a lock handle for a specific agent.
fn lock_for_agent(kernel: &LibreFangKernel, agent_id: AgentId) -> ConsolidationLock {
    let path = lock_dir_for_kernel(kernel).join(format!("{agent_id}.lock"));
    ConsolidationLock::new(path)
}

/// Outcome of a single per-agent gate-check pass.
#[derive(Debug)]
enum AgentGateResult {
    /// All gates passed; consolidation should fire. Carries the pre-acquire
    /// mtime so rollback can rewind on failure.
    Fire { prior_mtime: u64 },
    /// Time gate blocked — not enough hours since last dream.
    TooSoon { hours_remaining: f64 },
    /// Session-count gate blocked — this agent has been idle.
    NoActivity { sessions_since: u32, required: u32 },
    /// Lock held by another process. Silent skip.
    LockHeld,
    /// Underlying read failed — skip this agent this tick.
    Skipped(String),
}

/// Per-agent gate check. Skips the time gate when `bypass_time_gate` is true
/// (used by the manual-trigger API path) but still respects the lock.
async fn check_agent_gates(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    bypass_time_gate: bool,
) -> AgentGateResult {
    let cfg = kernel.config_snapshot();
    let lock = lock_for_agent(kernel, agent_id);

    let last_at = match lock.read_last_consolidated_at().await {
        Ok(t) => t,
        Err(e) => return AgentGateResult::Skipped(format!("read lock failed: {e}")),
    };

    if !bypass_time_gate {
        let now = now_ms();
        let hours_since = ((now.saturating_sub(last_at)) as f64) / 3_600_000.0;
        if hours_since < cfg.auto_dream.min_hours {
            return AgentGateResult::TooSoon {
                hours_remaining: cfg.auto_dream.min_hours - hours_since,
            };
        }
    }

    // Session-count gate. Skipped when min_sessions is 0 OR when this is a
    // manual trigger — users hitting "dream now" want the dream regardless
    // of activity.
    if cfg.auto_dream.min_sessions > 0 && !bypass_time_gate {
        match kernel
            .memory_substrate()
            .count_agent_sessions_touched_since(agent_id, last_at)
        {
            Ok(count) if count < cfg.auto_dream.min_sessions => {
                return AgentGateResult::NoActivity {
                    sessions_since: count,
                    required: cfg.auto_dream.min_sessions,
                };
            }
            Ok(_) => {}
            Err(e) => {
                // Don't block the dream on a transient substrate error —
                // fall through and let time+lock decide.
                tracing::warn!(agent = %agent_id, error = %e, "auto_dream: session count query failed, skipping gate");
            }
        }
    }

    match lock.try_acquire().await {
        Ok(Some(prior_mtime)) => AgentGateResult::Fire { prior_mtime },
        Ok(None) => AgentGateResult::LockHeld,
        Err(e) => AgentGateResult::Skipped(format!("lock acquire failed: {e}")),
    }
}

/// Fire a dream consolidation against the given agent. Uses a synthetic
/// sender context so session scoping tags the turn as `auto_dream` — this
/// isolates dream turns from regular user/channel sessions.
async fn fire_dream(
    kernel: Arc<LibreFangKernel>,
    target: AgentId,
    prior_mtime: u64,
    timeout_secs: u64,
) {
    let prompt_text = prompt::build_consolidation_prompt();
    let sender = SenderContext {
        channel: AUTO_DREAM_CHANNEL.to_string(),
        user_id: String::new(),
        display_name: AUTO_DREAM_CHANNEL.to_string(),
        is_group: false,
        was_mentioned: false,
        thread_id: None,
        account_id: None,
        ..Default::default()
    };

    let timeout = Duration::from_secs(timeout_secs.max(30));
    let send_fut = kernel.send_message_with_sender_context(target, &prompt_text, &sender);

    match tokio::time::timeout(timeout, send_fut).await {
        Ok(Ok(result)) => {
            tracing::info!(
                agent = %target,
                response_len = result.response.len(),
                "auto_dream: consolidation completed"
            );
        }
        Ok(Err(e)) => {
            tracing::warn!(agent = %target, error = %e, "auto_dream: dream turn failed, rolling back lock");
            let lock = lock_for_agent(&kernel, target);
            if let Err(rerr) = lock.rollback(prior_mtime).await {
                tracing::warn!(error = %rerr, "auto_dream: rollback after failure also failed");
            }
        }
        Err(_) => {
            tracing::warn!(
                agent = %target,
                timeout_s = timeout_secs,
                "auto_dream: consolidation timed out, rolling back lock"
            );
            let lock = lock_for_agent(&kernel, target);
            if let Err(rerr) = lock.rollback(prior_mtime).await {
                tracing::warn!(error = %rerr, "auto_dream: rollback after timeout also failed");
            }
        }
    }
}

/// Enumerate agents that have opted into auto-dream on their manifest.
fn enrolled_agents(kernel: &LibreFangKernel) -> Vec<(AgentId, String)> {
    kernel
        .agent_registry()
        .list()
        .into_iter()
        .filter(|e| e.manifest.auto_dream_enabled)
        .map(|e| (e.id, e.name))
        .collect()
}

/// Spawn the auto-dream scheduler loop on the current tokio runtime.
///
/// Idempotent at the module level — callers should only invoke this once per
/// kernel boot, alongside the other background tasks.
pub fn spawn_scheduler(kernel: Arc<LibreFangKernel>) {
    tokio::spawn(async move {
        // Log effective config once at startup so operators can verify the
        // feature is really on.
        {
            let cfg = kernel.config_snapshot();
            if cfg.auto_dream.enabled {
                tracing::info!(
                    min_hours = cfg.auto_dream.min_hours,
                    min_sessions = cfg.auto_dream.min_sessions,
                    check_interval_s = cfg.auto_dream.check_interval_secs,
                    "auto_dream: enabled (per-agent opt-in via manifest)"
                );
            } else {
                tracing::debug!("auto_dream: disabled");
            }
        }

        loop {
            // Read interval fresh each tick so config hot-reload takes effect.
            let interval_s = {
                let cfg = kernel.config_snapshot();
                cfg.auto_dream.check_interval_secs.max(30)
            };
            tokio::time::sleep(Duration::from_secs(interval_s)).await;

            if kernel.supervisor.is_shutting_down() {
                tracing::debug!("auto_dream: shutdown detected, scheduler exiting");
                return;
            }

            let cfg = kernel.config_snapshot();
            if !cfg.auto_dream.enabled {
                continue;
            }

            let timeout_s = cfg.auto_dream.timeout_secs;
            for (agent_id, name) in enrolled_agents(&kernel) {
                match check_agent_gates(&kernel, agent_id, false).await {
                    AgentGateResult::Fire { prior_mtime } => {
                        // Each dream runs serially inside the scheduler loop
                        // so the tick doesn't fan out too much CPU/IO. If N
                        // agents all pass simultaneously, they still dream
                        // one at a time — this keeps token-cost predictable.
                        fire_dream(Arc::clone(&kernel), agent_id, prior_mtime, timeout_s).await;
                    }
                    AgentGateResult::TooSoon { hours_remaining } => {
                        tracing::trace!(
                            agent = %agent_id,
                            hours_remaining,
                            "auto_dream: time gate not yet open"
                        );
                    }
                    AgentGateResult::NoActivity {
                        sessions_since,
                        required,
                    } => {
                        tracing::trace!(
                            agent = %agent_id,
                            sessions_since,
                            required,
                            "auto_dream: agent idle, session gate not met"
                        );
                    }
                    AgentGateResult::LockHeld => {
                        tracing::debug!(agent = %agent_id, "auto_dream: lock held, skipping tick");
                    }
                    AgentGateResult::Skipped(reason) => {
                        tracing::warn!(agent = %agent_id, name = %name, reason, "auto_dream: skipped agent this tick");
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// HTTP-facing status / trigger helpers
// ---------------------------------------------------------------------------

/// Per-agent auto-dream status, returned by `GET /api/auto-dream/status`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoDreamAgentStatus {
    pub agent_id: String,
    pub agent_name: String,
    pub auto_dream_enabled: bool,
    pub last_consolidated_at_ms: u64,
    pub next_eligible_at_ms: u64,
    pub hours_since_last: f64,
    pub sessions_since_last: u32,
    pub lock_path: String,
}

/// Global auto-dream status.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoDreamStatus {
    pub enabled: bool,
    pub min_hours: f64,
    pub min_sessions: u32,
    pub check_interval_secs: u64,
    pub lock_dir: String,
    /// One entry per opt-in agent. Agents without `auto_dream_enabled = true`
    /// on their manifest are omitted — use `/api/agents` for the full list.
    pub agents: Vec<AutoDreamAgentStatus>,
}

/// Snapshot the current auto-dream state. Cheap: one stat + one count query
/// per enrolled agent. Non-enrolled agents are skipped entirely.
pub async fn current_status(kernel: &LibreFangKernel) -> AutoDreamStatus {
    let cfg = kernel.config_snapshot();
    let lock_dir = lock_dir_for_kernel(kernel);
    let now = now_ms();
    let min_ms = (cfg.auto_dream.min_hours * 3_600_000.0) as u64;

    let mut agents = Vec::new();
    for (agent_id, name) in enrolled_agents(kernel) {
        let lock = lock_for_agent(kernel, agent_id);
        let last = lock.read_last_consolidated_at().await.unwrap_or(0);
        let hours_since = if last == 0 {
            f64::INFINITY
        } else {
            ((now.saturating_sub(last)) as f64) / 3_600_000.0
        };
        let sessions_since = kernel
            .memory_substrate()
            .count_agent_sessions_touched_since(agent_id, last)
            .unwrap_or(0);
        agents.push(AutoDreamAgentStatus {
            agent_id: agent_id.to_string(),
            agent_name: name,
            auto_dream_enabled: true,
            last_consolidated_at_ms: last,
            next_eligible_at_ms: last.saturating_add(min_ms),
            hours_since_last: hours_since,
            sessions_since_last: sessions_since,
            lock_path: lock.path().display().to_string(),
        });
    }

    AutoDreamStatus {
        enabled: cfg.auto_dream.enabled,
        min_hours: cfg.auto_dream.min_hours,
        min_sessions: cfg.auto_dream.min_sessions,
        check_interval_secs: cfg.auto_dream.check_interval_secs,
        lock_dir: lock_dir.display().to_string(),
        agents,
    }
}

/// Outcome of a manual trigger request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TriggerOutcome {
    pub fired: bool,
    pub agent_id: String,
    pub reason: String,
}

/// Manually trigger a consolidation for a specific agent. Bypasses the time
/// and session gates (user explicitly asked for a dream) but **still respects
/// the lock** so two overlapping triggers cannot double-fire.
///
/// Returns before the dream finishes: consolidation runs in a detached task
/// so the HTTP response isn't blocked on the LLM call.
pub async fn trigger_manual(kernel: Arc<LibreFangKernel>, agent_id: AgentId) -> TriggerOutcome {
    let id_str = agent_id.to_string();
    let cfg = kernel.config_snapshot();
    if !cfg.auto_dream.enabled {
        return TriggerOutcome {
            fired: false,
            agent_id: id_str,
            reason: "auto-dream is disabled in config".to_string(),
        };
    }

    // The target must actually exist. We don't *require* the manifest flag
    // for manual triggers — operators may want to one-off dream an agent
    // that isn't normally enrolled.
    if kernel.agent_registry().get(agent_id).is_none() {
        return TriggerOutcome {
            fired: false,
            agent_id: id_str,
            reason: "agent not found".to_string(),
        };
    }

    match check_agent_gates(&kernel, agent_id, true).await {
        AgentGateResult::Fire { prior_mtime } => {
            let timeout_s = cfg.auto_dream.timeout_secs;
            let k = Arc::clone(&kernel);
            tokio::spawn(async move {
                fire_dream(k, agent_id, prior_mtime, timeout_s).await;
            });
            TriggerOutcome {
                fired: true,
                agent_id: id_str,
                reason: "consolidation fired".to_string(),
            }
        }
        AgentGateResult::LockHeld => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            reason: "lock is held — consolidation already in progress".to_string(),
        },
        AgentGateResult::Skipped(reason) => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            reason,
        },
        // Time/session gates are bypassed for manual triggers, so these
        // arms are unreachable in practice — handle defensively.
        AgentGateResult::TooSoon { .. } | AgentGateResult::NoActivity { .. } => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            reason: "unexpected gate outcome for manual trigger".to_string(),
        },
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
