//! Typed MCP tool-call failures, and the seam that reports the transport ones back to the kernel's health monitor (#7963).
//!
//! # Why the error needed a type
//!
//! [`McpConnection::call_tool_with_caller`](crate::McpConnection::call_tool_with_caller) used to return `Result<String, String>`, which collapsed two failures with opposite remedies into the same value:
//!
//! - the MCP server answered, with a well-formed JSON-RPC error — bad
//!   arguments, file not found, rate limited. The transport is healthy and the
//!   only sensible thing to do is hand the message to the model.
//! - the MCP server did not answer — the request timed out, the transport was
//!   closed, the stdio pipe to the child process is dead. The connection needs
//!   to be rebuilt; handing the model a message is not a remedy at all.
//!
//! The caller could only tell them apart by matching on substrings of an error message it did not own, so in practice it did not try: the tool-call dispatch path folded every failure into a string and the health monitor never heard about any of them.
//! That is the bug in #7963 — auto-reconnect could not engage because nothing could move a connected server into `McpStatus::Error`.
//!
//! [`McpCallError`] carries the classification alongside the message, so the dispatch path asks [`McpCallError::is_transport`] instead of sniffing text.
//!
//! # The classification rule
//!
//! One rule covers every transport: **a well-formed JSON-RPC error response is an application error; anything that means "no well-formed response arrived" is a transport error.** For rmcp that is a structural match on [`rmcp::service::ServiceError`] (see [`classify_service_error`](crate::classify_service_error)) — no string matching anywhere on the path.

use std::sync::Arc;

/// Which of the two failure modes an MCP tool call hit — see the module docs for the rule that separates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpErrorKind {
    /// The server answered with an error. The connection is fine.
    Application,
    /// No usable answer arrived: timeout, closed transport, dead pipe, undecodable response.
    /// The connection may need rebuilding.
    Transport,
}

/// A classified MCP tool-call failure.
///
/// `Display` renders only [`message`](Self::message), so existing call sites that interpolate the error into user- or model-facing text are unchanged by the switch away from `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCallError {
    kind: McpErrorKind,
    message: String,
}

impl McpCallError {
    /// A failure the server reported, or that this client rejected before transmit (schema validation, taint scan).
    /// The connection is healthy.
    pub fn application(message: impl Into<String>) -> Self {
        Self {
            kind: McpErrorKind::Application,
            message: message.into(),
        }
    }

    /// A failure of the transport itself — nothing usable came back.
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            kind: McpErrorKind::Transport,
            message: message.into(),
        }
    }

    /// Build with an explicit kind, for call sites that classify first.
    pub fn new(kind: McpErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// The classification.
    pub fn kind(&self) -> McpErrorKind {
        self.kind
    }

    /// Whether this failure means the transport, not the server's answer, is broken.
    /// The tool-call dispatch path reports health only when this is `true`.
    pub fn is_transport(&self) -> bool {
        self.kind == McpErrorKind::Transport
    }

    /// The human-readable message.
    /// Same text the pre-#7963 `String` error carried.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for McpCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for McpCallError {}

/// Un-classified `String` errors default to [`McpErrorKind::Application`].
///
/// Defaulting to "application" is the safe direction: a misclassified application error costs the model one unhelpful message, while a misclassified transport error would tear down and rebuild a healthy MCP server.
/// Paths that *know* they hit the transport say so explicitly with [`McpCallError::transport`].
impl From<String> for McpCallError {
    fn from(message: String) -> Self {
        Self::application(message)
    }
}

impl From<&str> for McpCallError {
    fn from(message: &str) -> Self {
        Self::application(message)
    }
}

/// Lets helpers that still return `Result<_, String>` consume a classified error with `?` without every one of them changing signature.
impl From<McpCallError> for String {
    fn from(err: McpCallError) -> Self {
        err.message
    }
}

/// The seam from the agent runtime back to the kernel's MCP health monitor (#7963).
///
/// Defined here, in the runtime, and implemented in the kernel — the trait-injection pattern the `McpOAuthProvider` in this crate already uses, and for the same reason: the health monitor lives in `librefang-extensions` and the runtime must not depend on it.
///
/// A [`McpConnection`](crate::McpConnection) built by the kernel carries one of these, attached with [`with_health_reporter`](crate::McpConnection::with_health_reporter).
/// Stand-alone callers (tests, ad-hoc scripts) leave it unset and the reports are dropped.
pub trait McpTransportHealthReporter: Send + Sync {
    /// A tool call on `server` failed at the transport level.
    ///
    /// The kernel routes this to `HealthMonitor::report_transport_failure`, which flips the server to `McpStatus::Error` and — once `TRANSPORT_FAILURES_BEFORE_RECONNECT` consecutive failures have accumulated — makes it reconnect-eligible for the health loop.
    ///
    /// Implementations must be cheap and non-blocking: this runs inline on the agent's tool-call path.
    fn report_transport_failure(&self, server: &str, error: &str);

    /// A tool call on `server` completed.
    /// Resets the consecutive-failure count so the threshold only ever counts an *unbroken* run of failures, and refreshes `last_ok` so `/api/mcp/health` reflects real traffic rather than only the last handshake.
    fn report_call_ok(&self, server: &str, tool_count: usize);
}

/// Shared handle to a [`McpTransportHealthReporter`].
pub type SharedHealthReporter = Arc<dyn McpTransportHealthReporter>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_bare_message() {
        let err = McpCallError::transport("MCP tool call timed out after 60s");
        assert_eq!(err.to_string(), "MCP tool call timed out after 60s");
        assert_eq!(
            format!("MCP tool call failed: {err}"),
            "MCP tool call failed: MCP tool call timed out after 60s"
        );
    }

    #[test]
    fn string_conversion_defaults_to_application() {
        let err: McpCallError = "boom".to_string().into();
        assert_eq!(err.kind(), McpErrorKind::Application);
        assert!(!err.is_transport());
        // …and converts back to the same text, so `Result<_, String>` helpers keep working with `?`.
        assert_eq!(String::from(err), "boom");
    }

    #[test]
    fn transport_and_application_are_distinguishable() {
        assert!(McpCallError::transport("Transport closed").is_transport());
        assert!(!McpCallError::application("file not found").is_transport());
        assert_eq!(
            McpCallError::new(McpErrorKind::Transport, "eof").kind(),
            McpErrorKind::Transport
        );
    }
}
