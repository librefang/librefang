//! Auto-dream: periodic background memory consolidation.
//!
//! Port of libre-code's `autoDream` — a time-gated background task that asks
//! a designated agent to reflect on its memory and consolidate recent signal.
//!
//! # Gates
//!
//! 1. **Enabled** — `config.auto_dream.enabled` must be true AND a valid
//!    `target_agent` must be set.
//! 2. **Time** — at least `min_hours` must have elapsed since the last
//!    recorded consolidation (mtime of the lock file).
//! 3. **Lock** — a filesystem lock with PID-staleness detection prevents two
//!    daemons on the same data directory from double-firing.
//!
//! The MVP intentionally **skips** libre-code's session-count gate — the
//! substrate doesn't have a convenient "messages since timestamp" query yet.
//! Time + lock alone give us the core safety property (no more than one
//! consolidation per `min_hours` per host), which is the important one.
//!
//! # Single-target MVP
//!
//! Currently consolidates a single designated agent's memory per host. A
//! future revision can lift this to per-agent scheduling once the substrate
//! exposes per-agent activity queries.

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

/// Default lock-file name inside `data_dir` when the config doesn't override.
const DEFAULT_LOCK_FILE: &str = "auto_dream.lock";

/// Build the lock handle from the kernel's current config snapshot. Returns
/// `None` when no sensible location can be derived (shouldn't happen in
/// practice — `data_dir` is always set at boot).
fn lock_for_kernel(kernel: &LibreFangKernel) -> ConsolidationLock {
    let cfg = kernel.config_snapshot();
    let path = if cfg.auto_dream.lock_path.is_empty() {
        kernel.data_dir().join(DEFAULT_LOCK_FILE)
    } else {
        std::path::PathBuf::from(&cfg.auto_dream.lock_path)
    };
    ConsolidationLock::new(path)
}

/// Outcome of a single gate-check pass. Callers use the variant to decide
/// whether to fire consolidation.
#[derive(Debug)]
enum GateResult {
    /// All gates passed; consolidation should fire. Carries the pre-acquire
    /// mtime so rollback can rewind on failure.
    Fire { target: AgentId, prior_mtime: u64 },
    /// Feature disabled or misconfigured (no target agent).
    Disabled,
    /// Time-gate blocked. Carries hours remaining for logging.
    TooSoon { hours_remaining: f64 },
    /// Lock held by another process. Silent skip.
    LockHeld,
}

/// Run one pass of the gate check. Extracted from the tokio loop so the
/// manual-trigger API path can reuse the same logic (minus the time gate).
async fn check_gates(kernel: &LibreFangKernel) -> GateResult {
    let cfg = kernel.config_snapshot();
    if !cfg.auto_dream.enabled {
        return GateResult::Disabled;
    }
    let target = match cfg.auto_dream.target_agent.parse::<AgentId>() {
        Ok(id) => id,
        Err(_) => {
            // Empty string or malformed UUID. Logged once at startup; keep
            // the per-tick log at trace level to avoid noise.
            tracing::trace!("auto_dream: target_agent not set or invalid");
            return GateResult::Disabled;
        }
    };

    let lock = lock_for_kernel(kernel);

    // Time gate.
    let last_at = match lock.read_last_consolidated_at().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "auto_dream: read_last_consolidated_at failed");
            return GateResult::Disabled;
        }
    };
    let now_ms = now_ms();
    let hours_since = ((now_ms.saturating_sub(last_at)) as f64) / 3_600_000.0;
    if hours_since < cfg.auto_dream.min_hours {
        return GateResult::TooSoon {
            hours_remaining: cfg.auto_dream.min_hours - hours_since,
        };
    }

    // Lock gate.
    match lock.try_acquire().await {
        Ok(Some(prior_mtime)) => GateResult::Fire {
            target,
            prior_mtime,
        },
        Ok(None) => GateResult::LockHeld,
        Err(e) => {
            tracing::warn!(error = %e, "auto_dream: lock acquire failed");
            GateResult::Disabled
        }
    }
}

/// Fire a dream consolidation against the target agent. Uses a synthetic
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
            let lock = lock_for_kernel(&kernel);
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
            let lock = lock_for_kernel(&kernel);
            if let Err(rerr) = lock.rollback(prior_mtime).await {
                tracing::warn!(error = %rerr, "auto_dream: rollback after timeout also failed");
            }
        }
    }
}

/// Spawn the auto-dream scheduler loop on the current tokio runtime.
///
/// Idempotent at the module level — callers should only invoke this once per
/// kernel boot, alongside the other background tasks.
pub fn spawn_scheduler(kernel: Arc<LibreFangKernel>) {
    tokio::spawn(async move {
        // Initial log of the effective config so users see whether the
        // feature is on at startup, and the target agent is resolvable.
        {
            let cfg = kernel.config_snapshot();
            if cfg.auto_dream.enabled {
                match cfg.auto_dream.target_agent.parse::<AgentId>() {
                    Ok(id) => tracing::info!(
                        target = %id,
                        min_hours = cfg.auto_dream.min_hours,
                        check_interval_s = cfg.auto_dream.check_interval_secs,
                        "auto_dream: enabled"
                    ),
                    Err(_) => tracing::warn!(
                        target = %cfg.auto_dream.target_agent,
                        "auto_dream: enabled but target_agent is not a valid UUID — feature inert"
                    ),
                }
            } else {
                tracing::debug!("auto_dream: disabled");
            }
        }

        // The interval is read fresh each tick (not once at spawn) so config
        // hot-reload lowers/raises the cadence without a restart.
        loop {
            let interval_s = {
                let cfg = kernel.config_snapshot();
                cfg.auto_dream.check_interval_secs.max(30)
            };
            tokio::time::sleep(Duration::from_secs(interval_s)).await;

            if kernel.supervisor.is_shutting_down() {
                tracing::debug!("auto_dream: shutdown detected, scheduler exiting");
                return;
            }

            match check_gates(&kernel).await {
                GateResult::Fire {
                    target,
                    prior_mtime,
                } => {
                    let timeout_s = kernel.config_snapshot().auto_dream.timeout_secs;
                    fire_dream(Arc::clone(&kernel), target, prior_mtime, timeout_s).await;
                }
                GateResult::TooSoon { hours_remaining } => {
                    tracing::trace!(hours_remaining, "auto_dream: time gate not open yet");
                }
                GateResult::LockHeld => {
                    tracing::debug!("auto_dream: lock held by another process, skipping tick");
                }
                GateResult::Disabled => {
                    // Quiet — normal state when feature is off.
                }
            }
        }
    });
}

/// Public status object returned by `GET /api/auto-dream/status`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoDreamStatus {
    pub enabled: bool,
    pub target_agent: String,
    pub min_hours: f64,
    pub check_interval_secs: u64,
    pub last_consolidated_at_ms: u64,
    pub next_eligible_at_ms: u64,
    pub hours_since_last: f64,
    pub lock_path: String,
}

/// Compute the current auto-dream status for the status endpoint. Cheap —
/// one stat of the lock file plus config read.
pub async fn current_status(kernel: &LibreFangKernel) -> AutoDreamStatus {
    let cfg = kernel.config_snapshot();
    let lock = lock_for_kernel(kernel);
    let last = lock.read_last_consolidated_at().await.unwrap_or(0);
    let now = now_ms();
    let hours_since = if last == 0 {
        f64::INFINITY
    } else {
        ((now.saturating_sub(last)) as f64) / 3_600_000.0
    };
    let min_ms = (cfg.auto_dream.min_hours * 3_600_000.0) as u64;
    let next_eligible = last.saturating_add(min_ms);

    AutoDreamStatus {
        enabled: cfg.auto_dream.enabled,
        target_agent: cfg.auto_dream.target_agent.clone(),
        min_hours: cfg.auto_dream.min_hours,
        check_interval_secs: cfg.auto_dream.check_interval_secs,
        last_consolidated_at_ms: last,
        next_eligible_at_ms: next_eligible,
        hours_since_last: hours_since,
        lock_path: lock.path().display().to_string(),
    }
}

/// Outcome of a manual trigger request. Mirrors the API response shape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TriggerOutcome {
    pub fired: bool,
    pub reason: String,
}

/// Manually trigger a consolidation. Bypasses the time gate (for ad-hoc
/// user-initiated "dream now" requests) but **still respects the lock** —
/// two concurrent triggers can't double-fire.
///
/// Returns before the dream finishes: consolidation runs to completion in a
/// detached task so the HTTP response isn't blocked on the LLM call.
pub async fn trigger_manual(kernel: Arc<LibreFangKernel>) -> TriggerOutcome {
    let cfg = kernel.config_snapshot();
    if !cfg.auto_dream.enabled {
        return TriggerOutcome {
            fired: false,
            reason: "auto-dream is disabled in config".to_string(),
        };
    }
    let target = match cfg.auto_dream.target_agent.parse::<AgentId>() {
        Ok(id) => id,
        Err(_) => {
            return TriggerOutcome {
                fired: false,
                reason: "target_agent is not set or not a valid UUID".to_string(),
            };
        }
    };

    let lock = lock_for_kernel(&kernel);
    match lock.try_acquire().await {
        Ok(Some(prior_mtime)) => {
            let timeout_s = cfg.auto_dream.timeout_secs;
            let k = Arc::clone(&kernel);
            tokio::spawn(async move {
                fire_dream(k, target, prior_mtime, timeout_s).await;
            });
            TriggerOutcome {
                fired: true,
                reason: "consolidation fired".to_string(),
            }
        }
        Ok(None) => TriggerOutcome {
            fired: false,
            reason: "lock is held — consolidation already in progress".to_string(),
        },
        Err(e) => TriggerOutcome {
            fired: false,
            reason: format!("lock acquire failed: {e}"),
        },
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
