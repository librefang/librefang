//! Goal tracking tool.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Synchronous tool; `require_kernel_typed` is sync so it works here
//! unchanged.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) fn tool_goal_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    // Validate input before touching the kernel
    let goal_id = input["goal_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("goal_id"))?;
    let status = input["status"].as_str();
    let progress = input["progress"].as_u64().map(|p| p.min(100) as u8);

    if status.is_none() && progress.is_none() {
        // Reason text kept byte-identical to the pre-#3576 String error; only
        // the error channel changed.
        return Err(ToolError::InvalidParameter {
            name: "status",
            reason: "At least one of 'status' or 'progress' must be provided".to_string(),
        });
    }

    if let Some(s) = status {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&s) {
            return Err(ToolError::InvalidParameter {
                name: "status",
                reason: format!(
                    "Invalid status '{s}'. Must be: pending, in_progress, completed, or cancelled"
                ),
            });
        }
    }

    let kh = require_kernel_typed(kernel)?;
    let updated = kh
        .goal_update(goal_id, status, progress)
        .map_err(ToolError::upstream)?;
    Ok(serde_json::to_string_pretty(&updated).unwrap_or_else(|_| updated.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn goal_update_without_status_or_progress_is_invalid_parameter() {
        let r = tool_goal_update(&json!({"goal_id": "g1"}), None);
        match r {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "status");
                assert!(reason.contains("At least one"));
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }

    #[test]
    fn goal_update_rejects_unknown_status() {
        let r = tool_goal_update(&json!({"goal_id": "g1", "status": "bogus"}), None);
        assert!(matches!(
            r,
            Err(ToolError::InvalidParameter { name: "status", .. })
        ));
    }

    #[test]
    fn goal_update_missing_goal_id_is_missing_parameter() {
        let r = tool_goal_update(&json!({"status": "completed"}), None);
        assert!(matches!(r, Err(ToolError::MissingParameter("goal_id"))));
    }

    #[test]
    fn goal_update_without_kernel_returns_unavailable() {
        // Valid input, but no kernel handle -> Unavailable (validation passed).
        let r = tool_goal_update(&json!({"goal_id": "g1", "status": "completed"}), None);
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }
}
