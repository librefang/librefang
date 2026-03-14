//! Per-user, per-channel sliding-window rate limiter.
//!
//! Uses a simple timestamp-based approach: each bucket stores recent message
//! timestamps and evicts entries older than the 1-minute window on every check.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sliding-window rate limiter for channel messages.
///
/// Key: `"{channel_type}:{platform_id}"`, Value: timestamps of recent messages.
#[derive(Debug, Clone, Default)]
pub struct ChannelRateLimiter {
    /// Recent message timestamps per user key.
    buckets: Arc<DashMap<String, Vec<Instant>>>,
}

impl ChannelRateLimiter {
    /// Check if a user is rate-limited. Returns `Ok(())` if allowed, `Err(msg)` if blocked.
    ///
    /// `max_per_minute`: 0 means unlimited.
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

        let mut entry = self.buckets.entry(key).or_default();
        // Evict timestamps older than 1 minute
        entry.retain(|&ts| now.duration_since(ts) < window);

        if entry.len() >= max_per_minute as usize {
            return Err(format!(
                "Rate limit exceeded ({max_per_minute} messages/minute). Please wait."
            ));
        }

        entry.push(now);
        Ok(())
    }
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
            .buckets
            .entry(key.clone())
            .or_default()
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
            .buckets
            .entry(key.clone())
            .or_default()
            .extend(vec![old; 2]);

        // Insert 2 recent timestamps (within window)
        let recent = Instant::now();
        limiter
            .buckets
            .entry(key.clone())
            .or_default()
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
            .buckets
            .entry(key.clone())
            .or_default()
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
        assert!(limiter.buckets.is_empty());
    }
}
