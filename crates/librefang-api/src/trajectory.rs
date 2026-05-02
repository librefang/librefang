//! API-side facade for kernel trajectory export types.
//!
//! Route handlers must depend on this module rather than reach into
//! `librefang_kernel::trajectory` directly. Centralising the boundary
//! here keeps issue #3744 (kernel-internal imports leaking into the
//! API crate) tractable: when the trajectory pipeline migrates behind
//! `KernelHandle`, only this re-export shifts — every call site in
//! `routes/` already imports from `crate::trajectory`.

pub use librefang_kernel::trajectory::{AgentContext, RedactionPolicy, TrajectoryExporter};
