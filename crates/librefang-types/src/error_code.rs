//! Stable, machine-readable error codes returned in API error envelopes (#3639).
//!
//! The HTTP layer returns errors in the canonical `ApiErrorResponse` envelope
//! whose `code` field is a short, stable token like `"agent_not_found"` or
//! `"rate_limited"`. Clients use this token to drive retry / surface UX and
//! support engineers grep for it in logs. To keep callers consistent across
//! the workspace, we centralise the token alphabet here as a single enum
//! with an exhaustive `as_str()` mapping.
//!
//! ## Stability contract
//!
//! Once a variant is shipped, **its `as_str()` value is immutable**. Renaming
//! a variant is a breaking change for any client that switches on the string.
//! The `Serialize` impl emits the same `as_str()` token (snake_case), so
//! direct serialisation matches the canonical wire form.
//!
//! Adding a new variant is non-breaking — clients are expected to treat
//! unknown codes as `internal_error`.
//!
//! See `crates/librefang-api/src/types.rs::ApiErrorResponse` for the
//! envelope and `error.rs::kernel_op_code` for the kernel-error mapping.

use serde::{Serialize, Serializer};

/// Stable error codes surfaced in `ApiErrorResponse.code`.
///
/// The `as_str()` mapping is the documented wire contract; treat it as a
/// public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    // --- Generic HTTP-level codes ---------------------------------------
    /// Generic 400 — request validation / parsing failed.
    BadRequest,
    /// Caller is unauthenticated.
    Unauthorized,
    /// Caller is authenticated but lacks permission.
    Forbidden,
    /// Resource does not exist.
    NotFound,
    /// Conflicting state (e.g. duplicate name, optimistic concurrency).
    Conflict,
    /// Per-IP / per-token rate limit kicked in.
    RateLimited,
    /// Server hit an unhandled error.
    InternalError,
    /// Downstream dependency unavailable (kernel, storage, …).
    ServiceUnavailable,
    /// Caller asked for an unsupported feature on this build.
    NotSupported,
    /// Server-side serialisation / deserialisation failure.
    SerializeFailed,
    /// Generic catch-all for client-side input validation failures that are
    /// more specific than `BadRequest`.
    InvalidInput,

    // --- Domain-specific codes ------------------------------------------
    /// Agent referenced by the request does not exist.
    AgentNotFound,
    /// Session referenced by the request does not exist.
    SessionNotFound,
    /// Capability check failed.
    CapabilityDenied,
    /// Resource quota was exceeded.
    QuotaExceeded,
    /// Provider safety filter blocked the response.
    ContentFiltered,
    /// Caller is in an invalid state for the requested operation.
    InvalidState,
}

impl ErrorCode {
    /// Stable wire token. Once shipped, never rename.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::InternalError => "internal_error",
            Self::ServiceUnavailable => "service_unavailable",
            Self::NotSupported => "not_supported",
            Self::SerializeFailed => "serialize_failed",
            Self::InvalidInput => "invalid_input",
            Self::AgentNotFound => "agent_not_found",
            Self::SessionNotFound => "session_not_found",
            Self::CapabilityDenied => "capability_denied",
            Self::QuotaExceeded => "quota_exceeded",
            Self::ContentFiltered => "content_filtered",
            Self::InvalidState => "invalid_state",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl From<ErrorCode> for &'static str {
    fn from(code: ErrorCode) -> Self {
        code.as_str()
    }
}

impl From<ErrorCode> for String {
    fn from(code: ErrorCode) -> Self {
        code.as_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire tokens are part of the public contract — pin every variant.
    /// Adding a variant means adding an assertion here so reviewers see the
    /// new token explicitly.
    #[test]
    fn as_str_tokens_are_stable() {
        assert_eq!(ErrorCode::BadRequest.as_str(), "bad_request");
        assert_eq!(ErrorCode::Unauthorized.as_str(), "unauthorized");
        assert_eq!(ErrorCode::Forbidden.as_str(), "forbidden");
        assert_eq!(ErrorCode::NotFound.as_str(), "not_found");
        assert_eq!(ErrorCode::Conflict.as_str(), "conflict");
        assert_eq!(ErrorCode::RateLimited.as_str(), "rate_limited");
        assert_eq!(ErrorCode::InternalError.as_str(), "internal_error");
        assert_eq!(
            ErrorCode::ServiceUnavailable.as_str(),
            "service_unavailable"
        );
        assert_eq!(ErrorCode::NotSupported.as_str(), "not_supported");
        assert_eq!(ErrorCode::SerializeFailed.as_str(), "serialize_failed");
        assert_eq!(ErrorCode::InvalidInput.as_str(), "invalid_input");
        assert_eq!(ErrorCode::AgentNotFound.as_str(), "agent_not_found");
        assert_eq!(ErrorCode::SessionNotFound.as_str(), "session_not_found");
        assert_eq!(ErrorCode::CapabilityDenied.as_str(), "capability_denied");
        assert_eq!(ErrorCode::QuotaExceeded.as_str(), "quota_exceeded");
        assert_eq!(ErrorCode::ContentFiltered.as_str(), "content_filtered");
        assert_eq!(ErrorCode::InvalidState.as_str(), "invalid_state");
    }

    #[test]
    fn serializes_as_string_token() {
        let json = serde_json::to_string(&ErrorCode::AgentNotFound).unwrap();
        assert_eq!(json, "\"agent_not_found\"");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", ErrorCode::RateLimited), "rate_limited");
    }

    /// `From<ErrorCode> for String` lets callers feed it directly into
    /// `ApiErrorResponse.code: Option<String>` without `.to_string()`.
    #[test]
    fn into_string_uses_as_str() {
        let s: String = ErrorCode::NotFound.into();
        assert_eq!(s, "not_found");
    }
}
