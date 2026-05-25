//! Canvas / A2UI tool — sanitize agent-generated HTML and write it to the
//! workspace `output/` directory.
//!
//! `tool_canvas_present` migrated from `Result<String, String>` to
//! `Result<String, ToolError>` (#3576). The `sanitize_canvas_html` helper is
//! `pub` (re-exported and unit-tested directly), so its `Result<_, String>`
//! signature is left untouched; its validation/security messages are mapped to
//! `ToolError::InvalidParameter` at the tool boundary, preserved verbatim.

use super::error::{ToolError, ToolResult};
use super::CANVAS_MAX_BYTES;
use std::path::{Path, PathBuf};

/// Sanitize HTML for canvas presentation.
///
/// SECURITY: Strips dangerous elements and attributes to prevent XSS:
/// - Rejects <script>, <iframe>, <object>, <embed>, <applet> tags
/// - Strips all on* event attributes (onclick, onload, onerror, etc.)
/// - Strips javascript:, data:text/html, vbscript: URLs
/// - Enforces size limit
pub fn sanitize_canvas_html(html: &str, max_bytes: usize) -> Result<String, String> {
    if html.is_empty() {
        return Err("Empty HTML content".to_string());
    }
    if html.len() > max_bytes {
        return Err(format!(
            "HTML too large: {} bytes (max {})",
            html.len(),
            max_bytes
        ));
    }

    let lower = html.to_lowercase();

    // Reject dangerous tags
    let dangerous_tags = [
        "<script", "</script", "<iframe", "</iframe", "<object", "</object", "<embed", "<applet",
        "</applet",
    ];
    for tag in &dangerous_tags {
        if lower.contains(tag) {
            return Err(format!("Forbidden HTML tag detected: {tag}"));
        }
    }

    // Reject event handler attributes (on*)
    // Match patterns like: onclick=, onload=, onerror=, onmouseover=, etc.
    static EVENT_PATTERN: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| regex_lite::Regex::new(r"(?i)\bon[a-z]+\s*=").unwrap());
    if EVENT_PATTERN.is_match(html) {
        return Err(
            "Forbidden event handler attribute detected (on* attributes are not allowed)"
                .to_string(),
        );
    }

    // Reject dangerous URL schemes
    let dangerous_schemes = ["javascript:", "vbscript:", "data:text/html"];
    for scheme in &dangerous_schemes {
        if lower.contains(scheme) {
            return Err(format!("Forbidden URL scheme detected: {scheme}"));
        }
    }

    Ok(html.to_string())
}

/// Canvas presentation tool handler.
pub(super) async fn tool_canvas_present(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> ToolResult {
    let html = input["html"]
        .as_str()
        .ok_or(ToolError::MissingParameter("html"))?;
    let title = input["title"].as_str().unwrap_or("Canvas");

    // Use configured max from task-local (set by agent_loop from KernelConfig), or default 512KB.
    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    // The sanitizer's validation/security messages are user-facing — map them
    // onto the `html` parameter, keeping the text verbatim.
    let sanitized =
        sanitize_canvas_html(html, max_bytes).map_err(|reason| ToolError::InvalidParameter {
            name: "html",
            reason,
        })?;

    // Generate canvas ID
    let canvas_id = uuid::Uuid::new_v4().to_string();

    // Save to workspace output directory
    let output_dir = if let Some(root) = workspace_root {
        root.join("output")
    } else {
        PathBuf::from("output")
    };
    let _ = tokio::fs::create_dir_all(&output_dir).await;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!(
        "canvas_{timestamp}_{}.html",
        crate::str_utils::safe_truncate_str(&canvas_id, 8)
    );
    let filepath = output_dir.join(&filename);

    // Write the full HTML document
    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
    );
    tokio::fs::write(&filepath, &full_html)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to save canvas: {e}"),
            source: Some(Box::new(e)),
        })?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": title,
        "saved_to": filepath.to_string_lossy(),
        "size_bytes": full_html.len(),
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn canvas_present_missing_html_is_missing_parameter() {
        let r = tool_canvas_present(&serde_json::json!({}), None).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("html"))));
    }

    #[tokio::test]
    async fn canvas_present_forbidden_tag_is_invalid_parameter() {
        let input = serde_json::json!({ "html": "<script>alert(1)</script>" });
        match tool_canvas_present(&input, None).await {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "html");
                assert!(reason.contains("Forbidden"));
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }
}
