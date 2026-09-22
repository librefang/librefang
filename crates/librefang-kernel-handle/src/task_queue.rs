use async_trait::async_trait;

use super::*;

// ============================================================================
// 3. TaskQueue — shared task queue: post / claim / complete / list / etc.
// ============================================================================

/// Per-task scheduling limits carried by [`TaskQueue::task_post`].
///
/// Both fields exist because the Task Board enforces them, not because they
/// document intent: `priority` is the key the claim queue is ordered by, and
/// `timeout_secs` is the deadline the stuck-task sweeper measures against.
/// [`Default`] is the historical behaviour, so a caller with no opinion passes
/// `&TaskPostOptions::default()` and gets exactly what it got before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TaskPostOptions {
    /// Claim-queue ordering key — higher is served first, ties broken by age.
    /// `0` is the neutral default every historical row carries.
    pub priority: i64,
    /// Per-task override for `[task_board] claim_ttl_secs`, in seconds.
    ///
    /// `None` inherits the global TTL. `Some(0)` means "never reclaim" — the
    /// per-task spelling of the global disable. `Some(n)` reclaims the task
    /// after it has been held `in_progress` for `n` seconds.
    pub timeout_secs: Option<u32>,
}

#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Post a task to the shared task queue. Returns the task ID.
    ///
    /// `opts` carries the enforced per-task limits; see [`TaskPostOptions`].
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
        opts: &TaskPostOptions,
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

    /// List one page of tasks, plus the number of rows the filters match before the window is applied.
    ///
    /// Separate from [`Self::task_list`] because the API's `?limit=` and `?assigned_to=` were both applied to a fully materialised `Vec`, so a request for ten tasks allocated one `serde_json::Value` per row in the table first (#8219) — and `task_prune_finished` only removes terminal rows, so that table grows for the life of the install.
    ///
    /// The default implementation does exactly that filtering in Rust, so a stub implementing this trait keeps working unchanged.
    /// The kernel overrides it with `WHERE` / `LIMIT` / `OFFSET`.
    async fn task_list_page(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<(Vec<serde_json::Value>, u64), KernelOpError> {
        let mut tasks = self.task_list(status).await?;
        if let Some(assignee) = assigned_to {
            tasks.retain(|t| t["assigned_to"].as_str().unwrap_or("") == assignee);
        }
        let total = tasks.len() as u64;
        let offset = offset.unwrap_or(0) as usize;
        if offset > 0 {
            tasks.drain(..offset.min(tasks.len()));
        }
        if let Some(limit) = limit {
            tasks.truncate(limit as usize);
        }
        Ok((tasks, total))
    }

    /// Count of tasks per status, as `(status, count)` pairs.
    ///
    /// Separate from [`Self::task_list`] because the summary a caller wants four integers for should not cost one materialised row per task: `task_list` has no `LIMIT`, `task_prune_finished` only deletes terminal rows, and `GET /api/tasks/status` — which the dashboard polls — was deriving its counts that way (#8219).
    ///
    /// The default implementation does exactly that, so a stub implementing this trait keeps working unchanged.
    /// The kernel overrides it with a `GROUP BY`.
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
