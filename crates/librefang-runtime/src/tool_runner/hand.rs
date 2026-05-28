//! Hand tools (delegated to kernel via `KernelHandle`).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Clean kernel passthrough — no caller-auth concern.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_hand_list(kernel: Option<&Arc<dyn KernelHandle>>) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let hands = kh.hand_list().await.map_err(ToolError::upstream)?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = h["name"].as_str().unwrap_or("?");
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = h["description"].as_str().unwrap_or("");

        let status_marker = match status {
            "Active" => "[ACTIVE]",
            "Paused" => "[PAUSED]",
            _ => "[available]",
        };

        lines.push(format!("{} {} ({}) {}", icon, name, id, status_marker));
        if !desc.is_empty() {
            lines.push(format!("  {}", desc));
        }
        if let Some(iid) = h["instance_id"].as_str() {
            lines.push(format!("  Instance: {}", iid));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

pub(super) async fn tool_hand_activate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("hand_id"))?;
    let config: std::collections::HashMap<String, serde_json::Value> =
        if let Some(obj) = input["config"].as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            std::collections::HashMap::new()
        };

    let result = kh
        .hand_activate(hand_id, config)
        .await
        .map_err(ToolError::upstream)?;

    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");

    Ok(format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    ))
}

pub(super) async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("hand_id"))?;

    let result = kh.hand_status(hand_id).await.map_err(ToolError::upstream)?;

    let icon = result["icon"].as_str().unwrap_or("");
    let name = result["name"].as_str().unwrap_or(hand_id);
    let status = result["status"].as_str().unwrap_or("unknown");
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let activated = result["activated_at"].as_str().unwrap_or("?");

    Ok(format!(
        "{} {} — {}\n  Instance: {}\n  Agent: {}\n  Activated: {}",
        icon, name, status, instance_id, agent_name, activated
    ))
}

pub(super) async fn tool_hand_deactivate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let instance_id = input["instance_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("instance_id"))?;
    kh.hand_deactivate(instance_id)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Hand instance '{}' deactivated.", instance_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn hand_list_without_kernel_returns_unavailable() {
        assert!(matches!(
            tool_hand_list(None).await,
            Err(ToolError::Unavailable("Kernel handle"))
        ));
    }

    #[tokio::test]
    async fn hand_activate_without_kernel_returns_unavailable() {
        assert!(matches!(
            tool_hand_activate(&json!({"hand_id": "x"}), None).await,
            Err(ToolError::Unavailable("Kernel handle"))
        ));
    }

    #[tokio::test]
    async fn hand_status_without_kernel_returns_unavailable() {
        assert!(matches!(
            tool_hand_status(&json!({"hand_id": "x"}), None).await,
            Err(ToolError::Unavailable("Kernel handle"))
        ));
    }

    #[tokio::test]
    async fn hand_deactivate_without_kernel_returns_unavailable() {
        assert!(matches!(
            tool_hand_deactivate(&json!({"instance_id": "x"}), None).await,
            Err(ToolError::Unavailable("Kernel handle"))
        ));
    }
}
