//! [`kernel_handle::MemoryAccess`] — agent/peer-scoped key/value access on top of
//! the SQLite memory substrate, plus the per-user RBAC ACL resolver. Writes
//! publish a `MemoryUpdate` event so triggers can fan out without polling.

use librefang_types::agent::AgentId;
use std::collections::BTreeMap;

use async_trait::async_trait;
use librefang_memory::namespace_acl::MemoryNamespaceGuard;
use librefang_types::user_policy::UserMemoryAccess;

use librefang_runtime::kernel_handle;
use librefang_types::event::*;

use super::super::PUBLISH_EVENT_DEPTH;
use super::super::{
    escape_peer_id, peer_scoped_key, shared_memory_agent_id, spawn_logged, LibreFangKernel,
};

fn resolve_agent_id(agent_id: Option<&str>) -> Result<AgentId, kernel_handle::KernelOpError> {
    match agent_id {
        None => Ok(shared_memory_agent_id()),
        Some("") => Err(kernel_handle::KernelOpError::InvalidInput(
            "agent_id must be a valid UUID string, got empty string".into(),
        )),
        Some(s) => uuid::Uuid::parse_str(s).map(AgentId).map_err(|e| {
            kernel_handle::KernelOpError::InvalidInput(format!("invalid agent_id '{s}': {e}"))
        }),
    }
}

/// Reject a `peer_id` that is empty at the kernel-handle boundary (#5119).
/// Colons in `pid` are percent-encoded by [`escape_peer_id`] during storage
/// (#6100), so they no longer collapse the `peer:{pid}:{key}` framing and need
/// not be rejected. An empty `pid` still collides with the `None`-scope global
/// namespace (`peer::{key}`) and must be rejected.
fn reject_bad_peer_id(peer_id: Option<&str>) -> Result<(), kernel_handle::KernelOpError> {
    use kernel_handle::KernelOpError;
    if let Some(pid) = peer_id {
        if pid.is_empty() {
            return Err(KernelOpError::InvalidInput(
                "peer_id must not be empty (ambiguous with global scope)".to_string(),
            ));
        }
    }
    Ok(())
}

/// Reject an LLM-supplied key that starts with `peer:` at the kernel-handle
/// boundary (#5120). The `peer:` prefix is reserved for the kernel's internal
/// per-peer namespace; letting the tool layer write at `peer:victim:user_name`
/// would let an agent with no peer context plant rows that surface to
/// `memory_list(Some("victim"))` as if `victim` wrote them.
fn reject_peer_prefix_in_key(key: &str) -> Result<(), kernel_handle::KernelOpError> {
    use kernel_handle::KernelOpError;
    if key.starts_with("peer:") {
        return Err(KernelOpError::InvalidInput(format!(
            "memory key '{key}' must not start with reserved 'peer:' prefix"
        )));
    }
    Ok(())
}

/// Reject an empty memory key at the kernel-handle boundary (#5138).
///
/// `memory_store(key="", ...)` would otherwise land a row at
/// `(shared_agent, "")` and `memory_list(None)` would then surface a
/// nameless `""` entry. Mirrors the empty-`peer_id` rejection shape from
/// #5119 / #5071 so the substrate boundary uniformly refuses ambiguous
/// addressing.
fn reject_empty_key(key: &str) -> Result<(), kernel_handle::KernelOpError> {
    use kernel_handle::KernelOpError;
    if key.is_empty() {
        return Err(KernelOpError::InvalidInput(
            "memory key must not be empty".to_string(),
        ));
    }
    Ok(())
}

/// The memory-ACL namespace every semantic-memory call is gated on. Matches the
/// string the REST layer and the proactive-recall path already use, so one
/// `[[users]] memory_access` block governs the agent tools and the dashboard
/// identically.
const PROACTIVE_NAMESPACE: &str = "proactive";

/// Reject an empty / non-UUID caller agent id before it reaches the store.
///
/// Unlike the KV methods, semantic memory has no shared fallback namespace:
/// `ProactiveMemoryStore` parses `user_id` as an `AgentId`, so an unattributed
/// call has no meaningful scope and must fail rather than silently read or
/// write somebody else's memories.
fn require_agent_uuid(agent_id: &str) -> Result<AgentId, kernel_handle::KernelOpError> {
    use kernel_handle::KernelOpError;
    if agent_id.is_empty() {
        return Err(KernelOpError::InvalidInput(
            "agent_id must be a valid UUID string, got empty string".into(),
        ));
    }
    uuid::Uuid::parse_str(agent_id)
        .map(AgentId)
        .map_err(|e| KernelOpError::InvalidInput(format!("invalid agent_id '{agent_id}': {e}")))
}

/// Grant-everything ACL used when RBAC resolves to no policy.
///
/// `memory_acl_for_sender` returns `None` when `[[users]]` is unconfigured or
/// the sender could not be attributed. The KV tools treat that as "no per-user
/// restriction" (`enforce_memory_acl` returns `Ok(())`), and the semantic tools
/// must agree, or turning RBAC off would *tighten* memory access instead of
/// loosening it.
fn unrestricted_acl() -> UserMemoryAccess {
    UserMemoryAccess {
        readable_namespaces: vec!["*".into()],
        writable_namespaces: vec!["*".into()],
        pii_access: true,
        export_allowed: true,
        delete_allowed: true,
    }
}

#[async_trait]
impl kernel_handle::MemoryAccess for LibreFangKernel {
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        reject_empty_key(key)?;
        reject_peer_prefix_in_key(key)?;
        reject_bad_peer_id(peer_id)?;
        let agent_id = resolve_agent_id(agent_id)?;
        let scoped = peer_scoped_key(key, peer_id)?;
        // Derive Created vs Updated from the same transaction that performs
        // the write (#5138). The old `structured_get` pre-read then
        // `structured_set` raced: two concurrent first-time writes both saw
        // `had_old=false` and both published `Created`, and a write that
        // lost the SQLite race still announced its own value as `Created`
        // with no payload while triggers read the *other* writer's value.
        // `set_returning_existed` checks existence and writes atomically,
        // so the published operation reflects the committed transition. It
        // also enforces `MAX_KV_VALUE_BYTES`, surfacing an over-limit blob
        // as `InvalidInput` (#5138) before it can wedge the substrate.
        let had_old = self
            .memory
            .substrate
            .structured_set_returning_existed(agent_id, &scoped, value)
            .map_err(|e| match e {
                KernelOpError::InvalidInput(_) => e,
                other => KernelOpError::Internal(format!("Memory store failed: {other}")),
            })?;

        tracing::debug!(
            key = %scoped,
            agent_id = %agent_id,
            peer_id = ?peer_id,
            "memory_store: wrote key to KV namespace"
        );

        // Publish MemoryUpdate event so triggers can react
        let operation = if had_old {
            MemoryOperation::Updated
        } else {
            MemoryOperation::Created
        };
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::MemoryUpdate(MemoryDelta {
                operation,
                key: scoped.clone(),
                agent_id,
            }),
        );
        if let Some(weak) = self.self_handle.get() {
            if let Some(kernel) = weak.upgrade() {
                // Propagate trigger-chain depth across the spawn boundary
                // (#3735). Without this, a memory_store invoked from inside
                // a triggered agent would publish into a fresh top-level
                // depth=0 scope, defeating the depth cap on chains that
                // travel through memory updates.
                let parent_depth = PUBLISH_EVENT_DEPTH.try_with(|c| c.get()).unwrap_or(0);
                spawn_logged(
                    "memory_event_publish",
                    PUBLISH_EVENT_DEPTH.scope(std::cell::Cell::new(parent_depth), async move {
                        kernel.publish_event(event).await;
                    }),
                );
            }
        }
        Ok(())
    }

    fn memory_recall(
        &self,
        key: &str,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        reject_empty_key(key)?;
        reject_peer_prefix_in_key(key)?;
        reject_bad_peer_id(peer_id)?;
        let agent_id = resolve_agent_id(agent_id)?;
        let scoped = peer_scoped_key(key, peer_id)?;
        let value = self
            .memory
            .substrate
            .structured_get(agent_id, &scoped)
            .map_err(|e| KernelOpError::Internal(format!("Memory recall failed: {e}")))?;
        // Upgrade-compat fallback: if agent-scoped lookup misses, try the
        // pre-#5070 shared namespace. This preserves access to rows written
        // before per-agent isolation shipped. Remove after a release cycle.
        if value.is_none() && agent_id != shared_memory_agent_id() {
            let shared_id = shared_memory_agent_id();
            if let Ok(Some(legacy_val)) = self.memory.substrate.structured_get(shared_id, &scoped) {
                tracing::warn!(
                    key = %scoped,
                    ?agent_id,
                    "memory_recall: found value in deprecated shared namespace; \
                     run a re-key migration to move data into the per-agent namespace"
                );
                return Ok(Some(legacy_val));
            }
        }
        Ok(value)
    }

    fn memory_list(
        &self,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        // (#5119 / #6100) A colon-bearing query is now allowed but stays
        // isolated: the prefix below is built from the *escaped* peer_id, so a
        // Slack-style `T1:U2` lists under `peer:T1%3AU2:` and can never strip
        // `peer:T1:` off peer `T1`'s rows (and vice-versa). An empty `peer_id`
        // is still rejected before the recovery loop runs.
        reject_bad_peer_id(peer_id)?;
        let agent_id = resolve_agent_id(agent_id)?;
        let all_keys = self
            .memory
            .substrate
            .list_keys(agent_id)
            .map_err(|e| KernelOpError::Internal(format!("Memory list failed: {e}")))?;
        match peer_id {
            Some(pid) => {
                // Build the recovery prefix from the escaped peer_id so it
                // matches the form `peer_scoped_key` stored under (#6100).
                let escaped_pid = escape_peer_id(pid);
                let prefix = format!("peer:{escaped_pid}:");
                // SECURITY (#5120 read-side residual): the write path now
                // rejects `peer:`-prefixed keys, but rows planted *before* the
                // fix can still sit at `peer:{x}:...` in the shared substrate.
                // We strip `peer:{pid}:` to recover the candidate inner key,
                // then only surface it if it round-trips back through the
                // *now-strict* `peer_scoped_key(inner, Some(pid))` to the exact
                // stored key. This drops any recovered inner key that is
                // itself `peer:`-prefixed (nested / double-scoped plants like
                // `peer:victim:peer:other:k`) or otherwise malformed, so the
                // tool path can never enumerate a structurally-impossible row.
                //
                // RESIDUAL (documented in CHANGELOG, maintainer sign-off): a
                // pre-fix plant written by a `None`-scope agent at the *exact*
                // bytes `peer:{colon-free-pid}:{non-peer-key}` is byte-identical
                // to a row `pid` legitimately wrote post-fix — no in-code
                // predicate can separate the two without a writer-attribution
                // column. Distinguishing those requires a one-shot DB scrub of
                // `key LIKE 'peer:%'` on the shared-memory agent id; it is out
                // of scope for an in-code substrate-boundary fix.
                Ok(all_keys
                    .into_iter()
                    .filter_map(|k| {
                        let inner = k.strip_prefix(&prefix)?;
                        // Re-render through the strict canonical form. A
                        // legitimate row's stored key is exactly
                        // `peer:{pid}:{inner}`; anything that doesn't round-trip
                        // (e.g. inner itself starts with `peer:`, peer_scoped_key
                        // would reject it) is dropped.
                        match peer_scoped_key(inner, Some(pid)) {
                            Ok(canonical) if canonical == k => Some(inner.to_string()),
                            _ => None,
                        }
                    })
                    .collect())
            }
            None => Ok(all_keys
                .into_iter()
                .filter(|k| !k.starts_with("peer:"))
                .collect()),
        }
    }

    // ------------------------------------------------------------------
    // Semantic (vector) memory (#7808)
    // ------------------------------------------------------------------

    async fn memory_semantic_search(
        &self,
        query: &str,
        agent_id: &str,
        limit: usize,
        min_confidence: Option<f32>,
        min_similarity: Option<f32>,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::MemoryItem>, kernel_handle::KernelOpError> {
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;
        let guard = self.semantic_memory_guard(sender_id, channel);
        // Per-call argument wins; otherwise the agent's own manifest floor,
        // otherwise the deployment default resolved inside the store (#7808).
        let floor = min_similarity.or_else(|| self.resolved_min_similarity(agent));
        let items = store
            .search_with_guard(query, &agent.to_string(), limit, floor, &guard)
            .await?;
        Ok(match min_confidence {
            // A fragment with no stored confidence predates confidence
            // tracking; keep it rather than silently deleting history from the
            // caller's view when a floor is requested.
            Some(floor) => items
                .into_iter()
                .filter(|i| i.confidence.is_none_or(|c| c >= floor))
                .collect(),
            None => items,
        })
    }

    async fn memory_semantic_add(
        &self,
        content: &str,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::MemoryItem>, kernel_handle::KernelOpError> {
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;
        let guard = self.semantic_memory_guard(sender_id, channel);
        // Shaped as one user-role message so the extractor sees the same input
        // shape `POST /api/memory` feeds it.
        let messages = vec![serde_json::json!({ "role": "user", "content": content })];
        store
            .add_with_guard(&messages, &agent.to_string(), &guard)
            .await
    }

    async fn memory_semantic_forget(
        &self,
        memory_id: &str,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<bool, kernel_handle::KernelOpError> {
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;
        let guard = self.semantic_memory_guard(sender_id, channel);
        store
            .delete_with_guard(memory_id, &agent.to_string(), &guard)
            .await
    }

    async fn memory_semantic_stats(
        &self,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;
        let guard = self.semantic_memory_guard(sender_id, channel);
        // `stats` has no `_with_guard` wrapper in `librefang-memory` because no
        // caller needed one before; counts are still a read of the `proactive`
        // namespace, so gate it here rather than leaving the one unguarded
        // semantic read on the trait.
        if let librefang_memory::namespace_acl::NamespaceGate::Deny(reason) =
            guard.check_read(PROACTIVE_NAMESPACE)
        {
            return Err(KernelOpError::AuthDenied(reason));
        }
        let stats = store.stats(&agent.to_string()).await?;
        // #3298: `MemoryStats::categories` is a `HashMap`, so its iteration
        // order varies per process. Re-key through a `BTreeMap` so the rendered
        // tool result is byte-identical run to run — this result lands in the
        // message history and a reordered one invalidates the provider prompt
        // cache for every later turn in the conversation.
        let categories: BTreeMap<&String, &usize> = stats.categories.iter().collect();
        Ok(serde_json::json!({
            "total": stats.total,
            "user_count": stats.user_count,
            "session_count": stats.session_count,
            "agent_count": stats.agent_count,
            "categories": categories,
            "enabled": stats.enabled,
            "auto_memorize_enabled": stats.auto_memorize_enabled,
            "auto_retrieve_enabled": stats.auto_retrieve_enabled,
            "llm_extraction": stats.llm_extraction,
        }))
    }

    async fn memory_semantic_duplicates(
        &self,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<Vec<librefang_types::memory::MemoryItem>>, kernel_handle::KernelOpError> {
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;
        let guard = self.semantic_memory_guard(sender_id, channel);
        Ok(store
            .find_duplicates_with_guard(&agent.to_string(), None, &guard)
            .await?)
    }

    async fn memory_semantic_consolidate(
        &self,
        agent_id: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<u64, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let agent = require_agent_uuid(agent_id)?;
        let store = self.require_proactive_store()?;

        // The opt-in is checked here, in front of the store call, and NOT only
        // by omitting the tool from `available_tools` (#7808). Advertising is
        // a prompt-shaping decision; this is the enforcement. A tool name can
        // still arrive from a replayed transcript, a `tool_load` on a cached
        // name, a hand or skill that hardcodes it, or a manifest edited between
        // the two — and every one of those paths reaches dispatch without ever
        // consulting the advertised list.
        if !self.allows_self_consolidation(agent) {
            return Err(KernelOpError::AuthDenied(format!(
                "agent {agent} may not consolidate its own semantic memory: set \
                 `[proactive_memory] allow_self_consolidation = true` in that agent's agent.toml \
                 (not config.toml) to allow it. Consolidation merges near-duplicate groups across \
                 the whole store and soft-deletes every member but the newest. \
                 `memory_semantic_duplicates` reports the same groups without changing anything."
            )));
        }

        let guard = self.semantic_memory_guard(sender_id, channel);
        Ok(store
            .consolidate_with_guard(&agent.to_string(), &guard)
            .await?)
    }

    fn memory_acl_for_sender(
        &self,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Option<librefang_types::user_policy::UserMemoryAccess> {
        if !self.security.auth.is_enabled() {
            return None;
        }
        let user_id = self.security.auth.resolve_user(sender_id, channel)?;
        self.security.auth.memory_acl_for(user_id)
    }
}

impl LibreFangKernel {
    /// Borrow the proactive-memory store, or report it as unavailable.
    ///
    /// The store is a `OnceLock` filled during boot only when
    /// `[proactive_memory] enabled = true`, so "semantic memory is switched
    /// off" has to surface as `Unavailable` (503-shaped) rather than as an
    /// empty result set — an agent told "no memories found" would conclude the
    /// store is empty and stop asking.
    fn require_proactive_store(
        &self,
    ) -> Result<&std::sync::Arc<librefang_memory::ProactiveMemoryStore>, kernel_handle::KernelOpError>
    {
        use crate::MemorySubsystemApi;
        self.proactive_store().ok_or_else(|| {
            kernel_handle::KernelOpError::unavailable(
                "semantic memory (enable [proactive_memory] in config.toml)",
            )
        })
    }

    /// Whether `agent` has opted in to consolidating its own semantic memory
    /// unattended (#7808).
    ///
    /// An agent with no registry entry answers `false`: a caller the kernel
    /// cannot resolve to a manifest is exactly the caller whose opt-in cannot
    /// be confirmed, and this gate must fail closed.
    pub(crate) fn allows_self_consolidation(&self, agent: AgentId) -> bool {
        self.agents.registry.get(agent).is_some_and(|entry| {
            entry
                .manifest
                .proactive_memory
                .resolve_allow_self_consolidation()
        })
    }

    /// Resolve `agent`'s effective similarity floor: manifest override, else
    /// the kernel-global `[proactive_memory] min_similarity` (#7808).
    fn resolved_min_similarity(&self, agent: AgentId) -> Option<f32> {
        let cfg = self.config.load();
        match self.agents.registry.get(agent) {
            Some(entry) => entry
                .manifest
                .proactive_memory
                .resolve_min_similarity(&cfg.proactive_memory),
            None => cfg.proactive_memory.min_similarity,
        }
    }

    /// Resolve the per-user memory ACL for a sender into a namespace guard.
    ///
    /// Falls back to [`unrestricted_acl`] when RBAC is off or the sender is
    /// unattributed, matching the KV tools' `enforce_memory_acl` contract.
    fn semantic_memory_guard(
        &self,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> MemoryNamespaceGuard {
        use kernel_handle::MemoryAccess;
        MemoryNamespaceGuard::new(
            self.memory_acl_for_sender(sender_id, channel)
                .unwrap_or_else(unrestricted_acl),
        )
    }
}
