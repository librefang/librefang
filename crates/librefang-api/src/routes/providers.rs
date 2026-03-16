//! Model catalog, provider management, and Copilot OAuth handlers.

use super::network::remove_toml_section;
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

#[utoipa::path(
    get,
    path = "/api/models",
    tag = "models",
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
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

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
                    if p.auth_status == librefang_types::model_catalog::AuthStatus::Missing {
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
                .map(|p| p.auth_status != librefang_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == librefang_types::model_catalog::ModelTier::Custom);
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": m.provider,
                "tier": m.tier,
                "context_window": m.context_window,
                "max_output_tokens": m.max_output_tokens,
                "input_cost_per_m": m.input_cost_per_m,
                "output_cost_per_m": m.output_cost_per_m,
                "supports_tools": m.supports_tools,
                "supports_vision": m.supports_vision,
                "supports_streaming": m.supports_streaming,
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

#[utoipa::path(get, path = "/api/models/aliases", tag = "models", responses((status = 200, description = "List model aliases", body = serde_json::Value)))]
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog
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
#[utoipa::path(post, path = "/api/models/aliases", tag = "models", request_body = serde_json::Value, responses((status = 200, description = "Alias created", body = serde_json::Value)))]
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: alias"})),
        );
    }
    if model_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: model_id"})),
        );
    }

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_alias(&alias, &model_id) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Alias '{}' already exists", alias)})),
        );
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
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_alias(&alias) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Alias '{}' not found", alias)})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
}

#[utoipa::path(get, path = "/api/models/{id}", tag = "models", params(("id" = String, Path, description = "Model ID")), responses((status = 200, description = "Model details", body = serde_json::Value)))]
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(m) => {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != librefang_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == librefang_types::model_catalog::ModelTier::Custom);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": m.id,
                    "display_name": m.display_name,
                    "provider": m.provider,
                    "tier": m.tier,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "input_cost_per_m": m.input_cost_per_m,
                    "output_cost_per_m": m.output_cost_per_m,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "supports_streaming": m.supports_streaming,
                    "aliases": m.aliases,
                    "available": available,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Model '{}' not found", id)})),
        ),
    }
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
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    // Collect local providers that need probing
    let local_providers: Vec<(usize, String, String)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            librefang_runtime::provider_health::is_local_provider(&p.id) && !p.base_url.is_empty()
        })
        .map(|(i, p)| (i, p.id.clone(), p.base_url.clone()))
        .collect();

    // Fire all probes concurrently (cached results return instantly)
    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url)| {
            librefang_runtime::provider_health::probe_provider_cached(id, url, cache)
        })
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    // Index probe results by provider list position for O(1) lookup
    let mut probe_map: HashMap<usize, librefang_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((idx, _, _), result) in local_providers.iter().zip(probe_results.into_iter()) {
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
        });

        // For local providers, attach the probe result
        if let Some(probe) = probe_map.remove(&i) {
            entry["is_local"] = serde_json::json!(true);
            entry["reachable"] = serde_json::json!(probe.reachable);
            entry["latency_ms"] = serde_json::json!(probe.latency_ms);
            if !probe.discovered_models.is_empty() {
                entry["discovered_models"] = serde_json::json!(probe.discovered_models);
                // Merge discovered models into the catalog so agents can use them
                if let Ok(mut catalog) = state.kernel.model_catalog.write() {
                    catalog.merge_discovered_models(&p.id, &probe.discovered_models);
                }
            }
            if let Some(err) = &probe.error {
                entry["error"] = serde_json::json!(err);
            }
        } else if librefang_runtime::provider_health::is_local_provider(&p.id) {
            // Local HTTP provider with no probe result yet — still label it local.
            entry["is_local"] = serde_json::json!(true);
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

/// POST /api/models/custom — Add a custom model to the catalog.
///
/// Persists to `~/.librefang/custom_models.json` and makes the model immediately
/// available in the catalog.
#[utoipa::path(post, path = "/api/models/custom", tag = "models", request_body = serde_json::Value, responses((status = 200, description = "Custom model added", body = serde_json::Value)))]
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openrouter")
        .to_string();
    let context_window = body
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: id"})),
        );
    }

    let display = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let entry = librefang_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: librefang_types::model_catalog::ModelTier::Custom,
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
        aliases: vec![],
    };

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"error": format!("Model '{}' already exists for provider '{}'", id, provider)}),
            ),
        );
    }

    // Persist to disk
    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
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
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_custom_model(&model_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Custom model '{}' not found", model_id)})),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
}

// ── A2A (Agent-to-Agent) Protocol Endpoints ─────────────────────────

#[utoipa::path(post, path = "/api/providers/{name}/key", tag = "models", params(("name" = String, Path, description = "Provider name")), request_body = serde_json::Value, responses((status = 200, description = "API key set", body = serde_json::Value)))]
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'key' field"})),
            );
        }
    };

    // Look up env var from catalog; for unknown/custom providers derive one.
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom provider — derive env var: MY_PROVIDER → MY_PROVIDER_API_KEY
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    // Write to secrets.env file
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_var, &key) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write secrets.env: {e}")})),
        );
    }

    // Set env var in current process so detect_auth picks it up
    std::env::set_var(&env_var, &key);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

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
            .default_model_override
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => (dm.provider.clone(), dm.api_key_env.clone()),
            None => (
                state.kernel.config.default_model.provider.clone(),
                state.kernel.config.default_model.api_key_env.clone(),
            ),
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
                .model_catalog
                .read()
                .unwrap_or_else(|e| e.into_inner());
            catalog.default_model_for_provider(&name)
        };
        if let Some(model_id) = default_model {
            // Update config.toml to persist the switch
            let config_path = state.kernel.config.home_dir.join("config.toml");
            let update_toml = format!(
                "\n[default_model]\nprovider = \"{}\"\nmodel = \"{}\"\napi_key_env = \"{}\"\n",
                name, model_id, env_var
            );
            if let Ok(existing) = std::fs::read_to_string(&config_path) {
                // Remove existing [default_model] section if present, then append
                let cleaned = remove_toml_section(&existing, "default_model");
                if let Err(e) =
                    std::fs::write(&config_path, format!("{}\n{}", cleaned.trim(), update_toml))
                {
                    tracing::warn!("Failed to write config file: {e}");
                }
            } else if let Err(e) = std::fs::write(&config_path, update_toml) {
                tracing::warn!("Failed to write config file: {e}");
            }

            // Hot-update the in-memory default model override so resolve_driver()
            // immediately creates drivers for the new provider — no restart needed.
            {
                let new_dm = librefang_types::config::DefaultModelConfig {
                    provider: name.clone(),
                    model: model_id,
                    api_key_env: env_var.clone(),
                    base_url: None,
                };
                let mut guard = state
                    .kernel
                    .default_model_override
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
                .default_model_override
                .read()
                .unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(dm) => dm.api_key_env != env_var,
                None => state.kernel.config.default_model.api_key_env != env_var,
            }
        };
        if needs_update {
            let mut guard = state
                .kernel
                .default_model_override
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let base = guard
                .clone()
                .unwrap_or_else(|| state.kernel.config.default_model.clone());
            *guard = Some(librefang_types::config::DefaultModelConfig {
                api_key_env: env_var.clone(),
                ..base
            });
        }
        false
    } else {
        false
    };

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
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom/unknown provider — derive env var from convention
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    if env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider does not require an API key"})),
        );
    }

    // Remove from secrets.env
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update secrets.env: {e}")})),
        );
    }

    // Remove from process environment
    std::env::remove_var(&env_var);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "provider": name})),
    )
}

/// POST /api/providers/{name}/test — Test a provider's connectivity.
#[utoipa::path(post, path = "/api/providers/{name}/test", tag = "models", params(("name" = String, Path, description = "Provider name")), responses((status = 200, description = "Provider test result", body = serde_json::Value)))]
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (env_var, base_url, key_required) = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => (p.api_key_env.clone(), p.base_url.clone(), p.key_required),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
                );
            }
        }
    };

    let api_key = std::env::var(&env_var).ok();
    // Only require API key for providers that need one (skip local providers like ollama/vllm/lmstudio)
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider API key not configured"})),
        );
    }

    // ── CLI-based providers (no HTTP base URL) ──
    if base_url.is_empty() {
        let cli_ok = match name.as_str() {
            "claude-code" => librefang_runtime::drivers::claude_code::claude_code_available(),
            _ => false,
        };
        return if cli_ok {
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","provider":name,"latency_ms":0})),
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

    let start = std::time::Instant::now();
    let api_key_val = api_key.unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // ── Bedrock: AWS Signature auth — can't test with simple HTTP ──
    if name == "bedrock" || name == "aws-bedrock" {
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
    let test_url_str = match name.as_str() {
        "anthropic" => format!("{}/v1/models", base_url.trim_end_matches('/')),
        "gemini" | "google" => format!(
            "{}/v1beta/models?key={}",
            base_url.trim_end_matches('/'),
            api_key_val
        ),
        "chatgpt" => format!("{}/me", base_url.trim_end_matches('/')),
        "github-copilot" => format!("{}/models", base_url.trim_end_matches('/')),
        _ => format!("{}/models", base_url.trim_end_matches('/')),
    };

    let mut req = client.get(&test_url_str);
    match name.as_str() {
        "anthropic" => {
            req = req
                .header("x-api-key", &api_key_val)
                .header("anthropic-version", "2023-06-01");
        }
        "gemini" | "google" => {
            // Key is in query param, no header needed
        }
        "github-copilot" => {
            req = req.header("Authorization", format!("token {}", api_key_val));
        }
        _ => {
            if !api_key_val.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key_val));
            }
        }
    }

    let result = req.send().await;

    let (first_status, _first_body) = match result {
        Ok(resp) => {
            let sc = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            (sc, body)
        }
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

    // If /models returned 404, fall back to base URL (some providers don't expose /models).
    let status_code = if first_status == 404 {
        let mut fallback = client.get(&base_url);
        match name.as_str() {
            "anthropic" => {
                fallback = fallback
                    .header("x-api-key", &api_key_val)
                    .header("anthropic-version", "2023-06-01");
            }
            "gemini" | "google" => {}
            "github-copilot" => {
                fallback = fallback.header("Authorization", format!("token {}", api_key_val));
            }
            _ => {
                if !api_key_val.is_empty() {
                    fallback = fallback.header("Authorization", format!("Bearer {}", api_key_val));
                }
            }
        }
        match fallback.send().await {
            Ok(resp) => resp.status().as_u16(),
            Err(_) => first_status,
        }
    } else {
        first_status
    };

    let latency_ms = start.elapsed().as_millis();

    if status_code == 401 || status_code == 403 {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Authentication failed (HTTP {})", status_code),
            })),
        )
    } else if status_code >= 500 {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Server error (HTTP {})", status_code),
            })),
        )
    } else {
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
#[utoipa::path(put, path = "/api/providers/{name}/url", tag = "models", params(("name" = String, Path, description = "Provider name")), request_body = serde_json::Value, responses((status = 200, description = "Provider URL set", body = serde_json::Value)))]
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Accept any provider name — custom providers are supported via OpenAI-compatible format.
    let base_url = match body["base_url"].as_str() {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'base_url' field"})),
            );
        }
    };

    // Validate URL scheme
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base_url must start with http:// or https://"})),
        );
    }

    // Update catalog in memory
    {
        let mut catalog = state
            .kernel
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
    }

    // Persist to config.toml [provider_urls] section
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        );
    }

    // Probe reachability at the new URL
    let probe = librefang_runtime::provider_health::probe_provider(&name, &base_url).await;

    // Merge discovered models into catalog
    if !probe.discovered_models.is_empty() {
        if let Ok(mut catalog) = state.kernel.model_catalog.write() {
            catalog.merge_discovered_models(&name, &probe.discovered_models);
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

    (StatusCode::OK, Json(resp))
}

/// Upsert a provider URL in the `[provider_urls]` section of config.toml.
fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
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

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
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
#[utoipa::path(post, path = "/api/providers/github-copilot/oauth/start", tag = "models", responses((status = 200, description = "OAuth flow started", body = serde_json::Value)))]
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
#[utoipa::path(get, path = "/api/providers/github-copilot/oauth/poll/{poll_id}", tag = "models", params(("poll_id" = String, Path, description = "Poll ID")), responses((status = 200, description = "OAuth poll result", body = serde_json::Value)))]
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
            let secrets_path = state.kernel.config.home_dir.join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, "GITHUB_TOKEN", &access_token) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"status": "error", "error": format!("Failed to save token: {e}")}),
                    ),
                );
            }

            // Set in current process
            std::env::set_var("GITHUB_TOKEN", access_token.as_str());

            // Refresh auth detection
            state
                .kernel
                .model_catalog
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
#[utoipa::path(post, path = "/api/catalog/update", tag = "models", responses((status = 200, description = "Catalog updated", body = serde_json::Value)))]
pub async fn catalog_update(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match librefang_runtime::catalog_sync::sync_catalog().await {
        Ok(result) => {
            // Refresh the in-memory catalog so the new models are available immediately
            {
                let mut catalog = state
                    .kernel
                    .model_catalog
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                catalog.load_default_cached_catalog();
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
#[utoipa::path(get, path = "/api/catalog/status", tag = "models", responses((status = 200, description = "Catalog sync status", body = serde_json::Value)))]
pub async fn catalog_status() -> impl IntoResponse {
    let last_sync = librefang_runtime::catalog_sync::last_sync_time();
    Json(serde_json::json!({
        "last_sync": last_sync,
    }))
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
