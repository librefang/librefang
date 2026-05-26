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
    let progress = if input.get("progress").is_some() {
        let raw = input["progress"]
            .as_f64()
            .ok_or("Parameter 'progress' must be a number".to_string())?;
        if !(0.0..=100.0).contains(&raw) {
            return Err(format!(
                "Parameter 'progress' must be between 0 and 100, got {}",
                raw
            ));
        }
        let val = raw.round() as u8;
        Some(val)
    } else {
        None
    };

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
