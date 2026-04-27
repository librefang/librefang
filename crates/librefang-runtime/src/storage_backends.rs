//! Storage backend trait shells for the runtime crate.
//!
//! Phase 4 of the `surrealdb-storage-swap` plan introduces these traits so
//! the kernel and API layers can depend on backend-agnostic interfaces. The
//! existing rusqlite-backed [`crate::audit::AuditLog`] and
//! [`crate::trace_store::TraceStore`] implement them today; Phases 5–6 will
//! add SurrealDB equivalents and gate the rusqlite impls behind
//! `#[cfg(feature = "sqlite-backend")]` once parity is reached.
//!
//! The trait surfaces are deliberately narrow: only the always-needed
//! methods the kernel and API call through the existing handles. Wider
//! per-feature surfaces (trace replay, breaker queries, anchor management)
//! continue to live as inherent methods on the concrete types until the
//! Surreal implementations land.

use crate::audit::{AuditAction, AuditEntry, AuditLog, TrimReport};
use crate::context_engine::HookTrace;
#[cfg(feature = "sqlite-backend")]
use crate::trace_store::TraceStore;
use std::collections::HashMap;
use thiserror::Error;

/// Backend-agnostic error returned from [`TraceBackend`] operations.
///
/// Phase 6 of the `surrealdb-storage-swap` plan removed the leaky
/// `rusqlite::Result` from this trait so the SurrealDB implementation can
/// implement it without dragging the rusqlite types into its build.
#[derive(Debug, Error)]
pub enum TraceBackendError {
    /// The backend rejected the call (transport, query, decode, etc.).
    #[error("trace backend error: {0}")]
    Backend(String),
}

/// Convenience alias.
pub type TraceBackendResult<T> = Result<T, TraceBackendError>;

/// Append-only Merkle-chained audit trail surface.
///
/// The runtime ships exactly one implementor today —
/// [`crate::audit::AuditLog`]. The trait exists so the kernel can hold a
/// `Box<dyn AuditStore>` once the Surreal implementation lands in Phase 6.
pub trait AuditStore: Send + Sync {
    /// Append a new entry to the chain.
    fn record(&self, agent_id: &str, action: AuditAction, detail: &str, outcome: &str);

    /// Append a new entry with optional user / channel attribution (RBAC M1).
    ///
    /// Defaults to `record()` so existing impls compile without changes.
    fn record_with_context(
        &self,
        agent_id: &str,
        action: AuditAction,
        detail: &str,
        outcome: &str,
        user_id: Option<librefang_types::agent::UserId>,
        channel: Option<String>,
    ) {
        let _ = (user_id, channel);
        self.record(agent_id, action, detail, outcome);
    }

    /// Verify the integrity of every entry in the chain.
    fn verify_integrity(&self) -> Result<(), String>;

    /// Return the SHA-256 hash of the most recent entry, or all-zeros if empty.
    fn tip_hash(&self) -> String;

    /// Number of entries currently retained.
    fn len(&self) -> usize;

    /// Whether the chain currently contains zero entries.
    fn is_empty(&self) -> bool;

    /// Most recent `n` entries in chronological order.
    fn recent(&self, n: usize) -> Vec<AuditEntry>;

    /// Drop entries older than `retention_days`. Returns the number pruned.
    fn prune(&self, retention_days: u32) -> usize;

    /// Apply a full [`AuditRetentionConfig`] policy and return a [`TrimReport`].
    ///
    /// Defaults to a simple `prune()` using the minimum per-action retention
    /// so existing impls compile without changes.
    fn trim(
        &self,
        config: &librefang_types::config::AuditRetentionConfig,
        now: chrono::DateTime<chrono::Utc>,
    ) -> TrimReport {
        let days = config
            .retention_days_by_action
            .values()
            .copied()
            .min()
            .unwrap_or(u32::MAX);
        if days == u32::MAX {
            return TrimReport::default();
        }
        let _ = now;
        let pruned = self.prune(days);
        TrimReport {
            total_dropped: pruned,
            ..Default::default()
        }
    }
}

impl AuditStore for AuditLog {
    fn record(&self, agent_id: &str, action: AuditAction, detail: &str, outcome: &str) {
        AuditLog::record(self, agent_id, action, detail, outcome);
    }

    fn record_with_context(
        &self,
        agent_id: &str,
        action: AuditAction,
        detail: &str,
        outcome: &str,
        user_id: Option<librefang_types::agent::UserId>,
        channel: Option<String>,
    ) {
        AuditLog::record_with_context(self, agent_id, action, detail, outcome, user_id, channel);
    }

    fn verify_integrity(&self) -> Result<(), String> {
        AuditLog::verify_integrity(self)
    }

    fn tip_hash(&self) -> String {
        AuditLog::tip_hash(self)
    }

    fn len(&self) -> usize {
        AuditLog::len(self)
    }

    fn is_empty(&self) -> bool {
        AuditLog::is_empty(self)
    }

    fn recent(&self, n: usize) -> Vec<AuditEntry> {
        AuditLog::recent(self, n)
    }

    fn prune(&self, retention_days: u32) -> usize {
        AuditLog::prune(self, retention_days)
    }

    fn trim(
        &self,
        config: &librefang_types::config::AuditRetentionConfig,
        now: chrono::DateTime<chrono::Utc>,
    ) -> TrimReport {
        AuditLog::trim(self, config, now)
    }
}

/// Persistent hook-trace + circuit-breaker store surface.
///
/// Mirrors the always-needed methods on [`crate::trace_store::TraceStore`].
/// The Surreal implementation in Phase 6 will land behind the
/// `surreal-backend` feature; the rusqlite implementation here will move
/// behind `sqlite-backend` once parity is reached.
pub trait TraceBackend: Send + Sync {
    /// Insert a single trace record for a plugin.
    fn insert(&self, plugin: &str, trace: &HookTrace);

    /// Load every persisted circuit-breaker state, keyed by `<plugin>::<hook>`.
    fn load_circuit_states(&self) -> TraceBackendResult<HashMap<String, (u32, Option<String>)>>;

    /// Persist (insert-or-update) a circuit-breaker state.
    fn save_circuit_state(
        &self,
        key: &str,
        failures: u32,
        opened_at: Option<&str>,
    ) -> TraceBackendResult<()>;

    /// Remove a circuit-breaker state (e.g. after recovery).
    fn delete_circuit_state(&self, key: &str) -> TraceBackendResult<()>;
}

#[cfg(feature = "sqlite-backend")]
impl TraceBackend for TraceStore {
    fn insert(&self, plugin: &str, trace: &HookTrace) {
        TraceStore::insert(self, plugin, trace);
    }

    fn load_circuit_states(&self) -> TraceBackendResult<HashMap<String, (u32, Option<String>)>> {
        TraceStore::load_circuit_states(self).map_err(|e| TraceBackendError::Backend(e.to_string()))
    }

    fn save_circuit_state(
        &self,
        key: &str,
        failures: u32,
        opened_at: Option<&str>,
    ) -> TraceBackendResult<()> {
        TraceStore::save_circuit_state(self, key, failures, opened_at)
            .map_err(|e| TraceBackendError::Backend(e.to_string()))
    }

    fn delete_circuit_state(&self, key: &str) -> TraceBackendResult<()> {
        TraceStore::delete_circuit_state(self, key)
            .map_err(|e| TraceBackendError::Backend(e.to_string()))
    }
}
