//! A2A outbound tools — cross-instance agent communication.

use super::{check_taint_net_fetch, check_taint_outbound_text, require_kernel};
use crate::kernel_handle::prelude::*;
use librefang_types::taint::TaintSink;
use std::sync::Arc;

/// Discover an external A2A agent by fetching its agent card.
pub(super) async fn tool_a2a_discover(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

    // SSRF protection: block private/metadata IPs
    if crate::web_fetch::check_ssrf(url, &[]).is_err() {
        return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
    }

    let client = crate::a2a::A2aClient::new();
    let card = client.discover(url).await?;

    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

/// Send a task to an external A2A agent.
pub(super) async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Resolve agent URL: either directly provided or looked up by name.
    // Canonicalize early so the trust gate below sees the same string the
    // approve flow stored.
    let url = if let Some(raw) = input["agent_url"].as_str() {
        // SSRF protection
        if crate::web_fetch::check_ssrf(raw, &[]).is_err() {
            return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
        }
        crate::a2a::canonicalize_a2a_url(raw).unwrap_or_else(|| raw.to_string())
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name)
            .ok_or_else(|| format!("No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."))?
    } else {
        return Err("Missing 'agent_url' or 'agent_name' parameter".to_string());
    };

    // Taint sink: block secrets from being exfiltrated to an external A2A peer.
    // Runs before the trust gate so a tainted-message attempt always reports
    // the data-exfil reason (the test suite asserts this contract) — the
    // trust gate is purely about target authorization and would mask the
    // more serious finding.
    if let Some(violation) = check_taint_outbound_text(message, &TaintSink::agent_message()) {
        return Err(violation);
    }
    // Also gate the URL itself against query-string credential leaks.
    if let Some(violation) = check_taint_net_fetch(&url) {
        return Err(violation);
    }

    // SECURITY (Bug #3786): the HTTP route at `/api/a2a/send` enforces a
    // trust gate that requires the URL to live in `kernel.list_a2a_agents()`.
    // The agent-side tool path bypassed that gate entirely, so an LLM could
    // exfiltrate to any non-private URL the SSRF allowlist accepted. Mirror
    // the same check here.
    let trusted_urls: Vec<String> = kh.list_a2a_agents().into_iter().map(|(_, u)| u).collect();
    if !trusted_urls.iter().any(|u| u == &url) {
        return Err(format!(
            "A2A target '{url}' is not on the trusted-agent list. Discover and have an operator approve it via POST /api/a2a/agents/{{url}}/approve before agents may send to it."
        ));
    }

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client.send_task(&url, message, session_id).await?;

    serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
}
