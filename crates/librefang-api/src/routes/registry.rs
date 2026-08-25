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
            Err(e) => ApiErrorResponse::internal_scrub(e)
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
                Err(e) => ApiErrorResponse::internal_scrub(e)
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

/// Maximum identifier length. Bounds the generated filename so an over-long
/// identifier cannot overflow a path-component limit on any target filesystem.
const MAX_REGISTRY_IDENTIFIER_LEN: usize = 128;

#[derive(Debug)]
enum RegistryContentWriteError {
    AlreadyExists,
    Io(std::io::Error),
}

fn persist_registry_content(
    target: &std::path::Path,
    content: &[u8],
    allow_overwrite: bool,
) -> Result<Option<Vec<u8>>, RegistryContentWriteError> {
    if !allow_overwrite && target.exists() {
        return Err(RegistryContentWriteError::AlreadyExists);
    }
    let previous = if target.exists() {
        Some(std::fs::read(target).map_err(RegistryContentWriteError::Io)?)
    } else {
        None
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(RegistryContentWriteError::Io)?;
    }
    crate::atomic_write(target, content).map_err(RegistryContentWriteError::Io)?;
    Ok(previous)
}

fn rollback_registry_content(
    target: &std::path::Path,
    previous: Option<&[u8]>,
) -> std::io::Result<()> {
    match previous {
        Some(content) => crate::atomic_write(target, content),
        None => std::fs::remove_file(target),
    }
}

/// Validate a registry content identifier against `^[a-zA-Z0-9._-]+$` with a
/// length cap of [`MAX_REGISTRY_IDENTIFIER_LEN`].
///
/// The identifier is interpolated into a filesystem path, so the allowlist is
/// strict: only ASCII alphanumerics plus `.`, `_`, `-`. This rejects path
/// separators, `..`, whitespace, control characters, and shell/glob
/// metacharacters in one pass. Note `.` is permitted (provider files are
/// written as `{identifier}.toml`, and dotted ids like `my.provider` are
/// legitimate), but a bare `..` cannot pass because every character must be in
/// the class AND length ≥ 1 — and `Path::join` of a `.`/`..`-only component is
/// additionally blocked because such a string is all dots: `.` is in the
/// class, so guard the two traversal tokens explicitly.
fn is_valid_registry_identifier(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_REGISTRY_IDENTIFIER_LEN {
        return false;
    }
    // `.` is in the character class, so a `.`-only identifier (`.` / `..`)
    // would otherwise pass and resolve to the current/parent directory.
    if id == "." || id == ".." {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

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

    // Validate identifier with a strict allowlist. The identifier is joined
    // into a filesystem path (`{home}/<type>/<identifier>...`), so anything
    // outside `[a-zA-Z0-9._-]` — separators, `..`, whitespace, control bytes,
    // shell metacharacters — must be refused rather than merely the three
    // path-traversal tokens the old check caught. A length cap bounds the
    // resulting filename so an over-long identifier cannot overflow a path
    // limit on any target filesystem.
    if !is_valid_registry_identifier(&identifier) {
        return ApiErrorResponse::bad_request(
            "Invalid identifier: must be 1-128 characters of [a-zA-Z0-9._-]",
        )
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
            return ApiErrorResponse::internal_scrub(e)
                .into_json_tuple()
                .into_response();
        }
    };

    // Serialize the no-overwrite check with the durable replacement. Without
    // the shared guard, two concurrent POSTs can both observe an absent target
    // and silently overwrite one another. The fsync-based writer is blocking,
    // so perform the filesystem transaction off the async runtime worker.
    let _write_guard = state.config_write_lock.lock().await;
    let write_target = target.clone();
    let write_content = toml_string.into_bytes();
    let write_result = tokio::task::spawn_blocking(move || {
        persist_registry_content(&write_target, &write_content, allow_overwrite)
    })
    .await;
    let previous_content = match write_result {
        Ok(Ok(previous)) => previous,
        Ok(Err(RegistryContentWriteError::AlreadyExists)) => {
            // The message names the actions a caller can actually take. For
            // providers the answer is usually to edit the live key or URL,
            // not replace a registry-managed definition (#6703).
            let remedy = if content_type == "provider" {
                format!(
                    " — set its API key with POST /api/providers/{identifier}/key, its endpoint with PUT /api/providers/{identifier}/url, or replace the whole definition with PUT /api/registry/content/provider"
                )
            } else {
                format!(
                    " — replace the whole definition with PUT /api/registry/content/{content_type}"
                )
            };
            return ApiErrorResponse::conflict(format!(
                "{content_type} '{identifier}' already exists{remedy}"
            ))
            .into_json_tuple()
            .into_response();
        }
        Ok(Err(RegistryContentWriteError::Io(error))) => {
            tracing::error!(%error, "failed to persist registry content");
            return ApiErrorResponse::internal("failed to persist registry content")
                .into_json_tuple()
                .into_response();
        }
        Err(error) => {
            return ApiErrorResponse::internal_scrub(format!(
                "registry content write task failed: {error}"
            ))
            .into_json_tuple()
            .into_response();
        }
    };

    // For provider files, refresh the in-memory model catalog so new models
    // and provider config changes are available immediately.
    if content_type == "provider" {
        // Save the API key to secrets.env before detect_auth so the provider
        // is immediately recognized as configured.
        if let Some((env_var, key_value)) = &api_key_to_save {
            let secrets_path = state.kernel.home_dir().join("secrets.env");
            let env_var_for_write = env_var.clone();
            let key_value_for_write = key_value.clone();
            let secret_result = tokio::task::spawn_blocking(move || {
                write_secret_env(&secrets_path, &env_var_for_write, &key_value_for_write)
            })
            .await;
            if let Err(e) = secret_result.unwrap_or_else(|e| {
                Err(std::io::Error::other(format!(
                    "secret write task failed: {e}"
                )))
            }) {
                tracing::warn!("Failed to write API key to secrets.env: {e}");
            }
            // Serialized through the process-global env write guard (#5142):
            // `spawn_blocking` does NOT serialize concurrent env mutations.
            crate::secrets_env::set_env_var_guarded(env_var.clone(), key_value.clone()).await;
        }

        let target_for_closure = target.clone();
        let kernel = state.kernel.clone();
        // The trait method returns `()`; capture the load result via a
        // surrounding `&mut Option<_>` (per the `model_catalog_update` contract).
        let merge_result = tokio::task::spawn_blocking(move || {
            let mut merge_result: Option<Result<usize, String>> = None;
            let sink = &mut merge_result;
            kernel.model_catalog_update(&mut move |catalog| {
                *sink = Some(catalog.load_catalog_file(&target_for_closure));
                catalog.detect_auth();
            });
            merge_result
        })
        .await
        .unwrap_or_else(|error| Some(Err(format!("catalog load task failed: {error}"))));
        // Invalidate cached LLM drivers — URLs/keys may have changed.
        state.kernel.clear_driver_cache();

        // The file was written to disk but failed to load into the catalog.
        // Previously this was swallowed as a `warn!`, so the dashboard saw a
        // success while the provider silently never appeared (#5822). Roll
        // back the unusable file — leaving it would also re-trigger the same
        // parse warning on every daemon boot — and surface the error so the
        // operator can correct the definition.
        if let Some(Err(e)) = merge_result {
            let rollback_target = target.clone();
            let rollback_result = tokio::task::spawn_blocking(move || {
                rollback_registry_content(&rollback_target, previous_content.as_deref())
            })
            .await;
            if let Err(error) = rollback_result.unwrap_or_else(|error| {
                Err(std::io::Error::other(format!(
                    "registry rollback task failed: {error}"
                )))
            }) {
                tracing::error!(%error, "failed to roll back rejected provider definition");
            }
            return ApiErrorResponse::bad_request(format!(
                "Provider definition was rejected and not saved: {e}"
            ))
            .into_json_tuple()
            .into_response();
        }

        if api_key_to_save.is_some() {
            state.kernel.clone().spawn_key_validation();
        }
    }

    // Return a path relative to `home_dir` rather than an absolute path.
    // The absolute form leaks the operator's OS username via
    // `/Users/<user>` (macOS) or `/home/<user>` (Linux), which is a
    // low-severity but unnecessary information disclosure to any caller
    // of this endpoint. Relative paths still let the dashboard show
    // "where" the file lives under `~/.librefang/` (e.g.
    // `providers/openai.toml`) without revealing host filesystem layout.
    // Falls back to the absolute path's file name if `strip_prefix`
    // fails — defensive only; `target` is constructed from `home_dir`
    // above so the prefix should always match.
    let relative_path = target
        .strip_prefix(home_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(target.file_name().unwrap_or(target.as_os_str()))
        });
    // Render with forward-slash separators regardless of host OS — this
    // is a JSON API response and the dashboard / SDKs expect a
    // platform-independent shape (`providers/openai.toml`, not
    // `providers\openai.toml`). `Path::display()` uses the platform
    // separator, which produced backslashes on Windows and broke the
    // `registry_content_path_test` regression tests.
    let relative_path_str = relative_path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    Json(serde_json::json!({
        "ok": true,
        "content_type": content_type,
        "identifier": identifier,
        "path": relative_path_str,
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

#[cfg(test)]
mod identifier_validation_tests {
    use super::*;

    #[test]
    fn accepts_well_formed_identifiers() {
        for id in [
            "deepinfra",
            "my-provider",
            "my_provider",
            "my.provider",
            "Provider123",
            "a",
            "a.b-c_d.1",
        ] {
            assert!(
                is_valid_registry_identifier(id),
                "expected {id:?} to be accepted"
            );
        }
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for id in [
            "../etc",
            "..",
            ".",
            "a/b",
            "a\\b",
            "..\\..\\windows",
            "/abs/path",
            "\\\\server\\share",
        ] {
            assert!(
                !is_valid_registry_identifier(id),
                "expected {id:?} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_metacharacters_whitespace_and_control() {
        for id in [
            "a b", "a;b", "a|b", "a&b", "a$b", "a*b", "a?b", "a(b)", "a\tb", "a\nb", "a\0b", "a:b",
            "naïve", // non-ASCII
        ] {
            assert!(
                !is_valid_registry_identifier(id),
                "expected {id:?} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_and_overlong() {
        assert!(!is_valid_registry_identifier(""));
        let max = "a".repeat(MAX_REGISTRY_IDENTIFIER_LEN);
        assert!(is_valid_registry_identifier(&max), "exactly 128 must pass");
        let over = "a".repeat(MAX_REGISTRY_IDENTIFIER_LEN + 1);
        assert!(
            !is_valid_registry_identifier(&over),
            "129 chars must be rejected"
        );
    }
}

#[cfg(test)]
mod registry_content_write_tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn registry_serialization_errors_are_scrubbed() {
        let internal = "TOML serializer failed at providers.secret_field";
        let response = ApiErrorResponse::internal_scrub(internal).into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Internal server error"));
        assert!(!body.contains(internal));
        assert!(!body.contains("secret_field"));
    }

    #[test]
    fn creates_and_atomically_replaces_without_staging_residue() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("providers/example.toml");

        assert_eq!(
            persist_registry_content(&target, b"first", false).unwrap(),
            None
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"first");

        let error = persist_registry_content(&target, b"rejected", false).unwrap_err();
        assert!(matches!(error, RegistryContentWriteError::AlreadyExists));
        assert_eq!(std::fs::read(&target).unwrap(), b"first");

        assert_eq!(
            persist_registry_content(&target, b"second", true).unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"second");

        rollback_registry_content(&target, Some(b"first")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        rollback_registry_content(&target, None).unwrap();
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            0
        );
    }
}
