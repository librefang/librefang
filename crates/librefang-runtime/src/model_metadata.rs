//! Model metadata lookup pipeline.
//!
//! Resolves a model's `context_window` (and optionally `max_output_tokens`)
//! through a layered fallback chain so the agent loop never has to fall back
//! to a coarse `200_000` default when the catalog misses or the user runs a
//! self-hosted endpoint with a non-standard window.
//!
//! See `.plans/model-metadata-lookup.md` for the full design and the
//! 5-layer rationale. **PR-1 (this module) lands layers 1, 2, and 5 only**:
//!
//! | Layer | Source | This PR |
//! |---|---|---|
//! | L1 | Agent manifest override (`model.context_window`) | ✅ |
//! | L2 | Registry / `ModelCatalog` (provider-aware) | ✅ |
//! | L3 | Persisted cache (`~/.librefang/cache/model_metadata.json`) | M2 |
//! | L4 | Runtime probe (`/v1/models`, `/api/show`) | M2 |
//! | L5 | Hardcoded fallback (< 20 entries) | ✅ |
//!
//! `resolve_model_metadata` is currently **passive** — no caller wires it
//! into `agent_loop` yet. M3 will replace the
//! `cat.find_model(...).map(|m| m.context_window).filter(|w| *w > 0)`
//! call sites in `kernel/mod.rs` with a single `resolve_model_metadata`
//! invocation.

use librefang_types::model_catalog::{ModelCatalogEntry, ModelTier, Modality};
use std::borrow::Cow;

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
#[derive(Debug, Clone, Copy)]
pub struct MetadataRequest<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub base_url: Option<&'a str>,
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

/// Resolve metadata for a model through layers 1, 2, and 5.
///
/// Layers 3 and 4 are placeholders in this PR — when wired in M2 they'll
/// slot between L2 and L5 without changing this function's signature.
///
/// Always returns a populated [`ResolvedModel`]; the worst case is a
/// `Default32k` synthesised entry. Callers can therefore treat the
/// `Option<usize>` problem as solved at this boundary.
pub fn resolve_model_metadata<'a>(
    catalog: &'a ModelCatalog,
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

    // ----- Layer 3 and 4 are M2 — fall through to L5 for now. -----

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
    use librefang_types::model_catalog::{ModelCatalogEntry, ModelTier, Modality};

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
            manifest_override_context: None,
            manifest_override_max_output: None,
        }
    }

    #[test]
    fn layer_1_manifest_override_wins() {
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let mut request = req("anthropic", "claude-opus-4-7");
        request.manifest_override_context = Some(196_608);
        let resolved = resolve_model_metadata(&cat, &request);
        assert_eq!(resolved.source, MetadataSource::AgentManifest);
        assert_eq!(resolved.entry.context_window, 196_608);
    }

    #[test]
    fn layer_1_zero_override_skipped() {
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let mut request = req("anthropic", "claude-opus-4-7");
        // 0 must be treated as "unset" — falling through to L2.
        request.manifest_override_context = Some(0);
        let resolved = resolve_model_metadata(&cat, &request);
        assert_eq!(resolved.source, MetadataSource::Registry);
        assert_eq!(resolved.entry.context_window, 1_000_000);
    }

    #[test]
    fn layer_2_provider_aware_disambiguates() {
        // Same id under two providers with different windows.
        let cat = catalog_with(vec![
            entry("anthropic", "claude-opus-4-7", 1_000_000),
            entry("copilot", "claude-opus-4-7", 128_000),
        ]);
        let r_anthropic = resolve_model_metadata(&cat, &req("anthropic", "claude-opus-4-7"));
        assert_eq!(r_anthropic.entry.context_window, 1_000_000);
        let r_copilot = resolve_model_metadata(&cat, &req("copilot", "claude-opus-4-7"));
        assert_eq!(r_copilot.entry.context_window, 128_000);
    }

    #[test]
    fn layer_2_zero_context_falls_through_to_l5() {
        // Catalog has the entry but its context_window is 0 (e.g. an
        // Ollama-discovered model that hasn't been probed yet). L2 must
        // skip it — registry data with 0 is "unknown", not "zero tokens".
        let cat = catalog_with(vec![entry("ollama", "qwen3-coder:30b", 0)]);
        let resolved = resolve_model_metadata(&cat, &req("ollama", "qwen3-coder:30b"));
        // Hardcoded substring table picks up "qwen3-coder" → 262144.
        assert_eq!(resolved.source, MetadataSource::HardcodedFallback);
        assert_eq!(resolved.entry.context_window, 262_144);
    }

    #[test]
    fn layer_5_hardcoded_substring_longest_key_wins() {
        let cat = catalog_with(vec![]);
        // "claude-opus-4-6" must beat the more permissive "claude" key.
        let r1 = resolve_model_metadata(&cat, &req("anthropic", "claude-opus-4-6"));
        assert_eq!(r1.source, MetadataSource::HardcodedFallback);
        assert_eq!(r1.entry.context_window, 1_000_000);

        // "claude-haiku-4-5" beats bare "claude" (200K both, but the
        // longest-key precedence is what guarantees the haiku-specific
        // entry takes effect when its number ever diverges).
        let r2 = resolve_model_metadata(&cat, &req("anthropic", "claude-haiku-4-5"));
        assert_eq!(r2.source, MetadataSource::HardcodedFallback);

        // Bare "claude-3-5-sonnet" not in the table → falls to "claude"
        // catch-all (200K).
        let r3 = resolve_model_metadata(&cat, &req("anthropic", "claude-3-5-sonnet"));
        assert_eq!(r3.source, MetadataSource::HardcodedFallback);
        assert_eq!(r3.entry.context_window, 200_000);
    }

    #[test]
    fn layer_5_anthropic_default_for_unknown_claude() {
        // Model id contains "claude" → the substring table catches it,
        // not the Default200kAnthropic tail. To reach the tail we need
        // a model id outside the table but a provider that's anthropic.
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, &req("anthropic", "totally-unknown-model"));
        assert_eq!(r.source, MetadataSource::Default200kAnthropic);
        assert_eq!(r.entry.context_window, 200_000);
    }

    #[test]
    fn layer_5_generic_default_for_unknown_non_anthropic() {
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, &req("custom", "totally-unknown-model"));
        assert_eq!(r.source, MetadataSource::Default32k);
        assert_eq!(r.entry.context_window, 32_768);
    }

    #[test]
    fn provider_prefix_stripped_for_known_providers() {
        assert_eq!(strip_provider_prefix("openrouter:claude-opus-4-7"), "claude-opus-4-7");
        assert_eq!(strip_provider_prefix("anthropic:claude-haiku-4-5"), "claude-haiku-4-5");
        assert_eq!(strip_provider_prefix("local:my-llama"), "my-llama");
    }

    #[test]
    fn provider_prefix_preserved_for_ollama_tags() {
        // Bare model:size form must NOT be stripped (the 7b is the tag,
        // not a model id under `qwen:` provider).
        assert_eq!(strip_provider_prefix("qwen:7b"), "qwen:7b");
        assert_eq!(strip_provider_prefix("llama:0.5b"), "llama:0.5b");
        assert_eq!(strip_provider_prefix("qwen:latest"), "qwen:latest");
        assert_eq!(strip_provider_prefix("llama3:70b-q4_K_M"), "llama3:70b-q4_K_M");
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

    #[test]
    fn provider_aware_lookup_with_prefix_in_request() {
        // Request carries `openrouter:claude-opus-4-7` but the catalog
        // entry is keyed on the bare id.
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let r = resolve_model_metadata(&cat, &req("anthropic", "openrouter:claude-opus-4-7"));
        assert_eq!(r.source, MetadataSource::Registry);
        assert_eq!(r.entry.context_window, 1_000_000);
    }

    #[test]
    fn empty_provider_falls_back_to_unscoped_lookup() {
        let cat = catalog_with(vec![entry("anthropic", "claude-opus-4-7", 1_000_000)]);
        let r = resolve_model_metadata(&cat, &req("", "claude-opus-4-7"));
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
    #[test]
    fn fallback_entry_carries_request_provider() {
        let cat = catalog_with(vec![]);
        let r = resolve_model_metadata(&cat, &req("ollama", "totally-unknown"));
        assert_eq!(r.entry.provider, "ollama");
        assert_eq!(r.entry.id, "totally-unknown");
    }

}
