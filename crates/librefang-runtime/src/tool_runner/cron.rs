//! Low-level cron tools — direct KernelHandle::cron_* wrappers.
//!
//! For the natural-language `schedule_*` family see `tool_runner::schedule`.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_create")?;
    let mut job = input.clone();
    if let (Some(pid), Some(obj)) = (sender_id, job.as_object_mut()) {
        if !pid.is_empty() && !obj.contains_key("peer_id") {
            obj.insert(
                "peer_id".to_string(),
                serde_json::Value::String(pid.to_string()),
            );
        }
    }
    kh.cron_create(agent_id, job)
        .await
        .map_err(|e| e.to_string())
}

pub(super) async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_list")?;
    let jobs = kh.cron_list(agent_id).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&jobs).map_err(|e| format!("Failed to serialize cron jobs: {e}"))
}

pub(super) async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or("Missing 'job_id' parameter")?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_cancel")?;
    // Authorize: the caller may only cancel jobs that belong to them.
    // Otherwise an agent with the cron_cancel tool could delete any other
    // agent's jobs as long as it learns their UUID (via side-channel or
    // social engineering).
    let owned = kh.cron_list(agent_id).await.map_err(|e| e.to_string())?;
    let owns_job = owned.iter().any(|job| {
        job.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == job_id)
    });
    if !owns_job {
        return Err(format!(
            "Cron job '{job_id}' not found or not owned by this agent"
        ));
    }
    kh.cron_cancel(job_id).await.map_err(|e| e.to_string())?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}
