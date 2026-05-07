//! Field-owning subsystems split out of `LibreFangKernel` (refs #3565).
//!
//! Each subsystem is a thin struct that bundles a previously-flat cluster
//! of `LibreFangKernel` fields. Method bodies still live on
//! `LibreFangKernel` and reach into subsystems via `self.<sub>.<field>`.
//! That keeps the ~600 internal call-sites mechanical while shrinking the
//! kernel struct surface from ~70 fields to a dozen subsystem handles.
//!
//! Focused per-subsystem traits and method-body migration are explicit
//! follow-ups — kept out of this PR so the diff stays reviewable.

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
