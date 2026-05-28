//! Cross-agent task board tools backed by `KernelHandle::task_*`.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576) — third slice after `tool_runner::{cron, schedule}`.

use super::error::{ToolError, ToolResult};
use super::{caller_agent_id_missing, require_kernel_typed};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let title = input["title"]
        .as_str()
        .ok_or(ToolError::MissingParameter("title"))?;
    let description = input["description"]
        .as_str()
        .ok_or(ToolError::MissingParameter("description"))?;
    let assigned_to = input["assigned_to"].as_str();
    let task_id = kh
        .task_post(title, description, assigned_to, caller_agent_id)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Task created with ID: {task_id}"))
}

pub(super) async fn tool_task_claim(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("task_claim"))?;
    match kh.task_claim(agent_id).await.map_err(ToolError::upstream)? {
        Some(task) => Ok(serde_json::to_string_pretty(&task)?),
        None => Ok("No tasks available.".to_string()),
    }
}

pub(super) async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("task_complete"))?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("task_id"))?;
    let result = input["result"]
        .as_str()
        .ok_or(ToolError::MissingParameter("result"))?;
    kh.task_complete(agent_id, task_id, result)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Task {task_id} marked as completed."))
}

pub(super) async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await.map_err(ToolError::upstream)?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    Ok(serde_json::to_string_pretty(&tasks)?)
}

pub(super) async fn tool_task_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("task_id"))?;
    match kh.task_get(task_id).await.map_err(ToolError::upstream)? {
        Some(task) => {
            // Project to the same six columns comms_task_status returns from
            // the bridge SQL — keeps the native tool's contract tight even if
            // task_get later grows additional fields.
            let projected = serde_json::json!({
                "status":       task.get("status").cloned().unwrap_or(serde_json::Value::Null),
                "result":       task.get("result").cloned().unwrap_or(serde_json::Value::Null),
                "title":        task.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "assigned_to":  task.get("assigned_to").cloned().unwrap_or(serde_json::Value::Null),
                "created_at":   task.get("created_at").cloned().unwrap_or(serde_json::Value::Null),
                "completed_at": task.get("completed_at").cloned().unwrap_or(serde_json::Value::Null),
            });
            Ok(serde_json::to_string_pretty(&projected)?)
        }
        // Behaviour preserved from the pre-#3576 contract: a missing task is a
        // readable success message, NOT a `NotFound` error (see
        // `test_task_status_not_found_returns_message`).
        None => Ok(format!("Task '{task_id}' not found.")),
    }
}

#[cfg(test)]
mod tests {
    //! Boundary tests that run BEFORE any kernel call. Kernel-roundtrip paths
    //! (success, not-found message, projection) are covered against the
    //! `CapturingKernel` stub in `tests/tool_runner_forwarding_task_cron.rs`.
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn task_post_without_kernel_returns_unavailable() {
        let r = tool_task_post(&json!({}), None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn task_claim_without_kernel_returns_unavailable() {
        let r = tool_task_claim(None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn task_complete_without_kernel_returns_unavailable() {
        let r = tool_task_complete(&json!({}), None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn task_list_without_kernel_returns_unavailable() {
        let r = tool_task_list(&json!({}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn task_status_without_kernel_returns_unavailable() {
        let r = tool_task_status(&json!({}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn caller_agent_id_missing_surfaces_as_missing_parameter() {
        let e = caller_agent_id_missing("task_claim");
        assert!(matches!(e, ToolError::MissingParameter("agent_id")));
        assert!(e.to_string().contains("agent_id"));
    }
}
