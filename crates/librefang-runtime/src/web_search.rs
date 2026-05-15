//! Multi-provider web search engine with auto-fallback.
//!
//! Supports 5 providers: Tavily (AI-agent-native), Brave, Jina, Perplexity,
//! and DuckDuckGo (zero-config fallback). Auto mode cascades through
//! available providers based on configured API keys.
//!
//! All API keys use `Zeroizing<String>` via `resolve_api_key()` to auto-wipe
//! secrets from memory on drop.

use crate::web_cache::WebCache;
use crate::web_content::wrap_external_content;
use librefang_types::config::{SearchProvider, WebConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// TTL for the per-engine SearXNG `/config` categories cache.
///
/// `/config` rarely changes between releases of the upstream instance, so a
/// short-lived in-memory cache eliminates the second HTTP round-trip per
/// search call without making admin reconfigurations sticky for long.
const SEARXNG_CATEGORIES_TTL: Duration = Duration::from_secs(300);

/// Multi-provider web search engine.
pub struct WebSearchEngine {
    config: WebConfig,
    client: reqwest::Client,
    cache: Arc<WebCache>,
    /// Extra API key env var names from auth_profiles (sorted by priority).
    /// Used for key rotation when the primary key returns 402/429.
    brave_key_envs: Vec<String>,
    /// Cached `/config.categories` from the configured SearXNG instance.
    ///
    /// `None` until the first successful fetch. Refreshed when the cached
    /// entry is older than [`SEARXNG_CATEGORIES_TTL`].
    searxng_categories_cache: RwLock<Option<(Vec<String>, Instant)>>,
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
        let client = crate::http_client::proxied_client_builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("HTTP client build");

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
            searxng_categories_cache: RwLock::new(None),
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
            SearchProvider::Jina => self.search_jina(query, max_results).await,
            SearchProvider::Perplexity => self.search_perplexity(query).await,
            SearchProvider::DuckDuckGo => self.search_duckduckgo(query, max_results).await,
            SearchProvider::Searxng => self.search_searxng(query, max_results, None, 1).await,
            SearchProvider::Auto => self.search_auto(query, max_results).await,
        };

        // Cache successful results
        if let Ok(ref content) = result {
            self.cache.put(cache_key, content.clone());
        }

        result
    }

    /// Auto-select provider based on available API keys.
    /// Priority: Tavily → Brave → Jina → Perplexity → Searxng → DuckDuckGo
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

        // Jina third
        if resolve_api_key(&self.config.jina.api_key_env).is_some() {
            debug!("Auto: trying Jina");
            match self.search_jina(query, max_results).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Jina failed, falling back: {e}"),
            }
        }

        // Perplexity fourth
        if resolve_api_key(&self.config.perplexity.api_key_env).is_some() {
            debug!("Auto: trying Perplexity");
            match self.search_perplexity(query).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Perplexity failed, falling back: {e}"),
            }
        }

        // Searxng fifth (self-hosted, requires only a URL — no API key)
        if !self.config.searxng.url.is_empty() {
            debug!("Auto: trying Searxng");
            match self.search_searxng(query, max_results, None, 1).await {
                Ok(result) => return Ok(result),
                Err(e) => warn!("Searxng failed, falling back: {e}"),
            }
        }

        // DuckDuckGo as zero-config fallback — but it often gets captcha-blocked,
        // so treat it as best-effort and surface the last upstream error if it also
        // fails, giving the user a more actionable message.
        debug!("Auto: falling back to DuckDuckGo");
        match self.search_duckduckgo(query, max_results).await {
            Ok(result) if !result.trim().is_empty() => Ok(result),
            Ok(_) => Err("All search providers failed; DuckDuckGo returned empty results (likely captcha-blocked). Configure a Brave, Tavily, Jina, Perplexity, or SearXNG provider for reliable search.".to_string()),
            Err(e) => Err(format!("All search providers exhausted. Last error (DuckDuckGo): {e}")),
        }
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

    /// Search via Jina AI Search API (with single retry on transient errors).
    async fn search_jina(&self, query: &str, max_results: usize) -> Result<String, String> {
        let api_key =
            resolve_api_key(&self.config.jina.api_key_env).ok_or("Jina API key not set")?;

        let endpoint = if self.config.jina.use_eu_endpoint {
            "https://eu.s.jina.ai/"
        } else {
            "https://s.jina.ai/"
        };

        let mut body = serde_json::json!({
            "q": query,
            "num": max_results,
        });

        if !self.config.jina.country.is_empty() {
            body["gl"] = serde_json::Value::String(self.config.jina.country.clone());
        }
        if !self.config.jina.language.is_empty() {
            body["hl"] = serde_json::Value::String(self.config.jina.language.clone());
        }

        let mut attempts = 0u32;
        loop {
            attempts += 1;

            let mut req = self
                .client
                .post(endpoint)
                .header("Authorization", format!("Bearer {}", api_key.as_str()))
                .header("Accept", "application/json");

            if self.config.jina.no_cache {
                req = req.header("X-No-Cache", "true");
            }

            let resp_result = req.json(&body).send().await;

            match resp_result {
                Ok(resp) if resp.status().is_success() => {
                    let data: serde_json::Value = resp
                        .json()
                        .await
                        .map_err(|e| format!("Jina JSON parse failed: {e}"))?;

                    let results = data["data"].as_array().cloned().unwrap_or_default();

                    if results.is_empty() {
                        return Err(format!("No results found for '{query}' (Jina)."));
                    }

                    let mut output = format!("Search results for '{query}' (Jina):\n\n");
                    for (i, r) in results.iter().enumerate().take(max_results) {
                        let title = r["title"].as_str().unwrap_or("");
                        let url = r["url"].as_str().unwrap_or("");
                        let content = r["content"]
                            .as_str()
                            .or_else(|| r["description"].as_str())
                            .unwrap_or("");
                        output.push_str(&format!(
                            "{}. {}\n   URL: {}\n   {}\n\n",
                            i + 1,
                            title,
                            url,
                            content
                        ));
                    }

                    return Ok(wrap_external_content("jina-search", &output));
                }
                Ok(resp) => {
                    let status = resp.status();
                    // Only retry on transient errors: 429 (rate limit) or 5xx (server errors)
                    let is_transient = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error();
                    if is_transient && attempts < 2 {
                        warn!("Jina returned {status}, retrying...");
                        continue;
                    }
                    return Err(format!("Jina API returned {status}"));
                }
                Err(e) => {
                    if attempts < 2 {
                        warn!("Jina request failed: {e}, retrying...");
                        continue;
                    }
                    return Err(format!("Jina request failed: {e}"));
                }
            }
        }
    }

    /// Search via DuckDuckGo HTML (no API key needed).
    async fn search_duckduckgo(&self, query: &str, max_results: usize) -> Result<String, String> {
        debug!(query, "Searching via DuckDuckGo HTML");

        let resp = self
            .client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .header("User-Agent", "Mozilla/5.0 (compatible; BossFangAgent/0.1)")
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

    /// Search via a self-hosted SearXNG instance.
    ///
    /// SearXNG public instances reject the upstream `limit` query param, so we
    /// fetch a full page and truncate client-side. `category` defaults to
    /// "general" and is validated against the instance's `/config` endpoint
    /// (mismatches return an error so the LLM can pick a valid category on
    /// retry). `page` is 1-based and forwarded as the `pageno` SearXNG param.
    async fn search_searxng(
        &self,
        query: &str,
        max_results: usize,
        category: Option<&str>,
        page: u32,
    ) -> Result<String, String> {
        if self.config.searxng.url.is_empty() {
            return Err("SearXNG URL is not configured".to_string());
        }

        // SearXNG treats `pageno=0` as "no results" silently — guard up-front
        // so callers (LLM tool args, internal misuse) get a clear error
        // instead of an opaque empty response.
        if page == 0 {
            return Err("SearXNG pageno must be >= 1 (pages are 1-indexed)".to_string());
        }

        let category = category.unwrap_or("general");

        // Validate category against the instance — fail fast with the available
        // list so the agent can correct itself; treat connectivity errors as
        // non-fatal (validation is best-effort, the search request itself will
        // surface the real failure).
        match self.list_searxng_categories().await {
            Ok(cats) => {
                if !cats.iter().any(|c| c == category) {
                    return Err(format!(
                        "Invalid SearXNG category '{}'. Available: {}",
                        category,
                        cats.join(", ")
                    ));
                }
            }
            Err(e) => warn!("Could not validate SearXNG category: {e}"),
        }

        debug!(query, category, page, "Searching via SearXNG");

        let page_str = page.to_string();
        let resp = self
            .client
            .get(format!(
                "{}/search",
                self.config.searxng.url.trim_end_matches('/')
            ))
            .query(&[
                ("q", query),
                ("format", "json"),
                ("categories", category),
                ("pageno", page_str.as_str()),
            ])
            .header("User-Agent", "Mozilla/5.0 (compatible; BossFangAgent/0.1)")
            .send()
            .await
            .map_err(|e| format!("SearXNG request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("SearXNG API returned {}", resp.status()));
        }

        #[derive(serde::Deserialize)]
        struct SearxngResponse {
            results: Vec<SearxngResult>,
        }

        #[derive(serde::Deserialize)]
        struct SearxngResult {
            url: String,
            title: String,
            content: Option<String>,
            #[serde(alias = "pubdate")]
            published_date: Option<String>,
        }

        let data: SearxngResponse = resp
            .json()
            .await
            .map_err(|e| format!("SearXNG JSON parse failed: {e}"))?;

        if data.results.is_empty() {
            return Err(format!("No results found for '{query}' (SearXNG)."));
        }

        let total = data.results.len();
        let shown = total.min(max_results);
        let mut output = format!("Search results for '{query}' (SearXNG):\n\n");
        for (i, r) in data.results.iter().take(max_results).enumerate() {
            let content = r.content.as_deref().unwrap_or("");
            output.push_str(&format!("{}. {}\n   URL: {}\n", i + 1, r.title, r.url));
            if !content.is_empty() {
                output.push_str(&format!("   {content}\n"));
            }
            if let Some(date) = &r.published_date {
                output.push_str(&format!("   Published: {date}\n"));
            }
            output.push('\n');
        }

        // When client-side truncation hides results, tell the LLM so it can
        // either widen `max_results` or paginate via `pageno`.
        if shown < total {
            output.push_str(&format!(
                "Showing {shown} of {total} results on page {page} (truncated; pass a higher max_results or fetch the next page).\n",
            ));
        }

        Ok(wrap_external_content("searxng-search", &output))
    }

    /// List available search categories from the configured SearXNG instance.
    ///
    /// Fetches the `/config` endpoint and returns the categories the instance
    /// supports (e.g. "general", "images", "news", "videos"). Used both by
    /// `search_searxng` for category validation and as a discovery hook for
    /// agents that want to know what's available before issuing a query.
    ///
    /// Results are cached in-memory for [`SEARXNG_CATEGORIES_TTL`] so that a
    /// single user-facing search call costs one HTTP round-trip (`/search`)
    /// instead of two (`/config` + `/search`). On TTL miss the previous
    /// cached value is returned if the refresh fetch fails, so transient
    /// `/config` outages don't block searches.
    pub async fn list_searxng_categories(&self) -> Result<Vec<String>, String> {
        if self.config.searxng.url.is_empty() {
            return Err("SearXNG URL is not configured".to_string());
        }

        // Fast path — fresh cached value.
        {
            let guard = self.searxng_categories_cache.read().await;
            if let Some((cats, fetched_at)) = guard.as_ref() {
                if fetched_at.elapsed() < SEARXNG_CATEGORIES_TTL {
                    return Ok(cats.clone());
                }
            }
        }

        #[derive(serde::Deserialize)]
        struct SearxngConfig {
            categories: Vec<String>,
        }

        let fetch_result: Result<Vec<String>, String> = async {
            let resp = self
                .client
                .get(format!(
                    "{}/config",
                    self.config.searxng.url.trim_end_matches('/')
                ))
                .header("User-Agent", "Mozilla/5.0 (compatible; BossFangAgent/0.1)")
                .send()
                .await
                .map_err(|e| format!("SearXNG config request failed: {e}"))?;

            if !resp.status().is_success() {
                return Err(format!("SearXNG config API returned {}", resp.status()));
            }

            let data: SearxngConfig = resp
                .json()
                .await
                .map_err(|e| format!("SearXNG config JSON parse failed: {e}"))?;
            Ok(data.categories)
        }
        .await;

        match fetch_result {
            Ok(cats) => {
                // Refresh cache.
                let mut guard = self.searxng_categories_cache.write().await;
                *guard = Some((cats.clone(), Instant::now()));
                Ok(cats)
            }
            Err(e) => {
                // Stale cache fallback — never blocks search if `/config` is
                // briefly unreachable.
                let guard = self.searxng_categories_cache.read().await;
                if let Some((cats, _)) = guard.as_ref() {
                    warn!("SearXNG /config refresh failed, using stale cache: {e}");
                    Ok(cats.clone())
                } else {
                    Err(e)
                }
            }
        }
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

    // ── Jina provider tests ──────────────────────────────────

    #[test]
    fn test_jina_config_defaults() {
        let config = librefang_types::config::JinaSearchConfig::default();
        assert_eq!(config.api_key_env, "JINA_API_KEY");
        assert_eq!(config.max_results, 5);
        assert!(!config.use_eu_endpoint);
        assert!(config.country.is_empty());
        assert!(config.language.is_empty());
        assert!(!config.no_cache);
    }

    #[test]
    fn test_jina_search_provider_serde_json() {
        let provider = librefang_types::config::SearchProvider::Jina;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, "\"jina\"");
        let back: librefang_types::config::SearchProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, librefang_types::config::SearchProvider::Jina);
    }

    #[test]
    fn test_jina_search_provider_serde_toml() {
        // Ensure "jina" round-trips through TOML the same as JSON
        let toml_str = r#"search_provider = "jina""#;
        #[derive(serde::Deserialize)]
        struct W {
            search_provider: librefang_types::config::SearchProvider,
        }
        let w: W = toml::from_str(toml_str).unwrap();
        assert_eq!(
            w.search_provider,
            librefang_types::config::SearchProvider::Jina
        );
    }

    #[test]
    fn test_jina_config_toml_full_roundtrip() {
        let toml_str = r#"
            [web]
            search_provider = "jina"
            [web.jina]
            api_key_env = "MY_JINA_KEY"
            max_results = 10
            country = "US"
            language = "en"
            use_eu_endpoint = true
            no_cache = true
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.web.search_provider,
            librefang_types::config::SearchProvider::Jina
        );
        assert_eq!(config.web.jina.api_key_env, "MY_JINA_KEY");
        assert_eq!(config.web.jina.max_results, 10);
        assert_eq!(config.web.jina.country, "US");
        assert_eq!(config.web.jina.language, "en");
        assert!(config.web.jina.use_eu_endpoint);
        assert!(config.web.jina.no_cache);
    }

    #[test]
    fn test_jina_config_toml_partial_uses_defaults() {
        // Only set search_provider, leave [web.jina] section out entirely
        let toml_str = r#"
            [web]
            search_provider = "jina"
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.web.search_provider,
            librefang_types::config::SearchProvider::Jina
        );
        // All jina fields should fall back to defaults
        assert_eq!(config.web.jina.api_key_env, "JINA_API_KEY");
        assert_eq!(config.web.jina.max_results, 5);
        assert!(!config.web.jina.use_eu_endpoint);
        assert!(!config.web.jina.no_cache);
    }

    #[test]
    fn test_jina_config_serialize_roundtrip() {
        // Serialize a KernelConfig with Jina and deserialize it back
        let mut config = librefang_types::config::KernelConfig::default();
        config.web.search_provider = librefang_types::config::SearchProvider::Jina;
        config.web.jina.country = "PL".to_string();
        config.web.jina.language = "pl".to_string();
        config.web.jina.use_eu_endpoint = true;

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let back: librefang_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            back.web.search_provider,
            librefang_types::config::SearchProvider::Jina
        );
        assert_eq!(back.web.jina.country, "PL");
        assert_eq!(back.web.jina.language, "pl");
        assert!(back.web.jina.use_eu_endpoint);
    }

    #[test]
    fn test_web_config_default_includes_jina() {
        let config = librefang_types::config::WebConfig::default();
        assert_eq!(config.jina.api_key_env, "JINA_API_KEY");
        assert_eq!(config.jina.max_results, 5);
        assert!(!config.jina.use_eu_endpoint);
        assert!(config.jina.country.is_empty());
        assert!(config.jina.language.is_empty());
        assert!(!config.jina.no_cache);
    }

    #[test]
    fn test_kernel_config_default_has_jina_in_web() {
        let config = librefang_types::config::KernelConfig::default();
        assert_eq!(config.web.jina.api_key_env, "JINA_API_KEY");
    }

    #[test]
    fn test_jina_config_custom_env_var() {
        let toml_str = r#"
            [web.jina]
            api_key_env = "CUSTOM_JINA_TOKEN"
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.web.jina.api_key_env, "CUSTOM_JINA_TOKEN");
    }

    #[test]
    fn test_jina_eu_endpoint_default_false() {
        let config = librefang_types::config::JinaSearchConfig::default();
        assert!(
            !config.use_eu_endpoint,
            "EU endpoint should be disabled by default"
        );
    }

    #[test]
    fn test_jina_no_cache_default_false() {
        let config = librefang_types::config::JinaSearchConfig::default();
        assert!(!config.no_cache, "no_cache should be disabled by default");
    }

    #[test]
    fn test_jina_config_all_providers_coexist() {
        // Ensure Jina config doesn't break other providers in the same WebConfig
        let toml_str = r#"
            [web]
            search_provider = "auto"
            [web.brave]
            api_key_env = "MY_BRAVE"
            [web.tavily]
            api_key_env = "MY_TAVILY"
            [web.jina]
            api_key_env = "MY_JINA"
            max_results = 3
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.web.search_provider,
            librefang_types::config::SearchProvider::Auto
        );
        assert_eq!(config.web.brave.api_key_env, "MY_BRAVE");
        assert_eq!(config.web.tavily.api_key_env, "MY_TAVILY");
        assert_eq!(config.web.jina.api_key_env, "MY_JINA");
        assert_eq!(config.web.jina.max_results, 3);
        // Perplexity should keep its default
        assert_eq!(config.web.perplexity.api_key_env, "PERPLEXITY_API_KEY");
    }

    // ── SearXNG provider tests ───────────────────────────────

    #[test]
    fn test_searxng_config_default_url_empty() {
        let config = librefang_types::config::SearxngSearchConfig::default();
        assert!(
            config.url.is_empty(),
            "SearXNG defaults to disabled (empty URL) so users opt in explicitly"
        );
    }

    #[test]
    fn test_searxng_search_provider_serde_json() {
        let provider = librefang_types::config::SearchProvider::Searxng;
        let json = serde_json::to_string(&provider).unwrap();
        assert_eq!(json, "\"searxng\"");
        let back: librefang_types::config::SearchProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, librefang_types::config::SearchProvider::Searxng);
    }

    #[test]
    fn test_searxng_config_toml_full_roundtrip() {
        let toml_str = r#"
            [web]
            search_provider = "searxng"
            [web.searxng]
            url = "https://search.example.com"
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.web.search_provider,
            librefang_types::config::SearchProvider::Searxng
        );
        assert_eq!(config.web.searxng.url, "https://search.example.com");
    }

    #[test]
    fn test_searxng_config_toml_partial_uses_defaults() {
        let toml_str = r#"
            [web]
            search_provider = "searxng"
        "#;
        let config: librefang_types::config::KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.web.search_provider,
            librefang_types::config::SearchProvider::Searxng
        );
        assert!(config.web.searxng.url.is_empty());
    }

    #[test]
    fn test_web_config_default_includes_searxng() {
        let config = librefang_types::config::WebConfig::default();
        assert!(config.searxng.url.is_empty());
    }

    #[tokio::test]
    async fn test_search_searxng_unconfigured_url_errors() {
        let config = librefang_types::config::WebConfig::default();
        let cache = std::sync::Arc::new(crate::web_cache::WebCache::new(std::time::Duration::ZERO));
        let engine = WebSearchEngine::new(config, cache, Vec::new());
        let err = engine
            .search_searxng("test", 5, None, 1)
            .await
            .expect_err("empty SearXNG URL must fail");
        assert!(
            err.contains("SearXNG URL is not configured"),
            "expected configuration error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_list_searxng_categories_unconfigured_url_errors() {
        let config = librefang_types::config::WebConfig::default();
        let cache = std::sync::Arc::new(crate::web_cache::WebCache::new(std::time::Duration::ZERO));
        let engine = WebSearchEngine::new(config, cache, Vec::new());
        let err = engine
            .list_searxng_categories()
            .await
            .expect_err("empty SearXNG URL must fail");
        assert!(err.contains("SearXNG URL is not configured"));
    }

    #[tokio::test]
    async fn test_search_searxng_pageno_zero_rejected() {
        // Configure a non-empty URL so we get past the "URL not configured"
        // gate and actually exercise the pageno guard. The URL is never
        // contacted because the guard fires first.
        let mut config = librefang_types::config::WebConfig::default();
        config.searxng.url = "https://search.invalid".to_string();
        let cache = std::sync::Arc::new(crate::web_cache::WebCache::new(std::time::Duration::ZERO));
        let engine = WebSearchEngine::new(config, cache, Vec::new());
        let err = engine
            .search_searxng("test", 5, None, 0)
            .await
            .expect_err("pageno=0 must be rejected");
        assert!(
            err.contains("pageno must be >= 1"),
            "expected pageno guard error, got: {err}"
        );
    }
}
