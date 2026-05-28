//! `event_publish` — fan out an event onto the kernel bus.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Pure kernel passthrough — no caller-auth concern.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or(ToolError::MissingParameter("event_type"))?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Event '{event_type}' published successfully."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn event_publish_without_kernel_returns_unavailable() {
        let r = tool_event_publish(&json!({"event_type": "x"}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }
}
