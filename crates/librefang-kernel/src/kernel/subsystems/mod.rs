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

pub mod governance;
pub mod llm;
pub mod mcp;
pub mod media;
pub mod mesh;
pub mod metering;
pub mod processes;
pub mod security;
pub mod skills;
pub mod workflow;

pub use governance::GovernanceSubsystem;
pub use llm::LlmSubsystem;
pub use mcp::McpSubsystem;
pub use media::MediaSubsystem;
pub use mesh::MeshSubsystem;
pub use metering::MeteringSubsystem;
pub use processes::ProcessSubsystem;
pub use security::SecuritySubsystem;
pub use skills::SkillsSubsystem;
pub use workflow::WorkflowSubsystem;
