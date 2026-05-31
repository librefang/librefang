//! Legacy `web_fetch` / `web_search` fallbacks used when the agent's
//! `WebToolsContext` is unavailable (e.g. tests, REST/MCP bridges that
//! don't thread one through).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Missing params -> `MissingParameter`; the HTTP/transport
//! (`reqwest::Error`) failures -> `ToolError::Upstream` via `fetch_err`, which
//! keeps a stage-identifying prefix on the message AND the typed error on the
//! `source()` chain; the 10 MB response cap -> `upstream_msg`. SSRF check
//! failures -> `InvalidParameter` (the URL is the bad input).

use super::error::{ToolError, ToolResult};
use crate::web_search::parse_ddg_results;
use tracing::debug;

const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 20;
const LEGACY_UA: &str = concat!(
    "Mozilla/5.0 (compatible; librefang/",
    env!("CARGO_PKG_VERSION"),
    ")"
);

/// Wrap a transport failure in `ToolError::Upstream`, keeping a stage prefix on
/// the rendered message and the typed error on the `source()` chain.
fn fetch_err<E>(prefix: &str, e: E) -> ToolError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ToolError::Upstream {
        message: format!("{prefix}: {e}"),
        source: Some(Box::new(e)),
    }
}

/// Stream response body with a hard cap at [`MAX_BODY_BYTES`].
///
/// For non-2xx responses, returns `Ok` with the body (preserving legacy
/// behaviour where agents could read 403/404/500 bodies). Status is
/// prepended to the returned string by the caller.
async fn read_body_limited(resp: reqwest::Response) -> ToolResult {
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_BODY_BYTES {
            return Err(ToolError::upstream_msg(format!(
                "Response too large: {len} bytes (max 10MB)"
            )));
        }
    }

    let mut total: usize = 0;
    let mut chunks: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| fetch_err("Failed to read response chunk", e))?;
        total += chunk.len();
        if total > MAX_BODY_BYTES {
            return Err(ToolError::upstream_msg(format!(
                "Response body exceeded {MAX_BODY_BYTES} bytes during streaming"
            )));
        }
        chunks.extend_from_slice(&chunk);
    }

    // Non-2xx: return body as Ok (legacy behaviour — agents can read
    // 403/404/500 response bodies). Caller prepends "HTTP {status}\n\n".
    // This avoids the breaking change where every non-2xx became an Err
    // with only a 500-char preview.
    String::from_utf8(chunks)
        .map_err(|e| ToolError::upstream_msg(format!("Response body is not valid UTF-8: {e}")))
}

/// Legacy web fetch (with SSRF protection, no readability). Used when
/// WebToolsContext is unavailable.
pub(super) async fn tool_web_fetch_legacy(
    input: &serde_json::Value,
    spill_threshold: u64,
    max_artifact_bytes: u64,
) -> ToolResult {
    let url = input["url"]
        .as_str()
        .ok_or(ToolError::MissingParameter("url"))?;

    // SSRF protection — reject private/cloud-metadata IPs, userinfo, etc.
    crate::web_fetch::check_ssrf(url, &[]).map_err(|e| ToolError::InvalidParameter {
        name: "url",
        reason: e,
    })?;

    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| fetch_err("Failed to create HTTP client", e))?;
    let resp = client
        .get(url)
        .header("User-Agent", LEGACY_UA)
        .send()
        .await
        .map_err(|e| fetch_err("HTTP request failed", e))?;
    let status = resp.status();
    let body = read_body_limited(resp).await?;

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

/// Legacy web search via DuckDuckGo HTML only. Used when WebToolsContext is
/// unavailable.
pub(super) async fn tool_web_search_legacy(input: &serde_json::Value) -> ToolResult {
    let query = input["query"]
        .as_str()
        .ok_or(ToolError::MissingParameter("query"))?;
    let max_results = (input["max_results"].as_u64().unwrap_or(5) as usize).min(MAX_SEARCH_RESULTS);

    debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| fetch_err("Failed to create HTTP client", e))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", LEGACY_UA)
        .send()
        .await
        .map_err(|e| fetch_err("Search request failed", e))?;

    let body = read_body_limited(resp).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_fetch_legacy_missing_url_is_missing_parameter() {
        let r = tool_web_fetch_legacy(&serde_json::json!({}), 0, 0).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("url"))));
    }

    #[tokio::test]
    async fn web_search_legacy_missing_query_is_missing_parameter() {
        let r = tool_web_search_legacy(&serde_json::json!({})).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("query"))));
    }
}
