//! Canvas / A2UI tool — sanitize agent-generated HTML and write it to the
//! workspace `output/` directory.

use super::CANVAS_MAX_BYTES;
use std::path::{Path, PathBuf};

fn html_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
}

fn decode_html_entities(s: &str) -> String {
    static ENTITY_RE: std::sync::LazyLock<regex_lite::Regex> = std::sync::LazyLock::new(|| {
        regex_lite::Regex::new(r"&#(\d+);|&#[xX]([0-9a-fA-F]+);").unwrap()
    });
    let mut result = String::with_capacity(s.len());
    let mut pos = 0;
    while pos < s.len() {
        if let Some(m) = ENTITY_RE.find_at(s, pos) {
            result.push_str(&s[pos..m.start()]);
            if let Some(caps) = ENTITY_RE.captures_at(s, pos) {
                if let Some(dec) = caps.get(1) {
                    if let Ok(n) = dec.as_str().parse::<u32>() {
                        if let Some(c) = char::from_u32(n) {
                            result.push(c);
                        }
                    }
                } else if let Some(hex) = caps.get(2) {
                    if let Ok(n) = u32::from_str_radix(hex.as_str(), 16) {
                        if let Some(c) = char::from_u32(n) {
                            result.push(c);
                        }
                    }
                }
            }
            pos = m.end();
        } else {
            result.push_str(&s[pos..]);
            break;
        }
    }
    result
}

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

    static DANGEROUS_TAG_RE: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| {
            regex_lite::Regex::new(r"(?i)<\s*/?\s*(script|iframe|object|embed|applet)\b").unwrap()
        });
    if let Some(m) = DANGEROUS_TAG_RE.find(html) {
        return Err(format!("Forbidden HTML tag detected: {}", m.as_str()));
    }

    static EVENT_PATTERN: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| regex_lite::Regex::new(r"(?i)\bon[a-z]+\s*=").unwrap());
    if EVENT_PATTERN.is_match(html) {
        return Err(
            "Forbidden event handler attribute detected (on* attributes are not allowed)"
                .to_string(),
        );
    }

    let decoded = decode_html_entities(html);
    static DANGEROUS_SCHEME_RE: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| {
            regex_lite::Regex::new(r"(?i)(?:javascript\s*:|vbscript\s*:|data\s*:\s*text/html)")
                .unwrap()
        });
    if let Some(m) = DANGEROUS_SCHEME_RE.find(&decoded) {
        return Err(format!("Forbidden URL scheme detected: {}", m.as_str()));
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

    let escaped_title = html_escape(title);
    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{escaped_title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
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
