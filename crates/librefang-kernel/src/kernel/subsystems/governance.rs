//! Governance subsystem — approvals, lifecycle hooks, and the
//! idempotency guards for kernel-managed sweeper tasks.
//!
//! Bundles approval enforcement (`approval_manager`), the in-process
//! `HookRegistry` for plugin lifecycle hooks, the file-system based
//! `ExternalHookSystem`, and two `AtomicBool` flags that gate
//! singleton background sweepers (approval expiry, task-board stuck
//! tasks). Inner names are kept verbatim so the migration is
//! mechanical.

use std::sync::atomic::AtomicBool;

use librefang_runtime::hooks::HookRegistry;

use crate::approval::ApprovalManager;
use crate::hooks::ExternalHookSystem;

/// Approval + hooks + sweeper guard cluster — see module docs.
pub struct GovernanceSubsystem {
    /// Execution approval manager.
    pub(crate) approval_manager: ApprovalManager,
    /// Plugin lifecycle hook registry.
    pub(crate) hooks: HookRegistry,
    /// External file-system lifecycle hook system (HOOK.yaml based,
    /// fire-and-forget).
    pub(crate) external_hooks: ExternalHookSystem,
    /// Idempotency guard for the approval expiry sweep.
    pub(crate) approval_sweep_started: AtomicBool,
    /// Idempotency guard for the task-board stuck-task sweeper
    /// (issue #2923).
    pub(crate) task_board_sweep_started: AtomicBool,
}

impl GovernanceSubsystem {
    pub(crate) fn new(
        approval_manager: ApprovalManager,
        external_hooks: ExternalHookSystem,
    ) -> Self {
        Self {
            approval_manager,
            hooks: HookRegistry::new(),
            external_hooks,
            approval_sweep_started: AtomicBool::new(false),
            task_board_sweep_started: AtomicBool::new(false),
        }
    }

    /// Approval enforcement manager.
    #[inline]
    pub fn approvals(&self) -> &ApprovalManager {
        &self.approval_manager
    }

    /// In-process plugin lifecycle hook registry.
    #[inline]
    pub fn hook_registry(&self) -> &HookRegistry {
        &self.hooks
    }
}
