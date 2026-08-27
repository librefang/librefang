//! Shared memory tools backed by `KernelHandle::memory_*`.

use super::{enforce_memory_acl, kv_acl_namespace, require_kernel_typed, MemoryAclOp, ToolError};
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
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let key = input["key"]
        .as_str()
        .ok_or(ToolError::MissingParameter("key"))?;
    validate_key(key).map_err(|reason| ToolError::InvalidParameter {
        name: "key",
        reason,
    })?;
    let value = input
        .get("value")
        .ok_or(ToolError::MissingParameter("value"))?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Write,
        &kv_acl_namespace(peer_id),
    )
    .map_err(ToolError::PermissionDenied)?;
    kh.memory_store(key, value.clone(), caller_agent_id, peer_id)
        .map_err(ToolError::upstream)?;
    Ok(format!("Stored value under key '{key}'."))
}

pub(super) fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let key = input["key"]
        .as_str()
        .ok_or(ToolError::MissingParameter("key"))?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )
    .map_err(ToolError::PermissionDenied)?;
    match kh
        .memory_recall(key, caller_agent_id, peer_id)
        .map_err(ToolError::upstream)?
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
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    enforce_memory_acl(
        kernel,
        peer_id,
        channel,
        MemoryAclOp::Read,
        &kv_acl_namespace(peer_id),
    )
    .map_err(ToolError::PermissionDenied)?;
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
        .map_err(ToolError::upstream)?;
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

// ============================================================================
// Semantic memory tools (#7808)
// ============================================================================
//
// The three tools above are exact-key KV against `kv_store`. The four below are
// the embedding-backed `memories` table — the substrate the agent loop already
// injects recalls from, which until now had no agent-callable surface at all.
//
// The `memory_semantic_` prefix is load-bearing: `memory_store` (keyed, KV) and
// a hypothetical `memory_remember` (keyless, semantic) would read as synonyms
// and get picked at random, which is exactly the failure the `memory_search`
// alias produced.
//
// ACL: these gate on the `proactive` namespace, not `kv:{peer}`, and the gate
// lives kernel-side rather than in `enforce_memory_acl` here — see the design
// note on `MemoryAccess::memory_semantic_search`. The one thing this layer must
// not do is resolve the guard itself, because that would skip PII redaction.

/// Fragments returned by one `memory_semantic_search` call when the model does not ask for a limit.
///
/// Matched `MEMORY_RECALL_LIMIT` in `agent_loop::prompt` until #7920 split that constant into a per-class pair and widened the fact half; it now matches `MEMORY_RECALL_LIMIT_DIALOGUE`, the half that kept the historical window.
/// Deliberately not widened alongside the automatic recall: this is a tool call the model issues with a question in hand, and its results are read one at a time rather than budgeted per class into a section, so breadth here is straight context cost with none of the offsetting structure.
/// The model can still ask for more, up to `MAX_SEMANTIC_SEARCH_LIMIT`.
const DEFAULT_SEMANTIC_SEARCH_LIMIT: u64 = 5;
/// Hard ceiling on `limit`. Each fragment is free-form text that lands in the
/// prompt verbatim, so an unbounded limit is a context-window footgun.
const MAX_SEMANTIC_SEARCH_LIMIT: u64 = 50;
/// Longest `content` accepted by `memory_semantic_add`. Semantic memories are
/// meant to be distilled facts; anything longer is a document and belongs in a
/// file or the wiki.
const MAX_SEMANTIC_CONTENT_LEN: usize = 8 * 1024;

/// Lift a kernel-handle error into a `ToolError`, preserving an ACL refusal as
/// `PermissionDenied`.
///
/// This matters for more than the message: `ToolError::execution_status()` maps
/// `PermissionDenied` to the soft `ToolExecutionStatus::Denied` and everything
/// else to hard `Error`. A per-user policy refusal is permanent and non-fatal —
/// routing it through `Upstream` would count each one toward the agent loop's
/// consecutive-hard-failure abort and death-spiral the turn, which is exactly
/// what #5984 fixed for the KV tools. The semantic tools resolve their ACL
/// kernel-side, so the refusal arrives as `AuthDenied` rather than from
/// `enforce_memory_acl`, and needs its own mapping.
fn semantic_tool_error(err: librefang_types::error::LibreFangError) -> ToolError {
    match err {
        librefang_types::error::LibreFangError::AuthDenied(reason)
        | librefang_types::error::LibreFangError::CapabilityDenied(reason) => {
            ToolError::PermissionDenied(reason)
        }
        other => ToolError::upstream(other),
    }
}

fn require_caller_agent<'a>(
    caller_agent_id: Option<&'a str>,
    tool: &'static str,
) -> Result<&'a str, ToolError> {
    caller_agent_id.ok_or_else(|| super::caller_agent_id_missing(tool))
}

/// Read `limit`, clamped to [1, [`MAX_SEMANTIC_SEARCH_LIMIT`]].
///
/// Clamping rather than erroring: a model that asks for 500 wants "as many as
/// you can", and failing the call teaches it nothing it can act on.
fn semantic_limit(input: &serde_json::Value) -> usize {
    input
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_SEMANTIC_SEARCH_LIMIT)
        .clamp(1, MAX_SEMANTIC_SEARCH_LIMIT) as usize
}

fn semantic_min_confidence(input: &serde_json::Value) -> Result<Option<f32>, ToolError> {
    match input.get("min_confidence").and_then(|v| v.as_f64()) {
        None => Ok(None),
        Some(f) if (0.0..=1.0).contains(&f) => Ok(Some(f as f32)),
        Some(f) => Err(ToolError::InvalidParameter {
            name: "min_confidence",
            reason: format!("must be between 0.0 and 1.0, got {f}"),
        }),
    }
}

/// Read `min_similarity`, the cosine floor below which a fragment is not
/// returned at all.
///
/// Range is `[-1.0, 1.0]` rather than `[0.0, 1.0]`: cosine is genuinely
/// signed, and rejecting a negative floor would refuse a legitimate (if
/// unusual) "anything not actively opposite" request.
fn semantic_min_similarity(input: &serde_json::Value) -> Result<Option<f32>, ToolError> {
    match input.get("min_similarity").and_then(|v| v.as_f64()) {
        None => Ok(None),
        Some(f) if (-1.0..=1.0).contains(&f) => Ok(Some(f as f32)),
        Some(f) => Err(ToolError::InvalidParameter {
            name: "min_similarity",
            reason: format!("must be between -1.0 and 1.0, got {f}"),
        }),
    }
}

/// Render fragments as the JSON the model can act on: an `id` it can pass back
/// to `memory_semantic_forget`, plus the content and the decay metadata that
/// tells it how much to trust the fragment.
fn render_fragments(items: &[librefang_types::memory::MemoryItem]) -> String {
    let rows: Vec<serde_json::Value> = items
        .iter()
        .map(|i| {
            let mut row = serde_json::json!({
                "id": i.id,
                "content": i.content,
                "level": i.level.scope_str(),
                "category": i.category,
                "confidence": i.confidence,
                "created_at": i.created_at.to_rfc3339(),
            });
            // Present only when something measured it — a text-match fallback
            // or a row with no stored embedding measures nothing (#7808).
            // Emitting `0.0` there would read as "measured, and irrelevant",
            // and even `null` invites the model to compare it against a floor.
            if let (Some(score), Some(obj)) = (i.similarity, row.as_object_mut()) {
                obj.insert("similarity".to_string(), serde_json::json!(score));
            }
            row
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| format!("{rows:?}"))
}

pub(super) async fn tool_memory_semantic_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_search")?;
    let query = input["query"]
        .as_str()
        .ok_or(ToolError::MissingParameter("query"))?;
    if query.trim().is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "query",
            reason: "must not be empty or whitespace".to_string(),
        });
    }
    let limit = semantic_limit(input);
    let min_confidence = semantic_min_confidence(input)?;
    let min_similarity = semantic_min_similarity(input)?;
    let items = kh
        .memory_semantic_search(
            query,
            agent,
            limit,
            min_confidence,
            min_similarity,
            peer_id,
            channel,
        )
        .await
        .map_err(semantic_tool_error)?;
    if items.is_empty() {
        // Say which store was searched. "No results" from a tool named
        // `memory_*` is otherwise indistinguishable from a KV key miss, and the
        // model will retry the wrong tool.
        // Naming the floor matters as much as naming the store: with a floor
        // set, "no results" can mean "nothing cleared the bar" rather than
        // "nothing is stored", and a model that cannot tell those apart will
        // conclude its memory is empty and stop asking.
        let floor_note = match min_similarity {
            Some(f) => format!(
                " Nothing scored at or above min_similarity {f}; retry with a lower floor, or \
                 omit it, to see what the closest matches actually were."
            ),
            None => String::new(),
        };
        return Ok(format!(
            "No semantic memories matched '{query}'. (Searched this agent's semantic memory; \
             key/value entries are a separate store — use memory_list / memory_recall for those.){floor_note}"
        ));
    }
    Ok(render_fragments(&items))
}

pub(super) async fn tool_memory_semantic_add(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_add")?;
    let content = input["content"]
        .as_str()
        .ok_or(ToolError::MissingParameter("content"))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "content",
            reason: "must not be empty or whitespace".to_string(),
        });
    }
    if trimmed.len() > MAX_SEMANTIC_CONTENT_LEN {
        return Err(ToolError::InvalidParameter {
            name: "content",
            reason: format!(
                "too long: {} bytes (max {MAX_SEMANTIC_CONTENT_LEN}). Semantic memories hold \
                 distilled facts; store documents as files instead.",
                trimmed.len()
            ),
        });
    }
    let stored = kh
        .memory_semantic_add(trimmed, agent, peer_id, channel)
        .await
        .map_err(semantic_tool_error)?;
    if stored.is_empty() {
        // Report the no-op instead of an unqualified "stored". The extraction
        // pipeline legitimately declines input it finds redundant or
        // fact-free, and an agent told "stored" would never retry or fall back
        // to `memory_store`.
        return Ok(
            "Nothing was stored: the memory extractor found no durable fact in that content \
             (it may already be recorded). Rephrase it as a standalone fact, or use memory_store \
             for an exact key/value entry."
                .to_string(),
        );
    }
    Ok(format!(
        "Stored {} semantic memor{}:\n{}",
        stored.len(),
        if stored.len() == 1 { "y" } else { "ies" },
        render_fragments(&stored)
    ))
}

pub(super) async fn tool_memory_semantic_forget(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_forget")?;
    let memory_id = input["memory_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("memory_id"))?;
    let memory_id = memory_id.trim();
    // Validate the id shape here rather than letting the store reject it.
    // `ProactiveMemory::delete` maps a malformed id to `Internal`, which the
    // dispatcher surfaces as a HARD tool failure — three of those in a row abort
    // the turn. A hallucinated id is a caller mistake the model can correct, so
    // it has to arrive as `InvalidParameter` with the recovery step named.
    if uuid::Uuid::parse_str(memory_id).is_err() {
        return Err(ToolError::InvalidParameter {
            name: "memory_id",
            reason: "must be a memory id (UUID) as returned by memory_semantic_search".to_string(),
        });
    }
    let deleted = kh
        .memory_semantic_forget(memory_id, agent, peer_id, channel)
        .await
        .map_err(semantic_tool_error)?;
    if deleted {
        Ok(format!("Forgot semantic memory '{memory_id}'."))
    } else {
        Ok(format!(
            "No semantic memory '{memory_id}' belongs to this agent — nothing was deleted. \
             Run memory_semantic_search first to get a current id."
        ))
    }
}

pub(super) async fn tool_memory_semantic_duplicates(
    _input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_duplicates")?;
    let groups = kh
        .memory_semantic_duplicates(agent, peer_id, channel)
        .await
        .map_err(semantic_tool_error)?;
    // Groups of one are not duplicates. `find_duplicates` seeds a group per
    // unabsorbed memory, so the singletons are every memory with no near-twin —
    // rendering them would bury the actual finding under the whole store.
    let groups: Vec<&Vec<librefang_types::memory::MemoryItem>> =
        groups.iter().filter(|g| g.len() > 1).collect();
    if groups.is_empty() {
        return Ok("No near-duplicate memories found in this agent's semantic memory.".to_string());
    }
    let rendered: Vec<serde_json::Value> = groups
        .iter()
        .map(|group| {
            serde_json::json!({
                "count": group.len(),
                "memories": group
                    .iter()
                    .map(|i| serde_json::json!({
                        "id": i.id,
                        "content": i.content,
                        "confidence": i.confidence,
                        "created_at": i.created_at.to_rfc3339(),
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let total: usize = groups.iter().map(|g| g.len()).sum();
    Ok(format!(
        "{} duplicate group{} covering {total} memories. Consolidation would keep the newest of \
         each group and retract the other {}.\n{}",
        groups.len(),
        if groups.len() == 1 { "" } else { "s" },
        total - groups.len(),
        serde_json::to_string_pretty(&rendered).unwrap_or_else(|_| format!("{rendered:?}")),
    ))
}

pub(super) async fn tool_memory_semantic_consolidate(
    _input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_consolidate")?;
    let merged = kh
        .memory_semantic_consolidate(agent, peer_id, channel)
        .await
        .map_err(semantic_tool_error)?;
    if merged == 0 {
        return Ok(
            "Nothing to consolidate: no near-duplicate groups were found, so no memories were \
             retracted."
                .to_string(),
        );
    }
    Ok(format!(
        "Consolidated this agent's semantic memory: retracted {merged} duplicate memor{}, keeping \
         the newest of each group.",
        if merged == 1 { "y" } else { "ies" }
    ))
}

pub(super) async fn tool_memory_semantic_stats(
    _input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    peer_id: Option<&str>,
    channel: Option<&str>,
) -> Result<String, ToolError> {
    let kh = require_kernel_typed(kernel)?;
    let agent = require_caller_agent(caller_agent_id, "memory_semantic_stats")?;
    let stats = kh
        .memory_semantic_stats(agent, peer_id, channel)
        .await
        .map_err(semantic_tool_error)?;
    // `stats` is built from a `BTreeMap` kernel-side and `serde_json::Map` is
    // key-sorted, so this rendering is byte-identical across processes (#3298).
    Ok(serde_json::to_string_pretty(&stats).unwrap_or_else(|_| stats.to_string()))
}
