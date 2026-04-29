//! Storage backend trait shells for the kernel crate.
//!
//! Phase 4 of the `surrealdb-storage-swap` plan introduced these trait
//! surfaces so that Phase 6 could swap the existing rusqlite-backed
//! `totp_lockout` table for a SurrealDB-backed implementation behind the
//! `surreal-backend` feature.
//!
//! Phase 6 added [`TotpLockoutBackend`] — a backend-agnostic trait
//! describing the persistence primitives used by
//! [`crate::approval::ApprovalManager`] (load/save/clear). The Surreal
//! implementation lives in [`crate::backends::surreal_approval`] and
//! mirrors the rusqlite logic in `approval.rs::persist_totp_lockout_*`.
//!
//! The high-level [`ApprovalStore`] keeps its existing narrow surface
//! so callers (HTTP routes, plugins) continue to talk to the manager
//! directly; only the persistence layer is being swapped.

use thiserror::Error;

/// Persistent store for the per-sender TOTP lockout window.
///
/// `ApprovalManager` currently composes its own in-memory map over a tiny
/// rusqlite table; once Phase 7 wires the SurrealDB equivalent through to
/// the manager the manager will hold a `Box<dyn ApprovalStore>` instead.
pub trait ApprovalStore: Send + Sync {
    /// Whether `sender_id` is currently locked out from TOTP attempts.
    fn is_totp_locked_out(&self, sender_id: &str) -> bool;

    /// Record a single TOTP verification failure for `sender_id`.
    fn record_totp_failure(&self, sender_id: &str);
}

impl ApprovalStore for crate::approval::ApprovalManager {
    fn is_totp_locked_out(&self, sender_id: &str) -> bool {
        crate::approval::ApprovalManager::is_totp_locked_out(self, sender_id)
    }

    fn record_totp_failure(&self, sender_id: &str) {
        let _ = crate::approval::ApprovalManager::record_totp_failure(self, sender_id);
    }
}

/// Backend-agnostic error returned from [`TotpLockoutBackend`].
///
/// Wraps backend-specific errors (rusqlite, SurrealDB, …) so callers
/// only have to pattern-match on a single variant.
#[derive(Debug, Error)]
pub enum TotpLockoutError {
    /// The backend rejected the call (transport, query, decode, …).
    #[error("totp lockout backend error: {0}")]
    Backend(String),
}

/// Convenience alias.
pub type TotpLockoutResult<T> = Result<T, TotpLockoutError>;

/// One row of the TOTP lockout table as it travels in/out of a backend.
///
/// `locked_at_unix` is `None` when the sender has accumulated failures
/// but has not yet crossed the lockout threshold; once the threshold is
/// crossed the manager stamps `Some(now_in_unix_seconds)` so the window
/// can be reconstructed across daemon restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpLockoutRow {
    /// Sender id (channel-scoped, e.g. `"slack:U12345"`).
    pub sender_id: String,
    /// Number of consecutive failures.
    pub failures: u32,
    /// Unix-seconds timestamp when the lockout window started.
    pub locked_at_unix: Option<i64>,
}

/// Persistence primitives for the TOTP lockout table.
///
/// Implementations only have to know how to load, upsert, and delete
/// individual rows — the lockout *policy* (max failures, window length,
/// decay across restarts) lives entirely inside
/// [`crate::approval::ApprovalManager`] and is reused unchanged whether
/// the persistence backend is rusqlite or SurrealDB.
pub trait TotpLockoutBackend: Send + Sync {
    /// Load every persisted lockout row.
    ///
    /// Manager code is expected to discard rows whose lockout window has
    /// already elapsed; this method does not perform any expiry filtering
    /// on its own.
    fn load_all(&self) -> TotpLockoutResult<Vec<TotpLockoutRow>>;

    /// Upsert a single lockout row.
    fn upsert(&self, row: &TotpLockoutRow) -> TotpLockoutResult<()>;

    /// Delete the lockout row for `sender_id`. Missing rows are silently
    /// ignored.
    fn clear(&self, sender_id: &str) -> TotpLockoutResult<()>;
}

/// Backend for TOTP replay-prevention storage (upstream #3952).
///
/// Stores SHA-256 hashes of recently-used TOTP codes so that a code cannot
/// be reused within the same 30-second window (or an adjacent window).
/// Entries older than 120 seconds are pruned on every successful verification.
pub trait TotpUsedCodesBackend: Send + Sync + 'static {
    /// Returns `true` if `code_hash` appears in the table with `used_at >= window_start_secs`.
    fn is_code_used(&self, code_hash: &str, window_start_secs: i64) -> bool;
    /// Upsert: insert or update the hash with the given timestamp.
    fn mark_code_used(&self, code_hash: &str, used_at_secs: i64);
    /// Delete all entries with `used_at < cutoff_secs`.
    fn prune_old_codes(&self, cutoff_secs: i64);
}
