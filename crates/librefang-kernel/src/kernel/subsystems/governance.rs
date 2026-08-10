//! Governance subsystem — approvals, lifecycle hooks, and the
//! idempotency guards for kernel-managed sweeper tasks.
//!
//! Bundles approval enforcement (`approval_manager`), the in-process
//! `HookRegistry` for plugin lifecycle hooks, the file-system based
//! `ExternalHookSystem`, and two `AtomicBool` flags that gate
//! singleton background sweepers (approval expiry, task-board stuck
//! tasks). Inner names are kept verbatim so the migration is
//! mechanical.

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_runtime::hooks::HookRegistry;
use librefang_types::agent::AgentId;

use crate::approval::ApprovalManager;
use crate::hooks::ExternalHookSystem;

/// Per-assignee bookkeeping for the Task Board reconcile wake (#6728).
///
/// The reconcile is level-triggered, so without memory it would re-wake an agent on every tick for as long as a task stays `pending` — and a task an agent cannot act on (a failing provider, a missing `task_claim` capability) stays pending indefinitely.
/// That turns a delivery guarantee into an unbounded spend.
///
/// Keyed on the assignee rather than the task because the wake prompt is drain-style: one wake covers every task addressed to that agent.
#[derive(Debug, Clone)]
pub(crate) struct AssigneeWakeState {
    /// When the reconcile last woke this agent.
    pub(crate) last_wake: DateTime<Utc>,
    /// Consecutive wakes after which the agent's pending set did not shrink.
    /// Drives the backoff exponent; reset the moment anything is picked up.
    pub(crate) ineffective_wakes: u32,
    /// The pending task ids this agent was last woken for. Compared against
    /// the current set to tell "made progress" from "still stuck", which a
    /// count alone cannot do when one task completes while another arrives.
    pub(crate) woken_for: BTreeSet<String>,
}

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
    /// Backoff state for the Task Board reconcile wake, keyed by assignee (issue #6728).
    /// Lives here rather than on `LibreFangKernel` so the god-struct stays where #3565 left it; it is sweeper state and the sweeper's other guard is already here.
    ///
    /// Deliberately in-memory: losing it on restart costs one extra wake per backlogged agent, which is cheaper than a schema migration and cannot lose work in either direction.
    pub(crate) assignee_wake_state: DashMap<AgentId, AssigneeWakeState>,
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
            assignee_wake_state: DashMap::new(),
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
