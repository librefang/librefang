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

/// Build the redacted hint for an API key. Returns `"****"` plus the last
/// four characters (Unicode-safe — we count by `char`, never by byte
/// boundary, so an exotic key containing multi-byte chars cannot panic the
/// diagnostic path with an `is_char_boundary` slice error). API keys are
/// expected to be ASCII in practice; this is defense-in-depth so a bad
/// pool config never takes down the snapshot endpoint.
fn redact_key_hint(api_key: &str) -> String {
    // Collect the last four chars in original order without slicing on
    // arbitrary byte offsets. `chars().count()` is O(n) on UTF-8 but key
    // strings are short (typically < 200 bytes) so the cost is negligible
    // and is paid only at snapshot time (diagnostics), never per-request.
    let total = api_key.chars().count();
    if total >= 4 {
        let tail: String = api_key.chars().skip(total - 4).collect();
        format!("****{tail}")
    } else {
        "****".to_string()
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Default cooldown duration after a 429 rate-limit response.
pub const DEFAULT_EXHAUSTED_TTL: Duration = Duration::from_secs(60 * 60); // 1 hour
/// Default cooldown duration after a 402 credit-exhausted response.
///
/// Quota refresh windows are typically daily, so a longer cooldown avoids
/// burning retries against a key the provider has already disowned for the
/// current billing window. Issue #4965 specifies 24 hours.
pub const DEFAULT_CREDIT_EXHAUSTED_TTL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

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
    /// Operator-facing label from `config.toml` (e.g. `"Primary"`, `"Backup"`).
    /// Carried with the credential so the snapshot can attribute labels to
    /// the right materialized key — never reconstruct by indexing into the
    /// original config list, which loses alignment when boot skips a key
    /// whose env var is unset (Codex #5260 review: "Do not match credential
    /// labels by position").
    pub label: String,
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
        let hint = redact_key_hint(&self.api_key);
        f.debug_struct("PooledCredential")
            .field("api_key", &hint)
            .field("label", &self.label)
            .field("priority", &self.priority)
            .field("request_count", &self.request_count)
            .field("is_exhausted", &!self.is_available())
            .finish()
    }
}

impl PooledCredential {
    fn new(api_key: String, label: String, priority: u32) -> Self {
        Self {
            api_key,
            label,
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
    /// Operator-facing label from `config.toml` (e.g. `"Primary"`, `"Backup"`).
    /// Empty string when the pool was built without labels (legacy callers /
    /// tests). Always carried alongside the credential — never reconstructed
    /// by positional indexing into the original config list (Codex #5260).
    pub label: String,
    /// Redacted key hint, e.g. `"****abcd"`.
    pub key_hint: String,
    /// Higher value = higher priority.
    pub priority: u32,
    /// Number of successful requests dispatched with this credential.
    pub request_count: u64,
    /// Whether this credential is currently exhausted (in cooldown).
    pub is_exhausted: bool,
    /// Remaining cooldown in seconds when `is_exhausted = true`, else `None`.
    /// `Some(u64::MAX)` indicates a permanently-marked key (auth failure).
    pub cooldown_remaining_secs: Option<u64>,
}

impl CredentialSnapshot {
    fn from_credential(c: &PooledCredential) -> Self {
        let hint = redact_key_hint(&c.api_key);
        let now = Instant::now();
        let (is_exhausted, cooldown) = match c.exhausted_until {
            None => (false, None),
            Some(until) => {
                if now >= until {
                    (false, None)
                } else {
                    let remaining = until.saturating_duration_since(now).as_secs();
                    // mark_permanent uses Instant::now() + 100 years — any
                    // value larger than a year is treated as permanent for
                    // diagnostic purposes.
                    let remaining = if remaining > 365 * 86400 {
                        u64::MAX
                    } else {
                        remaining
                    };
                    (true, Some(remaining))
                }
            }
        };
        Self {
            label: c.label.clone(),
            key_hint: hint,
            priority: c.priority,
            request_count: c.request_count,
            is_exhausted,
            cooldown_remaining_secs: cooldown,
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
    /// How long a rate-limited (429) credential stays in cooldown.
    exhausted_ttl: Duration,
    /// How long a credit-exhausted (402) credential stays in cooldown.
    credit_exhausted_ttl: Duration,
}

impl CredentialPool {
    /// Create a new pool from a list of `(api_key, priority)` pairs.
    ///
    /// Credentials are sorted by priority **descending** so that `FillFirst`
    /// simply picks the first available entry.
    ///
    /// Each credential is materialized with an empty operator-facing label.
    /// Callers that have labels available from `config.toml` should use
    /// [`CredentialPool::new_with_labels`] so the redacted snapshot can
    /// attribute labels correctly (see Codex #5260).
    pub fn new(keys: Vec<(String, u32)>, strategy: PoolStrategy) -> Self {
        let labeled: Vec<(String, String, u32)> = keys
            .into_iter()
            .map(|(k, p)| (k, String::new(), p))
            .collect();
        Self::new_with_labels(labeled, strategy)
    }

    /// Create a new pool from `(api_key, label, priority)` triples.
    ///
    /// Use this constructor in any code path where the operator-facing label
    /// from `config.toml` is available. The label is carried inside each
    /// [`PooledCredential`] and emitted by [`CredentialPool::snapshot`], so
    /// diagnostics never reconstruct the label by indexing into the original
    /// config list (which loses alignment when boot skips a key whose env
    /// var is unset).
    pub fn new_with_labels(keys: Vec<(String, String, u32)>, strategy: PoolStrategy) -> Self {
        let mut credentials: Vec<PooledCredential> = keys
            .into_iter()
            .map(|(k, label, p)| PooledCredential::new(k, label, p))
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
            credit_exhausted_ttl: DEFAULT_CREDIT_EXHAUSTED_TTL,
        }
    }

    /// Create a pool with a custom rate-limit cooldown period.
    pub fn with_exhausted_ttl(
        keys: Vec<(String, u32)>,
        strategy: PoolStrategy,
        exhausted_ttl: Duration,
    ) -> Self {
        let mut pool = Self::new(keys, strategy);
        pool.exhausted_ttl = exhausted_ttl;
        pool
    }

    /// Create a pool with custom cooldowns for both rate-limit (429) and
    /// credit-exhausted (402) responses.
    pub fn with_cooldowns(
        keys: Vec<(String, u32)>,
        strategy: PoolStrategy,
        exhausted_ttl: Duration,
        credit_exhausted_ttl: Duration,
    ) -> Self {
        let mut pool = Self::new(keys, strategy);
        pool.exhausted_ttl = exhausted_ttl;
        pool.credit_exhausted_ttl = credit_exhausted_ttl;
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
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match self.strategy {
            PoolStrategy::FillFirst => Self::acquire_fill_first(&inner.credentials),
            PoolStrategy::RoundRobin => {
                let n = inner.credentials.len();
                if n == 0 {
                    return None;
                }
                // Defense-in-depth: normalize the cursor before use so any
                // prior shrink (hot-reload) cannot point us at an
                // out-of-bounds slot. Equivalent to the audit's
                // `self.index %= active.len().max(1)` post-mutation hook,
                // applied on every read so it survives any future code path
                // that mutates `credentials` without explicit cleanup.
                let start = inner.round_robin_idx % n;
                // Single cycle-aware computation produces both the selected
                // key and the next cursor — the outer cursor advance and the
                // visible-key view come from one snapshot, eliminating the
                // previous double-recompute race.
                match Self::acquire_round_robin(&inner.credentials, start) {
                    Some((key, next_idx)) => {
                        inner.round_robin_idx = next_idx;
                        Some(key)
                    }
                    None => None,
                }
            }
            PoolStrategy::Random => Self::acquire_random(&inner.credentials),
            PoolStrategy::LeastUsed => Self::acquire_least_used(&inner.credentials),
        }
    }

    /// Report that a request with `api_key` was rate-limited (429).  The
    /// credential is placed in cooldown for `exhausted_ttl` (default 1 hour).
    ///
    /// For quota-exhausted (402) responses use [`mark_credit_exhausted`]
    /// instead — quota windows are typically daily, so the cooldown is longer.
    pub fn mark_exhausted(&self, api_key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let until = Instant::now() + self.exhausted_ttl;
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.exhausted_until = Some(until);
        }
    }

    /// Report that a request with `api_key` returned 402 (credits / quota
    /// exhausted). The credential is placed in cooldown for
    /// `credit_exhausted_ttl` (default 24 hours per #4965 spec).
    pub fn mark_credit_exhausted(&self, api_key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let until = Instant::now() + self.credit_exhausted_ttl;
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.exhausted_until = Some(until);
        }
    }

    /// Report that a credential is permanently invalid (e.g. auth failure).
    /// Unlike [`mark_exhausted`] which uses a TTL-based cooldown, this marks
    /// the key as unavailable for the lifetime of the pool. The only way to
    /// recover a permanently-exhausted key is via [`mark_success`] from a
    /// concurrent code path or a hot-reload that rebuilds the pool.
    ///
    /// Implementation uses a far-future timestamp (100 years) so that
    /// `is_available()` naturally returns `false` without a separate flag.
    pub fn mark_permanent(&self, api_key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // ~100 years from now — well past any realistic daemon lifetime.
        let far_future = Instant::now() + Duration::from_secs(365 * 100 * 86400);
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.exhausted_until = Some(far_future);
        }
    }

    /// Report that a request with `api_key` succeeded.  Increments the
    /// credential's `request_count` and clears any leftover exhaustion marker
    /// (e.g. if a provider recovered before the TTL expired).
    pub fn mark_success(&self, api_key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = inner.credentials.iter_mut().find(|c| c.api_key == api_key) {
            c.request_count = c.request_count.saturating_add(1);
            // Always clear the exhaustion marker on success — the key is working
            // again regardless of whether the cooldown TTL has elapsed.
            c.exhausted_until = None;
        }
    }

    /// Number of currently available (non-exhausted) credentials.
    pub fn available_count(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner
            .credentials
            .iter()
            .filter(|c| c.is_available())
            .count()
    }

    /// Total number of credentials in the pool (available + exhausted).
    pub fn total_count(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.credentials.len()
    }

    /// Returns the pool's selection strategy.
    pub fn strategy(&self) -> PoolStrategy {
        self.strategy.clone()
    }

    /// Returns a redacted snapshot of all credentials (for diagnostics / dashboards).
    ///
    /// API keys are never included in the snapshot; each entry contains only a
    /// `key_hint` (last 4 chars prefixed by `****`), priority, request count,
    /// and exhaustion status.  The list is sorted by priority descending,
    /// matching the internal ordering.
    pub fn snapshot(&self) -> Vec<CredentialSnapshot> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
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
    ///
    /// Returns `(selected_api_key, next_cursor)` so the caller stores the
    /// cursor position for the slot **after** the one we just picked, and
    /// both values are derived from a single iteration over the credential
    /// list. The previous implementation collected `available` into a `Vec`,
    /// chose with `find(|&&i| i >= start_idx % n)`, then recomputed
    /// `available` a second time in the caller to advance the cursor — two
    /// snapshots that could disagree if the credential state changed
    /// between them. The combined return prevents that race and the
    /// `>=`-based scan that biased rotation toward low-index slots.
    ///
    /// Iteration is cycle-aware (`(0..n).cycle().skip(start).take(n)`) so
    /// the function never compares absolute indices and never panics on a
    /// shrunk credential list — the caller normalizes `start` modulo `n`
    /// in `acquire`, and `take(n)` bounds the search to at most one full
    /// rotation.
    fn acquire_round_robin(
        creds: &[PooledCredential],
        start_idx: usize,
    ) -> Option<(String, usize)> {
        let n = creds.len();
        if n == 0 {
            return None;
        }
        // Cycle-aware single scan. `start_idx` is expected to already be in
        // `[0, n)` (the caller normalizes), but `% n` here is defensive in
        // case this helper is invoked directly from a future call site.
        let start = start_idx % n;
        let picked = (0..n)
            .cycle()
            .skip(start)
            .take(n)
            .find(|&i| creds[i].is_available())?;
        let next_idx = (picked + 1) % n;
        Some((creds[picked].api_key.clone(), next_idx))
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

    /// Test-only: simulate a hot-reload that replaces the credential list
    /// (e.g. operator edited `config.toml`, the daemon rebuilt the pool with
    /// fewer keys). Production hot-reload constructs a brand-new
    /// `CredentialPool`, but in unit tests we want to exercise the path
    /// where the existing pool's `round_robin_idx` was previously advanced
    /// past the new `credentials.len()` and prove that `acquire` normalizes
    /// rather than panicking or wrapping silently to the wrong key.
    #[cfg(test)]
    fn replace_credentials_for_test(&self, new_keys: Vec<(String, u32)>) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut creds: Vec<PooledCredential> = new_keys
            .into_iter()
            .map(|(k, p)| PooledCredential::new(k, String::new(), p))
            .collect();
        creds.sort_unstable_by_key(|c| std::cmp::Reverse(c.priority));
        inner.credentials = creds;
        // Deliberately do NOT reset round_robin_idx — the regression we are
        // guarding against is exactly the case where the cursor is stale
        // relative to the new (smaller) credential list.
    }
}

// ── ArcPool convenience wrapper ───────────────────────────────────────────────

/// A cheaply-cloneable handle to a [`CredentialPool`].
///
/// Prefer this type when the pool needs to be shared across async tasks or
/// stored inside driver structs.
pub type ArcCredentialPool = Arc<CredentialPool>;

/// Construct a new [`ArcCredentialPool`] from key-priority pairs.
///
/// Credentials are stored without operator-facing labels — use
/// [`new_arc_pool_with_labels`] when labels from `config.toml` are available
/// so [`CredentialPool::snapshot`] can return them in the right order.
pub fn new_arc_pool(keys: Vec<(String, u32)>, strategy: PoolStrategy) -> ArcCredentialPool {
    Arc::new(CredentialPool::new(keys, strategy))
}

/// Construct a new [`ArcCredentialPool`] from `(api_key, label, priority)` triples.
///
/// Preferred over [`new_arc_pool`] in the kernel boot path so labels are
/// carried with the materialized credentials (Codex #5260: never attribute
/// labels by positional index after env-var resolution may have skipped a
/// configured key).
pub fn new_arc_pool_with_labels(
    keys: Vec<(String, String, u32)>,
    strategy: PoolStrategy,
) -> ArcCredentialPool {
    Arc::new(CredentialPool::new_with_labels(keys, strategy))
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

    // ── RoundRobin index-desync regressions (rollup item 1 + item 3) ─────────

    /// Three-key pool with the middle key exhausted: the cursor previously
    /// landed on slot 2 (`key-c`) on first acquire because the absolute-index
    /// `>=` scan biased toward higher slots, and a subsequent shrink of the
    /// active set could leave the cursor pointing past `available.len()`. The
    /// cycle-aware refactor must alternate between only the two surviving
    /// keys (`key-a` and `key-c`) in fixed order with no panic, no skip.
    #[test]
    fn round_robin_alternates_between_surviving_keys_when_middle_exhausted() {
        // All three credentials carry equal priority so the sort is stable
        // on the declared order — slot 0 = key-a, slot 1 = key-b, slot 2 = key-c.
        let pool = make_pool(
            &[("key-a", 1), ("key-b", 1), ("key-c", 1)],
            PoolStrategy::RoundRobin,
        );
        pool.mark_exhausted("key-b");
        let picks: Vec<String> = (0..4).filter_map(|_| pool.acquire()).collect();
        assert_eq!(picks.len(), 4, "no acquire returned None");
        for k in &picks {
            assert_ne!(k, "key-b", "exhausted key must never be returned");
            assert!(
                k == "key-a" || k == "key-c",
                "unexpected key {k} — pool returned a non-pool entry"
            );
        }
        // Must alternate in deterministic order — the previous biased rotation
        // would repeat the same key on successive acquires when the cursor
        // got stuck past the exhausted slot.
        assert_eq!(picks[0], picks[2], "alternation broken at slot 2");
        assert_eq!(picks[1], picks[3], "alternation broken at slot 3");
        assert_ne!(
            picks[0], picks[1],
            "two consecutive acquires returned same key"
        );
    }

    /// Five-key pool with two non-adjacent exhausted slots: rotation must
    /// visit only the three survivors in order, cycling without panic.
    #[test]
    fn round_robin_skips_multiple_exhausted_keys() {
        // Equal priority so the declared order is preserved.
        let pool = make_pool(
            &[
                ("key-1", 1),
                ("key-2", 1),
                ("key-3", 1),
                ("key-4", 1),
                ("key-5", 1),
            ],
            PoolStrategy::RoundRobin,
        );
        pool.mark_exhausted("key-3");
        pool.mark_exhausted("key-5");
        // Two full cycles — six acquires across three survivors.
        let picks: Vec<String> = (0..6).filter_map(|_| pool.acquire()).collect();
        assert_eq!(picks.len(), 6);
        let survivors: HashSet<&str> = picks.iter().map(String::as_str).collect();
        assert_eq!(
            survivors,
            HashSet::from(["key-1", "key-2", "key-4"]),
            "rotation visited an unexpected set of keys"
        );
        // Each survivor visited exactly twice across two cycles.
        for s in ["key-1", "key-2", "key-4"] {
            let count = picks.iter().filter(|k| k.as_str() == s).count();
            assert_eq!(count, 2, "survivor {s} visited {count} times, expected 2");
        }
    }

    /// Single-key pool with the only key exhausted: must return `None`, not
    /// panic on `% 0` or out-of-bounds indexing.
    #[test]
    fn round_robin_single_key_all_exhausted_returns_none() {
        let pool = make_pool(&[("only-key", 1)], PoolStrategy::RoundRobin);
        pool.mark_exhausted("only-key");
        assert!(pool.acquire().is_none());
        // Call repeatedly — must stay None, must not panic.
        for _ in 0..5 {
            assert!(pool.acquire().is_none());
        }
    }

    /// Hot-reload shrink: the cursor was advanced to a high slot when the
    /// pool held five credentials; reload replaces the list with two
    /// credentials and the stale cursor (e.g. 4) now points past the new
    /// `credentials.len()`. The next `acquire` must normalize the cursor
    /// and return one of the two surviving keys — never panic, never wrap
    /// silently to a key that no longer exists.
    #[test]
    fn round_robin_recovers_when_hot_reload_shrinks_pool() {
        let pool = make_pool(
            &[
                ("key-1", 1),
                ("key-2", 1),
                ("key-3", 1),
                ("key-4", 1),
                ("key-5", 1),
            ],
            PoolStrategy::RoundRobin,
        );
        // Advance the internal cursor by exhausting five rotations — the
        // cursor now sits somewhere in `[0, 5)` but specifically past slot
        // 1 after the fifth acquire.
        for _ in 0..5 {
            let _ = pool.acquire();
        }
        // Simulate hot-reload to a two-key pool.
        pool.replace_credentials_for_test(vec![
            ("survivor-a".to_string(), 1),
            ("survivor-b".to_string(), 1),
        ]);
        // First post-reload acquire must succeed and return one of the
        // survivors. Do this twice — the second proves rotation is still
        // sane after the normalize.
        let pick1 = pool.acquire().expect("post-reload acquire must succeed");
        assert!(
            pick1 == "survivor-a" || pick1 == "survivor-b",
            "unexpected key after reload: {pick1}"
        );
        let pick2 = pool
            .acquire()
            .expect("second post-reload acquire must succeed");
        assert!(
            pick2 == "survivor-a" || pick2 == "survivor-b",
            "unexpected key after reload: {pick2}"
        );
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

    // ── 402 credit-exhausted (#4965) ──────────────────────────────────────────

    #[test]
    fn mark_credit_exhausted_uses_longer_ttl() {
        // Confirm the 402 path uses the credit-exhausted TTL rather than the
        // rate-limit TTL: with rate-limit TTL = 0 but credit TTL = 1h, a key
        // marked via mark_credit_exhausted stays unavailable.
        let pool = CredentialPool::with_cooldowns(
            vec![("key-a".to_string(), 1)],
            PoolStrategy::FillFirst,
            Duration::from_secs(0), // rate-limit TTL — would recover immediately
            Duration::from_secs(3600), // credit TTL — would block for an hour
        );
        pool.mark_credit_exhausted("key-a");
        assert!(
            pool.acquire().is_none(),
            "credit-exhausted should respect credit_exhausted_ttl, not exhausted_ttl"
        );
    }

    #[test]
    fn mark_credit_exhausted_default_24h() {
        // Sanity: the constant matches the issue spec.
        assert_eq!(
            DEFAULT_CREDIT_EXHAUSTED_TTL,
            Duration::from_secs(24 * 60 * 60),
            "Issue #4965 spec: 402 cooldown is 24 hours"
        );
    }

    #[test]
    fn snapshot_reports_cooldown_remaining() {
        let pool = CredentialPool::with_exhausted_ttl(
            vec![("key-a".to_string(), 1)],
            PoolStrategy::FillFirst,
            Duration::from_secs(120),
        );
        pool.mark_exhausted("key-a");
        let snap = pool.snapshot();
        assert!(snap[0].is_exhausted);
        let remaining = snap[0].cooldown_remaining_secs.expect("cooldown set");
        // Some jitter is expected, but should be close to 120s.
        assert!(
            remaining > 60 && remaining <= 120,
            "expected ~120s cooldown remaining, got {remaining}"
        );
    }

    #[test]
    fn snapshot_reports_permanent_marker() {
        let pool = make_pool(&[("key-a", 1)], PoolStrategy::FillFirst);
        pool.mark_permanent("key-a");
        let snap = pool.snapshot();
        assert!(snap[0].is_exhausted);
        assert_eq!(
            snap[0].cooldown_remaining_secs,
            Some(u64::MAX),
            "mark_permanent should sentinel-encode as u64::MAX"
        );
    }

    // ── label carry-through (#5260 round-2) ──────────────────────────────────

    /// The label travels with the credential through pool construction,
    /// snapshot rendering, and priority sort — never reconstructed by
    /// positional indexing into the original config list.
    #[test]
    fn snapshot_carries_label_per_credential() {
        let pool = CredentialPool::new_with_labels(
            vec![
                ("sk-low".to_string(), "Backup".to_string(), 5),
                ("sk-high".to_string(), "Primary".to_string(), 10),
            ],
            PoolStrategy::FillFirst,
        );
        let snap = pool.snapshot();
        // Priority-descending order: Primary (10) before Backup (5).
        assert_eq!(snap[0].label, "Primary");
        assert_eq!(snap[0].priority, 10);
        assert_eq!(snap[1].label, "Backup");
        assert_eq!(snap[1].priority, 5);
    }

    /// Cooldown is recorded against the underlying api_key match, which
    /// continues to carry its label — so an exhausted key's snapshot row
    /// still reports the correct operator-facing label.
    #[test]
    fn snapshot_label_survives_mark_exhausted() {
        let pool = CredentialPool::new_with_labels(
            vec![
                ("sk-a".to_string(), "Primary".to_string(), 10),
                ("sk-b".to_string(), "Backup".to_string(), 5),
            ],
            PoolStrategy::FillFirst,
        );
        pool.mark_exhausted("sk-a");
        let snap = pool.snapshot();
        assert_eq!(
            snap[0].label, "Primary",
            "Primary remains labelled Primary even when exhausted"
        );
        assert!(snap[0].is_exhausted);
        assert_eq!(snap[1].label, "Backup");
        assert!(!snap[1].is_exhausted);
    }

    /// The legacy unlabeled constructor still works and emits empty labels.
    /// This keeps the in-kernel `pooled_driver` unit-test helpers and any
    /// future internal call sites compiling without touching them.
    #[test]
    fn unlabeled_constructor_emits_empty_labels() {
        let pool = make_pool(&[("sk-a", 10), ("sk-b", 5)], PoolStrategy::FillFirst);
        let snap = pool.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].label, "");
        assert_eq!(snap[1].label, "");
    }

    // ── key_hint redaction (UTF-8 boundary safety) ────────────────────────────

    /// Defense-in-depth: a key with a non-ASCII character at the suffix boundary
    /// must NOT panic the redaction helper. API keys are expected to be ASCII
    /// in practice, but the diagnostic snapshot path is invoked from HTTP/CLI/
    /// dashboard rendering — a panic there would surface as a 500 to the caller.
    #[test]
    fn redact_key_hint_handles_multibyte_chars() {
        // 8-char key ending in a 4-byte emoji — `&s[s.len() - 4..]` would
        // panic with "byte index N is not a char boundary" on the old impl.
        let key = "abcd🦀ef"; // 7 chars; last 4 are 'd', '🦀', 'e', 'f'.
        let hint = super::redact_key_hint(key);
        assert_eq!(hint, "****d🦀ef");
        // Pure-ASCII fast path still works.
        assert_eq!(super::redact_key_hint("sk-abcd1234"), "****1234");
        // Short key (< 4 chars) falls back to a plain redaction marker.
        assert_eq!(super::redact_key_hint("xyz"), "****");
        assert_eq!(super::redact_key_hint(""), "****");
        // 4-byte char that occupies multiple bytes still counts as one char.
        let key2 = "🦀🦀🦀🦀"; // 4 chars / 16 bytes.
        assert_eq!(super::redact_key_hint(key2), "****🦀🦀🦀🦀");
    }
}
