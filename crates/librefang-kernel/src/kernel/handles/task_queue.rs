//! [`kernel_handle::TaskQueue`] — durable task posting / claiming /
//! completion against the memory substrate. Each lifecycle change publishes
//! a [`SystemEvent`] so triggers and dashboards observe the transition.

use std::str::FromStr;

use librefang_runtime::kernel_handle;
use librefang_types::agent::AgentId;

use super::super::LibreFangKernel;

#[async_trait::async_trait]
impl kernel_handle::TaskQueue for LibreFangKernel {
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let task_id = self
            .memory
            .task_post(title, description, assigned_to, created_by)
            .await
            .map_err(|e| KernelOpError::Internal(format!("Task post failed: {e}")))?;

        let event = librefang_types::event::Event::new(
            AgentId::new(), // system-originated
            librefang_types::event::EventTarget::Broadcast,
            librefang_types::event::EventPayload::System(
                librefang_types::event::SystemEvent::TaskPosted {
                    task_id: task_id.clone(),
                    title: title.to_string(),
                    assigned_to: assigned_to.map(String::from),
                    created_by: created_by.map(String::from),
                },
            ),
        );
        self.publish_event(event).await;

        Ok(task_id)
    }

    async fn task_claim(
        &self,
        agent_id: &str,
    ) -> Result<Option<serde_json::Value>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        // Resolve `agent_id` to a canonical UUID and also capture the name.
        // Both are forwarded to `memory.task_claim` so that tasks whose
        // `assigned_to` field was stored as either a UUID *or* a name string
        // are correctly matched (issue #2841).
        let (resolved, resolved_name) = match librefang_types::agent::AgentId::from_str(agent_id) {
            Ok(parsed_id) => {
                // Caller passed a UUID — look up the name from the registry.
                let name = self.registry.get(parsed_id).map(|e| e.name.clone());
                (agent_id.to_string(), name)
            }
            Err(_) => match self.registry.find_by_name(agent_id) {
                Some(entry) => (entry.id.to_string(), Some(agent_id.to_string())),
                None => {
                    return Err(KernelOpError::AgentNotFound(agent_id.to_string()));
                }
            },
        };
        let result = self
            .memory
            .task_claim(&resolved, resolved_name.as_deref())
            .await
            .map_err(|e| KernelOpError::Internal(format!("Task claim failed: {e}")))?;

        if let Some(ref task) = result {
            let task_id = task["id"].as_str().unwrap_or("").to_string();
            let event = librefang_types::event::Event::new(
                AgentId::new(), // system-originated
                librefang_types::event::EventTarget::Broadcast,
                librefang_types::event::EventPayload::System(
                    librefang_types::event::SystemEvent::TaskClaimed {
                        task_id,
                        claimed_by: resolved.clone(),
                    },
                ),
            );
            self.publish_event(event).await;
        }

        Ok(result)
    }

    async fn task_complete(
        &self,
        agent_id: &str,
        task_id: &str,
        result: &str,
    ) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let resolved = match librefang_types::agent::AgentId::from_str(agent_id) {
            Ok(_) => agent_id.to_string(),
            Err(_) => match self.registry.find_by_name(agent_id) {
                Some(entry) => entry.id.to_string(),
                None => {
                    return Err(KernelOpError::AgentNotFound(agent_id.to_string()));
                }
            },
        };
        self.memory
            .task_complete(task_id, result)
            .await
            .map_err(|e| KernelOpError::Internal(format!("Task complete failed: {e}")))?;

        let event = librefang_types::event::Event::new(
            AgentId::new(), // system-originated
            librefang_types::event::EventTarget::Broadcast,
            librefang_types::event::EventPayload::System(
                librefang_types::event::SystemEvent::TaskCompleted {
                    task_id: task_id.to_string(),
                    completed_by: resolved,
                    result: result.to_string(),
                },
            ),
        );
        self.publish_event(event).await;

        Ok(())
    }

    async fn task_list(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        self.memory
            .task_list(status)
            .await
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("Task list failed: {e}")))
    }

    async fn task_delete(&self, task_id: &str) -> Result<bool, kernel_handle::KernelOpError> {
        self.memory
            .task_delete(task_id)
            .await
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("Task delete failed: {e}")))
    }

    async fn task_retry(&self, task_id: &str) -> Result<bool, kernel_handle::KernelOpError> {
        self.memory
            .task_retry(task_id)
            .await
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("Task retry failed: {e}")))
    }

    async fn task_get(
        &self,
        task_id: &str,
    ) -> Result<Option<serde_json::Value>, kernel_handle::KernelOpError> {
        self.memory
            .task_get(task_id)
            .await
            .map_err(|e| kernel_handle::KernelOpError::Internal(format!("Task get failed: {e}")))
    }

    async fn task_update_status(
        &self,
        task_id: &str,
        new_status: &str,
    ) -> Result<bool, kernel_handle::KernelOpError> {
        self.memory
            .task_update_status(task_id, new_status)
            .await
            .map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!("Task update status failed: {e}"))
            })
    }
}
