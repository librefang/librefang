//! `librefang models connect everyapi` — register the EveryAPI gateway as a
//! custom LLM provider.
//!
//! EveryAPI is a separate AI-API-gateway product. Once its CLI has run
//! `everyapi login`, it stores a relay credential at
//! `~/.config/everyapi/credentials.json` (respecting `$XDG_CONFIG_HOME`).
//! This command turns that credential into a LibreFang provider entry:
//!
//! 1. read `api_base` + `relay_key` from the credentials file;
//! 2. ask the gateway for its live `/v1/models` list;
//! 3. ask the gateway for its own `/api/pricing` feed — the gateway's book,
//!    not an upstream vendor's — and convert its billing ratios into
//!    per-million-token USD figures;
//! 4. synthesise a `providers/everyapi.toml` catalog file; where the gateway
//!    publishes no context/output token limit at all, borrow *only those two
//!    numbers* from the compiled-in OpenRouter snapshot and say so in the
//!    report;
//! 5. persist the relay key to `~/.librefang/.env` as `EVERYAPI_API_KEY`.
//!
//! No `librefang-llm-drivers` change is needed: `create_driver()` already
//! falls back to the OpenAI-compatible driver for any provider that carries a
//! `base_url` but is absent from `PROVIDER_REGISTRY`.
//!
//! The relay key is never printed, logged, or formatted into any message —
//! it only ever travels from the credentials file to `save_env_key` and into
//! the outbound `Authorization` header.

use crate::commands::prelude::*;
use librefang_types::model_catalog::{
    Modality, ModelCatalogEntry, ModelCatalogFile, ModelTier, ProviderCatalogToml,
};

/// The only `models connect` target implemented today.
const EVERYAPI_TARGET: &str = "everyapi";
/// Provider id written into `providers/everyapi.toml`.
const PROVIDER_ID: &str = "everyapi";
/// Display name shown in `librefang models providers`.
const PROVIDER_DISPLAY_NAME: &str = "EveryAPI";
/// Env var the OpenAI-compatible driver reads the gateway bearer from.
///
/// Deliberately NOT `EVERYAPI_RELAY_KEY` — that is EveryAPI's own script-facing variable name, and conflating the two would make `librefang models providers` disagree with `provider_to_env_var` (`commands/common.rs`), whose `other => "{OTHER}_API_KEY"` fallback already produces exactly this value for the id `everyapi`.
const API_KEY_ENV: &str = "EVERYAPI_API_KEY";
/// How long to wait for the gateway's model list before giving up and writing the provider entry with no `[[models]]`.
/// The same budget applies to the pricing feed, which is fetched from the same host.
const MODELS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Ratio → USD conversion factor for the gateway's billing ratios.
///
/// EveryAPI's backend meters in "quota" units with `QuotaPerUnit = 500_000` quota per USD, and charges `tokens * model_ratio` quota — so a ratio of 1 is `1_000_000 / 500_000` = $2.00 per million input tokens.
/// The gateway's own docs state the same rule ("upstream price per 1M = model_ratio x $2"), so the two independent sources agree.
///
/// The daemon-side TTL refresh in `librefang-api/src/everyapi_catalog.rs` carries an identical constant and conversion, because the two crates do not depend on each other in a direction that would let one reuse the other.
/// Both sides assert the same `claude-sonnet-5` => 2.00 / 10.00 value, so editing this rule on one side alone fails the other side's tests.
const RATIO_USD_PER_MILLION: f64 = 2.0;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// The two fields this command needs out of EveryAPI's credentials file.
///
/// The on-disk file also carries `access_token`, `role`, `user_id` and `username`; none of them are read here.
/// `relay_key` is the gateway bearer and is treated as a secret throughout — it is never rendered.
pub(crate) struct EveryApiCredentials {
    /// Gateway root, e.g.
    /// `https://api.everyapi.ai` (no `/v1` suffix).
    pub(crate) api_base: String,
    /// Gateway bearer credential.
    /// Never printed.
    pub(crate) relay_key: String,
}

/// Parse the credentials JSON.
///
/// `serde_json::Value` accessors only: `librefang-cli` has no `serde` dependency, so a `#[derive(Deserialize)]` struct is not available here.
/// Unknown extra fields are ignored by construction, which keeps this forward-compatible with whatever EveryAPI adds to the file next.
///
/// The `Err` string is an i18n *key*, not user-facing prose — the caller renders it.
/// Returning a key rather than a message keeps this function pure and unit-testable without a loaded locale bundle.
pub(crate) fn parse_credentials(raw: &str) -> Result<EveryApiCredentials, &'static str> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "everyapi-connect-credentials-malformed")?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let api_base = field("api_base").ok_or("everyapi-connect-credentials-no-api-base")?;
    let relay_key = field("relay_key").ok_or("everyapi-connect-credentials-no-relay-key")?;
    Ok(EveryApiCredentials {
        api_base: api_base.to_string(),
        relay_key: relay_key.to_string(),
    })
}

/// `$XDG_CONFIG_HOME/everyapi/credentials.json`, falling back to `~/.config/everyapi/credentials.json`.
///
/// Mirrors `doctor::everyapi_credentials_path`, which is private to that module (the doctor check and this command must agree on the location, so any change here needs the same change there).
fn credentials_path() -> Option<PathBuf> {
    let root = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()?.join(".config"),
    };
    Some(root.join("everyapi").join("credentials.json"))
}

/// Derive the provider `base_url` from the credentials' `api_base`.
///
/// The OpenAI driver builds request URLs as `"{base_url}/chat/completions"` (`openai.rs`), so the `/v1` segment must live in `base_url` itself.
/// It also strips one trailing slash from `base_url`, because `"https://x/v1/" + "/chat/completions"` yields a doubled separator that several gateways answer with a 504.
/// Trimming here means the stored value is already canonical instead of relying on that downstream repair.
pub(crate) fn derive_base_url(api_base: &str) -> String {
    format!("{}/v1", api_base.trim().trim_end_matches('/'))
}

// ---------------------------------------------------------------------------
// Gateway model list
// ---------------------------------------------------------------------------

/// One entry of the gateway's `GET /v1/models` response, reduced to the fields that affect catalog synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatewayModel {
    pub(crate) id: String,
    /// Vendor label — the token-limit snapshot lookup's namespace (`anthropic`, `openai`, `google`, `minimax`, …).
    /// Empty when the gateway omits it.
    pub(crate) owned_by: String,
    /// `supported_endpoint_types`, verbatim.
    /// Drives modality inference and the streaming-only warning.
    pub(crate) supported_endpoint_types: Vec<String>,
    /// `context_window` when the gateway published one on this endpoint.
    /// The most specific statement available: this is the model's own row in the gateway's model listing, so it outranks both the pricing feed's figure and the snapshot's.
    pub(crate) context_window: Option<u64>,
    /// `max_output` when the gateway published one.
    ///
    /// The gateway's `dto.OpenAIModels` struct carries a `max_output` field (json tag `max_output,omitempty`), but no observed account populates it — every live row omits it.
    /// Read anyway: the field is part of the gateway's published contract, and honouring it the day it starts arriving costs one line, whereas the alternative is silently preferring the snapshot's number over the gateway's own.
    pub(crate) max_output_tokens: Option<u64>,
}

/// Extract the model array from a `/v1/models` body.
///
/// Entries without a usable `id` are dropped rather than failing the whole response — one malformed row should not cost the user the other 17.
pub(crate) fn parse_gateway_models(body: &serde_json::Value) -> Vec<GatewayModel> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(|v| v.as_str()).map(str::trim)?;
            if id.is_empty() {
                return None;
            }
            let supported_endpoint_types = item
                .get("supported_endpoint_types")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            Some(GatewayModel {
                id: id.to_string(),
                owned_by: item
                    .get("owned_by")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                supported_endpoint_types,
                context_window: item
                    .get("context_window")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|v| *v > 0),
                max_output_tokens: item
                    .get("max_output")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|v| *v > 0),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Gateway pricing feed
// ---------------------------------------------------------------------------

/// One row of the gateway's `GET /api/pricing` feed.
///
/// This is the gateway's *own* billing book — what EveryAPI will actually charge — as opposed to the upstream vendor's list price.
/// It is served under optional auth, so it reads with or without the relay key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PricingRow {
    /// `model_name`, matching `/v1/models`' `id` verbatim.
    pub(crate) model_name: String,
    /// `false` for `quota_type == 1` / `billing_mode == "per_call"`, where the charge is a flat `model_price` per request and no per-token figure exists at all.
    pub(crate) per_token: bool,
    /// Input-side billing multiplier.
    /// Meaningless when `per_token` is false.
    pub(crate) model_ratio: f64,
    /// Output-side multiplier applied on top of `model_ratio`.
    pub(crate) completion_ratio: f64,
    /// `context_window` when the feed published a non-zero one.
    /// Zero in the feed means "unknown", not "no context".
    pub(crate) context_window: Option<u64>,
}

impl PricingRow {
    /// Input price per million tokens, in USD.
    ///
    /// The gateway additionally scales every charge by the account's `group_ratio` (observed values 0.25 / 0.35 / 0.55, i.e. all below 1), which is a per-account discount rather than a property of the model.
    /// It is deliberately NOT applied: the discount can change under the operator without the catalog being regenerated, and the undiscounted figure is an upper bound — over-reporting cost is the safe direction for budget and metering math.
    pub(crate) fn input_cost_per_m(&self) -> f64 {
        if !self.per_token {
            return 0.0;
        }
        self.model_ratio * RATIO_USD_PER_MILLION
    }

    /// Output price per million tokens, in USD.
    pub(crate) fn output_cost_per_m(&self) -> f64 {
        if !self.per_token {
            return 0.0;
        }
        self.model_ratio * self.completion_ratio * RATIO_USD_PER_MILLION
    }
}

/// Index the `GET /api/pricing` body by `model_name`.
///
/// A `BTreeMap` rather than a `HashMap` so any future iteration over the index is ordered (repo invariant #3298); lookups here are by exact key, but the type keeps the determinism guarantee from depending on nobody adding an iteration later.
///
/// Rows without a usable `model_name` are dropped rather than failing the whole feed — the same posture `parse_gateway_models` takes.
pub(crate) fn parse_pricing_rows(
    body: &serde_json::Value,
) -> std::collections::BTreeMap<String, PricingRow> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return std::collections::BTreeMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let model_name = item
                .get("model_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            // `quota_type` is the authoritative discriminator (the billing
            // implementation branches on it); `billing_mode` is the
            // human-readable mirror of the same fact. Treat either signal as
            // per-call so a feed that ships only one of them still bills
            // correctly.
            let per_call = item
                .get("quota_type")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|q| q != 0)
                || item.get("billing_mode").and_then(|v| v.as_str()) == Some("per_call");
            // `.filter(is_finite && >= 0.0)` mirrors the daemon-side parser
            // (`librefang-api/src/everyapi_catalog.rs::parse_pricing_entries`).
            // Without it a malformed or compromised feed carrying a negative
            // `model_ratio` would flow straight into
            // `input_cost_per_m`/`output_cost_per_m` as a *negative*,
            // confidently-known price — worse than the 0.0 case this file's
            // docs already guard against, because a negative figure actively
            // corrupts downstream budget math rather than merely looking
            // free. A row whose `model_ratio` fails the filter (missing,
            // non-finite, or negative) must also not claim `per_token`,
            // otherwise it falls back to `0.0` marked as a *known* price —
            // i.e. asserted free — rather than unknown.
            let model_ratio = item
                .get("model_ratio")
                .and_then(serde_json::Value::as_f64)
                .filter(|r| r.is_finite() && *r >= 0.0);
            Some((
                model_name.to_string(),
                PricingRow {
                    model_name: model_name.to_string(),
                    per_token: !per_call && model_ratio.is_some(),
                    model_ratio: model_ratio.unwrap_or(0.0),
                    completion_ratio: item
                        .get("completion_ratio")
                        .and_then(serde_json::Value::as_f64)
                        .filter(|r| r.is_finite() && *r >= 0.0)
                        .unwrap_or(0.0),
                    context_window: item
                        .get("context_window")
                        .and_then(serde_json::Value::as_u64)
                        .filter(|v| *v > 0),
                },
            ))
        })
        .collect()
}

/// Infer a model's modality from `supported_endpoint_types`.
///
/// The gateway does not publish a modality field, so the endpoint-type list is the only signal.
/// An EMPTY list means video: the `doubao-seedance-*` family publishes `[]` and is video-generation only.
/// That case must be checked before the text default, otherwise those entries would be graded as text models and then dropped for missing a context window.
/// Modality inference with the model's own context window as a tiebreaker.
///
/// The empty-`supported_endpoint_types` case is genuinely ambiguous.
/// Every observed empty row is a `doubao-seedance-*` video model, which is why it defaults to video — but the field is optional, so a gateway that stops publishing it would have its chat models registered as video: exempt from the context-window validation, and unusable for chat while looking present in the catalog.
///
/// A published `context_window` settles it.
/// Video generation has no context window and none of the observed video rows carry one, so its presence is positive evidence of a text model regardless of what the endpoint list omits.
/// Without that evidence the video default stands, since that is what every empty row has actually been.
pub(crate) fn infer_modality_with_context(
    supported_endpoint_types: &[String],
    context_window: Option<u64>,
) -> Modality {
    if supported_endpoint_types
        .iter()
        .any(|t| t == "image-generation")
    {
        return Modality::Image;
    }
    if supported_endpoint_types.iter().any(|t| t == "audio-speech") {
        return Modality::Audio;
    }
    if supported_endpoint_types.is_empty() {
        return match context_window {
            Some(w) if w > 0 => Modality::Text,
            _ => Modality::Video,
        };
    }
    Modality::Text
}

/// Whether a model only speaks the `openai-response` endpoint shape.
///
/// Such models (the `gpt-5.6-*` family) reject non-streaming requests with HTTP 400 `"Stream must be set to true"`.
/// Everything in LibreFang that calls `driver.complete()` rather than the streaming entry point — the compactor, proactive memory, the skill workshop, web augmentation — would fail against one, so they are excluded from automatic default selection and called out in the command's report.
pub(crate) fn is_openai_response_only(supported_endpoint_types: &[String]) -> bool {
    !supported_endpoint_types.is_empty()
        && supported_endpoint_types
            .iter()
            .all(|t| t == "openai-response")
}

// ---------------------------------------------------------------------------
// Metadata resolution
// ---------------------------------------------------------------------------

/// Context / pricing metadata resolved for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModelMetadata {
    pub(crate) context_window: u64,
    pub(crate) max_output_tokens: u64,
    pub(crate) input_cost_per_m: f64,
    pub(crate) output_cost_per_m: f64,
    pub(crate) pricing_known: bool,
    pub(crate) supports_tools: bool,
    pub(crate) supports_vision: bool,
    pub(crate) supports_thinking: bool,
    /// True when either token limit had to come from the OpenRouter snapshot because neither gateway endpoint published one.
    /// Surfaced in the command's report so the operator knows which numbers are the gateway's own and which are borrowed.
    pub(crate) token_limits_borrowed: bool,
}

/// Rewrite a `-`-separated version tail into the `.`-separated spelling.
///
/// The gateway and the OpenRouter snapshot disagree on one character: gateway `claude-haiku-4-5` is snapshot `anthropic/claude-haiku-4.5`.
/// Only a hyphen sitting between two ASCII digits is rewritten, so `gpt-5.6-luna` and `doubao-seedance-1-0-pro-250528` keep every other hyphen intact.
pub(crate) fn normalize_version_separators(id: &str) -> String {
    let bytes = id.as_bytes();
    id.char_indices()
        .map(|(i, c)| {
            let between_digits = c == '-'
                && i > 0
                && bytes[i - 1].is_ascii_digit()
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
            if between_digits {
                '.'
            } else {
                c
            }
        })
        .collect()
}

/// Candidate snapshot ids for one gateway model, most specific first.
///
/// The compiled-in snapshot keys entries as `openrouter/{vendor}/{model}`.
/// `ModelCatalog::find_model` already lowercases both sides, which covers the gateway's `MiniMax-M3` vs the snapshot's `minimax/minimax-m3`; the only remaining mismatch is the version-tail separator, handled by [`normalize_version_separators`].
///
/// Used **only** for the token-limit fallback described on [`resolve_metadata`] — never for pricing, which now comes from the gateway's own feed.
pub(crate) fn snapshot_lookup_ids(owned_by: &str, model_id: &str) -> Vec<String> {
    if owned_by.is_empty() {
        return Vec::new();
    }
    let mut ids = vec![format!("openrouter/{owned_by}/{model_id}")];
    let normalized = normalize_version_separators(model_id);
    if normalized != model_id {
        ids.push(format!("openrouter/{owned_by}/{normalized}"));
    }
    ids
}

/// Resolve one model's context / pricing metadata.
///
/// Source precedence, per field:
///
/// - **Pricing** — the gateway's `/api/pricing` row, converted by
///   [`PricingRow::input_cost_per_m`] / [`PricingRow::output_cost_per_m`].
///   The OpenRouter snapshot is *never* consulted for pricing: it carries
///   the upstream vendor's list price, which is a different number from
///   what this gateway bills and drifts independently of it. No pricing
///   row, or a per-call row with no per-token price at all, means
///   `pricing_known = false` with 0.0/0.0 — never a fabricated figure,
///   because `pricing_known` deserializes to `true` by default and a bare
///   0.0 would assert the model is genuinely free.
/// - **`context_window`** — `/v1/models` (the model's own row) beats
///   `/api/pricing` beats the snapshot. The gateway is authoritative about
///   its own deployment; the snapshot describes OpenRouter's copy.
/// - **`max_output_tokens`** — `/v1/models`' `max_output` when populated,
///   else the snapshot.
///
/// The snapshot fallback for the two token limits is deliberate and is the
/// only thing keeping most text models registrable. Neither gateway
/// endpoint publishes an output limit for any observed model, and
/// `/api/pricing` publishes a context window only for the claude family —
/// so with no fallback, `ModelCatalogEntry::validate()` would reject every
/// non-claude text model (`max_output_tokens == 0`) and the command would
/// register 0 text models instead of 8. The alternative candidates were
/// both worse: skipping is a silent feature regression, and deriving a
/// fraction of `context_window` would invent a number that feeds straight
/// into compaction thresholds. Borrowing a real published figure and
/// naming the affected models in the report keeps the provenance visible.
///
/// Capability flags (`supports_tools` / `supports_vision` /
/// `supports_thinking`) also come from the snapshot: the gateway publishes
/// none of them. `input_modalities` appears on some `/v1/models` rows but
/// never disagreed with the snapshot's vision flag on any observed model,
/// so it is not read.
///
/// The snapshot is `include_str!`-ed into `librefang-runtime` and merged by
/// every `ModelCatalog` construction, so an empty-directory catalog is a
/// sufficient (and hermetic) handle on it.
pub(crate) fn resolve_metadata(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    model: &GatewayModel,
    pricing: Option<&PricingRow>,
) -> ModelMetadata {
    let snapshot = snapshot_lookup_ids(&model.owned_by, &model.id)
        .into_iter()
        .find_map(|id| catalog.find_model(&id));

    let context_window = model
        .context_window
        .or_else(|| pricing.and_then(|p| p.context_window))
        .or_else(|| snapshot.map(|s| s.context_window).filter(|v| *v > 0));
    let max_output_tokens = model
        .max_output_tokens
        .or_else(|| snapshot.map(|s| s.max_output_tokens).filter(|v| *v > 0));

    let borrowed_context = context_window.is_some()
        && model.context_window.is_none()
        && pricing.and_then(|p| p.context_window).is_none();
    let borrowed_max_output = max_output_tokens.is_some() && model.max_output_tokens.is_none();

    ModelMetadata {
        context_window: context_window.unwrap_or(0),
        max_output_tokens: max_output_tokens.unwrap_or(0),
        input_cost_per_m: pricing.map(PricingRow::input_cost_per_m).unwrap_or(0.0),
        output_cost_per_m: pricing.map(PricingRow::output_cost_per_m).unwrap_or(0.0),
        pricing_known: pricing.is_some_and(|p| p.per_token),
        supports_tools: snapshot.is_some_and(|s| s.supports_tools),
        supports_vision: snapshot.is_some_and(|s| s.supports_vision),
        supports_thinking: snapshot.is_some_and(|s| s.supports_thinking),
        token_limits_borrowed: borrowed_context || borrowed_max_output,
    }
}

// ---------------------------------------------------------------------------
// Catalog synthesis
// ---------------------------------------------------------------------------

/// A model the command refused to register, with the i18n key explaining why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedModel {
    pub(crate) id: String,
    /// i18n key for the reason.
    /// A key rather than rendered prose so the synthesis stays pure and the locale files stay grep-able (each key appears verbatim in this file, which `test_no_dead_locale_keys` requires).
    pub(crate) reason_key: &'static str,
}

/// Everything the report needs out of one synthesis run.
pub(crate) struct SynthesisResult {
    pub(crate) file: ModelCatalogFile,
    pub(crate) skipped: Vec<SkippedModel>,
    /// Registered ids that reject non-streaming requests.
    pub(crate) streaming_only: Vec<String>,
    /// Registered ids whose context window and/or output limit had to be borrowed from the OpenRouter snapshot because the gateway published neither.
    /// Reported so a borrowed number is never mistaken for the gateway's own.
    pub(crate) borrowed_token_limits: Vec<String>,
    /// Registered ids carrying no per-token price: absent from the pricing feed, or billed per call.
    /// Their `pricing_known` is false, so budget math treats them as unknown rather than free.
    pub(crate) unpriced: Vec<String>,
}

/// Build the `providers/everyapi.toml` contents.
///
/// The skip rule is the load-bearing part.
/// `ModelCatalogEntry::validate()` rejects a `Modality::Text` entry whose `context_window` OR `max_output_tokens` is 0, and `merge_catalog_file` then drops it with only a `warn!`.
/// Emitting such an entry anyway would make the command look successful while the model silently vanished at the next daemon boot, so a text model whose limits cannot be resolved from any source — gateway listing, pricing feed, or the snapshot fallback described on [`resolve_metadata`] — is skipped and reported by name instead.
/// Inventing a plausible-looking number would be worse: a wrong context window feeds straight into compaction thresholds and budget math.
///
/// Non-text models are exempt from `validate()`'s token checks and are registered with whatever the gateway published.
///
/// Output is sorted by id (repo invariant #3298) so re-running the command against an unchanged gateway rewrites a byte-identical file.
pub(crate) fn synthesize_catalog(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    base_url: &str,
    models: &[GatewayModel],
    pricing: &std::collections::BTreeMap<String, PricingRow>,
) -> SynthesisResult {
    let mut entries: Vec<ModelCatalogEntry> = Vec::new();
    let mut skipped: Vec<SkippedModel> = Vec::new();
    let mut streaming_only: Vec<String> = Vec::new();
    let mut borrowed_token_limits: Vec<String> = Vec::new();
    let mut unpriced: Vec<String> = Vec::new();

    for model in models {
        let modality =
            infer_modality_with_context(&model.supported_endpoint_types, model.context_window);
        let metadata = resolve_metadata(catalog, model, pricing.get(&model.id));

        // `validate()` rejects a text entry missing EITHER limit, so the
        // reason must name which one is actually missing. A model whose
        // context window resolved from the pricing feed but whose output
        // limit did not is a real case (any claude-family model the
        // snapshot has not caught up with), and telling that operator "no
        // context window is known" would send them looking for the wrong
        // thing.
        if modality == Modality::Text {
            let reason_key = match (metadata.context_window, metadata.max_output_tokens) {
                (0, 0) => Some("everyapi-connect-skip-no-metadata"),
                (0, _) => Some("everyapi-connect-skip-no-context-window"),
                (_, 0) => Some("everyapi-connect-skip-no-output-limit"),
                _ => None,
            };
            if let Some(reason_key) = reason_key {
                skipped.push(SkippedModel {
                    id: model.id.clone(),
                    reason_key,
                });
                continue;
            }
        }

        if is_openai_response_only(&model.supported_endpoint_types) {
            streaming_only.push(model.id.clone());
        }
        if metadata.token_limits_borrowed {
            borrowed_token_limits.push(model.id.clone());
        }
        if !metadata.pricing_known {
            unpriced.push(model.id.clone());
        }

        entries.push(ModelCatalogEntry {
            id: model.id.clone(),
            display_name: model.id.clone(),
            provider: PROVIDER_ID.to_string(),
            // Deliberately NOT `ModelTier::Custom`. `find_model` returns the
            // first `Custom` match immediately so user-defined models beat
            // builtins (#983), and `merge_catalog_file` dedupes on
            // `(id, provider)` — so a `Custom` gateway copy of an id that
            // also exists upstream (`claude-sonnet-5`, `claude-opus-5`,
            // `gemini-3.5-flash`) would hijack every provider-blind lookup
            // (`pricing`, `effective_capabilities_for`, the last-resort arm
            // of `find_model_for_manifest`) and silently re-price agents
            // that never opted into this gateway. `Balanced` matches what
            // the registry's own provider catalog files carry, and the
            // gateway copy stays reachable via `find_model_for_provider`.
            tier: ModelTier::Balanced,
            modality,
            context_window: metadata.context_window,
            max_output_tokens: metadata.max_output_tokens,
            // Always emitted, never omitted: neither cost field carries
            // `#[serde(default)]`, so a missing one fails the whole file's
            // parse. `pricing_known` is the flag that distinguishes "free"
            // from "unknown" — it defaults to `true` on deserialize, so an
            // unpriced model must carry an explicit `false`.
            input_cost_per_m: metadata.input_cost_per_m,
            output_cost_per_m: metadata.output_cost_per_m,
            pricing_known: metadata.pricing_known,
            supports_tools: metadata.supports_tools,
            supports_vision: metadata.supports_vision,
            // Every OpenAI-shaped endpoint on this gateway streams; the
            // `openai-response` family *requires* it.
            supports_streaming: true,
            supports_thinking: metadata.supports_thinking,
            ..Default::default()
        });
    }

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    skipped.sort_by(|a, b| a.id.cmp(&b.id));
    streaming_only.sort();
    borrowed_token_limits.sort();
    unpriced.sort();

    SynthesisResult {
        file: ModelCatalogFile {
            provider: Some(ProviderCatalogToml {
                id: PROVIDER_ID.to_string(),
                display_name: PROVIDER_DISPLAY_NAME.to_string(),
                api_key_env: API_KEY_ENV.to_string(),
                base_url: base_url.to_string(),
                key_required: true,
                signup_url: None,
                regions: std::collections::HashMap::new(),
                // Derived from what actually got registered rather than left
                // empty: the media driver cache gates on the provider's
                // declared capabilities, so hardcoding none made every
                // registered image / audio / video model unreachable through
                // the media paths while still appearing in the catalog.
                media_capabilities: media_capabilities_for(&entries),
            }),
            models: entries,
        },
        skipped,
        streaming_only,
        borrowed_token_limits,
        unpriced,
    }
}

/// Pick the model to make default under `--set-default`.
///
/// Preference order:
/// 1. Text models that are NOT `openai-response`-only. Those reject
///    non-streaming requests (HTTP 400 `"Stream must be set to true"`), and
///    the compactor / proactive-memory / skill-workshop / web-augment paths
///    all call `driver.complete()` without streaming — making one the daemon
///    default would break every one of them the first time it fired.
/// 2. Any remaining text model, so `--set-default` still does something on a
///    gateway that only exposes the response-only family.
///
/// Within a tier the lowest id wins, purely so the choice is deterministic
/// across runs; entries are already id-sorted by [`synthesize_catalog`], and
/// ASCII order puts uppercase ids (`MiniMax-M3`) ahead of lowercase ones.
pub(crate) fn choose_default_model(
    entries: &[ModelCatalogEntry],
    streaming_only: &[String],
) -> Option<String> {
    let text_models = || entries.iter().filter(|m| m.modality == Modality::Text);
    // Rank rather than take-the-first. Entries are id-sorted for output
    // determinism, and ASCII puts uppercase ahead of lowercase, so the first
    // text model was whichever id happened to sort earliest — `MiniMax-M3`
    // on the live listing, over `claude-sonnet-5`. The CLI help promises
    // "this gateway's best model", so pick by capability.
    //
    // Price is the proxy for capability, because it is the one figure the
    // gateway publishes for every model and it tracks the vendor's own
    // tiering. Ties break on context window, then on id so the result stays
    // deterministic for a gateway that prices two models identically.
    let rank = |m: &ModelCatalogEntry| {
        (
            ordered_float(m.input_cost_per_m),
            m.context_window,
            std::cmp::Reverse(m.id.clone()),
        )
    };
    text_models()
        .filter(|m| !streaming_only.contains(&m.id))
        .max_by(|a, b| rank(a).cmp(&rank(b)))
        .or_else(|| text_models().max_by(|a, b| rank(a).cmp(&rank(b))))
        .map(|m| m.id.clone())
}

/// Total-order key for a price so it can participate in a tuple comparison.
///
/// Prices are non-negative and finite in every catalog entry we synthesise (`pricing_known = false` carries 0.0 rather than NaN), so scaling to an integer is exact enough for ranking and avoids `f64: Ord` not existing.
fn ordered_float(v: f64) -> u64 {
    (v.max(0.0) * 1_000.0) as u64
}

// ---------------------------------------------------------------------------
// Command entry point
// ---------------------------------------------------------------------------

/// Fetch `GET {base_url}/models` from the gateway.
///
/// `base_url` already ends in `/v1`, so this hits `/v1/models`.
/// Returns `None` on any transport error or non-2xx status: an unreachable model list is not fatal, because the `[provider]` section alone already makes the gateway usable with a hand-specified model id.
///
/// `key` is the relay credential.
/// It is named to match `daemon_client_with_api_key` in `commands/common.rs` so the header construction reads identically at both call sites; it is only ever written into the outbound header, never into output or a log.
/// Why a model-list fetch did not produce a listing.
///
/// Collapsing these into one `None` conflated two states that need opposite handling: an unreachable host is transient and the stored credential is probably still good, while a 401 means the relay key is dead and saving it would register a provider that fails every request.
/// The remediation text differs too — "check the gateway is reachable" is actively misleading when the real answer is `everyapi login`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModelFetchError {
    /// The gateway rejected the relay key (401/403).
    Unauthorized,
    /// Transport failure, or any other non-2xx status.
    Unreachable,
}

fn fetch_gateway_models(base_url: &str, key: &str) -> Result<Vec<GatewayModel>, ModelFetchError> {
    let client = crate::http_client::client_builder()
        .timeout(MODELS_FETCH_TIMEOUT)
        .build()
        .map_err(|_| ModelFetchError::Unreachable)?;
    let response = client
        .get(format!("{base_url}/models"))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"))
        .send()
        .map_err(|_| ModelFetchError::Unreachable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(classify_fetch_status(status.as_u16()));
    }
    let body = response
        .json::<serde_json::Value>()
        .map_err(|_| ModelFetchError::Unreachable)?;
    Ok(parse_gateway_models(&body))
}

/// Map an HTTP status to a fetch outcome.
///
/// Split out so the 401-vs-everything-else rule is unit-testable without a live gateway. 403 counts as unauthorized alongside 401: the gateway promotes `x-api-key` into a bearer before validating, and a revoked key can surface either way depending on which shim ran.
pub(crate) fn classify_fetch_status(status: u16) -> ModelFetchError {
    match status {
        401 | 403 => ModelFetchError::Unauthorized,
        _ => ModelFetchError::Unreachable,
    }
}

/// Fetch `GET {api_base}/api/pricing` — the gateway's own billing book.
///
/// Note the base: this endpoint sits at the gateway *root*, not under the OpenAI-compatible `/v1` prefix, so it is built from `api_base` rather than the derived `base_url`.
///
/// The bearer is sent even though the route is registered with optional auth and answers anonymously.
/// One code path for both gateway fetches is worth more than saving a header, and an authenticated read is the shape that keeps working if EveryAPI ever tightens the route or starts varying rows per account.
///
/// Returns `None` on any transport error or non-2xx status.
/// A missing pricing feed is not fatal: models still register, they just carry `pricing_known = false` instead of a fabricated price.
fn fetch_pricing_rows(
    api_base: &str,
    key: &str,
) -> Option<std::collections::BTreeMap<String, PricingRow>> {
    let client = crate::http_client::client_builder()
        .timeout(MODELS_FETCH_TIMEOUT)
        .build()
        .ok()?;
    let response = client
        .get(format!(
            "{}/api/pricing",
            api_base.trim().trim_end_matches('/')
        ))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"))
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<serde_json::Value>().ok()?;
    Some(parse_pricing_rows(&body))
}

/// Read + parse the credentials file, or exit(1) with a rendered message.
fn load_credentials() -> EveryApiCredentials {
    let Some(path) = credentials_path() else {
        ui::error_with_fix(
            &i18n::t("everyapi-connect-credentials-missing"),
            &i18n::t("everyapi-connect-credentials-missing-fix"),
        );
        std::process::exit(1);
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        ui::error_with_fix(
            &i18n::t_args(
                "everyapi-connect-credentials-unreadable",
                &[("path", &path.display().to_string())],
            ),
            &i18n::t("everyapi-connect-credentials-missing-fix"),
        );
        std::process::exit(1);
    };
    match parse_credentials(&raw) {
        Ok(credentials) => credentials,
        Err(reason_key) => {
            ui::error_with_fix(
                &i18n::t_args(reason_key, &[("path", &path.display().to_string())]),
                &i18n::t("everyapi-connect-credentials-missing-fix"),
            );
            std::process::exit(1);
        }
    }
}

/// Try to write the provider through the running daemon.
///
/// Returns `true` when the daemon accepted the write.
/// The endpoint converts the flat JSON body into the `[provider] … [[models]]` layout itself, then reloads the in-process catalog — so on this path there is nothing for the operator to restart.
/// No `api_key` field is sent: the relay key is already in `~/.librefang/.env`, and including it would additionally copy the secret into `secrets.env`.
///
/// `daemon_json_checked` rather than `daemon_json`, because the latter exits the process on a transport error and returns an empty body (not an error) on 4xx — either way the caller could never fall back to the direct write.
/// Outcome of the daemon-side provider write.
///
/// Three states rather than a bool because they need different handling.
/// A transport failure means the daemon went away and the file fallback is the right answer.
/// A *rejection* means the daemon parsed our payload and refused it — falling back would write the very definition it just deleted (the registry route removes the file it rejected), leaving a catalog file that fails to parse on every subsequent boot.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DaemonWrite {
    Accepted,
    /// The daemon is unreachable; the caller should write the file itself.
    Unreachable,
    /// The daemon understood the request and refused it.
    /// Do not fall back.
    Rejected(String),
}

/// Classify the daemon's response to the registry write.
///
/// Pure so the fallback rule is testable without a daemon.
/// A 5xx counts as unreachable — the daemon is up but broken, and a local file is still better than nothing.
pub(crate) fn classify_daemon_write(status: u16, body: &serde_json::Value) -> DaemonWrite {
    if (200..300).contains(&status) && body.get("error").is_none() {
        return DaemonWrite::Accepted;
    }
    if (400..500).contains(&status) {
        let detail = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        return DaemonWrite::Rejected(detail);
    }
    DaemonWrite::Unreachable
}

/// POST the provider definition to a running daemon.
///
/// Deliberately does NOT use `daemon_json_checked`: its transport-error arm ends in `std::process::exit(1)`, which would kill the process before the caller could fall back to the direct file write — i.e. it would abort on exactly the failure the fallback exists for.
fn write_provider_via_daemon(base: &str, body: &serde_json::Value) -> DaemonWrite {
    let client = daemon_client();
    let response = client
        .post(format!(
            "{base}/api/registry/content/provider?allow_overwrite=true"
        ))
        .json(body)
        .send();
    match response {
        Ok(r) => {
            let status = r.status().as_u16();
            let parsed = r.json::<serde_json::Value>().unwrap_or_default();
            classify_daemon_write(status, &parsed)
        }
        Err(_) => DaemonWrite::Unreachable,
    }
}

/// Make the relay key live in the running daemon's own environment.
///
/// The daemon reads `~/.librefang/.env` once at boot — `load_dotenv` is a `call_once` and no path re-reads it afterwards — so a key written during this command is invisible to a daemon that is already running.
/// This posts it to `POST /api/providers/{id}/key`, which writes `secrets.env` and calls `set_env_var_guarded`, so the key resolves for the very next turn.
///
/// Returns `false` when no daemon is reachable or the call failed, so the caller can fall back to telling the operator to restart.
/// The key value is never logged or echoed — only the outcome is reported.
fn push_key_to_daemon(base: Option<&str>, relay_key: &str) -> bool {
    let Some(base) = base else {
        return false;
    };
    let client = daemon_client();
    let (status, response) = daemon_json_checked(
        client
            .post(format!("{base}/api/providers/{PROVIDER_ID}/key"))
            .json(&serde_json::json!({ "key": relay_key }))
            .send(),
    );
    status.is_success() && response.get("error").is_none()
}

/// Write `{librefang_home}/providers/everyapi.toml` directly.
///
/// The path is fixed by agreement with `doctor::EveryApiWiringCheck`, which looks for exactly this file to decide whether LibreFang is wired to the gateway.
fn write_provider_file(toml_body: &str) -> Result<PathBuf, String> {
    let path = provider_file_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, toml_body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Media capability strings implied by the registered models' modalities.
///
/// The names match what the registry's own provider files carry and what `librefang-runtime-media` looks for — `image_generation`, `text_to_speech`, `video_generation`.
/// Sorted and deduplicated so the generated TOML stays byte-stable across runs (#3298).
/// Music has no corresponding capability string in the media crate, so a music entry contributes nothing rather than inventing a name nothing reads.
fn media_capabilities_for(entries: &[ModelCatalogEntry]) -> Vec<String> {
    let mut caps: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for entry in entries {
        match entry.modality {
            Modality::Image => {
                caps.insert("image_generation".to_string());
            }
            Modality::Audio => {
                caps.insert("text_to_speech".to_string());
            }
            Modality::Video => {
                caps.insert("video_generation".to_string());
            }
            // Text declares no media capability, and Music has no
            // corresponding string in `librefang-runtime-media`. `Modality`
            // is non-exhaustive, so a future variant contributes nothing
            // rather than guessing a capability name nothing reads.
            _ => {}
        }
    }
    caps.into_iter().collect()
}

/// Where the provider entry lives for this CLI invocation.
///
/// Single source of truth so the write path, the clobber guard, and the report all name the same file.
/// Note this resolves through the *CLI's* home: when the daemon runs with a different `LIBREFANG_HOME` than the shell invoking the CLI, the daemon-write path targets the daemon's home while this one does not.
/// `warn_on_home_divergence` surfaces that rather than silently writing where nothing reads.
fn provider_file_path() -> PathBuf {
    cli_librefang_home()
        .join("providers")
        .join(format!("{PROVIDER_ID}.toml"))
}

/// Whether a provider entry already exists AND carries at least one model.
///
/// Gates the overwrite on a failed fetch.
/// An entry with zero models is not worth protecting — that is the state a previous degraded run left behind, and replacing it is an improvement rather than a regression.
fn existing_provider_has_models() -> bool {
    let Ok(raw) = std::fs::read_to_string(provider_file_path()) else {
        return false;
    };
    toml::from_str::<librefang_types::model_catalog::ModelCatalogFile>(&raw)
        .map(|f| !f.models.is_empty())
        .unwrap_or(false)
}

/// `librefang models connect <target> [--set-default]`.
pub(crate) fn cmd_models_connect(target: &str, set_default: bool) {
    if target != EVERYAPI_TARGET {
        ui::error_with_fix(
            &i18n::t_args("everyapi-connect-unknown-target", &[("target", target)]),
            &i18n::t("everyapi-connect-unknown-target-fix"),
        );
        std::process::exit(1);
    }

    let credentials = load_credentials();
    let base_url = derive_base_url(&credentials.api_base);

    ui::section(&i18n::t("everyapi-connect-title"));
    ui::blank();

    let fetched = fetch_gateway_models(&base_url, &credentials.relay_key);
    let models = match &fetched {
        Ok(models) => models.as_slice(),
        Err(ModelFetchError::Unauthorized) => {
            // A dead key is not a partial success. Registering the provider
            // now would store a credential that fails every request, and
            // `--set-default` would migrate agents onto it. Stop before
            // anything is written.
            ui::error_with_fix(
                &i18n::t("everyapi-connect-models-fetch-unauthorized"),
                &i18n::t("everyapi-connect-models-fetch-unauthorized-fix"),
            );
            std::process::exit(1);
        }
        Err(ModelFetchError::Unreachable) => {
            // Refuse to overwrite a provider entry that already carries
            // models. Both persistence paths replace the file wholesale, so
            // continuing with an empty listing would downgrade a working
            // catalog to zero entries and still report success — a transient
            // network blip would silently unregister every gateway model.
            // A first run has nothing to lose, so it may proceed and produce
            // the provider entry with models filled in on a later run.
            if existing_provider_has_models() {
                ui::error_with_fix(
                    &i18n::t("everyapi-connect-models-fetch-failed-would-clobber"),
                    &i18n::t("everyapi-connect-models-fetch-failed-fix"),
                );
                std::process::exit(1);
            }
            ui::warn_with_fix(
                &i18n::t("everyapi-connect-models-fetch-failed"),
                &i18n::t("everyapi-connect-models-fetch-failed-fix"),
            );
            &[]
        }
    };

    let pricing = match fetch_pricing_rows(&credentials.api_base, &credentials.relay_key) {
        Some(rows) => rows,
        None => {
            ui::warn_with_fix(
                &i18n::t("everyapi-connect-pricing-fetch-failed"),
                &i18n::t("everyapi-connect-pricing-fetch-failed-fix"),
            );
            std::collections::BTreeMap::new()
        }
    };

    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    let synthesis = synthesize_catalog(&catalog, &base_url, models, &pricing);
    let toml_body = match toml::to_string_pretty(&synthesis.file) {
        Ok(body) => body,
        Err(e) => {
            ui::error(&i18n::t_args(
                "everyapi-connect-serialize-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };

    // Persist the key before the provider entry: a provider whose
    // `api_key_env` resolves to nothing is worse than no provider at all,
    // because the daemon will list it as configured-but-unauthenticated.
    if let Err(e) = dotenv::save_env_key(API_KEY_ENV, &credentials.relay_key) {
        ui::error(&i18n::t_args(
            "everyapi-connect-key-save-failed",
            &[("error", &e)],
        ));
        std::process::exit(1);
    }
    ui::success(&i18n::t_args(
        "everyapi-connect-key-saved",
        &[("env_var", API_KEY_ENV)],
    ));

    // Daemon first so the running catalog picks the provider up without a
    // restart; the direct file write is the fallback and also the only path
    // when no daemon is up.
    let daemon = find_daemon();
    let daemon_write = daemon.as_deref().map_or(DaemonWrite::Unreachable, |base| {
        write_provider_via_daemon(base, &provider_request_body(&synthesis.file))
    });
    // A rejection is terminal. The registry route deletes the file it could
    // not parse, so falling back to the direct write would restore exactly
    // the definition the daemon just refused — and it would fail to parse on
    // every subsequent boot, with only a log line to say so.
    if let DaemonWrite::Rejected(detail) = &daemon_write {
        ui::error(&i18n::t_args(
            "everyapi-connect-provider-rejected",
            &[("error", detail)],
        ));
        std::process::exit(1);
    }
    let via_daemon = daemon_write == DaemonWrite::Accepted;
    if via_daemon {
        ui::success(&i18n::t_args(
            "everyapi-connect-provider-written-daemon",
            &[("path", "providers/everyapi.toml")],
        ));
        // `save_env_key` above wrote the key to `~/.librefang/.env`, but the
        // daemon parsed that file exactly once at boot (`load_dotenv` is a
        // `call_once` and nothing re-reads it afterwards), so the running
        // process cannot see a key added now. Without this the provider is
        // registered and immediately unusable, and the "no restart needed"
        // message above would be a lie. `POST /api/providers/{name}/key`
        // writes `secrets.env` and calls `set_env_var_guarded`, which makes
        // the key live in the daemon's own environment. `.env` still holds
        // the authoritative copy for the next boot — it wins on reload
        // because the loader only fills vars that are not already set.
        if !push_key_to_daemon(daemon.as_deref(), &credentials.relay_key) {
            ui::hint(&i18n::t("everyapi-connect-restart-required"));
        }
    } else {
        match write_provider_file(&toml_body) {
            Ok(path) => {
                ui::success(&i18n::t_args(
                    "everyapi-connect-provider-written-file",
                    &[("path", &path.display().to_string())],
                ));
                ui::hint(&i18n::t("everyapi-connect-restart-required"));
            }
            Err(e) => {
                ui::error(&i18n::t_args(
                    "everyapi-connect-provider-write-failed",
                    &[("error", &e)],
                ));
                std::process::exit(1);
            }
        }
    }

    report_models(&synthesis);
    // Only attempt the live `POST .../default` call when the provider write
    // actually went through the daemon. When `via_daemon` is false (no daemon
    // detected, or the write failed and fell back to the direct file write),
    // the daemon's in-process catalog was never updated — the file path
    // above already told the operator a restart is required, so this must
    // fall into the same "needs daemon" branch as the no-daemon case rather
    // than attempt (and fail) a live call against a stale catalog.
    let default_daemon = via_daemon.then_some(daemon.as_deref()).flatten();
    handle_default_model(&synthesis, default_daemon, set_default);
}

/// Flat JSON body for `POST /api/registry/content/provider`.
///
/// The endpoint's `normalize_provider_body` nests every non-`models` key under `provider` itself, so the body is sent flat with `models` alongside.
fn provider_request_body(file: &ModelCatalogFile) -> serde_json::Value {
    let provider = file.provider.as_ref();
    serde_json::json!({
        "id": PROVIDER_ID,
        "display_name": PROVIDER_DISPLAY_NAME,
        "api_key_env": API_KEY_ENV,
        "base_url": provider.map(|p| p.base_url.as_str()).unwrap_or_default(),
        "key_required": true,
        "models": file.models.iter().map(model_request_value).collect::<Vec<_>>(),
    })
}

/// One `[[models]]` entry as JSON for the daemon route.
///
/// Written by hand rather than via `serde_json::to_value` because the CLI has no `serde` dependency to reach `Serialize` through — and because the endpoint's `json_to_toml_value` drops empty strings/arrays anyway, so only the fields that must survive are sent.
/// `pricing_known` is always included: it defaults to `true` on deserialize, so omitting it on an unpriced model would silently claim the model is free.
fn model_request_value(model: &ModelCatalogEntry) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "display_name": model.display_name,
        "provider": model.provider,
        "tier": model.tier.to_string(),
        "modality": model.modality.to_string(),
        "context_window": model.context_window,
        "max_output_tokens": model.max_output_tokens,
        "input_cost_per_m": model.input_cost_per_m,
        "output_cost_per_m": model.output_cost_per_m,
        "pricing_known": model.pricing_known,
        "supports_tools": model.supports_tools,
        "supports_vision": model.supports_vision,
        "supports_streaming": model.supports_streaming,
        "supports_thinking": model.supports_thinking,
    })
}

/// Print the registered / skipped / streaming-only summary.
fn report_models(synthesis: &SynthesisResult) {
    ui::success(&i18n::t_args(
        "everyapi-connect-models-registered",
        &[("count", &synthesis.file.models.len().to_string())],
    ));

    if !synthesis.skipped.is_empty() {
        ui::blank();
        ui::check_warn(&i18n::t_args(
            "everyapi-connect-models-skipped",
            &[("count", &synthesis.skipped.len().to_string())],
        ));
        for skipped in &synthesis.skipped {
            ui::hint(&i18n::t_args(skipped.reason_key, &[("model", &skipped.id)]));
        }
    }

    // Provenance, not decoration: a borrowed context window drives
    // compaction thresholds and an unknown price silences budget accounting,
    // so both sets are named rather than merely counted.
    if !synthesis.borrowed_token_limits.is_empty() {
        ui::blank();
        ui::check_warn(&i18n::t_args(
            "everyapi-connect-token-limits-borrowed",
            &[("models", &synthesis.borrowed_token_limits.join(", "))],
        ));
    }

    if !synthesis.unpriced.is_empty() {
        ui::blank();
        ui::check_warn(&i18n::t_args(
            "everyapi-connect-models-unpriced",
            &[("models", &synthesis.unpriced.join(", "))],
        ));
    }

    if !synthesis.streaming_only.is_empty() {
        ui::blank();
        ui::check_warn(&i18n::t_args(
            "everyapi-connect-streaming-only",
            &[("models", &synthesis.streaming_only.join(", "))],
        ));
    }
}

/// Apply (or advertise) the default-model switch.
fn handle_default_model(synthesis: &SynthesisResult, daemon: Option<&str>, set_default: bool) {
    let Some(model) = choose_default_model(&synthesis.file.models, &synthesis.streaming_only)
    else {
        ui::blank();
        if set_default {
            ui::check_warn(&i18n::t("everyapi-connect-default-no-candidate"));
        }
        return;
    };

    ui::blank();
    if !set_default {
        ui::hint(&i18n::t_args(
            "everyapi-connect-default-hint",
            &[("model", &model)],
        ));
        return;
    }

    let Some(base) = daemon else {
        // Editing config.toml by hand from here would race the daemon's own
        // writer and bypass the agent-migration the API path performs, so the
        // no-daemon case reports the command to run instead of guessing.
        ui::check_warn(&i18n::t_args(
            "everyapi-connect-default-needs-daemon",
            &[("model", &model)],
        ));
        return;
    };

    let client = daemon_client();
    // Pin the gateway URL into `[provider_urls]` BEFORE switching the
    // default. `persist_default_model` on the default route writes only
    // provider / model / api_key_env — never a `base_url` — and the daemon's
    // boot path resolves the primary driver from
    // `default_model.base_url.or(provider_urls.get(provider))` several
    // hundred lines before the model catalog that holds the gateway URL is
    // constructed. Without this the next boot builds the default driver with
    // no base URL and every turn on it fails, even though the catalog file on
    // disk has the address. Ordered first so a failure here surfaces before
    // agents are migrated onto a provider that could not resolve.
    let base_url = synthesis
        .file
        .provider
        .as_ref()
        .map(|p| p.base_url.clone())
        .unwrap_or_default();
    if !base_url.is_empty() {
        let (url_status, url_body) = daemon_json_checked(
            client
                .put(format!("{base}/api/providers/{PROVIDER_ID}/url"))
                .json(&serde_json::json!({ "url": base_url }))
                .send(),
        );
        if !url_status.is_success() || url_body.get("error").is_some() {
            ui::check_warn(&i18n::t("everyapi-connect-default-url-pin-failed"));
        }
    }
    let (status, body) = daemon_json_checked(
        client
            .post(format!("{base}/api/providers/{PROVIDER_ID}/default"))
            .json(&serde_json::json!({ "model": model }))
            .send(),
    );
    // 207 is a documented partial success on this route (default switched,
    // some agents failed to migrate), so any 2xx without an `error` counts.
    if status.is_success() && body.get("error").is_none() {
        ui::success(&i18n::t_args(
            "everyapi-connect-default-set",
            &[("model", &model)],
        ));
    } else {
        ui::error(&i18n::t_args(
            "everyapi-connect-default-failed",
            &[("model", &model), ("status", &status.to_string())],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        choose_default_model, classify_daemon_write, classify_fetch_status, derive_base_url,
        infer_modality_with_context, is_openai_response_only, model_request_value,
        normalize_version_separators, parse_credentials, parse_gateway_models, parse_pricing_rows,
        provider_request_body, push_key_to_daemon, resolve_metadata, snapshot_lookup_ids,
        synthesize_catalog, DaemonWrite, GatewayModel, ModelFetchError, PricingRow,
    };
    use crate::cli::{Cli, Commands, ModelsCommands};
    use clap::Parser;
    use librefang_runtime::model_catalog::ModelCatalog;
    use librefang_types::model_catalog::{
        Modality, ModelCatalogEntry, ModelCatalogFile, ModelTier,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A catalog built over an empty directory.
    /// The OpenRouter snapshot is `include_str!`-ed into `librefang-runtime` and merged unconditionally by every constructor, so this is a hermetic handle on the snapshot with no user providers mixed in.
    fn snapshot_catalog() -> ModelCatalog {
        let dir = tempfile::tempdir().expect("tempdir");
        ModelCatalog::new(dir.path())
    }

    fn gateway_model(id: &str, owned_by: &str, endpoints: &[&str]) -> GatewayModel {
        GatewayModel {
            id: id.to_string(),
            owned_by: owned_by.to_string(),
            supported_endpoint_types: endpoints.iter().map(|s| s.to_string()).collect(),
            context_window: None,
            max_output_tokens: None,
        }
    }

    /// A per-token pricing row, the common case.
    fn priced(model_name: &str, model_ratio: f64, completion_ratio: f64) -> PricingRow {
        PricingRow {
            model_name: model_name.to_string(),
            per_token: true,
            model_ratio,
            completion_ratio,
            context_window: None,
        }
    }

    fn pricing_index(rows: Vec<PricingRow>) -> BTreeMap<String, PricingRow> {
        rows.into_iter()
            .map(|row| (row.model_name.clone(), row))
            .collect()
    }

    /// The real 19-row `/api/pricing` listing, live-captured.
    /// Note it carries `claude-fable-5`, which the model listing does not — the two feeds genuinely differ in both directions.
    fn live_pricing_rows() -> BTreeMap<String, PricingRow> {
        let per_call = |model_name: &str| PricingRow {
            model_name: model_name.to_string(),
            per_token: false,
            model_ratio: 0.0,
            completion_ratio: 0.0,
            context_window: None,
        };
        let claude = |model_name: &str, ratio: f64| PricingRow {
            model_name: model_name.to_string(),
            per_token: true,
            model_ratio: ratio,
            completion_ratio: 5.0,
            context_window: Some(200_000),
        };
        pricing_index(vec![
            priced("MiniMax-M3", 0.15, 4.0),
            claude("claude-fable-5", 5.0),
            claude("claude-haiku-4-5", 0.5),
            claude("claude-opus-5", 2.5),
            claude("claude-sonnet-5", 1.0),
            per_call("doubao-seedance-1-0-pro-250528"),
            per_call("doubao-seedance-1-0-pro-fast-251015"),
            per_call("doubao-seedance-2-0-260128"),
            per_call("doubao-seedance-2-0-fast-260128"),
            per_call("doubao-seedance-2-0-mini-260615"),
            per_call("doubao-seedream-4-0-250828"),
            priced("gemini-3-flash", 0.75, 6.0),
            priced("gemini-3.1-pro-low", 1.0, 5.0),
            priced("gemini-3.5-flash", 0.75, 6.0),
            priced("gpt-5.6-luna", 0.5, 6.0),
            priced("gpt-5.6-sol", 2.5, 6.0),
            priced("gpt-5.6-terra", 1.25, 6.0),
            priced("tts-1", 7.5, 0.0),
            priced("tts-1-hd", 15.0, 0.0),
        ])
    }

    // ── credentials ───────────────────────────────────────────────────────

    #[test]
    fn credentials_parse_the_real_on_disk_shape() {
        // Exactly the field set EveryAPI writes — note there is no
        // `expires_at` / `refresh_token`, and the extra fields must not
        // prevent parsing.
        let raw = json!({
            "access_token": "at",
            "api_base": "https://api.everyapi.ai",
            "relay_key": "rk-secret",
            "role": "user",
            "user_id": "u1",
            "username": "someone"
        })
        .to_string();
        let parsed = parse_credentials(&raw).expect("parses");
        assert_eq!(parsed.api_base, "https://api.everyapi.ai");
        assert_eq!(parsed.relay_key, "rk-secret");
    }

    #[test]
    fn credentials_tolerate_unknown_future_fields() {
        let raw = json!({
            "api_base": "https://api.everyapi.ai",
            "relay_key": "rk",
            "some_field_added_next_year": { "nested": [1, 2, 3] }
        })
        .to_string();
        assert_eq!(parse_credentials(&raw).expect("parses").relay_key, "rk");
    }

    #[test]
    fn credentials_missing_relay_key_is_an_error() {
        let raw = json!({ "api_base": "https://api.everyapi.ai" }).to_string();
        assert_eq!(
            parse_credentials(&raw).err(),
            Some("everyapi-connect-credentials-no-relay-key")
        );
    }

    #[test]
    fn credentials_blank_relay_key_is_an_error() {
        let raw = json!({ "api_base": "https://api.everyapi.ai", "relay_key": "   " }).to_string();
        assert_eq!(
            parse_credentials(&raw).err(),
            Some("everyapi-connect-credentials-no-relay-key")
        );
    }

    #[test]
    fn credentials_missing_api_base_is_an_error() {
        let raw = json!({ "relay_key": "rk" }).to_string();
        assert_eq!(
            parse_credentials(&raw).err(),
            Some("everyapi-connect-credentials-no-api-base")
        );
    }

    #[test]
    fn credentials_malformed_json_errors_instead_of_panicking() {
        assert_eq!(
            parse_credentials("{not json at all").err(),
            Some("everyapi-connect-credentials-malformed")
        );
        assert_eq!(
            parse_credentials("").err(),
            Some("everyapi-connect-credentials-malformed")
        );
    }

    // ── base_url ──────────────────────────────────────────────────────────

    #[test]
    fn base_url_gains_exactly_one_v1_segment() {
        assert_eq!(
            derive_base_url("https://api.everyapi.ai"),
            "https://api.everyapi.ai/v1"
        );
        assert_eq!(
            derive_base_url("https://api.everyapi.ai/"),
            "https://api.everyapi.ai/v1"
        );
        assert_eq!(
            derive_base_url("  https://api.everyapi.ai///  "),
            "https://api.everyapi.ai/v1"
        );
    }

    // ── modality ──────────────────────────────────────────────────────────

    #[test]
    fn modality_is_inferred_from_supported_endpoint_types() {
        let m = |types: &[&str]| {
            infer_modality_with_context(
                &types.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                None,
            )
        };
        assert_eq!(m(&["image-generation", "openai"]), Modality::Image);
        assert_eq!(m(&["audio-speech"]), Modality::Audio);
        assert_eq!(m(&[]), Modality::Video);
        assert_eq!(m(&["openai", "anthropic"]), Modality::Text);
        assert_eq!(m(&["openai-response"]), Modality::Text);
    }

    #[test]
    fn openai_response_only_detection() {
        let f = |types: &[&str]| {
            is_openai_response_only(&types.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert!(f(&["openai-response"]));
        assert!(!f(&["openai-response", "openai"]));
        assert!(!f(&["openai"]));
        // An empty list is a video model, not a streaming-only text model.
        assert!(!f(&[]));
    }

    // ── gateway response parsing ──────────────────────────────────────────

    #[test]
    fn gateway_models_parse_and_drop_unusable_rows() {
        let body = json!({
            "object": "list",
            "success": true,
            "data": [
                { "id": "claude-haiku-4-5", "owned_by": "anthropic",
                  "supported_endpoint_types": ["openai", "anthropic"],
                  "context_window": 200000 },
                { "id": "tts-1", "owned_by": "system",
                  "supported_endpoint_types": ["audio-speech"] },
                { "owned_by": "anthropic" },
                { "id": "   ", "owned_by": "anthropic" }
            ]
        });
        let models = parse_gateway_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-haiku-4-5");
        assert_eq!(models[0].context_window, Some(200_000));
        assert_eq!(models[1].id, "tts-1");
        assert_eq!(models[1].context_window, None);
    }

    #[test]
    fn gateway_models_without_a_data_array_yield_nothing() {
        assert!(parse_gateway_models(&json!({ "error": "nope" })).is_empty());
    }

    #[test]
    fn gateway_max_output_is_read_when_the_listing_publishes_one() {
        // No observed account populates `max_output`, but the field is part
        // of the gateway's published DTO. When it arrives it must be read,
        // otherwise the snapshot's number would silently outrank the
        // gateway's own statement about its deployment.
        let body = json!({
            "data": [
                { "id": "with-limit", "owned_by": "vendor",
                  "supported_endpoint_types": ["openai"],
                  "context_window": 128000, "max_output": 8192 },
                { "id": "zero-limit", "owned_by": "vendor",
                  "supported_endpoint_types": ["openai"], "max_output": 0 },
                { "id": "no-limit", "owned_by": "vendor",
                  "supported_endpoint_types": ["openai"] }
            ]
        });
        let models = parse_gateway_models(&body);
        assert_eq!(models[0].max_output_tokens, Some(8_192));
        // Zero means "unknown", not "no output allowed".
        assert_eq!(models[1].max_output_tokens, None);
        assert_eq!(models[2].max_output_tokens, None);
    }

    // ── pricing feed parsing ──────────────────────────────────────────────

    #[test]
    fn pricing_rows_parse_the_real_feed_shape() {
        let body = json!({
            "success": true,
            "pricing_version": "dbb3811c",
            "group_ratio": { "grp_M3K-NEhOUc": 0.25 },
            "data": [
                { "model_name": "claude-sonnet-5", "quota_type": 0, "model_ratio": 1,
                  "completion_ratio": 5, "model_price": 0, "cache_ratio": 0.1,
                  "context_window": 200000, "billing_mode": "per_token" },
                { "model_name": "doubao-seedream-4-0-250828", "quota_type": 1,
                  "model_ratio": 0, "completion_ratio": 0, "model_price": 0.028,
                  "billing_mode": "per_call" },
                { "model_name": "   ", "quota_type": 0 },
                { "quota_type": 0, "model_ratio": 3 }
            ]
        });
        let rows = parse_pricing_rows(&body);
        // The two unusable rows are dropped, the good ones survive.
        assert_eq!(rows.len(), 2);

        let sonnet = &rows["claude-sonnet-5"];
        assert!(sonnet.per_token);
        assert_eq!(sonnet.context_window, Some(200_000));
        // The mandated sanity check, asserted through the real JSON parse
        // rather than a hand-built struct: the feed ships these ratios as
        // JSON *integers*, so this is what proves the `as_f64` accessor
        // does not silently yield 0.0 and price the model as free.
        assert_eq!(sonnet.input_cost_per_m(), 2.00);
        assert_eq!(sonnet.output_cost_per_m(), 10.00);

        let seedream = &rows["doubao-seedream-4-0-250828"];
        assert!(!seedream.per_token);
        // Zero in the feed means "unknown", not "no context".
        assert_eq!(seedream.context_window, None);
    }

    #[test]
    fn a_negative_or_non_finite_model_ratio_is_never_priced() {
        // A malformed or compromised `/api/pricing` response carrying a
        // negative `model_ratio` must not flow into `input_cost_per_m()` /
        // `output_cost_per_m()` as a negative, confidently-known price — that
        // would actively corrupt downstream budget math rather than merely
        // look free. NaN/infinity get the same treatment.
        let body = json!({ "data": [
            { "model_name": "negative-ratio", "quota_type": 0, "model_ratio": -5.0, "completion_ratio": 5.0 },
            { "model_name": "nan-ratio", "quota_type": 0, "model_ratio": f64::NAN, "completion_ratio": 5.0 },
            { "model_name": "infinite-ratio", "quota_type": 0, "model_ratio": f64::INFINITY, "completion_ratio": 5.0 },
            { "model_name": "negative-completion-ratio", "quota_type": 0, "model_ratio": 1.0, "completion_ratio": -5.0 },
        ]});
        let rows = parse_pricing_rows(&body);
        // These three have a bad `model_ratio`, so both cost fields must
        // stay at zero AND `per_token` must be false — otherwise the 0.0
        // fallback would be asserted as a *known* free price instead of an
        // unknown one.
        for id in ["negative-ratio", "nan-ratio", "infinite-ratio"] {
            let row = &rows[id];
            assert_eq!(row.input_cost_per_m(), 0.0, "{id} input");
            assert_eq!(row.output_cost_per_m(), 0.0, "{id} output");
            assert!(!row.per_token, "{id} per_token");
        }
        // Only `model_ratio` gates `per_token`; a bad `completion_ratio`
        // alone still means "known input price, zero output" — matching how
        // the tts family (completion_ratio 0.0) is treated elsewhere.
        assert!(rows["negative-completion-ratio"].per_token);
        assert_eq!(rows["negative-completion-ratio"].input_cost_per_m(), 2.0);
    }

    #[test]
    fn pricing_rows_without_a_data_array_yield_nothing() {
        assert!(parse_pricing_rows(&json!({ "success": false })).is_empty());
    }

    #[test]
    fn either_per_call_signal_alone_marks_a_row_as_not_per_token() {
        // `quota_type` is what the gateway's billing code branches on and
        // `billing_mode` is its human-readable mirror; a feed shipping only
        // one of them must still bill correctly.
        let body = json!({ "data": [
            { "model_name": "quota-only", "quota_type": 1, "model_ratio": 0 },
            { "model_name": "mode-only", "billing_mode": "per_call", "model_ratio": 0 },
            { "model_name": "neither", "quota_type": 0, "billing_mode": "per_token",
              "model_ratio": 1, "completion_ratio": 5 }
        ]});
        let rows = parse_pricing_rows(&body);
        assert!(!rows["quota-only"].per_token);
        assert!(!rows["mode-only"].per_token);
        assert!(rows["neither"].per_token);
    }

    // ── ratio → USD conversion ────────────────────────────────────────────

    #[test]
    fn billing_ratios_convert_to_usd_per_million_tokens() {
        // The gateway's anchor case: ratio 1 = $2/1M in, and the completion
        // ratio multiplies on top of it, not instead of it.
        let sonnet = priced("claude-sonnet-5", 1.0, 5.0);
        assert_eq!(sonnet.input_cost_per_m(), 2.00);
        assert_eq!(sonnet.output_cost_per_m(), 10.00);

        // A second, independent anchor with a non-integer ratio and a
        // different completion ratio.
        let terra = priced("gpt-5.6-terra", 1.25, 6.0);
        assert_eq!(terra.input_cost_per_m(), 2.5);
        assert_eq!(terra.output_cost_per_m(), 15.0);

        let minimax = priced("MiniMax-M3", 0.15, 4.0);
        assert_eq!(minimax.input_cost_per_m(), 0.3);
        assert!((minimax.output_cost_per_m() - 1.2).abs() < 1e-12);

        // completion_ratio 0 (the tts family) prices input only — it is not
        // a signal that the model is unpriced.
        let tts = priced("tts-1", 7.5, 0.0);
        assert_eq!(tts.input_cost_per_m(), 15.0);
        assert_eq!(tts.output_cost_per_m(), 0.0);
    }

    #[test]
    fn per_call_rows_carry_no_per_token_price() {
        // `model_price` is a flat per-request charge in USD. Forcing it into
        // a per-million-token field would misreport cost by orders of
        // magnitude, so both figures stay at zero and the caller marks the
        // model `pricing_known = false`.
        let row = PricingRow {
            model_name: "doubao-seedance-2-0-260128".to_string(),
            per_token: false,
            // Even if the feed shipped non-zero ratios on a per-call row,
            // they must not leak into a per-token figure.
            model_ratio: 3.0,
            completion_ratio: 4.0,
            context_window: None,
        };
        assert_eq!(row.input_cost_per_m(), 0.0);
        assert_eq!(row.output_cost_per_m(), 0.0);
    }

    // ── metadata resolution ───────────────────────────────────────────────

    #[test]
    fn version_separator_normalization_only_touches_digit_hyphen_digit() {
        assert_eq!(
            normalize_version_separators("claude-haiku-4-5"),
            "claude-haiku-4.5"
        );
        assert_eq!(normalize_version_separators("gpt-5.6-luna"), "gpt-5.6-luna");
        assert_eq!(
            normalize_version_separators("claude-sonnet-5"),
            "claude-sonnet-5"
        );
        assert_eq!(
            normalize_version_separators("doubao-seedance-1-0-pro-250528"),
            "doubao-seedance-1.0-pro-250528"
        );
    }

    #[test]
    fn snapshot_lookup_ids_add_a_normalized_retry_only_when_it_differs() {
        assert_eq!(
            snapshot_lookup_ids("anthropic", "claude-sonnet-5"),
            vec!["openrouter/anthropic/claude-sonnet-5"]
        );
        assert_eq!(
            snapshot_lookup_ids("anthropic", "claude-haiku-4-5"),
            vec![
                "openrouter/anthropic/claude-haiku-4-5",
                "openrouter/anthropic/claude-haiku-4.5"
            ]
        );
        assert!(snapshot_lookup_ids("", "claude-sonnet-5").is_empty());
    }

    #[test]
    fn pricing_comes_from_the_gateway_and_token_limits_from_the_snapshot() {
        let catalog = snapshot_catalog();
        let row = priced("claude-sonnet-5", 1.0, 5.0);
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("claude-sonnet-5", "anthropic", &["openai", "anthropic"]),
            Some(&row),
        );
        // The gateway's own book, not the snapshot's copy of the vendor's.
        assert_eq!(meta.input_cost_per_m, 2.0);
        assert_eq!(meta.output_cost_per_m, 10.0);
        assert!(meta.pricing_known);
        // Neither gateway endpoint published a limit here, so both are
        // borrowed and the caller must be told.
        assert_eq!(meta.max_output_tokens, 128_000);
        assert_eq!(meta.context_window, 1_000_000);
        assert!(meta.token_limits_borrowed);
        // Capability flags have no gateway source either.
        assert!(meta.supports_tools);
    }

    #[test]
    fn the_snapshot_is_never_consulted_for_pricing() {
        // `anthropic/claude-sonnet-5` sits in the snapshot at 2.0/10.0. A
        // model absent from the gateway's pricing feed must still come out
        // unpriced rather than inheriting the upstream vendor's list price.
        let catalog = snapshot_catalog();
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("claude-sonnet-5", "anthropic", &["openai"]),
            None,
        );
        assert_eq!(meta.input_cost_per_m, 0.0);
        assert_eq!(meta.output_cost_per_m, 0.0);
        assert!(
            !meta.pricing_known,
            "a zero cost must be reported as unknown, not as free"
        );
        // Token limits still resolve, so the model stays registrable.
        assert_eq!(meta.context_window, 1_000_000);
        assert_eq!(meta.max_output_tokens, 128_000);
    }

    #[test]
    fn pricing_feed_context_window_beats_the_snapshot() {
        // The gateway bills claude-sonnet-5 at a 200K window; the snapshot
        // describes OpenRouter's 1M copy. Taking the snapshot's number would
        // over-report the usable window 5x straight into compaction
        // thresholds.
        let catalog = snapshot_catalog();
        let mut row = priced("claude-sonnet-5", 1.0, 5.0);
        row.context_window = Some(200_000);
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("claude-sonnet-5", "anthropic", &["openai"]),
            Some(&row),
        );
        assert_eq!(meta.context_window, 200_000);
        // Only max_output was borrowed, but that is still a borrow.
        assert_eq!(meta.max_output_tokens, 128_000);
        assert!(meta.token_limits_borrowed);
    }

    #[test]
    fn model_listing_context_window_beats_the_pricing_feed() {
        // The model's own row on /v1/models is the most specific statement
        // about this gateway, so it outranks both other sources.
        let catalog = snapshot_catalog();
        let mut model = gateway_model("claude-sonnet-5", "anthropic", &["openai"]);
        model.context_window = Some(64_000);
        let mut row = priced("claude-sonnet-5", 1.0, 5.0);
        row.context_window = Some(200_000);
        let meta = resolve_metadata(&catalog, &model, Some(&row));
        assert_eq!(meta.context_window, 64_000);
    }

    #[test]
    fn a_published_gateway_max_output_is_not_borrowed() {
        let catalog = snapshot_catalog();
        let mut model = gateway_model("claude-sonnet-5", "anthropic", &["openai"]);
        model.context_window = Some(200_000);
        model.max_output_tokens = Some(32_000);
        let row = priced("claude-sonnet-5", 1.0, 5.0);
        let meta = resolve_metadata(&catalog, &model, Some(&row));
        assert_eq!(meta.max_output_tokens, 32_000, "gateway beats the snapshot");
        assert!(
            !meta.token_limits_borrowed,
            "both limits came from the gateway"
        );
    }

    #[test]
    fn snapshot_lookup_still_normalizes_the_version_tail_and_case() {
        // The snapshot fallback is what keeps these models registrable, so
        // both id-shape fixes must keep resolving.
        let catalog = snapshot_catalog();
        // Gateway "claude-haiku-4-5" vs snapshot "anthropic/claude-haiku-4.5".
        let haiku = resolve_metadata(
            &catalog,
            &gateway_model("claude-haiku-4-5", "anthropic", &["openai"]),
            None,
        );
        assert_eq!(haiku.max_output_tokens, 64_000);
        // Gateway "MiniMax-M3" vs snapshot "minimax/minimax-m3".
        let minimax = resolve_metadata(
            &catalog,
            &gateway_model("MiniMax-M3", "minimax", &["openai"]),
            None,
        );
        assert_eq!(minimax.context_window, 1_048_576);
        assert_eq!(minimax.max_output_tokens, 512_000);
    }

    #[test]
    fn a_model_absent_from_both_sources_resolves_to_nothing_registrable() {
        // gemini-3-flash is in the gateway's listing and pricing feed but is
        // in no snapshot, and neither gateway source publishes a token
        // limit. Pricing still resolves; the limits do not.
        let catalog = snapshot_catalog();
        let row = priced("gemini-3-flash", 0.75, 6.0);
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("gemini-3-flash", "google", &["openai"]),
            Some(&row),
        );
        assert_eq!(meta.input_cost_per_m, 1.5);
        assert_eq!(meta.output_cost_per_m, 9.0);
        assert!(meta.pricing_known);
        assert_eq!(meta.context_window, 0);
        assert_eq!(meta.max_output_tokens, 0);
        assert!(
            !meta.token_limits_borrowed,
            "nothing was borrowed — there was nothing to borrow"
        );
    }

    // ── catalog synthesis ─────────────────────────────────────────────────

    /// The real 18-model gateway listing, reduced to (id, owned_by, endpoints).
    fn live_gateway_listing() -> Vec<GatewayModel> {
        vec![
            gateway_model("claude-haiku-4-5", "anthropic", &["openai", "anthropic"]),
            gateway_model("claude-opus-5", "anthropic", &["openai", "anthropic"]),
            gateway_model("claude-sonnet-5", "anthropic", &["openai", "anthropic"]),
            gateway_model("gpt-5.6-luna", "openai", &["openai-response"]),
            gateway_model("gpt-5.6-sol", "openai", &["openai-response"]),
            gateway_model("gpt-5.6-terra", "openai", &["openai-response"]),
            gateway_model("gemini-3-flash", "google", &["openai"]),
            gateway_model("gemini-3.1-pro-low", "google", &["openai"]),
            gateway_model("gemini-3.5-flash", "google", &["openai"]),
            gateway_model("MiniMax-M3", "minimax", &["openai"]),
            gateway_model("doubao-seedance-1-0-pro-250528", "doubao", &[]),
            gateway_model("doubao-seedance-1-0-pro-fast-251015", "doubao", &[]),
            gateway_model("doubao-seedance-2-0-260128", "doubao", &[]),
            gateway_model("doubao-seedance-2-0-fast-260128", "doubao", &[]),
            gateway_model("doubao-seedance-2-0-mini-260615", "doubao", &[]),
            gateway_model(
                "doubao-seedream-4-0-250828",
                "doubao",
                &["image-generation", "openai"],
            ),
            gateway_model("tts-1", "system", &["audio-speech"]),
            gateway_model("tts-1-hd", "system", &["audio-speech"]),
        ]
    }

    #[test]
    fn text_model_without_resolvable_token_limits_is_skipped_not_emitted_with_zero() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );

        // The gateway prices both gemini models but publishes a token limit
        // for neither, and neither is in the snapshot — so pricing alone
        // does not rescue them.
        let skipped: Vec<&str> = result.skipped.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(skipped, vec!["gemini-3-flash", "gemini-3.1-pro-low"]);
        for skip in &result.skipped {
            assert_eq!(skip.reason_key, "everyapi-connect-skip-no-metadata");
        }
        // Skipped ids must not appear in the emitted entries at all.
        for id in &skipped {
            assert!(
                !result.file.models.iter().any(|m| &m.id == id),
                "{id} must not be emitted"
            );
        }
        // No emitted text entry may carry a zero token field — that shape
        // parses but is dropped by `merge_catalog_file`.
        for model in result
            .file
            .models
            .iter()
            .filter(|m| m.modality == Modality::Text)
        {
            assert!(model.context_window > 0, "{} ctx", model.id);
            assert!(model.max_output_tokens > 0, "{} maxout", model.id);
        }
        assert_eq!(result.file.models.len(), 16);
    }

    #[test]
    fn a_priced_text_model_with_no_token_limits_anywhere_is_still_skipped() {
        // Pricing resolves but neither token limit does, and `validate()`
        // rejects a text entry missing either. Emitting it would look like
        // success while the model vanished at the next daemon boot.
        let catalog = snapshot_catalog();
        let model = gateway_model("totally-unknown-model", "someone", &["openai"]);
        let pricing = pricing_index(vec![priced("totally-unknown-model", 1.0, 5.0)]);
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            std::slice::from_ref(&model),
            &pricing,
        );
        assert!(result.file.models.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].id, "totally-unknown-model");
        assert_eq!(
            result.skipped[0].reason_key,
            "everyapi-connect-skip-no-metadata"
        );

        // A context window alone does not rescue it either: `max_output` is
        // unobtainable from any source for this model. The reason must now
        // name the output limit specifically — claiming the context window
        // is unknown would be false here.
        let mut with_ctx = model.clone();
        with_ctx.context_window = Some(128_000);
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &[with_ctx],
            &pricing,
        );
        assert!(result.file.models.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(
            result.skipped[0].reason_key,
            "everyapi-connect-skip-no-output-limit"
        );
    }

    #[test]
    fn a_pricing_feed_context_window_without_an_output_limit_names_the_missing_one() {
        // The realistic shape: a claude-family model priced by the gateway
        // (so its 200K window resolves from the feed) that the compiled-in
        // snapshot has not caught up with, leaving `max_output_tokens`
        // unresolvable. It is still skipped, but for the right stated
        // reason.
        let catalog = snapshot_catalog();
        let mut row = priced("claude-nonesuch-9", 1.0, 5.0);
        row.context_window = Some(200_000);
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &[gateway_model("claude-nonesuch-9", "anthropic", &["openai"])],
            &pricing_index(vec![row]),
        );
        assert!(result.file.models.is_empty());
        assert_eq!(
            result.skipped[0].reason_key,
            "everyapi-connect-skip-no-output-limit"
        );
    }

    #[test]
    fn a_known_output_limit_without_a_context_window_names_the_missing_one() {
        // The mirror case: the gateway published `max_output` but no
        // context window, and no snapshot covers the model.
        let catalog = snapshot_catalog();
        let mut model = gateway_model("totally-unknown-model", "someone", &["openai"]);
        model.max_output_tokens = Some(8_192);
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &[model],
            &BTreeMap::new(),
        );
        assert!(result.file.models.is_empty());
        assert_eq!(
            result.skipped[0].reason_key,
            "everyapi-connect-skip-no-context-window"
        );
    }

    #[test]
    fn a_gateway_model_absent_from_the_pricing_feed_registers_unpriced() {
        // The two listings genuinely differ in both directions — the live
        // pricing feed carries `claude-fable-5`, which `/v1/models` omits —
        // so the reverse must also degrade gracefully. Token limits come
        // from the snapshot, so the model stays usable; only its price is
        // recorded as unknown.
        let catalog = snapshot_catalog();
        let models = vec![gateway_model(
            "claude-opus-5",
            "anthropic",
            &["openai", "anthropic"],
        )];
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &models,
            &BTreeMap::new(),
        );
        assert!(result.skipped.is_empty());
        let entry = &result.file.models[0];
        assert_eq!(entry.input_cost_per_m, 0.0);
        assert_eq!(entry.output_cost_per_m, 0.0);
        assert!(!entry.pricing_known);
        assert!(entry.context_window > 0 && entry.max_output_tokens > 0);
        assert_eq!(result.unpriced, vec!["claude-opus-5"]);
    }

    #[test]
    fn a_pricing_row_with_no_matching_gateway_model_is_ignored() {
        // `claude-fable-5` is priced but not listed. Pricing must not
        // conjure a model the gateway never offered.
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        assert!(
            !result.file.models.iter().any(|m| m.id == "claude-fable-5"),
            "a pricing-only row must not become a registered model"
        );
    }

    #[test]
    fn gateway_pricing_lands_on_the_emitted_entries() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        let entry = |id: &str| {
            result
                .file
                .models
                .iter()
                .find(|m| m.id == id)
                .unwrap_or_else(|| panic!("{id} registered"))
                .clone()
        };

        let sonnet = entry("claude-sonnet-5");
        assert_eq!(sonnet.input_cost_per_m, 2.0);
        assert_eq!(sonnet.output_cost_per_m, 10.0);
        assert!(sonnet.pricing_known);
        // The pricing feed's 200K window beats the snapshot's 1M copy.
        assert_eq!(sonnet.context_window, 200_000);

        // tts-1 has no snapshot presence at all, but the gateway prices it,
        // so it is no longer registered as unpriced.
        let tts = entry("tts-1");
        assert_eq!(tts.input_cost_per_m, 15.0);
        assert_eq!(tts.output_cost_per_m, 0.0);
        assert!(tts.pricing_known);
    }

    #[test]
    fn non_text_models_survive_without_any_token_limits() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        let by_modality = |modality: Modality| {
            result
                .file
                .models
                .iter()
                .filter(|m| m.modality == modality)
                .count()
        };
        assert_eq!(by_modality(Modality::Video), 5, "doubao-seedance-*");
        assert_eq!(by_modality(Modality::Image), 1, "doubao-seedream-4-0");
        assert_eq!(by_modality(Modality::Audio), 2, "tts-1, tts-1-hd");
        assert_eq!(by_modality(Modality::Text), 8);
    }

    #[test]
    fn per_call_models_are_unpriced_rather_than_free() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        for model in &result.file.models {
            if model.input_cost_per_m == 0.0 && model.output_cost_per_m == 0.0 {
                assert!(
                    !model.pricing_known,
                    "{} claims known pricing with zero cost",
                    model.id
                );
            }
        }
        // The doubao family is billed per call, so it has no per-token
        // price to report at all.
        assert_eq!(
            result.unpriced,
            vec![
                "doubao-seedance-1-0-pro-250528",
                "doubao-seedance-1-0-pro-fast-251015",
                "doubao-seedance-2-0-260128",
                "doubao-seedance-2-0-fast-260128",
                "doubao-seedance-2-0-mini-260615",
                "doubao-seedream-4-0-250828",
            ]
        );
        for id in &result.unpriced {
            let entry = result
                .file
                .models
                .iter()
                .find(|m| &m.id == id)
                .expect("registered");
            assert_eq!(entry.input_cost_per_m, 0.0);
            assert_eq!(entry.output_cost_per_m, 0.0);
        }
    }

    #[test]
    fn borrowed_token_limits_are_reported_and_gateway_supplied_ones_are_not() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        // Every registered text model borrows at least `max_output_tokens`:
        // no gateway source publishes one.
        let text_ids: Vec<&str> = result
            .file
            .models
            .iter()
            .filter(|m| m.modality == Modality::Text)
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(result.borrowed_token_limits, text_ids);
        // Non-text entries borrow nothing — they need no token limits.
        assert!(!result
            .borrowed_token_limits
            .iter()
            .any(|id| id.starts_with("tts-") || id.starts_with("doubao-")));
    }

    #[test]
    fn a_fully_self_described_model_borrows_nothing() {
        let catalog = snapshot_catalog();
        let mut model = gateway_model("claude-sonnet-5", "anthropic", &["openai"]);
        model.context_window = Some(200_000);
        model.max_output_tokens = Some(64_000);
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &[model],
            &pricing_index(vec![priced("claude-sonnet-5", 1.0, 5.0)]),
        );
        assert!(result.borrowed_token_limits.is_empty());
        assert!(result.unpriced.is_empty());
        assert_eq!(result.file.models[0].context_window, 200_000);
        assert_eq!(result.file.models[0].max_output_tokens, 64_000);
    }

    #[test]
    fn output_is_byte_identical_regardless_of_input_order() {
        let catalog = snapshot_catalog();
        let forward = live_gateway_listing();
        let mut reversed = forward.clone();
        reversed.reverse();
        let pricing = live_pricing_rows();

        let a = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &forward, &pricing);
        let b = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &reversed, &pricing);

        let render = |file: &ModelCatalogFile| toml::to_string_pretty(file).expect("serializes");
        assert_eq!(render(&a.file), render(&b.file));
        assert_eq!(a.skipped, b.skipped);
        assert_eq!(a.streaming_only, b.streaming_only);
        assert_eq!(a.borrowed_token_limits, b.borrowed_token_limits);
        assert_eq!(a.unpriced, b.unpriced);
    }

    /// A 401 must not be treated as a transient outage: the remediation is `everyapi login`, not "check the gateway is reachable", and the caller stops rather than persisting a credential that fails every request.
    #[test]
    fn an_unauthorized_status_is_distinguished_from_an_outage() {
        assert_eq!(classify_fetch_status(401), ModelFetchError::Unauthorized);
        assert_eq!(classify_fetch_status(403), ModelFetchError::Unauthorized);
        assert_eq!(classify_fetch_status(500), ModelFetchError::Unreachable);
        assert_eq!(classify_fetch_status(503), ModelFetchError::Unreachable);
    }

    /// A daemon that *understood* the payload and refused it must not be followed by the direct file write — the registry route deletes the file it rejected, so falling back would restore the very definition that fails to parse on every subsequent boot.
    #[test]
    fn a_daemon_rejection_is_not_a_fallback_signal() {
        assert_eq!(
            classify_daemon_write(200, &json!({"path": "providers/everyapi.toml"})),
            DaemonWrite::Accepted
        );
        assert_eq!(
            classify_daemon_write(400, &json!({"error": "rejected and not saved"})),
            DaemonWrite::Rejected("rejected and not saved".to_string())
        );
        // 5xx means the daemon is up but broken; a local file still beats
        // nothing, so that one does fall back.
        assert_eq!(
            classify_daemon_write(502, &json!({})),
            DaemonWrite::Unreachable
        );
    }

    /// A published context window is positive evidence of a text model, so a gateway that stops sending `supported_endpoint_types` does not get its chat models silently registered as video (exempt from the token-limit validation, and unusable for chat while looking present).
    #[test]
    fn an_empty_endpoint_list_defers_to_the_context_window() {
        assert_eq!(infer_modality_with_context(&[], None), Modality::Video);
        assert_eq!(infer_modality_with_context(&[], Some(0)), Modality::Video);
        assert_eq!(
            infer_modality_with_context(&[], Some(200_000)),
            Modality::Text
        );
    }

    /// The provider must declare the capabilities its own entries imply, or the media driver cache treats every registered image / audio / video model as unreachable.
    #[test]
    fn media_capabilities_follow_the_registered_modalities() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        let caps = &result
            .file
            .provider
            .as_ref()
            .expect("provider")
            .media_capabilities;
        assert!(caps.contains(&"image_generation".to_string()));
        assert!(caps.contains(&"text_to_speech".to_string()));
        assert!(caps.contains(&"video_generation".to_string()));
        // Sorted and deduplicated so the generated TOML is byte-stable.
        let mut sorted = caps.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(caps, &sorted);
    }

    #[test]
    fn default_choice_avoids_the_streaming_only_family() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        assert_eq!(
            result.streaming_only,
            vec!["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.6-terra"]
        );
        // Ranked by capability, not by id order. `claude-opus-5` is the
        // priciest non-streaming-only text model on the live listing
        // ($5.00/M input), so it wins over the id-first `MiniMax-M3`
        // ($0.30/M) that a take-the-first rule would have picked.
        let chosen =
            choose_default_model(&result.file.models, &result.streaming_only).expect("a default");
        assert_eq!(chosen, "claude-opus-5");
        assert!(!result.streaming_only.contains(&chosen));
    }

    #[test]
    fn default_choice_falls_back_when_only_streaming_only_models_exist() {
        let catalog = snapshot_catalog();
        let only_response_models = vec![
            gateway_model("gpt-5.6-sol", "openai", &["openai-response"]),
            gateway_model("gpt-5.6-luna", "openai", &["openai-response"]),
        ];
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &only_response_models,
            &live_pricing_rows(),
        );
        // Still returns something rather than silently doing nothing, but the
        // caller has already warned about the streaming requirement. The
        // same capability ranking applies within the fallback set, so the
        // pricier `gpt-5.6-sol` ($5.00/M) beats `gpt-5.6-luna` ($1.00/M).
        assert_eq!(
            choose_default_model(&result.file.models, &result.streaming_only).as_deref(),
            Some("gpt-5.6-sol")
        );
    }

    #[test]
    fn default_choice_is_none_when_no_text_model_is_registered() {
        let catalog = snapshot_catalog();
        let media_only = vec![
            gateway_model("tts-1", "system", &["audio-speech"]),
            gateway_model("doubao-seedance-2-0-260128", "doubao", &[]),
        ];
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &media_only,
            &live_pricing_rows(),
        );
        assert_eq!(result.file.models.len(), 2);
        assert!(choose_default_model(&result.file.models, &result.streaming_only).is_none());
    }

    // ── round trip ────────────────────────────────────────────────────────

    #[test]
    fn generated_toml_round_trips_and_every_entry_validates() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        let rendered = toml::to_string_pretty(&result.file).expect("serializes");

        let parsed: ModelCatalogFile = toml::from_str(&rendered).expect("round trips");
        let provider = parsed.provider.expect("[provider] section present");
        assert_eq!(provider.id, "everyapi");
        assert_eq!(provider.display_name, "EveryAPI");
        assert_eq!(provider.api_key_env, "EVERYAPI_API_KEY");
        assert_eq!(provider.base_url, "https://api.everyapi.ai/v1");
        assert!(provider.key_required);

        assert_eq!(parsed.models.len(), result.file.models.len());
        for model in &parsed.models {
            model
                .validate()
                .unwrap_or_else(|e| panic!("entry rejected downstream: {e}"));
        }
    }

    #[test]
    fn registering_the_gateway_does_not_hijack_provider_blind_model_lookup() {
        // Several gateway ids collide with builtin ids (`claude-sonnet-5`,
        // `claude-opus-5`, `gemini-3.5-flash`). `find_model` returns the
        // FIRST `ModelTier::Custom` match immediately (#983), and
        // `merge_catalog_file` dedupes on `(id, provider)` — so tagging
        // these entries `Custom` would make every provider-blind lookup
        // (`pricing`, `effective_capabilities_for`, the last-resort arm of
        // `find_model_for_manifest`) resolve to the gateway's copy instead
        // of the vendor's, silently re-pricing agents that never opted in.
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        for model in &result.file.models {
            assert_ne!(
                model.tier,
                ModelTier::Custom,
                "{} must not claim Custom tier",
                model.id
            );
        }

        // Merging must leave a same-id builtin reachable by a provider-blind
        // lookup rather than shadowing it.
        let mut merged = snapshot_catalog();
        let builtin = ModelCatalogEntry {
            id: "claude-sonnet-5".to_string(),
            display_name: "Claude Sonnet 5".to_string(),
            provider: "anthropic".to_string(),
            tier: ModelTier::Frontier,
            context_window: 1_000_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 2.0,
            output_cost_per_m: 10.0,
            ..Default::default()
        };
        merged.merge_catalog_file(ModelCatalogFile {
            provider: None,
            models: vec![builtin],
        });
        merged.merge_catalog_file(ModelCatalogFile {
            provider: result.file.provider.clone(),
            models: result.file.models.clone(),
        });

        assert_eq!(
            merged
                .find_model("claude-sonnet-5")
                .map(|m| m.provider.as_str()),
            Some("anthropic"),
            "the gateway copy must not shadow the vendor's entry"
        );
        // The gateway copy is still reachable when a provider is named.
        assert_eq!(
            merged
                .find_model_for_provider("everyapi", "claude-sonnet-5")
                .map(|m| m.provider.as_str()),
            Some("everyapi")
        );
    }

    // ── daemon request body ───────────────────────────────────────────────

    /// Without a daemon there is nothing to push the key into, so the caller must be told to restart.
    /// The bug this guards: the daemon parses `~/.librefang/.env` exactly once at boot, so a key written by this command is invisible to an already-running daemon — the provider gets registered and is immediately unusable while the output claims no restart is needed.
    #[test]
    fn no_daemon_means_the_key_was_not_pushed() {
        assert!(!push_key_to_daemon(None, "rk-never-sent"));
    }

    #[test]
    fn daemon_body_is_flat_carries_no_secret_and_keeps_pricing_known() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &live_pricing_rows(),
        );
        let body = provider_request_body(&result.file);

        // Flat shape — the endpoint's `normalize_provider_body` does the
        // nesting, so a pre-nested `provider` key would be passed through
        // untouched and produce `[provider.provider]`.
        assert_eq!(body["id"], "everyapi");
        assert_eq!(body["base_url"], "https://api.everyapi.ai/v1");
        assert_eq!(body["api_key_env"], "EVERYAPI_API_KEY");
        assert!(body.get("provider").is_none());
        // The relay key lives in .env; sending it here would additionally
        // copy the secret into the daemon's secrets.env. The running daemon
        // is told about the key separately, via `push_key_to_daemon` — see
        // `no_daemon_means_the_key_was_not_pushed`.
        assert!(body.get("api_key").is_none());
        assert_eq!(
            body["models"].as_array().map(Vec::len),
            Some(result.file.models.len())
        );

        // `pricing_known` must be explicit on every entry: it defaults to
        // true on deserialize, so an omitted field on a zero-cost model
        // would assert the model is free.
        for model in body["models"].as_array().expect("models array") {
            assert!(model.get("pricing_known").is_some_and(|v| v.is_boolean()));
        }
        // tts-1 is priced by the gateway even though no snapshot knows it.
        let tts = result
            .file
            .models
            .iter()
            .find(|m| m.id == "tts-1")
            .expect("tts-1 registered");
        assert_eq!(model_request_value(tts)["pricing_known"], json!(true));
        assert_eq!(model_request_value(tts)["input_cost_per_m"], json!(15.0));
        assert_eq!(model_request_value(tts)["modality"], json!("audio"));

        // A per-call model must ship an explicit `pricing_known = false`
        // alongside its zero costs.
        let seedream = result
            .file
            .models
            .iter()
            .find(|m| m.id == "doubao-seedream-4-0-250828")
            .expect("doubao-seedream registered");
        assert_eq!(model_request_value(seedream)["pricing_known"], json!(false));
        assert_eq!(
            model_request_value(seedream)["input_cost_per_m"],
            json!(0.0)
        );
        assert_eq!(
            model_request_value(seedream)["output_cost_per_m"],
            json!(0.0)
        );
    }

    // ── clap wiring ───────────────────────────────────────────────────────

    #[test]
    fn clap_parses_models_connect_with_and_without_set_default() {
        let parse = |args: &[&str]| match Cli::parse_from(args).command {
            Some(Commands::Models(ModelsCommands::Connect {
                target,
                set_default,
            })) => (target, set_default),
            other => panic!("unexpected command variant: {}", other.is_some()),
        };
        assert_eq!(
            parse(&["librefang", "models", "connect", "everyapi"]),
            ("everyapi".to_string(), false)
        );
        assert_eq!(
            parse(&[
                "librefang",
                "models",
                "connect",
                "everyapi",
                "--set-default"
            ]),
            ("everyapi".to_string(), true)
        );
    }

    #[test]
    fn clap_requires_a_connect_target() {
        assert!(Cli::try_parse_from(["librefang", "models", "connect"]).is_err());
    }

    #[test]
    fn an_unreachable_gateway_still_produces_a_loadable_provider_entry() {
        // The empty-model-list path: `fetch_gateway_models` returned None and
        // the command carried on. The `[provider]` section alone must still
        // round-trip, otherwise the fallback is worthless.
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &[],
            &BTreeMap::new(),
        );
        let rendered = toml::to_string_pretty(&result.file).expect("serializes");
        let parsed: ModelCatalogFile = toml::from_str(&rendered).expect("round trips");
        assert!(parsed.models.is_empty());
        assert_eq!(parsed.provider.expect("provider").id, "everyapi");
    }

    #[test]
    fn an_unreachable_pricing_feed_still_produces_a_loadable_catalog() {
        // `fetch_pricing_rows` returned None and the command carried on with
        // an empty index. Every model the snapshot can give token limits to
        // must still register and round-trip — just with no price. The
        // command reports the whole set by name so the operator knows their
        // spend is not being counted.
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
            &BTreeMap::new(),
        );
        assert_eq!(result.file.models.len(), 16);

        let rendered = toml::to_string_pretty(&result.file).expect("serializes");
        let parsed: ModelCatalogFile = toml::from_str(&rendered).expect("round trips");
        for model in &parsed.models {
            model
                .validate()
                .unwrap_or_else(|e| panic!("entry rejected downstream: {e}"));
            assert!(
                !model.pricing_known,
                "{} must not claim a price with no feed",
                model.id
            );
            assert_eq!(model.input_cost_per_m, 0.0);
            assert_eq!(model.output_cost_per_m, 0.0);
        }
        // Reported, not silent: all 16 are named as unpriced.
        assert_eq!(result.unpriced.len(), 16);
    }
}
