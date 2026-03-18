//! Provider health probing — lightweight HTTP checks for local LLM providers.
//!
//! Probes local providers (Ollama, vLLM, LM Studio) for reachability and
//! dynamically discovers which models they currently serve.
//!
//! Includes a [`ProbeCache`] with configurable TTL so that the `/api/providers`
//! endpoint returns instantly on repeated dashboard loads instead of blocking
//! on TCP connect timeouts to unreachable local services.

use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Enriched metadata for a discovered model (Ollama-specific fields are optional).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredModelInfo {
    /// Model name/ID (e.g., "llama3.2:latest").
    pub name: String,
    /// Parameter count string from Ollama (e.g., "8.0B").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_size: Option<String>,
    /// Quantization level (e.g., "Q4_K_M", "Q8_0").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization_level: Option<String>,
    /// Model family (e.g., "llama", "gemma").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// On-disk size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Result of probing a provider endpoint.
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    /// Whether the provider responded successfully.
    pub reachable: bool,
    /// Round-trip latency in milliseconds.
    pub latency_ms: u64,
    /// Model IDs discovered from the provider's listing endpoint.
    pub discovered_models: Vec<String>,
    /// Enriched model metadata (populated for Ollama, empty for others).
    pub discovered_model_info: Vec<DiscoveredModelInfo>,
    /// Error message if the probe failed.
    pub error: Option<String>,
}

/// Check if a provider is a local HTTP provider that supports health probing.
///
/// Returns true for `"ollama"`, `"vllm"`, `"lmstudio"`, and `"lemonade"`.
pub fn is_local_provider(provider: &str) -> bool {
    matches!(
        provider.to_lowercase().as_str(),
        "ollama" | "vllm" | "lmstudio" | "lemonade"
    )
}

/// Overall request timeout for local provider health probes (connect + response).
const PROBE_TIMEOUT_SECS: u64 = 2;

/// TCP connect timeout — fail fast when the local port is not listening.
const PROBE_CONNECT_TIMEOUT_SECS: u64 = 1;

/// Default TTL for cached probe results (seconds).
const PROBE_CACHE_TTL_SECS: u64 = 60;

// ── Probe cache ──────────────────────────────────────────────────────────

/// Thread-safe cache for provider probe results.
///
/// Entries expire after [`PROBE_CACHE_TTL_SECS`] seconds. The cache is
/// designed to be stored once in `AppState` and shared across requests.
pub struct ProbeCache {
    inner: DashMap<String, (Instant, ProbeResult)>,
    ttl: Duration,
}

impl ProbeCache {
    /// Create a new cache with the default 60-second TTL.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
            ttl: Duration::from_secs(PROBE_CACHE_TTL_SECS),
        }
    }

    /// Look up a cached probe result. Returns `None` if missing or expired.
    pub fn get(&self, provider_id: &str) -> Option<ProbeResult> {
        if let Some(entry) = self.inner.get(provider_id) {
            let (ts, ref result) = *entry;
            if ts.elapsed() < self.ttl {
                return Some(result.clone());
            }
            // Expired — drop the read guard before removing
            drop(entry);
            self.inner.remove(provider_id);
        }
        None
    }

    /// Store a probe result.
    pub fn insert(&self, provider_id: &str, result: ProbeResult) {
        self.inner
            .insert(provider_id.to_string(), (Instant::now(), result));
    }
}

impl Default for ProbeCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Probe a provider's health by hitting its model listing endpoint.
///
/// - **Ollama**: `GET {base_url_root}/api/tags` → parses `.models[].name`
/// - **OpenAI-compat** (vLLM, LM Studio): `GET {base_url}/models` → parses `.data[].id`
///
/// `base_url` should be the provider's base URL from the catalog (e.g.,
/// `http://localhost:11434/v1` for Ollama, `http://localhost:8000/v1` for vLLM).
pub async fn probe_provider(provider: &str, base_url: &str) -> ProbeResult {
    let start = Instant::now();

    let client = match crate::http_client::proxied_client_builder()
        .connect_timeout(Duration::from_secs(PROBE_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeResult {
                error: Some(format!("Failed to build HTTP client: {e}")),
                ..Default::default()
            };
        }
    };

    let lower = provider.to_lowercase();

    // Ollama uses a non-OpenAI endpoint for model listing
    let (url, is_ollama) = if lower == "ollama" {
        // base_url is typically "http://localhost:11434/v1" — strip /v1 for the tags endpoint
        let root = base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .trim_end_matches("/v1/");
        (format!("{root}/api/tags"), true)
    } else {
        // OpenAI-compatible: GET {base_url}/models
        let trimmed = base_url.trim_end_matches('/');
        (format!("{trimmed}/models"), false)
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return ProbeResult {
                latency_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("{e}")),
                ..Default::default()
            };
        }
    };

    if !resp.status().is_success() {
        return ProbeResult {
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("HTTP {}", resp.status())),
            ..Default::default()
        };
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return ProbeResult {
                reachable: true, // server responded, just bad JSON
                latency_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Invalid JSON: {e}")),
                ..Default::default()
            };
        }
    };

    let latency_ms = start.elapsed().as_millis() as u64;

    // Parse model names and metadata
    let (models, model_info) = if is_ollama {
        // Ollama: { "models": [ { "name": "llama3.2:latest", "size": 12345, "details": { ... } }, ... ] }
        let arr = body
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let names: Vec<String> = arr
            .iter()
            .filter_map(|m| {
                m.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        let info: Vec<DiscoveredModelInfo> = arr
            .iter()
            .filter_map(|m| {
                let name = m.get("name").and_then(|n| n.as_str())?.to_string();
                let details = m.get("details");
                Some(DiscoveredModelInfo {
                    name,
                    parameter_size: details
                        .and_then(|d| d.get("parameter_size"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    quantization_level: details
                        .and_then(|d| d.get("quantization_level"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    family: details
                        .and_then(|d| d.get("family"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    size: m.get("size").and_then(|v| v.as_u64()),
                })
            })
            .collect();

        (names, info)
    } else {
        // OpenAI-compatible: { "data": [ { "id": "model-name", ... }, ... ] }
        let names = body
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|n| n.as_str()).map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (names, vec![])
    };

    ProbeResult {
        reachable: true,
        latency_ms,
        discovered_models: models,
        discovered_model_info: model_info,
        error: None,
    }
}

/// Probe a provider, returning a cached result when available.
///
/// If the cache contains a non-expired entry the HTTP request is skipped
/// entirely, making repeated `/api/providers` calls instantaneous.
pub async fn probe_provider_cached(
    provider: &str,
    base_url: &str,
    cache: &ProbeCache,
) -> ProbeResult {
    if let Some(cached) = cache.get(provider) {
        return cached;
    }
    let result = probe_provider(provider, base_url).await;
    cache.insert(provider, result.clone());
    result
}

/// Lightweight model probe -- sends a minimal completion request to verify a model is responsive.
///
/// Unlike `probe_provider` which checks the listing endpoint, this actually sends
/// a tiny prompt ("Hi") to verify the model can generate completions. Used by the
/// circuit breaker to re-test a provider during cooldown.
///
/// Returns `Ok(latency_ms)` if the model responds, or `Err(error_message)` if it fails.
pub async fn probe_model(
    provider: &str,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<u64, String> {
    let start = Instant::now();

    let client = crate::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 1,
        "temperature": 0.0
    });

    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        // Detect provider to set correct auth header
        let lower = provider.to_lowercase();
        if lower == "gemini" {
            req = req.header("x-goog-api-key", key);
        } else {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let resp = req.send().await.map_err(|e| format!("{e}"))?;
    let latency = start.elapsed().as_millis() as u64;

    if resp.status().is_success() {
        Ok(latency)
    } else {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(format!(
            "HTTP {status}: {}",
            crate::str_utils::safe_truncate_str(&body, 200)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_local_provider_true_for_ollama() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("Ollama"));
        assert!(is_local_provider("OLLAMA"));
        assert!(is_local_provider("vllm"));
        assert!(is_local_provider("lmstudio"));
        assert!(is_local_provider("lemonade"));
    }

    #[test]
    fn test_is_local_provider_false_for_openai() {
        assert!(!is_local_provider("openai"));
        assert!(!is_local_provider("anthropic"));
        assert!(!is_local_provider("gemini"));
        assert!(!is_local_provider("groq"));
        assert!(!is_local_provider("claude-code"));
        assert!(!is_local_provider("qwen-code"));
    }

    #[test]
    fn test_probe_result_default() {
        let result = ProbeResult::default();
        assert!(!result.reachable);
        assert_eq!(result.latency_ms, 0);
        assert!(result.discovered_models.is_empty());
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn test_probe_unreachable_returns_error() {
        // Probe a port that's almost certainly not running a server
        let result = probe_provider("ollama", "http://127.0.0.1:19999").await;
        assert!(!result.reachable);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_probe_timeout_value() {
        assert_eq!(PROBE_TIMEOUT_SECS, 2);
        assert_eq!(PROBE_CONNECT_TIMEOUT_SECS, 1);
    }

    #[test]
    fn test_probe_model_url_construction() {
        // Verify the URL format logic used inside probe_model.
        let url = format!(
            "{}/chat/completions",
            "http://localhost:8000/v1".trim_end_matches('/')
        );
        assert_eq!(url, "http://localhost:8000/v1/chat/completions");

        let url2 = format!(
            "{}/chat/completions",
            "http://localhost:8000/v1/".trim_end_matches('/')
        );
        assert_eq!(url2, "http://localhost:8000/v1/chat/completions");
    }

    #[tokio::test]
    async fn test_probe_model_unreachable() {
        let result = probe_model("test", "http://127.0.0.1:19998/v1", "test-model", None).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_probe_cache_miss_returns_none() {
        let cache = ProbeCache::new();
        assert!(cache.get("ollama").is_none());
    }

    #[test]
    fn test_probe_cache_hit_returns_result() {
        let cache = ProbeCache::new();
        let result = ProbeResult {
            reachable: true,
            latency_ms: 42,
            discovered_models: vec!["llama3".into()],
            discovered_model_info: vec![],
            error: None,
        };
        cache.insert("ollama", result.clone());
        let cached = cache.get("ollama").expect("should be cached");
        assert!(cached.reachable);
        assert_eq!(cached.latency_ms, 42);
        assert_eq!(cached.discovered_models, vec!["llama3".to_string()]);
    }

    #[test]
    fn test_probe_cache_default() {
        let cache = ProbeCache::default();
        assert!(cache.get("anything").is_none());
        assert_eq!(cache.ttl, Duration::from_secs(PROBE_CACHE_TTL_SECS));
    }

    #[test]
    fn test_discovered_model_info_serialization() {
        let info = DiscoveredModelInfo {
            name: "llama3.2:latest".to_string(),
            parameter_size: Some("3.2B".to_string()),
            quantization_level: Some("Q4_K_M".to_string()),
            family: Some("llama".to_string()),
            size: Some(1_928_000_000),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["name"], "llama3.2:latest");
        assert_eq!(json["parameter_size"], "3.2B");
        assert_eq!(json["quantization_level"], "Q4_K_M");
        assert_eq!(json["family"], "llama");
        assert_eq!(json["size"], 1_928_000_000_u64);
    }

    #[test]
    fn test_discovered_model_info_skips_none_fields() {
        let info = DiscoveredModelInfo {
            name: "gpt-4".to_string(),
            parameter_size: None,
            quantization_level: None,
            family: None,
            size: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["name"], "gpt-4");
        assert!(json.get("parameter_size").is_none());
        assert!(json.get("quantization_level").is_none());
    }
}
