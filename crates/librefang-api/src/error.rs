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
//! mapping and the `KernelOpError` categories silently collapse to 500.
//!
//! After #3541 8/N, `KernelOpError` is a type alias for
//! `librefang_types::error::LibreFangError`, so matches must use
//! `LibreFangError` variants instead of the old struct-style variants
//! (`Unavailable { .. }`, `NotFound { .. }`, `Invalid { .. }`, …).

pub use librefang_kernel::error::KernelError;

use librefang_kernel_handle::KernelOpError;
use librefang_types::error_code::ErrorCode;

use crate::types::ApiErrorResponse;
use axum::http::StatusCode;

/// Map a typed `KernelOpError` (`LibreFangError` alias) to the canonical
/// HTTP status code.
///
/// | Variant(s)                                      | Status |
/// |-------------------------------------------------|--------|
/// | `AgentNotFound` / `SessionNotFound`             | 404    |
/// | `InvalidInput` / `InvalidState` / `ManifestParse` | 400  |
/// | `AuthDenied` / `CapabilityDenied`               | 403    |
/// | `Unavailable` / `ShuttingDown`                  | 503    |
/// | everything else                                 | 500    |
pub fn kernel_op_status(err: &KernelOpError) -> StatusCode {
    match err {
        KernelOpError::AgentNotFound(_) | KernelOpError::SessionNotFound(_) => {
            StatusCode::NOT_FOUND
        }
        KernelOpError::InvalidInput(_)
        | KernelOpError::InvalidState { .. }
        | KernelOpError::ManifestParse(_) => StatusCode::BAD_REQUEST,
        KernelOpError::AuthDenied(_) | KernelOpError::CapabilityDenied(_) => StatusCode::FORBIDDEN,
        KernelOpError::Unavailable(_) | KernelOpError::ShuttingDown => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Stable machine-readable code for client-side switch logic.
///
/// Routed through [`ErrorCode`] (#3639) so the wire token is enforced by the
/// type system; once a variant is shipped, its `as_str()` is part of the
/// public contract.
pub fn kernel_op_code(err: &KernelOpError) -> &'static str {
    kernel_op_error_code(err).as_str()
}

/// Typed counterpart of [`kernel_op_code`] returning the [`ErrorCode`]
/// variant. Useful when callers want to combine the code with other typed
/// fields without a string round-trip.
pub fn kernel_op_error_code(err: &KernelOpError) -> ErrorCode {
    match err {
        KernelOpError::AgentNotFound(_) | KernelOpError::SessionNotFound(_) => ErrorCode::NotFound,
        KernelOpError::InvalidInput(_)
        | KernelOpError::InvalidState { .. }
        | KernelOpError::ManifestParse(_) => ErrorCode::InvalidInput,
        KernelOpError::AuthDenied(_) => ErrorCode::Forbidden,
        KernelOpError::CapabilityDenied(_) => ErrorCode::CapabilityDenied,
        KernelOpError::Unavailable(_) | KernelOpError::ShuttingDown => {
            ErrorCode::ServiceUnavailable
        }
        KernelOpError::Serialization { .. } => ErrorCode::SerializeFailed,
        _ => ErrorCode::InternalError,
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
            request_id: None,
            status,
        }
    }
}
