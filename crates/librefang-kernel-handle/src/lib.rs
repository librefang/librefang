//! Role traits for kernel operations needed by the agent runtime.
//!
//! Historically this crate exposed a single 50+ method `KernelHandle`
//! god-trait (issue #3746). It is now split into role traits — `AgentControl`,
//! `MemoryAccess`, `TaskQueue`, `EventBus`, `KnowledgeGraph`, `CronControl`,
//! `ApprovalGate`, `HandsControl`, `A2ARegistry`, `ChannelSender`,
//! `PromptStore`, `WorkflowRunner`, `GoalControl`, `ToolPolicy` — so that
//!
//! 1. each role trait lives in its own module file instead of one giant
//!    trait file mixing every kernel domain,
//! 2. callers can express narrower bounds (e.g. `T: ApprovalGate`) instead of
//!    pulling the whole kernel surface in,
//! 3. test stubs/mocks group their fakes by capability and a missing
//!    capability is a compile error in the role-trait impl, not a silent
//!    `Err("not available")` at first runtime call.
//!
//! `KernelHandle` is preserved as a *supertrait alias* requiring all role
//! traits, with a blanket impl, so existing `Arc<dyn KernelHandle>` call
//! sites (117 of them at split time) keep working unchanged. Future PRs can
//! narrow individual sites without further churn here.
//!
//! ### Default impls
//!
//! Defaults that hide a missing capability behind a runtime
//! `Err("X not available")` are preserved as-is for now to keep this PR a
//! pure structural refactor (zero behavior change). They are gathered onto
//! the role trait that owns them, so a follow-up PR can tighten each role's
//! contract independently rather than having to land 30+ default removals
//! atomically.

// ============================================================================
// Typed kernel-op errors (#3541)
// ============================================================================
//
// `KernelOpError` is a re-export of `librefang_types::error::LibreFangError`
// — the canonical structured business-error enum that already existed in
// the workspace before this migration. The trait surface uses the alias
// for two reasons:
//
//   1. Callers that crossed the runtime↔kernel seam used to get
//      `Result<_, String>`, throwing away the variant info and forcing
//      substring-matching back to a category. The alias resolves that
//      directly: `match err { LibreFangError::AgentNotFound(_) => 404,
//      CapabilityDenied(_) => 403, Unavailable(_) => 503, … }`.
//   2. Reusing the existing enum (rather than introducing a parallel
//      "kernel handle error") keeps every layer (runtime, kernel, api)
//      working with the same vocabulary, so converting between layers is
//      a no-op rather than a `match`-and-rewrap dance.
//
// Use [`KernelResult<T>`] in new role-trait method signatures so the
// shape `Result<T, LibreFangError>` is consistent and self-documenting.
pub use librefang_types::error::LibreFangError as KernelOpError;

/// Canonical result type for `KernelHandle` role-trait methods (#3541).
/// Use this in new method signatures rather than respelling
/// `Result<T, KernelOpError>` each time.
pub type KernelResult<T> = Result<T, KernelOpError>;

// ============================================================================
// Role-trait modules (#3746). One file per capability domain; the god-trait
// was split into role traits earlier, this splits the file to match. Each
// module's traits are re-exported below so existing
// `librefang_kernel_handle::AgentControl` paths keep resolving unchanged.
// ============================================================================

mod a2a_registry;
mod acp_fs;
mod acp_terminal;
mod agent_control;
mod api_auth;
mod approval_gate;
mod catalog_query;
mod channel_sender;
mod cron_control;
mod event_bus;
mod goal_control;
mod hands_control;
mod knowledge_graph;
mod memory_access;
mod prompt_store;
mod session_writer;
mod task_queue;
mod tool_policy;
mod wiki_access;
mod workflow_runner;

pub use a2a_registry::*;
pub use acp_fs::*;
pub use acp_terminal::*;
pub use agent_control::*;
pub use api_auth::*;
pub use approval_gate::*;
pub use catalog_query::*;
pub use channel_sender::*;
pub use cron_control::*;
pub use event_bus::*;
pub use goal_control::*;
pub use hands_control::*;
pub use knowledge_graph::*;
pub use memory_access::*;
pub use prompt_store::*;
pub use session_writer::*;
pub use task_queue::*;
pub use tool_policy::*;
pub use wiki_access::*;
pub use workflow_runner::*;

// ============================================================================
// KernelHandle — supertrait alias of all 20 role traits.
//
// Existing call sites take `Arc<dyn KernelHandle>`; that keeps working because
// any type that impls every role trait automatically gets `KernelHandle` via
// the blanket impl below. To narrow a new call site, take only the role bounds
// you need (e.g. `fn foo<T: ApprovalGate + Send + Sync>(h: &T)`).
// ============================================================================

pub trait KernelHandle:
    AgentControl
    + MemoryAccess
    + WikiAccess
    + TaskQueue
    + EventBus
    + KnowledgeGraph
    + CronControl
    + ApprovalGate
    + HandsControl
    + A2ARegistry
    + ChannelSender
    + PromptStore
    + WorkflowRunner
    + GoalControl
    + ToolPolicy
    + ApiAuth
    + SessionWriter
    + AcpFsBridge
    + AcpTerminalBridge
    + CatalogQuery
    + Send
    + Sync
{
}

impl<T> KernelHandle for T where
    T: AgentControl
        + MemoryAccess
        + WikiAccess
        + TaskQueue
        + EventBus
        + KnowledgeGraph
        + CronControl
        + ApprovalGate
        + HandsControl
        + A2ARegistry
        + ChannelSender
        + PromptStore
        + WorkflowRunner
        + GoalControl
        + ToolPolicy
        + ApiAuth
        + SessionWriter
        + AcpFsBridge
        + AcpTerminalBridge
        + CatalogQuery
        + Send
        + Sync
        + ?Sized
{
}

/// Prelude — glob-import this to bring `KernelHandle` plus every role trait
/// into scope so that method calls like `kernel.send_channel_message(...)`
/// resolve. Replaces the pre-#3746 single-trait import pattern.
pub mod prelude {
    pub use super::{
        A2ARegistry, AcpFsBridge, AcpFsClient, AcpTerminalBridge, AcpTerminalClient,
        AcpTerminalRunResult, AgentControl, AgentInfo, ApiAuth, ApiAuthSnapshot,
        ApiUserConfigSnapshot, ApprovalGate, CatalogQuery, ChannelSender, CronControl,
        DashboardRawConfig, EventBus, GoalControl, HandsControl, KernelHandle, KnowledgeGraph,
        MemoryAccess, PromptStore, SessionWriter, StepOutputSummary, TaskQueue, ToolPolicy,
        WikiAccess, WorkflowDescription, WorkflowInputParam, WorkflowRunSummary, WorkflowRunner,
        WorkflowSummary,
    };
}

#[cfg(test)]
mod tests;
