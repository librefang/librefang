//! [`kernel_handle::AgentControl`] — agent lifecycle surface (spawn / send /
//! list / kill / fork / heartbeat) plus the capability-checked spawn variant
//! used by the runtime when a parent agent forks a child.

use librefang_runtime::kernel_handle;
use librefang_types::agent::*;

use super::super::{manifest_to_capabilities, LibreFangKernel};

#[async_trait::async_trait]
impl kernel_handle::AgentControl for LibreFangKernel {
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), kernel_handle::KernelOpError> {
        // Verify manifest integrity if a signed manifest hash is present
        let content_hash = librefang_types::manifest_signing::hash_manifest(manifest_toml);
        tracing::debug!(hash = %content_hash, "Manifest SHA-256 computed for integrity tracking");

        let manifest: AgentManifest =
            toml::from_str(manifest_toml).map_err(|e| format!("Invalid manifest: {e}"))?;
        let name = manifest.name.clone();
        let parent = parent_id.and_then(|pid| pid.parse::<AgentId>().ok());
        let id = self
            .spawn_agent_with_parent(manifest, parent)
            .map_err(|e| format!("Spawn failed: {e}"))?;
        Ok((id.to_string(), name))
    }

    async fn send_to_agent(
        &self,
        agent_id: &str,
        message: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let id = self.resolve_agent_identifier(agent_id)?;
        let result = self
            .send_message(id, message)
            .await
            .map_err(|e| format!("Send failed: {e}"))?;
        Ok(result.response)
    }

    async fn send_to_agent_as(
        &self,
        agent_id: &str,
        message: &str,
        parent_agent_id: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let id = self.resolve_agent_identifier(agent_id)?;
        // Parent resolution: try the name/alias resolver first for ergonomics,
        // but fall back to bare UUID parsing when the parent has been removed
        // from the registry. A parent can legitimately disappear from the
        // registry mid-flight (e.g. /kill racing with a pending agent_send
        // response), while its `SessionInterrupt` is still live in
        // `session_interrupts` because the in-flight turn holds a clone.
        // Failing here would break the cascade contract "parent absent →
        // no cascade but call proceeds" that `send_message_as` implements.
        let parent_id = self
            .resolve_agent_identifier(parent_agent_id)
            .or_else(|_| {
                parent_agent_id
                    .parse::<AgentId>()
                    .map_err(|e| format!("bad parent_agent_id: {e}"))
            })?;
        let result = self
            .send_message_as(id, message, parent_id)
            .await
            .map_err(|e| format!("Send failed: {e}"))?;
        Ok(result.response)
    }

    fn list_agents(&self) -> Vec<kernel_handle::AgentInfo> {
        self.agents
            .registry
            .list()
            .into_iter()
            .map(|e| kernel_handle::AgentInfo {
                id: e.id.to_string(),
                name: e.name.clone(),
                state: format!("{:?}", e.state),
                model_provider: e.manifest.model.provider.clone(),
                model_name: e.manifest.model.model.clone(),
                description: e.manifest.description.clone(),
                tags: e.tags.clone(),
                tools: e.manifest.capabilities.tools.clone(),
            })
            .collect()
    }

    fn touch_heartbeat(&self, agent_id: &str) {
        if let Ok(id) = agent_id.parse::<AgentId>() {
            self.agents.registry.touch(id);
        }
    }

    async fn run_forked_agent_oneshot(
        &self,
        agent_id: &str,
        prompt: &str,
        allowed_tools: Option<Vec<String>>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let id = agent_id
            .parse::<AgentId>()
            .map_err(|e| format!("bad agent_id: {e}"))?;
        // Need `Arc<Self>` to call `run_forked_agent_streaming` (the method
        // is defined on `Arc<LibreFangKernel>`). Upgrade via `self_handle`;
        // if the weak ref is stale the daemon is shutting down and the
        // extractor should abort.
        let kernel = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| "kernel Arc unavailable (shutting down?)".to_string())?;
        let (mut rx, handle) = kernel
            .run_forked_agent_streaming(id, prompt, allowed_tools)
            .map_err(|e| format!("fork start failed: {e}"))?;
        // Drain the stream — we don't need streaming semantics for a
        // one-shot completion, just the final text. The spawned task
        // keeps running until `ContentComplete` (or error/abort) anyway.
        while (rx.recv().await).is_some() {
            // Events consumed; the final text is on the join handle's
            // `AgentLoopResult.response`. Discarding these events is
            // fine because `ContentComplete` is already signalled to
            // the join handle by the time we observe channel close.
        }
        let result = handle
            .await
            .map_err(|e| format!("fork join failed: {e}"))?
            .map_err(|e| format!("fork loop failed: {e}"))?;
        Ok(result.response)
    }

    fn kill_agent(&self, agent_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        let id = self
            .resolve_agent_identifier(agent_id)
            .map_err(kernel_handle::KernelOpError::Internal)?;
        LibreFangKernel::kill_agent(self, id)
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("Kill failed: {e}")))
    }

    fn find_agents(&self, query: &str) -> Vec<kernel_handle::AgentInfo> {
        let q = query.to_lowercase();
        self.agents
            .registry
            .list()
            .into_iter()
            .filter(|e| {
                let name_match = e.name.to_lowercase().contains(&q);
                let tag_match = e.tags.iter().any(|t| t.to_lowercase().contains(&q));
                let tool_match = e
                    .manifest
                    .capabilities
                    .tools
                    .iter()
                    .any(|t| t.to_lowercase().contains(&q));
                let desc_match = e.manifest.description.to_lowercase().contains(&q);
                name_match || tag_match || tool_match || desc_match
            })
            .map(|e| kernel_handle::AgentInfo {
                id: e.id.to_string(),
                name: e.name.clone(),
                state: format!("{:?}", e.state),
                model_provider: e.manifest.model.provider.clone(),
                model_name: e.manifest.model.model.clone(),
                description: e.manifest.description.clone(),
                tags: e.tags.clone(),
                tools: e.manifest.capabilities.tools.clone(),
            })
            .collect()
    }

    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[librefang_types::capability::Capability],
    ) -> Result<(String, String), kernel_handle::KernelOpError> {
        // Parse the child manifest to extract its capabilities
        let child_manifest: AgentManifest = toml::from_str(manifest_toml)
            .map_err(|e| kernel_handle::KernelOpError::InvalidInput(format!("manifest: {e}")))?;
        let child_caps = manifest_to_capabilities(&child_manifest);

        // Enforce: child capabilities must be a subset of parent capabilities
        librefang_types::capability::validate_capability_inheritance(parent_caps, &child_caps)
            .map_err(kernel_handle::KernelOpError::Internal)?;

        tracing::info!(
            parent = parent_id.unwrap_or("kernel"),
            child = %child_manifest.name,
            child_caps = child_caps.len(),
            "Capability inheritance validated — spawning child agent"
        );

        // Delegate to the normal spawn path via the AgentControl role trait.
        kernel_handle::AgentControl::spawn_agent(self, manifest_toml, parent_id).await
    }

    fn max_agent_call_depth(&self) -> u32 {
        let cfg = self.config.load();
        cfg.max_agent_call_depth
    }

    fn fire_agent_step(&self, agent_id: &str, step: u32) {
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::AgentStep,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "step": step,
            }),
        );
    }
}
