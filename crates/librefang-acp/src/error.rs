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
}

impl AcpError {
    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Convert this error into an `agent_client_protocol::Error` suitable
    /// for returning from a request handler.
    pub fn into_acp_error(self) -> agent_client_protocol::Error {
        use agent_client_protocol::util::internal_error;
        match self {
            Self::Transport(e) => e,
            Self::UnknownSession(_) | Self::AgentNotFound(_) => {
                let msg = self.to_string();
                agent_client_protocol::Error::invalid_params()
                    .data(serde_json::json!({ "reason": msg }))
            }
            other => internal_error(other.to_string()),
        }
    }
}

pub type AcpResult<T> = Result<T, AcpError>;
