use async_trait::async_trait;

use super::*;

// ============================================================================
// 3. TaskQueue — shared task queue: post / claim / complete / list / etc.
// ============================================================================

#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Post a task to the shared task queue. Returns the task ID.
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, KernelOpError>;

    /// Claim the next available task (optionally filtered by assignee). Returns task JSON or None.
    async fn task_claim(&self, agent_id: &str) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// Mark a task as completed with a result string. `agent_id` identifies the completer.
    async fn task_complete(
        &self,
        agent_id: &str,
        task_id: &str,
        result: &str,
    ) -> Result<(), KernelOpError>;

    /// List tasks, optionally filtered by status.
    async fn task_list(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, KernelOpError>;

    /// Delete a task by ID. Returns true if deleted.
    async fn task_delete(&self, task_id: &str) -> Result<bool, KernelOpError>;

    /// Retry a task by resetting it to pending. Returns true if reset.
    async fn task_retry(&self, task_id: &str) -> Result<bool, KernelOpError>;

    /// Get a single task by ID including its result and retry_count.
    async fn task_get(&self, task_id: &str) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// Update a task's status to `pending` (reset) or `cancelled`.
    /// Returns true if the task was found and updated.
    async fn task_update_status(
        &self,
        task_id: &str,
        new_status: &str,
    ) -> Result<bool, KernelOpError>;
}
