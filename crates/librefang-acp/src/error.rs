//! Error type for the ACP adapter.

use thiserror::Error;

/// Errors raised by the ACP adapter.
///
/// Most variants wrap kernel-level [`librefang_types::error::LibreFangError`]
/// or transport-level [`agent_client_protocol::Error`] so the runtime can
/// translate them back into JSON-RPC error responses.
#[derive(Debug, Error)]
pub enum AcpError {
    /// The ACP `session_id` supplied by the client does not correspond to a
    /// session created by `session/new`.
    #[error("unknown ACP session id: {0}")]
    UnknownSession(String),

    /// The agent name/id supplied at startup or via `_meta` cannot be
    /// resolved to a live agent.
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    /// The kernel returned a structured error.
    #[error("kernel error: {0}")]
    Kernel(#[from] librefang_types::error::LibreFangError),

    /// The underlying ACP transport (JSON-RPC framing or peer disconnect).
    #[error("acp transport error: {0}")]
    Transport(#[from] agent_client_protocol::Error),

    /// `session/prompt` was invoked while another prompt for the same
    /// session was still in flight. ACP guarantees one prompt per session
    /// at a time, so this should never fire in conformant clients.
    #[error("session {0} already has an in-flight prompt")]
    PromptInFlight(String),

    /// Generic catch-all for unexpected internal failures (channel closed,
    /// task panic, …). Translated to JSON-RPC `internal_error`.
    #[error("internal acp error: {0}")]
    Internal(String),

    /// An optional ACP capability became temporarily unavailable.
    /// Kernel-facing adapters map this to `KernelOpError::Unavailable` so callers can use their documented local fallback.
    #[error("acp capability unavailable: {0}")]
    Unavailable(String),
}

impl AcpError {
    /// Construct an [`AcpError::Internal`] from a message. Used by the
    /// `librefang-cli` kernel adapter when wrapping unexpected join /
    /// channel failures.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Construct an [`AcpError::Unavailable`] for a reverse-RPC transport that can no longer serve an optional editor capability.
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self::Unavailable(msg.into())
    }

    /// Convert this error into an `agent_client_protocol::Error` suitable
    /// for returning from a request handler.
    pub fn into_acp_error(self) -> agent_client_protocol::Error {
        use agent_client_protocol::util::internal_error;
        match self {
            Self::Transport(e) => e,
            error @ (Self::UnknownSession(_) | Self::AgentNotFound(_)) => {
                let msg = error.to_string();
                agent_client_protocol::Error::invalid_params()
                    .data(serde_json::json!({ "reason": msg }))
            }
            Self::PromptInFlight(session_id) => agent_client_protocol::Error::invalid_params()
                .data(serde_json::json!({
                    "reason": format!("session {session_id} already has an in-flight prompt")
                })),
            error => {
                tracing::error!(error = %error, "ACP request failed");
                internal_error("internal acp error")
            }
        }
    }
}

pub type AcpResult<T> = Result<T, AcpError>;

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::ErrorCode;

    #[test]
    fn prompt_in_flight_is_an_invalid_params_error() {
        let error = AcpError::PromptInFlight("session-7".to_string()).into_acp_error();

        assert_eq!(error.code, ErrorCode::InvalidParams);
        assert_eq!(
            error.data,
            Some(serde_json::json!({
                "reason": "session session-7 already has an in-flight prompt"
            }))
        );
    }

    #[test]
    fn internal_error_details_are_not_exposed_to_clients() {
        let error = AcpError::internal("channel dropped at /private/agent.sock").into_acp_error();

        assert_eq!(error.code, ErrorCode::InternalError);
        assert_eq!(error.data, Some(serde_json::json!("internal acp error")));
    }

    #[test]
    fn kernel_error_details_are_not_exposed_to_clients() {
        let error = AcpError::Kernel(librefang_types::error::LibreFangError::Config(
            "secret config path: /private/config.toml".to_string(),
        ))
        .into_acp_error();

        assert_eq!(error.code, ErrorCode::InternalError);
        assert_eq!(error.data, Some(serde_json::json!("internal acp error")));
    }
}
