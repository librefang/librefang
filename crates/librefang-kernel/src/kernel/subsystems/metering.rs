//! Metering subsystem — audit trail, cost metering, and budget snapshot.
//!
//! Bundles the three fields that move together: every LLM call records
//! both an audit entry and a metering tick, and budget enforcement reads
//! the current `BudgetConfig` snapshot. Owning them on one struct removes
//! three slots from `LibreFangKernel` and keeps related boot-time wiring
//! co-located.

use std::sync::Arc;

use arc_swap::ArcSwap;
use librefang_runtime::audit::AuditLog;
use librefang_types::config::BudgetConfig;

use crate::metering::MeteringEngine;

/// Cost / audit / budget cluster — see module docs.
pub struct MeteringSubsystem {
    /// Merkle hash chain audit trail.
    pub(crate) audit_log: Arc<AuditLog>,
    /// Cost metering engine.
    pub(crate) engine: Arc<MeteringEngine>,
    /// Hot-reloadable budget configuration. Initialised from
    /// `KernelConfig.budget` at boot and mutated atomically via
    /// `LibreFangKernel::update_budget_config` from the API layer. Backed
    /// by `ArcSwap` so the LLM hot path (which reads it on every turn for
    /// budget enforcement) never parks a tokio worker thread on a
    /// blocking lock — see #3579.
    pub(crate) budget_config: ArcSwap<BudgetConfig>,
}

impl MeteringSubsystem {
    pub(crate) fn new(
        audit_log: Arc<AuditLog>,
        engine: Arc<MeteringEngine>,
        initial_budget: BudgetConfig,
    ) -> Self {
        Self {
            audit_log,
            engine,
            budget_config: ArcSwap::from_pointee(initial_budget),
        }
    }

    /// Audit log handle.
    #[inline]
    pub fn audit_log(&self) -> &Arc<AuditLog> {
        &self.audit_log
    }

    /// Metering engine handle.
    #[inline]
    pub fn engine(&self) -> &Arc<MeteringEngine> {
        &self.engine
    }

    /// Snapshot the current `BudgetConfig` (cheap clone of the
    /// `ArcSwap`-loaded value).
    #[inline]
    pub fn current_budget(&self) -> BudgetConfig {
        (*self.budget_config.load_full()).clone()
    }

    /// RCU-update the budget configuration. The closure receives a
    /// mutable copy; the result is atomically swapped in.
    pub fn update_budget(&self, f: impl Fn(&mut BudgetConfig)) {
        self.budget_config.rcu(|current| {
            let mut next = (**current).clone();
            f(&mut next);
            std::sync::Arc::new(next)
        });
    }
}
