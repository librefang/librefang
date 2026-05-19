//! Provider exhaustion state — budget-aware fallback chain (#4807).
//!
//! When a provider in a [`crate::LlmDriver`] fallback chain returns an
//! exhaustion-class error (rate-limit, quota / credit, operator-set budget,
//! authentication failure), the chain has no business retrying that slot on
//! the *next* request — the slot will simply error again, burning latency
//! and (for auth failures) potentially triggering lockouts.
//!
//! This module is the in-memory exhaustion ledger shared between the
//! fallback chain (which queries + records) and the metering layer (which
//! records when an operator-set spending cap kicks in). The store lives at
//! the kernel layer in a single [`std::sync::Arc`] and is handed to every
//! `FallbackChain` that gets constructed.
//!
//! ## Semantics
//!
//! - Storage is process-local: a daemon restart clears all state, by
//!   design. Persisting exhaustion across restarts would risk locking out
//!   a slot whose underlying issue (key rotation, billing top-up) is
//!   resolved out-of-band.
//! - Entries are auto-cleared by reads ([`Self::is_exhausted`]): once
//!   `until` passes, the next read returns `None` and removes the entry,
//!   so the chain naturally re-attempts the slot.
//! - For reasons without a server-reported reset hint
//!   ([`ExhaustionReason::QuotaExceeded`], [`ExhaustionReason::BudgetExceeded`],
//!   [`ExhaustionReason::AuthFailed`]) callers pass a long backoff
//!   (the canonical choice is [`DEFAULT_LONG_BACKOFF`] = 1h) — the
//!   underlying issue requires operator action, but auto-recovery lets the
//!   fallback chain heal on its own if the operator does fix things.
//!
//! ## Determinism
//!
//! Entry iteration order is non-deterministic ([`dashmap::DashMap`] sharded
//! by hash). [`Self::snapshot`] returns a `BTreeMap`-ordered `Vec` so any
//! string-formatted output (error messages, logs that surface exhaustion
//! state) is byte-identical across processes — preserving prompt-cache
//! determinism (#3298) even when exhaustion data leaks into a prompt.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::Serialize;

/// Long backoff applied to exhaustion reasons that require operator action
/// before the slot is usable again (quota, budget, auth). One hour is short
/// enough that a fixed operator action heals the chain on its own, long
/// enough that the chain doesn't waste an attempt every minute waiting for
/// the human.
pub const DEFAULT_LONG_BACKOFF: Duration = Duration::from_secs(60 * 60);

/// Reason a provider slot is currently considered unavailable. The variants
/// drive nothing here — they're recorded for logs / metrics / surfaced error
/// detail. The fallback chain treats every variant identically: skip the
/// slot until [`ProviderExhaustion::until`] passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ExhaustionReason {
    /// 429 / "rate limit" / "quota per minute". Server reported a reset
    /// time (or we synthesized one from `Retry-After`).
    RateLimited,
    /// 402 / "credit exhausted" / "out of funds". Operator must top up.
    QuotaExceeded,
    /// Operator-set budget cap (`config.toml: [budget]`) was crossed
    /// pre-dispatch. No provider was called — the metering layer is the
    /// caller that records this reason.
    BudgetExceeded,
    /// 401 / 403 / invalid API key / missing API key. Operator must rotate
    /// or supply credentials.
    AuthFailed,
}

impl ExhaustionReason {
    /// Stable lower-case label used in logs and metric tags. Deliberately
    /// kebab-case (no spaces) so it can be embedded in a Prometheus label
    /// or a structured log field without quoting.
    pub const fn as_metric_label(&self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::QuotaExceeded => "quota_exceeded",
            Self::BudgetExceeded => "budget_exceeded",
            Self::AuthFailed => "auth_failed",
        }
    }
}

/// One exhaustion record: why the slot is out, and when (if ever) it should
/// be retried automatically.
#[derive(Debug, Clone)]
pub struct ProviderExhaustion {
    /// Free-form provider identifier — typically the kernel's provider
    /// name (e.g. `"openai"`, `"groq"`, `"anthropic"`). The chain matches
    /// against the same `ChainEntry::provider_name`.
    pub provider_id: String,
    /// Why the slot is unavailable.
    pub reason: ExhaustionReason,
    /// `Instant` after which the slot should be retried. `None` means
    /// "indefinite" — the slot stays exhausted until
    /// [`ProviderExhaustionStore::clear_exhausted`] is called explicitly.
    /// In practice every caller passes `Some(_)`; `None` is provided for
    /// completeness so a worker that detects an unrecoverable failure can
    /// "park" the slot without an arbitrary timer.
    pub until: Option<Instant>,
}

impl ProviderExhaustion {
    /// `true` when [`Self::until`] is in the past — i.e. the slot is ready
    /// to be retried.
    pub fn is_expired(&self, now: Instant) -> bool {
        match self.until {
            Some(until) => now >= until,
            None => false,
        }
    }
}

/// Snapshot row for [`ProviderExhaustionStore::snapshot`]. Cheap to
/// serialize for diagnostic endpoints.
#[derive(Debug, Clone, Serialize)]
pub struct ExhaustionSnapshotRow {
    pub provider_id: String,
    pub reason: ExhaustionReason,
    /// Remaining duration until auto-clear, in milliseconds. `None` for
    /// indefinite entries (operator must clear).
    pub remaining_ms: Option<u128>,
}

/// In-memory exhaustion ledger. Cheap-clone (internal `Arc<DashMap>`),
/// safe to share across tasks, and hot-read / medium-write.
#[derive(Debug, Clone, Default)]
pub struct ProviderExhaustionStore {
    inner: Arc<DashMap<String, ProviderExhaustion>>,
}

impl ProviderExhaustionStore {
    /// Build an empty store. Equivalent to `Default::default()`; kept as a
    /// named constructor for call-site readability at construction time.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `provider_id` is exhausted for `reason` until `until`
    /// (or indefinitely when `until` is `None`).
    ///
    /// Marking the same provider twice replaces the previous entry — this
    /// is intentional. If a slot was rate-limited and then later fails
    /// auth, the most recent reason is the actionable one.
    pub fn mark_exhausted(
        &self,
        provider_id: impl Into<String>,
        reason: ExhaustionReason,
        until: Option<Instant>,
    ) {
        let provider_id = provider_id.into();
        // tracing::info! at INFO so this is visible at the default level
        // — exhaustion events are operator-actionable signal, not debug
        // noise. `target` is "metering" so existing tracing-subscriber
        // filters that route metering events to a dashboard pick this up
        // without additional wiring.
        tracing::info!(
            target: "metering",
            event = "provider_exhaustion_set",
            provider = %provider_id,
            reason = reason.as_metric_label(),
            "provider marked exhausted in fallback chain"
        );
        self.inner.insert(
            provider_id.clone(),
            ProviderExhaustion {
                provider_id,
                reason,
                until,
            },
        );
    }

    /// Query whether `provider_id` is currently exhausted.
    ///
    /// Returns `Some(record)` when the slot should be skipped, `None`
    /// otherwise. **Side effect**: when an entry's `until` has passed, the
    /// entry is removed atomically and `None` is returned — so the next
    /// call to the same provider naturally re-attempts.
    pub fn is_exhausted(&self, provider_id: &str) -> Option<ProviderExhaustion> {
        let now = Instant::now();
        // Read first; only take a write lock if the record actually
        // expired. Hot path: read-only DashMap shard lock.
        let still_live = self.inner.get(provider_id).map(|entry| {
            if entry.is_expired(now) {
                None
            } else {
                Some(entry.clone())
            }
        });
        match still_live {
            Some(Some(rec)) => Some(rec),
            Some(None) => {
                // Expired — atomically remove and return None.
                // `remove_if` so a concurrent `mark_exhausted` that just
                // replaced the entry with a fresh `until` is NOT clobbered.
                self.inner.remove_if(provider_id, |_, v| v.is_expired(now));
                None
            }
            None => None,
        }
    }

    /// Convenience: increment the "skipped because exhausted" counter and
    /// return the underlying record. Use from the fallback chain when a
    /// pre-attempt check finds the slot exhausted; the counter is purely
    /// observational and never affects routing.
    pub fn record_skip(&self, provider_id: &str) -> Option<ProviderExhaustion> {
        let rec = self.is_exhausted(provider_id)?;
        tracing::info!(
            target: "metering",
            event = "provider_skipped_exhausted",
            provider = %rec.provider_id,
            reason = rec.reason.as_metric_label(),
            "fallback chain skipping exhausted provider"
        );
        Some(rec)
    }

    /// Explicitly remove the exhaustion entry for `provider_id`, if any.
    /// Used by test fixtures and by the CLI / admin endpoint that lets an
    /// operator force-retry a slot ahead of its scheduled reset time.
    pub fn clear_exhausted(&self, provider_id: &str) {
        self.inner.remove(provider_id);
    }

    /// Snapshot of the current ledger, ordered by `provider_id` ascending
    /// so any stringified output is deterministic across processes
    /// (#3298). Cheap — copies the map under the hot-read shard locks and
    /// drops them before sorting.
    pub fn snapshot(&self) -> Vec<ExhaustionSnapshotRow> {
        let now = Instant::now();
        // BTreeMap keyed by provider_id so the resulting Vec is sorted.
        let mut sorted: BTreeMap<String, ExhaustionSnapshotRow> = BTreeMap::new();
        for entry in self.inner.iter() {
            // Skip rows whose timer has already elapsed — `is_exhausted`
            // would have cleaned these up on the next query anyway, but a
            // diagnostic snapshot should not show stale entries.
            if entry.is_expired(now) {
                continue;
            }
            let remaining_ms = entry
                .until
                .map(|u| u.saturating_duration_since(now).as_millis());
            sorted.insert(
                entry.provider_id.clone(),
                ExhaustionSnapshotRow {
                    provider_id: entry.provider_id.clone(),
                    reason: entry.reason,
                    remaining_ms,
                },
            );
        }
        sorted.into_values().collect()
    }

    /// Number of currently-live (non-expired) entries. Cheap diagnostic.
    pub fn live_count(&self) -> usize {
        let now = Instant::now();
        self.inner.iter().filter(|e| !e.is_expired(now)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_then_is_exhausted_returns_record() {
        let store = ProviderExhaustionStore::new();
        let until = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("openai", ExhaustionReason::RateLimited, Some(until));

        let rec = store.is_exhausted("openai").expect("just marked");
        assert_eq!(rec.provider_id, "openai");
        assert_eq!(rec.reason, ExhaustionReason::RateLimited);
        assert_eq!(rec.until, Some(until));
    }

    #[test]
    fn is_exhausted_returns_none_for_unmarked_provider() {
        let store = ProviderExhaustionStore::new();
        assert!(store.is_exhausted("never-seen").is_none());
    }

    #[test]
    fn entry_auto_clears_after_until_passes() {
        let store = ProviderExhaustionStore::new();
        // until in the past: should auto-clear on read.
        let past = Instant::now() - Duration::from_secs(1);
        store.mark_exhausted("groq", ExhaustionReason::QuotaExceeded, Some(past));

        assert!(
            store.is_exhausted("groq").is_none(),
            "expired entry must auto-clear"
        );
        // Subsequent read still returns None; the entry has been removed.
        assert!(store.is_exhausted("groq").is_none());
        assert_eq!(store.live_count(), 0);
    }

    #[test]
    fn indefinite_entry_does_not_auto_clear() {
        let store = ProviderExhaustionStore::new();
        store.mark_exhausted("anthropic", ExhaustionReason::AuthFailed, None);
        // No timer — only an explicit clear should remove it.
        assert!(store.is_exhausted("anthropic").is_some());
        assert!(store.is_exhausted("anthropic").is_some());
    }

    #[test]
    fn clear_exhausted_removes_entry() {
        let store = ProviderExhaustionStore::new();
        store.mark_exhausted(
            "openai",
            ExhaustionReason::BudgetExceeded,
            Some(Instant::now() + Duration::from_secs(60)),
        );
        assert!(store.is_exhausted("openai").is_some());

        store.clear_exhausted("openai");
        assert!(store.is_exhausted("openai").is_none());
    }

    #[test]
    fn mark_replaces_previous_reason() {
        let store = ProviderExhaustionStore::new();
        let in_a_min = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("openai", ExhaustionReason::RateLimited, Some(in_a_min));
        store.mark_exhausted("openai", ExhaustionReason::AuthFailed, Some(in_a_min));

        let rec = store.is_exhausted("openai").unwrap();
        assert_eq!(
            rec.reason,
            ExhaustionReason::AuthFailed,
            "most recent reason must win"
        );
    }

    #[test]
    fn snapshot_is_sorted_by_provider_id() {
        let store = ProviderExhaustionStore::new();
        let until = Instant::now() + Duration::from_secs(60);
        // Insert in non-alphabetical order on purpose.
        store.mark_exhausted("openai", ExhaustionReason::RateLimited, Some(until));
        store.mark_exhausted("anthropic", ExhaustionReason::AuthFailed, Some(until));
        store.mark_exhausted("groq", ExhaustionReason::QuotaExceeded, Some(until));

        let rows = store.snapshot();
        let ids: Vec<&str> = rows.iter().map(|r| r.provider_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["anthropic", "groq", "openai"],
            "snapshot must be sorted by provider_id"
        );
    }

    #[test]
    fn snapshot_excludes_expired_entries() {
        let store = ProviderExhaustionStore::new();
        let past = Instant::now() - Duration::from_secs(1);
        let future = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("expired", ExhaustionReason::RateLimited, Some(past));
        store.mark_exhausted("live", ExhaustionReason::AuthFailed, Some(future));

        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "live");
    }

    #[test]
    fn record_skip_returns_record_when_exhausted() {
        let store = ProviderExhaustionStore::new();
        let until = Instant::now() + Duration::from_secs(60);
        store.mark_exhausted("groq", ExhaustionReason::RateLimited, Some(until));

        let rec = store.record_skip("groq").expect("should be exhausted");
        assert_eq!(rec.provider_id, "groq");
    }

    #[test]
    fn record_skip_returns_none_when_not_exhausted() {
        let store = ProviderExhaustionStore::new();
        assert!(store.record_skip("openai").is_none());
    }

    #[test]
    fn store_clone_shares_underlying_state() {
        let store_a = ProviderExhaustionStore::new();
        let store_b = store_a.clone();
        let until = Instant::now() + Duration::from_secs(60);
        store_a.mark_exhausted("openai", ExhaustionReason::RateLimited, Some(until));

        assert!(
            store_b.is_exhausted("openai").is_some(),
            "Clone must share the same DashMap"
        );
    }

    #[test]
    fn live_count_reflects_active_entries() {
        let store = ProviderExhaustionStore::new();
        let future = Instant::now() + Duration::from_secs(60);
        let past = Instant::now() - Duration::from_secs(1);
        store.mark_exhausted("a", ExhaustionReason::RateLimited, Some(future));
        store.mark_exhausted("b", ExhaustionReason::AuthFailed, Some(past));
        store.mark_exhausted("c", ExhaustionReason::BudgetExceeded, None);
        // a (live) + c (indefinite, not expired) = 2
        assert_eq!(store.live_count(), 2);
    }
}
