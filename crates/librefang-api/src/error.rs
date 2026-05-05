//! Kernel error → HTTP response mappings used by API routes.
//!
//! Issue #3744: keep route modules from importing
//! `librefang_kernel::error::*` directly. Several handlers in
//! `routes/agents.rs` need to pattern-match on kernel error variants
//! (`LibreFang(_)`, `Backpressure(_)`, …) to translate them into HTTP
//! status codes; routing those matches through this re-export keeps
//! the kernel internal module path off the route call sites.
//!
//! Issue #3541: this module also owns the `KernelOpError → ApiErrorResponse`
//! mapping. Centralising it here lets every route handler delegate via
//! `?` / `.map_err(Into::into)` instead of building its own ad-hoc
//! match. Without this, each handler invents its own status-code
//! mapping and the `KernelOpError::NotFound` / `Invalid` / `Unavailable`
//! categories silently collapse to 500.

pub use librefang_kernel::error::KernelError;

use librefang_kernel_handle::KernelOpError;

use crate::types::ApiErrorResponse;
use axum::http::StatusCode;

/// Map a typed `KernelOpError` to the canonical HTTP status code.
///
/// The mapping is the contract advertised on `KernelOpError`'s variant
/// docs in `librefang-kernel-handle/src/lib.rs`:
///
/// | Variant       | Status                          |
/// |---------------|---------------------------------|
/// | `Unavailable` | 503 Service Unavailable         |
/// | `NotFound`    | 404 Not Found                   |
/// | `Invalid`     | 400 Bad Request                 |
/// | `Serialize`   | 500 Internal Server Error       |
/// | `Other`       | 500 Internal Server Error       |
pub fn kernel_op_status(err: &KernelOpError) -> StatusCode {
    match err {
        KernelOpError::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        KernelOpError::NotFound { .. } => StatusCode::NOT_FOUND,
        KernelOpError::Invalid { .. } => StatusCode::BAD_REQUEST,
        KernelOpError::Serialize(_) | KernelOpError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Stable machine-readable code for client-side switch logic.
pub fn kernel_op_code(err: &KernelOpError) -> &'static str {
    match err {
        KernelOpError::Unavailable { .. } => "service_unavailable",
        KernelOpError::NotFound { .. } => "not_found",
        KernelOpError::Invalid { .. } => "invalid_input",
        KernelOpError::Serialize(_) => "serialize_failed",
        KernelOpError::Other(_) => "internal_error",
    }
}

impl From<KernelOpError> for ApiErrorResponse {
    fn from(err: KernelOpError) -> Self {
        let status = kernel_op_status(&err);
        let code = kernel_op_code(&err).to_string();
        ApiErrorResponse {
            error: err.to_string(),
            code: Some(code.clone()),
            r#type: Some(code),
            details: None,
            status,
        }
    }
}
