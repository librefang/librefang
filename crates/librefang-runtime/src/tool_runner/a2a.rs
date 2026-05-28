//! A2A outbound tools — cross-instance agent communication.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). The security blocks here (SSRF, taint sinks, trusted-agent gate)
//! keep their exact messages — they are wrapped in message-preserving
//! `ToolError` variants (`PermissionDenied` / `InvalidParameter`) so the wire
//! string the LLM and operator logs see is unchanged.

use super::error::{ToolError, ToolResult};
use super::{check_taint_net_fetch, check_taint_outbound_text, require_kernel_typed};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::sync::Arc;

/// Discover an external A2A agent by fetching its agent card.
pub(super) async fn tool_a2a_discover(input: &serde_json::Value) -> ToolResult {
    let url = input["url"]
        .as_str()
        .ok_or(ToolError::MissingParameter("url"))?;

    // SSRF protection: block private/metadata IPs
    if crate::web_fetch::check_ssrf(url, &[]).is_err() {
        return Err(ToolError::InvalidParameter {
            name: "url",
            reason: "SSRF blocked: URL resolves to a private or metadata address".to_string(),
        });
    }

    let client = crate::a2a::A2aClient::new();
    let card = client
        .discover(url)
        .await
        .map_err(ToolError::upstream_msg)?;

    Ok(serde_json::to_string_pretty(&card)?)
}

/// Send a task to an external A2A agent.
pub(super) async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or(ToolError::MissingParameter("message"))?;

    // Resolve agent URL: either directly provided or looked up by name.
    // Canonicalize early so the trust gate below sees the same string the
    // approve flow stored.
    let url = if let Some(raw) = input["agent_url"].as_str() {
        // SSRF protection
        if crate::web_fetch::check_ssrf(raw, &[]).is_err() {
            return Err(ToolError::InvalidParameter {
                name: "agent_url",
                reason: "SSRF blocked: URL resolves to a private or metadata address".to_string(),
            });
        }
        crate::a2a::canonicalize_a2a_url(raw).unwrap_or_else(|| raw.to_string())
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name).ok_or_else(|| {
            // Reason kept byte-identical to the pre-#3576 String error.
            let reason = format!(
                "No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."
            );
            ToolError::InvalidParameter {
                name: "agent_name",
                reason,
            }
        })?
    } else {
        return Err(ToolError::InvalidParameter {
            name: "agent_url",
            reason: "provide either 'agent_url' or 'agent_name'".to_string(),
        });
    };

    // Taint sink: block secrets from being exfiltrated to an external A2A peer.
    // Runs before the trust gate so a tainted-message attempt always reports
    // the data-exfil reason (the test suite asserts this contract) — the
    // trust gate is purely about target authorization and would mask the
    // more serious finding. The original violation message is preserved inside
    // PermissionDenied so its "taint"/"violation" wording survives.
    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(ToolError::PermissionDenied(violation));
    }
    // Also gate the URL itself against query-string credential leaks.
    if let Some(violation) = check_taint_net_fetch(&url) {
        return Err(ToolError::PermissionDenied(violation));
    }

    // SECURITY (Bug #3786): the HTTP route at `/api/a2a/send` enforces a
    // trust gate that requires the URL to live in `kernel.list_a2a_agents()`.
    // The agent-side tool path bypassed that gate entirely, so an LLM could
    // exfiltrate to any non-private URL the SSRF allowlist accepted. Mirror
    // the same check here.
    let trusted_urls: Vec<String> = kh.list_a2a_agents().into_iter().map(|(_, u)| u).collect();
    if !trusted_urls.iter().any(|u| u == &url) {
        return Err(ToolError::PermissionDenied(format!(
            "A2A target '{url}' is not on the trusted-agent list. Discover and have an operator approve it via POST /api/a2a/agents/{{url}}/approve before agents may send to it."
        )));
    }

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client
        .send_task(&url, message, session_id)
        .await
        .map_err(ToolError::upstream_msg)?;

    Ok(serde_json::to_string_pretty(&task)?)
}

#[cfg(test)]
mod tests {
    //! Boundary tests that run before any network / kernel call. The taint
    //! sink and trust-gate paths are exercised against a real kernel stub in
    //! `tool_runner::tests` (`test_tool_a2a_send_blocks_secret_in_message`).
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a2a_discover_missing_url_is_missing_parameter() {
        let r = tool_a2a_discover(&json!({})).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("url"))));
    }

    #[tokio::test]
    async fn a2a_send_without_kernel_returns_unavailable() {
        let r = tool_a2a_send(&json!({"message": "hi"}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }
}
