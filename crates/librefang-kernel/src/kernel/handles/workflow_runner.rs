//! [`kernel_handle::WorkflowRunner`] — execute a workflow by UUID or by
//! name. Resolves the name to an id by scanning [`crate::workflow`]'s
//! registered workflows, then delegates to the inherent
//! [`LibreFangKernel::run_workflow`].

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

#[async_trait::async_trait]
impl kernel_handle::WorkflowRunner for LibreFangKernel {
    async fn run_workflow(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<(String, String), kernel_handle::KernelOpError> {
        use crate::workflow::WorkflowId;
        use kernel_handle::KernelOpError;

        // Try parsing as UUID first, then fall back to name lookup.
        let wf_id = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
            WorkflowId(uuid)
        } else {
            // Name-based lookup: scan all registered workflows.
            let name_lower = workflow_id.to_lowercase();
            let workflows = self.workflows.engine.list_workflows().await;
            workflows
                .iter()
                .find(|w| w.name.to_lowercase() == name_lower)
                .map(|w| w.id)
                .ok_or_else(|| {
                    KernelOpError::Internal(format!("workflow `{}` not found", workflow_id))
                })?
        };

        let (run_id, output) = LibreFangKernel::run_workflow(self, wf_id, input.to_string())
            .await
            .map_err(|e| KernelOpError::Internal(format!("Workflow execution failed: {e}")))?;

        Ok((run_id.to_string(), output))
    }

    async fn list_workflows(&self) -> Vec<kernel_handle::WorkflowSummary> {
        let mut summaries: Vec<kernel_handle::WorkflowSummary> = self
            .workflows
            .engine
            .list_workflows()
            .await
            .into_iter()
            .map(|w| kernel_handle::WorkflowSummary {
                id: w.id.0.to_string(),
                name: w.name,
                description: w.description,
                step_count: w.steps.len(),
            })
            .collect();
        // Sort by name for deterministic prompt output (#3298).
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        summaries
    }

    async fn get_workflow_run(&self, run_id: &str) -> Option<kernel_handle::WorkflowRunSummary> {
        use crate::workflow::WorkflowRunId;

        let uuid = uuid::Uuid::parse_str(run_id).ok()?;
        let run = self.workflows.engine.get_run(WorkflowRunId(uuid)).await?;

        let state = serde_json::to_value(&run.state)
            .ok()
            .and_then(|v| {
                // `WorkflowRunState` serializes as snake_case string or object for Paused.
                // Extract the variant name string.
                if v.is_string() {
                    v.as_str().map(|s| s.to_string())
                } else if let Some(obj) = v.as_object() {
                    obj.keys().next().map(|k| k.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        Some(kernel_handle::WorkflowRunSummary {
            run_id: run.id.0.to_string(),
            workflow_id: run.workflow_id.0.to_string(),
            workflow_name: run.workflow_name,
            state,
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|t| t.to_rfc3339()),
            output: run.output,
            error: run.error,
            step_count: run.step_results.len(),
            last_step_name: run.step_results.last().map(|r| r.step_name.clone()),
        })
    }

    async fn start_workflow_async(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        use crate::workflow::WorkflowId;
        use kernel_handle::KernelOpError;

        // Resolve workflow_id (UUID or name) — same logic as run_workflow.
        let wf_id = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
            WorkflowId(uuid)
        } else {
            let name_lower = workflow_id.to_lowercase();
            let workflows = self.workflows.engine.list_workflows().await;
            workflows
                .iter()
                .find(|w| w.name.to_lowercase() == name_lower)
                .map(|w| w.id)
                .ok_or_else(|| {
                    KernelOpError::Internal(format!("workflow `{}` not found", workflow_id))
                })?
        };

        let run_id = self
            .workflows
            .engine
            .create_run(wf_id, input.to_string())
            .await
            .ok_or_else(|| KernelOpError::Internal("Workflow not found".to_string()))?;

        // Spawn execution in the background via self_handle (same pattern as
        // trigger dispatch — upgrade the stored Weak<LibreFangKernel> so the
        // spawned task can call send_message through the full kernel).
        let kernel_arc = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                KernelOpError::Internal(
                    "kernel not yet initialised for async workflow spawn".to_string(),
                )
            })?;

        tokio::spawn(async move {
            // Both closures must be `Fn` (not `FnOnce`), so we clone the Arc
            // on each invocation rather than moving it into the closure body.
            let k1 = std::sync::Arc::clone(&kernel_arc);
            let k2 = std::sync::Arc::clone(&kernel_arc);
            let resolver = move |agent_ref: &crate::workflow::StepAgent| {
                use librefang_types::agent::AgentId;
                match agent_ref {
                    crate::workflow::StepAgent::ById { id } => {
                        let agent_id: AgentId = id.parse().ok()?;
                        let entry = k1.agents.registry.get(agent_id)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((agent_id, entry.name.clone(), inherit))
                    }
                    crate::workflow::StepAgent::ByName { name } => {
                        let entry = k1.agents.registry.find_by_name(name)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((entry.id, entry.name.clone(), inherit))
                    }
                }
            };
            // `session_mode_override` carries `WorkflowStep::session_mode`
            // (#4834). Threaded through `send_message_full`'s existing
            // session-mode-override slot so the async-spawn path matches the
            // synchronous `run_workflow` path in precedence: per-step
            // override > target agent manifest > Persistent default.
            let send_message = move |agent_id: librefang_types::agent::AgentId,
                                     message: String,
                                     session_mode_override: Option<
                librefang_types::agent::SessionMode,
            >| {
                let k = std::sync::Arc::clone(&k2);
                async move {
                    let handle = k.kernel_handle();
                    k.send_message_full(
                        agent_id,
                        &message,
                        handle,
                        None,
                        None,
                        session_mode_override,
                        None,
                        None,
                    )
                    .await
                    .map(|r| {
                        (
                            r.response,
                            r.total_usage.input_tokens,
                            r.total_usage.output_tokens,
                        )
                    })
                    .map_err(|e| format!("{e}"))
                }
            };
            // Don't swallow the result — without a log the agent that
            // called workflow_start has no way to learn the run failed
            // except by polling get_workflow_run for the Failed state.
            if let Err(e) = kernel_arc
                .workflows
                .engine
                .execute_run(run_id, resolver, send_message)
                .await
            {
                tracing::warn!(
                    run_id = %run_id,
                    "Async workflow execution failed: {e}"
                );
            }
        });

        Ok(run_id.0.to_string())
    }

    async fn cancel_workflow_run(&self, run_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        use crate::workflow::{CancelRunError, WorkflowRunId};
        use kernel_handle::KernelOpError;

        let uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|_| KernelOpError::Internal(format!("Invalid run_id UUID: {run_id}")))?;

        self.workflows
            .engine
            .cancel_run(WorkflowRunId(uuid))
            .await
            .map_err(|e| match e {
                CancelRunError::NotFound(_) => {
                    KernelOpError::Internal(format!("workflow run not found: {run_id}"))
                }
                CancelRunError::AlreadyTerminal { state, .. } => {
                    KernelOpError::Internal(format!("cannot cancel: run is already {state}"))
                }
            })
    }
}
