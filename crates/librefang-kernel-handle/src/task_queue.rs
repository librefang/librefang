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

    /// Count of tasks per status, as `(status, count)` pairs.
    ///
    /// Separate from [`Self::task_list`] because the summary a caller wants four integers for should not cost one materialised row per task: `task_list` has no `LIMIT`, `task_prune_finished` only deletes terminal rows, and `GET /api/tasks/status` — which the dashboard polls — was deriving its counts that way (#8219).
    ///
    /// The default implementation does exactly that, so a stub implementing this trait keeps working unchanged. The kernel overrides it with a `GROUP BY`.
    async fn task_status_counts(&self) -> Result<Vec<(String, u64)>, KernelOpError> {
        let tasks = self.task_list(None).await?;
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for task in &tasks {
            let status = task["status"].as_str().unwrap_or("").to_string();
            *counts.entry(status).or_insert(0) += 1;
        }
        Ok(counts.into_iter().collect())
    }

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
