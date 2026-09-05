use std::time::Duration;

/// Trait defining a strategy for calculating reconnection delay.
pub trait BackoffStrategy: Send + Sync + 'static {
    /// Return the delay duration for the next reconnection attempt.
    /// `attempt` is 1-indexed (1 for the first retry, etc.).
    fn next_delay(&self, attempt: u32) -> Duration;
}

/// A constant reconnection delay strategy.
#[derive(Debug, Clone)]
pub struct ConstantBackoff {
    pub delay: Duration,
}

impl ConstantBackoff {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}

impl BackoffStrategy for ConstantBackoff {
    fn next_delay(&self, _attempt: u32) -> Duration {
        self.delay
    }
}

/// A linear reconnection delay strategy.
#[derive(Debug, Clone)]
pub struct LinearBackoff {
    pub initial_delay: Duration,
    pub step: Duration,
    pub max_delay: Duration,
}

impl LinearBackoff {
    pub fn new(initial_delay: Duration, step: Duration, max_delay: Duration) -> Self {
        Self {
            initial_delay,
            step,
            max_delay,
        }
    }
}

impl BackoffStrategy for LinearBackoff {
    fn next_delay(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::from_secs(0);
        }
        // Saturate instead of panicking: `Duration * u32` and `Duration + Duration` both panic on overflow, and clamping to `max_delay` afterwards happens too late to prevent it.
        let delay = self
            .step
            .checked_mul(attempt - 1)
            .and_then(|scaled| self.initial_delay.checked_add(scaled))
            .unwrap_or(self.max_delay);
        std::cmp::min(delay, self.max_delay)
    }
}

/// An exponential reconnection delay strategy.
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    pub initial_delay: Duration,
    pub multiplier: f64,
    pub max_delay: Duration,
}

impl ExponentialBackoff {
    pub fn new(initial_delay: Duration, multiplier: f64, max_delay: Duration) -> Self {
        Self {
            initial_delay,
            multiplier,
            max_delay,
        }
    }
}

impl BackoffStrategy for ExponentialBackoff {
    fn next_delay(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::from_secs(0);
        }
        // `i32::try_from` rather than `as i32`: a wrapped exponent turns into a negative power, which would shrink the delay towards zero and busy-retry instead of backing off.
        let exponent = i32::try_from(attempt - 1).unwrap_or(i32::MAX);
        let factor = self.multiplier.powi(exponent);
        let secs = self.initial_delay.as_secs_f64() * factor;
        // Clamp in `f64` before constructing the `Duration`, not after.
        // `Duration::from_secs_f64` panics on a value that overflows `Duration` or is not finite, so a `min(delay, max_delay)` applied to the constructed `Duration` never gets the chance to bound it.
        // A caller that keeps incrementing `attempt` across consecutive failures does reach that point — with a 10 s initial delay and a multiplier of 2.0 it is attempt 62 — and the panic takes down the retry loop for the life of the process even though every delay from attempt 7 onward was already pinned at `max_delay`.
        let cap = self.max_delay.as_secs_f64();
        if !secs.is_finite() || secs >= cap {
            return self.max_delay;
        }
        // A multiplier below zero alternates the sign of `factor`, and `from_secs_f64` panics on negative seconds too.
        Duration::from_secs_f64(secs.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_backoff() {
        let strategy = ConstantBackoff::new(Duration::from_secs(5));
        assert_eq!(strategy.next_delay(1), Duration::from_secs(5));
        assert_eq!(strategy.next_delay(5), Duration::from_secs(5));
    }

    #[test]
    fn test_linear_backoff() {
        let strategy = LinearBackoff::new(
            Duration::from_secs(10),
            Duration::from_secs(5),
            Duration::from_secs(25),
        );
        assert_eq!(strategy.next_delay(0), Duration::from_secs(0));
        assert_eq!(strategy.next_delay(1), Duration::from_secs(10));
        assert_eq!(strategy.next_delay(2), Duration::from_secs(15));
        assert_eq!(strategy.next_delay(3), Duration::from_secs(20));
        assert_eq!(strategy.next_delay(4), Duration::from_secs(25));
        assert_eq!(strategy.next_delay(5), Duration::from_secs(25));
    }

    #[test]
    fn test_exponential_backoff() {
        let strategy =
            ExponentialBackoff::new(Duration::from_secs(2), 2.0, Duration::from_secs(10));
        assert_eq!(strategy.next_delay(0), Duration::from_secs(0));
        assert_eq!(strategy.next_delay(1), Duration::from_secs(2));
        assert_eq!(strategy.next_delay(2), Duration::from_secs(4));
        assert_eq!(strategy.next_delay(3), Duration::from_secs(8));
        assert_eq!(strategy.next_delay(4), Duration::from_secs(10));
    }

    #[test]
    fn exponential_backoff_saturates_at_max_delay_instead_of_panicking() {
        // The desktop tray's parameters, and the attempt count it reaches after hours of consecutive reconnect failures.
        // `10 * 2^61` seconds overflows `Duration`, which used to panic inside `Duration::from_secs_f64` before the `min(_, max_delay)` clamp could bound it.
        let strategy =
            ExponentialBackoff::new(Duration::from_secs(10), 2.0, Duration::from_secs(300));
        assert_eq!(strategy.next_delay(61), Duration::from_secs(300));
        assert_eq!(strategy.next_delay(62), Duration::from_secs(300));
        assert_eq!(strategy.next_delay(u32::MAX), Duration::from_secs(300));
    }

    #[test]
    fn exponential_backoff_survives_non_finite_and_negative_growth() {
        // `multiplier.powi(...)` reaches `inf` on its own well before the multiplication overflows, and `inf` panics the same constructor.
        let steep = ExponentialBackoff::new(Duration::from_secs(1), 10.0, Duration::from_secs(60));
        assert_eq!(steep.next_delay(400), Duration::from_secs(60));
        // A negative multiplier yields negative seconds on alternating attempts, which `Duration::from_secs_f64` also rejects.
        let negative =
            ExponentialBackoff::new(Duration::from_secs(5), -2.0, Duration::from_secs(60));
        assert_eq!(negative.next_delay(2), Duration::from_secs(0));
    }

    #[test]
    fn linear_backoff_saturates_at_max_delay_instead_of_panicking() {
        // `step * (attempt - 1)` used the panicking `Mul<u32>` impl, so a large step overflowed before the `max_delay` clamp applied.
        let strategy = LinearBackoff::new(
            Duration::from_secs(10),
            Duration::from_secs(u64::MAX / 2),
            Duration::from_secs(300),
        );
        assert_eq!(strategy.next_delay(1), Duration::from_secs(10));
        assert_eq!(strategy.next_delay(5), Duration::from_secs(300));
        assert_eq!(strategy.next_delay(u32::MAX), Duration::from_secs(300));
    }
}
