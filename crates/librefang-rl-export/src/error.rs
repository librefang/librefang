//! Error types for the RL trajectory exporter.
//!
//! The exporter speaks to several different upstream services (W&B today,
//! Tinker and Atropos in follow-up PRs) over HTTP. The error enum is kept
//! flat and string-payload-heavy on purpose: callers (CLI, dashboard,
//! runtime telemetry hook) generally want to render the upstream's own
//! message back to the operator rather than translate it. Distinct
//! variants only exist where the call site needs to branch on the cause
//! (auth retry vs upstream-4xx surface vs transport bounce).

use thiserror::Error;

/// Errors that can occur while exporting a trajectory.
///
/// `NetworkError` is the catch-all for transport-layer failures (DNS,
/// connect, read timeout, TLS, …). Surface the inner message verbatim;
/// upstream-specific 4xx bodies use `UpstreamRejected` instead so the
/// status code stays inspectable.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Transport-level failure talking to the upstream — DNS, TCP/TLS,
    /// read timeout, malformed response framing, etc. The wrapped string
    /// carries reqwest's own message.
    #[error("network error: {0}")]
    NetworkError(String),

    /// Authentication was rejected (HTTP 401 / 403) or the supplied
    /// API key was empty. Distinct from `UpstreamRejected` so callers
    /// can prompt the operator to refresh credentials without surfacing
    /// the raw body (which often contains the rejected token in error
    /// text on some upstreams).
    #[error("authentication rejected by upstream")]
    AuthError,

    /// Upstream returned a non-2xx status that is not an auth failure.
    /// Body is forwarded verbatim so the operator sees the upstream's
    /// own diagnostic (e.g. "project does not exist", "quota exceeded").
    #[error("upstream rejected request: status={status} body={body}")]
    UpstreamRejected {
        /// HTTP status code returned by the upstream.
        status: u16,
        /// Response body as a UTF-8 string (lossy decoded if the body
        /// was not valid UTF-8). Truncated to 4 KiB before storage so
        /// pathological upstream payloads cannot bloat the error.
        body: String,
    },

    /// The exporter could not parse the upstream's response as the
    /// expected shape (missing field, wrong type). Indicates the upstream
    /// API changed; callers should treat this as a hard failure rather
    /// than retry.
    #[error("malformed upstream response: {0}")]
    MalformedResponse(String),

    /// Configuration error caught before any network I/O — e.g. empty
    /// API key, malformed run URL hint. The operator's config needs to
    /// change; no retry will help.
    #[error("invalid export configuration: {0}")]
    InvalidConfig(String),
}

impl From<reqwest::Error> for ExportError {
    fn from(err: reqwest::Error) -> Self {
        ExportError::NetworkError(err.to_string())
    }
}
