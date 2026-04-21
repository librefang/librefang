//! Session auto-reset policy for the LibreFang kernel.
//!
//! Implements idle-timeout and daily fixed-time session reset strategies,
//! mirroring the `SessionResetPolicy` logic from hermes-agent/gateway/session.py.
//!
//! # Strategies
//! - `Off`   – no automatic reset (default, fully backward-compatible)
//! - `Idle`  – reset when last-active is older than `idle_minutes`
//! - `Daily` – reset once per day at a fixed local hour (`daily_at_hour`)
//! - `Both`  – reset when *either* condition is satisfied
//!
//! # Special flags (on [`AgentEntry`])
//! - `suspended`      – force a hard reset on next access (broken-session recovery)
//! - `resume_pending` – preserve session_id across restart interruptions
//! - `reset_reason`   – last recorded reset cause for observability

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which automatic-reset strategy is active.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    /// No automatic reset. Existing sessions persist indefinitely. (default)
    #[default]
    Off,
    /// Reset after `idle_minutes` of inactivity.
    Idle,
    /// Reset once per day at `daily_at_hour` (local clock, 0-23).
    Daily,
    /// Reset when *either* idle or daily condition is satisfied.
    Both,
}

/// Why a session was reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetReason {
    /// Last-active exceeded `idle_minutes`.
    Idle,
    /// The daily fixed-time boundary was crossed.
    Daily,
    /// Session was flagged `suspended` (forced by operator / stuck-loop recovery).
    Suspended,
    /// Manual reset requested via API or CLI.
    Manual,
}

impl std::fmt::Display for ResetReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => f.write_str("idle"),
            Self::Daily => f.write_str("daily"),
            Self::Suspended => f.write_str("suspended"),
            Self::Manual => f.write_str("manual"),
        }
    }
}

/// Per-agent (or global) session auto-reset policy.
///
/// The default is `mode = Off`, which keeps all pre-existing behaviour.
///
/// ```toml
/// [session.reset]
/// mode = "idle"
/// idle_minutes = 1440   # 24 h
///
/// # or
/// mode = "both"
/// idle_minutes = 60
/// daily_at_hour = 4
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionResetPolicy {
    /// Which strategy (or strategies) to apply.
    pub mode: ResetMode,
    /// Inactivity threshold in minutes for the `Idle` / `Both` modes.
    /// Default: 1440 (24 hours).
    pub idle_minutes: u64,
    /// Hour of day (0-23, local clock) at which the `Daily` / `Both` reset fires.
    /// Default: 4 (04:00 local).
    pub daily_at_hour: u8,
}

impl Default for SessionResetPolicy {
    fn default() -> Self {
        Self {
            mode: ResetMode::Off,
            idle_minutes: 1440,
            daily_at_hour: 4,
        }
    }
}

impl SessionResetPolicy {
    /// Evaluate whether a session should be reset.
    ///
    /// # Parameters
    /// - `last_active`  – [`SystemTime`] of the last user/agent interaction.
    /// - `suspended`    – when `true`, always returns [`ResetReason::Suspended`]
    ///                    regardless of the configured mode (hard-wipe flag).
    ///
    /// Returns `Some(reason)` if the session should be reset, `None` if it is
    /// still valid.
    pub fn should_reset(&self, last_active: SystemTime, suspended: bool) -> Option<ResetReason> {
        // The `suspended` flag is a hard forced-wipe signal that bypasses
        // every other check.
        if suspended {
            return Some(ResetReason::Suspended);
        }

        if self.mode == ResetMode::Off {
            return None;
        }

        let now = SystemTime::now();

        if matches!(self.mode, ResetMode::Idle | ResetMode::Both) {
            let idle_threshold = Duration::from_secs(self.idle_minutes * 60);
            if now
                .duration_since(last_active)
                .map(|elapsed| elapsed >= idle_threshold)
                .unwrap_or(false)
            {
                return Some(ResetReason::Idle);
            }
        }

        if matches!(self.mode, ResetMode::Daily | ResetMode::Both) {
            // Guard: daily_at_hour must be 0-23.  Values ≥ 24 would produce a
            // target_secs_into_day that exceeds 86 400, causing the boundary
            // calculation to pick yesterday's slot every time and fire on
            // every invocation.  Treat out-of-range values as misconfiguration
            // and skip the check entirely rather than silently misfiring.
            if self.daily_at_hour <= 23 && self.crossed_daily_boundary(last_active, now) {
                return Some(ResetReason::Daily);
            }
        }

        None
    }

    /// Returns `true` when `last_active` was before the most-recent occurrence
    /// of `daily_at_hour:00:00` (local time), and that occurrence is ≤ `now`.
    ///
    /// Implementation: work in UTC seconds, offset by the local UTC offset
    /// inferred from [`chrono::Local`] when that crate is available.  We keep
    /// the dependency surface small by computing the local offset once per call
    /// via the system timezone.
    fn crossed_daily_boundary(&self, last_active: SystemTime, now: SystemTime) -> bool {
        // Convert SystemTime → seconds since UNIX epoch.
        let last_secs = system_time_to_secs(last_active);
        let now_secs = system_time_to_secs(now);

        // Determine the local UTC offset in seconds (sign: east = positive).
        let utc_offset_secs = local_utc_offset_secs();

        // Local day seconds for `now` and `last_active`.
        let now_local = now_secs + utc_offset_secs;
        let last_local = last_secs + utc_offset_secs;

        let secs_per_day: i64 = 86_400;
        let target_secs_into_day = (self.daily_at_hour as i64) * 3600;

        // Midnight (00:00) of the current local day (epoch-aligned).
        let now_day_start = (now_local / secs_per_day) * secs_per_day;
        let reset_today = now_day_start + target_secs_into_day;

        // Most-recent reset boundary that has already passed.
        let last_boundary = if now_local >= now_day_start + target_secs_into_day {
            reset_today
        } else {
            reset_today - secs_per_day
        };

        // `last_active` is before that boundary.
        last_local < last_boundary
    }
}

// ---------------------------------------------------------------------------
// Small helpers (no extra deps)
// ---------------------------------------------------------------------------

fn system_time_to_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Approximate local UTC offset by comparing `std::time::SystemTime::now()`
/// to `chrono::Local::now()` — but since we can't take chrono as a dep from
/// inside the kernel without checking, we use a fallback approach:
/// read the `TZ` environment variable and parse a numeric offset, or use 0.
///
/// In practice LibreFang runs in environments where either chrono is already
/// a transitive dep (it is, via `librefang-types`) or the local offset is UTC.
/// For correctness we try chrono first via the types crate.
fn local_utc_offset_secs() -> i64 {
    // chrono is a dependency of librefang-types which is always in scope.
    use chrono::{Local, Offset};
    let offset = Local::now().offset().fix();
    offset.local_minus_utc() as i64
}

// ---------------------------------------------------------------------------
// Conversions from librefang-types config types
// ---------------------------------------------------------------------------

impl From<librefang_types::config::SessionResetMode> for ResetMode {
    fn from(m: librefang_types::config::SessionResetMode) -> Self {
        use librefang_types::config::SessionResetMode as T;
        match m {
            T::Off => ResetMode::Off,
            T::Idle => ResetMode::Idle,
            T::Daily => ResetMode::Daily,
            T::Both => ResetMode::Both,
        }
    }
}

impl From<librefang_types::config::SessionResetPolicy> for SessionResetPolicy {
    fn from(p: librefang_types::config::SessionResetPolicy) -> Self {
        SessionResetPolicy {
            mode: p.mode.into(),
            idle_minutes: p.idle_minutes,
            daily_at_hour: p.daily_at_hour,
        }
    }
}

/// Convert a kernel-internal [`ResetReason`] into the types-layer
/// [`librefang_types::config::SessionResetReason`] for storage on `AgentEntry`.
impl From<ResetReason> for librefang_types::config::SessionResetReason {
    fn from(r: ResetReason) -> Self {
        use librefang_types::config::SessionResetReason as T;
        match r {
            ResetReason::Idle => T::Idle,
            ResetReason::Daily => T::Daily,
            ResetReason::Suspended => T::Suspended,
            ResetReason::Manual => T::Manual,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn mins_ago(m: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(m * 60)
    }

    #[test]
    fn off_mode_never_resets() {
        let policy = SessionResetPolicy {
            mode: ResetMode::Off,
            ..Default::default()
        };
        // Even with a very old last_active, Off mode returns None.
        assert_eq!(policy.should_reset(mins_ago(10_000), false), None);
    }

    #[test]
    fn suspended_always_resets() {
        let policy = SessionResetPolicy::default(); // Off mode
        assert_eq!(
            policy.should_reset(mins_ago(1), true),
            Some(ResetReason::Suspended)
        );
    }

    #[test]
    fn idle_triggers_after_threshold() {
        let policy = SessionResetPolicy {
            mode: ResetMode::Idle,
            idle_minutes: 30,
            ..Default::default()
        };
        // Active 31 minutes ago → should reset
        assert_eq!(
            policy.should_reset(mins_ago(31), false),
            Some(ResetReason::Idle)
        );
        // Active 29 minutes ago → still valid
        assert_eq!(policy.should_reset(mins_ago(29), false), None);
    }

    #[test]
    fn idle_does_not_trigger_below_threshold() {
        let policy = SessionResetPolicy {
            mode: ResetMode::Idle,
            idle_minutes: 1440,
            ..Default::default()
        };
        assert_eq!(policy.should_reset(mins_ago(100), false), None);
    }

    #[test]
    fn both_mode_idle_wins_first() {
        let policy = SessionResetPolicy {
            mode: ResetMode::Both,
            idle_minutes: 10,
            daily_at_hour: 4,
        };
        assert_eq!(
            policy.should_reset(mins_ago(15), false),
            Some(ResetReason::Idle)
        );
    }

    #[test]
    fn suspended_overrides_off_mode() {
        let policy = SessionResetPolicy {
            mode: ResetMode::Off,
            ..Default::default()
        };
        assert_eq!(
            policy.should_reset(SystemTime::now(), true),
            Some(ResetReason::Suspended)
        );
    }

    #[test]
    fn default_policy_is_off() {
        let policy = SessionResetPolicy::default();
        assert_eq!(policy.mode, ResetMode::Off);
        assert_eq!(policy.idle_minutes, 1440);
        assert_eq!(policy.daily_at_hour, 4);
    }
}
