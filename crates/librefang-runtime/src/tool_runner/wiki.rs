//! Memory-wiki tools (issue #3329).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). The per-user ACL gate (`enforce_memory_acl`, shared and still
//! `Result<_, String>`) returns only its Deny message on `Err`, so it maps to
//! `ToolError::PermissionDenied`; the message text is preserved verbatim.

use super::error::{ToolError, ToolResult};
use super::{enforce_memory_acl, require_kernel_typed, MemoryAclOp};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

/// The wiki vault is a single shared knowledge base (not peer-scoped), so it
/// maps to one ACL namespace. `default_memory_acl` grants this to every role
/// (read for `viewer`, read+write for `user`, `*` for owner/admin) so the
/// pre-#5139 "all attributed users may use the wiki" behaviour is preserved;
/// an operator who sets an explicit `memory_access` can now restrict it.
const WIKI_NAMESPACE: &str = "wiki";

pub(super) fn tool_wiki_get(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let topic = input["topic"]
        .as_str()
        .ok_or(ToolError::MissingParameter("topic"))?;
    // #5139: gate the read on the per-user ACL before hitting the vault.
    enforce_memory_acl(
        kernel,
        sender_id,
        channel,
        MemoryAclOp::Read,
        WIKI_NAMESPACE,
    )
    .map_err(ToolError::PermissionDenied)?;
    let value = kh.wiki_get(topic).map_err(ToolError::upstream)?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let query = input["query"]
        .as_str()
        .ok_or(ToolError::MissingParameter("query"))?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(10);
    // #5139: search reads page bodies — gate it the same as `wiki_get`.
    enforce_memory_acl(
        kernel,
        sender_id,
        channel,
        MemoryAclOp::Read,
        WIKI_NAMESPACE,
    )
    .map_err(ToolError::PermissionDenied)?;
    let value = kh.wiki_search(query, limit).map_err(ToolError::upstream)?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let topic = input["topic"]
        .as_str()
        .ok_or(ToolError::MissingParameter("topic"))?;
    let body = input["body"]
        .as_str()
        .ok_or(ToolError::MissingParameter("body"))?;
    let force = input
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // #5139: enforce the per-user write ACL before mutating the shared vault.
    enforce_memory_acl(
        kernel,
        sender_id,
        channel,
        MemoryAclOp::Write,
        WIKI_NAMESPACE,
    )
    .map_err(ToolError::PermissionDenied)?;

    // Provenance is constructed kernel-side rather than left to the LLM:
    // (1) every write is required to carry an agent attribution per #3329's
    //     acceptance criterion #3, and (2) the calling agent / sender ids
    //     are authoritative — letting the model spoof them would defeat the
    //     audit value of the frontmatter.
    let agent = caller_agent_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    // Keep `channel` and `sender` as DISTINCT fields in the audit
    // frontmatter: `channel` is the transport/room (telegram, slack, "cron",
    // …) and `sender` is the attributed user. Conflating them — as an
    // earlier draft did by writing `sender_id` into the `channel` slot —
    // pollutes the wiki history with channel rows that actually identify
    // users, defeating the audit value of the provenance trail.
    let provenance = serde_json::json!({
        "agent": agent,
        "channel": channel,
        "sender": sender_id,
        "at": chrono::Utc::now().to_rfc3339(),
    });

    let value = kh
        .wiki_write(topic, body, provenance, force)
        .map_err(ToolError::upstream)?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wiki_get_without_kernel_returns_unavailable() {
        let r = tool_wiki_get(&json!({"topic": "x"}), None, None, None);
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn wiki_search_without_kernel_returns_unavailable() {
        let r = tool_wiki_search(&json!({"query": "x"}), None, None, None);
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn wiki_write_without_kernel_returns_unavailable() {
        let r = tool_wiki_write(&json!({"topic": "x", "body": "y"}), None, None, None, None);
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }
}
