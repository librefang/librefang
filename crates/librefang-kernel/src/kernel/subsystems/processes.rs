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
