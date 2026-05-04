//! Re-export of the kernel `ApprovalManager` used by API routes.
//!
//! Issue #3744: keep route modules from importing
//! `librefang_kernel::approval::*` directly. New code in `librefang-api`
//! should reach for [`ApprovalManager`] via this re-export so the kernel
//! internal module path is not part of the API crate's import surface.
//!
//! The underlying type still lives in the kernel because kernel state
//! constructs and owns the manager (see `LibreFangKernel::approvals`).
//! API routes only call associated functions on it (e.g. the static
//! `verify_totp_code_with_issuer`), so a thin re-export is sufficient
//! and avoids widening `KernelHandle` for what is really a stateless
//! helper.

pub use librefang_kernel::approval::ApprovalManager;
