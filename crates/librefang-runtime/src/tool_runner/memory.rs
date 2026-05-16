//! Per-agent memory tools backed by `KernelHandle::memory_*`.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    agent_id: Option<&str>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    kh.memory_store(key, value.clone(), agent_id, peer_id)
        .map_err(|e| e.to_string())?;
    Ok(format!("Stored value under key '{key}'."))
}

pub(super) fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    agent_id: Option<&str>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    match kh
        .memory_recall(key, agent_id, peer_id)
        .map_err(|e| e.to_string())?
    {
        Some(val) => Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())),
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

pub(super) fn tool_memory_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    agent_id: Option<&str>,
    peer_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let keys = kh
        .memory_list(agent_id, peer_id)
        .map_err(|e| e.to_string())?;
    if keys.is_empty() {
        return Ok("No entries found in the agent's memory.".to_string());
    }
    Ok(serde_json::to_string_pretty(&keys).unwrap_or_else(|_| format!("{:?}", keys)))
}
