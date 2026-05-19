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
    InvalidParameter { name: &'static str, reason: String },

    /// A runtime subsystem the tool needs isn't wired in this build /
    /// configuration (kernel handle missing, web/browser context missing,
    /// docker exec disabled, …). Mirrors [`LibreFangError::Unavailable`] and
    /// maps to HTTP 503.
    ///
    /// NOT used for internal call-context attribution gaps (caller agent id,
    /// session id) — those are dispatcher invariants the LLM cannot recover
    /// from and belong under `Internal`, not 503. Lying about a subsystem
    /// being unavailable when the real failure is a missing attribution
    /// would mislead both the upstream caller's retry logic and the
    /// operator's status dashboards.
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
    ///
    /// `source` carries the original `serde_json::Error` (or other typed
    /// serializer error) so callers walking the chain can downcast — same
    /// shape as [`LibreFangError::Serialization`].
    #[error("Serialization error: {message}")]
    Serialization {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

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

    /// Build [`Self::Serialization`] from a typed serializer error (typically
    /// `serde_json::Error`), preserving it on the `source()` chain. Mirrors
    /// [`LibreFangError::serialization`] so the chain survives the bridge into
    /// the application enum.
    pub fn serialization<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Serialization {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build [`Self::Serialization`] from a free-form message (no underlying
    /// typed error — invariant / framing check).
    pub fn serialization_msg(message: impl Into<String>) -> Self {
        Self::Serialization {
            message: message.into(),
            source: None,
        }
    }
}

/// Auto-conversion so call sites can `?`-bubble `serde_json` failures
/// without a `.map_err`. Preserves the underlying `serde_json::Error` on the
/// `source()` chain (matches the `From<serde_json::Error> for LibreFangError`
/// impl in `librefang-types`).
impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        Self::serialization(e)
    }
}

/// Lift [`ToolError`] into [`LibreFangError`] so callers further up the
/// stack can `?`-bubble it without explicit `.map_err`. Maps each kind to
/// the closest existing semantic in the application enum:
///
/// - `MissingParameter` / `InvalidParameter` → `InvalidInput` (caller bug,
///   400-class on the HTTP boundary).
/// - `Unavailable` → `Unavailable` (missing subsystem, 503-class).
/// - `NotFound` → `InvalidInput` (no generic `NotFound { kind }` in the app
///   enum yet — `AgentNotFound` / `SessionNotFound` are the only typed
///   not-found variants. `InvalidInput` keeps the rendered `Display` and
///   maps to 400, which is friendlier than the 500 `Internal` would give if
///   a future caller bubbles a `ToolError::NotFound` through to an HTTP
///   handler).
/// - `PermissionDenied` → `CapabilityDenied` (403-class).
/// - `Upstream` → `ToolExecution` for the `Display`; the typed source is
///   re-attached to `Memory` / `Network` / `LlmDriver` / `Serialization`
///   when we can identify it, otherwise stays on `ToolExecution.reason`
///   as a stringified message. (The `tool_id` field is set to `"unknown"`
///   because the dispatch boundary — not the submodule fn — knows the tool
///   name; slice 5 of #3576 will lift dispatch to return `ToolError` and
///   thread the tool id at that boundary.)
/// - `Serialization` → `LibreFangError::Serialization` preserving the
///   `source()` chain.
/// - `Internal` → `Internal`.
impl From<ToolError> for LibreFangError {
    fn from(e: ToolError) -> Self {
        match e {
            ToolError::MissingParameter(_)
            | ToolError::InvalidParameter { .. }
            | ToolError::NotFound { .. } => LibreFangError::InvalidInput(e.to_string()),
            ToolError::Unavailable(cap) => LibreFangError::unavailable(cap),
            ToolError::PermissionDenied(_) => LibreFangError::CapabilityDenied(e.to_string()),
            ToolError::Upstream { message, source } => match source {
                // The upstream is itself a `LibreFangError` (kernel handle
                // round-trip): unwrap it so the typed kind survives — keeping
                // `LibreFangError::Memory{source}` etc. intact, with its own
                // `BoxedSource` chain — instead of flattening to
                // `ToolExecution{reason: <stringified>}`.
                Some(boxed) => match boxed.downcast::<LibreFangError>() {
                    Ok(inner) => *inner,
                    Err(other) => LibreFangError::ToolExecution {
                        tool_id: "unknown".to_string(),
                        reason: other.to_string(),
                    },
                },
                None => LibreFangError::ToolExecution {
                    tool_id: "unknown".to_string(),
                    reason: message,
                },
            },
            ToolError::Serialization { message, source } => {
                LibreFangError::Serialization { message, source }
            }
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

        let e: LibreFangError = ToolError::serialization_msg("bad utf8").into();
        assert!(matches!(e, LibreFangError::Serialization { .. }));

        let e: LibreFangError = ToolError::Internal("invariant".into()).into();
        assert!(matches!(e, LibreFangError::Internal(_)));
    }

    /// `NotFound` lifts to `InvalidInput` (400), not `Internal` (500). The app
    /// enum does not yet carry a generic `NotFound{kind}` variant — only
    /// `AgentNotFound` / `SessionNotFound` — and falling through to `Internal`
    /// would surface as HTTP 500 on the api boundary, which is wrong for a
    /// "you asked for a thing that does not exist or is not yours" failure.
    /// The rendered `Display` is preserved either way.
    #[test]
    fn not_found_lifts_to_invalid_input_with_display_preserved() {
        let e: LibreFangError = ToolError::NotFound {
            kind: "Cron job",
            id: "abc-123".to_string(),
        }
        .into();
        match e {
            LibreFangError::InvalidInput(msg) => {
                assert_eq!(msg, "Cron job 'abc-123' not found");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    /// `Upstream` carrying a foreign typed error (`std::io::Error` — not a
    /// `LibreFangError`) lifts to `ToolExecution` and the reason field
    /// surfaces the underlying source's `Display`.
    #[test]
    fn upstream_foreign_source_lifts_to_tool_execution() {
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

    /// `Upstream` carrying a typed `LibreFangError` (the common kernel-handle
    /// case — `KernelOpError == LibreFangError`) MUST unwrap, not flatten to
    /// `ToolExecution`. Flattening would lose the variant kind and erase the
    /// `Memory{source} / Network{source}` chain that #3745 went out of its
    /// way to preserve; retry logic walking `source()` would see the boxed
    /// `LibreFangError` instead of being able to downcast to the storage /
    /// transport error directly.
    #[test]
    fn upstream_carrying_librefang_error_round_trips_variant() {
        let inner = LibreFangError::AgentNotFound("agent-x".into());
        let lifted: LibreFangError = ToolError::upstream(inner).into();
        match lifted {
            LibreFangError::AgentNotFound(id) => assert_eq!(id, "agent-x"),
            other => panic!("expected AgentNotFound to round-trip, got {other:?}"),
        }
    }

    /// `Upstream` carrying a typed `LibreFangError::Memory{source}` must
    /// preserve BOTH the outer `Memory` variant AND the inner `BoxedSource`
    /// chain — otherwise the bridge silently undoes #3745.
    #[test]
    fn upstream_carrying_memory_error_preserves_inner_source_chain() {
        let storage = std::io::Error::other("disk full");
        let mem = LibreFangError::memory(storage);
        let lifted: LibreFangError = ToolError::upstream(mem).into();
        match &lifted {
            LibreFangError::Memory {
                message,
                source: Some(s),
            } => {
                assert_eq!(message, "disk full");
                assert!(
                    s.downcast_ref::<std::io::Error>().is_some(),
                    "inner source must still downcast to io::Error"
                );
            }
            other => panic!("expected Memory{{source: Some(_)}}, got {other:?}"),
        }
    }

    /// `Serialization` lifts to `LibreFangError::Serialization` AND keeps the
    /// underlying serde_json::Error on the `source()` chain. The earlier
    /// `Serialization(String)` shape silently dropped it.
    #[test]
    fn serialization_into_librefang_preserves_source_chain() {
        let json_err = serde_json::from_str::<u32>("not a number").unwrap_err();
        let e: LibreFangError = ToolError::serialization(json_err).into();
        match &e {
            LibreFangError::Serialization {
                source: Some(s), ..
            } => {
                assert!(
                    s.downcast_ref::<serde_json::Error>().is_some(),
                    "inner source must downcast to serde_json::Error"
                );
            }
            other => panic!("expected Serialization{{source: Some(_)}}, got {other:?}"),
        }
    }

    /// `From<serde_json::Error> for ToolError` mirrors the
    /// `From<serde_json::Error> for LibreFangError` shape so `?` works the
    /// same on both sides of the boundary.
    #[test]
    fn from_serde_json_error_preserves_source() {
        let json_err = serde_json::from_str::<u32>("nope").unwrap_err();
        let e: ToolError = json_err.into();
        let src = e.source().expect("Serialization should carry a source");
        assert!(
            src.downcast_ref::<serde_json::Error>().is_some(),
            "source must downcast to serde_json::Error"
        );
    }
}
