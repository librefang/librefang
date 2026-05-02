//! Model catalog, provider management, and Copilot OAuth handlers.

/// Build routes for the model/provider domain.
pub fn router() -> axum::Router<std::sync::Arc<super::AppState>> {
    axum::Router::new()
        .route("/models", axum::routing::get(list_models))
        .route(
            "/models/aliases",
            axum::routing::get(list_aliases).post(create_alias),
        )
        .route(
            "/models/aliases/{alias}",
            axum::routing::delete(delete_alias),
        )
        .route("/models/custom", axum::routing::post(add_custom_model))
        .route(
            "/models/custom/{*id}",
            axum::routing::delete(remove_custom_model),
        )
        .route(
            "/models/overrides/{*id}",
            axum::routing::get(get_model_overrides)
                .put(set_model_overrides)
                .delete(delete_model_overrides),
        )
        .route("/models/{*id}", axum::routing::get(get_model))
        .route("/providers", axum::routing::get(list_providers))
        .route("/catalog/update", axum::routing::post(catalog_update))
        .route("/catalog/status", axum::routing::get(catalog_status))
        .route(
            "/providers/ollama/detect",
            axum::routing::get(detect_ollama),
        )
        .route(
            "/providers/github-copilot/oauth/start",
            axum::routing::post(copilot_oauth_start),
        )
        .route(
            "/providers/github-copilot/oauth/poll/{poll_id}",
            axum::routing::get(copilot_oauth_poll),
        )
        .route(
            "/providers/{name}/key",
            axum::routing::post(set_provider_key).delete(delete_provider_key),
        )
        .route("/providers/{name}/test", axum::routing::post(test_provider))
        .route(
            "/providers/{name}/url",
            axum::routing::put(set_provider_url),
        )
        .route("/providers/{name}", axum::routing::get(get_provider))
        .route(
            "/providers/{name}/default",
            axum::routing::post(set_default_provider),
        )
}

use super::skills::{remove_secret_env, write_secret_env};
use super::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use crate::types::ApiErrorResponse;
#[utoipa::path(
    get,
    path = "/api/models",
    tag = "models",
    operation_id = "list_all_models",
    responses(
        (status = 200, description = "List available models", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // Pre-compute the live-discovered model ID set per local provider so we
    // can hide static catalog entries whose IDs aren't actually exposed by
    // the user's running daemon. Issue #3191: a user pointing the `ollama`
    // provider slot at Lemonade Server saw `gemma4` (a real Ollama tag, but
    // not on Lemonade) listed in the Models page, picked it, and got
    // "model not found" from the chat call. The catalog still ships static
    // entries for upstream-known Ollama models — those are correct for an
    // actual Ollama install, but wrong for any other OpenAI-compatible
    // server that happens to be configured under the same provider slot.
    //
    // Policy:
    //   - probe cache hit + reachable + non-empty discovered list → only
    //     keep catalog entries whose ID matches a discovered name.
    //   - probe failed / cache miss → keep all static entries (don't make
    //     things worse than the pre-fix state when we can't see live).
    //   - Custom-tier models (user-added via /api/models/custom) always pass
    //     through — they're explicit user intent, not catalog inheritance.
    use std::collections::HashSet;
    // Index each discovered name both verbatim and with `:latest` stripped so static entries survive whether Ollama returns `llama3.2` or `llama3.2:latest`.
    fn strip_latest(s: &str) -> &str {
        s.strip_suffix(":latest").unwrap_or(s)
    }
    let live_models_per_provider: std::collections::HashMap<String, HashSet<String>> = catalog
        .list_providers()
        .iter()
        .filter(|p| librefang_runtime::provider_health::is_local_provider(&p.id))
        .filter_map(|p| {
            let probe = state.provider_probe_cache.get(&p.id)?;
            if !probe.reachable || probe.discovered_models.is_empty() {
                return None;
            }
            let mut set: HashSet<String> = HashSet::new();
            for s in &probe.discovered_models {
                let lower = s.to_lowercase();
                set.insert(strip_latest(&lower).to_string());
                set.insert(lower);
            }
            Some((p.id.to_lowercase(), set))
        })
        .collect();

    let models: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .filter(|m| {
            if let Some(ref p) = provider_filter {
                if m.provider.to_lowercase() != *p {
                    return false;
                }
            }
            if let Some(ref t) = tier_filter {
                if m.tier.to_string() != *t {
                    return false;
                }
            }
            if available_only {
                let provider = catalog.get_provider(&m.provider);
                if let Some(p) = provider {
                    if !p.auth_status.is_available() {
                        return false;
                    }
                }
            }
            // Live-discovered filter for local providers (see comment above).
            if m.tier != librefang_types::model_catalog::ModelTier::Custom {
                if let Some(live_set) = live_models_per_provider.get(&m.provider.to_lowercase()) {
                    let lower = m.id.to_lowercase();
                    let bare = strip_latest(&lower);
                    if !live_set.contains(&lower) && !live_set.contains(bare) {
                        return false;
                    }
                }
            }
            true
        })
        .map(|m| {
            // Custom models from unknown providers are assumed available
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status.is_available())
                .unwrap_or(m.tier == librefang_types::model_catalog::ModelTier::Custom);
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": m.provider,
                "tier": m.tier,
                "modality": m.modality,
                "context_window": m.context_window,
                "max_output_tokens": m.max_output_tokens,
                "input_cost_per_m": m.input_cost_per_m,
                "output_cost_per_m": m.output_cost_per_m,
                "image_input_cost_per_m": m.image_input_cost_per_m,
                "image_output_cost_per_m": m.image_output_cost_per_m,
                "supports_tools": m.supports_tools,
                "supports_vision": m.supports_vision,
                "supports_streaming": m.supports_streaming,
                "supports_thinking": m.supports_thinking,
                "aliases": m.aliases,
                "available": available,
            })
        })
        .collect();

    let total = catalog.list_models().len();
    let available_count = catalog.available_models().len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": models,
            "total": total,
            "available": available_count,
        })),
    )
}

#[utoipa::path(get, path = "/api/models/aliases", tag = "models", responses((status = 200, description = "List model aliases", body = crate::types::JsonObject)))]
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list_aliases()
        .clone();
    let entries: Vec<serde_json::Value> = aliases
        .iter()
        .map(|(alias, model_id)| {
            serde_json::json!({
                "alias": alias,
                "model_id": model_id,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "aliases": entries,
            "total": entries.len(),
        })),
    )
}

/// POST /api/models/aliases — Create a new alias mapping.
///
/// Body: `{ "alias": "my-alias", "model_id": "gpt-4o" }`
#[utoipa::path(post, path = "/api/models/aliases", tag = "models", request_body = crate::types::JsonObject, responses((status = 200, description = "Alias created", body = crate::types::JsonObject)))]
pub async fn create_alias(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let alias = body
        .get("alias")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_id = body
        .get("model_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if alias.is_empty() {
        return ApiErrorResponse::bad_request("Missing required field: alias").into_json_tuple();
    }
    if model_id.is_empty() {
        return ApiErrorResponse::bad_request("Missing required field: model_id").into_json_tuple();
    }

    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_alias(&alias, &model_id) {
        return ApiErrorResponse::conflict(format!("Alias '{}' already exists", alias))
            .into_json_tuple();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "alias": alias.to_lowercase(),
            "model_id": model_id,
            "status": "created"
        })),
    )
}

/// DELETE /api/models/aliases/{alias} — Remove an alias mapping.
#[utoipa::path(delete, path = "/api/models/aliases/{alias}", tag = "models", params(("alias" = String, Path, description = "Alias name")), responses((status = 200, description = "Alias deleted")))]
pub async fn delete_alias(
    State(state): State<Arc<AppState>>,
    Path(alias): Path<String>,
) -> impl IntoResponse {
    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_alias(&alias) {
        return ApiErrorResponse::not_found(format!("Alias '{}' not found", alias))
            .into_json_tuple();
    }

    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

#[utoipa::path(get, path = "/api/models/{id}", tag = "models", params(("id" = String, Path, description = "Model ID")), responses((status = 200, description = "Model details", body = crate::types::JsonObject)))]
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(m) => {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status.is_available())
                .unwrap_or(m.tier == librefang_types::model_catalog::ModelTier::Custom);
            let override_key = format!("{}:{}", m.provider, m.id);
            let overrides = catalog.get_overrides(&override_key);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": m.id,
                    "display_name": m.display_name,
                    "provider": m.provider,
                    "tier": m.tier,
                    "modality": m.modality,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "input_cost_per_m": m.input_cost_per_m,
                    "output_cost_per_m": m.output_cost_per_m,
                    "image_input_cost_per_m": m.image_input_cost_per_m,
                    "image_output_cost_per_m": m.image_output_cost_per_m,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "supports_streaming": m.supports_streaming,
                    "aliases": m.aliases,
                    "available": available,
                    "overrides": overrides,
                })),
            )
        }
        None => ApiErrorResponse::not_found(format!("Model '{}' not found", id)).into_json_tuple(),
    }
}

// ── Per-model overrides ─────────────────────────────────────────────────────

/// GET /api/models/overrides/{id} — Get inference parameter overrides for a model.
pub async fn get_model_overrides(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.get_overrides(&id) {
        Some(o) => (StatusCode::OK, Json(serde_json::to_value(o).unwrap())),
        None => (StatusCode::OK, Json(serde_json::json!({}))),
    }
}

/// PUT /api/models/overrides/{id} — Set inference parameter overrides for a model.
pub async fn set_model_overrides(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<librefang_types::model_catalog::ModelOverrides>,
) -> impl IntoResponse {
    let overrides_path = state
        .kernel
        .home_dir()
        .join("data")
        .join("model_overrides.json");
    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let previous = catalog.get_overrides(&id).cloned();
    catalog.set_overrides(id.clone(), body);
    if let Err(e) = catalog.save_overrides(&overrides_path) {
        tracing::warn!("Failed to persist model overrides: {e}");
        // Roll back in-memory change so catalog stays consistent with disk.
        match previous {
            Some(prev) => catalog.set_overrides(id, prev),
            None => {
                catalog.remove_overrides(&id);
            }
        }
        return ApiErrorResponse::internal(format!("Failed to persist overrides: {e}"))
            .into_json_tuple();
    }
    // Return the persisted overrides entity so callers can `setQueryData`
    // without a follow-up GET. (Refs #3832.)
    let persisted = catalog.get_overrides(&id).cloned().unwrap_or_default();
    (
        StatusCode::OK,
        Json(serde_json::to_value(persisted).unwrap_or_else(|_| serde_json::json!({}))),
    )
}

/// DELETE /api/models/overrides/{id} — Remove inference parameter overrides for a model.
pub async fn delete_model_overrides(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let overrides_path = state
        .kernel
        .home_dir()
        .join("data")
        .join("model_overrides.json");
    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    catalog.remove_overrides(&id);
    if let Err(e) = catalog.save_overrides(&overrides_path) {
        tracing::warn!("Failed to persist model overrides: {e}");
    }
    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

/// Attach local-provider probe results to a JSON entry and optionally merge
/// discovered models into the catalog.
fn attach_probe_result(
    entry: &mut serde_json::Value,
    probe: &librefang_runtime::provider_health::ProbeResult,
    provider_id: &str,
    catalog: &std::sync::RwLock<librefang_runtime::model_catalog::ModelCatalog>,
) {
    entry["is_local"] = serde_json::json!(true);
    entry["reachable"] = serde_json::json!(probe.reachable);
    entry["latency_ms"] = serde_json::json!(probe.latency_ms);
    if !probe.discovered_models.is_empty() {
        entry["discovered_models"] = serde_json::json!(&probe.discovered_models);
        if let Ok(mut cat) = catalog.write() {
            let info: Vec<_> = if probe.discovered_model_info.is_empty() {
                probe
                    .discovered_models
                    .iter()
                    .map(
                        |name| librefang_runtime::provider_health::DiscoveredModelInfo {
                            name: name.clone(),
                            parameter_size: None,
                            quantization_level: None,
                            family: None,
                            families: None,
                            size: None,
                            capabilities: vec![],
                        },
                    )
                    .collect()
            } else {
                probe.discovered_model_info.clone()
            };
            cat.merge_discovered_models(provider_id, &info);
        }
    }
    if !probe.discovered_model_info.is_empty() {
        entry["discovered_model_info"] = serde_json::json!(&probe.discovered_model_info);
    }
    if let Some(err) = &probe.error {
        entry["error_message"] = serde_json::json!(err);
    }
    entry["last_tested"] = serde_json::json!(&probe.probed_at);
}

/// GET /api/providers — List all providers with auth status.
///
/// For local providers (ollama, vllm, lmstudio), also probes reachability and
/// discovers available models via their health endpoints.
///
/// Probes run **concurrently** and results are **cached for 60 seconds** so the
/// endpoint responds instantly on repeated dashboard loads even when local
/// services are offline.
#[utoipa::path(
    get,
    path = "/api/providers",
    tag = "models",
    responses(
        (status = 200, description = "List configured providers", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let provider_list: Vec<librefang_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    // Collect local providers that need probing
    let local_providers: Vec<(usize, String, String, Option<String>)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            librefang_runtime::provider_health::is_local_provider(&p.id) && !p.base_url.is_empty()
        })
        .map(|(i, p)| {
            // Resolve the provider's api_key env var (catalog field, falling
            // back to the {PROVIDER}_API_KEY convention) and read its value
            // for the probe. Local providers fronted by an authenticating
            // reverse proxy (Open WebUI, LiteLLM, etc.) need this Bearer
            // token forwarded; bare-localhost setups have nothing in the
            // env so the probe runs unauthenticated as before.
            let env_var = if p.api_key_env.trim().is_empty() {
                format!("{}_API_KEY", p.id.to_uppercase().replace('-', "_"))
            } else {
                p.api_key_env.clone()
            };
            let api_key = std::env::var(&env_var)
                .ok()
                .filter(|v| !v.trim().is_empty());
            (i, p.id.clone(), p.base_url.clone(), api_key)
        })
        .collect();

    // Fire all probes concurrently (cached results return instantly)
    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url, api_key)| {
            librefang_runtime::provider_health::probe_provider_cached(
                id,
                url,
                api_key.as_deref(),
                cache,
            )
        })
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    // Index probe results by provider list position for O(1) lookup
    let mut probe_map: HashMap<usize, librefang_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((idx, _, _, _), result) in local_providers.iter().zip(probe_results) {
        probe_map.insert(*idx, result);
    }

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());

    for (i, p) in provider_list.iter().enumerate() {
        let mut entry = serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "auth_status": p.auth_status,
            "model_count": p.model_count,
            "key_required": p.key_required,
            "api_key_env": p.api_key_env,
            "base_url": p.base_url,
            "proxy_url": p.proxy_url,
            "media_capabilities": p.media_capabilities,
            "is_custom": p.is_custom,
        });

        // Attach region map so the dashboard can show available regions
        if !p.regions.is_empty() {
            let regions: serde_json::Map<String, serde_json::Value> = p
                .regions
                .iter()
                .map(|(name, rc)| {
                    (
                        name.clone(),
                        serde_json::json!({
                            "base_url": rc.base_url,
                            "api_key_env": rc.api_key_env,
                        }),
                    )
                })
                .collect();
            entry["regions"] = serde_json::Value::Object(regions);

            // Mark which region is active (if configured via [provider_regions])
            if let Some(active) = state.kernel.config_ref().provider_regions.get(&p.id) {
                entry["active_region"] = serde_json::json!(active);
            }
        }

        // For local providers, attach the probe result and downgrade
        // auth_status when the service is not reachable so the dashboard
        // shows "needs setup" instead of "configured".
        if let Some(probe) = probe_map.remove(&i) {
            attach_probe_result(&mut entry, &probe, &p.id, state.kernel.model_catalog_ref());
            if !probe.reachable {
                entry["auth_status"] = serde_json::json!("missing");
            }
        } else if librefang_runtime::provider_health::is_local_provider(&p.id) {
            // Local HTTP provider with no probe result yet — still label it local.
            entry["is_local"] = serde_json::json!(true);
        }

        // Attach cached manual test result if no probe already set it.
        // TTL: 10 minutes — stale results are ignored.
        if let Some(ref_entry) = state.provider_test_cache.get(&p.id) {
            let (tested_at, ms, tested_rfc3339, reachable) = ref_entry.value();
            if tested_at.elapsed() < std::time::Duration::from_secs(600) {
                if entry.get("latency_ms").is_none() || entry["latency_ms"].is_null() {
                    entry["latency_ms"] = serde_json::json!(ms);
                }
                entry["last_tested"] = serde_json::json!(tested_rfc3339);
                entry["reachable"] = serde_json::json!(reachable);
            }
        }

        providers.push(entry);
    }

    let total = providers.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "providers": providers,
            "total": total,
        })),
    )
}

/// Returns providers list for the dashboard snapshot endpoint.
pub(crate) async fn providers_snapshot(state: &Arc<AppState>) -> Vec<serde_json::Value> {
    let provider_list: Vec<librefang_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    let local_providers: Vec<(usize, String, String, Option<String>)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            librefang_runtime::provider_health::is_local_provider(&p.id) && !p.base_url.is_empty()
        })
        .map(|(i, p)| {
            // See sibling site above — same env-var resolution so Open WebUI
            // / LiteLLM-fronted local providers get a Bearer token attached.
            let env_var = if p.api_key_env.trim().is_empty() {
                format!("{}_API_KEY", p.id.to_uppercase().replace('-', "_"))
            } else {
                p.api_key_env.clone()
            };
            let api_key = std::env::var(&env_var)
                .ok()
                .filter(|v| !v.trim().is_empty());
            (i, p.id.clone(), p.base_url.clone(), api_key)
        })
        .collect();

    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url, api_key)| {
            librefang_runtime::provider_health::probe_provider_cached(
                id,
                url,
                api_key.as_deref(),
                cache,
            )
        })
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    let mut probe_map: HashMap<usize, librefang_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((idx, _, _, _), result) in local_providers.iter().zip(probe_results) {
        probe_map.insert(*idx, result);
    }

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());
    for (i, p) in provider_list.iter().enumerate() {
        let mut entry = serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "auth_status": p.auth_status,
            "model_count": p.model_count,
            "key_required": p.key_required,
            "api_key_env": p.api_key_env,
            "base_url": p.base_url,
            "proxy_url": p.proxy_url,
            "media_capabilities": p.media_capabilities,
            "is_custom": p.is_custom,
        });
        if let Some(probe) = probe_map.remove(&i) {
            attach_probe_result(&mut entry, &probe, &p.id, state.kernel.model_catalog_ref());
            if !probe.reachable {
                entry["auth_status"] = serde_json::json!("missing");
            }
        } else if librefang_runtime::provider_health::is_local_provider(&p.id) {
            entry["is_local"] = serde_json::json!(true);
        }
        providers.push(entry);
    }

    providers
}

/// GET /api/providers/{name} — Get details for a single provider.
#[utoipa::path(
    get,
    path = "/api/providers/{name}",
    tag = "models",
    params(("name" = String, Path, description = "Provider identifier")),
    responses(
        (status = 200, description = "Provider details", body = crate::types::JsonObject),
        (status = 404, description = "Provider not found")
    )
)]
pub async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (provider, models) = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => {
                let models: Vec<serde_json::Value> = catalog
                    .models_by_provider(&name)
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id,
                            "display_name": m.display_name,
                            "tier": m.tier,
                            "modality": m.modality,
                            "context_window": m.context_window,
                            "max_output_tokens": m.max_output_tokens,
                            "input_cost_per_m": m.input_cost_per_m,
                            "output_cost_per_m": m.output_cost_per_m,
                            "image_input_cost_per_m": m.image_input_cost_per_m,
                            "image_output_cost_per_m": m.image_output_cost_per_m,
                            "supports_tools": m.supports_tools,
                            "supports_vision": m.supports_vision,
                            "supports_streaming": m.supports_streaming,
                        })
                    })
                    .collect();
                (p.clone(), models)
            }
            None => {
                return ApiErrorResponse::not_found(format!("Provider '{}' not found", name))
                    .into_json_tuple();
            }
        }
    };

    let mut entry = serde_json::json!({
        "id": provider.id,
        "display_name": provider.display_name,
        "auth_status": provider.auth_status,
        "model_count": provider.model_count,
        "key_required": provider.key_required,
        "api_key_env": provider.api_key_env,
        "base_url": provider.base_url,
        "proxy_url": provider.proxy_url,
        "models": models,
    });

    // For local providers, run a probe and attach the result
    if librefang_runtime::provider_health::is_local_provider(&provider.id)
        && !provider.base_url.is_empty()
    {
        let cache = &state.provider_probe_cache;
        // Forward the api_key when present so reverse-proxy-fronted local
        // providers (Open WebUI, LiteLLM) get a valid Bearer token.
        let env_var = if provider.api_key_env.trim().is_empty() {
            format!("{}_API_KEY", provider.id.to_uppercase().replace('-', "_"))
        } else {
            provider.api_key_env.clone()
        };
        let api_key = std::env::var(&env_var)
            .ok()
            .filter(|v| !v.trim().is_empty());
        let probe = librefang_runtime::provider_health::probe_provider_cached(
            &provider.id,
            &provider.base_url,
            api_key.as_deref(),
            cache,
        )
        .await;

        attach_probe_result(
            &mut entry,
            &probe,
            &provider.id,
            state.kernel.model_catalog_ref(),
        );
        if !probe.reachable {
            entry["auth_status"] = serde_json::json!("missing");
        }
    } else if librefang_runtime::provider_health::is_local_provider(&provider.id) {
        entry["is_local"] = serde_json::json!(true);
    }

    (StatusCode::OK, Json(entry))
}

/// POST /api/models/custom — Add a custom model to the catalog.
///
/// Persists to `~/.librefang/custom_models.json` and makes the model immediately
/// available in the catalog.
#[utoipa::path(post, path = "/api/models/custom", tag = "models", request_body = crate::types::JsonObject, responses((status = 200, description = "Custom model added", body = crate::types::JsonObject)))]
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let default_provider = state.kernel.config_ref().default_model.provider.clone();
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(default_provider);
    let context_window = body
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return ApiErrorResponse::bad_request("Missing required field: id").into_json_tuple();
    }

    let display = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let modality = match body.get("modality").and_then(|v| v.as_str()) {
        Some("image") => librefang_types::model_catalog::Modality::Image,
        Some("audio") => librefang_types::model_catalog::Modality::Audio,
        Some("video") => librefang_types::model_catalog::Modality::Video,
        Some("music") => librefang_types::model_catalog::Modality::Music,
        _ => librefang_types::model_catalog::Modality::Text,
    };

    let entry = librefang_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: librefang_types::model_catalog::ModelTier::Custom,
        modality,
        context_window,
        max_output_tokens: max_output,
        input_cost_per_m: body
            .get("input_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        output_cost_per_m: body
            .get("output_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        image_input_cost_per_m: body.get("image_input_cost_per_m").and_then(|v| v.as_f64()),
        image_output_cost_per_m: body.get("image_output_cost_per_m").and_then(|v| v.as_f64()),
        supports_tools: body
            .get("supports_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        supports_vision: body
            .get("supports_vision")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        supports_streaming: body
            .get("supports_streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        supports_thinking: body
            .get("supports_thinking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        aliases: vec![],
    };

    // Same modality-aware gate the catalog loaders apply: text entries
    // must have nonzero context_window and max_output_tokens. Reject
    // synchronously so misconfigured custom models can't enter the
    // catalog and propagate `0` into compaction / budget math.
    if let Err(e) = entry.validate() {
        return ApiErrorResponse::bad_request(e).into_json_tuple();
    }

    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return ApiErrorResponse::conflict(format!(
            "Model '{}' already exists for provider '{}'",
            id, provider
        ))
        .into_json_tuple();
    }

    // Persist to disk. If save fails, roll back the in-memory add so the
    // catalog stays consistent with what's on disk — otherwise the caller
    // sees "added" now but the model vanishes on the next daemon restart.
    let custom_path = state
        .kernel
        .home_dir()
        .join("data")
        .join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
        catalog.remove_custom_model(&id);
        return ApiErrorResponse::internal(format!("Failed to persist custom model: {e}"))
            .into_json_tuple();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "provider": provider,
            "status": "added"
        })),
    )
}

/// DELETE /api/models/custom/{id} — Remove a custom model.
#[utoipa::path(delete, path = "/api/models/custom/{id}", tag = "models", params(("id" = String, Path, description = "Model ID")), responses((status = 200, description = "Custom model removed")))]
pub async fn remove_custom_model(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let mut catalog = state
        .kernel
        .model_catalog_ref()
        .write()
        .unwrap_or_else(|e| e.into_inner());

    // Snapshot the entry before removing so we can restore it if the
    // subsequent persist fails — keeps the in-memory catalog consistent
    // with disk across failure paths.
    let snapshot = catalog.find_model(&model_id).cloned();
    if !catalog.remove_custom_model(&model_id) {
        return ApiErrorResponse::not_found(format!("Custom model '{}' not found", model_id))
            .into_json_tuple();
    }

    let custom_path = state
        .kernel
        .home_dir()
        .join("data")
        .join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
        if let Some(entry) = snapshot {
            catalog.add_custom_model(entry);
        }
        return ApiErrorResponse::internal(format!("Failed to persist custom model: {e}"))
            .into_json_tuple();
    }

    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

// ── A2A (Agent-to-Agent) Protocol Endpoints ─────────────────────────

#[utoipa::path(post, path = "/api/providers/{name}/key", tag = "models", params(("name" = String, Path, description = "Provider name")), request_body = crate::types::JsonObject, responses((status = 200, description = "API key set", body = crate::types::JsonObject)))]
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'key' field").into_json_tuple();
        }
    };

    // Look up env var from catalog; for unknown/custom providers derive one.
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .filter(|env| !env.trim().is_empty())
            .unwrap_or_else(|| {
                // Custom provider — derive env var: MY_PROVIDER → MY_PROVIDER_API_KEY
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    // Write to secrets.env file
    let secrets_path = state.kernel.home_dir().join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_var, &key) {
        return ApiErrorResponse::internal(format!("Failed to write secrets.env: {e}"))
            .into_json_tuple();
    }

    // Set env var in current process so detect_auth picks it up.
    // `std::env::set_var` is not thread-safe in an async context; delegate to
    // a blocking thread to avoid undefined behaviour in the tokio runtime.
    {
        let env_var_clone = env_var.clone();
        let key_clone = key.clone();
        let _ = tokio::task::spawn_blocking(move || {
            // SAFETY: single mutation on a dedicated blocking thread.
            unsafe { std::env::set_var(&env_var_clone, &key_clone) };
        })
        .await;
    }

    // Re-enable fallback detection (user is adding a key, undo any prior suppress)
    // and refresh auth status.
    {
        let mut catalog = state
            .kernel
            .model_catalog_ref()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.unsuppress_provider(&name);
        catalog.save_suppressed(
            &state
                .kernel
                .home_dir()
                .join("data")
                .join("suppressed_providers.json"),
        );
        catalog.detect_auth();
    }

    // Kick off a background probe to validate the new key immediately so the
    // dashboard reflects ValidatedKey / InvalidKey without waiting for restart.
    state.kernel.clone().spawn_key_validation();

    // Auto-switch default provider if current default has no working key.
    // This fixes the common case where a user adds e.g. a Gemini key via dashboard
    // but their agent still tries to use the previous provider (which has no key).
    //
    // Read the effective default from the hot-reload override (if set) rather than
    // the stale boot-time config — a previous set_provider_key call may have already
    // switched the default.
    let (current_provider, current_key_env) = {
        let guard = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => (dm.provider.clone(), dm.api_key_env.clone()),
            None => {
                let dm = state.kernel.config_ref().default_model.clone();
                (dm.provider, dm.api_key_env)
            }
        }
    };
    let current_has_key = if current_key_env.is_empty() {
        false
    } else {
        std::env::var(&current_key_env)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
    };
    let switched = if !current_has_key && current_provider != name {
        // Find a default model for the newly-keyed provider
        let default_model = {
            let catalog = state
                .kernel
                .model_catalog_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            catalog.default_model_for_provider(&name)
        };
        if let Some(model_id) = default_model {
            // Update config.toml to persist the switch
            let config_path = state.kernel.home_dir().join("config.toml");
            if let Err(e) = persist_default_model(&config_path, &name, &model_id, &env_var) {
                tracing::warn!("Failed to persist default_model to config.toml: {e}");
            }

            // Hot-update the in-memory default model override so resolve_driver()
            // immediately creates drivers for the new provider — no restart needed.
            {
                let new_dm = librefang_types::config::DefaultModelConfig {
                    provider: name.clone(),
                    model: model_id,
                    api_key_env: env_var.clone(),
                    base_url: None,
                    ..Default::default()
                };
                let mut guard = state
                    .kernel
                    .default_model_override_ref()
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                *guard = Some(new_dm);
            }
            true
        } else {
            false
        }
    } else if current_provider == name {
        // User is saving a key for the CURRENT default provider. The env var is
        // already set (set_var above), but we must ensure default_model_override
        // has the correct api_key_env so resolve_driver reads the right variable.
        let needs_update = {
            let guard = state
                .kernel
                .default_model_override_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(dm) => dm.api_key_env != env_var,
                None => state.kernel.config_ref().default_model.api_key_env != env_var,
            }
        };
        if needs_update {
            let mut guard = state
                .kernel
                .default_model_override_ref()
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let base = guard
                .clone()
                .unwrap_or_else(|| state.kernel.config_ref().default_model.clone());
            *guard = Some(librefang_types::config::DefaultModelConfig {
                api_key_env: env_var.clone(),
                ..base
            });
        }
        false
    } else {
        false
    };

    // Reset log-once flag so future provider removal gets logged again
    state
        .kernel
        .provider_unconfigured_flag()
        .store(false, std::sync::atomic::Ordering::Relaxed);

    // Trigger all active hands so they resume immediately
    state.kernel.trigger_all_hands();

    // If default provider switched, update registry entries for agents that were
    // using the old default so they immediately pick up the new provider/model.
    if switched {
        let new_dm = {
            let guard = state
                .kernel
                .default_model_override_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            guard
                .clone()
                .unwrap_or_else(|| state.kernel.config_ref().default_model.clone())
        };
        state
            .kernel
            .sync_default_model_agents(&current_provider, &new_dm);
    }

    let mut resp = serde_json::json!({"status": "saved", "provider": name});
    if switched {
        resp["switched_default"] = serde_json::json!(true);
        resp["message"] = serde_json::json!(format!(
            "API key saved and default provider switched to '{}'.",
            name
        ));
    }

    (StatusCode::OK, Json(resp))
}

/// DELETE /api/providers/{name}/key — Remove an API key for a provider.
#[utoipa::path(delete, path = "/api/providers/{name}/key", tag = "models", params(("name" = String, Path, description = "Provider name")), responses((status = 200, description = "API key deleted")))]
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .filter(|env| !env.trim().is_empty())
            .unwrap_or_else(|| {
                // Custom/unknown provider — derive env var from convention
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    if env_var.is_empty() {
        return ApiErrorResponse::bad_request("Provider does not require an API key")
            .into_json_tuple();
    }

    // Remove from secrets.env
    let secrets_path = state.kernel.home_dir().join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return ApiErrorResponse::internal(format!("Failed to update secrets.env: {e}"))
            .into_json_tuple();
    }

    // Remove from process environment
    std::env::remove_var(&env_var);

    // Suppress fallback/CLI detection for this provider and refresh auth
    {
        let mut catalog = state
            .kernel
            .model_catalog_ref()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.suppress_provider(&name);
        catalog.save_suppressed(
            &state
                .kernel
                .home_dir()
                .join("data")
                .join("suppressed_providers.json"),
        );
        catalog.detect_auth();
    }

    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

/// POST /api/providers/{name}/test — Test a provider's connectivity.
#[utoipa::path(post, path = "/api/providers/{name}/test", tag = "models", params(("name" = String, Path, description = "Provider name")), responses((status = 200, description = "Provider test result", body = crate::types::JsonObject)))]
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (env_var, base_url, key_required) = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => (p.api_key_env.clone(), p.base_url.clone(), p.key_required),
            None => {
                return ApiErrorResponse::not_found(format!("Unknown provider '{}'", name))
                    .into_json_tuple();
            }
        }
    };

    // ── CLI-based providers (no HTTP base URL) ──
    // Only treat as CLI provider if key is not required (true CLI providers
    // like claude-code, gemini-cli). Providers with key_required but empty
    // base_url are API providers missing configuration (e.g. OpenRouter proxied).
    if base_url.is_empty() && !key_required {
        let cli_start = Instant::now();
        let cli_ok = librefang_runtime::drivers::cli_provider_available(name.as_str());
        let cli_latency = cli_start.elapsed().as_millis();
        state.provider_test_cache.insert(
            name.clone(),
            (
                Instant::now(),
                cli_latency,
                chrono::Utc::now().to_rfc3339(),
                cli_ok,
            ),
        );
        return if cli_ok {
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","provider":name,"latency_ms":cli_latency})),
            )
        } else {
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status":"error","provider":name,"error":"CLI not found in PATH"}),
                ),
            )
        };
    }

    // ── Local providers (Ollama / vLLM / LM Studio / lemonade) ──
    // Delegate to the kernel's shared probe helper so the on-demand test
    // updates `auth_status` in the catalog (NotRequired on success,
    // LocalOffline on failure). Before this, the endpoint only refreshed an
    // in-memory cache — users could start Ollama after LibreFang booted and
    // the dashboard would stay stuck on `local_offline` forever.
    if librefang_runtime::provider_health::is_local_provider(&name) {
        let result = state
            .kernel
            .probe_local_provider(
                &name, &base_url, false, // user-triggered test — don't escalate to warn!
            )
            .await;
        let latency = result.latency_ms as u128;
        state.provider_test_cache.insert(
            name.clone(),
            (
                Instant::now(),
                latency,
                chrono::Utc::now().to_rfc3339(),
                result.reachable,
            ),
        );
        return if result.reachable {
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "provider": name,
                    "latency_ms": latency,
                    "discovered_models": result.discovered_models.len(),
                })),
            )
        } else {
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "error",
                    "provider": name,
                    "latency_ms": latency,
                    "error": result.error.unwrap_or_else(|| "unreachable".to_string()),
                })),
            )
        };
    }

    // API providers with no base_url configured cannot be tested.
    if base_url.is_empty() {
        return ApiErrorResponse::bad_request("Provider base URL not configured").into_json_tuple();
    }

    // Treat empty-string env vars the same as missing — an env var set to ""
    // (e.g. `DEEPSEEK_API_KEY=` in secrets.env) should not bypass the guard.
    let api_key = std::env::var(&env_var)
        .ok()
        .filter(|k| !k.trim().is_empty());
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return ApiErrorResponse::bad_request("Provider API key not configured").into_json_tuple();
    }

    let api_key_val = api_key.unwrap_or_default();
    // Reuse the shared probe HTTP client instead of building a fresh
    // `reqwest::Client` per request. Each rebuild paid a TLS-config init +
    // root-cert-chain load (~50–100 ms) on top of the actual handshake,
    // and that cost was being **counted as provider latency** below — so
    // a single `byteplus_coding 230 ms` round-trip surfaced on the
    // dashboard as `~500 ms` purely from the rebuilt client. Sharing the
    // pool also lets the second click reuse the warm TLS session.
    let client = librefang_runtime::provider_health::probe_client();
    let start = std::time::Instant::now();

    // ── Bedrock: AWS Signature auth — can't test with simple HTTP ──
    if name == "bedrock" || name == "aws-bedrock" {
        state.provider_test_cache.insert(
            name.clone(),
            (Instant::now(), 0, chrono::Utc::now().to_rfc3339(), true),
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "provider": name,
                "latency_ms": 0,
                "note": "AWS Bedrock uses IAM auth; key presence verified"
            })),
        );
    }

    // ── Provider-specific test URL ──
    // Anthropic-format providers (anthropic + byteplus_coding +
    // volcengine_coding + …) all probe via `/v1/models` with x-api-key
    // headers. Look up the registered ApiFormat instead of duplicating
    // the registry's name list here, so future Anthropic-protocol
    // providers don't need a parallel edit in this file.
    let api_format = librefang_llm_drivers::drivers::provider_api_format(&name);
    let is_anthropic_shape = matches!(
        api_format,
        Some(librefang_llm_drivers::drivers::ApiFormat::Anthropic)
    );
    let test_url_str = if is_anthropic_shape {
        format!("{}/v1/models", base_url.trim_end_matches('/'))
    } else {
        match name.as_str() {
            "gemini" | "google" => format!(
                "{}/v1beta/models?key={}",
                base_url.trim_end_matches('/'),
                api_key_val
            ),
            "chatgpt" => format!("{}/me", base_url.trim_end_matches('/')),
            "github-copilot" => format!("{}/models", base_url.trim_end_matches('/')),
            "elevenlabs" => format!("{}/user", base_url.trim_end_matches('/')),
            _ => format!("{}/models", base_url.trim_end_matches('/')),
        }
    };

    let mut req = client.get(&test_url_str);
    if is_anthropic_shape {
        req = req
            .header("x-api-key", &api_key_val)
            .header("anthropic-version", "2023-06-01");
    } else {
        match name.as_str() {
            "gemini" | "google" => {
                // Key is in query param, no header needed
            }
            "github-copilot" => {
                req = req.header("Authorization", format!("token {}", api_key_val));
            }
            "elevenlabs" => {
                req = req.header("xi-api-key", &api_key_val);
            }
            _ => {
                if !api_key_val.is_empty() {
                    req = req.header("Authorization", format!("Bearer {}", api_key_val));
                }
            }
        }
    }

    let result = req.send().await;

    let status_code = match result {
        Ok(resp) => resp.status().as_u16(),
        Err(e) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "error",
                    "provider": name,
                    "error": format!("Connection failed: {e}"),
                })),
            );
        }
    };

    // Any HTTP response (even 400/404/500) means the service is reachable.
    // Only connection failures (handled above as Err) indicate unreachable.
    // Treat auth errors (401/403) specially — key is wrong.
    let latency_ms = start.elapsed().as_millis();

    // Cache test result so GET /api/providers can show latency for all providers.
    state.provider_test_cache.insert(
        name.clone(),
        (
            Instant::now(),
            latency_ms,
            chrono::Utc::now().to_rfc3339(),
            true,
        ),
    );

    // For Anthropic-protocol providers, 401/403/404 on /v1/models is a
    // **listing-endpoint-not-exposed** signal, not an auth failure. The
    // BytePlus and Volcengine "coding plan" tokens, for instance, work
    // fine for /v1/messages (the real chat path) but the same key gets
    // a 401 from /v1/models because that endpoint isn't part of the
    // coding-plan scope. Reporting "Authentication failed" makes the
    // dashboard show a red "broken provider" tile when the key is in
    // fact valid for actual inference. Treat the same status codes that
    // the background `probe_provider` already treats as `Ok` here too —
    // both paths now converge on the same Anthropic-shape semantics.
    if !is_anthropic_shape && (status_code == 401 || status_code == 403) {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Authentication failed (HTTP {})", status_code),
            })),
        )
    } else {
        // Any other HTTP response (200, 400, 404, 429, 500, etc.) means
        // the service is reachable. Report success with the status code.
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "provider": name,
                "latency_ms": latency_ms,
            })),
        )
    }
}

/// PUT /api/providers/{name}/url — Set a custom base URL for a provider.
#[utoipa::path(put, path = "/api/providers/{name}/url", tag = "models", params(("name" = String, Path, description = "Provider name")), request_body = crate::types::JsonObject, responses((status = 200, description = "Provider URL set", body = crate::types::JsonObject)))]
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Accept any provider name — custom providers are supported via OpenAI-compatible format.
    let base_url_raw = match body["base_url"].as_str() {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'base_url' field")
                .into_json_tuple();
        }
    };

    // Validate URL scheme
    if !base_url_raw.starts_with("http://") && !base_url_raw.starts_with("https://") {
        return ApiErrorResponse::bad_request("base_url must start with http:// or https://")
            .into_json_tuple();
    }

    // Normalize for the common Ollama / vLLM / LM Studio mistake: users
    // paste `http://host:port` (no path) and the OpenAI driver then hits
    // `/chat/completions` instead of `/v1/chat/completions`, getting a 404.
    // If the user gave us a host-only URL (path is empty or just "/"),
    // append `/v1` so OpenAI-compatible endpoints work out of the box.
    // Custom paths (`/api/openai`, `/openai/v1`, etc.) are left alone.
    // Issue #3138.
    let base_url = normalize_base_url(&base_url_raw);

    // Optional proxy_url in same request
    let proxy_url = body["proxy_url"].as_str().map(|s| s.trim().to_string());
    if let Some(ref pu) = proxy_url {
        if !pu.is_empty()
            && !pu.starts_with("http://")
            && !pu.starts_with("https://")
            && !pu.starts_with("socks5://")
            && !pu.starts_with("socks5h://")
        {
            return ApiErrorResponse::bad_request(
                "proxy_url must start with http://, https://, socks5://, or socks5h://",
            )
            .into_json_tuple();
        }
    }

    // Update catalog in memory
    {
        let mut catalog = state
            .kernel
            .model_catalog_ref()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
        if let Some(ref pu) = proxy_url {
            catalog.set_provider_proxy_url(&name, pu);
        }
    }

    // Persist to config.toml [provider_urls] section
    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return ApiErrorResponse::internal(format!("Failed to save config: {e}")).into_json_tuple();
    }
    if let Some(ref pu) = proxy_url {
        if let Err(e) = upsert_provider_proxy_url(&config_path, &name, pu) {
            tracing::warn!("Failed to persist proxy_url: {e}");
        }
    }

    // Probe reachability at the new URL. Forward the configured api_key so
    // reverse-proxy-fronted endpoints (Open WebUI, LiteLLM, etc.) accept
    // the listing request — without this, they return 401 even when the
    // backing model server is healthy.
    let probe_env_var = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .filter(|env| !env.trim().is_empty())
            .unwrap_or_else(|| format!("{}_API_KEY", name.to_uppercase().replace('-', "_")))
    };
    let probe_api_key = std::env::var(&probe_env_var)
        .ok()
        .filter(|v| !v.trim().is_empty());
    let probe = librefang_runtime::provider_health::probe_provider(
        &name,
        &base_url,
        probe_api_key.as_deref(),
    )
    .await;

    // Merge discovered models into catalog
    if !probe.discovered_models.is_empty() {
        if let Ok(mut catalog) = state.kernel.model_catalog_ref().write() {
            let info: Vec<_> = if probe.discovered_model_info.is_empty() {
                probe
                    .discovered_models
                    .iter()
                    .map(
                        |n| librefang_runtime::provider_health::DiscoveredModelInfo {
                            name: n.clone(),
                            parameter_size: None,
                            quantization_level: None,
                            family: None,
                            families: None,
                            size: None,
                            capabilities: vec![],
                        },
                    )
                    .collect()
            } else {
                probe.discovered_model_info.clone()
            };
            catalog.merge_discovered_models(&name, &info);
        }
    }

    let mut resp = serde_json::json!({
        "status": "saved",
        "provider": name,
        "base_url": base_url,
        "reachable": probe.reachable,
        "latency_ms": probe.latency_ms,
    });
    if !probe.discovered_models.is_empty() {
        resp["discovered_models"] = serde_json::json!(probe.discovered_models);
    }
    if !probe.discovered_model_info.is_empty() {
        resp["discovered_model_info"] = serde_json::json!(probe.discovered_model_info);
    }

    (StatusCode::OK, Json(resp))
}

/// POST /api/providers/{name}/default — Set a provider as the default model provider.
///
/// Looks up the best default model for the given provider and updates both
/// the in-memory override and config.toml so it persists across restarts.
#[utoipa::path(
    post,
    path = "/api/providers/{name}/default",
    tag = "models",
    params(("name" = String, Path, description = "Provider identifier")),
    responses(
        (status = 200, description = "Default provider updated", body = crate::types::JsonObject),
        (status = 400, description = "No model found for provider"),
        (status = 404, description = "Provider not found")
    )
)]
pub async fn set_default_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Option<axum::Json<serde_json::Value>>,
) -> impl IntoResponse {
    // Accept optional {"model": "model-id"} body to override the auto-selected model.
    // This is needed for providers like ollama where models are dynamic and may
    // not be in the static catalog.
    let user_model = body
        .as_ref()
        .and_then(|b| b.get("model"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(String::from);

    // Verify the provider exists in the catalog
    let (default_model, env_var) = {
        let catalog = state
            .kernel
            .model_catalog_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let provider = match catalog.get_provider(&name) {
            Some(p) => p.clone(),
            None => {
                return ApiErrorResponse::not_found(format!("Provider '{}' not found", name))
                    .into_json_tuple();
            }
        };
        let model_id = user_model.or_else(|| catalog.default_model_for_provider(&name));
        (model_id, provider.api_key_env.clone())
    };

    let model_id = match default_model {
        Some(id) => id,
        None => {
            return ApiErrorResponse::bad_request(format!(
                "No models found for provider '{}'. Specify a model in the request body: {{\"model\": \"model-name\"}}",
                name
            ))
            .into_json_tuple();
        }
    };

    // Update config.toml to persist the switch
    let config_path = state.kernel.home_dir().join("config.toml");
    let persisted = match persist_default_model(&config_path, &name, &model_id, &env_var) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("Failed to persist default_model to config.toml: {e}");
            false
        }
    };

    // Read old default before updating, so sync_default_model_agents knows what to migrate
    let old_provider = {
        let guard = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => dm.provider.clone(),
            None => state.kernel.config_ref().default_model.provider.clone(),
        }
    };

    // Hot-update the in-memory default model override
    let new_dm = librefang_types::config::DefaultModelConfig {
        provider: name.clone(),
        model: model_id.clone(),
        api_key_env: env_var.clone(),
        base_url: None,
        ..Default::default()
    };
    {
        let mut guard = state
            .kernel
            .default_model_override_ref()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(new_dm.clone());
    }

    // Update registry entries for agents that were tracking the old default
    state
        .kernel
        .sync_default_model_agents(&old_provider, &new_dm);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "provider": name,
            "model": model_id,
            "api_key_env": env_var,
            "persisted": persisted,
        })),
    )
}

/// Safely persist the `[default_model]` section into config.toml using proper
/// TOML serialization (avoids format-string injection).
fn persist_default_model(
    config_path: &std::path::Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dm_table = toml::map::Map::new();
    dm_table.insert(
        "provider".to_string(),
        toml::Value::String(provider.to_string()),
    );
    dm_table.insert("model".to_string(), toml::Value::String(model.to_string()));
    dm_table.insert(
        "api_key_env".to_string(),
        toml::Value::String(api_key_env.to_string()),
    );

    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };
    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;
    root.insert("default_model".to_string(), toml::Value::Table(dm_table));
    let toml_str = toml::to_string_pretty(&doc)?;
    crate::atomic_write(config_path, toml_str.as_bytes())?;
    Ok(())
}

/// Normalize a user-supplied provider base URL.
///
/// `http://host:port` (no path) and `http://host:port/` are rewritten to
/// `http://host:port/v1` because every OpenAI-compatible local server we
/// support (Ollama, vLLM, LM Studio, LlamaSwap, llama-server, etc.) serves
/// its chat-completions endpoint at `/v1/chat/completions`. Without the
/// normalisation the OpenAI driver produces `/chat/completions` and the
/// server returns HTTP 404 — see issue #3138.
///
/// Custom paths (`/api/openai`, `/openai/v1`, `/router/some/path`) are left
/// untouched: if a user explicitly typed a path we trust it.
fn normalize_base_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/').to_string();
    // After scheme there must be host[:port][/path…]. Find the first '/'
    // after the scheme separator to know whether a path was supplied.
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    let has_path = after_scheme.find('/').is_some();
    if has_path {
        // User supplied an explicit path — respect it.
        trimmed
    } else {
        // Bare host[:port] — assume OpenAI-compatible default.
        format!("{trimmed}/v1")
    }
}

#[cfg(test)]
#[test]
fn normalize_base_url_appends_v1_for_bare_host() {
    assert_eq!(
        normalize_base_url("http://192.168.1.10:11434"),
        "http://192.168.1.10:11434/v1"
    );
    assert_eq!(
        normalize_base_url("http://192.168.1.10:11434/"),
        "http://192.168.1.10:11434/v1"
    );
    assert_eq!(
        normalize_base_url("http://localhost:8000"),
        "http://localhost:8000/v1"
    );
}

#[cfg(test)]
#[test]
fn normalize_base_url_preserves_explicit_path() {
    assert_eq!(
        normalize_base_url("http://192.168.1.10:11434/v1"),
        "http://192.168.1.10:11434/v1"
    );
    assert_eq!(
        normalize_base_url("http://192.168.1.10:11434/v1/"),
        "http://192.168.1.10:11434/v1"
    );
    assert_eq!(
        normalize_base_url("https://api.openai.com/v1"),
        "https://api.openai.com/v1"
    );
    assert_eq!(
        normalize_base_url("https://example.com/api/openai"),
        "https://example.com/api/openai"
    );
}

#[cfg(test)]
#[test]
fn normalize_base_url_trims_whitespace() {
    assert_eq!(
        normalize_base_url("  http://localhost:11434  "),
        "http://localhost:11434/v1"
    );
}

/// Upsert a provider URL in the `[provider_urls]` section of config.toml.
fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if config_path.file_name().and_then(|n| n.to_str()) != Some("config.toml") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid config path '{}'", config_path.display()),
        )
        .into());
    }
    // Block path-traversal (`..`) but allow Windows drive-letter prefixes
    if config_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsafe config path '{}'", config_path.display()),
        )
        .into());
    }

    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_urls") {
        root.insert(
            "provider_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let urls_table = root
        .get_mut("provider_urls")
        .and_then(|v| v.as_table_mut())
        .ok_or("provider_urls is not a table")?;

    urls_table.insert(provider.to_string(), toml::Value::String(url.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let toml_str = toml::to_string_pretty(&doc)?;
    crate::atomic_write(config_path, toml_str.as_bytes())?;
    Ok(())
}

/// Persist a per-provider proxy URL to `[provider_proxy_urls]` in config.toml.
fn upsert_provider_proxy_url(
    config_path: &std::path::Path,
    provider: &str,
    proxy_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_proxy_urls") {
        root.insert(
            "provider_proxy_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let table = root
        .get_mut("provider_proxy_urls")
        .and_then(|v| v.as_table_mut())
        .ok_or("provider_proxy_urls is not a table")?;

    if proxy_url.is_empty() {
        table.remove(provider);
    } else {
        table.insert(
            provider.to_string(),
            toml::Value::String(proxy_url.to_string()),
        );
    }

    let toml_str = toml::to_string_pretty(&doc)?;
    crate::atomic_write(config_path, toml_str.as_bytes())?;
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════
// GitHub Copilot OAuth Device Flow
// ══════════════════════════════════════════════════════════════════════

/// State for an in-progress device flow.
struct CopilotFlowState {
    device_code: String,
    interval: u64,
    expires_at: Instant,
}

/// Active device flows, keyed by poll_id. Auto-expire after the flow's TTL.
static COPILOT_FLOWS: LazyLock<DashMap<String, CopilotFlowState>> = LazyLock::new(DashMap::new);

/// POST /api/providers/github-copilot/oauth/start
///
/// Initiates a GitHub device flow for Copilot authentication.
/// Returns a user code and verification URI that the user visits in their browser.
#[utoipa::path(post, path = "/api/providers/github-copilot/oauth/start", tag = "models", responses((status = 200, description = "OAuth flow started", body = crate::types::JsonObject)))]
pub async fn copilot_oauth_start() -> impl IntoResponse {
    // Clean up expired flows first
    COPILOT_FLOWS.retain(|_, state| state.expires_at > Instant::now());

    match librefang_runtime::copilot_oauth::start_device_flow().await {
        Ok(resp) => {
            let poll_id = uuid::Uuid::new_v4().to_string();

            COPILOT_FLOWS.insert(
                poll_id.clone(),
                CopilotFlowState {
                    device_code: resp.device_code,
                    interval: resp.interval,
                    expires_at: Instant::now() + std::time::Duration::from_secs(resp.expires_in),
                },
            );

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "user_code": resp.user_code,
                    "verification_uri": resp.verification_uri,
                    "poll_id": poll_id,
                    "expires_in": resp.expires_in,
                    "interval": resp.interval,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ),
    }
}

/// GET /api/providers/github-copilot/oauth/poll/{poll_id}
///
/// Poll the status of a GitHub device flow.
/// Returns `pending`, `complete`, `expired`, `denied`, or `error`.
/// On `complete`, saves the token to secrets.env and sets GITHUB_TOKEN.
#[utoipa::path(get, path = "/api/providers/github-copilot/oauth/poll/{poll_id}", tag = "models", params(("poll_id" = String, Path, description = "Poll ID")), responses((status = 200, description = "OAuth poll result", body = crate::types::JsonObject)))]
pub async fn copilot_oauth_poll(
    State(state): State<Arc<AppState>>,
    Path(poll_id): Path<String>,
) -> impl IntoResponse {
    let flow = match COPILOT_FLOWS.get(&poll_id) {
        Some(f) => f,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "not_found", "error": "Unknown poll_id"})),
            )
        }
    };

    if flow.expires_at <= Instant::now() {
        drop(flow);
        COPILOT_FLOWS.remove(&poll_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired"})),
        );
    }

    let device_code = flow.device_code.clone();
    drop(flow);

    match librefang_runtime::copilot_oauth::poll_device_flow(&device_code).await {
        librefang_runtime::copilot_oauth::DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "pending"})),
        ),
        librefang_runtime::copilot_oauth::DeviceFlowStatus::Complete { access_token } => {
            // Save to secrets.env
            let secrets_path = state.kernel.home_dir().join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, "GITHUB_TOKEN", &access_token) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"status": "error", "error": format!("Failed to save token: {e}")}),
                    ),
                );
            }

            // Set in current process.
            // `std::env::set_var` is not thread-safe inside async; push to a
            // blocking thread to avoid UB in the multithreaded tokio runtime.
            {
                let token = access_token.to_string();
                let _ = tokio::task::spawn_blocking(move || {
                    // SAFETY: single mutation on a dedicated blocking thread.
                    unsafe { std::env::set_var("GITHUB_TOKEN", &token) };
                })
                .await;
            }

            // Refresh auth detection
            state
                .kernel
                .model_catalog_ref()
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .detect_auth();

            // Clean up flow state
            COPILOT_FLOWS.remove(&poll_id);

            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "complete"})),
            )
        }
        librefang_runtime::copilot_oauth::DeviceFlowStatus::SlowDown { new_interval } => {
            // Update interval
            if let Some(mut f) = COPILOT_FLOWS.get_mut(&poll_id) {
                f.interval = new_interval;
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending", "interval": new_interval})),
            )
        }
        librefang_runtime::copilot_oauth::DeviceFlowStatus::Expired => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "expired"})),
            )
        }
        librefang_runtime::copilot_oauth::DeviceFlowStatus::AccessDenied => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "denied"})),
            )
        }
        librefang_runtime::copilot_oauth::DeviceFlowStatus::Error(e) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Catalog sync endpoints
// ---------------------------------------------------------------------------

/// POST /api/catalog/update — Sync model catalog from the remote repository.
///
/// Downloads the latest catalog TOML files from GitHub and caches them locally.
/// After syncing, the kernel's in-memory catalog is refreshed.
#[utoipa::path(post, path = "/api/catalog/update", tag = "models", responses((status = 200, description = "Catalog updated", body = crate::types::JsonObject)))]
pub async fn catalog_update(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let mirror = &cfg.registry.registry_mirror;
    match librefang_runtime::catalog_sync::sync_catalog_to(state.kernel.home_dir(), mirror).await {
        Ok(result) => {
            // Refresh the in-memory catalog so the new models are available immediately
            {
                let mut catalog = state
                    .kernel
                    .model_catalog_ref()
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                catalog.load_cached_catalog_for(state.kernel.home_dir());
                let cfg = state.kernel.config_ref();
                if !cfg.provider_regions.is_empty() {
                    let region_urls = catalog.resolve_region_urls(&cfg.provider_regions);
                    if !region_urls.is_empty() {
                        catalog.apply_url_overrides(&region_urls);
                    }
                }
                if !cfg.provider_urls.is_empty() {
                    catalog.apply_url_overrides(&cfg.provider_urls);
                }
                catalog.detect_auth();
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "files_downloaded": result.files_downloaded,
                    "models_count": result.models_count,
                    "timestamp": result.timestamp,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": e,
            })),
        )
            .into_response(),
    }
}

/// GET /api/catalog/status — Check last catalog sync time.
#[utoipa::path(get, path = "/api/catalog/status", tag = "models", responses((status = 200, description = "Catalog sync status", body = crate::types::JsonObject)))]
pub async fn catalog_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let last_sync = librefang_runtime::catalog_sync::last_sync_time_for(state.kernel.home_dir());
    Json(serde_json::json!({
        "last_sync": last_sync,
    }))
}

/// GET /api/providers/ollama/detect — Probe localhost for Ollama availability
pub async fn detect_ollama() -> impl IntoResponse {
    let client = match librefang_runtime::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return Json(serde_json::json!({ "available": false, "models": [] }));
        }
    };

    // Use 127.0.0.1 instead of localhost: on dual-stack hosts (macOS)
    // localhost resolves to ::1 first and Ollama binds IPv4 only, causing
    // probes to fail without reliable IPv4 fallback.
    match client.get("http://127.0.0.1:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_else(|e| {
                tracing::warn!("Ollama responded but JSON parse failed: {e}");
                serde_json::Value::Null
            });
            let models: Vec<String> = body["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Json(serde_json::json!({ "available": true, "models": models }))
        }
        _ => Json(serde_json::json!({ "available": false, "models": [] })),
    }
}

#[cfg(test)]
mod tests {
    use crate::routes::system::{get_profile, list_profiles};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn profile_router() -> Router {
        Router::new()
            .route("/api/profiles", get(list_profiles))
            .route("/api/profiles/{name}", get(get_profile))
    }

    #[tokio::test]
    async fn test_get_profile_found() {
        let app = profile_router();

        for name in &[
            "minimal",
            "coding",
            "research",
            "messaging",
            "automation",
            "full",
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/profiles/{name}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "profile '{name}' should exist"
            );

            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["name"], *name);
            assert!(
                json["tools"].is_array(),
                "tools should be an array for '{name}'"
            );
        }
    }

    #[tokio::test]
    async fn test_get_profile_not_found() {
        let app = profile_router();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/profiles/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn test_provider_json_includes_media_capabilities() {
        let provider = librefang_types::model_catalog::ProviderInfo {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            media_capabilities: vec!["image_generation".into(), "text_to_speech".into()],
            ..Default::default()
        };
        let json = serde_json::json!({
            "id": provider.id,
            "display_name": provider.display_name,
            "media_capabilities": provider.media_capabilities,
        });
        let caps = json["media_capabilities"].as_array().unwrap();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0], "image_generation");
        assert_eq!(caps[1], "text_to_speech");
    }

    #[tokio::test]
    async fn test_list_profiles_returns_all() {
        let app = profile_router();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/profiles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 6);
    }
}
