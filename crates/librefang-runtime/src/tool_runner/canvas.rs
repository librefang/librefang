//! Canvas / A2UI tool — sanitize agent-generated HTML and write it to the
//! workspace `output/` directory.
//!
//! `tool_canvas_present` migrated from `Result<String, String>` to
//! `Result<String, ToolError>` (#3576). The `sanitize_canvas_html` helper is
//! `pub` (re-exported and unit-tested directly), so its `Result<_, String>`
//! signature is left untouched; its validation/security messages are mapped to
//! `ToolError::InvalidParameter` at the tool boundary, preserved verbatim.

use super::error::{ToolError, ToolResult};
use super::{CANVAS_ALLOWED_TAGS, CANVAS_MAX_BYTES};
use std::path::{Path, PathBuf};

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

const ALLOWED_TAGS: &[&str] = &[
    "p",
    "br",
    "hr",
    "b",
    "i",
    "u",
    "s",
    "strong",
    "em",
    "span",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "caption",
    "colgroup",
    "col",
    "a",
    "img",
    "figure",
    "figcaption",
    "blockquote",
    "pre",
    "code",
    "details",
    "summary",
    "mark",
    "small",
    "sub",
    "sup",
    "abbr",
];

const VOID_TAGS: &[&str] = &["br", "hr", "img", "col"];

fn is_allowed_tag(name: &str, allowed_tags: &[String]) -> bool {
    if allowed_tags.is_empty() {
        // Operator did not override `[canvas] allowed_tags` — fall back to the
        // built-in conservative allowlist (unchanged behaviour).
        ALLOWED_TAGS.contains(&name)
    } else {
        allowed_tags.iter().any(|t| t.eq_ignore_ascii_case(name))
    }
}

fn is_void_tag(name: &str) -> bool {
    VOID_TAGS.contains(&name)
}

fn is_safe_url(url: &str) -> bool {
    let trimmed = url.trim().trim_matches(|c| c == '"' || c == '\'');
    let lower = trimmed.to_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
        return false;
    }
    if lower.starts_with("data:") {
        let safe_prefixes = [
            "data:image/png;",
            "data:image/jpeg;",
            "data:image/gif;",
            "data:image/webp;",
            "data:image/svg+xml;",
        ];
        return safe_prefixes.iter().any(|p| lower.starts_with(p));
    }
    true
}

fn consume_attr(rest: &str) -> (&str, Option<(String, String)>) {
    let name_end = rest
        .find(|c: char| c == '=' || c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(rest.len());
    let attr_name = rest[..name_end].trim();
    if attr_name.is_empty() {
        return ("", None);
    }
    let after_name = &rest[name_end..];
    let (remaining, value) = if after_name.trim_start().starts_with('=') {
        let eq_pos = after_name.find('=').unwrap();
        let after_eq = after_name[eq_pos + 1..].trim_start();
        if let Some((val_str, consumed)) = parse_attr_value(after_eq) {
            (after_eq[consumed..].trim_start(), Some(val_str.to_string()))
        } else {
            (after_eq, Some(String::from("\"\"")))
        }
    } else {
        (after_name.trim_start(), None)
    };
    (
        remaining,
        Some((attr_name.to_string(), value.unwrap_or_default())),
    )
}

fn strip_dangerous_attrs(attrs: &str) -> String {
    let mut safe = String::new();
    let mut rest = attrs.trim();
    while !rest.is_empty() {
        let (remaining, parsed) = consume_attr(rest);
        if remaining.is_empty() && parsed.is_none() {
            break;
        }
        rest = remaining;
        let (name, value) = match parsed {
            Some(p) => p,
            None => break,
        };
        let lower = name.to_lowercase();
        if lower.starts_with("on") || lower == "style" {
            continue;
        }
        if (lower == "href" || lower == "src") && !value.is_empty() && !is_safe_url(&value) {
            continue;
        }
        if !safe.is_empty() {
            safe.push(' ');
        }
        safe.push_str(&name);
        if !value.is_empty() {
            safe.push('=');
            safe.push_str(&value);
        }
    }
    safe
}

fn parse_attr_value(s: &str) -> Option<(&str, usize)> {
    if let Some(stripped) = s.strip_prefix('"') {
        let end = stripped.find('"').map(|i| i + 1)?;
        Some((&s[..end + 1], end + 1))
    } else if let Some(stripped) = s.strip_prefix('\'') {
        let end = stripped.find('\'').map(|i| i + 1)?;
        Some((&s[..end + 1], end + 1))
    } else {
        let end = s
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        Some((&s[..end], end))
    }
}

fn parse_tag_open(html: &str) -> Option<(String, String, usize)> {
    let rest = html.strip_prefix('<')?;
    let name_end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    let tag_name = rest[..name_end].to_lowercase();
    let after_name = &rest[name_end..];
    let close_pos = after_name.find('>')?;
    let attrs = after_name[..close_pos].trim();
    Some((tag_name, attrs.to_string(), 1 + name_end + close_pos + 1))
}

fn parse_tag_close(html: &str) -> Option<(String, usize)> {
    let rest = html.strip_prefix("</")?;
    let close_pos = rest.find('>')?;
    let name = rest[..close_pos].trim().to_lowercase();
    Some((name, 2 + close_pos + 1))
}

pub fn sanitize_canvas_html(html: &str, max_bytes: usize) -> Result<String, String> {
    sanitize_canvas_html_with_tags(html, max_bytes, &[])
}

/// Like [`sanitize_canvas_html`] but with an operator-configured tag
/// allowlist. An empty `allowed_tags` falls back to the built-in
/// `ALLOWED_TAGS`; a non-empty list replaces it (case-insensitive).
pub fn sanitize_canvas_html_with_tags(
    html: &str,
    max_bytes: usize,
    allowed_tags: &[String],
) -> Result<String, String> {
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
    let dangerous_tags = [
        "<script", "</script", "<iframe", "</iframe", "<object", "</object", "<embed", "<applet",
        "</applet", "<form", "</form", "<input", "<button", "</button", "<meta", "<link", "<base",
    ];
    for tag in &dangerous_tags {
        if lower.contains(tag) {
            return Err(format!("Forbidden HTML tag detected: {tag}"));
        }
    }

    let mut result = String::with_capacity(html.len());
    let mut pos = 0;
    let bytes = html.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            if bytes[pos..].starts_with(b"<!--") {
                if let Some(end) = html[pos..].find("-->") {
                    pos += end + 3;
                    continue;
                }
                return Err("Unclosed HTML comment".to_string());
            }
            if bytes[pos..].starts_with(b"</") {
                if let Some((name, consumed)) = parse_tag_close(&html[pos..]) {
                    if is_allowed_tag(&name, allowed_tags) {
                        result.push_str("</");
                        result.push_str(&name);
                        result.push('>');
                    }
                    pos += consumed;
                    continue;
                }
            }
            if let Some((name, attrs, consumed)) = parse_tag_open(&html[pos..]) {
                if is_allowed_tag(&name, allowed_tags) {
                    let safe_attrs = strip_dangerous_attrs(&attrs);
                    result.push('<');
                    result.push_str(&name);
                    if !safe_attrs.is_empty() {
                        result.push(' ');
                        result.push_str(&safe_attrs);
                    }
                    if is_void_tag(&name) {
                        result.push_str(" /");
                    }
                    result.push('>');
                }
                pos += consumed;
                continue;
            }
            result.push_str("&lt;");
            pos += 1;
            continue;
        }
        if bytes[pos] == b'>' {
            result.push_str("&gt;");
            pos += 1;
            continue;
        }
        if bytes[pos] == b'&' {
            if let Some(semi) = html[pos + 1..].find(';') {
                let content = &html[pos + 1..pos + 1 + semi];
                let valid = if content.is_empty() {
                    false
                } else if content.as_bytes()[0] == b'#' {
                    let num = &content[1..];
                    !num.is_empty()
                        && (num.bytes().all(|b| b.is_ascii_digit())
                            || (num.len() > 1
                                && (num.as_bytes()[0] == b'x' || num.as_bytes()[0] == b'X')
                                && num[1..].bytes().all(|b| b.is_ascii_hexdigit())))
                } else {
                    content.bytes().all(|b| b.is_ascii_alphabetic())
                };
                if valid {
                    let entity = &html[pos..pos + 2 + semi];
                    result.push_str(entity);
                    pos += 2 + semi;
                    continue;
                }
            }
            result.push_str("&amp;");
            pos += 1;
            continue;
        }
        let start = pos;
        while pos < bytes.len() && bytes[pos] != b'<' && bytes[pos] != b'>' && bytes[pos] != b'&' {
            pos += 1;
        }
        result.push_str(&html[start..pos]);
    }

    if result.len() > max_bytes {
        return Err(format!(
            "Sanitized HTML too large: {} bytes (max {})",
            result.len(),
            max_bytes
        ));
    }

    Ok(result)
}

pub(super) async fn tool_canvas_present(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> ToolResult {
    let html = input["html"]
        .as_str()
        .ok_or(ToolError::MissingParameter("html"))?;
    let raw_title = input["title"].as_str().unwrap_or("Canvas");
    let title = escape_html(raw_title);

    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    let allowed_tags = CANVAS_ALLOWED_TAGS
        .try_with(|t| t.clone())
        .unwrap_or_default();
    // The sanitizer's validation/security messages are user-facing — map them
    // onto the `html` parameter, keeping the text verbatim.
    let sanitized =
        sanitize_canvas_html_with_tags(html, max_bytes, &allowed_tags).map_err(|reason| {
            ToolError::InvalidParameter {
                name: "html",
                reason,
            }
        })?;

    let canvas_id = uuid::Uuid::new_v4().to_string();

    let output_dir = if let Some(root) = workspace_root {
        root.join("output")
    } else {
        PathBuf::from("output")
    };
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to create output directory: {e}"),
            source: Some(Box::new(e)),
        })?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!(
        "canvas_{timestamp}_{}.html",
        crate::str_utils::safe_truncate_str(&canvas_id, 8)
    );
    let filepath = output_dir.join(&filename);

    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
    );

    if full_html.len() > max_bytes {
        return Err(ToolError::InvalidParameter {
            name: "html",
            reason: format!(
                "Full canvas document too large: {} bytes (max {})",
                full_html.len(),
                max_bytes
            ),
        });
    }

    tokio::fs::write(&filepath, &full_html)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to save canvas: {e}"),
            source: Some(Box::new(e)),
        })?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": raw_title,
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

    /// Regression (#6441 follow-up): the configured `max_html_bytes` and
    /// `allowed_tags` must actually take effect. A below-default cap rejects an
    /// over-cap document, a restrictive allowlist strips otherwise-allowed
    /// tags, and an empty allowlist falls back to the built-in `ALLOWED_TAGS`.
    #[test]
    fn sanitize_honors_custom_max_bytes_and_allowed_tags() {
        let big = format!("<p>{}</p>", "x".repeat(1000));
        assert!(
            sanitize_canvas_html_with_tags(&big, 64, &[]).is_err(),
            "a below-default max_html_bytes must reject an over-cap document"
        );

        let html = "<p>hi</p><table><tr><td>x</td></tr></table>";
        let restricted =
            sanitize_canvas_html_with_tags(html, 512 * 1024, &["p".to_string()]).unwrap();
        assert!(restricted.contains("<p>"), "listed tag kept: {restricted}");
        assert!(
            !restricted.contains("<table"),
            "unlisted tag stripped: {restricted}"
        );

        let builtin = sanitize_canvas_html_with_tags(html, 512 * 1024, &[]).unwrap();
        assert!(
            builtin.contains("<table"),
            "empty allowed_tags falls back to the built-in allowlist: {builtin}"
        );
    }

    /// Regression (#6441 follow-up): the scoped `CANVAS_MAX_BYTES` task-local
    /// must reach `canvas_present` — previously it was never `.scope()`d, so
    /// the tool always used the hardcoded 512 KiB fallback and the operator's
    /// `[canvas] max_html_bytes` was silently ignored.
    #[tokio::test]
    async fn canvas_present_reads_scoped_config_task_locals() {
        let input = serde_json::json!({ "html": format!("<p>{}</p>", "x".repeat(2000)) });
        let fut = tool_canvas_present(&input, None);
        let res = CANVAS_MAX_BYTES
            .scope(
                64usize,
                CANVAS_ALLOWED_TAGS.scope(std::sync::Arc::new(Vec::<String>::new()), fut),
            )
            .await;
        match res {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "html");
                assert!(reason.contains("too large"), "reason: {reason}");
            }
            other => panic!("expected InvalidParameter(too large), got {other:?}"),
        }
    }
}
