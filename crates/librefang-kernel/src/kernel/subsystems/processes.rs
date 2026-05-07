//! Process subsystem — long-lived child processes spawned by tool calls.
//!
//! Bundles the two process registries that move together: the persistent
//! `ProcessManager` (interactive REPLs / dev servers held open across
//! turns) and the fire-and-forget `ProcessRegistry` (`shell_exec`
//! background tasks with rolling output buffers). Both are `Arc` so they
//! can be cloned cheaply into spawned tasks.

use std::sync::Arc;

use librefang_runtime::process_manager::ProcessManager;
use librefang_runtime::process_registry::ProcessRegistry;

/// Focused process-management API.
pub trait ProcessSubsystemApi: Send + Sync {
    /// Persistent process manager handle.
    fn process_manager_ref(&self) -> &Arc<ProcessManager>;
    /// Background process registry handle.
    fn process_registry_ref(&self) -> &Arc<ProcessRegistry>;
}

/// Process management cluster — see module docs.
pub struct ProcessSubsystem {
    /// Persistent process manager for interactive sessions (REPLs, servers).
    pub(crate) manager: Arc<ProcessManager>,
    /// Background process registry — tracks fire-and-forget processes
    /// spawned by `shell_exec` with a rolling 200 KB output buffer per
    /// process.
    pub(crate) registry: Arc<ProcessRegistry>,
}

impl ProcessSubsystem {
    pub(crate) fn new(manager: Arc<ProcessManager>, registry: Arc<ProcessRegistry>) -> Self {
        Self { manager, registry }
    }
}

impl ProcessSubsystemApi for ProcessSubsystem {
    #[inline]
    fn process_manager_ref(&self) -> &Arc<ProcessManager> {
        &self.manager
    }

    #[inline]
    fn process_registry_ref(&self) -> &Arc<ProcessRegistry> {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    //! Boundary tests: prove the focused trait is usable through `&dyn`,
    //! object-safe, and `Send + Sync` — exercising the trait shape so
    //! drift in lifetimes / bounds breaks here, not at the first real
    //! caller. Mirror this pattern when adding a new subsystem trait.
    use super::*;

    /// Test-only stub. Implements the trait without going through the
    /// real subsystem, so we know the trait shape is implementable
    /// independently of `LibreFangKernel` boot.
    struct StubProcesses {
        manager: Arc<ProcessManager>,
        registry: Arc<ProcessRegistry>,
    }

    impl ProcessSubsystemApi for StubProcesses {
        fn process_manager_ref(&self) -> &Arc<ProcessManager> {
            &self.manager
        }
        fn process_registry_ref(&self) -> &Arc<ProcessRegistry> {
            &self.registry
        }
    }

    fn read_via_trait(api: &dyn ProcessSubsystemApi) -> (usize, usize) {
        // Strong-count probes are enough — the contract is just "the
        // trait exposes the underlying Arc handles unchanged".
        (
            Arc::strong_count(api.process_manager_ref()),
            Arc::strong_count(api.process_registry_ref()),
        )
    }

    #[test]
    fn process_subsystem_routes_through_focused_trait() {
        let manager = Arc::new(ProcessManager::new(5));
        let registry = Arc::new(ProcessRegistry::new());
        let sub = ProcessSubsystem::new(Arc::clone(&manager), Arc::clone(&registry));

        // Real subsystem: trait dispatch returns the same Arc the
        // inherent constructor stored.
        let (mgr_count, reg_count) = read_via_trait(&sub);
        assert!(
            mgr_count >= 2,
            "manager Arc must be shared, got {mgr_count}"
        );
        assert!(
            reg_count >= 2,
            "registry Arc must be shared, got {reg_count}"
        );
        assert!(Arc::ptr_eq(sub.process_manager_ref(), &manager));
        assert!(Arc::ptr_eq(sub.process_registry_ref(), &registry));

        // Stub: same trait, no kernel scaffolding — proves the trait is
        // implementable by a future mock.
        let stub = StubProcesses {
            manager: Arc::clone(&manager),
            registry: Arc::clone(&registry),
        };
        let _ = read_via_trait(&stub);
    }

    #[test]
    fn process_trait_is_object_safe_and_thread_safe() {
        // Compile-time assertions: the trait must be object-safe (so
        // `&dyn` works) and `Send + Sync` (so kernel — held inside
        // `Arc<LibreFangKernel>` — can travel through `tokio::spawn`).
        fn assert_dyn_safe(_: &dyn ProcessSubsystemApi) {}
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ProcessSubsystemApi>();
        let manager = Arc::new(ProcessManager::new(1));
        let registry = Arc::new(ProcessRegistry::new());
        let sub = ProcessSubsystem::new(manager, registry);
        assert_dyn_safe(&sub);
    }
}
