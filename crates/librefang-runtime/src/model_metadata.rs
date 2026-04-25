//! Model metadata lookup pipeline.
//!
//! Resolves a model's `context_window` (and optionally `max_output_tokens`)
//! through a layered fallback chain so the agent loop never has to fall back
//! to a coarse `200_000` default when the catalog misses or the user runs a
//! self-hosted endpoint with a non-standard window.
//!
//! See `.plans/model-metadata-lookup.md` for the full design and the
//! 5-layer rationale. **Status after PR-2.5**: all five layers are live.
//! L4 now covers Ollama `/api/show`, Anthropic `/v1/models/{id}`, and a
//! generic OpenAI-compat `/v1/models/{id}` branch (vLLM / LM Studio /
//! LiteLLM-style endpoints).
//!
//! | Layer | Source | Status |
//! |---|---|---|
//! | L1 | Agent manifest override (`model.context_window`) | ✅ |
//! | L2 | Registry / `ModelCatalog` (provider-aware) | ✅ |
//! | L3 | Persisted cache (`~/.librefang/cache/model_metadata.json`, 24h TTL) | ✅ |
//! | L4 | Runtime probe — Ollama / Anthropic / OpenAI-compat `/v1/models` | ✅ |
//! | L5 | Hardcoded fallback (< 20 entries) + provider default | ✅ |
//!
//! `resolve_model_metadata` is currently **passive** — no caller wires
//! it into `agent_loop` yet. PR-3 will replace the
//! `cat.find_model(...).map(|m| m.context_window).filter(|w| *w > 0)`
//! call sites in `kernel/mod.rs` with a single `resolve_model_metadata`
//! invocation, and retire the uniform 200K default in
//! `agent_loop.rs:1285`.

use chrono::{DateTime, Utc};
use librefang_types::model_catalog::{Modality, ModelCatalogEntry, ModelTier};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::model_catalog::ModelCatalog;

/// Result of a metadata lookup, plus the layer that produced it (for
/// telemetry — the dashboard surfaces this string so users can see *why*
/// their context window resolved to a particular value).
#[derive(Debug, Clone)]
pub struct ResolvedModel<'a> {
    pub entry: Cow<'a, ModelCatalogEntry>,
    pub source: MetadataSource,
}

/// Which layer of the lookup pipeline produced this metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSource {
    /// L1 — explicit `[model] context_window = N` in agent.toml.
    AgentManifest,
    /// L2 — entry returned by `ModelCatalog::find_model_for_provider`.
    Registry,
    /// L3 — fresh entry in the persisted cache (M2 / not yet wired).
    PersistedCache,
    /// L4 — live `/v1/models` or `/api/show` probe (M2 / not yet wired).
    RuntimeProbe,
    /// L5 — substring match in [`HARDCODED_FALLBACKS`].
    HardcodedFallback,
    /// L5 tail — anthropic-host generic default (200K).
    Default200kAnthropic,
    /// L5 tail — generic default for unknown providers (32K).
    Default32k,
}

impl MetadataSource {
    /// Stable string used in tracing and the dashboard surface.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentManifest => "agent_manifest",
            Self::Registry => "registry",
            Self::PersistedCache => "persisted_cache",
            Self::RuntimeProbe => "runtime_probe",
            Self::HardcodedFallback => "hardcoded_fallback",
            Self::Default200kAnthropic => "default_200k_anthropic",
            Self::Default32k => "default_32k",
        }
    }
}

/// Inputs to a metadata lookup.
///
/// `provider` is the agent's configured provider name (e.g. `"anthropic"`,
/// `"ollama"`). It can be empty when unknown — the pipeline will then
/// degrade `find_model_for_provider` to a provider-blind `find_model` and
/// the substring fallback table will look at the bare model name.
/// `api_key` is required for L4 probes against authenticated endpoints
/// (e.g. Anthropic `/v1/models`). Local probes (Ollama `/api/show`) do
/// not need it.
#[derive(Debug, Clone, Copy)]
pub struct MetadataRequest<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub base_url: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub manifest_override_context: Option<u64>,
    pub manifest_override_max_output: Option<u64>,
}

/// Built-in last-resort table for `context_window` lookup.
///
/// Each entry is a (lowercase substring, context_window) pair. At lookup
/// time we lowercase the model ID, sort by **longest key first**, and
/// take the first substring hit so `claude-sonnet-4-6` matches the 1M
/// entry instead of the more permissive `claude` 200K entry.
///
/// **Deliberately small (< 20 entries).** The registry already covers
/// every supported model with full pricing/capabilities; this table only
/// catches the case where the registry is stale (new model id) or
/// missing (offline daemon, fresh install).
const HARDCODED_FALLBACKS: &[(&str, u64)] = &[
    // Anthropic — order matters: longer (more specific) keys first when
    // the lookup loop sorts them.
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-haiku-4-5", 200_000),
    ("claude", 200_000),
    // OpenAI
    ("gpt-5.4", 1_050_000),
    ("gpt-5", 400_000),
    ("gpt-4.1", 1_047_576),
    ("gpt-4", 128_000),
    // Google
    ("gemini-2", 1_048_576),
    ("gemini", 1_048_576),
    ("gemma-3", 131_072),
    // Open weights
    ("deepseek", 128_000),
    ("llama", 131_072),
    ("qwen3-coder", 262_144),
    ("qwen", 131_072),
    ("kimi", 262_144),
    ("nemotron", 131_072),
    ("grok-4", 256_000),
    ("grok", 131_072),
];

/// Last-resort default when neither registry, cache, probe, nor the
/// hardcoded table can identify the model. Returned together with
/// `MetadataSource::Default32k` (or `Default200kAnthropic` when the
/// provider is recognisably Anthropic).
const DEFAULT_GENERIC_CONTEXT: u64 = 32_768;
const DEFAULT_ANTHROPIC_CONTEXT: u64 = 200_000;

/// Provider-prefix tokens stripped from model IDs at the top of the
/// pipeline (e.g. `openrouter:claude-opus-4-7` → `claude-opus-4-7`).
///
/// Mirrors hermes-agent's `_PROVIDER_PREFIXES` frozenset.
const PROVIDER_PREFIXES: &[&str] = &[
    "openrouter",
    "anthropic",
    "openai",
    "openai-codex",
    "gemini",
    "google",
    "deepseek",
    "ollama",
    "ollama-cloud",
    "copilot",
    "github",
    "github-copilot",
    "kimi",
    "moonshot",
    "stepfun",
    "minimax",
    "alibaba",
    "qwen",
    "qwen-oauth",
    "xai",
    "grok",
    "z-ai",
    "zai",
    "glm",
    "nvidia",
    "nim",
    "bedrock",
    "groq",
    "fireworks",
    "novita",
    "custom",
    "local",
];

/// Strip a leading `provider:` prefix when the prefix is a recognised
/// provider name. Preserves Ollama-style `model:tag` IDs (e.g. `qwen:7b`,
/// `llama3:70b-q4`) — for those the part after `:` is a model variant,
/// not a provider name.
///
/// We use a conservative heuristic: strip only when the prefix is in
/// [`PROVIDER_PREFIXES`] **and** the suffix doesn't match common Ollama
/// tag patterns (a digit + `b` size suffix, `latest`, quantisation tag,
/// or one of `instruct/chat/coder/vision/text`).
fn strip_provider_prefix(model: &str) -> &str {
    if !model.contains(':') || model.starts_with("http") {
        return model;
    }
    let Some((prefix, suffix)) = model.split_once(':') else {
        return model;
    };
    let prefix_lc = prefix.to_ascii_lowercase();
    if !PROVIDER_PREFIXES.contains(&prefix_lc.as_str()) {
        return model;
    }
    if looks_like_ollama_tag(suffix) {
        return model;
    }
    suffix
}

/// Heuristic: does this suffix look like an Ollama model tag rather than
/// the bare model id under a `provider:` prefix?
///
/// Returns `true` for: bare digit-letter size tokens (`7b`, `27b`,
/// `0.5b`), the literals `latest`/`stable`, quantisation prefixes
/// (`q4`, `q4_K_M`, `fp16`), and common variant tags (`instruct`,
/// `chat`, `coder`, `vision`, `text`).
fn looks_like_ollama_tag(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "latest" | "stable" | "instruct" | "chat" | "coder" | "vision" | "text"
    ) {
        return true;
    }
    // size: starts with digit, ends with 'b' (with an optional decimal).
    if lower.ends_with('b') {
        let body = &lower[..lower.len() - 1];
        if !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return true;
        }
    }
    // quantisation: q\d+, fp\d+
    let quant_prefix = lower.starts_with('q')
        && lower
            .chars()
            .nth(1)
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
    let fp_prefix = lower.starts_with("fp")
        && lower
            .chars()
            .nth(2)
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
    quant_prefix || fp_prefix
}

/// Hardcoded substring lookup. Returns the longest matching key's value.
///
/// The match is case-insensitive on the model ID. Substring keys are
/// sorted longest-first at each call (the table is < 20 entries, so the
/// allocation cost is negligible compared to a full lookup pipeline run).
fn lookup_hardcoded(model_id: &str) -> Option<u64> {
    let lower = model_id.to_ascii_lowercase();
    let mut keys: Vec<(&str, u64)> = HARDCODED_FALLBACKS.to_vec();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    for (needle, ctx) in keys {
        if lower.contains(needle) {
            return Some(ctx);
        }
    }
    None
}

/// Build a synthetic `ModelCatalogEntry` for a layer that doesn't have a
/// registry-backed entry to borrow (L1 / L5).
fn synthesize_entry(
    model: &str,
    provider: &str,
    context_window: u64,
    max_output_tokens: u64,
) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: model.to_string(),
        display_name: model.to_string(),
        provider: provider.to_string(),
        tier: ModelTier::Custom,
        modality: Modality::Text,
        context_window,
        max_output_tokens,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        image_input_cost_per_m: None,
        image_output_cost_per_m: None,
        supports_tools: false,
        supports_vision: false,
        supports_streaming: false,
        supports_thinking: false,
        aliases: Vec::new(),
    }
}

/// Whether this provider name is anthropic-shaped — used to pick the
/// 200K vs 32K final default. Matches the bare `"anthropic"` provider
/// plus claude-routed providers like `bedrock` and `vertexai` whose
/// catalog entries are also Claude models with 200K minimum windows.
fn is_anthropic_host(provider: &str, model_id: &str) -> bool {
    let p = provider.to_ascii_lowercase();
    if p == "anthropic" || p == "claude-code" {
        return true;
    }
    // Heuristic on model id: claude-* models served via OpenRouter,
    // bedrock, etc. should still get the anthropic default.
    model_id.to_ascii_lowercase().starts_with("claude")
}

// ===== Layer 3: persisted cache =====

const CACHE_FILE: &str = "cache/model_metadata.json";
const CACHE_TTL_SECS: i64 = 86_400;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default = "default_cache_version")]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, CacheEntry>,
}

fn default_cache_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    context_window: u64,
    #[serde(default)]
    max_output_tokens: u64,
    fetched_at: DateTime<Utc>,
    #[serde(default)]
    source: String,
}

fn cache_key(provider: &str, base_url: Option<&str>, model: &str) -> String {
    format!("{}|{}|{}", provider, base_url.unwrap_or(""), model)
}

fn cache_path(home_dir: &Path) -> PathBuf {
    home_dir.join(CACHE_FILE)
}

fn read_cache_file_blocking(home_dir: &Path) -> CacheFile {
    let path = cache_path(home_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return CacheFile::default();
    };
    match serde_json::from_slice::<CacheFile>(&bytes) {
        Ok(f) if f.version == default_cache_version() => f,
        Ok(_) => {
            tracing::warn!(
                target: "librefang::model_metadata",
                path = %path.display(),
                "model metadata cache version mismatch, ignoring file"
            );
            CacheFile::default()
        }
        Err(e) => {
            tracing::warn!(
                target: "librefang::model_metadata",
                path = %path.display(),
                error = %e,
                "model metadata cache parse failed, ignoring file"
            );
            CacheFile::default()
        }
    }
}

fn write_cache_file_blocking(home_dir: &Path, file: &CacheFile) {
    let path = cache_path(home_dir);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                target: "librefang::model_metadata",
                error = %e,
                "model metadata cache mkdir failed"
            );
            return;
        }
    }
    let tmp = path.with_extension("json.tmp");
    let Ok(bytes) = serde_json::to_vec_pretty(file) else {
        return;
    };
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        tracing::warn!(
            target: "librefang::model_metadata",
            error = %e,
            "model metadata cache write failed"
        );
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        tracing::warn!(
            target: "librefang::model_metadata",
            error = %e,
            "model metadata cache atomic rename failed"
        );
    }
}

async fn read_cache_entry(home_dir: &Path, key: &str) -> Option<CacheEntry> {
    let home = home_dir.to_path_buf();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || {
        let file = read_cache_file_blocking(&home);
        let entry = file.entries.get(&key).cloned()?;
        let age = (Utc::now() - entry.fetched_at).num_seconds();
        if (0..CACHE_TTL_SECS).contains(&age) {
            Some(entry)
        } else {
            None
        }
    })
    .await
    .ok()
    .flatten()
}

async fn write_cache_entry(home_dir: &Path, key: &str, entry: CacheEntry) {
    let home = home_dir.to_path_buf();
    let key = key.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let mut file = read_cache_file_blocking(&home);
        file.entries.insert(key, entry);
        if file.version == 0 {
            file.version = default_cache_version();
        }
        write_cache_file_blocking(&home, &file);
    })
    .await;
}

// ===== Layer 4: runtime probe =====

const PROBE_TIMEOUT_SECS: u64 = 3;

fn probe_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(librefang_http::proxied_client)
}

/// Parse an Ollama `/api/show` response.
///
/// Two source fields can carry context info:
/// - `parameters` — multi-line string; the line `num_ctx <N>` is the
///   *effective* window the server will use.
/// - `model_info` — object whose `*.context_length` keys carry the
///   model's *nominal* maximum.
///
/// Prefer `num_ctx` because it's the actual cap (the model might
/// support 128K but the server is configured for 16K). Fall back to
/// the first `*.context_length` we see.
fn parse_ollama_show(json: &serde_json::Value) -> Option<u64> {
    if let Some(params) = json.get("parameters").and_then(|v| v.as_str()) {
        for line in params.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("num_ctx ") {
                if let Ok(n) = rest.trim().parse::<u64>() {
                    if n > 0 {
                        return Some(n);
                    }
                }
            }
        }
    }
    if let Some(info) = json.get("model_info").and_then(|v| v.as_object()) {
        for (k, v) in info {
            if k.ends_with(".context_length") {
                if let Some(n) = v.as_u64() {
                    if n > 0 {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

async fn probe_ollama(client: &reqwest::Client, base_url: &str, model: &str) -> Option<u64> {
    let url = format!("{}/api/show", base_url.trim_end_matches('/'));
    // Ollama expects `name`; the `model` alias was added more recently
    // and not all server versions accept it. Stick with `name` for
    // compatibility.
    let body = serde_json::json!({ "name": model });
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    parse_ollama_show(&json)
}

fn looks_like_ollama(provider: &str, base_url: Option<&str>) -> bool {
    if provider.eq_ignore_ascii_case("ollama") || provider.eq_ignore_ascii_case("ollama-cloud") {
        return true;
    }
    base_url.map(|u| u.contains(":11434")).unwrap_or(false)
}

/// Parse an Anthropic `/v1/models/{id}` response.
///
/// The official schema returns `context_window` as a top-level integer.
/// Zero is rejected — an Anthropic model with a 0 window is a server
/// bug we'd rather fall back from than cache.
fn parse_anthropic_model(json: &serde_json::Value) -> Option<u64> {
    let n = json.get("context_window").and_then(|v| v.as_u64())?;
    if n > 0 { Some(n) } else { None }
}

/// Probe Anthropic's `GET /v1/models/{model}` endpoint.
///
/// Requires an API key; uses the documented `x-api-key` +
/// `anthropic-version` headers. The model id is URL-segment-safe (no
/// whitespace or slashes for any Claude model), so we splice it into
/// the path directly.
async fn probe_anthropic(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Option<u64> {
    let url = format!("{}/v1/models/{}", base_url.trim_end_matches('/'), model);
    let resp = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    parse_anthropic_model(&json)
}

/// Parse a generic OpenAI-compatible `/v1/models/{id}` response.
///
/// There is no formal spec for the per-model object beyond `id` /
/// `object` / `created` / `owned_by`, so different servers expose the
/// context window under different keys. We try them in priority order:
///
/// 1. `max_model_len` — vLLM canonical.
/// 2. `context_length` — LM Studio, llama.cpp server, common GGUF
///    metadata key.
/// 3. `context_window` — some Anthropic-flavoured proxies.
/// 4. `max_input_tokens` — LiteLLM normalised key.
/// 5. `max_tokens` — last-ditch (some forks conflate this with the
///    full window).
///
/// Zero values are rejected at every step so a misconfigured server
/// can't poison the cache with a useless `0`.
fn parse_openai_model(json: &serde_json::Value) -> Option<u64> {
    const KEYS: &[&str] = &[
        "max_model_len",
        "context_length",
        "context_window",
        "max_input_tokens",
        "max_tokens",
    ];
    for key in KEYS {
        if let Some(n) = json.get(*key).and_then(|v| v.as_u64()) {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}

/// Probe a generic OpenAI-compatible `GET /v1/models/{model}` endpoint.
///
/// No auth header is set — most self-hosted servers (vLLM, LM Studio,
/// llama.cpp) don't require one for the models endpoint, and forcing
/// an `Authorization` header without a configured token would cause
/// gateways like LiteLLM to 401 on what should be an open route.
/// Callers that need bearer auth should rely on the registry (L2) or
/// extend this branch in a follow-up.
async fn probe_openai_compat(
    client: &reqwest::Client,
    base_url: &str,
    model: &str,
) -> Option<u64> {
    let url = format!("{}/v1/models/{}", base_url.trim_end_matches('/'), model);
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    parse_openai_model(&json)
}

/// L4 dispatcher. Order matters: Ollama is identified first by provider
/// tag or `:11434` heuristic, Anthropic by provider name (it requires
/// an API key, so an empty key short-circuits to `None`), and any other
/// caller-supplied `base_url` falls into the generic OpenAI-compat path.
/// Without a `base_url` and without a known provider tag, the probe is
/// skipped — there's nowhere to send a request.
async fn probe_runtime(request: &MetadataRequest<'_>) -> Option<u64> {
    if looks_like_ollama(request.provider, request.base_url) {
        let base = request.base_url.unwrap_or("http://localhost:11434");
        return probe_ollama(probe_client(), base, request.model).await;
    }
    if request.provider.eq_ignore_ascii_case("anthropic") {
        let base = request.base_url.unwrap_or("https://api.anthropic.com");
        let api_key = request.api_key?;
        return probe_anthropic(probe_client(), base, api_key, request.model).await;
    }
    // Generic OpenAI-compat — only when caller provided base_url. We
    // don't probe the public OpenAI / Groq / Anthropic endpoints by
    // default because those are already covered by the registry (L2).
    if let Some(base) = request.base_url {
        return probe_openai_compat(probe_client(), base, request.model).await;
    }
    None
}

/// Resolve metadata for a model through the layered fallback pipeline.
///
/// Layers 1, 2, 3, 4, 5 in order. Always returns a populated
/// [`ResolvedModel`]; the worst case is a `Default32k` synthesised entry.
/// Callers can therefore treat the `Option<usize>` problem as solved at
/// this boundary.
pub async fn resolve_model_metadata<'a>(
    catalog: &'a ModelCatalog,
    home_dir: &Path,
    request: &MetadataRequest<'_>,
) -> ResolvedModel<'a> {
    // ----- Layer 1: agent manifest override -----
    if let Some(ctx) = request.manifest_override_context.filter(|v| *v > 0) {
        let max_out = request.manifest_override_max_output.unwrap_or(0);
        let entry = synthesize_entry(request.model, request.provider, ctx, max_out);
        return ResolvedModel {
            entry: Cow::Owned(entry),
            source: MetadataSource::AgentManifest,
        };
    }

    // ----- Layer 2: provider-aware registry lookup -----
    let stripped = strip_provider_prefix(request.model);
    if let Some(entry) = catalog.find_model_for_provider(request.provider, stripped) {
        if entry.context_window > 0 {
            return ResolvedModel {
                entry: Cow::Borrowed(entry),
                source: MetadataSource::Registry,
            };
        }
    }
    // Fall back to provider-blind lookup: same model under any provider.
    // Useful when the agent's `provider` is empty or stale relative to
    // the registry layout (registry providers may rename across syncs).
    if let Some(entry) = catalog.find_model(stripped) {
        if entry.context_window > 0 {
            return ResolvedModel {
                entry: Cow::Borrowed(entry),
                source: MetadataSource::Registry,
            };
        }
    }

    // ----- Layer 3: persisted cache -----
    let key = cache_key(request.provider, request.base_url, request.model);
    if let Some(cached) = read_cache_entry(home_dir, &key).await {
        if cached.context_window > 0 {
            let entry = synthesize_entry(
                request.model,
                request.provider,
                cached.context_window,
                cached.max_output_tokens,
            );
            return ResolvedModel {
                entry: Cow::Owned(entry),
                source: MetadataSource::PersistedCache,
            };
        }
    }

    // ----- Layer 4: live probe (Ollama in PR-2) -----
    if let Some(ctx) = probe_runtime(request).await {
        // Best-effort write — losing the cache write is preferable to
        // blocking the agent on disk IO.
        write_cache_entry(
            home_dir,
            &key,
            CacheEntry {
                context_window: ctx,
                max_output_tokens: 0,
                fetched_at: Utc::now(),
                source: MetadataSource::RuntimeProbe.as_str().to_string(),
            },
        )
        .await;
        let entry = synthesize_entry(request.model, request.provider, ctx, 0);
        return ResolvedModel {
            entry: Cow::Owned(entry),
            source: MetadataSource::RuntimeProbe,
        };
    }

    // ----- Layer 5: hardcoded substring table + provider default -----
    if let Some(ctx) = lookup_hardcoded(stripped) {
        let entry = synthesize_entry(request.model, request.provider, ctx, 0);
        return ResolvedModel {
            entry: Cow::Owned(entry),
            source: MetadataSource::HardcodedFallback,
        };
    }

    // Final default: anthropic-shaped → 200K, anything else → 32K.
    if is_anthropic_host(request.provider, stripped) {
        let entry = synthesize_entry(
            request.model,
            request.provider,
            DEFAULT_ANTHROPIC_CONTEXT,
            0,
        );
        return ResolvedModel {
            entry: Cow::Owned(entry),
            source: MetadataSource::Default200kAnthropic,
        };
    }
    let entry = synthesize_entry(request.model, request.provider, DEFAULT_GENERIC_CONTEXT, 0);
    ResolvedModel {
        entry: Cow::Owned(entry),
        source: MetadataSource::Default32k,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::model_catalog::{Modality, ModelCatalogEntry, ModelTier};

    /// Build a minimal in-memory catalog with the given entries; bypasses
    /// the TOML loader so unit tests don't need fixtures on disk.
    fn catalog_with(entries: Vec<ModelCatalogEntry>) -> ModelCatalog {
        ModelCatalog::from_entries(entries, vec![])
    }

    fn entry(provider: &str, id: &str, context_window: u64) -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: id.to_string(),
            display_name: id.to_string(),
            provider: provider.to_string(),
            tier: ModelTier::Balanced,
            modality: Modality::Text,
            context_window,
            max_output_tokens: 4096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            image_input_cost_per_m: None,
            image_output_cost_per_m: None,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: false,
            supports_thinking: false,
            aliases: vec![],
        }
    }

    fn req<'a>(provider: &'a str, model: &'a str) -> MetadataRequest<'a> {
        MetadataRequest {
            provider,
            model,
            base_url: None,
            api_key: None,
            manifest_override_context: None,
            manifest_override_max_output: None,
        }
    }

    /// Tempdir-backed `home_dir` used by every async test. The
    /// `TempDir` is owned by the caller and gets cleaned up at test
    /// end via Drop, so cache writes never bleed across tests.
    fn fresh_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir for cache home")
    }

    #[tokio::test]
    async fn layer_1_manifest_override_wins() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let mut request = req("anthropic", "claude-opus-4-7");
        request.manifest_override_context = Some(196_608);
        let resolved = resolve_model_metadata(&cat, home, &request).await;
        assert_eq!(resolved.source, MetadataSource::AgentManifest);
        assert_eq!(resolved.entry.context_window, 196_608);
    }

    #[tokio::test]
    async fn layer_1_zero_override_skipped() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let mut request = req("anthropic", "claude-opus-4-7");
        // 0 must be treated as "unset" — falling through to L2.
        request.manifest_override_context = Some(0);
        let resolved = resolve_model_metadata(&cat, home, &request).await;
        assert_eq!(resolved.source, MetadataSource::Registry);
        assert_eq!(resolved.entry.context_window, 1_000_000);
    }

    #[tokio::test]
    async fn layer_2_provider_aware_disambiguates() {
        let _home = fresh_home();
        let home = _home.path();
        // Same id under two providers with different windows.
        let cat = catalog_with(vec![
            entry("anthropic", "claude-opus-4-7", 1_000_000),
            entry("copilot", "claude-opus-4-7", 128_000),
        ]);
        let r_anthropic = resolve_model_metadata(&cat, home, &req("anthropic", "claude-opus-4-7")).await;
        assert_eq!(r_anthropic.entry.context_window, 1_000_000);
        let r_copilot = resolve_model_metadata(&cat, home, &req("copilot", "claude-opus-4-7")).await;
        assert_eq!(r_copilot.entry.context_window, 128_000);
    }

    #[tokio::test]
    async fn layer_2_zero_context_falls_through_to_l5() {
        let _home = fresh_home();
        let home = _home.path();
        // Catalog has the entry but its context_window is 0 (e.g. an
        // Ollama-discovered model that hasn't been probed yet). L2 must
        // skip it — registry data with 0 is "unknown", not "zero tokens".
        let cat = catalog_with(vec![entry("ollama", "qwen3-coder:30b", 0)]);
        let resolved = resolve_model_metadata(&cat, home, &req("ollama", "qwen3-coder:30b")).await;
        // Hardcoded substring table picks up "qwen3-coder" → 262144.
        assert_eq!(resolved.source, MetadataSource::HardcodedFallback);
        assert_eq!(resolved.entry.context_window, 262_144);
    }

    #[tokio::test]
    async fn layer_5_hardcoded_substring_longest_key_wins() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![]);
        // "claude-opus-4-6" must beat the more permissive "claude" key.
        let r1 = resolve_model_metadata(&cat, home, &req("anthropic", "claude-opus-4-6")).await;
        assert_eq!(r1.source, MetadataSource::HardcodedFallback);
        assert_eq!(r1.entry.context_window, 1_000_000);

        // "claude-haiku-4-5" beats bare "claude" (200K both, but the
        // longest-key precedence is what guarantees the haiku-specific
        // entry takes effect when its number ever diverges).
        let r2 = resolve_model_metadata(&cat, home, &req("anthropic", "claude-haiku-4-5")).await;
        assert_eq!(r2.source, MetadataSource::HardcodedFallback);

        // Bare "claude-3-5-sonnet" not in the table → falls to "claude"
        // catch-all (200K).
        let r3 = resolve_model_metadata(&cat, home, &req("anthropic", "claude-3-5-sonnet")).await;
        assert_eq!(r3.source, MetadataSource::HardcodedFallback);
        assert_eq!(r3.entry.context_window, 200_000);
    }

    #[tokio::test]
    async fn layer_5_anthropic_default_for_unknown_claude() {
        let _home = fresh_home();
        let home = _home.path();
        // Model id contains "claude" → the substring table catches it,
        // not the Default200kAnthropic tail. To reach the tail we need
        // a model id outside the table but a provider that's anthropic.
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, home, &req("anthropic", "totally-unknown-model")).await;
        assert_eq!(r.source, MetadataSource::Default200kAnthropic);
        assert_eq!(r.entry.context_window, 200_000);
    }

    #[tokio::test]
    async fn layer_5_generic_default_for_unknown_non_anthropic() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, home, &req("custom", "totally-unknown-model")).await;
        assert_eq!(r.source, MetadataSource::Default32k);
        assert_eq!(r.entry.context_window, 32_768);
    }

    #[test]
    fn provider_prefix_stripped_for_known_providers() {
        assert_eq!(
            strip_provider_prefix("openrouter:claude-opus-4-7"),
            "claude-opus-4-7"
        );
        assert_eq!(
            strip_provider_prefix("anthropic:claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix("local:my-llama"), "my-llama");
    }

    #[test]
    fn provider_prefix_preserved_for_ollama_tags() {
        // Bare model:size form must NOT be stripped (the 7b is the tag,
        // not a model id under `qwen:` provider).
        assert_eq!(strip_provider_prefix("qwen:7b"), "qwen:7b");
        assert_eq!(strip_provider_prefix("llama:0.5b"), "llama:0.5b");
        assert_eq!(strip_provider_prefix("qwen:latest"), "qwen:latest");
        assert_eq!(
            strip_provider_prefix("llama3:70b-q4_K_M"),
            "llama3:70b-q4_K_M"
        );
        assert_eq!(strip_provider_prefix("qwen:q4"), "qwen:q4");
        assert_eq!(strip_provider_prefix("mistral:fp16"), "mistral:fp16");
        assert_eq!(strip_provider_prefix("llama2:instruct"), "llama2:instruct");
    }

    #[test]
    fn provider_prefix_unknown_namespace_preserved() {
        // `myorg:custom` — myorg isn't in PROVIDER_PREFIXES, so stripping
        // would drop the namespace and let "custom" leak through.
        assert_eq!(strip_provider_prefix("myorg:custom"), "myorg:custom");
        // URLs are also left alone (caller may pass full base_url-style
        // identifiers in some flows).
        assert_eq!(
            strip_provider_prefix("https://example.com/models/foo"),
            "https://example.com/models/foo",
        );
    }

    #[tokio::test]
    async fn provider_aware_lookup_with_prefix_in_request() {
        let _home = fresh_home();
        let home = _home.path();
        // Request carries `openrouter:claude-opus-4-7` but the catalog
        // entry is keyed on the bare id.
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let r = resolve_model_metadata(&cat, home, &req("anthropic", "openrouter:claude-opus-4-7")).await;
        assert_eq!(r.source, MetadataSource::Registry);
        assert_eq!(r.entry.context_window, 1_000_000);
    }

    #[tokio::test]
    async fn empty_provider_falls_back_to_unscoped_lookup() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let r = resolve_model_metadata(&cat, home, &req("", "claude-opus-4-7")).await;
        assert_eq!(r.source, MetadataSource::Registry);
        assert_eq!(r.entry.context_window, 1_000_000);
    }

    #[test]
    fn metadata_source_str_round_trip() {
        for s in [
            MetadataSource::AgentManifest,
            MetadataSource::Registry,
            MetadataSource::PersistedCache,
            MetadataSource::RuntimeProbe,
            MetadataSource::HardcodedFallback,
            MetadataSource::Default200kAnthropic,
            MetadataSource::Default32k,
        ] {
            assert!(!s.as_str().is_empty());
        }
    }

    /// Defence-in-depth: providers also gets dropped into the synthesised
    /// fallback entry so the kernel can later log `provider=...` even
    /// when the catalog miss synthesised the result.
    #[tokio::test]
    async fn fallback_entry_carries_request_provider() {
        let _home = fresh_home();
        let home = _home.path();
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, home, &req("ollama", "totally-unknown")).await;
        assert_eq!(r.entry.provider, "ollama");
        assert_eq!(r.entry.id, "totally-unknown");
    }

    // ---- PR-2 tests: persisted cache + Ollama parser ----

    /// Writing a cache entry then resolving the same key picks it up at
    /// L3 (PersistedCache). The catalog is empty and the model isn't in
    /// the hardcoded table, so without the cache we'd fall to L5
    /// `Default32k`.
    #[tokio::test]
    async fn layer_3_cache_round_trip() {
        let _home = fresh_home();
        let home = _home.path();
        let key = cache_key("ollama", Some("http://localhost:11434"), "qwen3-foo:30b");
        write_cache_entry(
            home,
            &key,
            CacheEntry {
                context_window: 65_536,
                max_output_tokens: 0,
                fetched_at: Utc::now(),
                source: "runtime_probe".to_string(),
            },
        )
        .await;

        let cat = catalog_with(vec![]);
        let mut request = req("ollama", "qwen3-foo:30b");
        request.base_url = Some("http://localhost:11434");
        let r = resolve_model_metadata(&cat, home, &request).await;
        assert_eq!(r.source, MetadataSource::PersistedCache);
        assert_eq!(r.entry.context_window, 65_536);
    }

    /// A cache entry whose `fetched_at` is older than the TTL must not
    /// be returned. Falling through here lands on the hardcoded `qwen`
    /// 131K entry (the model id contains "qwen3-foo" → matches "qwen").
    #[tokio::test]
    async fn layer_3_stale_cache_falls_through() {
        let _home = fresh_home();
        let home = _home.path();
        let key = cache_key("ollama", Some("http://localhost:11434"), "qwen3-foo:30b");
        // Entry fetched 25h ago — past the 24h TTL.
        let stale_at = Utc::now() - chrono::Duration::seconds(CACHE_TTL_SECS + 3_600);
        write_cache_entry(
            home,
            &key,
            CacheEntry {
                context_window: 65_536,
                max_output_tokens: 0,
                fetched_at: stale_at,
                source: "runtime_probe".to_string(),
            },
        )
        .await;

        let cat = catalog_with(vec![]);
        let mut request = req("ollama", "qwen3-foo:30b");
        request.base_url = Some("http://localhost:11434");
        let r = resolve_model_metadata(&cat, home, &request).await;
        // Expired cache + L4 unreachable in test env (no Ollama running)
        // → falls to L5 hardcoded "qwen" → 131_072.
        assert_ne!(r.source, MetadataSource::PersistedCache);
        assert_eq!(r.entry.context_window, 131_072);
    }

    /// A corrupted cache file must not block startup or panic the
    /// pipeline. The reader logs a warning and returns an empty cache.
    #[tokio::test]
    async fn layer_3_corrupted_file_does_not_panic() {
        let _home = fresh_home();
        let home = _home.path();
        // Hand-write garbage to the cache path.
        let path = cache_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not even close to JSON {{{").unwrap();

        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, home, &req("custom", "totally-unknown")).await;
        // Expect a clean fallback to L5 generic 32K, not a panic.
        assert_eq!(r.source, MetadataSource::Default32k);
    }

    /// Parser: `parameters` carrying `num_ctx 32768` wins over a
    /// nominal `*.context_length` further down — we want the
    /// effective server cap, not the model's theoretical max.
    #[test]
    fn parse_ollama_show_prefers_num_ctx() {
        let json = serde_json::json!({
            "parameters": "stop \"<eos>\"\nnum_ctx 32768\ntemperature 0.7\n",
            "model_info": { "qwen3.context_length": 262144u64 }
        });
        assert_eq!(parse_ollama_show(&json), Some(32_768));
    }

    /// Parser: when `parameters` lacks `num_ctx`, fall back to the first
    /// `*.context_length` we find in `model_info`.
    #[test]
    fn parse_ollama_show_falls_back_to_model_info() {
        let json = serde_json::json!({
            "parameters": "stop \"<eos>\"\ntemperature 0.7\n",
            "model_info": { "llama.context_length": 8192u64 }
        });
        assert_eq!(parse_ollama_show(&json), Some(8_192));
    }

    /// Parser: empty / missing fields → `None`. The probe layer treats
    /// `None` as "no value found", falls through to L5 instead of
    /// caching a misleading zero.
    #[test]
    fn parse_ollama_show_returns_none_on_missing_fields() {
        assert_eq!(parse_ollama_show(&serde_json::json!({})), None);
        assert_eq!(
            parse_ollama_show(&serde_json::json!({ "parameters": "stop \"<eos>\"\n" })),
            None,
        );
        assert_eq!(
            parse_ollama_show(&serde_json::json!({ "model_info": { "foo": "bar" } })),
            None,
        );
    }

    /// Parser: zero values in either field are rejected (a server with
    /// `num_ctx 0` is a misconfiguration; we'd rather fall back than
    /// cache zero and break downstream budget math).
    #[test]
    fn parse_ollama_show_rejects_zero() {
        assert_eq!(
            parse_ollama_show(&serde_json::json!({
                "parameters": "num_ctx 0\n",
                "model_info": {}
            })),
            None,
        );
        assert_eq!(
            parse_ollama_show(&serde_json::json!({
                "model_info": { "x.context_length": 0u64 }
            })),
            None,
        );
    }

    /// `looks_like_ollama` matches both the literal provider tag and a
    /// generic endpoint hosted on the canonical 11434 port.
    #[test]
    fn ollama_detection_provider_or_port() {
        assert!(looks_like_ollama("ollama", None));
        assert!(looks_like_ollama("OLLAMA", None));
        assert!(looks_like_ollama("ollama-cloud", None));
        assert!(looks_like_ollama("custom", Some("http://10.0.0.5:11434")));
        assert!(!looks_like_ollama("custom", Some("https://api.example.com")));
        assert!(!looks_like_ollama("anthropic", None));
    }

    // ---- PR-2.5 tests: Anthropic + OpenAI-compat parsers ----

    /// Parser: standard Anthropic `/v1/models/{id}` response carries
    /// `context_window` at the top level.
    #[test]
    fn parse_anthropic_model_extracts_context_window() {
        let json = serde_json::json!({
            "id": "claude-opus-4-7",
            "type": "model",
            "display_name": "Claude Opus 4.7",
            "context_window": 1_000_000u64,
            "max_output_tokens": 64_000u64
        });
        assert_eq!(parse_anthropic_model(&json), Some(1_000_000));
    }

    /// Parser: missing `context_window` → `None` (probe layer treats
    /// this as "no value", falls through to L5).
    #[test]
    fn parse_anthropic_model_missing_field_returns_none() {
        let json = serde_json::json!({
            "id": "claude-opus-4-7",
            "type": "model"
        });
        assert_eq!(parse_anthropic_model(&json), None);
    }

    /// Parser: a server returning `context_window: 0` is broken; reject
    /// it rather than caching the zero.
    #[test]
    fn parse_anthropic_model_rejects_zero() {
        let json = serde_json::json!({ "context_window": 0u64 });
        assert_eq!(parse_anthropic_model(&json), None);
    }

    /// Parser: vLLM's canonical key wins over LM Studio's
    /// `context_length` when both are present. vLLM is authoritative
    /// for what the server will actually accept.
    #[test]
    fn parse_openai_model_prefers_max_model_len() {
        let json = serde_json::json!({
            "id": "qwen3-coder-30b",
            "max_model_len": 32_768u64,
            "context_length": 16_384u64
        });
        assert_eq!(parse_openai_model(&json), Some(32_768));
    }

    /// Parser: LM Studio / llama.cpp servers expose `context_length`
    /// when `max_model_len` is absent.
    #[test]
    fn parse_openai_model_falls_back_to_context_length() {
        let json = serde_json::json!({
            "id": "qwen3-coder-30b",
            "context_length": 16_384u64
        });
        assert_eq!(parse_openai_model(&json), Some(16_384));
    }

    /// Parser: some Anthropic-flavoured proxies expose `context_window`
    /// instead of `context_length`.
    #[test]
    fn parse_openai_model_falls_back_to_context_window() {
        let json = serde_json::json!({
            "id": "claude-via-proxy",
            "context_window": 200_000u64
        });
        assert_eq!(parse_openai_model(&json), Some(200_000));
    }

    /// Parser: LiteLLM exposes `max_input_tokens` as its normalised
    /// model-info key.
    #[test]
    fn parse_openai_model_falls_back_to_max_input_tokens() {
        let json = serde_json::json!({
            "id": "groq-llama",
            "max_input_tokens": 131_072u64
        });
        assert_eq!(parse_openai_model(&json), Some(131_072));
    }

    /// Parser: last-ditch fallback to `max_tokens` when no other key
    /// is set. Some forks conflate this with the full window.
    #[test]
    fn parse_openai_model_falls_back_to_max_tokens() {
        let json = serde_json::json!({
            "id": "obscure-model",
            "max_tokens": 8_192u64
        });
        assert_eq!(parse_openai_model(&json), Some(8_192));
    }

    /// Parser: object with none of the recognised keys → `None`.
    #[test]
    fn parse_openai_model_returns_none_on_no_recognised_keys() {
        let json = serde_json::json!({
            "id": "obscure-model",
            "object": "model",
            "owned_by": "someone"
        });
        assert_eq!(parse_openai_model(&json), None);
    }

    /// Parser: every recognised key set to 0 must be skipped, not
    /// returned. We'd rather fall through to the next layer than cache
    /// a bogus zero.
    #[test]
    fn parse_openai_model_rejects_zero() {
        let json = serde_json::json!({
            "max_model_len": 0u64,
            "context_length": 0u64,
            "context_window": 0u64,
            "max_input_tokens": 0u64,
            "max_tokens": 0u64
        });
        assert_eq!(parse_openai_model(&json), None);
    }

    /// Cache key composition: `provider|base_url|model` triple keeps
    /// same-id models on different endpoints from sharing entries.
    #[test]
    fn cache_key_separates_endpoint_namespaces() {
        assert_ne!(
            cache_key("ollama", Some("http://host-a:11434"), "qwen:7b"),
            cache_key("ollama", Some("http://host-b:11434"), "qwen:7b"),
        );
        assert_eq!(
            cache_key("ollama", None, "qwen:7b"),
            cache_key("ollama", Some(""), "qwen:7b"),
        );
    }
}
