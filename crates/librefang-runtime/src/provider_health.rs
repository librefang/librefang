//! Provider health probing — lightweight HTTP checks for local LLM providers.
//!
//! Probes local providers (Ollama, vLLM, LM Studio) for reachability and
//! dynamically discovers which models they currently serve.
//!
//! Includes a [`ProbeCache`] with configurable TTL so that the `/api/providers`
//! endpoint returns instantly on repeated dashboard loads instead of blocking
//! on TCP connect timeouts to unreachable local services.

use dashmap::DashMap;
use std::sync::OnceLock;
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
    /// Model family (e.g., "llama", "gemma", "nomic-bert").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// All model families reported by Ollama (e.g., ["llama", "clip"]).
    /// "clip" indicates a vision-capable model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub families: Option<Vec<String>>,
    /// On-disk size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Capabilities reported by Ollama (e.g., ["completion", "vision", "tools"]).
    /// Newer Ollama versions (≥0.7) include this in /api/tags; older versions omit it.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub capabilities: Vec<String>,
}

/// Result of probing a provider endpoint.
#[derive(Debug, Clone)]
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
    /// Wall-clock time when the probe was executed (RFC 3339).
    pub probed_at: String,
}

impl Default for ProbeResult {
    fn default() -> Self {
        Self {
            reachable: false,
            latency_ms: 0,
            discovered_models: Vec::new(),
            discovered_model_info: Vec::new(),
            error: None,
            probed_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Infer Ollama model capabilities from the model name and family when the
/// server does not include an explicit `capabilities` array (Ollama < 0.7).
///
/// Returns a subset of `["completion", "embedding", "vision", "tools"]`.
fn infer_ollama_capabilities(name: &str, family: Option<&str>) -> Vec<String> {
    let lower = name.to_lowercase();
    let fam = family.unwrap_or("").to_lowercase();

    // Embedding model detection — these do NOT support chat completions.
    let is_embed = fam.contains("bert")
        || lower.contains("embed")
        || lower.contains("minilm")
        || lower.contains("bge-")
        || lower.contains("e5-")
        || lower.contains("gte-");
    if is_embed {
        return vec!["embedding".to_string()];
    }

    let mut caps = vec!["completion".to_string()];

    // Vision detection.
    let has_vision = fam.contains("clip")
        || lower.contains("llava")
        || lower.contains("vision")
        || lower.contains("vl:")
        || lower.contains("-vl-")
        || lower.contains("minicpm-v")
        || lower.contains("bakllava")
        || lower.contains("moondream");
    if has_vision {
        caps.push("vision".to_string());
    }

    caps
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

/// Per-request total timeout for loopback probe targets.
///
/// The shared probe client (see [`PROBE_CLIENT`]) is configured with the
/// relaxed remote budget so it can serve HTTPS reverse-proxy fronts; loopback
/// callers tighten the total via `RequestBuilder::timeout(...)` so a dead
/// local daemon still surfaces fast. `connect_timeout` cannot be overridden
/// per-request — both profiles share the client's 3 s connect budget — but
/// this 2 s request budget fires first on any pathological local stall.
const PROBE_TIMEOUT_SECS: u64 = 2;

/// Total request timeout for non-loopback probe targets (HTTPS reverse proxies,
/// remote ollama, etc.). 8 s comfortably absorbs cold-start TLS handshakes
/// (~1–3 s on first connect) plus end-to-end WAN latency.
const PROBE_REMOTE_TIMEOUT_SECS: u64 = 8;

/// TCP connect timeout for the shared probe client. Applied uniformly because
/// reqwest does not expose per-request connect timeouts.
const PROBE_REMOTE_CONNECT_TIMEOUT_SECS: u64 = 3;

/// Format a `reqwest::Error` with its cause chain so probe failures stay
/// diagnosable. The default `Display` impl drops the inner cause, leaving the
/// generic `error sending request for url (...)` line for everything from a
/// timeout to a TLS handshake failure to DNS — flatten the chain so the WARN
/// line tells operators which it actually was.
fn format_request_error(err: &reqwest::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(err);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    // reqwest renders timeouts as "operation timed out" / "timed out" — the
    // substring is "timed out", not "timeout". Check both so we don't
    // double-append the marker when the cause chain already mentioned it.
    if err.is_timeout()
        && !parts.iter().any(|p| {
            let s = p.to_lowercase();
            s.contains("timeout") || s.contains("timed out")
        })
    {
        parts.push("timed out".to_string());
    }
    parts.join(": ")
}

/// Process-wide cache for the probe HTTP client. Constructed once on first
/// use so the connection pool and TLS sessions persist across probe cycles —
/// otherwise every 60-second probe to an HTTPS reverse-proxy front (Open WebUI
/// etc.) pays a fresh ~1–2 s cold-start TLS handshake.
///
/// Configured with the relaxed remote timeouts; loopback callers tighten the
/// total request budget via per-request `RequestBuilder::timeout(...)`. The
/// connect timeout cannot be overridden per-request, so loopback uses the
/// same 3-second connect budget as remote — acceptable because a localhost
/// daemon that can't accept TCP within 3 s is already broken.
///
/// Limitation: not invalidated on `[proxy]` config reload. Users who change
/// `http_proxy` / `https_proxy` at runtime need to restart the daemon for
/// probe traffic to pick up the new forward proxy. Acceptable because the
/// motivating case (reverse-proxy ollama URLs like Open WebUI) does not
/// involve forward proxies, and `[proxy]` reloads are rare in practice.
static PROBE_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Return the shared probe HTTP client, building it on first call.
///
/// Exposed publicly so on-demand `/api/providers/{name}/test` handlers
/// can reuse the same connection-pooled client instead of paying a fresh
/// TLS handshake (~80 ms over the Pacific) on every dashboard click.
/// Before this was public, that endpoint was rebuilding a `reqwest::Client`
/// per call and reporting the rebuilt-client + cold-handshake cost as the
/// "provider latency", roughly doubling what operators saw vs. the steady-
/// state probe cycle.
pub fn probe_client() -> &'static reqwest::Client {
    PROBE_CLIENT.get_or_init(|| {
        crate::http_client::proxied_client_builder()
            .connect_timeout(Duration::from_secs(PROBE_REMOTE_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(PROBE_REMOTE_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|e| {
                // Build can fail in exotic TLS configurations. Fall back to a
                // default reqwest::Client so probes still run rather than
                // failing every cycle with the same construction error.
                tracing::warn!(
                    error = %e,
                    "probe HTTP client build failed; falling back to default reqwest::Client"
                );
                reqwest::Client::new()
            })
    })
}

/// Returns `true` when `base_url` points at a loopback host. Loopback hosts
/// get the tight [`PROBE_TIMEOUT_SECS`] per-request total; everything else
/// uses the shared client's [`PROBE_REMOTE_TIMEOUT_SECS`] default.
///
/// Both profiles share the [`PROBE_REMOTE_CONNECT_TIMEOUT_SECS`] connect
/// budget because reqwest does not expose per-request connect timeouts; this
/// is tolerable since a localhost daemon that cannot accept TCP within 3 s is
/// already broken.
///
/// Unparseable URLs are treated as loopback to preserve fast-fail behaviour
/// for plain `localhost:11434` style strings.
fn is_loopback_base_url(base_url: &str) -> bool {
    match url::Url::parse(base_url) {
        Ok(u) => match u.host_str() {
            Some(h) => matches!(
                h.to_ascii_lowercase().as_str(),
                "localhost" | "127.0.0.1" | "::1" | "[::1]"
            ),
            None => true,
        },
        Err(_) => true,
    }
}

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
///
/// `api_key` is forwarded as `Authorization: Bearer <key>` when `Some` and
/// non-empty. Required when the local provider is fronted by an
/// authenticating reverse proxy (e.g. Open WebUI exposes ollama under
/// `/ollama/v1` behind a JWT) — a bare `None` keeps the original
/// "no auth" behaviour for direct-to-localhost setups.
pub async fn probe_provider(provider: &str, base_url: &str, api_key: Option<&str>) -> ProbeResult {
    let start = Instant::now();
    let lower = provider.to_lowercase();
    let is_loopback = is_loopback_base_url(base_url);

    // For the "ollama" provider slot we try the native /api/tags first, then
    // fall back to the OpenAI-compatible /v1/models. Lemonade Server, LM Studio
    // run with OpenAI-only mode, llama.cpp's server, and any other "looks like
    // ollama but isn't" daemon all expose only the OpenAI shape — without the
    // fallback, those users see "probe failed" forever even though the chat
    // endpoint works fine. See issue #3191.
    if lower == "ollama" {
        let root = base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .trim_end_matches("/v1/");
        let tags_url = format!("{root}/api/tags");
        match try_probe_endpoint(&tags_url, EndpointShape::OllamaTags, api_key, is_loopback).await {
            EndpointOutcome::Ok { models, model_info } => {
                return ProbeResult {
                    reachable: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                    discovered_models: models,
                    discovered_model_info: model_info,
                    error: None,
                    ..Default::default()
                };
            }
            EndpointOutcome::Failed { error: tags_error } => {
                let openai_url = format!("{}/models", base_url.trim_end_matches('/'));
                match try_probe_endpoint(
                    &openai_url,
                    EndpointShape::OpenAiModels,
                    api_key,
                    is_loopback,
                )
                .await
                {
                    EndpointOutcome::Ok { models, model_info } => {
                        return ProbeResult {
                            reachable: true,
                            latency_ms: start.elapsed().as_millis() as u64,
                            discovered_models: models,
                            discovered_model_info: model_info,
                            error: None,
                            ..Default::default()
                        };
                    }
                    EndpointOutcome::Failed {
                        error: openai_error,
                    } => {
                        // Surface both errors so operators can tell whether the
                        // server is wholly unreachable or just doesn't speak
                        // either dialect.
                        return ProbeResult {
                            latency_ms: start.elapsed().as_millis() as u64,
                            error: Some(format!(
                                "/api/tags: {tags_error}; /v1/models: {openai_error}"
                            )),
                            ..Default::default()
                        };
                    }
                }
            }
        }
    }

    // Non-ollama: dispatch on the registered API format. Anthropic-protocol
    // providers (anthropic, byteplus_coding, volcengine_coding, …) don't
    // serve `{base}/models` and answer requests with `Authorization: Bearer`
    // with a 4xx — which the old probe reported as "unreachable" while
    // *also* counting the 4xx round-trip in `latency_ms`, doubly misleading
    // the dashboard. Pick the right path + auth headers per format.
    let shape = match librefang_llm_drivers::drivers::provider_api_format(provider) {
        Some(librefang_llm_drivers::drivers::ApiFormat::Anthropic) => {
            EndpointShape::AnthropicModels
        }
        // OpenAI-compat is the historical default for everything else and
        // remains correct for OpenAI/Groq/DeepSeek/Mistral/etc. Unknown
        // providers also fall through here (matches pre-PR behaviour).
        _ => EndpointShape::OpenAiModels,
    };
    let probe_path = match shape {
        EndpointShape::AnthropicModels => "/v1/models",
        _ => "/models",
    };
    let probe_url = format!("{}{}", base_url.trim_end_matches('/'), probe_path);
    match try_probe_endpoint(&probe_url, shape, api_key, is_loopback).await {
        EndpointOutcome::Ok { models, model_info } => ProbeResult {
            reachable: true,
            latency_ms: start.elapsed().as_millis() as u64,
            discovered_models: models,
            discovered_model_info: model_info,
            error: None,
            ..Default::default()
        },
        EndpointOutcome::Failed { error } => ProbeResult {
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(error),
            ..Default::default()
        },
    }
}

/// Wire format expected at a probe endpoint.
#[derive(Debug, Clone, Copy)]
enum EndpointShape {
    /// Ollama native: `{ "models": [{ "name": "...", "details": {...} }, ...] }`
    OllamaTags,
    /// OpenAI-compatible: `{ "data": [{ "id": "...", ... }, ...] }`. Auth via
    /// `Authorization: Bearer <key>`.
    OpenAiModels,
    /// Anthropic-compatible `/v1/models`. Same response shape as OpenAI
    /// (`{ "data": [...] }`) but auth is `x-api-key` + `anthropic-version`.
    /// 401/403 here is treated as reachable: it's a positive signal that
    /// the server speaks the Anthropic wire protocol — only the key is
    /// rejected, which is a *configured-but-unauthenticated* state, not
    /// network failure. Otherwise a misconfigured provider would burn ~RTT
    /// on every probe and report inflated latency to the dashboard.
    AnthropicModels,
}

/// Internal result of one probe attempt — used to drive the ollama→openai
/// fallback chain inside [`probe_provider`].
enum EndpointOutcome {
    Ok {
        models: Vec<String>,
        model_info: Vec<DiscoveredModelInfo>,
    },
    Failed {
        error: String,
    },
}

/// Send one probe request and parse the response in the given shape.
///
/// Treats "transport / non-2xx / non-JSON / wrong-shape body" all as `Failed`
/// so the caller can decide whether to fall back to a different endpoint
/// (the ollama path needs this — Lemonade returns 404 on /api/tags but a
/// well-formed list on /v1/models).
async fn try_probe_endpoint(
    url: &str,
    shape: EndpointShape,
    api_key: Option<&str>,
    is_loopback: bool,
) -> EndpointOutcome {
    let client = probe_client();
    let mut req = client.get(url);
    if is_loopback {
        req = req.timeout(Duration::from_secs(PROBE_TIMEOUT_SECS));
    }
    if let Some(key) = api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            req = match shape {
                EndpointShape::AnthropicModels => req
                    .header("x-api-key", trimmed)
                    .header("anthropic-version", "2023-06-01"),
                EndpointShape::OllamaTags | EndpointShape::OpenAiModels => {
                    req.header("Authorization", format!("Bearer {trimmed}"))
                }
            };
        }
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return EndpointOutcome::Failed {
                error: format_request_error(&e),
            };
        }
    };

    let status = resp.status();
    if !status.is_success() {
        // Anthropic-protocol endpoints often answer GET /v1/models with 401
        // ("API key invalid for this scope") or 404 ("listing not exposed
        // for this plan") even when the chat path works fine. Treat those
        // as *reachable but no model list* rather than failing the probe —
        // a fail flips the dashboard provider tile to "broken" and reports
        // the round-trip latency on a path nobody actually uses for
        // inference. Real outages still surface as 5xx / connection errors.
        if matches!(shape, EndpointShape::AnthropicModels)
            && (status.as_u16() == 401 || status.as_u16() == 403 || status.as_u16() == 404)
        {
            return EndpointOutcome::Ok {
                models: Vec::new(),
                model_info: Vec::new(),
            };
        }
        return EndpointOutcome::Failed {
            error: format!("HTTP {status}"),
        };
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return EndpointOutcome::Failed {
                error: format!("Invalid JSON: {e}"),
            };
        }
    };

    match shape {
        EndpointShape::OllamaTags => parse_ollama_tags(&body),
        // Anthropic /v1/models returns the same `{ "data": [{ "id": ... }] }`
        // shape OpenAI does, so the OpenAI parser handles both.
        EndpointShape::OpenAiModels | EndpointShape::AnthropicModels => parse_openai_models(&body),
    }
}

/// Parse an Ollama `/api/tags` response into discovered models + enriched info.
///
/// Reports `Failed` (rather than `Ok` with empty lists) when the body is not
/// the expected shape so the caller can fall back to a different endpoint.
fn parse_ollama_tags(body: &serde_json::Value) -> EndpointOutcome {
    let arr = match body.get("models").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            return EndpointOutcome::Failed {
                error: "response did not contain a `models` array".to_string(),
            };
        }
    };

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
            let families = details
                .and_then(|d| d.get("families"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|f| f.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty());
            let family = details
                .and_then(|d| d.get("family"))
                .and_then(|v| v.as_str())
                .map(String::from);

            // Ollama ≥0.7 exposes a top-level `capabilities` array per
            // model in /api/tags. Older versions omit it — we fall back
            // to heuristic detection from the model name and family.
            let capabilities: Vec<String> =
                if let Some(caps) = m.get("capabilities").and_then(|v| v.as_array()) {
                    caps.iter()
                        .filter_map(|c| c.as_str().map(String::from))
                        .collect()
                } else {
                    infer_ollama_capabilities(&name, family.as_deref())
                };

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
                family,
                families,
                size: m.get("size").and_then(|v| v.as_u64()),
                capabilities,
            })
        })
        .collect();

    EndpointOutcome::Ok {
        models: names,
        model_info: info,
    }
}

/// Parse an OpenAI-compatible `/v1/models` response into discovered model IDs.
///
/// `data[].id` is the only field required by spec. We don't try to derive
/// capabilities from this shape because OpenAI-format `/v1/models` doesn't
/// expose vision/tools flags — capability inference is left to whatever
/// downstream consumer cares (e.g. `merge_discovered_models`).
fn parse_openai_models(body: &serde_json::Value) -> EndpointOutcome {
    let arr = match body.get("data").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            return EndpointOutcome::Failed {
                error: "response did not contain a `data` array".to_string(),
            };
        }
    };

    let names: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("id").and_then(|n| n.as_str()).map(String::from))
        .collect();

    EndpointOutcome::Ok {
        models: names,
        model_info: vec![],
    }
}

/// Probe a provider, returning a cached result when available.
///
/// If the cache contains a non-expired entry the HTTP request is skipped
/// entirely, making repeated `/api/providers` calls instantaneous.
pub async fn probe_provider_cached(
    provider: &str,
    base_url: &str,
    api_key: Option<&str>,
    cache: &ProbeCache,
) -> ProbeResult {
    if let Some(cached) = cache.get(provider) {
        return cached;
    }
    let result = probe_provider(provider, base_url, api_key).await;
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
        let result = probe_provider("ollama", "http://127.0.0.1:19999", None).await;
        assert!(!result.reachable);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_probe_timeout_value() {
        assert_eq!(PROBE_TIMEOUT_SECS, 2);
        assert_eq!(PROBE_REMOTE_TIMEOUT_SECS, 8);
        assert_eq!(PROBE_REMOTE_CONNECT_TIMEOUT_SECS, 3);
        // Remote budget must always exceed the loopback per-request override;
        // otherwise loopback callers would slow themselves down vs the shared
        // client's default.
        const { assert!(PROBE_REMOTE_TIMEOUT_SECS > PROBE_TIMEOUT_SECS) };
    }

    #[test]
    fn test_probe_client_is_shared() {
        // The shared client is initialised lazily and persisted via OnceLock —
        // every probe call must see the same instance so the connection pool
        // and TLS sessions actually amortise across the 60-second probe loop.
        let a = probe_client();
        let b = probe_client();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn test_is_loopback_base_url_loopback_hosts() {
        assert!(is_loopback_base_url("http://127.0.0.1:11434/v1"));
        assert!(is_loopback_base_url("http://localhost:11434/v1"));
        assert!(is_loopback_base_url("http://LOCALHOST:11434"));
        assert!(is_loopback_base_url("http://[::1]:11434/v1"));
        assert!(is_loopback_base_url("http://127.0.0.1:8000/"));
    }

    #[test]
    fn test_is_loopback_base_url_remote_hosts() {
        assert!(!is_loopback_base_url("https://webui.example.com/ollama/v1"));
        assert!(!is_loopback_base_url("http://192.168.1.10:11434"));
        assert!(!is_loopback_base_url("https://api.openai.com/v1"));
        // LAN hosts that resolve via mDNS / private DNS are still "remote"
        // for the probe — they cross a network boundary.
        assert!(!is_loopback_base_url("http://nas.local:11434"));
    }

    #[test]
    fn test_is_loopback_base_url_unparseable_falls_back_to_loopback() {
        // Unparseable URLs preserve the legacy fast-fail budget rather than
        // mysteriously slowing every probe in a malformed-config scenario.
        assert!(is_loopback_base_url("not a url"));
        assert!(is_loopback_base_url(""));
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
            ..Default::default()
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
            families: None,
            size: Some(1_928_000_000),
            capabilities: vec!["completion".to_string()],
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
            families: None,
            size: None,
            capabilities: vec![],
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["name"], "gpt-4");
        assert!(json.get("parameter_size").is_none());
        assert!(json.get("quantization_level").is_none());
        // Empty capabilities should be skipped
        assert!(json.get("capabilities").is_none());
    }

    #[test]
    fn test_infer_ollama_capabilities_embedding() {
        assert_eq!(
            infer_ollama_capabilities("nomic-embed-text:latest", Some("nomic-bert")),
            vec!["embedding"]
        );
        assert_eq!(
            infer_ollama_capabilities("bge-small-en:latest", None),
            vec!["embedding"]
        );
        // all-minilm variants (e.g. all-minilm:l6-v2) must be detected as embedding
        assert_eq!(
            infer_ollama_capabilities("all-minilm:l6-v2", None),
            vec!["embedding"]
        );
    }

    #[test]
    fn test_infer_ollama_capabilities_vision() {
        let caps = infer_ollama_capabilities("llava:latest", Some("llava"));
        assert!(caps.contains(&"completion".to_string()));
        assert!(caps.contains(&"vision".to_string()));
    }

    #[test]
    fn test_infer_ollama_capabilities_chat_only() {
        let caps = infer_ollama_capabilities("llama3.2:latest", Some("llama"));
        assert_eq!(caps, vec!["completion"]);
    }

    fn ok_or_panic(outcome: EndpointOutcome) -> (Vec<String>, Vec<DiscoveredModelInfo>) {
        match outcome {
            EndpointOutcome::Ok { models, model_info } => (models, model_info),
            EndpointOutcome::Failed { error } => panic!("expected Ok, got Failed: {error}"),
        }
    }

    fn fail_or_panic(outcome: EndpointOutcome) -> String {
        match outcome {
            EndpointOutcome::Failed { error } => error,
            EndpointOutcome::Ok { models, .. } => {
                panic!("expected Failed, got Ok with {} models", models.len())
            }
        }
    }

    #[test]
    fn test_parse_ollama_tags_extracts_names_and_capabilities() {
        // Mirrors what `ollama list` returns on a box that has llama3.2 pulled.
        let body = serde_json::json!({
            "models": [
                {
                    "name": "llama3.2:latest",
                    "size": 2_000_000_000u64,
                    "details": {
                        "family": "llama",
                        "families": ["llama"],
                        "parameter_size": "3.2B",
                        "quantization_level": "Q4_K_M"
                    },
                    "capabilities": ["completion", "tools"]
                }
            ]
        });
        let (models, info) = ok_or_panic(parse_ollama_tags(&body));
        assert_eq!(models, vec!["llama3.2:latest"]);
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].name, "llama3.2:latest");
        assert_eq!(info[0].family.as_deref(), Some("llama"));
        assert_eq!(info[0].capabilities, vec!["completion", "tools"]);
    }

    #[test]
    fn test_parse_ollama_tags_failed_on_wrong_shape() {
        // Lemonade's /api/tags returns 404, but if it returned a 200 with the
        // wrong shape we still need to fall back — assert the parser flags it.
        let body = serde_json::json!({"data": []}); // OpenAI shape, no "models"
        let err = fail_or_panic(parse_ollama_tags(&body));
        assert!(err.contains("models"));
    }

    #[test]
    fn test_parse_openai_models_extracts_data_ids() {
        // Exact body the issue #3191 reporter pasted from Lemonade — verify
        // the parser pulls `Gemma-4-26B-A4B-it-GGUF` out as-is, no normalization.
        let body = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "Gemma-4-26B-A4B-it-GGUF", "object": "model"},
                {"id": "nomic-embed-text-v2-moe-GGUF", "object": "model"}
            ]
        });
        let (models, info) = ok_or_panic(parse_openai_models(&body));
        assert_eq!(
            models,
            vec!["Gemma-4-26B-A4B-it-GGUF", "nomic-embed-text-v2-moe-GGUF"]
        );
        // OpenAI shape has no capability metadata.
        assert!(info.is_empty());
    }

    #[test]
    fn test_parse_openai_models_failed_on_wrong_shape() {
        let body = serde_json::json!({"models": []}); // Ollama shape, no "data"
        let err = fail_or_panic(parse_openai_models(&body));
        assert!(err.contains("data"));
    }

    #[tokio::test]
    async fn test_probe_provider_ollama_unreachable_reports_both_endpoints() {
        // The whole point of the fallback chain is to cover servers like
        // Lemonade that 404 on /api/tags but answer /v1/models. When *both*
        // endpoints fail the error message must mention both so an operator
        // can tell "wrong server type" apart from "server is dead".
        let result = probe_provider("ollama", "http://127.0.0.1:19994", None).await;
        assert!(!result.reachable);
        let err = result.error.expect("probe should have an error");
        assert!(
            err.contains("/api/tags"),
            "missing /api/tags in error: {err}"
        );
        assert!(
            err.contains("/v1/models"),
            "missing /v1/models in error: {err}"
        );
    }
}
