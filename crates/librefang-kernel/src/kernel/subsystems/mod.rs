//! Field-owning subsystems split out of `LibreFangKernel` (refs #3565).
//!
//! Each subsystem is a thin struct that bundles a previously-flat cluster
//! of `LibreFangKernel` fields. Method bodies still live on
//! `LibreFangKernel` and reach into subsystems via `self.<sub>.<field>`.
//! That keeps the ~600 internal call-sites mechanical while shrinking the
//! kernel struct surface from ~70 fields to a dozen subsystem handles.
//!
//! ## Status
//!
//! - **Field extraction (this PR)**: 13 subsystems, mechanical rename.
//! - **Focused per-subsystem traits (this PR, follow-ups #2 + #3)**: each
//!   subsystem exposes an `*SubsystemApi` trait that `LibreFangKernel`
//!   also implements via [`super::subsystem_forwards`]. New callers can
//!   bind `&dyn MeteringSubsystemApi` instead of dragging in the entire
//!   `KernelApi` surface; existing `Arc<dyn KernelApi>` flows are
//!   unchanged. Trait shapes are exercised by `#[cfg(test)]` boundary
//!   tests next to each impl (see `processes::tests`,
//!   `metering::tests`).
//! - **Method-body migration**: not done; method bodies still live on
//!   `LibreFangKernel` and read through `self.<sub>.<field>`. Moving
//!   bodies into per-subsystem `impl` blocks is the next refactor.
//!
//! ## Shutdown ordering invariant
//!
//! Field reorganization could in principle change the implicit Rust
//! drop order, but the kernel **does not rely on drop order for graceful
//! shutdown**. Shutdown is explicit and broadcast-based — see
//! [`super::LibreFangKernel::shutdown`]:
//!
//! 1. `shutdown_tx.send(true)` signals every subscriber (cron tick,
//!    background sweeps, approval expiry, session-stream-hub GC,
//!    auto-dream scheduler, inbox watcher) to exit their loops.
//! 2. `agents.supervisor.shutdown()` stops accepting new work.
//! 3. `workflows.engine.drain_on_shutdown()` pauses any `Running` /
//!    `Pending` workflow runs and persists them with a resume token.
//! 4. Agent state is flushed to the memory substrate (`Suspended`).
//!
//! After that explicit dance returns, the daemon process exits and the
//! kernel struct is dropped. Field declaration order on
//! `LibreFangKernel` is therefore not load-bearing — every component
//! that owns long-running tasks (`scheduler`, `cron_scheduler`,
//! `background`, `triggers`, `peer_node`) has already received the
//! `shutdown_tx` signal and aborted its tasks before destructors run.
//! When extending this module, keep new long-running tasks
//! `shutdown_tx`-aware so this property continues to hold.

pub mod agents;
pub mod events;
pub mod governance;
pub mod llm;
pub mod mcp;
pub mod media;
pub mod memory;
pub mod mesh;
pub mod metering;
pub mod processes;
pub mod security;
pub mod skills;
pub mod workflow;

pub use agents::{AgentSubsystem, AgentSubsystemApi};
pub use events::{EventSubsystem, EventSubsystemApi};
pub use governance::{GovernanceSubsystem, GovernanceSubsystemApi};
pub use llm::{LlmSubsystem, LlmSubsystemApi};
pub use mcp::{McpSubsystem, McpSubsystemApi};
pub use media::{MediaSubsystem, MediaSubsystemApi};
pub use memory::{MemorySubsystem, MemorySubsystemApi};
pub use mesh::{MeshSubsystem, MeshSubsystemApi};
pub use metering::{MeteringSubsystem, MeteringSubsystemApi};
pub use processes::{ProcessSubsystem, ProcessSubsystemApi};
pub use security::{SecuritySubsystem, SecuritySubsystemApi};
pub use skills::{SkillsSubsystem, SkillsSubsystemApi};
pub use workflow::{WorkflowSubsystem, WorkflowSubsystemApi};
