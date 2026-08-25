//! Kernel error → HTTP response mappings used by API routes.
//!
//! Issue #3744: keep route modules from importing
//! `librefang_kernel::error::*` directly. Several handlers in
//! `routes/agents/` need to pattern-match on kernel error variants
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
/// | `Conflict`                                      | 409    |
/// | `AuthDenied` / `CapabilityDenied`               | 403    |
/// | `QuotaExceeded`                                 | 429    |
/// | `Unavailable` / `ShuttingDown`                  | 503    |
/// | everything else                                 | 500    |
///
/// `QuotaExceeded` was in the `_ => 500` bucket until #6699, which meant a self-imposed budget ceiling reached the client as a scrubbed "Internal server error" — indistinguishable from a crash, and an invitation for a client to retry the request that just refused it.
/// `routes/memory.rs` had already reached 429 through a hand-rolled local error type; this is the same answer, in the one place every other route inherits.
pub fn kernel_op_status(err: &KernelOpError) -> StatusCode {
    match err {
        KernelOpError::AgentNotFound(_)
        | KernelOpError::SessionNotFound(_)
        | KernelOpError::ResourceNotFound { .. } => StatusCode::NOT_FOUND,
        KernelOpError::InvalidInput(_)
        | KernelOpError::InvalidState { .. }
        | KernelOpError::ManifestParse(_) => StatusCode::BAD_REQUEST,
        KernelOpError::Conflict(_) => StatusCode::CONFLICT,
        KernelOpError::AuthDenied(_) | KernelOpError::CapabilityDenied(_) => StatusCode::FORBIDDEN,
        KernelOpError::QuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
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
        KernelOpError::AgentNotFound(_)
        | KernelOpError::SessionNotFound(_)
        | KernelOpError::ResourceNotFound { .. } => ErrorCode::NotFound,
        KernelOpError::InvalidInput(_)
        | KernelOpError::InvalidState { .. }
        | KernelOpError::ManifestParse(_) => ErrorCode::InvalidInput,
        KernelOpError::Conflict(_) => ErrorCode::Conflict,
        KernelOpError::AuthDenied(_) => ErrorCode::Forbidden,
        KernelOpError::CapabilityDenied(_) => ErrorCode::CapabilityDenied,
        KernelOpError::QuotaExceeded(_) => ErrorCode::QuotaExceeded,
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
        let error = if status.is_server_error() {
            tracing::error!(error = %err, code, "kernel error scrubbed before response");
            if status == StatusCode::SERVICE_UNAVAILABLE {
                "Service unavailable".to_string()
            } else {
                "Internal server error".to_string()
            }
        } else {
            err.to_string()
        };
        ApiErrorResponse {
            error,
            code: Some(code.clone()),
            r#type: Some(code),
            details: None,
            request_id: None,
            status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A budget ceiling is the caller's own configuration, not a fault, so it must arrive as 429 with its message intact rather than as a scrubbed 500.
    #[test]
    fn an_exhausted_quota_is_a_429_that_keeps_its_message() {
        let response = ApiErrorResponse::from(KernelOpError::QuotaExceeded(
            "hourly cost budget exhausted".to_string(),
        ));
        assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.code.as_deref(), Some("quota_exceeded"));
        assert!(
            response.error.contains("hourly cost budget"),
            "a client-side refusal must not be scrubbed, got: {}",
            response.error
        );
    }

    #[test]
    fn internal_kernel_errors_are_scrubbed_but_keep_typed_code() {
        let response = ApiErrorResponse::from(KernelOpError::Internal(
            "database path /srv/private/state.db failed".to_string(),
        ));

        assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.error, "Internal server error");
        assert_eq!(response.code.as_deref(), Some("internal_error"));
        assert!(!response.error.contains("/srv/private"));
    }

    #[test]
    fn client_kernel_errors_retain_actionable_message() {
        let response = ApiErrorResponse::from(KernelOpError::InvalidInput(
            "agent name must not be empty".to_string(),
        ));

        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert!(response.error.contains("agent name must not be empty"));
        assert_eq!(response.code.as_deref(), Some("invalid_input"));
    }

    #[test]
    fn unavailable_kernel_errors_are_scrubbed() {
        let response = ApiErrorResponse::from(KernelOpError::Unavailable(
            "internal queue worker crashed".to_string(),
        ));

        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.error, "Service unavailable");
        assert_eq!(response.code.as_deref(), Some("service_unavailable"));
    }
}
