use librefang_types::tool::ToolResult;

const MAX_NOTIFY_LEN: usize = 4096;

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == ' ').collect()
}

fn truncate_chars(s: &str, limit: usize) -> String {
    s.chars().take(limit).collect()
}

pub(super) fn tool_notify_owner(tool_use_id: &str, input: &serde_json::Value) -> ToolResult {
    let reason_raw = input
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let summary_raw = input
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let reason = truncate_chars(&sanitize(reason_raw), MAX_NOTIFY_LEN);
    let summary = truncate_chars(&sanitize(summary_raw), MAX_NOTIFY_LEN);

    if reason.is_empty() || summary.is_empty() {
        return ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: "Error: notify_owner requires non-empty 'reason' and 'summary' string fields."
                .to_string(),
            is_error: true,
            ..Default::default()
        };
    }

    let owner_payload = format!("[NOTIFY] {reason}: {summary}");

    tracing::info!(
        event = "owner_notify",
        reason_len = reason.len(),
        summary_len = summary.len(),
        "notify_owner tool invoked"
    );

    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: "Notice queued for the owner. Do not repeat the summary in your public reply."
            .to_string(),
        is_error: false,
        owner_notice: Some(owner_payload),
        ..Default::default()
    }
}
