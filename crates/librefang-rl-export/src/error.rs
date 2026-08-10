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
///
/// `#[non_exhaustive]` so future variants (e.g. a structured
/// `RateLimited { retry_after }`) can land non-breaking — matches the
/// stance on `ExportTarget` and reserves room for the next obvious
/// addition.
#[non_exhaustive]
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

    /// Atropos-specific: the local trainer process has not finished
    /// booting (`register-env` returned 200 with the sentinel body
    /// `{"status": "wait for trainer to start"}` and no `env_id`).
    /// Caller should poll with backoff until the trainer is ready.
    /// Distinct variant rather than a synthetic 503 so callers can
    /// branch on the condition without parsing the body, and so the
    /// synthesised status doesn't collide with a real 503 from the
    /// upstream (refs PR review nit).
    #[error("atropos trainer not ready: {status_label}")]
    TrainerNotReady {
        /// Status string echoed from the Atropos `register-env` 200-as-
        /// busy sentinel body (typically `"wait for trainer to start"`).
        status_label: String,
    },
}

impl From<reqwest::Error> for ExportError {
    fn from(err: reqwest::Error) -> Self {
        ExportError::NetworkError(err.to_string())
    }
}

/// Maximum upstream response body size kept on an error. Larger bodies
/// are truncated so a pathological upstream cannot bloat the returned
/// [`ExportError::UpstreamRejected`]. Shared across all exporters so
/// the error surface stays uniform.
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 4096;

/// Map an HTTP status + body to the appropriate `ExportError` variant.
/// 401 / 403 collapse into `AuthError` so callers can prompt for a
/// fresh credential without showing the raw body (some upstreams echo
/// the rejected token). All other non-2xx codes surface as
/// `UpstreamRejected` with the (already-truncated) body forwarded.
pub(crate) fn classify_status(status: u16, body: String) -> ExportError {
    if status == 401 || status == 403 {
        ExportError::AuthError
    } else {
        ExportError::UpstreamRejected { status, body }
    }
}

/// Read at most [`MAX_ERROR_BODY_BYTES`] from an error response, then stop
/// consuming the stream. Lossy UTF-8 decoding ensures an upstream that
/// returns non-text bytes still surfaces *something* without buffering its
/// complete body first.
pub(crate) async fn read_body_truncated(mut resp: reqwest::Response) -> String {
    let mut bytes = Vec::with_capacity(MAX_ERROR_BODY_BYTES);
    while bytes.len() < MAX_ERROR_BODY_BYTES {
        let chunk = match resp.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => return format!("<error reading body: {e}>"),
        };
        let remaining = MAX_ERROR_BODY_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Classify a `reqwest::Error` raised by [`reqwest::Response::json`].
/// Inside that call, the body is first streamed off the wire and then
/// deserialized — either step can fail. Splitting them keeps the error
/// taxonomy honest:
///
/// - **Decode failure** (`is_decode()`): the body arrived intact but
///   didn't match the expected shape. Upstream contract drift; surface
///   as [`ExportError::MalformedResponse`].
/// - **Anything else** (transport drop while reading the body, TLS
///   reset mid-read, …): treat as [`ExportError::NetworkError`] so a
///   transient blip isn't mislabelled as an upstream API change.
///
/// `context` identifies the call site (e.g. `"create-run JSON"`) and
/// is prepended to the message on the decode path so the operator can
/// tell which response failed to parse.
pub(crate) fn classify_response_decode_error(err: reqwest::Error, context: &str) -> ExportError {
    if err.is_decode() {
        ExportError::MalformedResponse(format!("{context}: {err}"))
    } else {
        ExportError::NetworkError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn truncated_body_returns_without_buffering_the_declared_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\n\
                      Content-Length: 1000000000\r\n\
                      Connection: close\r\n\r\n",
                )
                .await
                .unwrap();
            stream
                .write_all(&vec![b'x'; MAX_ERROR_BODY_BYTES + 1])
                .await
                .unwrap();
            stream.flush().await.unwrap();
            std::future::pending::<()>().await;
        });

        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/oversized"))
            .send()
            .await
            .unwrap();
        let body = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_body_truncated(response),
        )
        .await
        .expect("the cap must stop reading before the declared body completes");

        server.abort();
        let _ = server.await;
        assert_eq!(body.len(), MAX_ERROR_BODY_BYTES);
        assert!(body.bytes().all(|byte| byte == b'x'));
    }
}
