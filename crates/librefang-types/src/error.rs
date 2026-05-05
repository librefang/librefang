//! Shared error types for the LibreFang system.

use thiserror::Error;

/// Boxed underlying error preserved on the `source()` chain for the four
/// stringly-typed variants migrated in #3745. Boxed (rather than carrying
/// `rusqlite::Error` / `reqwest::Error` directly) because `librefang-types`
/// must stay free of storage-backend and HTTP-client dependencies — typed
/// `#[from]` would invert the workspace dependency graph. The boxed value is
/// still walkable via `std::error::Error::source()`, so retry / circuit-break
/// logic can downcast when it cares about the concrete underlying type.
pub type BoxedSource = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Top-level error type for the LibreFang system.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum LibreFangError {
    /// The requested agent was not found.
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    /// An agent with this name or ID already exists.
    #[error("Agent already exists: {0}")]
    AgentAlreadyExists(String),

    /// A capability check failed.
    #[error("Capability denied: {0}")]
    CapabilityDenied(String),

    /// A resource quota was exceeded.
    #[error("Resource quota exceeded: {0}")]
    QuotaExceeded(String),

    /// The agent is in an invalid state for the requested operation.
    #[error("Agent is in invalid state '{current}' for operation '{operation}'")]
    InvalidState {
        /// The current state of the agent.
        current: String,
        /// The operation that was attempted.
        operation: String,
    },

    /// The requested session was not found.
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// A memory substrate error occurred.
    ///
    /// `source` carries the original `rusqlite::Error` (or other storage
    /// backend error) when one was available, preserving the
    /// `std::error::Error::source()` chain so retry / circuit-break logic can
    /// downcast and inspect the underlying type. When the error originates
    /// from a free-form invariant check (no underlying typed error), `source`
    /// is `None`. See #3745.
    #[error("Memory error: {message}")]
    Memory {
        /// Human-readable message (the underlying error's `Display`, or a
        /// free-form description when no typed source is available).
        message: String,
        /// Underlying error preserved on the `source()` chain.
        #[source]
        source: Option<BoxedSource>,
    },

    /// A tool execution failed.
    #[error("Tool execution failed: {tool_id} — {reason}")]
    ToolExecution {
        /// The tool that failed.
        tool_id: String,
        /// Why it failed.
        reason: String,
    },

    /// An LLM driver error occurred.
    ///
    /// `source` carries the original `LlmError` (boxed because
    /// `librefang-types` must not depend on `librefang-llm-driver`) so
    /// callers further up the stack can recover the typed driver error via
    /// `std::error::Error::source()` + downcast. See #3745.
    #[error("LLM driver error: {message}")]
    LlmDriver {
        /// Human-readable message (typically the underlying `LlmError`'s
        /// `Display`).
        message: String,
        /// Underlying error preserved on the `source()` chain.
        #[source]
        source: Option<BoxedSource>,
    },

    /// A configuration error occurred.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Failed to parse an agent manifest.
    #[error("Manifest parsing error: {0}")]
    ManifestParse(String),

    /// A WASM sandbox error occurred.
    #[error("WASM sandbox error: {0}")]
    Sandbox(String),

    /// A network error occurred.
    ///
    /// `source` carries the original `reqwest::Error` (or other transport
    /// error) when one was available so retry logic can downcast for
    /// per-error-kind handling (e.g. `is_timeout()` vs `is_connect()`). See
    /// #3745.
    #[error("Network error: {message}")]
    Network {
        /// Human-readable message.
        message: String,
        /// Underlying error preserved on the `source()` chain.
        #[source]
        source: Option<BoxedSource>,
    },

    /// A serialization/deserialization error occurred.
    ///
    /// `source` carries the original `serde_json::Error` / `rmp_serde::*`
    /// error (boxed because `librefang-types` does not depend on
    /// `rmp_serde`) so callers can downcast for format-specific recovery.
    /// `source` is `None` for free-form invariant violations (length-prefix
    /// or framing checks). See #3745.
    #[error("Serialization error: {message}")]
    Serialization {
        /// Human-readable message.
        message: String,
        /// Underlying error preserved on the `source()` chain.
        #[source]
        source: Option<BoxedSource>,
    },

    /// The agent loop exceeded the maximum iteration count.
    #[error("Max iterations exceeded: {0}")]
    MaxIterationsExceeded(u32),

    /// The kernel is shutting down.
    #[error("Shutdown in progress")]
    ShuttingDown,

    /// An I/O error occurred.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// An internal error occurred.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Authentication/authorization denied.
    #[error("Auth denied: {0}")]
    AuthDenied(String),

    /// Metering/cost tracking error.
    #[error("Metering error: {0}")]
    MeteringError(String),

    /// Invalid user input.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// The capability is not wired in this build / configuration. Used by
    /// kernel-handle role-trait default impls to signal that an optional
    /// subsystem (cron scheduler, hands system, approval queue, channel
    /// adapter, …) is unavailable. The string carries the capability name
    /// so callers can surface it without parsing the formatted message.
    /// Maps cleanly to HTTP 503 Service Unavailable on the api boundary.
    /// Replaces the historical `Result<_, "X not available">` String shape
    /// (#3541).
    #[error("{0} not available")]
    Unavailable(String),

    /// The agent loop exited because tools failed in N consecutive iterations.
    #[error(
        "Repeated tool failures: {iterations} consecutive iterations with {error_count} errors"
    )]
    RepeatedToolFailures {
        /// How many consecutive iterations had all tools fail.
        iterations: u32,
        /// The total count of tool errors in the final iteration.
        error_count: u32,
    },

    /// The provider blocked the response with a safety / policy filter.
    /// Carries any partial text the model returned before the refusal so
    /// the caller can surface it to the user (#3450).
    #[error("Content filtered by provider: {message}")]
    ContentFiltered {
        /// Partial text emitted before the refusal (may be empty).
        message: String,
    },
}

/// Alias for Result with LibreFangError.
pub type LibreFangResult<T> = Result<T, LibreFangError>;

// ---------------------------------------------------------------------------
// Constructor helpers (#3745)
//
// The four variants migrated from `Foo(String)` to `Foo { message, source }`
// keep the source error reachable via `Error::source()`. Helpers below
// preserve the ergonomics of the old call sites:
//
//   .map_err(|e| LibreFangError::Memory(e.to_string()))    // before
//   .map_err(LibreFangError::memory)                       // after  (typed e)
//
//   LibreFangError::Memory(format!("FTS delete failed"))   // before
//   LibreFangError::memory_msg(format!("FTS delete failed")) // after (no source)
//
// `*_msg` variants build the error WITHOUT a `source` (used for free-form
// invariant violations); the bare-named variants accept any
// `std::error::Error + Send + Sync + 'static` and box it into the source
// chain.
// ---------------------------------------------------------------------------

impl LibreFangError {
    /// Build an [`Self::Unavailable`] for a missing optional subsystem
    /// (cron scheduler, hands system, approval queue, channel adapter, …).
    /// Used by `KernelHandle` role-trait default impls to signal that the
    /// capability is wired off in this build / configuration. (#3541)
    pub fn unavailable(capability: impl Into<String>) -> Self {
        Self::Unavailable(capability.into())
    }

    /// Build a [`Self::Memory`] from a typed error, preserving it on the
    /// `source()` chain.
    pub fn memory<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Memory {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build a [`Self::Memory`] from a free-form message (no underlying typed
    /// error available — invariant violation, framing check, …).
    pub fn memory_msg(message: impl Into<String>) -> Self {
        Self::Memory {
            message: message.into(),
            source: None,
        }
    }

    /// Build a [`Self::LlmDriver`] from a typed error (typically `LlmError`),
    /// preserving it on the `source()` chain.
    pub fn llm_driver<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::LlmDriver {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build a [`Self::LlmDriver`] from a free-form message (no underlying
    /// typed error available — synthesised inside the agent loop, etc.).
    pub fn llm_driver_msg(message: impl Into<String>) -> Self {
        Self::LlmDriver {
            message: message.into(),
            source: None,
        }
    }

    /// Build a [`Self::Network`] from a typed error (typically
    /// `reqwest::Error`), preserving it on the `source()` chain.
    pub fn network<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Network {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build a [`Self::Network`] from a free-form message (no underlying
    /// typed error available).
    pub fn network_msg(message: impl Into<String>) -> Self {
        Self::Network {
            message: message.into(),
            source: None,
        }
    }

    /// Build a [`Self::Serialization`] from a typed error (typically
    /// `serde_json::Error` / `rmp_serde::*`), preserving it on the
    /// `source()` chain.
    pub fn serialization<E>(source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Serialization {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    /// Build a [`Self::Serialization`] from a free-form description (no
    /// underlying typed error — framing / invariant checks).
    pub fn serialization_msg(message: impl Into<String>) -> Self {
        Self::Serialization {
            message: message.into(),
            source: None,
        }
    }
}

impl From<serde_json::Error> for LibreFangError {
    fn from(e: serde_json::Error) -> Self {
        Self::serialization(e)
    }
}

/// String → `Internal(_)`. Lets callers that produced a `String` error
/// (during the migration window away from `Result<_, String>`) flow it
/// through `?` into a function returning `LibreFangError`. New code SHOULD
/// pick a more specific variant where one fits.
impl From<String> for LibreFangError {
    fn from(s: String) -> Self {
        Self::Internal(s)
    }
}

impl From<&str> for LibreFangError {
    fn from(s: &str) -> Self {
        Self::Internal(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    /// `Memory::source()` must walk all the way to the underlying typed error
    /// (rusqlite::Error, std::io::Error, …) so retry/circuit-break logic can
    /// downcast. The historical `Memory(String)` shape lost this. (#3745)
    #[test]
    fn memory_preserves_typed_source_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err = LibreFangError::memory(io_err);
        let src = err.source().expect("Memory should carry source");
        let downcast = src
            .downcast_ref::<std::io::Error>()
            .expect("source should downcast to std::io::Error");
        assert_eq!(downcast.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn memory_msg_has_no_source() {
        let err = LibreFangError::memory_msg("FTS delete failed: corruption");
        assert!(err.source().is_none());
        assert_eq!(
            err.to_string(),
            "Memory error: FTS delete failed: corruption"
        );
    }

    #[test]
    fn llm_driver_preserves_typed_source_chain() {
        let inner = std::io::Error::other("upstream parse failure");
        let err = LibreFangError::llm_driver(inner);
        assert!(err.source().is_some(), "LlmDriver should carry source");
        assert!(err.to_string().starts_with("LLM driver error: "));
    }

    #[test]
    fn llm_driver_msg_has_no_source() {
        let err = LibreFangError::llm_driver_msg("budget exhausted");
        assert!(err.source().is_none());
        assert_eq!(err.to_string(), "LLM driver error: budget exhausted");
    }

    #[test]
    fn network_preserves_typed_source_chain() {
        let inner = std::io::Error::new(std::io::ErrorKind::TimedOut, "read timed out");
        let err = LibreFangError::network(inner);
        let src = err.source().expect("Network should carry source");
        let downcast = src
            .downcast_ref::<std::io::Error>()
            .expect("source should downcast to std::io::Error");
        assert_eq!(downcast.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn serialization_preserves_typed_source_chain_via_from() {
        // `From<serde_json::Error>` keeps the source chain alive for the
        // common case where the failing call already returns one — `?`
        // continues to work without explicit `.map_err`.
        let bad = serde_json::from_str::<serde_json::Value>("{not json");
        let err: LibreFangError = bad.unwrap_err().into();
        let src = err.source().expect("Serialization should carry source");
        assert!(
            src.downcast_ref::<serde_json::Error>().is_some(),
            "source should downcast to serde_json::Error"
        );
        assert!(err.to_string().starts_with("Serialization error: "));
    }

    #[test]
    fn serialization_constructor_accepts_arbitrary_error() {
        // `serialization()` accepts any std::error::Error — the boxed source
        // is used for `rmp_serde` and other formats `librefang-types` cannot
        // depend on directly.
        let inner = std::io::Error::other("bad msgpack frame");
        let err = LibreFangError::serialization(inner);
        let src = err.source().expect("Serialization should carry source");
        let downcast = src
            .downcast_ref::<std::io::Error>()
            .expect("source should downcast to std::io::Error");
        assert_eq!(downcast.to_string(), "bad msgpack frame");
    }

    #[test]
    fn serialization_msg_has_no_source() {
        let err = LibreFangError::serialization_msg("length prefix mismatch");
        assert!(err.source().is_none());
        assert_eq!(
            err.to_string(),
            "Serialization error: length prefix mismatch"
        );
    }
}
