//! Memory-wiki tools (issue #3329).

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) fn tool_wiki_get(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    let value = kh.wiki_get(topic).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(10);
    let value = kh.wiki_search(query, limit).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()))
}

pub(super) fn tool_wiki_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let topic = input["topic"].as_str().ok_or("Missing 'topic' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
    let force = input
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

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
