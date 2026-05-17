//! [`kernel_handle::MemoryAccess`] — peer-scoped key/value access on top of
//! the SQLite memory substrate, plus the per-user RBAC ACL resolver. Writes
//! publish a `MemoryUpdate` event so triggers can fan out without polling.

use librefang_runtime::kernel_handle;
use librefang_types::event::*;

use super::super::PUBLISH_EVENT_DEPTH;
use super::super::{peer_scoped_key, shared_memory_agent_id, spawn_logged, LibreFangKernel};

/// Reject a `peer_id` that is empty or contains `:` at the kernel-handle
/// boundary (#5119). The historical `peer:{pid}:{key}` framing is only
/// injective when `pid` is non-empty and colon-free; otherwise
/// `memory_list`'s `strip_prefix("peer:{pid}:")` recovery path lets one peer
/// see another peer's keys, and an empty `pid` collides with the `None`-scope
/// global namespace.
fn reject_bad_peer_id(peer_id: Option<&str>) -> Result<(), kernel_handle::KernelOpError> {
    use kernel_handle::KernelOpError;
    if let Some(pid) = peer_id {
        if pid.is_empty() {
            return Err(KernelOpError::InvalidInput(
                "peer_id must not be empty (ambiguous with global scope)".to_string(),
            ));
        }
        if pid.contains(':') {
            return Err(KernelOpError::InvalidInput(format!(
                "peer_id '{pid}' must not contain ':' (reserved namespace separator)"
            )));
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

impl kernel_handle::MemoryAccess for LibreFangKernel {
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        peer_id: Option<&str>,
    ) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        reject_peer_prefix_in_key(key)?;
        reject_bad_peer_id(peer_id)?;
        let agent_id = shared_memory_agent_id();
        let scoped = peer_scoped_key(key, peer_id)?;
        // Check whether key already exists to determine Created vs Updated
        let had_old = self
            .memory
            .substrate
            .structured_get(agent_id, &scoped)
            .ok()
            .flatten()
            .is_some();
        self.memory
            .substrate
            .structured_set(agent_id, &scoped, value)
            .map_err(|e| KernelOpError::Internal(format!("Memory store failed: {e}")))?;

        tracing::debug!(
            key = %scoped,
            peer_id = ?peer_id,
            "memory_store: wrote key to shared KV namespace"
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
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        reject_peer_prefix_in_key(key)?;
        reject_bad_peer_id(peer_id)?;
        let agent_id = shared_memory_agent_id();
        let scoped = peer_scoped_key(key, peer_id)?;
        self.memory
            .substrate
            .structured_get(agent_id, &scoped)
            .map_err(|e| KernelOpError::Internal(format!("Memory recall failed: {e}")))
    }

    fn memory_list(
        &self,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        // (#5119) An attacker cannot even issue a colon-bearing / empty
        // `peer_id` query: `reject_bad_peer_id` fails the call before the
        // recovery loop runs, so a Slack-style `T1:U2` can never strip
        // `peer:T1:` off `peer:T1:U2:car` to read peer `T1`'s neighbour.
        reject_bad_peer_id(peer_id)?;
        let agent_id = shared_memory_agent_id();
        let all_keys = self
            .memory
            .substrate
            .list_keys(agent_id)
            .map_err(|e| KernelOpError::Internal(format!("Memory list failed: {e}")))?;
        match peer_id {
            Some(pid) => {
                let prefix = format!("peer:{pid}:");
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
            None => {
                // When no peer context, return only non-peer-scoped keys
                Ok(all_keys
                    .into_iter()
                    .filter(|k| !k.starts_with("peer:"))
                    .collect())
            }
        }
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
