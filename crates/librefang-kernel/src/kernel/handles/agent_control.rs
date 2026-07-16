//! [`kernel_handle::AgentControl`] — agent lifecycle surface (spawn / send /
//! list / kill / fork / heartbeat) plus the capability-checked spawn variant
//! used by the runtime when a parent agent forks a child.

use librefang_runtime::kernel_handle;
use librefang_types::agent::*;

use super::super::{manifest_to_capabilities, LibreFangKernel};

/// Threshold above which a delegation result is spilled to the artifact
/// store so the caller can `read_artifact` the full content instead of
/// receiving a truncated `[ASYNC_RESULT]` preview.
const SPILL_DELEGATION_RESULT_ABOVE: usize = 500;

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

    async fn send_to_agent_with_key(
        &self,
        agent_id: &str,
        message: &str,
        conversation_key: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let id = self.resolve_agent_identifier(agent_id)?;
        // No parent agent id is available for system-initiated sends — pass a
        // nil UUID as a sentinel. `any_session_interrupt_for_agent` will find
        // nothing registered for it (no cascade), but the session pin still
        // applies via the `session_id_override` path.
        let no_parent = AgentId(uuid::Uuid::nil());
        let result = self
            .send_message_as_with_key(id, message, no_parent, conversation_key)
            .await
            .map_err(|e| format!("Send failed: {e}"))?;
        Ok(result.response)
    }

    async fn send_to_agent_as_with_key(
        &self,
        agent_id: &str,
        message: &str,
        parent_agent_id: &str,
        conversation_key: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        let id = self.resolve_agent_identifier(agent_id)?;
        let parent_id = self
            .resolve_agent_identifier(parent_agent_id)
            .or_else(|_| {
                parent_agent_id
                    .parse::<AgentId>()
                    .map_err(|e| format!("bad parent_agent_id: {e}"))
            })?;
        let result = self
            .send_message_as_with_key(id, message, parent_id, conversation_key)
            .await
            .map_err(|e| format!("Send failed: {e}"))?;
        Ok(result.response)
    }

    /// Non-blocking `agent_send` (#6043). Registers a
    /// [`TaskKind::Delegation`] on the async-task tracker (#4983), spawns the
    /// callee loop detached via `self_handle`, and returns the task id
    /// immediately. On completion the spawned task calls
    /// [`complete_async_task`](crate::kernel::LibreFangKernel::complete_async_task),
    /// which injects the reply back into the caller's session (mid-turn or
    /// wake-idle). Mirrors `start_workflow_async_tracked`.
    async fn send_to_agent_async_tracked(
        &self,
        agent_id: &str,
        message: &str,
        caller_agent_id: &str,
        caller_session_id: Option<&str>,
        conversation_key: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        use librefang_types::task::{TaskKind, TaskStatus};

        // Resolve target + caller up front so a bad id fails fast (before
        // any registration or spawn). Parent resolution mirrors
        // `send_to_agent_as`: name/alias first, bare UUID fallback so a
        // caller that left the registry mid-flight still resolves.
        let target_id = self.resolve_agent_identifier(agent_id)?;
        let parent_id = self
            .resolve_agent_identifier(caller_agent_id)
            .or_else(|_| {
                caller_agent_id
                    .parse::<AgentId>()
                    .map_err(|e| format!("bad caller_agent_id: {e}"))
            })?;

        // The tracker keys completion delivery on the originating
        // `(agent, session)`. Without a parseable caller session there is
        // nowhere to deliver the reply, so fall back to a blocking send
        // (caller still gets the answer, just inline) rather than spawning
        // an orphaned delegation whose result is dropped.
        let session_id = match caller_session_id.and_then(|s| s.parse::<SessionId>().ok()) {
            Some(sid) => sid,
            None => {
                tracing::debug!(
                    agent = %agent_id,
                    "send_to_agent_async_tracked: no parseable caller session; falling back to blocking send"
                );
                // Await inside each arm — the two async fns return distinct
                // opaque future types that can't unify as a single match value.
                let result = match conversation_key {
                    Some(key) => {
                        self.send_message_as_with_key(target_id, message, parent_id, key)
                            .await
                    }
                    None => self.send_message_as(target_id, message, parent_id).await,
                }
                .map_err(|e| format!("Send failed: {e}"))?;
                return Ok(result.response);
            }
        };

        // Opaque, deterministic prompt hash so callers can dedup repeat
        // delegations without the kernel storing the full prompt (the field
        // is documented as caller's-choice / opaque to the kernel).
        let prompt_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            message.hash(&mut h);
            format!("{:016x}", h.finish())
        };

        let handle = self.register_async_task(
            parent_id,
            session_id,
            TaskKind::Delegation {
                agent_id: target_id,
                prompt_hash,
            },
            chat_id.map(String::from),
        );
        let task_id = handle.id;

        // Spawn the callee loop detached through the upgraded self-handle,
        // same as the async workflow path.
        let kernel_arc = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                KernelOpError::Internal(
                    "kernel not yet initialised for async agent_send spawn".to_string(),
                )
            })?;

        let msg = message.to_string();
        let conv_key = conversation_key.map(String::from);
        tokio::spawn(async move {
            let exec = match &conv_key {
                Some(key) => {
                    kernel_arc
                        .send_message_as_with_key(target_id, &msg, parent_id, key)
                        .await
                }
                None => kernel_arc.send_message_as(target_id, &msg, parent_id).await,
            };
            let terminal_status = match exec {
                Ok(result) => {
                    let mut payload = serde_json::json!({
                        "agent_id": target_id.to_string(),
                        "response": result.response,
                    });
                    // Spill large delegation results to the artifact store
                    // so the caller gets a real handle instead of a
                    // truncated preview that provokes hallucinated hashes.
                    let resp_bytes = result.response.as_bytes();
                    if resp_bytes.len() > SPILL_DELEGATION_RESULT_ABOVE {
                        let artifact_dir =
                            librefang_runtime::artifact_store::default_artifact_storage_dir();
                        match librefang_runtime::artifact_store::write(
                            resp_bytes,
                            &artifact_dir,
                            librefang_runtime::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
                        ) {
                            Ok(handle) => {
                                payload["artifact_handle"] =
                                    serde_json::Value::String(handle.to_string());
                                tracing::debug!(
                                    task_id = %task_id,
                                    target = %target_id,
                                    handle = %handle,
                                    bytes = resp_bytes.len(),
                                    "Spilled delegation result to artifact store"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    task_id = %task_id,
                                    target = %target_id,
                                    error = %e,
                                    "Failed to spill delegation result; falling back to inline text"
                                );
                            }
                        }
                    }
                    TaskStatus::Completed(payload)
                }
                Err(e) => TaskStatus::Failed(format!("agent_send delegation failed: {e}")),
            };
            if let Err(err) = kernel_arc
                .complete_async_task(task_id, terminal_status)
                .await
            {
                tracing::warn!(
                    task_id = %task_id,
                    target = %target_id,
                    "Failed to inject delegation TaskCompletionEvent: {err}"
                );
            }
        });

        Ok(task_id.to_string())
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

    fn register_async_task(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        kind: librefang_types::task::TaskKind,
        chat_id: Option<String>,
    ) -> Option<librefang_types::task::TaskHandle> {
        Some(LibreFangKernel::register_async_task(
            self, agent_id, session_id, kind, chat_id,
        ))
    }

    async fn complete_async_task(
        &self,
        task_id: librefang_types::task::TaskId,
        status: librefang_types::task::TaskStatus,
    ) -> Result<bool, kernel_handle::KernelOpError> {
        LibreFangKernel::complete_async_task(self, task_id, status)
            .await
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("{e}")))
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
