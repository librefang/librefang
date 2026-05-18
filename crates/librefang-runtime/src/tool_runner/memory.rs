//! Shared memory tools backed by `KernelHandle::memory_*`.

use super::{enforce_memory_acl, kv_acl_namespace, require_kernel, MemoryAclOp};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    // #5139: enforce the per-user write ACL BEFORE touching the substrate.
    // `peer_id` is the attributed sender, also used as the channel-derived
    // platform id for the ACL resolver.
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Write,
        &kv_acl_namespace(peer_id),
    )?;
    kh.memory_store(key, value.clone(), peer_id)
        .map_err(|e| e.to_string())?;
    Ok(format!("Stored value under key '{key}'."))
}

pub(super) fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    // #5139: enforce the per-user read ACL before recall.
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )?;
    match kh.memory_recall(key, peer_id).map_err(|e| e.to_string())? {
        Some(val) => Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())),
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

pub(super) fn tool_memory_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    // #5139: enumerating a namespace's keys is a read; gate it the same way
    // as recall so a restricted user cannot probe another scope's key set.
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )?;
    let keys = kh.memory_list(peer_id).map_err(|e| e.to_string())?;
    if keys.is_empty() {
        return Ok("No entries found in shared memory.".to_string());
    }
    Ok(serde_json::to_string_pretty(&keys).unwrap_or_else(|_| format!("{:?}", keys)))
}
