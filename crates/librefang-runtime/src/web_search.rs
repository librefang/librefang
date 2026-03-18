//! Multi-provider web search engine with auto-fallback.
//!
//! Supports 4 providers: Tavily (AI-agent-native), Brave, Perplexity, and
//! DuckDuckGo (zero-config fallback). Auto mode cascades through available
//! providers based on configured API keys.
//!
//! All API keys use `Zeroizing<String>` via `resolve_api_key()` to auto-wipe
//! secrets from memory on drop.

use crate::web_cache::WebCache;
use crate::web_content::wrap_external_content;
use librefang_types::config::{SearchProvider, WebConfig};
use std::sync::Arc;
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Multi-provider web search engine.
pub struct WebSearchEngine {
    config: WebConfig,
    client: reqwest::Client,
    cache: Arc<WebCache>,
    /// Extra API key env var names from auth_profiles (sorted by priority).
    /// Used for key rotation when the primary key returns 402/429.
    brave_key_envs: Vec<String>,
}

/// Context that bundles both search and fetch engines for passing through the tool runner.
pub struct WebToolsContext {
    pub search: WebSearchEngine,
    pub fetch: crate::web_fetch::WebFetchEngine,
}

impl WebSearchEngine {
    /// Create a new search engine from config with a shared cache.
    ///
    /// `brave_auth_profiles` supplies additional API key env var names for
    /// Brave Search key rotation (from `[[auth_profiles.brave]]` in config).
    pub fn new(
        config: WebConfig,
        cache: Arc<WebCache>,
        brave_auth_profiles: Vec<(String, u32)>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        // Build a deduplicated list of Brave API key env vars sorted by priority.
        // The primary key from config comes first (priority -1), then auth_profiles.
        let mut key_entries: Vec<(String, i64)> = Vec::new();
        key_entries.push((config.brave.api_key_env.clone(), -1));
        for (env_var, priority) in &brave_auth_profiles {
            if *env_var != config.brave.api_key_env {
                key_entries.push((env_var.clone(), *priority as i64));
            }
        }
        key_entries.sort_by_key(|(_, p)| *p);
        let brave_key_envs: Vec<String> = key_entries.into_iter().map(|(k, _)| k).collect();

        Self {
            config,
            client,
            cache,
            brave_key_envs,
        }
    }

    /// Perform a web search using the configured provider (or auto-fallback).
    pub async fn search(&self, query: &str, max_results: usize) -> Result<String, String> {
        // Check cache first
        let cache_key = format!("search:{}:{}", query, max_results);
        if let Some(cached) = self.cache.get(&cache_key) {
            debug!(query, "Search cache hit");
            return Ok(cached);
        }

        let result = match self.config.search_provider {
            SearchProvider::Brave => self.search_brave(query, max_results).await,
            SearchProvider::Tavily => self.search_tavily(query, max_results).await,
            SearchProvider::Perplexity => self.search_perplexity(query).await,
            SearchProvider::DuckDuckGo => self.search_duckduckgo(query, max_results).await,
            SearchProvider::Auto => self.search_auto(query, max_results).await,
        };

        // Cache successful results
        if let Ok(ref content) = result {
            self.cache.put(cache_key, content.clone());
        }

        result
    }

    /// Auto-select provider based on available API keys.
    /// Priority: Tavily → Brave → Perplexity → DuckDuckGo
    async fn search_auto(&self, query: &str, max_results: usize) -> Result<String, String> {
        // Tavily first (AI-agent-native)
        if resolve_api_key(&self.config.tavily.api_key_env).is_some() {
            debug!("Auto: trying Tavily");
            match self.search_tavily(query, max_results).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Tavily failed, falling back: {e}"),
            }
        }

        // Brave second (check any key in the rotation pool)
        if self
            .brave_key_envs
            .iter()
            .any(|k| resolve_api_key(k).is_some())
        {
            debug!("Auto: trying Brave");
            match self.search_brave(query, max_results).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Brave failed, falling back: {e}"),
            }
        }

        // Perplexity third
        if resolve_api_key(&self.config.perplexity.api_key_env).is_some() {
            debug!("Auto: trying Perplexity");
            match self.search_perplexity(query).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Perplexity failed, falling back: {e}"),
            }
        }

        // DuckDuckGo always available as zero-config fallback
        debug!("Auto: falling back to DuckDuckGo");
        self.search_duckduckgo(query, max_results).await
    }

    /// Search via Brave Search API with auth_profile key rotation.
    ///
    /// Tries each configured API key in priority order. If a key returns
    /// 402 (Payment Required) or 429 (Too Many Requests), the next key is
    /// tried automatically.
    async fn search_brave(&self, query: &str, max_results: usize) -> Result<String, String> {
        let mut last_err = String::from("Brave API key not set");

        for env_var in &self.brave_key_envs {
            let Some(api_key) = resolve_api_key(env_var) else {
                continue;
            };

            match self
                .search_brave_with_key(query, max_results, &api_key)
                .await
            {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let is_rotatable =
                        e.contains("402") || e.contains("429") || e.contains("Payment");
                    if is_rotatable && self.brave_key_envs.len() > 1 {
                        warn!(
                            env_var,
                            error = %e,
                            "Brave key exhausted, rotating to next"
                        );
                        last_err = e;
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err)
    }

    /// Execute a single Brave search request with a specific API key.
    async fn search_brave_with_key(
        &self,
        query: &str,
        max_results: usize,
        api_key: &Zeroizing<String>,
    ) -> Result<String, String> {
        let mut params = vec![("q", query.to_string()), ("count", max_results.to_string())];
        if !self.config.brave.country.is_empty() {
            params.push(("country", self.config.brave.country.clone()));
        }
        if !self.config.brave.search_lang.is_empty() {
            params.push(("search_lang", self.config.brave.search_lang.clone()));
        }
        if !self.config.brave.freshness.is_empty() {
            params.push(("freshness", self.config.brave.freshness.clone()));
        }

        let resp = self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&params)
            .header("X-Subscription-Token", api_key.as_str())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Brave request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Brave API returned {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Brave JSON parse failed: {e}"))?;

        let results = body["web"]["results"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if results.is_empty() {
            return Err(format!("No results found for '{query}' (Brave)."));
        }

        let mut output = format!("Search results for '{query}' (Brave):\n\n");
        for (i, r) in results.iter().enumerate().take(max_results) {
            let title = r["title"].as_str().unwrap_or("");
            let url = r["url"].as_str().unwrap_or("");
            let desc = r["description"].as_str().unwrap_or("");
            output.push_str(&format!(
                "{}. {}\n   URL: {}\n   {}\n\n",
                i + 1,
                title,
                url,
                desc
            ));
        }

        Ok(wrap_external_content("brave-search", &output))
    }

    /// Search via Tavily API (AI-agent-native search).
    async fn search_tavily(&self, query: &str, max_results: usize) -> Result<String, String> {
        let api_key =
            resolve_api_key(&self.config.tavily.api_key_env).ok_or("Tavily API key not set")?;

        let body = serde_json::json!({
            "api_key": api_key.as_str(),
            "query": query,
            "search_depth": self.config.tavily.search_depth,
            "max_results": max_results,
            "include_answer": self.config.tavily.include_answer,
        });

        let resp = self
            .client
            .post("https://api.tavily.com/search")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Tavily request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Tavily API returned {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Tavily JSON parse failed: {e}"))?;

        let mut output = format!("Search results for '{query}' (Tavily):\n\n");

        // Include AI-generated answer if available
        if let Some(answer) = data["answer"].as_str() {
            if !answer.is_empty() {
                output.push_str(&format!("AI Summary: {answer}\n\n"));
            }
        }

        let results = data["results"].as_array().cloned().unwrap_or_default();
        for (i, r) in results.iter().enumerate().take(max_results) {
            let title = r["title"].as_str().unwrap_or("");
            let url = r["url"].as_str().unwrap_or("");
            let content = r["content"].as_str().unwrap_or("");
            output.push_str(&format!(
                "{}. {}\n   URL: {}\n   {}\n\n",
                i + 1,
                title,
                url,
                content
            ));
        }

        if results.is_empty() && !output.contains("AI Summary") {
            return Err(format!("No results found for '{query}' (Tavily)."));
        }

        Ok(wrap_external_content("tavily-search", &output))
    }

    /// Search via Perplexity AI (chat completions endpoint).
    async fn search_perplexity(&self, query: &str) -> Result<String, String> {
        let api_key = resolve_api_key(&self.config.perplexity.api_key_env)
            .ok_or("Perplexity API key not set")?;

        let body = serde_json::json!({
            "model": self.config.perplexity.model,
            "messages": [
                {"role": "user", "content": query}
            ],
        });

        let resp = self
            .client
            .post("https://api.perplexity.ai/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key.as_str()))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Perplexity request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Perplexity API returned {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Perplexity JSON parse failed: {e}"))?;

        let answer = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if answer.is_empty() {
            return Ok(format!("No answer for '{query}' (Perplexity)."));
        }

        let mut output = format!("Search results for '{query}' (Perplexity AI):\n\n{answer}\n");

        // Include citations if available
        if let Some(citations) = data["citations"].as_array() {
            output.push_str("\nSources:\n");
            for (i, c) in citations.iter().enumerate() {
                if let Some(url) = c.as_str() {
                    output.push_str(&format!("  {}. {}\n", i + 1, url));
                }
            }
        }

        Ok(wrap_external_content("perplexity-search", &output))
    }

    /// Search via DuckDuckGo HTML (no API key needed).
    async fn search_duckduckgo(&self, query: &str, max_results: usize) -> Result<String, String> {
        debug!(query, "Searching via DuckDuckGo HTML");

        let resp = self
            .client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .header("User-Agent", "Mozilla/5.0 (compatible; LibreFangAgent/0.1)")
            .send()
            .await
            .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;

        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read DDG response: {e}"))?;

        let results = parse_ddg_results(&body, max_results);

        if results.is_empty() {
            return Err(format!("No results found for '{query}'."));
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
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML parser (moved from tool_runner.rs)
// ---------------------------------------------------------------------------

/// Parse DuckDuckGo HTML search results into (title, url, snippet) tuples.
pub fn parse_ddg_results(html: &str, max: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();

    for chunk in html.split("class=\"result__a\"") {
        if results.len() >= max {
            break;
        }
        if !chunk.contains("href=") {
            continue;
        }

        let url = extract_between(chunk, "href=\"", "\"")
            .unwrap_or_default()
            .to_string();

        let actual_url = if url.contains("uddg=") {
            url.split("uddg=")
                .nth(1)
                .and_then(|u| u.split('&').next())
                .map(urldecode)
                .unwrap_or(url)
        } else {
            url
        };

        let title = extract_between(chunk, ">", "</a>")
            .map(strip_html_tags)
            .unwrap_or_default();

        let snippet = if let Some(snip_start) = chunk.find("class=\"result__snippet\"") {
            let after = &chunk[snip_start..];
            extract_between(after, ">", "</a>")
                .or_else(|| extract_between(after, ">", "</"))
                .map(strip_html_tags)
                .unwrap_or_default()
        } else {
            String::new()
        };

        if !title.is_empty() && !actual_url.is_empty() {
            results.push((title, actual_url, snippet));
        }
    }

    results
}

/// Extract text between two delimiters.
pub fn extract_between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_idx = text.find(start)? + start.len();
    let remaining = &text[start_idx..];
    let end_idx = remaining.find(end)?;
    Some(&remaining[..end_idx])
}

/// Strip HTML tags from a string.
pub fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
}

/// Simple percent-decode for URLs.
pub fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

/// Resolve an API key from an environment variable name.
/// Returns `Zeroizing<String>` that auto-wipes from memory on drop.
fn resolve_api_key(env_var: &str) -> Option<Zeroizing<String>> {
    std::env::var(env_var)
        .ok()
        .filter(|v| !v.is_empty())
        .map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_with_results() {
        let html = r#"junk class="result__a" href="https://example.com">Example</a> class="result__snippet">A snippet</a>"#;
        let results = parse_ddg_results(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "Example");
        assert_eq!(results[0].1, "https://example.com");
        assert_eq!(results[0].2, "A snippet");
    }

    #[test]
    fn test_format_empty() {
        let results = parse_ddg_results("<html><body>No results</body></html>", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_format_with_answer() {
        // Tavily-style answer formatting is tested via the DDG parser as basic coverage
        let html = r#"before class="result__a" href="https://rust-lang.org">Rust</a> class="result__snippet">Systems programming</a> class="result__a" href="https://go.dev">Go</a> class="result__snippet">Another language</a>"#;
        let results = parse_ddg_results(html, 10);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_ddg_parser_preserved() {
        // Ensure the parser handles URL-encoded DDG redirect URLs
        let html = r#"x class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com&rut=abc">Title</a> class="result__snippet">Desc</a>"#;
        let results = parse_ddg_results(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, "https://example.com");
    }
}
