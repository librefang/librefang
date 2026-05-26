//! Shared memory tools backed by `KernelHandle::memory_*`.

use super::{enforce_memory_acl, kv_acl_namespace, require_kernel, MemoryAclOp};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

const MAX_KEY_LEN: usize = 256;
const MAX_RECALL_BYTES: usize = 64 * 1024;
const DEFAULT_LIST_LIMIT: usize = 100;

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Memory key must not be empty".to_string());
    }
    if key.len() > MAX_KEY_LEN {
        return Err(format!(
            "Memory key too long: {} bytes (max {MAX_KEY_LEN})",
            key.len()
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(
            "Memory key contains invalid characters (allowed: alphanumeric, _, -, .)".to_string(),
        );
    }
    Ok(())
}

fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut truncated = s[..boundary].to_string();
    truncated.push_str("... [truncated]");
    truncated
}

pub(super) fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    validate_key(key)?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Write,
        &kv_acl_namespace(peer_id),
    )
    .map_err(|e| e.to_string())?;
    kh.memory_store(key, value.clone(), caller_agent_id, peer_id)
        .map_err(|e| e.to_string())?;
    Ok(format!("Stored value under key '{key}'."))
}

pub(super) fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    validate_key(key)?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )
    .map_err(|e| e.to_string())?;
    match kh
        .memory_recall(key, caller_agent_id, peer_id)
        .map_err(|e| e.to_string())?
    {
        Some(val) => {
            let rendered = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
            Ok(truncate_output(&rendered, MAX_RECALL_BYTES))
        }
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

pub(super) fn tool_memory_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )
    .map_err(|e| e.to_string())?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LIST_LIMIT);
    let offset = input
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0);
    let keys = kh
        .memory_list(caller_agent_id, peer_id)
        .map_err(|e| e.to_string())?;
    if keys.is_empty() {
        return Ok("No entries found in this agent's memory.".to_string());
    }
    let total = keys.len();
    let sliced: Vec<_> = keys.into_iter().skip(offset).take(limit).collect();
    if sliced.is_empty() {
        return Ok(format!(
            "No entries in range (offset={offset}, limit={limit}, total={total})."
        ));
    }
    let mut out = serde_json::to_string_pretty(&sliced).unwrap_or_else(|_| format!("{:?}", sliced));
    if total > offset + sliced.len() {
        out.push_str(&format!(
            "\n\nShowing {shown} of {total} entries (offset={offset}). Use offset={next} to see more.",
            shown = sliced.len(),
            next = offset + sliced.len(),
        ));
    }
    Ok(out)
}
