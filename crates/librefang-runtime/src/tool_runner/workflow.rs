//! Workflow execution tools — run / list / status / start / cancel.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_workflow_run(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    // Serialize optional input object to a JSON string for the workflow engine.
    let input_str = match input.get("input") {
        Some(v) if v.is_object() => serde_json::to_string(v)
            .map_err(|e| format!("Failed to serialize workflow input: {e}"))?,
        Some(v) if v.is_null() => String::new(),
        Some(_) => return Err("'input' must be a JSON object or null".to_string()),
        None => String::new(),
    };

    let kh = require_kernel(kernel)?;
    let (run_id, output) = kh
        .run_workflow(workflow_id, &input_str)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "run_id": run_id,
        "output": output,
    })
    .to_string())
}

pub(super) async fn tool_workflow_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let mut summaries = kh.list_workflows().await;
    // Sort by name for deterministic LLM prompt output (#3298).
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    let json_array: Vec<serde_json::Value> = summaries
        .into_iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "name": w.name,
                "description": w.description,
                "step_count": w.step_count,
            })
        })
        .collect();
    serde_json::to_string(&json_array)
        .map_err(|e| format!("Failed to serialize workflow list: {e}"))
}

pub(super) async fn tool_workflow_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;

    // Validate UUID format before calling kernel — returns a clear error
    // rather than silently returning not-found for a malformed id.
    uuid::Uuid::parse_str(run_id)
        .map_err(|_| format!("Invalid run_id — must be a UUID: {run_id}"))?;

    let kh = require_kernel(kernel)?;
    let summary = kh
        .get_workflow_run(run_id)
        .await
        .ok_or_else(|| format!("workflow run not found: {run_id}"))?;

    serde_json::to_string(&serde_json::json!({
        "run_id": summary.run_id,
        "workflow_id": summary.workflow_id,
        "workflow_name": summary.workflow_name,
        "state": summary.state,
        "started_at": summary.started_at,
        "completed_at": summary.completed_at,
        "output": summary.output,
        "error": summary.error,
        "step_count": summary.step_count,
        "last_step_name": summary.last_step_name,
    }))
    .map_err(|e| format!("Failed to serialize workflow status: {e}"))
}

pub(super) async fn tool_workflow_start(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    caller_session_id: Option<librefang_types::agent::SessionId>,
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    // Serialize optional input object to a JSON string for the workflow engine.
    let input_str = match input.get("input") {
        Some(v) if v.is_object() => serde_json::to_string(v)
            .map_err(|e| format!("Failed to serialize workflow input: {e}"))?,
        Some(v) if v.is_null() => String::new(),
        Some(_) => return Err("'input' must be a JSON object or null".to_string()),
        None => String::new(),
    };

    let kh = require_kernel(kernel)?;

    // Forward caller context so the kernel can register the workflow on
    // its async-task tracker (#4983) and inject a `TaskCompletionEvent`
    // into the originating session when the run finishes. Falls back to
    // the historical fire-and-forget behaviour when either id is
    // missing (legacy / test call sites that don't carry context).
    let session_id_str = caller_session_id.map(|sid| sid.0.to_string());
    let run_id = kh
        .start_workflow_async_tracked(
            workflow_id,
            &input_str,
            caller_agent_id,
            session_id_str.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "run_id": run_id }).to_string())
}

pub(super) async fn tool_workflow_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;

    // Validate UUID format before calling kernel.
    uuid::Uuid::parse_str(run_id)
        .map_err(|_| format!("Invalid run_id — must be a UUID: {run_id}"))?;

    let kh = require_kernel(kernel)?;
    kh.cancel_workflow_run(run_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "run_id": run_id,
        "state": "cancelled",
    })
    .to_string())
}
