//! `event_publish` — fan out an event onto the kernel bus.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Caller-auth gate added: `caller_agent_id` is required so
//! reserved-prefix enforcement can attribute the attempt.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

const RESERVED_PREFIXES: &[&str] = &["system.", "internal."];

pub(super) async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let caller_agent_id = caller_agent_id.ok_or_else(|| {
        ToolError::PermissionDenied("caller_agent_id is required to publish events".into())
    })?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or(ToolError::MissingParameter("event_type"))?;

    let trimmed = event_type.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "event_type",
            reason: "must not be empty or whitespace-only".into(),
        });
    }

    if RESERVED_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return Err(ToolError::PermissionDenied(format!(
            "agent '{caller_agent_id}' cannot publish reserved event type '{trimmed}'"
        )));
    }

    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(trimmed, payload)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Event '{trimmed}' published successfully."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn event_publish_without_kernel_returns_unavailable() {
        let r = tool_event_publish(&json!({"event_type": "x"}), None, Some("agent-A")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }
}
