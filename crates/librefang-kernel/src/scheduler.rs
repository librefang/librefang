//! Agent scheduler — manages agent execution and resource tracking.

use dashmap::DashMap;
use librefang_types::agent::{AgentId, ResourceQuota};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::TokenUsage;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::debug;

/// Snapshot of usage stats returned by [`AgentScheduler::get_usage`].
#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub llm_calls: u64,
}

/// Tracks resource usage for an agent with a rolling hourly window.
#[derive(Debug)]
pub struct UsageTracker {
    /// Total tokens consumed within the current hourly window.
    pub total_tokens: u64,
    /// Input tokens consumed within the current hourly window.
    pub input_tokens: u64,
    /// Output tokens consumed within the current hourly window.
    pub output_tokens: u64,
    /// Total tool calls made (lifetime counter for snapshot).
    pub tool_calls: u64,
    /// Total LLM API calls made within the current hourly window.
    pub llm_calls: u64,
    /// Start of the current hourly usage window.
    pub window_start: Instant,
    /// Sliding window of tool-call timestamps for per-minute rate limiting.
    pub tool_call_timestamps: VecDeque<Instant>,
    /// Sliding window of (timestamp, token_count) for burst limiting.
    /// Prevents burning the entire hourly quota in a single minute.
    pub token_timestamps: VecDeque<(Instant, u64)>,
}

/// One minute as a Duration constant.
const ONE_MINUTE: Duration = Duration::from_secs(60);
/// One hour as a Duration constant.
const ONE_HOUR: Duration = Duration::from_secs(3600);

impl Default for UsageTracker {
    fn default() -> Self {
        Self {
            total_tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            llm_calls: 0,
            window_start: Instant::now(),
            tool_call_timestamps: VecDeque::new(),
            token_timestamps: VecDeque::new(),
        }
    }
}

impl UsageTracker {
    /// Reset counters if the current window has expired (1 hour).
    fn reset_if_expired(&mut self) {
        if self.window_start.elapsed() >= ONE_HOUR {
            self.total_tokens = 0;
            self.input_tokens = 0;
            self.output_tokens = 0;
            self.tool_calls = 0;
            self.llm_calls = 0;
            self.window_start = Instant::now();
            self.tool_call_timestamps.clear();
            self.token_timestamps.clear();
        }
    }

    /// Evict tool-call timestamps older than 1 minute and return how many remain.
    fn tool_calls_in_last_minute(&mut self) -> u32 {
        let cutoff = Instant::now() - ONE_MINUTE;
        while self
            .tool_call_timestamps
            .front()
            .is_some_and(|t| *t < cutoff)
        {
            self.tool_call_timestamps.pop_front();
        }
        self.tool_call_timestamps.len() as u32
    }

    /// Return total tokens consumed in the last minute (burst window).
    fn tokens_in_last_minute(&mut self) -> u64 {
        let cutoff = Instant::now() - ONE_MINUTE;
        while self
            .token_timestamps
            .front()
            .is_some_and(|(t, _)| *t < cutoff)
        {
            self.token_timestamps.pop_front();
        }
        self.token_timestamps.iter().map(|(_, n)| n).sum()
    }
}

/// The agent scheduler manages execution ordering and resource quotas.
pub struct AgentScheduler {
    /// Resource quotas per agent.
    quotas: DashMap<AgentId, ResourceQuota>,
    /// Usage tracking per agent.
    usage: DashMap<AgentId, UsageTracker>,
    /// Active task handles per agent.
    tasks: DashMap<AgentId, JoinHandle<()>>,
}

impl AgentScheduler {
    /// Create a new scheduler.
    pub fn new() -> Self {
        Self {
            quotas: DashMap::new(),
            usage: DashMap::new(),
            tasks: DashMap::new(),
        }
    }

    /// Register an agent with its resource quota.
    pub fn register(&self, agent_id: AgentId, quota: ResourceQuota) {
        self.quotas.insert(agent_id, quota);
        self.usage.insert(agent_id, UsageTracker::default());
    }

    /// Update an agent's resource quota **without** resetting its usage
    /// tracker. Use this when hot-reloading `agent.toml` so accumulated
    /// LLM-token / tool-call counts stay accurate but the new limits
    /// take effect immediately. Issue #2317.
    pub fn update_quota(&self, agent_id: AgentId, quota: ResourceQuota) {
        self.quotas.insert(agent_id, quota);
    }

    /// Record token usage for an agent.
    pub fn record_usage(&self, agent_id: AgentId, usage: &TokenUsage) {
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.reset_if_expired();
            let total = usage.total();
            tracker.total_tokens += total;
            tracker.input_tokens += usage.input_tokens;
            tracker.output_tokens += usage.output_tokens;
            tracker.llm_calls += 1;
            // Record in the per-minute sliding window for burst detection
            tracker.token_timestamps.push_back((Instant::now(), total));
        }
    }

    /// Record tool calls for an agent (call after each LLM turn that used tools).
    pub fn record_tool_calls(&self, agent_id: AgentId, count: u32) {
        if count == 0 {
            return;
        }
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.reset_if_expired();
            let now = Instant::now();
            for _ in 0..count {
                tracker.tool_call_timestamps.push_back(now);
            }
            tracker.tool_calls += u64::from(count);
        }
    }

    /// Check if an agent has exceeded its quota.
    pub fn check_quota(&self, agent_id: AgentId) -> LibreFangResult<()> {
        let quota = match self.quotas.get(&agent_id) {
            Some(q) => q.clone(),
            None => return Ok(()), // No quota = no limit
        };
        let mut tracker = match self.usage.get_mut(&agent_id) {
            Some(t) => t,
            None => return Ok(()),
        };

        // Reset the window if an hour has passed
        tracker.reset_if_expired();

        // --- Token limits (hourly) ---
        let token_limit = quota.effective_token_limit();
        if token_limit > 0 && tracker.total_tokens > token_limit {
            return Err(LibreFangError::QuotaExceeded(format!(
                "Token limit exceeded: {} / {}",
                tracker.total_tokens, token_limit
            )));
        }

        // --- Burst limit: no more than 1/5 of the hourly token budget in any single minute ---
        if token_limit > 0 {
            let burst_cap = token_limit / 5;
            let tokens_last_min = tracker.tokens_in_last_minute();
            if burst_cap > 0 && tokens_last_min > burst_cap {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Token burst limit exceeded: {} tokens in last minute (max {}/min)",
                    tokens_last_min, burst_cap
                )));
            }
        }

        // --- Tool-call rate limit (per minute) ---
        if quota.max_tool_calls_per_minute > 0 {
            let recent = tracker.tool_calls_in_last_minute();
            if recent >= quota.max_tool_calls_per_minute {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Tool call rate limit exceeded: {} / {} per minute",
                    recent, quota.max_tool_calls_per_minute
                )));
            }
        }

        Ok(())
    }

    /// Atomically check the per-agent quota **and** pre-charge an estimated
    /// token budget.
    ///
    /// This closes the TOCTOU window between `check_quota` and
    /// `record_usage`: N concurrent callers all calling `check_quota` before
    /// any of them calls `record_usage` can each individually pass the check
    /// while the combined spend blows past the limit.  By reserving
    /// `estimated_tokens` inside the same DashMap entry write-lock, at most
    /// one caller can pass for any given budget slot.
    ///
    /// **Pessimistic by design.** Callers pass the model's `max_tokens`
    /// (output cap) which is almost always larger than the real per-call
    /// usage. This is intentional — the quota holds firm under concurrent
    /// bursts at the cost of triggering `QuotaExceeded` slightly earlier
    /// than perfectly-tight accounting would. `settle_reservation` corrects
    /// `total_tokens` down to the actual amount once the call finishes, so
    /// over the long run the counters remain accurate.
    ///
    /// After the LLM call completes, the caller **must** call
    /// `settle_reservation` with the actual [`TokenUsage`] so the
    /// reservation is corrected and the sliding-window counters are updated.
    /// **Do not call `record_usage` for a pre-charged call** — `settle_reservation`
    /// does both jobs in one atomic step.
    ///
    /// Returns `Ok(estimated_tokens)` (the amount reserved) on success, or
    /// `Err(QuotaExceeded)` if the reservation would breach the limit.
    /// Returns `Ok(0)` when no quota is configured for the agent (caller
    /// should still call `record_usage` normally in that case).
    pub fn check_quota_and_reserve(
        &self,
        agent_id: AgentId,
        estimated_tokens: u64,
    ) -> LibreFangResult<u64> {
        let quota = match self.quotas.get(&agent_id) {
            Some(q) => q.clone(),
            None => return Ok(0), // No quota = no limit; nothing to reserve
        };
        let mut tracker = match self.usage.get_mut(&agent_id) {
            Some(t) => t,
            None => return Ok(0),
        };

        tracker.reset_if_expired();

        let token_limit = quota.effective_token_limit();
        if token_limit > 0 {
            let projected = tracker.total_tokens.saturating_add(estimated_tokens);
            if projected > token_limit {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Token limit would be exceeded: {} + {} reserved > {}",
                    tracker.total_tokens, estimated_tokens, token_limit
                )));
            }
            // Burst check against the projected spend
            let burst_cap = token_limit / 5;
            let tokens_last_min = tracker.tokens_in_last_minute();
            if burst_cap > 0 && tokens_last_min.saturating_add(estimated_tokens) > burst_cap {
                return Err(LibreFangError::QuotaExceeded(format!(
                    "Token burst limit would be exceeded: {} + {} reserved in last minute (max {}/min)",
                    tokens_last_min, estimated_tokens, burst_cap
                )));
            }
            // Atomically pre-charge inside the same DashMap entry write-lock
            tracker.total_tokens = projected;
        }

        Ok(estimated_tokens)
    }

    /// Settle a prior [`check_quota_and_reserve`] reservation.
    ///
    /// Replaces the pre-charged estimate in `total_tokens` with the actual
    /// token count consumed, and updates the sliding-window / per-dimension
    /// counters that [`record_usage`] normally maintains.  Callers MUST use
    /// this instead of `record_usage` after a pre-charged call so the
    /// counters are not double-incremented.
    ///
    /// When `estimated_tokens == 0` (no quota was configured) the function
    /// falls back to the same logic as `record_usage`.
    pub fn settle_reservation(&self, agent_id: AgentId, estimated_tokens: u64, usage: &TokenUsage) {
        let actual_tokens = usage.total();
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.reset_if_expired();

            if estimated_tokens > 0 {
                // Correct the pre-charged estimate to the actual amount:
                //   total_tokens was incremented by `estimated`; adjust it
                //   to reflect `actual` instead.
                tracker.total_tokens = tracker
                    .total_tokens
                    .saturating_sub(estimated_tokens)
                    .saturating_add(actual_tokens);
            } else {
                // No reservation was made (no quota) — behave like record_usage
                tracker.total_tokens += actual_tokens;
            }

            // Per-dimension counters (never pre-charged)
            tracker.input_tokens += usage.input_tokens;
            tracker.output_tokens += usage.output_tokens;
            tracker.llm_calls += 1;

            // Sliding-window for burst detection
            tracker
                .token_timestamps
                .push_back((Instant::now(), actual_tokens));
        }
    }

    /// Reset usage tracking for an agent (e.g. on session reset).
    pub fn reset_usage(&self, agent_id: AgentId) {
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.total_tokens = 0;
            tracker.input_tokens = 0;
            tracker.output_tokens = 0;
            tracker.tool_calls = 0;
            tracker.llm_calls = 0;
            tracker.window_start = Instant::now();
            tracker.tool_call_timestamps.clear();
            tracker.token_timestamps.clear();
        }
    }

    /// Abort an agent's active task.
    pub fn abort_task(&self, agent_id: AgentId) {
        if let Some((_, handle)) = self.tasks.remove(&agent_id) {
            handle.abort();
            debug!(agent = %agent_id, "Aborted agent task");
        }
    }

    /// Remove an agent from the scheduler.
    pub fn unregister(&self, agent_id: AgentId) {
        self.abort_task(agent_id);
        self.quotas.remove(&agent_id);
        self.usage.remove(&agent_id);
    }

    /// Get usage stats for an agent.
    pub fn get_usage(&self, agent_id: AgentId) -> Option<UsageSnapshot> {
        self.usage.get(&agent_id).map(|t| UsageSnapshot {
            total_tokens: t.total_tokens,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            tool_calls: t.tool_calls,
            llm_calls: t.llm_calls,
        })
    }
}

impl Default for AgentScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_usage() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        scheduler.register(id, ResourceQuota::default());
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        );
        let snap = scheduler.get_usage(id).unwrap();
        assert_eq!(snap.total_tokens, 150);
        assert_eq!(snap.input_tokens, 100);
        assert_eq!(snap.output_tokens, 50);
        assert_eq!(snap.llm_calls, 1);
    }

    #[test]
    fn test_quota_check() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(100),
            ..Default::default()
        };
        scheduler.register(id, quota);
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 60,
                output_tokens: 50,
                ..Default::default()
            },
        );
        assert!(scheduler.check_quota(id).is_err());
    }

    #[test]
    fn test_tool_call_rate_limit() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        let quota = ResourceQuota {
            max_tool_calls_per_minute: 5,
            max_llm_tokens_per_hour: Some(0), // unlimited tokens
            ..Default::default()
        };
        scheduler.register(id, quota);

        // 4 tool calls — should be fine
        scheduler.record_tool_calls(id, 4);
        assert!(scheduler.check_quota(id).is_ok());

        // 1 more — hits the limit (5 >= 5)
        scheduler.record_tool_calls(id, 1);
        assert!(scheduler.check_quota(id).is_err());
    }

    #[test]
    fn test_burst_limit() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        // 1000 tokens/hour => burst cap = 200/min
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(1000),
            max_tool_calls_per_minute: 0, // unlimited tool calls
            ..Default::default()
        };
        scheduler.register(id, quota);

        // Use 150 tokens — under burst cap
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        );
        assert!(scheduler.check_quota(id).is_ok());

        // Use 60 more — total in last minute = 210, exceeds burst cap of 200
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 30,
                output_tokens: 30,
                ..Default::default()
            },
        );
        assert!(scheduler.check_quota(id).is_err());
    }

    #[test]
    fn test_no_quota_no_limit() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        // No registration = no quota
        assert!(scheduler.check_quota(id).is_ok());
    }

    #[test]
    fn test_zero_limits_means_unlimited() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(0),
            max_tool_calls_per_minute: 0,
            ..Default::default()
        };
        scheduler.register(id, quota);

        // Record tons of usage — should never fail
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 999_999,
                output_tokens: 999_999,
                ..Default::default()
            },
        );
        scheduler.record_tool_calls(id, 9999);
        assert!(scheduler.check_quota(id).is_ok());
    }

    /// Regression test for #3736 — TOCTOU between check_quota and record_usage.
    ///
    /// Many threads racing through `check_quota_and_reserve` for the same
    /// agent must collectively reserve no more than `token_limit`. The old
    /// `check_quota` + `record_usage` split allowed all N to pass the check
    /// before any of them recorded usage; this test would fail under the
    /// old code with `succeeded > expected_max`.
    #[test]
    fn test_concurrent_check_and_reserve_respects_limit() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use std::thread;

        let scheduler = Arc::new(AgentScheduler::new());
        let id = AgentId::new();
        // 100 token-per-hour limit, each call wants 10 → at most 10 can pass.
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(100),
            max_tool_calls_per_minute: 0,
            ..Default::default()
        };
        scheduler.register(id, quota);

        let succeeded = Arc::new(AtomicU64::new(0));
        let denied = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        for _ in 0..50 {
            let sched = Arc::clone(&scheduler);
            let succ = Arc::clone(&succeeded);
            let den = Arc::clone(&denied);
            handles.push(thread::spawn(move || {
                match sched.check_quota_and_reserve(id, 10) {
                    Ok(_) => {
                        succ.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(_) => {
                        den.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let s = succeeded.load(Ordering::SeqCst);
        let d = denied.load(Ordering::SeqCst);
        assert_eq!(s + d, 50, "all 50 threads should have a verdict");
        // 10 reservations * 10 tokens = exactly 100 (the limit). 11 would
        // project to 110 > 100 and be denied. Burst cap is 100/5=20 → only
        // 2 can pass before burst kicks in. So success count is at most 2.
        assert!(
            s <= 10,
            "no more than 10 reservations of 10 tokens may fit in a 100-token quota, got {s}"
        );
    }

    /// Regression test for #3736 — settle_reservation must correctly adjust
    /// the pre-charged total to the actual token count.
    #[test]
    fn test_settle_reservation_corrects_overestimate() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(10_000),
            max_tool_calls_per_minute: 0,
            ..Default::default()
        };
        scheduler.register(id, quota);

        // Reserve 1000 (pessimistic); actual usage is 100.
        let reserved = scheduler.check_quota_and_reserve(id, 1000).unwrap();
        assert_eq!(reserved, 1000);
        let after_reserve = scheduler.get_usage(id).unwrap();
        assert_eq!(after_reserve.total_tokens, 1000);

        scheduler.settle_reservation(
            id,
            reserved,
            &TokenUsage {
                input_tokens: 60,
                output_tokens: 40,
                ..Default::default()
            },
        );
        let after_settle = scheduler.get_usage(id).unwrap();
        assert_eq!(
            after_settle.total_tokens, 100,
            "settle should correct down to actual"
        );
        assert_eq!(after_settle.input_tokens, 60);
        assert_eq!(after_settle.output_tokens, 40);
        assert_eq!(after_settle.llm_calls, 1);
    }

    /// Regression test for #3736 — settle_reservation with empty usage (e.g.
    /// the agent loop failed before the LLM call) must release the entire
    /// pre-charged amount, not leave it permanently consumed.
    #[test]
    fn test_settle_empty_usage_releases_full_reservation() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        // Limit 100k → burst cap 20k, so 500 reserved comfortably fits.
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: Some(100_000),
            max_tool_calls_per_minute: 0,
            ..Default::default()
        };
        scheduler.register(id, quota);

        let reserved = scheduler.check_quota_and_reserve(id, 500).unwrap();
        scheduler.settle_reservation(id, reserved, &TokenUsage::default());
        let after = scheduler.get_usage(id).unwrap();
        assert_eq!(
            after.total_tokens, 0,
            "failed call should release the reservation"
        );
        // llm_calls is still incremented — the call was attempted.
        assert_eq!(after.llm_calls, 1);
    }
}
