//! Jittered exponential backoff for LLM driver retry loops.
//!
//! Implements decorrelated jitter to prevent thundering-herd retry spikes when
//! multiple agent sessions hit the same rate-limited provider concurrently.
//! The algorithm mirrors the Python reference in `hermes-agent/agent/retry_utils.py`.
//!
//! Formula: `delay = min(base * 2^(attempt-1), max_delay) + jitter`
//! where `jitter ∈ [0, jitter_ratio * delay]`.
//!
//! The random seed combines `SystemTime::now().subsec_nanos()` with a
//! process-global monotonic counter so that seeds remain diverse even when the
//! OS clock has coarse granularity (e.g. 15 ms on Windows).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Process-global counter that advances on every `jittered_backoff` call.
/// Combined with wall-clock nanoseconds it ensures seed diversity even when
/// multiple concurrent retry loops fire within the same clock tick.
static JITTER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Compute a jittered exponential backoff delay.
///
/// # Arguments
/// * `attempt` — 1-based retry attempt number (attempt 1 → `base_delay`, attempt 2 → `2 * base_delay`, …).
/// * `base_delay` — Base delay for the first attempt.
/// * `max_delay` — Upper cap on the exponential component.
/// * `jitter_ratio` — Fraction of the computed delay added as random jitter;
///   `0.5` means jitter is uniform in `[0, 0.5 * exp_delay]`.
///
/// # Returns
/// Total sleep duration: `exp_delay + jitter`.
///
/// # Example
/// ```
/// use std::time::Duration;
/// use librefang_llm_drivers::backoff::jittered_backoff;
///
/// let delay = jittered_backoff(1, Duration::from_secs(2), Duration::from_secs(60), 0.5);
/// assert!(delay >= Duration::from_secs(2));
/// assert!(delay <= Duration::from_secs(3)); // base + up to 50 % jitter
/// ```
pub fn jittered_backoff(
    attempt: u32,
    base_delay: Duration,
    max_delay: Duration,
    jitter_ratio: f64,
) -> Duration {
    // Exponential component, capped at max_delay.
    // saturating_sub(1) so attempt=0 behaves the same as attempt=1.
    let exp = attempt.saturating_sub(1) as i32;
    let exp_delay = base_delay
        .mul_f64(2_f64.powi(exp))
        .min(max_delay);

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
    let r = (mixed >> 33) as f64 / u32::MAX as f64;

    let jitter = exp_delay.mul_f64((jitter_ratio * r).clamp(0.0, 1.0));
    exp_delay + jitter
}

/// Return `true` when the caller should make another attempt.
///
/// Equivalent to `attempt < max_attempts`.
#[inline]
pub fn should_retry(attempt: u32, max_attempts: u32) -> bool {
    attempt < max_attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt1_returns_at_least_base() {
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let d = jittered_backoff(1, base, max, 0.5);
        assert!(d >= base, "delay should be ≥ base: {d:?}");
        assert!(d <= base + base.mul_f64(0.5), "jitter must stay within ratio: {d:?}");
    }

    #[test]
    fn respects_max_delay_cap() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(15);
        // attempt 5: 10 * 2^4 = 160s, but should be capped to 15s before jitter
        let d = jittered_backoff(5, base, max, 0.5);
        // upper bound: max + 50 % jitter on max
        assert!(d <= max + max.mul_f64(0.5), "delay exceeds max + jitter: {d:?}");
    }

    #[test]
    fn successive_calls_are_not_identical() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        // Draw 20 samples; at least two should differ (probability of collision ≈ 0).
        let samples: Vec<_> = (0..20)
            .map(|_| jittered_backoff(1, base, max, 0.5))
            .collect();
        let all_same = samples.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "all 20 samples are identical — jitter is broken");
    }

    #[test]
    fn should_retry_logic() {
        assert!(should_retry(0, 3));
        assert!(should_retry(2, 3));
        assert!(!should_retry(3, 3));
        assert!(!should_retry(5, 3));
    }

    #[test]
    fn zero_jitter_ratio_equals_pure_exp() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(120);
        let d = jittered_backoff(3, base, max, 0.0);
        // attempt 3: base * 2^2 = 4s, no jitter
        assert_eq!(d, Duration::from_secs(4));
    }
}
