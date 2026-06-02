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
    )?;
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
    // Memory reads degrade gracefully: a missing key, a denied ACL, or a
    // backend error returns an explanatory `Ok` rather than `Err`. An `Err`
    // here is a *hard* tool failure, and three consecutive hard failures abort
    // the whole turn (`MAX_CONSECUTIVE_ALL_FAILED` in agent_loop). A
    // non-essential recall that fails deterministically (e.g. the same ACL
    // denial every time) would otherwise be retried into a death spiral that
    // kills the turn and discards the user's actual request — so we never let
    // an optional read produce a hard failure.
    let Some(key) = input["key"].as_str() else {
        return Ok("memory_recall needs a 'key' (the exact storage key to look \
                   up). Call memory_list to see the available keys."
            .to_string());
    };
    if let Err(reason) = enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    ) {
        tracing::warn!(%key, %reason, "memory_recall denied by ACL — continuing without it");
        return Ok(format!("Could not read memory: {reason}"));
    }
    match kh.memory_recall(key, caller_agent_id, peer_id) {
        Ok(Some(val)) => {
            let rendered = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
            Ok(truncate_output(&rendered, MAX_RECALL_BYTES))
        }
        Ok(None) => Ok(format!("No value found for key '{key}'.")),
        Err(e) => {
            let e = e.to_string();
            tracing::warn!(%key, error = %e, "memory_recall backend error — continuing without it");
            Ok(format!(
                "Could not read memory for key '{key}': {e}. Continuing without it."
            ))
        }
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
    // Reads degrade gracefully — see `tool_memory_recall` for why an optional
    // memory read must never produce a hard tool failure.
    if let Err(reason) = enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    ) {
        tracing::warn!(%reason, "memory_list denied by ACL — continuing without it");
        return Ok(format!("Could not list memory: {reason}"));
    }
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
    let keys = match kh.memory_list(caller_agent_id, peer_id) {
        Ok(keys) => keys,
        Err(e) => {
            let e = e.to_string();
            tracing::warn!(error = %e, "memory_list backend error — continuing without it");
            return Ok(format!("Could not list memory: {e}. Continuing without it."));
        }
    };
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
