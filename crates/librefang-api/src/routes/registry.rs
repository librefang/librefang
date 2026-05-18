//! Registry schema + content creation endpoints.
//!
//! Extracted from `system.rs` (issue #3749) — public paths and behavior
//! unchanged. Covers:
//! - `GET /api/registry/schema` — full machine-parseable registry schema
//! - `GET /api/registry/schema/{content_type}` — per-type schema
//! - `POST/PUT /api/registry/content/{content_type}` — create/update a
//!   registry TOML file (provider, agent, hand, mcp, skill, plugin), with
//!   provider-specific catalog refresh + secrets.env handling.

use super::skills::write_secret_env;
use super::AppState;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

/// Build the `/registry/...` sub-router.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/registry/schema", axum::routing::get(registry_schema))
        .route(
            "/registry/schema/{content_type}",
            axum::routing::get(registry_schema_by_type),
        )
        .route(
            "/registry/content/{content_type}",
            axum::routing::post(create_registry_content).put(update_registry_content),
        )
}

// ---------------------------------------------------------------------------
// Registry Schema
// ---------------------------------------------------------------------------

/// GET /api/registry/schema — Return the full registry schema for all content types.
async fn registry_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let home_dir = state.kernel.home_dir();
    match librefang_types::registry_schema::load_registry_schema(home_dir) {
        Some(schema) => match serde_json::to_value(&schema) {
            Ok(val) => Json(val).into_response(),
            Err(e) => ApiErrorResponse::internal(e.to_string())
                .into_json_tuple()
                .into_response(),
        },
        None => ApiErrorResponse::not_found(
            "Registry schema not found or not yet in machine-parseable format",
        )
        .into_json_tuple()
        .into_response(),
    }
}

/// GET /api/registry/schema/:content_type — Return schema for a specific content type.
async fn registry_schema_by_type(
    State(state): State<Arc<AppState>>,
    Path(content_type): Path<String>,
) -> impl IntoResponse {
    let home_dir = state.kernel.home_dir();
    match librefang_types::registry_schema::load_registry_schema(home_dir) {
        Some(schema) => match schema.content_types.get(&content_type) {
            Some(ct) => match serde_json::to_value(ct) {
                Ok(val) => Json(val).into_response(),
                Err(e) => ApiErrorResponse::internal(e.to_string())
                    .into_json_tuple()
                    .into_response(),
            },
            None => ApiErrorResponse::not_found(format!(
                "Content type '{content_type}' not found in registry schema"
            ))
            .into_json_tuple()
            .into_response(),
        },
        None => ApiErrorResponse::not_found("Registry schema not found")
            .into_json_tuple()
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Registry Content Creation
// ---------------------------------------------------------------------------

/// POST /api/registry/content/:content_type — Create or update a registry content file.
///
/// Accepts JSON form values, converts to TOML, and writes to the appropriate
/// directory under `~/.librefang/`.
///
/// Query parameters:
/// - `allow_overwrite=true` — allow overwriting an existing file (default: false).
///
/// For provider files, the in-memory model catalog is refreshed after the write
/// so new models / provider changes are available immediately without a restart.
async fn create_registry_content(
    State(state): State<Arc<AppState>>,
    Path(content_type): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let home_dir = state.kernel.home_dir();
    let allow_overwrite = params
        .get("allow_overwrite")
        .is_some_and(|v| v == "true" || v == "1");

    // Extract identifier (id or name) from the values.
    // Check top-level first, then look in nested sections (e.g. skill.name).
    let identifier = body.as_object().and_then(|m| {
        // Top-level id/name
        m.get("id")
            .or_else(|| m.get("name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                // Search one level deep in sections (e.g. {"skill": {"name": "..."}})
                m.values().find_map(|v| {
                    v.as_object().and_then(|sub| {
                        sub.get("id")
                            .or_else(|| sub.get("name"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    })
                })
            })
    });

    let identifier = match identifier {
        Some(id) => id,
        None => {
            return ApiErrorResponse::bad_request("Missing required 'id' or 'name' field")
                .into_json_tuple()
                .into_response();
        }
    };

    // Validate identifier (prevent path traversal)
    if identifier.contains('/') || identifier.contains('\\') || identifier.contains("..") {
        return ApiErrorResponse::bad_request("Invalid identifier")
            .into_json_tuple()
            .into_response();
    }

    // Determine target file path
    let target = match content_type.as_str() {
        "provider" => home_dir
            .join("providers")
            .join(format!("{identifier}.toml")),
        "agent" => home_dir
            .join("workspaces")
            .join("agents")
            .join(&identifier)
            .join("agent.toml"),
        "hand" => home_dir.join("hands").join(&identifier).join("HAND.toml"),
        "mcp" => home_dir
            .join("mcp")
            .join("catalog")
            .join(format!("{identifier}.toml")),
        "skill" => home_dir.join("skills").join(&identifier).join("skill.toml"),
        "plugin" => home_dir
            .join("plugins")
            .join(&identifier)
            .join("plugin.toml"),
        _ => {
            return ApiErrorResponse::bad_request(format!("Unknown content type '{content_type}'"))
                .into_json_tuple()
                .into_response();
        }
    };

    // Don't overwrite existing content unless explicitly allowed
    if target.exists() && !allow_overwrite {
        return ApiErrorResponse::conflict(format!(
            "{content_type} '{identifier}' already exists (use ?allow_overwrite=true to replace)"
        ))
        .into_json_tuple()
        .into_response();
    }

    // For providers: extract the `api_key` value (if present) before writing TOML.
    // The actual key is stored in secrets.env, NOT in the provider TOML file.
    let api_key_to_save: Option<(String, String)> = if content_type == "provider" {
        let obj = body.as_object();
        let api_key = obj
            .and_then(|m| m.get("api_key"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());
        let api_key_env = obj
            .and_then(|m| m.get("api_key_env"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}_API_KEY", identifier.to_uppercase().replace('-', "_")));
        api_key.map(|k| (api_key_env, k))
    } else {
        None
    };

    // Convert JSON values to TOML.
    // For providers: the catalog TOML format requires a `[provider]` section header.
    // If the body is a flat object (fields at the top level), restructure it so that
    // non-`models` fields are nested under a `"provider"` key, producing the correct
    // `[provider] … [[models]] …` layout that `ModelCatalogFile` expects.
    // Strip `api_key` from the body so the secret is not written to the TOML file.
    let body_without_secret = if content_type == "provider" {
        let mut b = body.clone();
        if let Some(obj) = b.as_object_mut() {
            obj.remove("api_key");
        }
        b
    } else {
        body.clone()
    };
    let body_for_toml = if content_type == "provider" {
        normalize_provider_body(&body_without_secret)
    } else {
        body_without_secret
    };
    let toml_value = json_to_toml_value(&body_for_toml);
    let toml_string = match toml::to_string_pretty(&toml_value) {
        Ok(s) => s,
        Err(e) => {
            return ApiErrorResponse::internal(e.to_string())
                .into_json_tuple()
                .into_response();
        }
    };

    // Create parent directories and write file
    if let Some(parent) = target.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ApiErrorResponse::internal(e.to_string())
                .into_json_tuple()
                .into_response();
        }
    }
    if let Err(e) = std::fs::write(&target, &toml_string) {
        return ApiErrorResponse::internal(e.to_string())
            .into_json_tuple()
            .into_response();
    }

    // For provider files, refresh the in-memory model catalog so new models
    // and provider config changes are available immediately.
    if content_type == "provider" {
        // Save the API key to secrets.env before detect_auth so the provider
        // is immediately recognized as configured.
        if let Some((env_var, key_value)) = &api_key_to_save {
            let secrets_path = state.kernel.home_dir().join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, env_var, key_value) {
                tracing::warn!("Failed to write API key to secrets.env: {e}");
            }
            // Serialized through the process-global env write guard (#5142):
            // `spawn_blocking` does NOT serialize concurrent env mutations.
            crate::secrets_env::set_env_var_guarded(env_var.clone(), key_value.clone()).await;
        }

        let target_for_closure = target.clone();
        state.kernel.model_catalog_update(&mut move |catalog| {
            if let Err(e) = catalog.load_catalog_file(&target_for_closure) {
                tracing::warn!("Failed to merge provider file into catalog: {e}");
            }
            catalog.detect_auth();
        });
        // Invalidate cached LLM drivers — URLs/keys may have changed.
        state.kernel.clear_driver_cache();

        if api_key_to_save.is_some() {
            state.kernel.clone().spawn_key_validation();
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "content_type": content_type,
        "identifier": identifier,
        "path": target.display().to_string(),
    }))
    .into_response()
}

/// PUT /api/registry/content/:content_type — Update (overwrite) a registry content file.
///
/// Same as POST but always allows overwriting existing files.
async fn update_registry_content(
    state: State<Arc<AppState>>,
    path: Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut overwrite = HashMap::new();
    overwrite.insert("allow_overwrite".to_string(), "true".to_string());
    create_registry_content(state, path, Query(overwrite), Json(body)).await
}

/// Ensure a provider JSON body has the `[provider]` wrapper required by
/// `ModelCatalogFile`. If the body is already wrapped (contains a `"provider"`
/// key), it is returned unchanged. Otherwise the non-`models` fields are moved
/// under `"provider"` and `models` is kept at the top level so TOML
/// serialization produces the correct `[provider] … [[models]] …` structure.
fn normalize_provider_body(body: &serde_json::Value) -> serde_json::Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };
    if obj.contains_key("provider") {
        return body.clone();
    }
    let models = obj.get("models").cloned();
    let provider_fields: serde_json::Map<String, serde_json::Value> = obj
        .iter()
        .filter(|(k, _)| k.as_str() != "models")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut restructured = serde_json::Map::new();
    restructured.insert(
        "provider".to_string(),
        serde_json::Value::Object(provider_fields),
    );
    if let Some(serde_json::Value::Array(arr)) = models {
        restructured.insert("models".to_string(), serde_json::Value::Array(arr));
    }
    serde_json::Value::Object(restructured)
}

/// Recursively convert serde_json::Value to toml::Value, stripping empty
/// strings and empty arrays to keep the generated TOML clean.
fn json_to_toml_value(json: &serde_json::Value) -> toml::Value {
    match json {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            let items: Vec<toml::Value> = arr.iter().map(json_to_toml_value).collect();
            toml::Value::Array(items)
        }
        serde_json::Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                // Skip empty strings, empty arrays, and null values
                match v {
                    serde_json::Value::String(s) if s.is_empty() => continue,
                    serde_json::Value::Array(a) if a.is_empty() => continue,
                    serde_json::Value::Null => continue,
                    // Skip empty sub-objects (sections with all empty values)
                    serde_json::Value::Object(m) if m.is_empty() => continue,
                    _ => {}
                }
                table.insert(k.clone(), json_to_toml_value(v));
            }
            toml::Value::Table(table)
        }
    }
}

// ---------------------------------------------------------------------------
// normalize_provider_body tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod provider_body_tests {
    use super::*;
    use librefang_types::model_catalog::ModelCatalogFile;

    fn round_trip(body: serde_json::Value) -> ModelCatalogFile {
        let normalized = normalize_provider_body(&body);
        let toml_value = json_to_toml_value(&normalized);
        let toml_str = toml::to_string_pretty(&toml_value).expect("serialization failed");
        toml::from_str(&toml_str).expect("TOML did not parse as ModelCatalogFile")
    }

    #[test]
    fn flat_body_gets_provider_section() {
        let body = serde_json::json!({
            "id": "deepinfra",
            "display_name": "Deepinfra",
            "api_key_env": "DEEPINFRA_API_KEY",
            "base_url": "https://api.deepinfra.com/v1/openai",
            "key_required": true
        });
        let catalog = round_trip(body);
        let provider = catalog.provider.expect("provider section must be present");
        assert_eq!(provider.id, "deepinfra");
        assert_eq!(provider.display_name, "Deepinfra");
    }

    #[test]
    fn flat_body_with_models_preserves_models() {
        let body = serde_json::json!({
            "id": "deepinfra",
            "display_name": "Deepinfra",
            "api_key_env": "DEEPINFRA_API_KEY",
            "base_url": "https://api.deepinfra.com/v1/openai",
            "key_required": true,
            "models": [{
                "id": "nvidia/NVIDIA-Nemotron-3-Super-120B-A12B",
                "display_name": "Nemotron 3 Super",
                "tier": "frontier",
                "context_window": 200000,
                "max_output_tokens": 16000,
                "input_cost_per_m": 0.1,
                "output_cost_per_m": 0.5,
                "supports_streaming": true,
                "supports_tools": true,
                "supports_vision": true
            }]
        });
        let catalog = round_trip(body);
        assert!(catalog.provider.is_some());
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(
            catalog.models[0].id,
            "nvidia/NVIDIA-Nemotron-3-Super-120B-A12B"
        );
    }

    #[test]
    fn already_wrapped_body_is_unchanged() {
        let body = serde_json::json!({
            "provider": {
                "id": "deepinfra",
                "display_name": "Deepinfra",
                "api_key_env": "DEEPINFRA_API_KEY",
                "base_url": "https://api.deepinfra.com/v1/openai",
                "key_required": true
            }
        });
        let normalized = normalize_provider_body(&body);
        // Should not double-wrap
        assert!(normalized["provider"].is_object());
        assert!(normalized
            .get("provider")
            .and_then(|p| p.get("provider"))
            .is_none());
    }

    #[test]
    fn non_object_body_is_returned_as_is() {
        let body = serde_json::json!("not an object");
        let normalized = normalize_provider_body(&body);
        assert_eq!(normalized, body);
    }
}
