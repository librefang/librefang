//! Goal tracking tool.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) fn tool_goal_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    // Validate input before touching the kernel
    let goal_id = input["goal_id"]
        .as_str()
        .ok_or("Missing 'goal_id' parameter")?;
    let status = input["status"].as_str();
    let progress = input["progress"].as_u64().map(|p| p.min(100) as u8);

    if status.is_none() && progress.is_none() {
        return Err("At least one of 'status' or 'progress' must be provided".to_string());
    }

    if let Some(s) = status {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&s) {
            return Err(format!(
                "Invalid status '{}'. Must be: pending, in_progress, completed, or cancelled",
                s
            ));
        }
    }

    let kh = require_kernel(kernel)?;
    let updated = kh
        .goal_update(goal_id, status, progress)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_string_pretty(&updated).unwrap_or_else(|_| updated.to_string()))
}
