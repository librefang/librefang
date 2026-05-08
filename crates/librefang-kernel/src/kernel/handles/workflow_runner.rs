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
}
