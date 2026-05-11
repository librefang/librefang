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
}
