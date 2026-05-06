//! `KernelApi` — the trait surface consumed by the HTTP API layer (#3566).
//!
//! ## Why this exists
//!
//! `librefang-api` historically held `Arc<LibreFangKernel>` in `AppState`,
//! which means every public inherent method on the kernel struct was
//! automatically part of the HTTP layer's coupling surface. Renaming any
//! kernel internal forced edits in `routes/`, the API layer could not be
//! versioned independently of the kernel, and route tests could not stub
//! the kernel without dragging the whole runtime along.
//!
//! `KernelApi` is the *single explicit contract* between the API and the
//! kernel. `AppState.kernel` is `Arc<dyn KernelApi>`; routes call methods
//! through this trait. The trait is the only place where the API↔kernel
//! coupling is permitted, so widening it is an explicit choice rather than
//! an accidental side-effect of adding a method on `LibreFangKernel`.
//!
//! Distinction from [`crate::kernel_handle`]: that crate exposes
//! kernel-ops needed by the *runtime* (so an agent loop can call back
//! into the kernel without a circular crate dep). `KernelApi` is the
//! analogous trait for the *HTTP layer*. The two trait surfaces overlap
//! conceptually but their scopes diverge — the runtime cares about
//! agent/memory/task primitives, while the API cares about admin /
//! observability surface (audit, config, MCP wiring, hot-reload, …).
//!
//! The trait is implemented for [`LibreFangKernel`] in [`super::kernel`]
//! and the methods delegate to the kernel's inherent impls.

use std::sync::Arc;

use crate::LibreFangKernel;

/// HTTP-API-facing kernel trait.
///
/// `AppState.kernel` is `Arc<dyn KernelApi>`. Routes interact with the
/// kernel exclusively through this trait — there is no `state.kernel.X`
/// path that bypasses it.
pub trait KernelApi: Send + Sync {}

impl KernelApi for LibreFangKernel {}

/// Convenience: type-erase any `Arc<T: KernelApi>` to `Arc<dyn KernelApi>`.
pub fn as_dyn<T: KernelApi + 'static>(k: Arc<T>) -> Arc<dyn KernelApi> {
    k
}
