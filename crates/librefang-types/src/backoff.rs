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
        let delay = self.initial_delay + self.step * (attempt - 1);
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
        let factor = self.multiplier.powi((attempt - 1) as i32);
        let secs = self.initial_delay.as_secs_f64() * factor;
        let delay = Duration::from_secs_f64(secs);
        std::cmp::min(delay, self.max_delay)
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
}
