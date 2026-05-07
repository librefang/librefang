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

/// Focused metering API — read-only accessors that the rest of the
/// kernel and the API layer need. Generic mutators (`update_budget`)
/// stay as inherent methods on `MeteringSubsystem` since trait methods
/// cannot accept `impl Fn` arguments.
pub trait MeteringSubsystemApi: Send + Sync {
    /// Audit log handle.
    fn audit_log(&self) -> &Arc<AuditLog>;
    /// Metering engine handle.
    fn metering_engine(&self) -> &Arc<MeteringEngine>;
    /// Snapshot the current `BudgetConfig`.
    fn current_budget(&self) -> BudgetConfig;
}

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

    /// RCU-update the budget configuration. Inherent (not on the
    /// trait) so callers can pass `impl Fn` directly.
    pub fn update_budget(&self, f: impl Fn(&mut BudgetConfig)) {
        self.budget_config.rcu(|current| {
            let mut next = (**current).clone();
            f(&mut next);
            std::sync::Arc::new(next)
        });
    }
}

impl MeteringSubsystemApi for MeteringSubsystem {
    #[inline]
    fn audit_log(&self) -> &Arc<AuditLog> {
        &self.audit_log
    }

    #[inline]
    fn metering_engine(&self) -> &Arc<MeteringEngine> {
        &self.engine
    }

    #[inline]
    fn current_budget(&self) -> BudgetConfig {
        (*self.budget_config.load_full()).clone()
    }
}

#[cfg(test)]
mod tests {
    //! Boundary tests for `MeteringSubsystemApi`. The trait returns
    //! both a borrowed `&Arc<...>` (`audit_log`, `metering_engine`) and
    //! an owned snapshot (`current_budget`), so this also exercises
    //! that mixed-shape contract — important because trait methods
    //! cannot return `impl Trait` and any future change to the return
    //! shape would have to break this test first.
    use super::*;
    use librefang_memory::usage::UsageStore;
    use librefang_memory::MemorySubstrate;

    fn read_budget_via_trait(api: &dyn MeteringSubsystemApi) -> BudgetConfig {
        api.current_budget()
    }

    #[test]
    fn metering_subsystem_routes_through_focused_trait() {
        // Build with the same wiring `boot.rs` uses: in-memory pool ⇒
        // `UsageStore` ⇒ `MeteringEngine`, plus a parameter-less
        // `AuditLog`.
        let substrate = MemorySubstrate::open_in_memory(0.01).expect("in-memory substrate");
        let engine = Arc::new(MeteringEngine::new(Arc::new(UsageStore::new(
            substrate.pool(),
        ))));
        let audit = Arc::new(AuditLog::new());
        let initial = BudgetConfig::default();
        let sub = MeteringSubsystem::new(Arc::clone(&audit), Arc::clone(&engine), initial.clone());

        // Borrowed-handle methods: trait dispatch returns the same Arc
        // the constructor stored — no hidden clone or rewrapping.
        assert!(Arc::ptr_eq(MeteringSubsystemApi::audit_log(&sub), &audit));
        assert!(Arc::ptr_eq(
            MeteringSubsystemApi::metering_engine(&sub),
            &engine
        ));

        // Owned-snapshot method: round-trips through `&dyn` cleanly.
        let snap = read_budget_via_trait(&sub);
        assert_eq!(snap.max_daily_usd, initial.max_daily_usd);

        // Inherent (non-trait) RCU mutator still works alongside the
        // trait-dispatched read path.
        sub.update_budget(|c| c.max_daily_usd = 42.0);
        let after = read_budget_via_trait(&sub);
        assert_eq!(after.max_daily_usd, 42.0);
    }

    #[test]
    fn metering_trait_is_object_safe_and_thread_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn MeteringSubsystemApi>();
    }
}
