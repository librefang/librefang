//! Memory-wiki tools (issue #3329).

use super::{enforce_memory_acl, require_kernel, MemoryAclOp};
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
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    // #5139: gate the read on the per-user ACL before hitting the vault.
    enforce_memory_acl(
        kernel,
        sender_id,
        channel,
        MemoryAclOp::Read,
        WIKI_NAMESPACE,
    )?;
    let value = kh.wiki_get(topic).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
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
    )?;
    let value = kh.wiki_search(query, limit).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
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
    )?;

    // Provenance is constructed kernel-side rather than left to the LLM:
    // (1) every write is required to carry an agent attribution per #3329's
    //     acceptance criterion #3, and (2) the calling agent / sender ids
    //     are authoritative — letting the model spoof them would defeat the
    //     audit value of the frontmatter.
    let agent = caller_agent_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let provenance = serde_json::json!({
        "agent": agent,
        "channel": sender_id,
        "at": chrono::Utc::now().to_rfc3339(),
    });

    let value = kh
        .wiki_write(topic, body, provenance, force)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}
