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
//! 3. synthesise a `providers/everyapi.toml` catalog file, enriching text
//!    models with context/pricing metadata from the compiled-in OpenRouter
//!    snapshot;
//! 4. persist the relay key to `~/.librefang/.env` as `EVERYAPI_API_KEY`.
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
/// Deliberately NOT `EVERYAPI_RELAY_KEY` — that is EveryAPI's own
/// script-facing variable name, and conflating the two would make
/// `librefang models providers` disagree with `provider_to_env_var`
/// (`commands/common.rs`), whose `other => "{OTHER}_API_KEY"` fallback
/// already produces exactly this value for the id `everyapi`.
const API_KEY_ENV: &str = "EVERYAPI_API_KEY";
/// How long to wait for the gateway's model list before giving up and
/// writing the provider entry with no `[[models]]`.
const MODELS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// The two fields this command needs out of EveryAPI's credentials file.
///
/// The on-disk file also carries `access_token`, `role`, `user_id` and
/// `username`; none of them are read here. `relay_key` is the gateway bearer
/// and is treated as a secret throughout — it is never rendered.
pub(crate) struct EveryApiCredentials {
    /// Gateway root, e.g. `https://api.everyapi.ai` (no `/v1` suffix).
    pub(crate) api_base: String,
    /// Gateway bearer credential. Never printed.
    pub(crate) relay_key: String,
}

/// Parse the credentials JSON.
///
/// `serde_json::Value` accessors only: `librefang-cli` has no `serde`
/// dependency, so a `#[derive(Deserialize)]` struct is not available here.
/// Unknown extra fields are ignored by construction, which keeps this
/// forward-compatible with whatever EveryAPI adds to the file next.
///
/// The `Err` string is an i18n *key*, not user-facing prose — the caller
/// renders it. Returning a key rather than a message keeps this function
/// pure and unit-testable without a loaded locale bundle.
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

/// `$XDG_CONFIG_HOME/everyapi/credentials.json`, falling back to
/// `~/.config/everyapi/credentials.json`.
///
/// Mirrors `doctor::everyapi_credentials_path`, which is private to that
/// module (the doctor check and this command must agree on the location, so
/// any change here needs the same change there).
fn credentials_path() -> Option<PathBuf> {
    let root = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()?.join(".config"),
    };
    Some(root.join("everyapi").join("credentials.json"))
}

/// Derive the provider `base_url` from the credentials' `api_base`.
///
/// The OpenAI driver builds request URLs as `"{base_url}/chat/completions"`
/// (`openai.rs`), so the `/v1` segment must live in `base_url` itself. It
/// also strips one trailing slash from `base_url`, because
/// `"https://x/v1/" + "/chat/completions"` yields a doubled separator that
/// several gateways answer with a 504. Trimming here means the stored value
/// is already canonical instead of relying on that downstream repair.
pub(crate) fn derive_base_url(api_base: &str) -> String {
    format!("{}/v1", api_base.trim().trim_end_matches('/'))
}

// ---------------------------------------------------------------------------
// Gateway model list
// ---------------------------------------------------------------------------

/// One entry of the gateway's `GET /v1/models` response, reduced to the
/// fields that affect catalog synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatewayModel {
    pub(crate) id: String,
    /// Vendor label — the snapshot lookup's namespace (`anthropic`,
    /// `openai`, `google`, `minimax`, …). Empty when the gateway omits it.
    pub(crate) owned_by: String,
    /// `supported_endpoint_types`, verbatim. Drives modality inference and
    /// the streaming-only warning.
    pub(crate) supported_endpoint_types: Vec<String>,
    /// `context_window` when the gateway published one. Authoritative over
    /// the snapshot: the gateway knows what *it* will accept.
    pub(crate) context_window: Option<u64>,
}

/// Extract the model array from a `/v1/models` body.
///
/// Entries without a usable `id` are dropped rather than failing the whole
/// response — one malformed row should not cost the user the other 17.
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
            })
        })
        .collect()
}

/// Infer a model's modality from `supported_endpoint_types`.
///
/// The gateway does not publish a modality field, so the endpoint-type list
/// is the only signal. An EMPTY list means video: the `doubao-seedance-*`
/// family publishes `[]` and is video-generation only. That case must be
/// checked before the text default, otherwise those entries would be graded
/// as text models and then dropped for missing a context window.
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

/// Whether a model only speaks the `openai-response` endpoint shape.
///
/// Such models (the `gpt-5.6-*` family) reject non-streaming requests with
/// HTTP 400 `"Stream must be set to true"`. Everything in LibreFang that
/// calls `driver.complete()` rather than the streaming entry point — the
/// compactor, proactive memory, the skill workshop, web augmentation — would
/// fail against one, so they are excluded from automatic default selection
/// and called out in the command's report.
pub(crate) fn is_openai_response_only(supported_endpoint_types: &[String]) -> bool {
    !supported_endpoint_types.is_empty()
        && supported_endpoint_types
            .iter()
            .all(|t| t == "openai-response")
}

// ---------------------------------------------------------------------------
// Metadata resolution
// ---------------------------------------------------------------------------

/// Context / pricing metadata resolved for one text model.
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
}

/// Rewrite a `-`-separated version tail into the `.`-separated spelling.
///
/// The gateway and the OpenRouter snapshot disagree on one character:
/// gateway `claude-haiku-4-5` is snapshot `anthropic/claude-haiku-4.5`.
/// Only a hyphen sitting between two ASCII digits is rewritten, so
/// `gpt-5.6-luna` and `doubao-seedance-1-0-pro-250528` keep every other
/// hyphen intact.
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
/// `ModelCatalog::find_model` already lowercases both sides, which covers
/// the gateway's `MiniMax-M3` vs the snapshot's `minimax/minimax-m3`; the
/// only remaining mismatch is the version-tail separator, handled by
/// [`normalize_version_separators`].
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

/// Look one gateway model up in the compiled-in OpenRouter snapshot.
///
/// The snapshot is `include_str!`-ed into `librefang-runtime` and merged by
/// every `ModelCatalog` construction, so an empty-directory catalog is a
/// sufficient (and hermetic) handle on it.
///
/// `gateway_context_window` wins over the snapshot's when present: the
/// snapshot describes what OpenRouter's copy of the model accepts, whereas
/// the gateway is authoritative about its own deployment.
pub(crate) fn resolve_metadata(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    model: &GatewayModel,
) -> Option<ModelMetadata> {
    let entry = snapshot_lookup_ids(&model.owned_by, &model.id)
        .into_iter()
        .find_map(|id| catalog.find_model(&id))?;
    Some(ModelMetadata {
        context_window: model.context_window.unwrap_or(entry.context_window),
        max_output_tokens: entry.max_output_tokens,
        input_cost_per_m: entry.input_cost_per_m,
        output_cost_per_m: entry.output_cost_per_m,
        pricing_known: entry.pricing_known,
        supports_tools: entry.supports_tools,
        supports_vision: entry.supports_vision,
        supports_thinking: entry.supports_thinking,
    })
}

// ---------------------------------------------------------------------------
// Catalog synthesis
// ---------------------------------------------------------------------------

/// A model the command refused to register, with the i18n key explaining why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedModel {
    pub(crate) id: String,
    /// i18n key for the reason. A key rather than rendered prose so the
    /// synthesis stays pure and the locale files stay grep-able (each key
    /// appears verbatim in this file, which `test_no_dead_locale_keys`
    /// requires).
    pub(crate) reason_key: &'static str,
}

/// Everything the report needs out of one synthesis run.
pub(crate) struct SynthesisResult {
    pub(crate) file: ModelCatalogFile,
    pub(crate) skipped: Vec<SkippedModel>,
    /// Registered ids that reject non-streaming requests.
    pub(crate) streaming_only: Vec<String>,
}

/// Build the `providers/everyapi.toml` contents.
///
/// The skip rule is the load-bearing part. `ModelCatalogEntry::validate()`
/// rejects a `Modality::Text` entry whose `context_window` OR
/// `max_output_tokens` is 0, and `merge_catalog_file` then drops it with only
/// a `warn!`. The gateway publishes no max-output field at all, so a text
/// model's `max_output_tokens` is resolvable ONLY from a snapshot hit —
/// meaning a text model with no snapshot hit cannot produce a loadable entry
/// even when the gateway supplied its own `context_window`. Emitting it
/// anyway would make the feature look successful while the model silently
/// vanished at the next daemon boot, so such models are skipped and reported
/// by name instead. Inventing a plausible-looking fallback would be worse:
/// a wrong context window feeds straight into compaction thresholds and
/// budget math.
///
/// Non-text models are exempt from `validate()`'s token checks and are
/// registered with whatever the gateway published.
///
/// Output is sorted by id (repo invariant #3298) so re-running the command
/// against an unchanged gateway rewrites a byte-identical file.
pub(crate) fn synthesize_catalog(
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    base_url: &str,
    models: &[GatewayModel],
) -> SynthesisResult {
    let mut entries: Vec<ModelCatalogEntry> = Vec::new();
    let mut skipped: Vec<SkippedModel> = Vec::new();
    let mut streaming_only: Vec<String> = Vec::new();

    for model in models {
        let modality = infer_modality(&model.supported_endpoint_types);
        let metadata = resolve_metadata(catalog, model);

        if modality == Modality::Text && metadata.is_none() {
            skipped.push(SkippedModel {
                id: model.id.clone(),
                reason_key: "everyapi-connect-skip-no-metadata",
            });
            continue;
        }

        if is_openai_response_only(&model.supported_endpoint_types) {
            streaming_only.push(model.id.clone());
        }

        let metadata = metadata.unwrap_or(ModelMetadata {
            context_window: 0,
            max_output_tokens: 0,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            // No snapshot hit means no pricing was resolved. `pricing_known`
            // defaults to `true` on deserialize, so leaving it unset here
            // would assert that 0.0/0.0 is the model's *real* price and
            // poison every budget and metering figure derived from it.
            pricing_known: false,
            supports_tools: false,
            supports_vision: false,
            supports_thinking: false,
        });

        entries.push(ModelCatalogEntry {
            id: model.id.clone(),
            display_name: model.id.clone(),
            provider: PROVIDER_ID.to_string(),
            tier: ModelTier::Custom,
            modality,
            context_window: metadata.context_window,
            max_output_tokens: metadata.max_output_tokens,
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
                media_capabilities: Vec::new(),
            }),
            models: entries,
        },
        skipped,
        streaming_only,
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
    text_models()
        .find(|m| !streaming_only.contains(&m.id))
        .or_else(|| text_models().next())
        .map(|m| m.id.clone())
}

// ---------------------------------------------------------------------------
// Command entry point
// ---------------------------------------------------------------------------

/// Fetch `GET {base_url}/models` from the gateway.
///
/// `base_url` already ends in `/v1`, so this hits `/v1/models`. Returns
/// `None` on any transport error or non-2xx status: an unreachable model
/// list is not fatal, because the `[provider]` section alone already makes
/// the gateway usable with a hand-specified model id.
///
/// `key` is the relay credential. It is named to match
/// `daemon_client_with_api_key` in `commands/common.rs` so the header
/// construction reads identically at both call sites; it is only ever
/// written into the outbound header, never into output or a log.
fn fetch_gateway_models(base_url: &str, key: &str) -> Option<Vec<GatewayModel>> {
    let client = crate::http_client::client_builder()
        .timeout(MODELS_FETCH_TIMEOUT)
        .build()
        .ok()?;
    let response = client
        .get(format!("{base_url}/models"))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {key}"))
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<serde_json::Value>().ok()?;
    Some(parse_gateway_models(&body))
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
/// Returns `true` when the daemon accepted the write. The endpoint converts
/// the flat JSON body into the `[provider] … [[models]]` layout itself, then
/// reloads the in-process catalog — so on this path there is nothing for the
/// operator to restart. No `api_key` field is sent: the relay key is already
/// in `~/.librefang/.env`, and including it would additionally copy the
/// secret into `secrets.env`.
///
/// `daemon_json_checked` rather than `daemon_json`, because the latter exits
/// the process on a transport error and returns an empty body (not an error)
/// on 4xx — either way the caller could never fall back to the direct write.
fn write_provider_via_daemon(base: &str, body: &serde_json::Value) -> bool {
    let client = daemon_client();
    let (status, response) = daemon_json_checked(
        client
            .post(format!(
                "{base}/api/registry/content/provider?allow_overwrite=true"
            ))
            .json(body)
            .send(),
    );
    status.is_success() && response.get("error").is_none()
}

/// Write `{librefang_home}/providers/everyapi.toml` directly.
///
/// The path is fixed by agreement with `doctor::EveryApiWiringCheck`, which
/// looks for exactly this file to decide whether LibreFang is wired to the
/// gateway.
fn write_provider_file(toml_body: &str) -> Result<PathBuf, String> {
    let dir = cli_librefang_home().join("providers");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("everyapi.toml");
    std::fs::write(&path, toml_body).map_err(|e| e.to_string())?;
    Ok(path)
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
        Some(models) => models.as_slice(),
        None => {
            ui::warn_with_fix(
                &i18n::t("everyapi-connect-models-fetch-failed"),
                &i18n::t("everyapi-connect-models-fetch-failed-fix"),
            );
            &[]
        }
    };

    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    let synthesis = synthesize_catalog(&catalog, &base_url, models);
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
    let via_daemon = daemon.as_deref().is_some_and(|base| {
        write_provider_via_daemon(base, &provider_request_body(&synthesis.file))
    });
    if via_daemon {
        ui::success(&i18n::t_args(
            "everyapi-connect-provider-written-daemon",
            &[("path", "providers/everyapi.toml")],
        ));
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
    handle_default_model(&synthesis, daemon.as_deref(), set_default);
}

/// Flat JSON body for `POST /api/registry/content/provider`.
///
/// The endpoint's `normalize_provider_body` nests every non-`models` key
/// under `provider` itself, so the body is sent flat with `models` alongside.
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
/// Written by hand rather than via `serde_json::to_value` because the CLI has
/// no `serde` dependency to reach `Serialize` through — and because the
/// endpoint's `json_to_toml_value` drops empty strings/arrays anyway, so only
/// the fields that must survive are sent. `pricing_known` is always included:
/// it defaults to `true` on deserialize, so omitting it on an unpriced model
/// would silently claim the model is free.
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
        choose_default_model, derive_base_url, infer_modality, is_openai_response_only,
        model_request_value, normalize_version_separators, parse_credentials, parse_gateway_models,
        provider_request_body, resolve_metadata, snapshot_lookup_ids, synthesize_catalog,
        GatewayModel,
    };
    use crate::cli::{Cli, Commands, ModelsCommands};
    use clap::Parser;
    use librefang_runtime::model_catalog::ModelCatalog;
    use librefang_types::model_catalog::{Modality, ModelCatalogFile};
    use serde_json::json;

    /// A catalog built over an empty directory. The OpenRouter snapshot is
    /// `include_str!`-ed into `librefang-runtime` and merged unconditionally
    /// by every constructor, so this is a hermetic handle on the snapshot
    /// with no user providers mixed in.
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
        }
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
            infer_modality(&types.iter().map(|s| s.to_string()).collect::<Vec<_>>())
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

    // ── snapshot lookup ───────────────────────────────────────────────────

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
    fn snapshot_exact_hit_carries_context_and_pricing() {
        let catalog = snapshot_catalog();
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("claude-sonnet-5", "anthropic", &["openai", "anthropic"]),
        )
        .expect("anthropic/claude-sonnet-5 is in the snapshot");
        assert_eq!(meta.context_window, 1_000_000);
        assert_eq!(meta.max_output_tokens, 128_000);
        assert_eq!(meta.input_cost_per_m, 2.0);
        assert_eq!(meta.output_cost_per_m, 10.0);
        assert!(meta.pricing_known);
    }

    #[test]
    fn snapshot_hit_is_case_insensitive() {
        // Gateway says "MiniMax-M3"; the snapshot stores "minimax/minimax-m3".
        let catalog = snapshot_catalog();
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("MiniMax-M3", "minimax", &["openai"]),
        )
        .expect("minimax/minimax-m3 resolves case-insensitively");
        assert_eq!(meta.context_window, 1_048_576);
        assert_eq!(meta.input_cost_per_m, 0.3);
        assert_eq!(meta.output_cost_per_m, 1.2);
    }

    #[test]
    fn snapshot_hit_via_normalized_version_tail() {
        // Gateway "claude-haiku-4-5" vs snapshot "anthropic/claude-haiku-4.5".
        let catalog = snapshot_catalog();
        let meta = resolve_metadata(
            &catalog,
            &gateway_model("claude-haiku-4-5", "anthropic", &["openai"]),
        )
        .expect("normalized retry resolves");
        assert_eq!(meta.context_window, 200_000);
        assert_eq!(meta.max_output_tokens, 64_000);
    }

    #[test]
    fn snapshot_genuine_miss_returns_none() {
        let catalog = snapshot_catalog();
        assert!(resolve_metadata(
            &catalog,
            &gateway_model("doubao-seedance-2-0-260128", "doubao", &[])
        )
        .is_none());
        assert!(resolve_metadata(
            &catalog,
            &gateway_model("gemini-3-flash", "google", &["openai"])
        )
        .is_none());
    }

    #[test]
    fn gateway_context_window_overrides_the_snapshot() {
        let catalog = snapshot_catalog();
        let mut model = gateway_model("claude-sonnet-5", "anthropic", &["openai"]);
        model.context_window = Some(64_000);
        let meta = resolve_metadata(&catalog, &model).expect("hit");
        assert_eq!(meta.context_window, 64_000, "gateway wins over snapshot");
        // max_output_tokens still comes from the snapshot — the gateway
        // publishes no such field.
        assert_eq!(meta.max_output_tokens, 128_000);
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
    fn text_model_without_resolvable_metadata_is_skipped_not_emitted_with_zero() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
        );

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
    fn a_gateway_context_window_alone_does_not_rescue_a_snapshot_miss() {
        // A text model the snapshot does not know, but for which the gateway
        // published a context_window. `max_output_tokens` is unobtainable —
        // the gateway has no such field — so the entry would be dropped by
        // `validate()` downstream. It must be skipped loudly instead.
        let catalog = snapshot_catalog();
        let mut model = gateway_model("totally-unknown-model", "someone", &["openai"]);
        model.context_window = Some(128_000);
        let result = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &[model]);
        assert!(result.file.models.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].id, "totally-unknown-model");
    }

    #[test]
    fn non_text_models_survive_without_any_snapshot_metadata() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
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
    fn pricing_known_is_false_whenever_pricing_did_not_resolve() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
        );
        for model in &result.file.models {
            let has_snapshot_pricing =
                model.input_cost_per_m > 0.0 || model.output_cost_per_m > 0.0;
            if !has_snapshot_pricing {
                assert!(
                    !model.pricing_known,
                    "{} claims known pricing with zero cost",
                    model.id
                );
            }
        }
        // The doubao/system families have no snapshot presence at all.
        let unpriced: Vec<&str> = result
            .file
            .models
            .iter()
            .filter(|m| !m.pricing_known)
            .map(|m| m.id.as_str())
            .collect();
        assert!(unpriced.contains(&"tts-1"));
        assert!(unpriced.contains(&"doubao-seedream-4-0-250828"));
        assert!(!unpriced.contains(&"claude-sonnet-5"));
    }

    #[test]
    fn output_is_byte_identical_regardless_of_input_order() {
        let catalog = snapshot_catalog();
        let forward = live_gateway_listing();
        let mut reversed = forward.clone();
        reversed.reverse();

        let a = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &forward);
        let b = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &reversed);

        let render = |file: &ModelCatalogFile| toml::to_string_pretty(file).expect("serializes");
        assert_eq!(render(&a.file), render(&b.file));
        assert_eq!(a.skipped, b.skipped);
        assert_eq!(a.streaming_only, b.streaming_only);
    }

    #[test]
    fn default_choice_avoids_the_streaming_only_family() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
        );
        assert_eq!(
            result.streaming_only,
            vec!["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.6-terra"]
        );
        // Entries are id-sorted and ASCII puts uppercase ahead of lowercase,
        // so `MiniMax-M3` is the first non-streaming-only text model.
        let chosen =
            choose_default_model(&result.file.models, &result.streaming_only).expect("a default");
        assert_eq!(chosen, "MiniMax-M3");
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
        );
        // Still returns something rather than silently doing nothing, but the
        // caller has already warned about the streaming requirement.
        assert_eq!(
            choose_default_model(&result.file.models, &result.streaming_only).as_deref(),
            Some("gpt-5.6-luna")
        );
    }

    #[test]
    fn default_choice_is_none_when_no_text_model_is_registered() {
        let catalog = snapshot_catalog();
        let media_only = vec![
            gateway_model("tts-1", "system", &["audio-speech"]),
            gateway_model("doubao-seedance-2-0-260128", "doubao", &[]),
        ];
        let result = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &media_only);
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

    // ── daemon request body ───────────────────────────────────────────────

    #[test]
    fn daemon_body_is_flat_carries_no_secret_and_keeps_pricing_known() {
        let catalog = snapshot_catalog();
        let result = synthesize_catalog(
            &catalog,
            "https://api.everyapi.ai/v1",
            &live_gateway_listing(),
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
        // copy the secret into the daemon's secrets.env.
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
        let tts = result
            .file
            .models
            .iter()
            .find(|m| m.id == "tts-1")
            .expect("tts-1 registered");
        assert_eq!(model_request_value(tts)["pricing_known"], json!(false));
        assert_eq!(model_request_value(tts)["modality"], json!("audio"));
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
        let result = synthesize_catalog(&catalog, "https://api.everyapi.ai/v1", &[]);
        let rendered = toml::to_string_pretty(&result.file).expect("serializes");
        let parsed: ModelCatalogFile = toml::from_str(&rendered).expect("round trips");
        assert!(parsed.models.is_empty());
        assert_eq!(parsed.provider.expect("provider").id, "everyapi");
    }
}
