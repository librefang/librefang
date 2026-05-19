//! Canvas / A2UI tool — sanitize agent-generated HTML and write it to the
//! workspace `output/` directory.

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
) -> Result<String, String> {
    let html = input["html"].as_str().ok_or("Missing 'html' parameter")?;
    let title = input["title"].as_str().unwrap_or("Canvas");

    // Use configured max from task-local (set by agent_loop from KernelConfig), or default 512KB.
    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    let sanitized = sanitize_canvas_html(html, max_bytes)?;

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
        .map_err(|e| format!("Failed to save canvas: {e}"))?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": title,
        "saved_to": filepath.to_string_lossy(),
        "size_bytes": full_html.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}
