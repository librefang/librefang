//! [`kernel_handle::MemoryAccess`] — agent/peer-scoped key/value access on top of
//! the SQLite memory substrate, plus the per-user RBAC ACL resolver. Writes
//! publish a `MemoryUpdate` event so triggers can fan out without polling.

use librefang_types::agent::AgentId;

use librefang_runtime::kernel_handle;
use librefang_types::event::*;

use super::super::PUBLISH_EVENT_DEPTH;
use super::super::{peer_scoped_key, shared_memory_agent_id, spawn_logged, LibreFangKernel};

fn resolve_agent_id(agent_id: Option<&str>) -> Result<AgentId, kernel_handle::KernelOpError> {
    match agent_id {
        None => Ok(shared_memory_agent_id()),
        Some(s) if s.is_empty() => Err(kernel_handle::KernelOpError::InvalidInput(
            "agent_id must be a valid UUID string, got empty string".into(),
        )),
        Some(s) => uuid::Uuid::parse_str(s).map(AgentId).map_err(|e| {
            kernel_handle::KernelOpError::InvalidInput(format!("invalid agent_id '{s}': {e}"))
        }),
    }
}

impl kernel_handle::MemoryAccess for LibreFangKernel {
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let agent_id = resolve_agent_id(agent_id)?;
        let scoped = peer_scoped_key(key, peer_id);
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
        let agent_id = resolve_agent_id(agent_id)?;
        let scoped = peer_scoped_key(key, peer_id);
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
        let agent_id = resolve_agent_id(agent_id)?;
        let all_keys = self
            .memory
            .substrate
            .list_keys(agent_id)
            .map_err(|e| KernelOpError::Internal(format!("Memory list failed: {e}")))?;
        match peer_id {
            Some(pid) => {
                let prefix = format!("peer:{pid}:");
                Ok(all_keys
                    .into_iter()
                    .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
                    .collect())
            }
            None => Ok(all_keys
                .into_iter()
                .filter(|k| !k.starts_with("peer:"))
                .collect()),
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
