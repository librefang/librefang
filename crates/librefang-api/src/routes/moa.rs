//! HTTP routes for the Mixture-of-Agents (MoA) configuration surface.
//!
//! MoA runs a user turn in two phases: N advisor models produce private
//! advice from a flattened, text-only view of the conversation, then an
//! aggregator model receives that advice and acts as the agent (answering
//! the user, calling tools, driving the normal loop). Configuration lives in
//! the `[moa]` section of `config.toml` as a set of named presets; when an
//! agent sets `provider = "moa"`, the kernel resolves the named preset (or
//! `[moa].default_preset`) into a composite driver.
//!
//! These endpoints expose that section for the dashboard's MoA editor:
//!
//! - `GET    /api/moa`                  → normalized `[moa]` view
//! - `PUT    /api/moa`                  → replace the whole `[moa]` section
//! - `GET    /api/moa/presets`          → list presets, default marked
//! - `PUT    /api/moa/presets/{name}`   → create/update a single preset
//! - `DELETE /api/moa/presets/{name}`   → delete a single preset
//!
//! Writes follow the `persist_budget` pipeline: lock → read → `toml_edit` →
//! validate → atomic write → hot-reload. Boot uses the tolerant normalizer
//! (`MoaConfig::normalized`), so a malformed preset never blocks daemon
//! start; the strict `KernelConfig::validate_moa` here rejects bad input at
//! the API boundary before anything touches disk.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::AppState;
use crate::types::ApiErrorResponse;
use librefang_types::config::{KernelConfig, MoaConfig, MoaPreset};

/// Build routes for the MoA configuration domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/moa", axum::routing::get(get_moa).put(put_moa))
        .route("/moa/presets", axum::routing::get(list_presets))
        .route(
            "/moa/presets/{name}",
            axum::routing::put(put_preset).delete(delete_preset),
        )
}

/// GET /api/moa — Return the normalized `[moa]` configuration.
///
/// Normalization injects the built-in default preset when none is configured
/// and repairs dangling pointers, so the dashboard always sees a usable view
/// even on a fresh install with an empty `[moa]` section.
#[utoipa::path(
    get,
    path = "/api/moa",
    tag = "moa",
    responses(
        (status = 200, description = "Normalized MoA configuration", body = crate::types::JsonObject)
    )
)]
pub async fn get_moa(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.kernel.config_ref();
    let normalized = config.moa.normalized();
    Json(serde_json::to_value(&normalized).unwrap_or_default())
}

/// PUT /api/moa — Replace the whole `[moa]` section and hot-reload.
///
/// The body is deserialized as a complete `MoaConfig`. It is validated with
/// the strict `validate_moa` before any write; on failure the response carries
/// the full problem list so the editor can point at every offending field.
#[utoipa::path(
    put,
    path = "/api/moa",
    tag = "moa",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "MoA configuration replaced and reloaded", body = crate::types::JsonObject),
        (status = 400, description = "Validation failed; see `problems`", body = crate::types::JsonObject)
    )
)]
pub async fn put_moa(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let new_moa: MoaConfig = match serde_json::from_value(body) {
        Ok(m) => m,
        Err(e) => {
            return ApiErrorResponse::bad_request(format!("invalid [moa] body: {e}"))
                .into_response();
        }
    };

    if let Some(problems) = moa_validation_problems(&state, &new_moa) {
        return validation_error(problems);
    }

    match persist_moa(&state, &new_moa).await {
        Ok(reload) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "reload": reload,
                "moa": serde_json::to_value(&new_moa).unwrap_or_default(),
            })),
        )
            .into_response(),
        Err(e) => persist_error_response(e),
    }
}

/// GET /api/moa/presets — List presets with the default marked.
///
/// Returns the normalized preset set, so the built-in default appears even
/// when the on-disk `[moa]` section is empty.
#[utoipa::path(
    get,
    path = "/api/moa/presets",
    tag = "moa",
    responses(
        (status = 200, description = "MoA presets", body = crate::types::JsonObject)
    )
)]
pub async fn list_presets(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.kernel.config_ref();
    let moa = config.moa.normalized();
    let presets: Vec<serde_json::Value> = moa
        .presets
        .iter()
        .map(|(name, preset)| {
            serde_json::json!({
                "name": name,
                "is_default": *name == moa.default_preset,
                "enabled": preset.enabled,
                "preset": serde_json::to_value(preset).unwrap_or_default(),
            })
        })
        .collect();
    Json(serde_json::json!({
        "default_preset": moa.default_preset,
        "presets": presets,
    }))
}

/// PUT /api/moa/presets/{name} — Create or update a single preset.
///
/// Merges the submitted preset into the on-disk `[moa]` section (other
/// presets are left untouched), validates the result, and hot-reloads. The
/// built-in default preset is implicit — it is not materialized into
/// `config.toml` by this edit.
#[utoipa::path(
    put,
    path = "/api/moa/presets/{name}",
    tag = "moa",
    params(("name" = String, Path, description = "Preset name")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Preset created or updated", body = crate::types::JsonObject),
        (status = 400, description = "Validation failed; see `problems`", body = crate::types::JsonObject)
    )
)]
pub async fn put_preset(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if name.trim().is_empty() {
        return ApiErrorResponse::bad_request("preset name must not be empty").into_response();
    }
    let preset: MoaPreset = match serde_json::from_value(body) {
        Ok(p) => p,
        Err(e) => {
            return ApiErrorResponse::bad_request(format!("invalid preset body: {e}"))
                .into_response();
        }
    };

    // Merge into the raw on-disk section so unrelated presets are preserved
    // and the implicit built-in default stays implicit.
    let mut new_moa = state.kernel.config_ref().moa.clone();
    new_moa.presets.insert(name.clone(), preset);

    if let Some(problems) = moa_validation_problems(&state, &new_moa) {
        return validation_error(problems);
    }

    match persist_moa(&state, &new_moa).await {
        Ok(reload) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "reload": reload,
                "name": name,
            })),
        )
            .into_response(),
        Err(e) => persist_error_response(e),
    }
}

/// DELETE /api/moa/presets/{name} — Delete a single preset.
///
/// Refuses to delete the last remaining preset (MoA needs at least one). If
/// the deleted preset was the `default_preset`, the default pointer is
/// reassigned to the first remaining preset.
#[utoipa::path(
    delete,
    path = "/api/moa/presets/{name}",
    tag = "moa",
    params(("name" = String, Path, description = "Preset name")),
    responses(
        (status = 200, description = "Preset deleted", body = crate::types::JsonObject),
        (status = 404, description = "Preset not found", body = crate::types::JsonObject),
        (status = 409, description = "Cannot delete the last preset", body = crate::types::JsonObject)
    )
)]
pub async fn delete_preset(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    let mut new_moa = state.kernel.config_ref().moa.clone();

    if new_moa.presets.remove(&name).is_none() {
        return ApiErrorResponse::not_found(format!("preset '{name}' not found")).into_response();
    }

    if new_moa.presets.is_empty() {
        return ApiErrorResponse::conflict(
            "cannot delete the last MoA preset; at least one must remain",
        )
        .into_response();
    }

    // Reassign the default pointer if we just deleted it.
    if new_moa.default_preset == name {
        if let Some(first) = new_moa.presets.keys().next() {
            new_moa.default_preset = first.clone();
        }
    }

    match persist_moa(&state, &new_moa).await {
        Ok(reload) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "reload": reload,
                "deleted": name,
                "default_preset": new_moa.default_preset,
            })),
        )
            .into_response(),
        Err(e) => persist_error_response(e),
    }
}

/// Run the strict MoA validator against a candidate config carrying `new_moa`.
///
/// Returns `None` when valid, or the problem list. The validator runs against
/// a full `KernelConfig` clone (live snapshot + the proposed `[moa]`) so the
/// recursion guard and slot checks see exactly what would be persisted.
fn moa_validation_problems(state: &Arc<AppState>, new_moa: &MoaConfig) -> Option<Vec<String>> {
    let mut candidate = (*state.kernel.config_snapshot()).clone();
    candidate.moa = new_moa.clone();
    let problems = candidate.validate_moa();
    if problems.is_empty() {
        None
    } else {
        Some(problems)
    }
}

/// Build the structured 400 response carrying the validator's problem list.
fn validation_error(problems: Vec<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "status": "error",
            "error": "invalid MoA configuration",
            "problems": problems,
        })),
    )
        .into_response()
}

/// Failure modes for [`persist_moa`], mirroring `PersistBudgetError`.
///
/// `BadRequest` is a validator rejection the operator can fix; `Internal`
/// covers genuine I/O / kernel failures whose detail is scrubbed from the
/// response (audit: rusqlite-errors-leak) and kept in the log only.
enum PersistMoaError {
    BadRequest(String),
    Internal(String),
}

/// Replace the `[moa]` table in `config.toml` with the serialised form of
/// `new_moa`, preserving comments and unrelated sections, then call
/// `reload_config()` so driver resolution picks up the change.
///
/// Mirrors the `persist_budget` pipeline (lock → read → `toml_edit` →
/// validate → atomic write → reload). A read failure on an existing file
/// aborts rather than falling back to empty, which would silently drop every
/// other section on the next write (#3368).
async fn persist_moa(
    state: &Arc<AppState>,
    new_moa: &MoaConfig,
) -> Result<String, PersistMoaError> {
    let _guard = state.config_write_lock.lock().await;

    let config_path = state.kernel.home_dir().join("config.toml");
    if config_path.file_name().and_then(|n| n.to_str()) != Some("config.toml")
        || config_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(PersistMoaError::Internal(
            "invalid config file path".to_string(),
        ));
    }

    let raw = match tokio::fs::read_to_string(&config_path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(PersistMoaError::Internal(format!(
                "could not read existing config.toml: {e}"
            )));
        }
    };
    let mut doc: toml_edit::DocumentMut = raw.parse().map_err(|e| {
        PersistMoaError::Internal(format!(
            "config.toml is not valid TOML — refusing to overwrite: {e}"
        ))
    })?;

    // Serialise `MoaConfig` to a TOML table and replace the existing `[moa]`
    // table. `toml_edit::ser::to_document` handles the nested `presets` map
    // sitting alongside scalar fields without the `ValueAfterTable` reorder
    // hazard the strict `toml` crate rejects.
    let serialised = toml_edit::ser::to_document(new_moa)
        .map_err(|e| PersistMoaError::Internal(format!("serialize moa: {e}")))?;
    doc.insert("moa", toml_edit::Item::Table(serialised.as_table().clone()));

    let new_toml = doc.to_string();
    let parsed: KernelConfig = toml::from_str(&new_toml)
        .map_err(|e| PersistMoaError::Internal(format!("invalid config after edit: {e}")))?;
    if let Err(errors) = state.kernel.validate_config_for_reload(&parsed) {
        return Err(PersistMoaError::BadRequest(format!(
            "invalid config: {}",
            errors.join("; ")
        )));
    }

    crate::atomic_write(&config_path, new_toml.as_bytes())
        .map_err(|e| PersistMoaError::Internal(format!("write config.toml: {e}")))?;

    let reload = match state.kernel.reload_config().await {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(e) => {
            return Err(PersistMoaError::Internal(format!(
                "config written but reload failed: {e}"
            )));
        }
    };

    Ok(reload.to_string())
}

/// Map a [`PersistMoaError`] onto an HTTP response, scrubbing internal detail.
fn persist_error_response(e: PersistMoaError) -> Response {
    match e {
        PersistMoaError::BadRequest(m) => ApiErrorResponse::bad_request(m).into_response(),
        PersistMoaError::Internal(m) => {
            tracing::error!(error = %m, "MoA config persist failed");
            ApiErrorResponse::internal("Internal server error").into_response()
        }
    }
}
