//! `notify_owner` tool — opaque acknowledgement back to the operator.

use librefang_types::tool::ToolResult;

pub(super) fn tool_notify_owner(tool_use_id: &str, input: &serde_json::Value) -> ToolResult {
    let reason = input
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let summary = input
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if reason.is_empty() || summary.is_empty() {
        return ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: "Error: notify_owner requires non-empty 'reason' and 'summary' string fields."
                .to_string(),
            is_error: true,
            ..Default::default()
        };
    }

    // Compose the owner-side payload. The reason is prefixed so the operator
    // can scan a long stream of notices without parsing the body. Format:
    //     🎩 {reason}: {summary}
    let owner_payload = format!("🎩 {reason}: {summary}");

    // Structured log per OBS-01 — dispatch decision is recorded even before
    // the gateway fans it out. Target JID(s) are resolved downstream.
    tracing::info!(
        event = "owner_notify",
        reason = %reason,
        summary_len = summary.len(),
        "notify_owner tool invoked"
    );

    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        // Opaque ack — intentionally devoid of summary content.
        content: "Notice queued for the owner. Do not repeat the summary in your public reply."
            .to_string(),
        is_error: false,
        owner_notice: Some(owner_payload),
        ..Default::default()
    }
}
