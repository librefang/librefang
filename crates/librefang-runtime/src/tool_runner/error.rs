//! Typed errors for tool-runner submodules.
//!
//! Replaces the historical `Result<String, String>` shape with a structured
//! enum so the dispatch layer, the agent loop, and any future HTTP / metering
//! surface can branch on the *kind* of failure (missing parameter vs. upstream
//! crash vs. permission denial) rather than substring-matching the rendered
//! error string.
//!
//! Migration is per-module — see [`docs/architecture/error-contracts.md`] for
//! the full sequence. The dispatch site continues to consume
//! `Result<String, String>`; modules that have migrated convert at their own
//! boundary via `.map_err(|e: ToolError| e.to_string())` so the migration can
//! land incrementally without cascading edits across ~180 sites.
//!
//! Refs: #3576.

use librefang_types::error::{BoxedSource, LibreFangError};
use thiserror::Error;

/// Structured error type returned by tool-runner submodule fns.
///
/// `#[non_exhaustive]` because the variant set will grow as more modules
/// migrate (see the per-module catalog in
/// `docs/architecture/error-contracts.md`). External pattern-matches must
/// include a `_` arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolError {
    /// A required input parameter is missing from the tool-call JSON, or it
    /// is present but the wrong JSON type. The string is the parameter name
    /// (compile-time constant — every call site knows the name statically).
    ///
    /// Maps to "the LLM called the tool wrong — re-prompt with the schema".
    #[error("Missing required parameter '{0}'")]
    MissingParameter(&'static str),

    /// A required input parameter is present but its value is invalid.
    /// `name` is the schema field; `reason` is a free-form human-readable
    /// explanation suitable for relaying back to the LLM.
    #[error("Invalid parameter '{name}': {reason}")]
    InvalidParameter {
        name: &'static str,
        reason: String,
    },

    /// A runtime capability the tool needs isn't wired in this call context
    /// (kernel handle missing, caller agent id missing, web/browser context
    /// missing, …). Mirrors [`LibreFangError::Unavailable`]'s "this build /
    /// configuration doesn't include the subsystem" semantics.
    #[error("{0} unavailable")]
    Unavailable(&'static str),

    /// A target resource was not found OR the caller does not own it. Both
    /// are collapsed into a single variant on purpose: revealing the
    /// distinction is a side-channel for enumeration (e.g. a cron job id
    /// you didn't create but exists for another agent).
    #[error("{kind} '{id}' not found")]
    NotFound { kind: &'static str, id: String },

    /// The caller lacks the right to perform the operation. Distinct from
    /// `NotFound` for cases where the resource's existence is already
    /// public and the failure is purely authorisation (e.g. RBAC `Deny`).
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// A downstream subsystem (kernel handle, MCP server, skill loader)
    /// failed. The upstream error is preserved on the `source()` chain so
    /// callers walking it can downcast back to `LibreFangError`,
    /// `KernelError`, etc.
    #[error("{message}")]
    Upstream {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Serialization of the tool's response (typically `serde_json::to_string`
    /// on a successful upstream result) failed. Distinct from `Upstream` so
    /// the agent loop can distinguish "the tool ran but I couldn't hand you
    /// the answer" from "the tool itself failed".
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Internal invariant violation. Use sparingly — prefer one of the more
    /// specific variants above.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Convenience alias for tool-runner submodule signatures.
pub type ToolResult<T = String> = Result<T, ToolError>;

impl ToolError {
    /// Build [`Self::Upstream`] from any typed error, preserving it on the
    /// `source()` chain. Use for `kh.cron_create(...).map_err(ToolError::upstream)`
    /// where `cron_create` returns a typed `KernelOpError` / `LibreFangError`.
    pub fn upstream<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Upstream {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build [`Self::Upstream`] from a free-form message (no underlying
    /// typed error). Use only where the upstream surface is itself stringly
    /// typed — prefer [`Self::upstream`] when a typed error is available.
    pub fn upstream_msg(message: impl Into<String>) -> Self {
        Self::Upstream {
            message: message.into(),
            source: None,
        }
    }
}

/// Lift [`ToolError`] into [`LibreFangError`] so callers further up the
/// stack can `?`-bubble it without explicit `.map_err`. Maps each kind to
/// the closest existing semantic in the application enum:
///
/// - `MissingParameter` / `InvalidParameter` → `InvalidInput` (caller bug).
/// - `Unavailable` → `Unavailable` (missing subsystem).
/// - `NotFound` → `Internal` (no `NotFound { kind }` in the app enum yet;
///   keeping the `Display` content preserves the rendered message).
/// - `PermissionDenied` → `CapabilityDenied`.
/// - `Upstream` → `ToolExecution` (closest match; the typed source still
///   rides on `BoxedSource`).
/// - `Serialization` → `serialization_msg`.
/// - `Internal` → `Internal`.
impl From<ToolError> for LibreFangError {
    fn from(e: ToolError) -> Self {
        match e {
            ToolError::MissingParameter(_) | ToolError::InvalidParameter { .. } => {
                LibreFangError::InvalidInput(e.to_string())
            }
            ToolError::Unavailable(cap) => LibreFangError::unavailable(cap),
            ToolError::NotFound { .. } => LibreFangError::Internal(e.to_string()),
            ToolError::PermissionDenied(_) => LibreFangError::CapabilityDenied(e.to_string()),
            ToolError::Upstream { message, source } => LibreFangError::ToolExecution {
                tool_id: "unknown".to_string(),
                reason: source
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or(message),
            },
            ToolError::Serialization(msg) => LibreFangError::serialization_msg(msg),
            ToolError::Internal(msg) => LibreFangError::Internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn missing_parameter_renders_with_quoted_name() {
        let e = ToolError::MissingParameter("goal_id");
        assert_eq!(e.to_string(), "Missing required parameter 'goal_id'");
    }

    #[test]
    fn invalid_parameter_includes_reason() {
        let e = ToolError::InvalidParameter {
            name: "status",
            reason: "must be one of: pending, in_progress, completed, cancelled".to_string(),
        };
        assert_eq!(
            e.to_string(),
            "Invalid parameter 'status': must be one of: pending, in_progress, completed, cancelled"
        );
    }

    #[test]
    fn unavailable_renders_with_capability_name() {
        let e = ToolError::Unavailable("Kernel handle");
        assert_eq!(e.to_string(), "Kernel handle unavailable");
    }

    #[test]
    fn not_found_does_not_reveal_authz_distinction() {
        let e = ToolError::NotFound {
            kind: "Cron job",
            id: "abc-123".to_string(),
        };
        // Single phrasing regardless of whether the resource doesn't exist
        // OR exists but the caller doesn't own it.
        assert_eq!(e.to_string(), "Cron job 'abc-123' not found");
    }

    #[test]
    fn upstream_preserves_typed_source_chain() {
        let inner = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e = ToolError::upstream(inner);
        let src = e.source().expect("Upstream should carry a source");
        let downcast = src
            .downcast_ref::<std::io::Error>()
            .expect("source should downcast to io::Error");
        assert_eq!(downcast.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn upstream_msg_has_no_source() {
        let e = ToolError::upstream_msg("kernel call failed");
        assert!(e.source().is_none());
        assert_eq!(e.to_string(), "kernel call failed");
    }

    /// Bridge to the shared application enum. Each variant must land on the
    /// closest existing semantic so callers further up the stack can `?`
    /// without losing the kind.
    #[test]
    fn into_librefang_error_maps_kinds() {
        let e: LibreFangError = ToolError::MissingParameter("x").into();
        assert!(matches!(e, LibreFangError::InvalidInput(_)));

        let e: LibreFangError = ToolError::Unavailable("Cron scheduler").into();
        assert!(matches!(e, LibreFangError::Unavailable(_)));

        let e: LibreFangError = ToolError::PermissionDenied("nope".into()).into();
        assert!(matches!(e, LibreFangError::CapabilityDenied(_)));

        let e: LibreFangError = ToolError::Serialization("bad utf8".into()).into();
        assert!(matches!(e, LibreFangError::Serialization { .. }));

        let e: LibreFangError = ToolError::Internal("invariant".into()).into();
        assert!(matches!(e, LibreFangError::Internal(_)));
    }

    /// `Upstream` lifts to `ToolExecution`. The reason field must prefer the
    /// underlying typed source's `Display` when available so the rendered
    /// message contains the actual cause, not the wrapper's repeating prefix.
    #[test]
    fn upstream_into_librefang_error_carries_source_display() {
        let inner = std::io::Error::new(std::io::ErrorKind::TimedOut, "read timed out");
        let e: LibreFangError = ToolError::upstream(inner).into();
        match e {
            LibreFangError::ToolExecution { tool_id, reason } => {
                assert_eq!(tool_id, "unknown");
                assert_eq!(reason, "read timed out");
            }
            other => panic!("expected ToolExecution, got {other:?}"),
        }
    }
}
