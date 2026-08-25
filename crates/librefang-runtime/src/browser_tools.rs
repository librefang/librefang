//! Browser tool dispatcher (browser_navigate / click / type / etc.).
//!
//! Split out of `browser.rs` so SSRF and content-wrapping wiring
//! (`crate::web_fetch::check_ssrf`, `crate::web_content::wrap_external_content`)
//! stays next to the rest of the tool dispatchers in `tool_runner/` rather
//! than mixed with the CDP/WebSocket transport in `browser.rs`.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576): missing params map to `MissingParameter`, a blocked/invalid URL to
//! `InvalidParameter`, and CDP transport / command failures to `Upstream` via
//! `upstream_msg`. The dispatch boundary previously narrowed every error to
//! `upstream_msg`; the typed variants now flow through `tool_result_from_typed`.

use crate::browser::{BrowserCommand, BrowserManager};
use crate::tool_runner::ToolError;

/// The same body, inside the untrusted-content boundary, for a tool result.
///
/// The table belongs inside the fence: every URL in it comes from the page and is exactly as attacker-controlled as the prose it was lifted out of.
fn render_page(source_url: &str, data: &serde_json::Value) -> String {
    crate::web_content::wrap_external_content(source_url, &crate::browser::render_page_body(data))
}

pub async fn tool_browser_navigate(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let url = input["url"]
        .as_str()
        .ok_or(ToolError::MissingParameter("url"))?;
    // Browser navigation goes through CDP/WebSocket, not reqwest, so DNS-pinning
    // the resolved address is not possible here. We still call check_ssrf to
    // validate the URL scheme and reject IPs that resolve to internal/loopback
    // ranges; the SsrfResolution result is intentionally discarded.
    let _ = crate::web_fetch::check_ssrf(url, &[]).map_err(|e| ToolError::InvalidParameter {
        name: "url",
        reason: e,
    })?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Navigate {
                url: url.to_string(),
            },
        )
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Navigate failed".to_string()),
        ));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let page_url = data["url"].as_str().unwrap_or(url);
    let wrapped = render_page(page_url, &data);

    Ok(format!(
        "Navigated to: {page_url}\nTitle: {title}\n\n{wrapped}"
    ))
}

/// browser_click: Click an element by CSS selector or visible text.
pub async fn tool_browser_click(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let selector = input["selector"]
        .as_str()
        .ok_or(ToolError::MissingParameter("selector"))?;
    if selector.trim().is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "selector",
            reason: "must not be empty".to_string(),
        });
    }

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Click {
                selector: selector.to_string(),
            },
        )
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Click failed".to_string()),
        ));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    Ok(format!("Clicked: {selector}\nPage: {title}\nURL: {url}"))
}

/// browser_type: Type text into an input field.
pub async fn tool_browser_type(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let selector = input["selector"]
        .as_str()
        .ok_or(ToolError::MissingParameter("selector"))?;
    let text = input["text"]
        .as_str()
        .ok_or(ToolError::MissingParameter("text"))?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Type {
                selector: selector.to_string(),
                text: text.to_string(),
            },
        )
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Type failed".to_string()),
        ));
    }
    Ok(format!("Typed into {selector}: {text}"))
}

/// Decode and persist browser-provided screenshot bytes when present.
async fn persist_screenshot(
    image_base64: Option<&serde_json::Value>,
    upload_dir: &std::path::Path,
) -> Result<Option<String>, ToolError> {
    let image_base64 = match image_base64 {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::String(value)) if value.is_empty() => return Ok(None),
        Some(serde_json::Value::String(value)) => value,
        Some(_) => {
            return Err(ToolError::upstream_msg(
                "Invalid screenshot data: image_base64 must be a string",
            ));
        }
    };

    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(image_base64)
        .map_err(|error| ToolError::upstream_msg(format!("Invalid screenshot data: {error}")))?;
    crate::uploaded_file::save_shared_upload(upload_dir, &decoded, "image/png", "screenshot.png")
        .await
        .map(Some)
        .map_err(ToolError::upstream_msg)
}

/// browser_screenshot: Take a screenshot of the current page.
pub async fn tool_browser_screenshot(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
    upload_dir: &std::path::Path,
) -> Result<String, ToolError> {
    let resp = mgr
        .send_command(agent_id, BrowserCommand::Screenshot)
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error
                .unwrap_or_else(|| "Screenshot failed".to_string()),
        ));
    }

    let data = resp.data.unwrap_or_default();
    let url = data["url"].as_str().unwrap_or("");

    let image_urls: Vec<String> = persist_screenshot(data.get("image_base64"), upload_dir)
        .await?
        .into_iter()
        .collect();

    Ok(serde_json::json!({
        "screenshot": true,
        "url": url,
        "image_urls": image_urls,
    })
    .to_string())
}

/// browser_read_page: Read current page content as markdown.
pub async fn tool_browser_read_page(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let resp = mgr
        .send_command(agent_id, BrowserCommand::ReadPage)
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "ReadPage failed".to_string()),
        ));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    let wrapped = render_page(url, &data);

    Ok(format!("Page: {title}\nURL: {url}\n\n{wrapped}"))
}

/// browser_close: Close the browser session.
pub async fn tool_browser_close(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    mgr.close_session(agent_id).await;
    Ok("Browser session closed.".to_string())
}

/// browser_scroll: Scroll the page in a direction.
pub async fn tool_browser_scroll(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let direction = input["direction"].as_str().unwrap_or("down").to_string();
    let amount = input["amount"].as_i64().unwrap_or(600) as i32;

    let resp = mgr
        .send_command(agent_id, BrowserCommand::Scroll { direction, amount })
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Scroll failed".to_string()),
        ));
    }
    let data = resp.data.unwrap_or_default();
    Ok(format!(
        "Scrolled. Position: scrollX={}, scrollY={}",
        data["scrollX"], data["scrollY"]
    ))
}

/// browser_wait: Wait for a CSS selector to appear on the page.
pub async fn tool_browser_wait(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let selector = input["selector"]
        .as_str()
        .ok_or(ToolError::MissingParameter("selector"))?;
    let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(5000);

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Wait {
                selector: selector.to_string(),
                timeout_ms,
            },
        )
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Wait timed out".to_string()),
        ));
    }
    Ok(format!("Element found: {selector}"))
}

/// browser_run_js: Run JavaScript on the current page.
pub async fn tool_browser_run_js(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let expression = input["expression"]
        .as_str()
        .ok_or(ToolError::MissingParameter("expression"))?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::RunJs {
                expression: expression.to_string(),
            },
        )
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error
                .unwrap_or_else(|| "JS execution failed".to_string()),
        ));
    }
    let data = resp.data.unwrap_or_default();
    Ok(serde_json::to_string_pretty(&data["result"]).unwrap_or_else(|_| "null".to_string()))
}

/// browser_back: Go back in browser history.
pub async fn tool_browser_back(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, ToolError> {
    let resp = mgr
        .send_command(agent_id, BrowserCommand::Back)
        .await
        .map_err(ToolError::upstream_msg)?;
    if !resp.success {
        return Err(ToolError::upstream_msg(
            resp.error.unwrap_or_else(|| "Back failed".to_string()),
        ));
    }
    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    Ok(format!("Went back.\nPage: {title}\nURL: {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_with_links() -> serde_json::Value {
        serde_json::json!({
            "title": "Hacker News",
            "url": "https://news.ycombinator.com/",
            "content": "Ask HN\u{27e8}1\u{27e9}\nRust\u{27e8}2\u{27e9}",
            "links": [
                {"id": 1, "url": "/ask"},
                {"id": 2, "url": "https://rust-lang.org/"},
            ],
            "links_base": "https://news.ycombinator.com",
        })
    }

    #[tokio::test]
    async fn empty_click_selector_is_invalid_input_without_opening_a_session() {
        let manager = BrowserManager::new(Default::default());
        let error = tool_browser_click(
            &serde_json::json!({"selector": "   "}),
            &manager,
            "test-agent",
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            ToolError::InvalidParameter {
                name: "selector",
                ..
            }
        ));
    }

    /// The link table is content lifted out of the page, so it belongs inside the untrusted-content boundary rather than in the trusted preamble around it.
    #[test]
    fn test_link_table_stays_inside_the_untrusted_boundary() {
        let out = render_page("https://news.ycombinator.com/", &page_with_links());
        let boundary = crate::web_content::content_boundary("https://news.ycombinator.com/");
        let close = format!("<<</{boundary}>>>");
        let table_at = out
            .find("\u{27e8}1\u{27e9} /ask")
            .expect("table is rendered");
        let close_at = out.find(&close).expect("boundary is closed");
        assert!(
            table_at < close_at,
            "a URL taken from the page must not sit outside the untrusted-content fence"
        );
    }

    #[test]
    fn test_link_table_lists_every_marker_the_prose_carries() {
        let out = render_page("https://news.ycombinator.com/", &page_with_links());
        assert!(out.contains("\u{27e8}1\u{27e9} /ask"));
        // Cross-origin entries stay absolute; same-origin ones are a path.
        assert!(out.contains("\u{27e8}2\u{27e9} https://rust-lang.org/"));
        assert!(out.contains("relative to https://news.ycombinator.com"));
    }

    /// A page with no links renders exactly as it did before the table existed.
    #[test]
    fn test_page_without_links_is_unchanged() {
        let data = serde_json::json!({"content": "Just prose.", "links": [], "links_base": ""});
        assert_eq!(
            render_page("https://example.com/", &data),
            crate::web_content::wrap_external_content("https://example.com/", "Just prose.")
        );
    }

    /// An extraction that predates the table — or any caller that only sets `content` — must still render, since `links` is an additive field.
    #[test]
    fn test_missing_links_field_falls_back_to_content_only() {
        let data = serde_json::json!({"content": "Just prose."});
        assert_eq!(
            render_page("https://example.com/", &data),
            crate::web_content::wrap_external_content("https://example.com/", "Just prose.")
        );
    }

    #[tokio::test]
    async fn screenshot_rejects_invalid_base64() {
        let dir = tempfile::tempdir().unwrap();
        let image = serde_json::json!("not base64!");
        let error = persist_screenshot(Some(&image), dir.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Invalid screenshot data"));
    }

    #[tokio::test]
    async fn screenshot_rejects_non_string_base64() {
        let dir = tempfile::tempdir().unwrap();
        let image = serde_json::json!({ "bytes": "cG5n" });
        let error = persist_screenshot(Some(&image), dir.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("image_base64 must be a string"));
    }

    #[tokio::test]
    async fn screenshot_surfaces_shared_upload_failure() {
        let dir = tempfile::tempdir().unwrap();
        let upload_path = dir.path().join("not-a-directory");
        tokio::fs::write(&upload_path, b"occupied").await.unwrap();

        let image = serde_json::json!("cG5n");
        let error = persist_screenshot(Some(&image), &upload_path)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to create upload directory"));
    }
}
