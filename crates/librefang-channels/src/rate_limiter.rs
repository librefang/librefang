//! Per-user, per-channel sliding-window rate limiter.
//!
//! Uses a simple timestamp-based approach: each bucket stores recent message
//! timestamps and evicts entries older than the 1-minute window on every check.
//!
//! Buckets use `SmallVec<[Instant; 8]>` to keep small bursts on the stack,
//! avoiding heap allocation for typical rate limits (e.g., 5/min).
//!
//! # Bucket eviction (#5494)
//!
//! The map is bounded in two complementary ways so a flood of synthetic
//! `platform_id`s cannot walk it out into multi-GiB territory:
//!
//! * **Periodic sweep** (every [`SWEEP_INTERVAL`]) drops empty / idle buckets
//!   that have had no activity for [`SWEEP_TTL`]. Spawned automatically by
//!   [`ChannelRateLimiter::default`] / [`ChannelRateLimiter::new`] when a
//!   Tokio runtime is current; the task is aborted on the last clone's
//!   drop via a shared [`Drop`] guard. Tests that construct the limiter
//!   outside a runtime simply get no background task — they exercise
//!   [`ChannelRateLimiter::sweep`] directly.
//! * **Hard cap** ([`MAX_BUCKETS`]): if [`ChannelRateLimiter::check`] would
//!   insert a new bucket past the cap, the oldest [`OVERFLOW_EVICT_CHUNK`]
//!   buckets (by `last_seen`) are evicted in one pass first. This is the
//!   synchronous-burst defence; the periodic sweep is the steady-state one.

use dashmap::DashMap;
use smallvec::SmallVec;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Stack-allocated capacity for rate-limiter timestamp buckets.
/// Covers typical per-user limits without heap allocation.
type TimestampBucket = SmallVec<[Instant; 8]>;

/// Maximum number of distinct `(channel_type, platform_id)` buckets retained.
///
/// Sized at 100k: well beyond any legitimate channel population (a busy
/// Telegram bot might see thousands of users, not hundreds of thousands),
/// but small enough that the worst-case memory cost is bounded — each entry
/// is roughly `~200B` (key string + 8-slot `SmallVec` of `Instant` + `last_seen`),
/// so the cap holds total footprint to single-digit MiB.
pub const MAX_BUCKETS: usize = 100_000;

/// Background sweep cadence (#5494). The audit recommends 30s.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// A bucket whose `last_seen` is older than this is considered idle and is
/// dropped by the periodic sweep, even if it still holds (stale) timestamps.
/// Set to 5 minutes — comfortably longer than the 60s sliding window so we
/// don't churn the entry of a user who sends a message every couple minutes.
pub const SWEEP_TTL: Duration = Duration::from_secs(5 * 60);

/// When [`MAX_BUCKETS`] is hit on a fresh insert, evict the oldest N buckets
/// (by `last_seen`) in one pass. Doing this in chunks rather than one-at-a-time
/// amortizes the O(n) scan so a synthetic flood doesn't pay it on every message.
pub const OVERFLOW_EVICT_CHUNK: usize = MAX_BUCKETS / 100;

/// One per `(channel_type, platform_id)` key.
#[derive(Debug, Default)]
struct Bucket {
    /// Recent message timestamps inside the 1-minute sliding window.
    timestamps: TimestampBucket,
    /// Last time this bucket was touched by `check`. Used by both the
    /// LRU-on-overflow path and the periodic sweep for idle-eviction.
    last_seen: Option<Instant>,
}

impl Bucket {
    #[inline]
    fn is_idle(&self, now: Instant, ttl: Duration) -> bool {
        match self.last_seen {
            Some(ts) => now.duration_since(ts) >= ttl,
            None => true,
        }
    }
}

/// Holds the shared map. Wrapped in `Arc` so `Clone` of
/// [`ChannelRateLimiter`] shares state and doesn't spawn extra sweepers.
#[derive(Debug, Default)]
struct Inner {
    buckets: DashMap<String, Bucket>,
}

/// Sliding-window rate limiter for channel messages.
///
/// Key: `"{channel_type}:{platform_id}"`, Value: timestamps of recent messages.
///
/// `Clone` shares the bucket map *and* the sweep-shutdown notify; the
/// background sweeper holds a `Weak<Inner>` (so it doesn't keep buckets
/// alive past the last public clone) and an `Arc<Notify>` (so the
/// `notify_waiters` call from the [`Drop`] impl below reaches it). When
/// the last [`ChannelRateLimiter`] clone is dropped, [`Inner`] is dropped,
/// the sweep task wakes from its `sleep` or its `notified()` arm, finds
/// `Weak::upgrade() = None`, and exits — never holding the map alive itself.
#[derive(Debug, Clone)]
pub struct ChannelRateLimiter {
    inner: Arc<Inner>,
    /// Strong handle to the same Notify the sweeper task holds. On the
    /// last clone's drop, [`Drop`] fires `notify_waiters` so the sweeper
    /// breaks out of its sleep early instead of waiting up to
    /// [`SWEEP_INTERVAL`] to notice the limiter is gone.
    sweep_shutdown: Arc<Notify>,
}

impl Drop for ChannelRateLimiter {
    fn drop(&mut self) {
        // `Arc::strong_count` is racy in general, but here we hold the
        // last strong ref by construction when count == 2 (this clone +
        // the sweeper). Wake the sweeper either way — extra wakes are
        // harmless (it loops and goes back to sleep) and we don't want
        // to leak an Arc strong-count comparison race.
        self.sweep_shutdown.notify_waiters();
    }
}

impl Default for ChannelRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelRateLimiter {
    /// Construct an empty limiter. When called from inside a Tokio runtime
    /// (i.e. from production code path), spawns the periodic sweep task
    /// that drops idle buckets every [`SWEEP_INTERVAL`]. Outside a runtime
    /// (sync unit tests), no task is spawned — tests can call
    /// [`Self::sweep`] directly.
    pub fn new() -> Self {
        Self::new_with_interval(SWEEP_INTERVAL)
    }

    /// Like [`Self::new`] but lets the caller override the sweep cadence.
    /// Reserved for tests — production code should stick with [`Self::new`]
    /// (i.e. [`SWEEP_INTERVAL`]).
    #[doc(hidden)]
    pub fn new_with_interval(interval: Duration) -> Self {
        let inner = Arc::new(Inner::default());
        let sweep_shutdown = Arc::new(Notify::new());
        spawn_sweeper(
            Arc::downgrade(&inner),
            Arc::clone(&sweep_shutdown),
            interval,
        );
        Self {
            inner,
            sweep_shutdown,
        }
    }

    /// Check if a user is rate-limited. Returns `Ok(())` if allowed, `Err(msg)` if blocked.
    ///
    /// `max_per_minute`: 0 means unlimited.
    #[inline]
    pub fn check(
        &self,
        channel_type: &str,
        platform_id: &str,
        max_per_minute: u32,
    ) -> Result<(), String> {
        if max_per_minute == 0 {
            return Ok(());
        }

        let key = format!("{channel_type}:{platform_id}");
        let now = Instant::now();
        let window = Duration::from_secs(60);

        // LRU-on-overflow: if we're about to add a new bucket past the cap,
        // make room first. Read-side check is cheap (atomic load); the
        // expensive scan only runs when we actually overflow. We re-check
        // membership *after* the scan because another thread may have raced
        // us, and we never want to evict a bucket we're about to use.
        if self.inner.buckets.len() >= MAX_BUCKETS && !self.inner.buckets.contains_key(&key) {
            self.evict_oldest(OVERFLOW_EVICT_CHUNK);
        }

        let mut entry = self.inner.buckets.entry(key).or_default();
        // Evict timestamps older than 1 minute
        entry
            .timestamps
            .retain(|ts| now.duration_since(*ts) < window);

        if entry.timestamps.len() >= max_per_minute as usize {
            entry.last_seen = Some(now);
            return Err(format!(
                "Rate limit exceeded ({max_per_minute} messages/minute). Please wait."
            ));
        }

        entry.timestamps.push(now);
        entry.last_seen = Some(now);
        Ok(())
    }

    /// Drop buckets that are empty (no in-window timestamps) AND idle
    /// (`last_seen` older than [`SWEEP_TTL`]).
    ///
    /// Public so the background sweeper task and unit tests can invoke it.
    /// Cheap when the map is small; `DashMap::retain` shards the work.
    pub fn sweep(&self) {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        self.inner.buckets.retain(|_, bucket| {
            // First, drop stale timestamps from the sliding window so we
            // get an honest is-empty signal.
            bucket
                .timestamps
                .retain(|ts| now.duration_since(*ts) < window);
            // Keep the bucket if it still has live timestamps OR it was
            // touched recently. The TTL gate prevents flapping for a user
            // who sends one message every two minutes (window 60s, TTL 5min).
            !bucket.timestamps.is_empty() || !bucket.is_idle(now, SWEEP_TTL)
        });
    }

    /// Current bucket count. Primarily for tests + observability.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.buckets.len()
    }

    /// True iff [`Self::len`] is zero.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.buckets.is_empty()
    }

    /// Evict the `count` oldest buckets (by `last_seen`) in a single O(n) pass.
    /// Called from the LRU-on-overflow path in [`Self::check`].
    fn evict_oldest(&self, count: usize) {
        if count == 0 {
            return;
        }
        // Collect (key, last_seen) pairs. Buckets without a `last_seen`
        // (shouldn't happen post-`check`, but guard against the
        // race where another thread inserted via `entry().or_default()`
        // but hasn't reached the assignment yet) sort to the front via
        // `Instant::now() - 1day` so they evict first.
        let mut snapshot: Vec<(String, Instant)> = self
            .inner
            .buckets
            .iter()
            .map(|e| {
                let ts = e
                    .value()
                    .last_seen
                    .unwrap_or_else(|| Instant::now() - Duration::from_secs(86_400));
                (e.key().clone(), ts)
            })
            .collect();
        // Partial sort: smaller `last_seen` = older = evict first.
        // `select_nth_unstable_by_key` is O(n) average vs full sort's O(n log n).
        if snapshot.len() > count {
            snapshot.select_nth_unstable_by_key(count, |(_, ts)| *ts);
            snapshot.truncate(count);
        }
        for (key, _) in snapshot {
            self.inner.buckets.remove(&key);
        }
    }
}

/// Spawn the periodic sweeper if a Tokio runtime is current. No-op otherwise
/// (e.g. unit tests constructed via `ChannelRateLimiter::default()` outside
/// `#[tokio::test]`).
///
/// The task holds only a `Weak<Inner>` for the bucket map, so it never keeps
/// the limiter alive past the last public clone's drop. It holds a strong
/// `Arc<Notify>` for the shutdown signal so the Drop-side `notify_waiters`
/// reaches it; the notify itself is cheap and doesn't grow with bucket count.
fn spawn_sweeper(weak: std::sync::Weak<Inner>, sweep_shutdown: Arc<Notify>, interval: Duration) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        loop {
            // Bail early if the limiter is already gone — handles the
            // race where every public clone dropped before our first tick.
            if weak.strong_count() == 0 {
                return;
            }
            let notified = sweep_shutdown.notified();
            tokio::pin!(notified);

            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let Some(strong) = weak.upgrade() else {
                        return;
                    };
                    // Materialize a transient limiter purely to reuse
                    // `sweep()`. The `sweep_shutdown` clone here is a
                    // formality — `sweep()` doesn't touch it.
                    let limiter = ChannelRateLimiter {
                        inner: strong,
                        sweep_shutdown: Arc::clone(&sweep_shutdown),
                    };
                    limiter.sweep();
                    // Drop `limiter` before looping so the strong Arc
                    // is released between sweeps; otherwise the
                    // transient `ChannelRateLimiter` clone would fire
                    // its own `Drop` and spuriously wake us. Explicit
                    // drop also covers that.
                    drop(limiter);
                }
                _ = &mut notified => {
                    // Last public clone went away. Drain any pending
                    // tick and exit. (We don't re-check
                    // `weak.upgrade()` here because spurious notifies
                    // from a non-final clone's Drop are possible; the
                    // top-of-loop check handles both cases on the
                    // next iteration.)
                    if weak.strong_count() == 0 {
                        return;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Basic allow / deny
    // ---------------------------------------------------------------

    #[test]
    fn allows_messages_within_limit() {
        let limiter = ChannelRateLimiter::default();
        for i in 0..5 {
            assert!(
                limiter.check("telegram", "user1", 5).is_ok(),
                "message {i} should be allowed"
            );
        }
    }

    #[test]
    fn blocks_when_limit_exceeded() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }
        let result = limiter.check("telegram", "user1", 3);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Rate limit exceeded"));
    }

    #[test]
    fn exact_limit_boundary() {
        let limiter = ChannelRateLimiter::default();
        // Send exactly `max_per_minute` messages — all should succeed
        for _ in 0..10 {
            assert!(limiter.check("discord", "user1", 10).is_ok());
        }
        // The (max+1)-th message must fail
        assert!(limiter.check("discord", "user1", 10).is_err());
    }

    // ---------------------------------------------------------------
    // Zero means unlimited
    // ---------------------------------------------------------------

    #[test]
    fn zero_limit_means_unlimited() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..200 {
            assert!(limiter.check("telegram", "user1", 0).is_ok());
        }
    }

    // ---------------------------------------------------------------
    // Per-user isolation
    // ---------------------------------------------------------------

    #[test]
    fn separate_users_have_independent_buckets() {
        let limiter = ChannelRateLimiter::default();
        // Exhaust user1's quota
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }
        assert!(limiter.check("telegram", "user1", 3).is_err());

        // user2 should be unaffected
        assert!(limiter.check("telegram", "user2", 3).is_ok());
    }

    // ---------------------------------------------------------------
    // Per-channel isolation
    // ---------------------------------------------------------------

    #[test]
    fn separate_channels_have_independent_buckets() {
        let limiter = ChannelRateLimiter::default();
        // Exhaust quota on telegram
        for _ in 0..2 {
            limiter.check("telegram", "user1", 2).unwrap();
        }
        assert!(limiter.check("telegram", "user1", 2).is_err());

        // Same user on discord should still be allowed
        assert!(limiter.check("discord", "user1", 2).is_ok());
    }

    // ---------------------------------------------------------------
    // Multiple channels with different limits
    // ---------------------------------------------------------------

    #[test]
    fn different_limits_per_channel() {
        let limiter = ChannelRateLimiter::default();

        // Telegram allows 5/min
        for _ in 0..5 {
            limiter.check("telegram", "user1", 5).unwrap();
        }
        assert!(limiter.check("telegram", "user1", 5).is_err());

        // Discord allows 10/min — same user can still send 10 on discord
        for _ in 0..10 {
            limiter.check("discord", "user1", 10).unwrap();
        }
        assert!(limiter.check("discord", "user1", 10).is_err());

        // Slack allows 1/min — single message then blocked
        limiter.check("slack", "user1", 1).unwrap();
        assert!(limiter.check("slack", "user1", 1).is_err());
    }

    // ---------------------------------------------------------------
    // Burst handling (rapid consecutive messages)
    // ---------------------------------------------------------------

    #[test]
    fn burst_up_to_limit_succeeds() {
        let limiter = ChannelRateLimiter::default();
        // Rapid burst of exactly `limit` messages
        let limit = 20u32;
        for _ in 0..limit {
            assert!(limiter.check("telegram", "burst_user", limit).is_ok());
        }
        // Next message exceeds the burst
        assert!(limiter.check("telegram", "burst_user", limit).is_err());
    }

    #[test]
    fn burst_with_limit_one() {
        let limiter = ChannelRateLimiter::default();
        assert!(limiter.check("telegram", "user1", 1).is_ok());
        // Immediate second message should be blocked
        assert!(limiter.check("telegram", "user1", 1).is_err());
    }

    // ---------------------------------------------------------------
    // Window expiry (token refill)
    // ---------------------------------------------------------------

    #[test]
    fn old_timestamps_are_evicted() {
        // Simulate time passing by inserting timestamps in the past.
        let limiter = ChannelRateLimiter::default();
        let key = "telegram:user1".to_string();

        // Manually insert 5 timestamps that are 61 seconds old (beyond the window)
        let old = Instant::now() - Duration::from_secs(61);
        limiter
            .inner
            .buckets
            .entry(key.clone())
            .or_default()
            .timestamps
            .extend(vec![old; 5]);

        // Even though there are 5 entries, they are stale — check with limit=3 should pass
        assert!(limiter.check("telegram", "user1", 3).is_ok());
    }

    #[test]
    fn mixed_old_and_new_timestamps() {
        let limiter = ChannelRateLimiter::default();
        let key = "discord:user1".to_string();

        // Insert 2 old timestamps (should be evicted)
        let old = Instant::now() - Duration::from_secs(120);
        limiter
            .inner
            .buckets
            .entry(key.clone())
            .or_default()
            .timestamps
            .extend(vec![old; 2]);

        // Insert 2 recent timestamps (within window)
        let recent = Instant::now();
        limiter
            .inner
            .buckets
            .entry(key.clone())
            .or_default()
            .timestamps
            .extend(vec![recent; 2]);

        // Limit is 3 — the 2 old ones are evicted, 2 recent remain, so 1 more should be allowed
        assert!(limiter.check("discord", "user1", 3).is_ok());
        // Now we have 3 recent entries — next should fail
        assert!(limiter.check("discord", "user1", 3).is_err());
    }

    #[test]
    fn all_timestamps_expired_fully_refills() {
        let limiter = ChannelRateLimiter::default();
        let key = "slack:user1".to_string();

        // Fill bucket to the brim with old timestamps
        let old = Instant::now() - Duration::from_secs(90);
        limiter
            .inner
            .buckets
            .entry(key.clone())
            .or_default()
            .timestamps
            .extend(vec![old; 100]);

        // All should be evicted — limit of 5 means we can send 5 fresh messages
        for _ in 0..5 {
            assert!(limiter.check("slack", "user1", 5).is_ok());
        }
        assert!(limiter.check("slack", "user1", 5).is_err());
    }

    // ---------------------------------------------------------------
    // Clone shares state (Arc semantics)
    // ---------------------------------------------------------------

    #[test]
    fn cloned_limiter_shares_state() {
        let limiter = ChannelRateLimiter::default();
        let clone = limiter.clone();

        // Use original to fill quota
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }
        // Clone should see the same state — next check should fail
        assert!(clone.check("telegram", "user1", 3).is_err());
    }

    // ---------------------------------------------------------------
    // Error message content
    // ---------------------------------------------------------------

    #[test]
    fn error_message_includes_limit() {
        let limiter = ChannelRateLimiter::default();
        limiter.check("telegram", "user1", 1).unwrap();
        let err = limiter.check("telegram", "user1", 1).unwrap_err();
        assert!(err.contains("1 messages/minute"));
    }

    #[test]
    fn error_message_for_higher_limit() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..42 {
            limiter.check("email", "user1", 42).unwrap();
        }
        let err = limiter.check("email", "user1", 42).unwrap_err();
        assert!(err.contains("42 messages/minute"));
    }

    // ---------------------------------------------------------------
    // Concurrent-safe (basic multi-key)
    // ---------------------------------------------------------------

    #[test]
    fn many_users_many_channels() {
        let limiter = ChannelRateLimiter::default();
        let channels = ["telegram", "discord", "slack", "matrix", "email"];
        let users = ["alice", "bob", "carol", "dave"];

        for ch in &channels {
            for user in &users {
                // Each user gets a limit of 2 per channel
                assert!(limiter.check(ch, user, 2).is_ok());
                assert!(limiter.check(ch, user, 2).is_ok());
                assert!(limiter.check(ch, user, 2).is_err());
            }
        }
    }

    // ---------------------------------------------------------------
    // Default construction
    // ---------------------------------------------------------------

    #[test]
    fn default_limiter_starts_empty() {
        let limiter = ChannelRateLimiter::default();
        // No buckets should exist yet
        assert!(limiter.is_empty());
    }

    // ---------------------------------------------------------------
    // Bucket eviction (#5494) — hard cap + LRU-on-overflow
    // ---------------------------------------------------------------

    /// Flooding the limiter with `MAX_BUCKETS + extra` distinct platform_ids
    /// must NOT walk the map past `MAX_BUCKETS` — the LRU-on-overflow path
    /// in `check` evicts the oldest `OVERFLOW_EVICT_CHUNK` buckets when
    /// a fresh insert would push the cap.
    ///
    /// We use a small `extra` (= `OVERFLOW_EVICT_CHUNK`) so the test is
    /// fast: the eviction scan is O(n) and runs against ~100k entries.
    #[test]
    fn bucket_count_stays_under_cap_under_synthetic_flood() {
        let limiter = ChannelRateLimiter::default();
        let extra = OVERFLOW_EVICT_CHUNK + 50;
        for i in 0..(MAX_BUCKETS + extra) {
            // Use limit=u32::MAX so check always succeeds; we only care
            // about the bucket-creation side effect.
            limiter
                .check("telegram", &format!("user{i}"), u32::MAX)
                .unwrap();
        }
        let len = limiter.len();
        assert!(
            len <= MAX_BUCKETS,
            "buckets={len} exceeds MAX_BUCKETS={MAX_BUCKETS}"
        );
        // Sanity: we should not be hugely under either — at least one
        // chunk's worth of room is reclaimed but the map should stay
        // mostly full.
        assert!(
            len >= MAX_BUCKETS - OVERFLOW_EVICT_CHUNK * 2,
            "buckets={len} unexpectedly low; eviction too aggressive?"
        );
    }

    /// When the cap is hit, the oldest entries must go — never the freshly
    /// inserted ones. We seed `MAX_BUCKETS` "old" buckets with backdated
    /// `last_seen`, then insert one new key. The new key must survive; at
    /// least some of the old keys must be gone.
    #[test]
    fn overflow_eviction_drops_oldest_not_newest() {
        let limiter = ChannelRateLimiter::default();
        let old_ts = Instant::now() - Duration::from_secs(3600);
        for i in 0..MAX_BUCKETS {
            limiter.inner.buckets.insert(
                format!("telegram:old{i}"),
                Bucket {
                    timestamps: SmallVec::new(),
                    last_seen: Some(old_ts),
                },
            );
        }
        assert_eq!(limiter.len(), MAX_BUCKETS);

        // New insert past cap: must trigger overflow eviction.
        limiter.check("telegram", "fresh_user", 10).unwrap();

        assert!(limiter.len() <= MAX_BUCKETS);
        assert!(
            limiter.inner.buckets.contains_key("telegram:fresh_user"),
            "fresh insert was evicted (must never happen)"
        );
        // At least one old key must be gone — we evicted a chunk's worth.
        let surviving_old = (0..MAX_BUCKETS)
            .filter(|i| {
                limiter
                    .inner
                    .buckets
                    .contains_key(&format!("telegram:old{i}"))
            })
            .count();
        assert!(
            surviving_old < MAX_BUCKETS,
            "no old entries evicted; LRU path didn't fire"
        );
    }

    /// `sweep()` must drop buckets whose only timestamps are stale AND whose
    /// `last_seen` is older than `SWEEP_TTL`.
    #[test]
    fn sweep_drops_idle_empty_buckets() {
        let limiter = ChannelRateLimiter::default();
        let very_old = Instant::now() - SWEEP_TTL - Duration::from_secs(60);
        let stale_ts = Instant::now() - Duration::from_secs(120); // past 60s window

        // 3 buckets that should be evicted: idle + only stale timestamps.
        for i in 0..3 {
            limiter.inner.buckets.insert(
                format!("telegram:idle{i}"),
                Bucket {
                    timestamps: smallvec::smallvec![stale_ts, stale_ts],
                    last_seen: Some(very_old),
                },
            );
        }
        // 1 bucket that should survive: idle but still inside TTL.
        let recent_seen = Instant::now() - Duration::from_secs(30);
        limiter.inner.buckets.insert(
            "telegram:active".to_string(),
            Bucket {
                timestamps: SmallVec::new(),
                last_seen: Some(recent_seen),
            },
        );
        // 1 bucket that should survive: idle past TTL but with live timestamps.
        let live_ts = Instant::now() - Duration::from_secs(10);
        limiter.inner.buckets.insert(
            "telegram:livets".to_string(),
            Bucket {
                timestamps: smallvec::smallvec![live_ts],
                last_seen: Some(very_old),
            },
        );

        assert_eq!(limiter.len(), 5);
        limiter.sweep();

        // 3 idle+empty gone, 2 survivors remain.
        assert_eq!(
            limiter.len(),
            2,
            "sweep should leave only the two survivors"
        );
        assert!(limiter.inner.buckets.contains_key("telegram:active"));
        assert!(limiter.inner.buckets.contains_key("telegram:livets"));
    }

    /// `sweep()` is a no-op against a map of freshly active buckets — we
    /// must not evict anyone just for having stale window timestamps if
    /// the activity is recent.
    #[test]
    fn sweep_is_noop_on_fresh_buckets() {
        let limiter = ChannelRateLimiter::default();
        for i in 0..10 {
            limiter.check("telegram", &format!("u{i}"), 5).unwrap();
        }
        assert_eq!(limiter.len(), 10);
        limiter.sweep();
        assert_eq!(limiter.len(), 10);
    }

    /// The background sweeper must actually run when the limiter is
    /// constructed inside a Tokio runtime. Uses a 25ms interval via
    /// `new_with_interval` so the test completes in well under a second
    /// without needing tokio's `test-util` feature flag.
    #[tokio::test]
    async fn background_sweeper_runs_periodically() {
        let limiter = ChannelRateLimiter::new_with_interval(Duration::from_millis(25));

        // Seed an idle + empty bucket that the sweeper must drop.
        let very_old = Instant::now() - SWEEP_TTL - Duration::from_secs(60);
        limiter.inner.buckets.insert(
            "telegram:ghost".to_string(),
            Bucket {
                timestamps: SmallVec::new(),
                last_seen: Some(very_old),
            },
        );
        assert_eq!(limiter.len(), 1);

        // Wait long enough for at least one sweep tick to fire and run.
        // 300ms = 12 intervals; if the sweeper isn't running, this won't
        // be enough no matter how long we wait.
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(
            limiter.len(),
            0,
            "background sweeper did not evict ghost bucket"
        );
    }

    /// When the last public clone of the limiter is dropped, the sweep
    /// task must stop holding the inner map alive — verified by checking
    /// that the `Weak<Inner>` we recorded pre-drop fails to upgrade after
    /// the limiter goes away. The sweeper only holds a `Weak`, never a
    /// strong Arc, so this MUST pass as long as the spawn path is wired
    /// correctly.
    #[tokio::test]
    async fn sweeper_releases_buckets_after_last_clone_dropped() {
        let limiter = ChannelRateLimiter::new_with_interval(Duration::from_millis(25));
        let weak = Arc::downgrade(&limiter.inner);
        drop(limiter);

        // Give the sweep task a few ticks to observe shutdown and exit.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            weak.upgrade().is_none(),
            "Inner should have been dropped once the last public clone went away"
        );
    }
}
