//! `event_publish` — fan out an event onto the kernel bus.

use super::require_kernel;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or("Missing 'event_type' parameter")?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Event '{event_type}' published successfully."))
}
