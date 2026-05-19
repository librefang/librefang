//! Legacy `web_fetch` / `web_search` fallbacks used when the agent's
//! `WebToolsContext` is unavailable (e.g. tests, REST/MCP bridges that
//! don't thread one through).

use crate::web_search::parse_ddg_results;
use tracing::debug;

/// Legacy web fetch (no SSRF protection, no readability). Used when WebToolsContext is unavailable.
pub(super) async fn tool_web_fetch_legacy(
    input: &serde_json::Value,
    spill_threshold: u64,
    max_artifact_bytes: u64,
) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    // Reject responses larger than 10MB to prevent memory exhaustion
    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    // Artifact spill: if the body exceeds the configured threshold, write it
    // to the artifact store and return a compact stub with a handle.  On write
    // failure (including per-artifact size cap exceeded), fall through to the
    // existing byte-cap truncation so callers always get a usable (if partial)
    // response.
    let body_bytes = body.as_bytes();
    if let Some(stub) = crate::artifact_store::maybe_spill(
        "web_fetch",
        body_bytes,
        spill_threshold,
        max_artifact_bytes,
        &crate::artifact_store::default_artifact_storage_dir(),
    ) {
        return Ok(format!("HTTP {status}\n\n{stub}"));
    }

    let max_len = 50_000;
    let truncated = if body.len() > max_len {
        format!(
            "{}... [truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&body, max_len),
            body.len()
        )
    } else {
        body
    };
    Ok(format!("HTTP {status}\n\n{truncated}"))
}

/// Legacy web search via DuckDuckGo HTML only. Used when WebToolsContext is unavailable.
pub(super) async fn tool_web_search_legacy(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

    debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; LibreFangAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))?;

    // Parse DuckDuckGo HTML results
    let results = parse_ddg_results(&body, max_results);

    if results.is_empty() {
        return Ok(format!("No results found for '{query}'."));
    }

    let mut output = format!("Search results for '{query}':\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            title,
            url,
            snippet
        ));
    }

    Ok(output)
}
