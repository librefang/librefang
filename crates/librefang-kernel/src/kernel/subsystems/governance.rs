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

/// Focused approval + hooks API.
pub trait GovernanceSubsystemApi: Send + Sync {
    /// Approval enforcement manager.
    fn approvals(&self) -> &ApprovalManager;
    /// In-process plugin lifecycle hook registry.
    fn hook_registry(&self) -> &HookRegistry;
}

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
}

impl GovernanceSubsystemApi for GovernanceSubsystem {
    #[inline]
    fn approvals(&self) -> &ApprovalManager {
        &self.approval_manager
    }

    #[inline]
    fn hook_registry(&self) -> &HookRegistry {
        &self.hooks
    }
}
