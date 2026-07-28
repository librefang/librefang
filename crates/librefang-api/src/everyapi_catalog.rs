//! TTL-based refresh of the EveryAPI gateway's live model catalog.
//!
//! `librefang models connect everyapi` writes `providers/everyapi.toml` once,
//! from whatever the gateway published at that moment. The gateway's model
//! list is an account property: it changes when EveryAPI adds or delists a
//! model and when the operator's plan changes. Without a refresh the daemon
//! keeps serving that frozen snapshot until somebody re-runs the command by
//! hand.
//!
//! This module is the EveryAPI counterpart of [`crate::openrouter_catalog`]
//! and deliberately mirrors its shape: the same `needs_*` / `refresh_if_*`
//! surface, the same `REFRESH_ATTEMPTS` retry window, the same
//! `reconcile_live_provider_models` merge. It is a parallel module rather
//! than a generic abstraction over both providers because the two differ in
//! the parts that matter — OpenRouter's catalog is public and single-endpoint,
//! EveryAPI's needs a bearer token and two endpoints — and folding them into
//! one generic path would put the load-bearing OpenRouter code at risk for no
//! gain here.
//!
//! Unlike OpenRouter, metadata comes from two gateway endpoints:
//!
//! * `GET {base_url}/models` — bearer auth, authoritative id list and (when
//!   published) per-model context window.
//! * `GET {origin}/api/pricing` — public (optional auth), carries the
//!   gateway's own billing ratios plus a context window for the models that
//!   declare one.
//!
//! Anything neither endpoint publishes — most importantly `max_output_tokens`,
//! which the gateway exposes nowhere — is carried forward from the entry the
//! connect command already wrote, because
//! [`ModelCatalog::reconcile_live_provider_models`] replaces a provider's
//! whole non-custom entry set. Refreshing without that carry-forward would
//! silently delete the metadata the user just imported.

use dashmap::DashMap;
use librefang_kernel::kernel_api::KernelApi;
use librefang_kernel::model_catalog::ModelCatalog;
use librefang_types::model_catalog::{AuthStatus, Modality, ModelCatalogEntry, ModelTier};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// Catalog provider id written by `librefang models connect everyapi`.
const PROVIDER_ID: &str = "everyapi";

/// Convention env var for the relay key when the provider entry omits one.
const DEFAULT_API_KEY_ENV: &str = "EVERYAPI_API_KEY";

/// Guards against hammering a down or rate-limiting gateway: the stamp is
/// written on every attempt (success or failure), so this is a hard floor on
/// refresh frequency per base URL, not a failure-only backoff. Keyed by base
/// URL rather than provider id so integration tests on sequential ephemeral
/// ports do not contaminate each other (#6384).
static REFRESH_ATTEMPTS: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);
const REFRESH_RETRY_WINDOW: Duration = Duration::from_secs(60);

/// How long a fetched EveryAPI model list stays fresh.
///
/// 15 minutes, matching `OPENROUTER_MODEL_CATALOG_TTL`. The two providers
/// have the same refresh economics — an aggregating gateway whose model list
/// turns over on the order of days, read on interactive request paths where a
/// blocking round trip is only acceptable if it is rare. A shorter TTL would
/// put two HTTP calls (one of them authenticated) in front of ordinary
/// dashboard loads for no practical freshness gain; a longer one would keep a
/// newly-purchased plan's models hidden for most of a session. Keeping the
/// number identical to OpenRouter's also means operators reason about one
/// cadence rather than two.
const EVERYAPI_MODEL_CATALOG_TTL: Duration = Duration::from_secs(15 * 60);

/// Ratio-to-USD conversion for the gateway's billing ratios.
///
/// The gateway's own documentation and its billing implementation agree that
/// `ratio 1` bills at `$0.002 / 1K tokens`, i.e. $2 per million input tokens:
/// quota accrues as `tokens * model_ratio` with `QuotaPerUnit = 500_000`
/// quota per USD. So `input $/1M = model_ratio * 2` and
/// `output $/1M = model_ratio * completion_ratio * 2`.
const RATIO_USD_PER_MILLION: f64 = 2.0;

// ---------------------------------------------------------------------------
// Gate + freshness predicates
// ---------------------------------------------------------------------------

/// Whether an `everyapi` provider is configured well enough to be worth a
/// network round trip.
///
/// Deliberately narrower than the OpenRouter gate, which also accepts
/// [`AuthStatus::InvalidKey`] because OpenRouter's `/models` is public.
/// EveryAPI's `/v1/models` requires a valid bearer, so a key the gateway has
/// already rejected can only produce a 401 — retrying it every 15 minutes
/// would burn requests and log noise to learn nothing.
fn catalog_provider_is_configured(catalog: &ModelCatalog) -> bool {
    catalog.get_provider(PROVIDER_ID).is_some_and(|provider| {
        matches!(
            provider.auth_status,
            AuthStatus::Configured | AuthStatus::ValidatedKey | AuthStatus::AutoDetected
        )
    })
}

fn catalog_needs_initial_refresh(catalog: &ModelCatalog) -> bool {
    catalog_provider_is_configured(catalog) && !catalog.has_live_provider_models(PROVIDER_ID)
}

fn catalog_needs_stale_refresh(catalog: &ModelCatalog) -> bool {
    catalog_provider_is_configured(catalog)
        && catalog.live_provider_models_are_stale(PROVIDER_ID, EVERYAPI_MODEL_CATALOG_TTL)
}

pub(crate) fn needs_initial_refresh(kernel: &Arc<dyn KernelApi>) -> bool {
    catalog_needs_initial_refresh(&kernel.model_catalog_ref().load())
}

fn needs_stale_refresh(kernel: &Arc<dyn KernelApi>) -> bool {
    catalog_needs_stale_refresh(&kernel.model_catalog_ref().load())
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Refresh only when this process has never fetched the gateway's list.
pub(crate) async fn refresh_if_missing(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    if !needs_initial_refresh(kernel) {
        return Ok(0);
    }
    refresh_now(kernel).await
}

/// Refresh when the last successful fetch is older than the TTL.
pub(crate) async fn refresh_if_stale(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    if !needs_stale_refresh(kernel) {
        return Ok(0);
    }
    refresh_now(kernel).await
}

/// Kick off an initial backfill without making the caller wait on the network.
pub(crate) fn refresh_if_missing_in_background(kernel: &Arc<dyn KernelApi>) {
    if !needs_initial_refresh(kernel) {
        return;
    }
    let kernel = Arc::clone(kernel);
    tokio::spawn(async move {
        if let Err(error) = refresh_if_missing(&kernel).await {
            tracing::warn!(%error, "EveryAPI live catalog background refresh failed");
        }
    });
}

// ---------------------------------------------------------------------------
// URL derivation
// ---------------------------------------------------------------------------

/// Derive the gateway origin that serves `/api/pricing` from the provider's
/// `base_url`.
///
/// `base_url` is the OpenAI-compatible root and therefore already ends in
/// `/v1` (the connect command appends it so the driver can build
/// `{base_url}/chat/completions`). `/api/pricing` is a sibling of `/v1`, not a
/// child, so exactly one trailing `/v1` segment is removed.
///
/// A `base_url` that does not end in `/v1` is returned trimmed and otherwise
/// untouched — a self-hosted gateway mounted at the root is a legitimate
/// configuration and must not panic or lose a path segment. Stripping is also
/// refused when it would leave an empty string, so a nonsense `base_url` of
/// `"/v1"` degrades to itself rather than to `""`.
pub(crate) fn pricing_origin(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    match trimmed.strip_suffix("/v1") {
        Some(origin) if !origin.is_empty() => origin.trim_end_matches('/').to_string(),
        _ => trimmed.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Gateway response parsing
// ---------------------------------------------------------------------------

/// One entry of `GET {base_url}/models`, reduced to the fields that affect
/// catalog synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveModel {
    pub(crate) id: String,
    pub(crate) supported_endpoint_types: Vec<String>,
    /// Present only for the models that declare one; authoritative over the
    /// pricing feed because it is what the serving endpoint will accept.
    pub(crate) context_window: Option<u64>,
}

/// Per-model figures recovered from `GET {origin}/api/pricing`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PricingEntry {
    /// `0` means the feed reported no context window, not a zero-token model.
    pub(crate) context_window: u64,
    pub(crate) input_cost_per_m: f64,
    pub(crate) output_cost_per_m: f64,
    /// False for per-call models, which have no per-token price at all.
    pub(crate) pricing_known: bool,
}

/// Parse the id list. Rows without a usable `id` are dropped rather than
/// failing the whole response — one malformed row should not cost the
/// operator every other model.
pub(crate) fn parse_live_models(body: &serde_json::Value) -> Vec<LiveModel> {
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
            Some(LiveModel {
                id: id.to_string(),
                supported_endpoint_types: item
                    .get("supported_endpoint_types")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                context_window: item
                    .get("context_window")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|v| *v > 0),
            })
        })
        .collect()
}

/// Parse `/api/pricing` into a lookup table keyed by lowercased model name.
///
/// The pricing feed is a lookup table only, never a source of model ids: it
/// can list models the account cannot actually call (the gateway publishes
/// `claude-fable-5` here while `/v1/models` omits it), and registering those
/// would offer the operator models that 404 on first use.
///
/// The top-level `group_ratio` is deliberately NOT applied. The gateway's
/// billing formula multiplies by it, but it is a per-account discount
/// multiplier whose selected group is not derivable from this response; the
/// documented published price is the un-grouped `model_ratio * 2`, which is
/// also what the previously-registered entries carry. Applying a group ratio
/// here would make catalog prices disagree with the gateway's own price page.
pub(crate) fn parse_pricing_entries(body: &serde_json::Value) -> HashMap<String, PricingEntry> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for item in items {
        let Some(name) = item
            .get("model_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let context_window = item
            .get("context_window")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        // `quota_type == 1` (equivalently `billing_mode == "per_call"`) bills
        // a flat `model_price` per request instead of per token. Those are the
        // image / video models; forcing a per-call USD figure into a
        // per-million-token field would corrupt every budget projection, so
        // they are recorded as "pricing not known" for token purposes.
        let per_call = item
            .get("quota_type")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|q| q != 0)
            || item.get("billing_mode").and_then(|v| v.as_str()) == Some("per_call");
        let model_ratio = item
            .get("model_ratio")
            .and_then(serde_json::Value::as_f64)
            .filter(|r| r.is_finite() && *r >= 0.0);
        let completion_ratio = item
            .get("completion_ratio")
            .and_then(serde_json::Value::as_f64)
            .filter(|r| r.is_finite() && *r >= 0.0)
            .unwrap_or(0.0);

        let entry = match (per_call, model_ratio) {
            (false, Some(model_ratio)) => PricingEntry {
                context_window,
                input_cost_per_m: model_ratio * RATIO_USD_PER_MILLION,
                output_cost_per_m: model_ratio * completion_ratio * RATIO_USD_PER_MILLION,
                pricing_known: true,
            },
            _ => PricingEntry {
                context_window,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                pricing_known: false,
            },
        };
        out.insert(name.to_lowercase(), entry);
    }
    out
}

/// Infer a model's modality from `supported_endpoint_types`.
///
/// The gateway publishes no modality field, so the endpoint-type list is the
/// only signal. An EMPTY list means video: the `doubao-seedance-*` family
/// publishes `[]` and is video-generation only. That case must be checked
/// before the text default, otherwise those entries would be graded as text
/// models and then dropped for missing a context window.
///
/// This duplicates `infer_modality` in
/// `librefang-cli/src/commands/everyapi.rs`; see the module note in the PR
/// report on why the shared home would be `librefang-runtime::model_catalog`.
pub(crate) fn infer_modality(supported_endpoint_types: &[String]) -> Modality {
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
        return Modality::Video;
    }
    Modality::Text
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// Build the replacement entry set for the `everyapi` provider.
///
/// `existing` is the provider's current catalog entries — what the connect
/// command wrote, plus whatever a previous refresh produced.
/// [`ModelCatalog::reconcile_live_provider_models`] deletes every non-custom
/// entry for the provider and installs this vec wholesale, and its own
/// carry-forward only donates `tier` / `reasoning_echo_policy` / `aliases`.
/// Everything else that the gateway does not publish — above all
/// `max_output_tokens`, which appears in neither endpoint — has to be carried
/// forward here or it is destroyed by the very act of refreshing.
///
/// Ids are emitted bare (`claude-sonnet-5`), matching what the connect command
/// writes. Prefixing them the way the OpenRouter path does would make the
/// case-insensitive id match inside `replace_provider_snapshot` miss every
/// existing entry, so carry-forward would silently find nothing and the
/// catalog would double.
///
/// Text models that still have no context window or output limit after all
/// sources are consulted are dropped: `ModelCatalogEntry::validate()` rejects
/// them on the next daemon boot anyway, and a `0` context window feeds
/// straight into compaction thresholds and budget math.
///
/// Output is sorted by id (repo invariant #3298).
pub(crate) fn build_catalog_entries(
    live: &[LiveModel],
    pricing: &HashMap<String, PricingEntry>,
    existing: &[ModelCatalogEntry],
) -> Vec<ModelCatalogEntry> {
    let previous: HashMap<String, &ModelCatalogEntry> = existing
        .iter()
        .map(|entry| (entry.id.to_lowercase(), entry))
        .collect();

    let mut entries: Vec<ModelCatalogEntry> = live
        .iter()
        .filter_map(|model| {
            let key = model.id.to_lowercase();
            let prior = previous.get(&key).copied();
            let priced = pricing.get(&key);
            let modality = infer_modality(&model.supported_endpoint_types);

            // Precedence: the serving endpoint's own figure, then the pricing
            // feed, then whatever was registered before.
            let context_window = model
                .context_window
                .or_else(|| priced.map(|p| p.context_window).filter(|c| *c > 0))
                .or_else(|| prior.map(|p| p.context_window).filter(|c| *c > 0))
                .unwrap_or(0);
            // The gateway publishes no max-output figure on either endpoint,
            // so a previously-registered value is the only source there is.
            let max_output_tokens = prior.map(|p| p.max_output_tokens).unwrap_or(0);

            if modality == Modality::Text && (context_window == 0 || max_output_tokens == 0) {
                return None;
            }

            // Pricing moves as one unit: a cost is never carried forward
            // while `pricing_known` is reset, and `0.0 / 0.0` is never
            // emitted as a known price (which would assert the model is free
            // and poison metering).
            let (input_cost_per_m, output_cost_per_m, pricing_known) = match priced {
                Some(p) if p.pricing_known => (p.input_cost_per_m, p.output_cost_per_m, true),
                _ => match prior.filter(|p| p.pricing_known) {
                    Some(p) => (p.input_cost_per_m, p.output_cost_per_m, true),
                    None => (0.0, 0.0, false),
                },
            };

            Some(ModelCatalogEntry {
                id: model.id.clone(),
                display_name: prior
                    .map(|p| p.display_name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| model.id.clone()),
                provider: PROVIDER_ID.to_string(),
                // Deliberately NOT `ModelTier::Custom`: `find_model` returns
                // the first `Custom` match immediately (#983), so a Custom
                // gateway copy of an id that also exists upstream
                // (`claude-sonnet-5`, `gemini-3.5-flash`, …) would hijack every
                // provider-blind lookup and silently re-price agents that never
                // opted into this gateway. `reconcile_live_provider_models`
                // donates the previous tier when one existed.
                tier: prior.map(|p| p.tier).unwrap_or(ModelTier::Balanced),
                modality,
                context_window,
                max_output_tokens,
                input_cost_per_m,
                output_cost_per_m,
                pricing_known,
                supports_tools: prior.is_some_and(|p| p.supports_tools),
                supports_vision: prior.is_some_and(|p| p.supports_vision),
                // Every OpenAI-shaped endpoint on this gateway streams; the
                // `openai-response` family requires it.
                supports_streaming: true,
                supports_thinking: prior.is_some_and(|p| p.supports_thinking),
                ..Default::default()
            })
        })
        .collect();

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

/// Take the per-base-URL refresh slot, or refuse because the window is open.
fn try_claim_refresh_slot(base_url: &str) -> Result<(), String> {
    match REFRESH_ATTEMPTS.entry(base_url.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(mut attempt) => {
            if attempt.get().elapsed() < REFRESH_RETRY_WINDOW {
                return Err("EveryAPI catalog refresh is in the 60-second retry window".to_string());
            }
            attempt.insert(Instant::now());
            Ok(())
        }
        dashmap::mapref::entry::Entry::Vacant(attempt) => {
            attempt.insert(Instant::now());
            Ok(())
        }
    }
}

/// Fetch the authenticated model listing.
///
/// The relay key is placed in the `Authorization` header only. It never
/// reaches the URL, the returned error strings, or any log line — call sites
/// `warn!` these errors verbatim.
async fn fetch_live_models(base_url: &str, api_key: &str) -> Result<Vec<LiveModel>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let response = librefang_kernel::provider_health::probe_client()
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|error| format!("EveryAPI model request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "EveryAPI model request returned HTTP {}",
            status.as_u16()
        ));
    }
    let body = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("EveryAPI model response was invalid JSON: {error}"))?;
    let models = parse_live_models(&body);
    if models.is_empty() {
        return Err("EveryAPI model response contained no models".to_string());
    }
    Ok(models)
}

/// Fetch the public pricing feed. Best-effort: a failure degrades the refresh
/// to "ids and carried-forward metadata" rather than aborting it, because the
/// authoritative id list has already been obtained at that point.
async fn fetch_pricing(origin: &str) -> Result<HashMap<String, PricingEntry>, String> {
    let url = format!("{origin}/api/pricing");
    let response = librefang_kernel::provider_health::probe_client()
        .get(url)
        .send()
        .await
        .map_err(|error| format!("EveryAPI pricing request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "EveryAPI pricing request returned HTTP {}",
            status.as_u16()
        ));
    }
    let body = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("EveryAPI pricing response was invalid JSON: {error}"))?;
    Ok(parse_pricing_entries(&body))
}

async fn refresh_now(kernel: &Arc<dyn KernelApi>) -> Result<usize, String> {
    let (base_url, api_key_env) = {
        let catalog = kernel.model_catalog_ref().load();
        let provider = catalog
            .get_provider(PROVIDER_ID)
            .ok_or_else(|| "EveryAPI provider is not configured".to_string())?;
        if provider.base_url.trim().is_empty() {
            return Err("EveryAPI base URL is not configured".to_string());
        }
        let env_var = if provider.api_key_env.trim().is_empty() {
            DEFAULT_API_KEY_ENV.to_string()
        } else {
            provider.api_key_env.clone()
        };
        (provider.base_url.clone(), env_var)
    };

    // Treat an empty env var the same as an absent one — `/v1/models` answers
    // an empty bearer with a 401 either way.
    let api_key = std::env::var(&api_key_env)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("EveryAPI relay key env var {api_key_env} is not set"))?;

    try_claim_refresh_slot(&base_url)?;

    let live = fetch_live_models(&base_url, &api_key).await?;
    let pricing = match fetch_pricing(&pricing_origin(&base_url)).await {
        Ok(pricing) => pricing,
        Err(error) => {
            // Non-fatal: previously-registered pricing is carried forward.
            tracing::warn!(%error, "EveryAPI pricing feed unavailable; keeping registered prices");
            HashMap::new()
        }
    };

    // Snapshot the existing entries and compute the replacement set OUTSIDE
    // the update closure: `model_catalog_update` is an RCU that may run the
    // closure more than once under contention, so it must stay cheap and free
    // of reads that could observe a partially-updated catalog.
    let existing: Vec<ModelCatalogEntry> = {
        let catalog = kernel.model_catalog_ref().load();
        catalog
            .models_by_provider(PROVIDER_ID)
            .into_iter()
            .cloned()
            .collect()
    };
    let entries = build_catalog_entries(&live, &pricing, &existing);
    if entries.is_empty() {
        // Reconciling an empty snapshot would delete the provider's entire
        // model list; refuse instead and leave the previous catalog standing.
        return Err("EveryAPI refresh produced no usable models".to_string());
    }

    let mut available_models: Vec<String> = live.iter().map(|model| model.id.clone()).collect();
    available_models.sort();
    let model_count = entries.len();
    kernel.model_catalog_update(&mut move |catalog| {
        catalog.reconcile_live_provider_models(
            PROVIDER_ID,
            available_models.clone(),
            entries.clone(),
        );
    });
    Ok(model_count)
}

/// Clear the retry window for one base URL so sequential integration tests on
/// reused ephemeral ports do not contaminate each other (#6384).
///
/// The key is the provider's `base_url` — i.e. the value that ends in `/v1`,
/// not the bare server origin.
#[cfg(feature = "test-util")]
pub fn clear_refresh_attempts(base_url: &str) {
    REFRESH_ATTEMPTS.remove(base_url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::model_catalog::ProviderInfo;

    fn provider(auth_status: AuthStatus) -> ProviderInfo {
        ProviderInfo {
            id: PROVIDER_ID.to_string(),
            display_name: "EveryAPI".to_string(),
            api_key_env: DEFAULT_API_KEY_ENV.to_string(),
            base_url: "https://api.everyapi.ai/v1".to_string(),
            key_required: true,
            auth_status,
            ..Default::default()
        }
    }

    fn text_entry(id: &str) -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: id.to_string(),
            display_name: id.to_string(),
            provider: PROVIDER_ID.to_string(),
            tier: ModelTier::Balanced,
            modality: Modality::Text,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 2.0,
            output_cost_per_m: 10.0,
            pricing_known: true,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            supports_thinking: true,
            ..Default::default()
        }
    }

    fn live(id: &str, endpoints: &[&str], context_window: Option<u64>) -> LiveModel {
        LiveModel {
            id: id.to_string(),
            supported_endpoint_types: endpoints.iter().map(|s| s.to_string()).collect(),
            context_window,
        }
    }

    // -- URL derivation ---------------------------------------------------

    #[test]
    fn pricing_origin_strips_exactly_one_v1_segment() {
        assert_eq!(
            pricing_origin("https://api.everyapi.ai/v1"),
            "https://api.everyapi.ai"
        );
    }

    #[test]
    fn pricing_origin_tolerates_a_trailing_slash_and_surrounding_space() {
        assert_eq!(
            pricing_origin("  https://api.everyapi.ai/v1/  "),
            "https://api.everyapi.ai"
        );
    }

    #[test]
    fn pricing_origin_leaves_a_base_url_without_the_suffix_alone() {
        assert_eq!(
            pricing_origin("https://gateway.internal"),
            "https://gateway.internal"
        );
        assert_eq!(
            pricing_origin("https://gateway.internal/"),
            "https://gateway.internal"
        );
    }

    #[test]
    fn pricing_origin_keeps_a_mounted_path_prefix() {
        // A self-hosted gateway behind a path prefix keeps the prefix; only
        // the OpenAI-compat `/v1` root is removed.
        assert_eq!(
            pricing_origin("https://gateway.internal/relay/v1"),
            "https://gateway.internal/relay"
        );
    }

    #[test]
    fn pricing_origin_does_not_split_a_segment_that_merely_ends_in_v1() {
        assert_eq!(
            pricing_origin("https://gateway.internal/apiv1"),
            "https://gateway.internal/apiv1"
        );
    }

    #[test]
    fn pricing_origin_refuses_to_strip_itself_down_to_nothing() {
        assert_eq!(pricing_origin("/v1"), "/v1");
        assert_eq!(pricing_origin(""), "");
    }

    // -- Gate + freshness -------------------------------------------------

    #[test]
    fn an_unconfigured_or_rejected_key_never_triggers_a_refresh() {
        for status in [
            AuthStatus::Missing,
            // Unlike OpenRouter's public catalog, a rejected relay key can
            // only ever produce a 401 from `/v1/models`.
            AuthStatus::InvalidKey,
            AuthStatus::CliNotInstalled,
        ] {
            let catalog = ModelCatalog::from_entries(Vec::new(), vec![provider(status)]);
            assert!(
                !catalog_needs_initial_refresh(&catalog),
                "{status:?} should not trigger an initial refresh"
            );
            assert!(
                !catalog_needs_stale_refresh(&catalog),
                "{status:?} should not trigger a stale refresh"
            );
        }
    }

    #[test]
    fn a_configured_provider_with_no_live_fetch_is_both_missing_and_stale() {
        let catalog =
            ModelCatalog::from_entries(Vec::new(), vec![provider(AuthStatus::Configured)]);
        assert!(catalog_needs_initial_refresh(&catalog));
        assert!(catalog_needs_stale_refresh(&catalog));
    }

    #[test]
    fn a_just_fetched_catalog_is_neither_missing_nor_stale() {
        let mut catalog =
            ModelCatalog::from_entries(Vec::new(), vec![provider(AuthStatus::ValidatedKey)]);
        catalog.set_provider_available_models(PROVIDER_ID, vec!["claude-sonnet-5".to_string()]);
        assert!(!catalog_needs_initial_refresh(&catalog));
        assert!(!catalog_needs_stale_refresh(&catalog));
    }

    #[test]
    fn a_missing_everyapi_provider_is_inert() {
        let catalog = ModelCatalog::from_entries(Vec::new(), Vec::new());
        assert!(!catalog_needs_initial_refresh(&catalog));
        assert!(!catalog_needs_stale_refresh(&catalog));
    }

    // -- Backoff ----------------------------------------------------------

    #[test]
    fn the_first_claim_wins_and_the_second_is_refused_inside_the_window() {
        // Distinct per test: `REFRESH_ATTEMPTS` is a process-global static.
        let base_url = "https://backoff-first.test/v1";
        assert!(try_claim_refresh_slot(base_url).is_ok());
        let refused = try_claim_refresh_slot(base_url).unwrap_err();
        assert!(refused.contains("retry window"), "{refused}");
    }

    #[test]
    fn the_window_is_scoped_per_base_url() {
        let occupied = "https://backoff-scoped-a.test/v1";
        let untouched = "https://backoff-scoped-b.test/v1";
        assert!(try_claim_refresh_slot(occupied).is_ok());
        assert!(try_claim_refresh_slot(occupied).is_err());
        assert!(try_claim_refresh_slot(untouched).is_ok());
    }

    #[test]
    fn an_expired_stamp_lets_the_next_attempt_through() {
        let base_url = "https://backoff-expired.test/v1";
        REFRESH_ATTEMPTS.insert(
            base_url.to_string(),
            Instant::now() - REFRESH_RETRY_WINDOW - Duration::from_secs(1),
        );
        assert!(try_claim_refresh_slot(base_url).is_ok());
        assert!(try_claim_refresh_slot(base_url).is_err());
    }

    // -- Parsing ----------------------------------------------------------

    #[test]
    fn live_model_rows_without_a_usable_id_are_dropped_not_fatal() {
        let body = serde_json::json!({"data": [
            {"id": "claude-sonnet-5", "supported_endpoint_types": ["openai", "anthropic"], "context_window": 200000},
            {"id": "   "},
            {"object": "model"},
            {"id": "doubao-seedream-4-0-250828", "supported_endpoint_types": ["image-generation"]},
        ]});
        let models = parse_live_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-sonnet-5");
        assert_eq!(models[0].context_window, Some(200_000));
        assert_eq!(models[1].context_window, None);
    }

    #[test]
    fn a_non_object_pricing_body_yields_an_empty_table() {
        assert!(parse_pricing_entries(&serde_json::json!({"success": false})).is_empty());
        assert!(parse_live_models(&serde_json::json!({"success": false})).is_empty());
    }

    #[test]
    fn per_token_ratios_convert_to_usd_per_million() {
        let body = serde_json::json!({"data": [
            {"model_name": "claude-sonnet-5", "quota_type": 0, "model_ratio": 1.0, "completion_ratio": 5.0, "context_window": 200000, "billing_mode": "per_token"},
            {"model_name": "MiniMax-M3", "quota_type": 0, "model_ratio": 0.15, "completion_ratio": 4.0, "context_window": 0, "billing_mode": "per_token"},
        ]});
        let pricing = parse_pricing_entries(&body);
        let sonnet = pricing.get("claude-sonnet-5").expect("sonnet priced");
        // ratio 1 / completion 5 is the gateway's published $2 in / $10 out.
        assert!((sonnet.input_cost_per_m - 2.0).abs() < 1e-9);
        assert!((sonnet.output_cost_per_m - 10.0).abs() < 1e-9);
        assert!(sonnet.pricing_known);
        assert_eq!(sonnet.context_window, 200_000);

        let minimax = pricing.get("minimax-m3").expect("lookup is lowercased");
        assert!((minimax.input_cost_per_m - 0.30).abs() < 1e-9);
        assert!((minimax.output_cost_per_m - 1.20).abs() < 1e-9);
        assert_eq!(minimax.context_window, 0);
    }

    #[test]
    fn per_call_models_report_no_token_pricing() {
        let body = serde_json::json!({"data": [
            {"model_name": "doubao-seedream-4-0-250828", "quota_type": 1, "model_ratio": 0.0, "completion_ratio": 0.0, "model_price": 0.028, "billing_mode": "per_call"},
        ]});
        let pricing = parse_pricing_entries(&body);
        let entry = pricing
            .get("doubao-seedream-4-0-250828")
            .expect("present but unpriced");
        assert!(!entry.pricing_known);
        assert_eq!(entry.input_cost_per_m, 0.0);
        assert_eq!(entry.output_cost_per_m, 0.0);
    }

    #[test]
    fn a_row_without_a_model_ratio_is_not_asserted_to_be_free() {
        let body = serde_json::json!({"data": [
            {"model_name": "mystery-model", "quota_type": 0, "billing_mode": "per_token"},
        ]});
        let entry = parse_pricing_entries(&body)["mystery-model"];
        assert!(!entry.pricing_known);
    }

    #[test]
    fn modality_comes_from_the_endpoint_type_list() {
        assert_eq!(infer_modality(&[]), Modality::Video);
        assert_eq!(
            infer_modality(&["image-generation".to_string()]),
            Modality::Image
        );
        assert_eq!(
            infer_modality(&["audio-speech".to_string()]),
            Modality::Audio
        );
        assert_eq!(
            infer_modality(&["openai".to_string(), "anthropic".to_string()]),
            Modality::Text
        );
    }

    // -- Merge shape ------------------------------------------------------

    #[test]
    fn refreshed_pricing_replaces_the_registered_figures() {
        let live = vec![live(
            "claude-sonnet-5",
            &["openai", "anthropic"],
            Some(200_000),
        )];
        let mut pricing = HashMap::new();
        pricing.insert(
            "claude-sonnet-5".to_string(),
            PricingEntry {
                context_window: 200_000,
                input_cost_per_m: 3.0,
                output_cost_per_m: 15.0,
                pricing_known: true,
            },
        );
        let entries = build_catalog_entries(&live, &pricing, &[text_entry("claude-sonnet-5")]);
        assert_eq!(entries.len(), 1);
        assert!((entries[0].input_cost_per_m - 3.0).abs() < 1e-9);
        assert!((entries[0].output_cost_per_m - 15.0).abs() < 1e-9);
        assert!(entries[0].pricing_known);
    }

    #[test]
    fn metadata_the_gateway_never_publishes_is_carried_forward() {
        // `max_output_tokens` and the capability flags exist on neither
        // endpoint; refreshing must not delete them.
        let live = vec![live("claude-sonnet-5", &["openai"], Some(200_000))];
        let entries =
            build_catalog_entries(&live, &HashMap::new(), &[text_entry("claude-sonnet-5")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].max_output_tokens, 64_000);
        assert!(entries[0].supports_tools);
        assert!(entries[0].supports_vision);
        assert!(entries[0].supports_thinking);
        assert!(entries[0].supports_streaming);
        // Pricing survives as a unit when the feed is unavailable.
        assert!((entries[0].input_cost_per_m - 2.0).abs() < 1e-9);
        assert!(entries[0].pricing_known);
    }

    #[test]
    fn a_text_model_the_gateway_newly_added_is_skipped_until_an_output_limit_is_known() {
        // No prior entry means no `max_output_tokens`, and
        // `ModelCatalogEntry::validate()` rejects a text model without one —
        // registering it would make it vanish at the next daemon boot.
        let live = vec![live("gemini-3-flash", &["openai"], Some(1_000_000))];
        let entries = build_catalog_entries(&live, &HashMap::new(), &[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn non_text_models_register_without_any_token_metadata() {
        let live = vec![
            live("doubao-seedance-2-0-260128", &[], None),
            live("tts-1", &["audio-speech"], None),
        ];
        let entries = build_catalog_entries(&live, &HashMap::new(), &[]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].modality, Modality::Video);
        assert_eq!(entries[1].modality, Modality::Audio);
        // Never assert 0.0/0.0 is a real price.
        assert!(entries.iter().all(|e| !e.pricing_known));
    }

    #[test]
    fn a_delisted_model_disappears_and_ids_stay_bare() {
        let live = vec![live("claude-opus-5", &["openai"], Some(200_000))];
        let existing = vec![text_entry("claude-opus-5"), text_entry("claude-haiku-4-5")];
        let entries = build_catalog_entries(&live, &HashMap::new(), &existing);
        assert_eq!(entries.len(), 1);
        // Bare id, matching what `models connect everyapi` writes — a
        // `everyapi/`-prefixed id would break carry-forward matching.
        assert_eq!(entries[0].id, "claude-opus-5");
        assert_eq!(entries[0].provider, "everyapi");
    }

    #[test]
    fn the_pricing_feed_never_introduces_a_model_id() {
        // `claude-fable-5` is published by `/api/pricing` but absent from
        // `/v1/models`; registering it would offer a model that 404s.
        let live = vec![live("claude-opus-5", &["openai"], Some(200_000))];
        let mut pricing = HashMap::new();
        for id in ["claude-opus-5", "claude-fable-5"] {
            pricing.insert(
                id.to_string(),
                PricingEntry {
                    context_window: 200_000,
                    input_cost_per_m: 5.0,
                    output_cost_per_m: 25.0,
                    pricing_known: true,
                },
            );
        }
        let entries = build_catalog_entries(&live, &pricing, &[text_entry("claude-opus-5")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "claude-opus-5");
    }

    #[test]
    fn a_gateway_context_window_outranks_the_pricing_feed_and_the_previous_entry() {
        let live = vec![live("claude-sonnet-5", &["openai"], Some(500_000))];
        let mut pricing = HashMap::new();
        pricing.insert(
            "claude-sonnet-5".to_string(),
            PricingEntry {
                context_window: 200_000,
                input_cost_per_m: 2.0,
                output_cost_per_m: 10.0,
                pricing_known: true,
            },
        );
        let entries = build_catalog_entries(&live, &pricing, &[text_entry("claude-sonnet-5")]);
        assert_eq!(entries[0].context_window, 500_000);
    }

    #[test]
    fn the_pricing_feed_supplies_a_context_window_the_listing_omits() {
        let live = vec![live("claude-sonnet-5", &["openai"], None)];
        let mut pricing = HashMap::new();
        pricing.insert(
            "claude-sonnet-5".to_string(),
            PricingEntry {
                context_window: 200_000,
                input_cost_per_m: 2.0,
                output_cost_per_m: 10.0,
                pricing_known: true,
            },
        );
        let mut prior = text_entry("claude-sonnet-5");
        prior.context_window = 128_000;
        let entries = build_catalog_entries(&live, &pricing, &[prior]);
        assert_eq!(entries[0].context_window, 200_000);
    }

    #[test]
    fn output_is_sorted_by_id_regardless_of_gateway_order() {
        let live = vec![
            live("tts-1", &["audio-speech"], None),
            live("doubao-seedance-2-0-260128", &[], None),
            live("MiniMax-M3", &["openai"], Some(245_760)),
        ];
        let entries = build_catalog_entries(&live, &HashMap::new(), &[text_entry("MiniMax-M3")]);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["MiniMax-M3", "doubao-seedance-2-0-260128", "tts-1"]);
    }
}
