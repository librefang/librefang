//! Hand tools (delegated to kernel via `KernelHandle`).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Clean kernel passthrough — no caller-auth concern.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

const ALLOWED_CONFIG_KEYS: &[&str] = &[
    "model",
    "temperature",
    "max_tokens",
    "system_prompt",
    "schedule",
    "session_mode",
    "auto_restart",
    "budget_limit",
];

fn truncate_char_boundary(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    match s.char_indices().nth(max_len) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn sanitize_value(val: &serde_json::Value, max_len: usize) -> String {
    let s = match val {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.len() > max_len {
        format!("{}…[truncated]", truncate_char_boundary(&s, max_len))
    } else {
        s
    }
}

fn validate_required_string<'a>(
    input: &'a serde_json::Value,
    key: &'static str,
) -> ToolResult<&'a str> {
    let val = input[key]
        .as_str()
        .ok_or(ToolError::MissingParameter(key))?;
    if val.trim().is_empty() {
        return Err(ToolError::InvalidParameter {
            name: key,
            reason: "must not be empty".to_string(),
        });
    }
    Ok(val)
}

pub(super) async fn tool_hand_list(kernel: Option<&Arc<dyn KernelHandle>>) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let mut hands = kh.hand_list().await.map_err(ToolError::upstream)?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    hands.sort_by(|a, b| {
        let id_a = a["id"].as_str().unwrap_or("");
        let id_b = b["id"].as_str().unwrap_or("");
        id_a.cmp(id_b)
    });

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = sanitize_value(&h["name"], 80);
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = sanitize_value(&h["description"], 200);

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
            lines.push(format!(
                "  Instance: {}",
                sanitize_value(&serde_json::Value::String(iid.to_string()), 64)
            ));
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
    let hand_id = validate_required_string(input, "hand_id")?;

    let mut config: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    let mut dropped_keys: Vec<&str> = Vec::new();

    if let Some(obj) = input["config"].as_object() {
        for (k, v) in obj {
            if ALLOWED_CONFIG_KEYS.contains(&k.as_str()) {
                config.insert(k.clone(), v.clone());
            } else {
                dropped_keys.push(k.as_str());
            }
        }
    }

    let result = kh
        .hand_activate(hand_id, config)
        .await
        .map_err(ToolError::upstream)?;

    let instance_id = sanitize_value(&result["instance_id"], 64);
    let agent_name = sanitize_value(&result["agent_name"], 80);
    let status = sanitize_value(&result["status"], 32);

    let mut output = format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    );

    if !dropped_keys.is_empty() {
        output.push_str(&format!(
            "\n  Warning: unknown config keys ignored: {}",
            dropped_keys.join(", ")
        ));
    }

    Ok(output)
}

pub(super) async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let hand_id = validate_required_string(input, "hand_id")?;

    let result = kh.hand_status(hand_id).await.map_err(ToolError::upstream)?;

    let icon = result["icon"].as_str().unwrap_or("");
    let name = sanitize_value(&result["name"], 80);
    let status = sanitize_value(&result["status"], 32);
    let instance_id = sanitize_value(&result["instance_id"], 64);
    let agent_name = sanitize_value(&result["agent_name"], 80);
    let activated = sanitize_value(&result["activated_at"], 40);

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
    let instance_id = validate_required_string(input, "instance_id")?;
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
