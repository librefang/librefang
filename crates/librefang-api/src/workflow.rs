//! Re-exports of kernel workflow types used by API routes.
//!
//! Issue #3744 (next slice): keep route modules from importing
//! `librefang_kernel::workflow::*` directly. Mirrors the same pattern
//! already established for triggers in `crate::triggers`.

pub use librefang_kernel::workflow::{
    BranchArm, CancelRunError, ErrorMode, PauseRunError, ResumeRunError, StepAgent, StepMode,
    Workflow, WorkflowId, WorkflowRun, WorkflowRunId, WorkflowRunState, WorkflowStep,
};
