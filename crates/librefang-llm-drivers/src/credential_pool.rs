//! Multi-credential pool for same-provider API key failover.
//!
//! Holds multiple API keys for a single provider and selects among available
//! (non-exhausted) credentials using one of four strategies:
//!
//! - **FillFirst** — always pick the highest-priority available key until it is
//!   exhausted, then fall back to the next. Maximises utilisation of premium keys.
//! - **RoundRobin** — cycle through available keys in order, distributing load evenly.
//! - **Random** — choose a random available key on every call.
//! - **LeastUsed** — always pick the key with the fewest `request_count` so far.
//!
//! Exhausted credentials (those that received a 429 or 402 response) are placed in
//! a cooldown period (`exhausted_ttl`, default 1 hour) and excluded from selection
//! until the period expires.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default cooldown duration after a 429 / 402 response.
pub const DEFAULT_EXHAUSTED_TTL: Duration = Duration::from_secs(60 * 60); // 1 hour

// ── Strategy ─────────────────────────────────────────────────────────────────

/// Credential selection strategy for [`CredentialPool`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PoolStrategy {
    /// Always try the highest-priority available credential first.
    FillFirst,
    /// Cycle through available credentials in priority order.
    #[default]
    RoundRobin,
    /// Choose a random available credential.
    Random,
    /// Choose the credential with the fewest successful requests so far.
    LeastUsed,
}

// ── PooledCredential ─────────────────────────────────────────────────────────

/// A single credential entry inside a [`CredentialPool`].
#[derive(Clone)]
pub struct PooledCredential {
    /// The API key string.
    pub api_key: String,
    /// Higher value = higher priority. Credentials are sorted descending by
    /// priority on pool creation.
    pub priority: u32,
    /// Number of successful (non-exhausted) requests dispatched with this key.
    pub request_count: u64,
    /// When `Some(t)`, this credential is exhausted and must not be used until
    /// `Instant::now() >= t`.
    exhausted_until: Option<Instant>,
}

impl std::fmt::Debug for PooledCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the API key so it is never printed in logs or panic messages.
        let hint = if self.api_key.len() >= 4 {
            format!("****{}", &self.api_key[self.api_key.len() - 4..])
        } else {
            "****".to_string()
        };
        f.debug_struct("PooledCredential")
            .field("api_key", &hint)
            .field("priority", &self.priority)
            .field("request_count", &self.request_count)
            .field("is_exhausted", &!self.is_available())
            .finish()
    }
}

impl PooledCredential {
    fn new(api_key: String, priority: u32) -> Self {
        Self {
            api_key,
            priority,
            request_count: 0,
            exhausted_until: None,
        }
    }

    /// Returns `true` if the credential is currently available (not exhausted).
    fn is_available(&self) -> bool {
        match self.exhausted_until {
            None => true,
            Some(until) => Instant::now() >= until,
        }
    }
}

// ── CredentialSnapshot ───────────────────────────────────────────────────────

/// Redacted view of a [`PooledCredential`] safe for diagnostics and dashboards.
///
/// The raw API key is never exposed; only a hint showing the last four
/// characters (prefixed by `****`) is included.
#[derive(Debug, Clone)]
pub struct CredentialSnapshot {
    /// Redacted key hint, e.g. `"****abcd"`.
    pub key_hint: String,
    /// Higher value = higher priority.
    pub priority: u32,
    /// Number of successful requests dispatched with this credential.
    pub request_count: u64,
    /// Whether this credential is currently exhausted (in cooldown).
    pub is_exhausted: bool,
}

impl CredentialSnapshot {
    fn from_credential(c: &PooledCredential) -> Self {
        let hint = if c.api_key.len() >= 4 {
            format!("****{}", &c.api_key[c.api_key.len() - 4..])
        } else {
            "****".to_string()
        };
        Self {
            key_hint: hint,
            priority: c.priority,
            request_count: c.request_count,
            is_exhausted: !c.is_available(),
        }
    }
}

// ── CredentialPool ────────────────────────────────────────────────────────────

/// Thread-safe pool of API keys for a single provider.
///
/// The pool is `Send + Sync` and intended to be shared behind an `Arc`.
///
/// ```rust
/// use librefang_llm_drivers::credential_pool::{CredentialPool, PoolStrategy};
///
/// let pool = CredentialPool::new(
///     vec![
///         ("sk-key-a".to_string(), 10),
///         ("sk-key-b".to_string(), 5),
///     ],
///     PoolStrategy::RoundRobin,
/// );
///
/// if let Some(key) = pool.acquire() {
///     // use key …
///     pool.mark_success(&key);
/// }
/// ```
/// Inner state protected by a single mutex so that the RoundRobin index and
/// the credential list are always read and written atomically together,
/// eliminating any TOCTOU between reading the index and selecting the
/// credential.
struct CredentialPoolInner {
    credentials: Vec<PooledCredential>,
    /// Next candidate index for `RoundRobin` (absolute index into `credentials`).
    round_robin_idx: usize,
}

pub struct CredentialPool {
    /// All mutable state behind a single lock.
    inner: Mutex<CredentialPoolInner>,
    strategy: PoolStrategy,
    /// How long an exhausted credential stays in cooldown.
    exhausted_ttl: Duration,
}

impl CredentialPool {
    /// Create a new pool from a list of `(api_key, priority)` pairs.
    ///
    /// Credentials are sorted by priority **descending** so that `FillFirst`
    /// simply picks the first available entry.
    pub fn new(keys: Vec<(String, u32)>, strategy: PoolStrategy) -> Self {
        let mut credentials: Vec<PooledCredential> = keys
            .into_iter()
            .map(|(k, p)| PooledCredential::new(k, p))
            .collect();
        // Sort descending: highest priority first.
        credentials.sort_unstable_by_key(|c| std::cmp::Reverse(c.priority));

        Self {
            inner: Mutex::new(CredentialPoolInner {
                credentials,
                round_robin_idx: 0,
            }),
            strategy,
            exhausted_ttl: DEFAULT_EXHAUSTED_TTL,
        }
    }

    /// Create a pool with a custom exhaustion cooldown period.
    pub fn with_exhausted_ttl(
        keys: Vec<(String, u32)>,
        strategy: PoolStrategy,
        exhausted_ttl: Duration,
    ) -> Self {
        let mut pool = Self::new(keys, strategy);
        pool.exhausted_ttl = exhausted_ttl;
        pool
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Select the next available credential according to the pool strategy.
    ///
    /// Returns a **cloned** copy of the chosen API key, or `None` when all
    /// credentials are currently exhausted.
    pub fn acquire(&self) -> Option<String> {
        // Lock the entire inner state so that the RoundRobin index read and
        // the credential selection happen atomically — no other thread can
        // advance the index between reading it and using it.
        let mut inner = self.inner.lock().expect("credential pool lock poisoned");
        match self.strategy {
            PoolStrategy::FillFirst => Self::acquire_fill_first(&inner.credentials),
            PoolStrategy::RoundRobin => {
                let start = inner.round_robin_idx;
                let result = Self::acquire_round_robin(&inner.credentials, start);
                if result.is_some() {
                    // Advance the index past the entry we just selected so
                    // subsequent calls pick the next one.
                    let available: Vec<usize> = (0..inner.credentials.len())
                        .filter(|&i| inner.credentials[i].is_available())
                        .collect();
                    if available.len() > 1 {
                        let pos_in_available = available
                            .iter()
                            .position(|&i| {
                                result.as_deref() == Some(inner.credentials[i].api_key.as_str())
                            })
                            .unwrap_or(0);
                        // Store the index of the *next* available slot (wrapping).
                        let next_pos = (pos_in_available + 1) % available.len();
                        inner.round_robin_idx = available[next_pos];
                    }
                }
                result
            }
            PoolStrategy::Random => Self::acquire_random(&inner.credentials),
            PoolStrategy::LeastUsed => Self::acquire_least_used(&inner.credentials),
        }
    }

    /// Report that a request with `api_key` was rate-limited (429) or quota-
    /// exhausted (402).  The credential is placed in cooldown for
    /// `exhausted_ttl`.
    pub fn mark_exhausted(&self, api_key: &str) {
        let mut inner = self.inner.lock().expect("credential pool lock poisoned");
        let until = Instant::now() + self.exhausted_ttl;
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.exhausted_until = Some(until);
        }
    }

    /// Report that a request with `api_key` succeeded.  Increments the
    /// credential's `request_count` and clears any leftover exhaustion marker
    /// (e.g. if a provider recovered before the TTL expired).
    pub fn mark_success(&self, api_key: &str) {
        let mut inner = self.inner.lock().expect("credential pool lock poisoned");
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.request_count = c.request_count.saturating_add(1);
            // Always clear the exhaustion marker on success — the key is working
            // again regardless of whether the cooldown TTL has elapsed.
            c.exhausted_until = None;
        }
    }

    /// Number of currently available (non-exhausted) credentials.
    pub fn available_count(&self) -> usize {
        let inner = self.inner.lock().expect("credential pool lock poisoned");
        inner
            .credentials
            .iter()
            .filter(|c| c.is_available())
            .count()
    }

    /// Total number of credentials in the pool (available + exhausted).
    pub fn total_count(&self) -> usize {
        let inner = self.inner.lock().expect("credential pool lock poisoned");
        inner.credentials.len()
    }

    /// Returns a redacted snapshot of all credentials (for diagnostics / dashboards).
    ///
    /// API keys are never included in the snapshot; each entry contains only a
    /// `key_hint` (last 4 chars prefixed by `****`), priority, request count,
    /// and exhaustion status.  The list is sorted by priority descending,
    /// matching the internal ordering.
    pub fn snapshot(&self) -> Vec<CredentialSnapshot> {
        let inner = self.inner.lock().expect("credential pool lock poisoned");
        inner
            .credentials
            .iter()
            .map(CredentialSnapshot::from_credential)
            .collect()
    }

    // ── Strategy helpers (operate on a locked slice) ─────────────────────────

    /// FillFirst: return the first available entry (highest priority first
    /// because the vec is sorted descending).
    fn acquire_fill_first(creds: &[PooledCredential]) -> Option<String> {
        creds
            .iter()
            .find(|c| c.is_available())
            .map(|c| c.api_key.clone())
    }

    /// RoundRobin: starting from `start_idx`, pick the next available entry
    /// (wrapping around).
    fn acquire_round_robin(creds: &[PooledCredential], start_idx: usize) -> Option<String> {
        let n = creds.len();
        if n == 0 {
            return None;
        }
        // Collect indices of available credentials in sorted order.
        let available: Vec<usize> = (0..n).filter(|&i| creds[i].is_available()).collect();
        if available.is_empty() {
            return None;
        }
        // Find the first available index >= start_idx (wrap around if needed).
        let idx = available
            .iter()
            .find(|&&i| i >= start_idx % n)
            .copied()
            .unwrap_or(available[0]);
        Some(creds[idx].api_key.clone())
    }

    /// Random: pick a random available entry using a simple LCG so we avoid
    /// pulling in `rand` crate.
    fn acquire_random(creds: &[PooledCredential]) -> Option<String> {
        let available: Vec<&PooledCredential> = creds.iter().filter(|c| c.is_available()).collect();
        if available.is_empty() {
            return None;
        }
        // Simple LCG seeded by the current time in nanoseconds.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize;
        let idx = seed % available.len();
        Some(available[idx].api_key.clone())
    }

    /// LeastUsed: pick the available credential with the lowest `request_count`.
    fn acquire_least_used(creds: &[PooledCredential]) -> Option<String> {
        creds
            .iter()
            .filter(|c| c.is_available())
            .min_by_key(|c| c.request_count)
            .map(|c| c.api_key.clone())
    }
}

// ── ArcPool convenience wrapper ───────────────────────────────────────────────

/// A cheaply-cloneable handle to a [`CredentialPool`].
///
/// Prefer this type when the pool needs to be shared across async tasks or
/// stored inside driver structs.
pub type ArcCredentialPool = Arc<CredentialPool>;

/// Construct a new [`ArcCredentialPool`] from key-priority pairs.
pub fn new_arc_pool(keys: Vec<(String, u32)>, strategy: PoolStrategy) -> ArcCredentialPool {
    Arc::new(CredentialPool::new(keys, strategy))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn make_pool(keys: &[(&str, u32)], strategy: PoolStrategy) -> CredentialPool {
        let keys = keys.iter().map(|(k, p)| (k.to_string(), *p)).collect();
        CredentialPool::new(keys, strategy)
    }

    // ── FillFirst ─────────────────────────────────────────────────────────────

    #[test]
    fn fill_first_picks_highest_priority() {
        let pool = make_pool(
            &[("key-low", 1), ("key-high", 10), ("key-mid", 5)],
            PoolStrategy::FillFirst,
        );
        assert_eq!(pool.acquire().as_deref(), Some("key-high"));
        assert_eq!(pool.acquire().as_deref(), Some("key-high"));
    }

    #[test]
    fn fill_first_falls_back_when_exhausted() {
        let pool = make_pool(&[("key-a", 10), ("key-b", 5)], PoolStrategy::FillFirst);
        pool.mark_exhausted("key-a");
        assert_eq!(pool.acquire().as_deref(), Some("key-b"));
    }

    #[test]
    fn fill_first_returns_none_when_all_exhausted() {
        let pool = make_pool(&[("key-a", 10)], PoolStrategy::FillFirst);
        pool.mark_exhausted("key-a");
        assert!(pool.acquire().is_none());
    }

    // ── RoundRobin ────────────────────────────────────────────────────────────

    #[test]
    fn round_robin_distributes_across_keys() {
        let pool = make_pool(
            &[("key-a", 1), ("key-b", 1), ("key-c", 1)],
            PoolStrategy::RoundRobin,
        );
        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..6 {
            if let Some(k) = pool.acquire() {
                seen.insert(k);
            }
        }
        assert_eq!(seen.len(), 3, "all three keys should be used");
    }

    #[test]
    fn round_robin_skips_exhausted() {
        let pool = make_pool(&[("key-a", 1), ("key-b", 1)], PoolStrategy::RoundRobin);
        pool.mark_exhausted("key-a");
        for _ in 0..4 {
            assert_eq!(pool.acquire().as_deref(), Some("key-b"));
        }
    }

    // ── LeastUsed ─────────────────────────────────────────────────────────────

    #[test]
    fn least_used_picks_freshest_key() {
        let pool = make_pool(&[("key-a", 1), ("key-b", 1)], PoolStrategy::LeastUsed);
        pool.mark_success("key-a");
        pool.mark_success("key-a");
        // key-b has 0 requests, key-a has 2 — pool should choose key-b.
        assert_eq!(pool.acquire().as_deref(), Some("key-b"));
    }

    // ── Random ────────────────────────────────────────────────────────────────

    #[test]
    fn random_returns_available_key() {
        let pool = make_pool(&[("key-only", 1)], PoolStrategy::Random);
        assert_eq!(pool.acquire().as_deref(), Some("key-only"));
    }

    #[test]
    fn random_none_when_all_exhausted() {
        let pool = make_pool(&[("key-only", 1)], PoolStrategy::Random);
        pool.mark_exhausted("key-only");
        assert!(pool.acquire().is_none());
    }

    // ── mark_success / mark_exhausted ─────────────────────────────────────────

    #[test]
    fn mark_success_increments_request_count() {
        let pool = make_pool(&[("key-a", 1)], PoolStrategy::FillFirst);
        pool.mark_success("key-a");
        pool.mark_success("key-a");
        let snap = pool.snapshot();
        assert_eq!(snap[0].request_count, 2);
    }

    #[test]
    fn mark_success_clears_active_exhaustion() {
        // A credential marked exhausted with a long TTL should become available
        // immediately after mark_success (early-recovery path).
        let pool = CredentialPool::with_exhausted_ttl(
            vec![("key-a".to_string(), 1)],
            PoolStrategy::FillFirst,
            Duration::from_secs(3600),
        );
        pool.mark_exhausted("key-a");
        assert!(pool.acquire().is_none(), "should be exhausted");
        pool.mark_success("key-a");
        assert_eq!(
            pool.acquire().as_deref(),
            Some("key-a"),
            "should be available after mark_success clears exhaustion"
        );
    }

    #[test]
    fn mark_exhausted_then_available_count_decreases() {
        let pool = make_pool(&[("key-a", 10), ("key-b", 5)], PoolStrategy::FillFirst);
        assert_eq!(pool.available_count(), 2);
        pool.mark_exhausted("key-a");
        assert_eq!(pool.available_count(), 1);
        pool.mark_exhausted("key-b");
        assert_eq!(pool.available_count(), 0);
    }

    // ── Priority ordering ─────────────────────────────────────────────────────

    #[test]
    fn credentials_sorted_by_priority_descending() {
        let pool = make_pool(
            &[("key-low", 1), ("key-high", 100), ("key-mid", 50)],
            PoolStrategy::FillFirst,
        );
        let snap = pool.snapshot();
        assert_eq!(snap[0].priority, 100);
        assert_eq!(snap[1].priority, 50);
        assert_eq!(snap[2].priority, 1);
    }

    // ── available_count / total_count ─────────────────────────────────────────

    #[test]
    fn total_count_stable() {
        let pool = make_pool(&[("k1", 1), ("k2", 2)], PoolStrategy::FillFirst);
        assert_eq!(pool.total_count(), 2);
        pool.mark_exhausted("k1");
        assert_eq!(pool.total_count(), 2); // exhausted ≠ removed
    }

    // ── Custom TTL ────────────────────────────────────────────────────────────

    #[test]
    fn custom_ttl_zero_recovers_immediately() {
        let pool = CredentialPool::with_exhausted_ttl(
            vec![("key-a".to_string(), 1)],
            PoolStrategy::FillFirst,
            Duration::from_secs(0),
        );
        pool.mark_exhausted("key-a");
        // With TTL=0 the instant is already in the past — available immediately.
        assert!(pool.acquire().is_some());
    }

    // ── ArcCredentialPool ─────────────────────────────────────────────────────

    #[test]
    fn arc_pool_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ArcCredentialPool>();
    }

    #[test]
    fn new_arc_pool_works() {
        let pool = new_arc_pool(vec![("key-a".to_string(), 1)], PoolStrategy::RoundRobin);
        assert_eq!(pool.acquire().as_deref(), Some("key-a"));
    }
}
