//! Jittered exponential backoff for LLM driver retry loops.
//!
//! Implements exponential backoff with proportional jitter — the delay grows
//! exponentially with each retry attempt, and a random fraction of that delay
//! is added as jitter to spread out concurrent retry spikes from multiple sessions.
//!
//! Formula: `delay = min(base * 2^(attempt-1), max_delay) + jitter`
//! where `jitter ∈ [0, jitter_ratio * exp_delay]`.
//!
//! The random seed combines `SystemTime::now().subsec_nanos()` with a
//! process-global monotonic counter so that seeds remain diverse even when the
//! OS clock has coarse granularity (e.g. 15 ms on Windows).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

static TEST_ZERO_BACKOFF: AtomicBool = AtomicBool::new(false);

/// Enable zero-delay backoff for integration tests. Returns a guard that
/// disables it on drop.
pub fn enable_test_zero_backoff() -> ZeroBackoffGuard {
    TEST_ZERO_BACKOFF.store(true, Ordering::Relaxed);
    ZeroBackoffGuard(())
}

pub struct ZeroBackoffGuard(());

impl Drop for ZeroBackoffGuard {
    fn drop(&mut self) {
        TEST_ZERO_BACKOFF.store(false, Ordering::Relaxed);
    }
}

/// Process-global counter that advances on every `jittered_backoff` call.
/// Combined with wall-clock nanoseconds it ensures seed diversity even when
/// multiple concurrent retry loops fire within the same clock tick.
static JITTER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Compute a jittered exponential backoff delay with an optional server-supplied
/// minimum floor (e.g. a `Retry-After` header value).
///
/// # Arguments
/// * `attempt` — 1-based retry attempt number (attempt 1 → `base_delay`, attempt 2 → `2 * base_delay`, …).
/// * `base_delay` — Base delay for the first attempt.
/// * `max_delay` — Upper cap on the exponential component.
/// * `jitter_ratio` — Fraction of the computed delay added as random jitter;
///   `0.5` means jitter is uniform in `[0, 0.5 * exp_delay]`.
/// * `floor` — Minimum deterministic delay before jitter is applied.  Pass
///   `Duration::ZERO` when there is no server-supplied floor.  When a
///   `Retry-After` header is present the server's value should be passed here
///   so that it is always honoured regardless of jitter.
///
/// # Returns
/// `max(exp_delay, floor) + jitter`, where `floor` is capped at 300 s.
///
/// # Example
/// ```
/// use std::time::Duration;
/// use librefang_llm_drivers::backoff::jittered_backoff;
///
/// let delay = jittered_backoff(1, Duration::from_secs(2), Duration::from_secs(60), 0.5, Duration::ZERO);
/// assert!(delay >= Duration::from_secs(2));
/// assert!(delay <= Duration::from_secs(3)); // base + up to 50 % jitter
/// ```
pub fn jittered_backoff(
    attempt: u32,
    base_delay: Duration,
    max_delay: Duration,
    jitter_ratio: f64,
    floor: Duration,
) -> Duration {
    // Compute the exponential component entirely in f64 to avoid
    // Duration::mul_f64 panicking when base * 2^exp overflows Duration (which
    // happens at exp ~34 for a 2 s base, well below the old cap of 62).
    // We clamp the f64 result against max_delay_secs before constructing a
    // Duration, so the Duration is always in range.
    if TEST_ZERO_BACKOFF.load(Ordering::Relaxed) {
        return floor.min(Duration::from_secs(300));
    }

    let exp = attempt.saturating_sub(1) as i32;
    let base_secs = base_delay.as_secs_f64();
    let max_secs = max_delay.as_secs_f64();
    // 2_f64.powi saturates to +inf for exp >= 1024; multiplying by base_secs
    // (a finite positive f64) keeps the product finite or +inf.  Either way
    // the f64::min(max_secs) clamp produces a finite result in [0, max_secs].
    let exp_secs = (base_secs * 2_f64.powi(exp)).min(max_secs);
    let exp_delay = Duration::from_secs_f64(exp_secs);

    // Apply the server-supplied floor (capped at 300 s to match the 300 s
    // guard previously applied at each call site).  The comparison is against
    // the deterministic exp_delay so that Retry-After is always honoured
    // regardless of where random jitter would land.
    let floor_capped = floor.min(Duration::from_secs(300));
    let base_for_jitter = exp_delay.max(floor_capped);

    // Build a 64-bit seed from wall-clock nanoseconds XOR a Weyl-sequence
    // counter. The Weyl increment (Knuth's magic constant) maximises bit
    // dispersion between consecutive calls.
    let tick = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let seed = nanos ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15);

    // One step of an LCG (Knuth) to mix the seed, then take the upper 32 bits
    // as a uniform sample in [0, 1).
    let mixed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    // `>> 32` extracts the high 32 bits (range [0, 2^32 - 1]).
    // Dividing by 2^32 (not u32::MAX) maps that range to [0, 1).
    let r = (mixed >> 32) as f64 / (1u64 << 32) as f64;

    // `jitter_ratio` is caller-supplied. `f64::clamp` panics if the value
    // being clamped is NaN, and `NaN * r` / `inf * r` is NaN — so a non-finite
    // ratio would panic the retry hot path. Coerce any non-finite ratio to
    // 0.0 (no jitter) before the clamp.
    let safe_ratio = if jitter_ratio.is_finite() {
        jitter_ratio
    } else {
        0.0
    };
    let jitter = base_for_jitter.mul_f64((safe_ratio * r).clamp(0.0, 1.0));
    base_for_jitter + jitter
}

/// Standard LLM-driver retry delay using 2 s base, 60 s cap, 50 % jitter.
///
/// Pass a `Retry-After` `Duration` as `floor` when the server supplies one;
/// pass `Duration::ZERO` otherwise.
pub fn standard_retry_delay(attempt: u32, floor: Duration) -> Duration {
    jittered_backoff(
        attempt,
        Duration::from_secs(2),
        Duration::from_secs(60),
        0.5,
        floor,
    )
}

/// Variant for tool-use failures with faster 1.5 s base.
pub fn tool_use_retry_delay(attempt: u32) -> Duration {
    jittered_backoff(
        attempt,
        Duration::from_millis(1500),
        Duration::from_secs(60),
        0.5,
        Duration::ZERO,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt1_returns_at_least_base() {
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(1, base, max, 0.5, Duration::ZERO);
        assert!(d >= base, "delay should be ≥ base: {d:?}");
        assert!(
            d <= base + base.mul_f64(0.5),
            "jitter must stay within ratio: {d:?}"
        );
    }

    #[test]
    fn respects_max_delay_cap() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(15);
        // attempt 5: 10 * 2^4 = 160s, but should be capped to 15s before jitter
        let d = jittered_backoff(5, base, max, 0.5, Duration::ZERO);
        // upper bound: max + 50 % jitter on max
        assert!(
            d <= max + max.mul_f64(0.5),
            "delay exceeds max + jitter: {d:?}"
        );
    }

    #[test]
    fn successive_calls_are_not_identical() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        // Draw 20 samples; at least two should differ (probability of collision ≈ 0).
        let samples: Vec<_> = (0..20)
            .map(|_| jittered_backoff(1, base, max, 0.5, Duration::ZERO))
            .collect();
        let all_same = samples.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "all 20 samples are identical — jitter is broken");
    }

    #[test]
    fn zero_jitter_ratio_equals_pure_exp() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(120);
        let d = jittered_backoff(3, base, max, 0.0, Duration::ZERO);
        // attempt 3: base * 2^2 = 4s, no jitter
        assert_eq!(d, Duration::from_secs(4));
    }

    #[test]
    fn attempt_0_treated_as_base() {
        // attempt=0 is normalized to attempt=1 via saturating_sub(1)
        let base = Duration::from_secs(5);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(0, base, max, 0.5, Duration::ZERO);
        // should behave like attempt=1: base + up to 50% jitter
        assert!(d >= base);
        assert!(d <= base + base.mul_f64(0.5));
    }

    #[test]
    fn large_attempt_does_not_panic() {
        // For large attempt values the f64 product overflows Duration's internal
        // u64 nanosecond counter if computed naively.  The f64-space clamping
        // must keep the result in [0, max_delay] without panicking.
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(30);
        // attempt=35 would overflow Duration with the old mul_f64 approach
        // (2s * 2^34 = 3.4e10 s > u64::MAX nanos / 1e9 ≈ 1.8e10 s).
        for attempt in [35u32, 50, 100, u32::MAX] {
            let d = jittered_backoff(attempt, base, max, 0.5, Duration::ZERO);
            assert!(
                d <= max + max.mul_f64(0.5),
                "attempt={attempt}: delay {d:?} exceeds max + jitter"
            );
        }
    }

    #[test]
    fn jitter_ratio_over_1_clamped_to_1() {
        // jitter_ratio > 1.0 is clamped to 1.0, so jitter ≤ base_for_jitter
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(2, base, max, 3.0, Duration::ZERO);
        // attempt=2: exp_delay = 4s; jitter capped so total ≤ 4s + 4s = 8s
        assert!(d <= max.mul_f64(2.0));
    }

    #[test]
    fn floor_is_respected_deterministically() {
        // floor > exp_delay: the result must be >= floor regardless of jitter.
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let floor = Duration::from_secs(10);
        // attempt=1: exp_delay = 2s < floor = 10s
        for _ in 0..20 {
            let d = jittered_backoff(1, base, max, 0.5, floor);
            assert!(
                d >= floor,
                "delay {d:?} is below the server-supplied floor {floor:?}"
            );
        }
    }

    #[test]
    fn floor_capped_at_300s() {
        // A pathological Retry-After of 9999s must be capped at 300s.
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let floor = Duration::from_secs(9999);
        let d = jittered_backoff(1, base, max, 0.5, floor);
        // floor is capped at 300s; jitter on top is at most 50% of 300s = 150s
        assert!(d <= Duration::from_secs(300) + Duration::from_secs(150));
    }

    // -- #5136: non-finite caller-supplied jitter_ratio must not panic -----

    #[test]
    fn nan_jitter_ratio_does_not_panic() {
        // `jitter_ratio` is caller-supplied. `(NaN * r).clamp(0.0, 1.0)`
        // panics inside f64::clamp (clamp panics when the value is NaN).
        // A non-finite ratio must be coerced to 0.0 (no jitter), leaving
        // the result equal to the deterministic exponential delay.
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(2, base, max, f64::NAN, Duration::ZERO);
        // attempt=2 → exp_delay = base * 2^1 = 4s, no jitter added.
        assert_eq!(d, Duration::from_secs(4));
    }

    #[test]
    fn infinite_jitter_ratio_does_not_panic() {
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(1, base, max, f64::INFINITY, Duration::ZERO);
        // attempt=1 → exp_delay = base * 2^0 = 2s, no jitter added.
        assert_eq!(d, Duration::from_secs(2));
    }
}
