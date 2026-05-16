//! Cross-agent task board tools backed by `KernelHandle::task_*`.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let task_id = kh
        .task_post(title, description, assigned_to, caller_agent_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Task created with ID: {task_id}"))
}

pub(super) async fn tool_task_claim(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("task_claim requires a calling agent context")?;
    match kh.task_claim(agent_id).await.map_err(|e| e.to_string())? {
        Some(task) => {
            serde_json::to_string_pretty(&task).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok("No tasks available.".to_string()),
    }
}

pub(super) async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("task_complete requires a calling agent context")?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let result = input["result"]
        .as_str()
        .ok_or("Missing 'result' parameter")?;
    kh.task_complete(agent_id, task_id, result)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Task {task_id} marked as completed."))
}

pub(super) async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await.map_err(|e| e.to_string())?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    serde_json::to_string_pretty(&tasks).map_err(|e| format!("Serialize error: {e}"))
}

pub(super) async fn tool_task_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    match kh.task_get(task_id).await.map_err(|e| e.to_string())? {
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
            serde_json::to_string_pretty(&projected).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok(format!("Task '{task_id}' not found.")),
    }
}
