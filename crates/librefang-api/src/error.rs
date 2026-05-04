//! Re-export of the kernel `KernelError` used by API routes.
//!
//! Issue #3744: keep route modules from importing
//! `librefang_kernel::error::*` directly. Several handlers in
//! `routes/agents.rs` need to pattern-match on kernel error variants
//! (`LibreFang(_)`, `Backpressure(_)`, …) to translate them into HTTP
//! status codes; routing those matches through this re-export keeps
//! the kernel internal module path off the route call sites.
//!
//! The error type itself still lives in the kernel because it is the
//! kernel's own error surface; this module is purely a path-level
//! shortcut, not a re-definition.

pub use librefang_kernel::error::KernelError;
