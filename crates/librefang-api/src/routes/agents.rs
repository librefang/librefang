//! Agent CRUD, messaging, sessions, files, and upload handlers.

use super::AppState;

/// Build all routes for the Agent domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route(
            "/agents",
            axum::routing::get(list_agents).post(spawn_agent),
        )
        // Bulk agent operations (placed before /agents/{id} to avoid path conflicts)
        .route(
            "/agents/bulk",
            axum::routing::post(bulk_create_agents).delete(bulk_delete_agents),
        )
        .route(
            "/agents/bulk/start",
            axum::routing::post(bulk_start_agents),
        )
        .route(
            "/agents/bulk/stop",
            axum::routing::post(bulk_stop_agents),
        )
        .route(
            "/agents/{id}",
            axum::routing::get(get_agent)
                .delete(kill_agent)
                .patch(patch_agent),
        )
        .route(
            "/agents/{id}/mode",
            axum::routing::put(set_agent_mode),
        )
        .route(
            "/agents/{id}/suspend",
            axum::routing::put(suspend_agent),
        )
        .route(
            "/agents/{id}/resume",
            axum::routing::put(resume_agent),
        )
        .route(
            "/agents/{id}/message",
            axum::routing::post(send_message),
        )
        .route(
            "/agents/{id}/inject",
            axum::routing::post(inject_message),
        )
        .route(
            "/agents/{id}/message/stream",
            axum::routing::post(send_message_stream),
        )
        .route(
            "/agents/{id}/session",
            axum::routing::get(get_agent_session),
        )
        .route(
            "/agents/{id}/sessions",
            axum::routing::get(list_agent_sessions).post(create_agent_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/switch",
            axum::routing::post(switch_agent_session),
        )
        .route(
            "/agents/{id}/sessions/{session_id}/export",
            axum::routing::get(export_session),
        )
        .route(
            "/agents/{id}/sessions/import",
            axum::routing::post(import_session),
        )
        .route(
            "/agents/{id}/session/reset",
            axum::routing::post(reset_session),
        )
        .route(
            "/agents/{id}/session/reboot",
            axum::routing::post(reboot_session),
        )
        .route(
            "/agents/{id}/history",
            axum::routing::delete(clear_agent_history),
        )
        .route(
            "/agents/{id}/session/compact",
            axum::routing::post(compact_session),
        )
        .route("/agents/{id}/stop", axum::routing::post(stop_agent))
        .route("/agents/{id}/model", axum::routing::put(set_model))
        .route(
            "/agents/{id}/traces",
            axum::routing::get(get_agent_traces),
        )
        .route(
            "/agents/{id}/tools",
            axum::routing::get(get_agent_tools).put(set_agent_tools),
        )
        .route(
            "/agents/{id}/skills",
            axum::routing::get(get_agent_skills).put(set_agent_skills),
        )
        .route(
            "/agents/{id}/mcp_servers",
            axum::routing::get(get_agent_mcp_servers).put(set_agent_mcp_servers),
        )
        .route(
            "/agents/{id}/identity",
            axum::routing::patch(update_agent_identity),
        )
        .route(
            "/agents/{id}/config",
            axum::routing::patch(patch_agent_config),
        )
        .route(
            "/agents/{id}/clone",
            axum::routing::post(clone_agent),
        )
        .route(
            "/agents/{id}/reload",
            axum::routing::post(reload_agent_manifest),
        )
        .route(
            "/agents/{id}/files",
            axum::routing::get(list_agent_files),
        )
        .route(
            "/agents/{id}/files/{filename}",
            axum::routing::get(get_agent_file)
                .put(set_agent_file)
                .delete(delete_agent_file),
        )
        .route(
            "/agents/{id}/metrics",
            axum::routing::get(agent_metrics),
        )
        .route("/agents/{id}/logs", axum::routing::get(agent_logs))
        .route(
            "/agents/{id}/deliveries",
            axum::routing::get(get_agent_deliveries),
        )
        .route("/agents/{id}/ws", axum::routing::get(crate::ws::agent_ws))
        .route(
            "/uploads/{file_id}",
            axum::routing::get(serve_upload),
        )
        .route(
            "/agents/{id}/update",
            axum::routing::put(update_agent),
        )
        .route(
            "/agents/{id}/push",
            axum::routing::post(push_message),
        )
}
use crate::middleware::RequestLanguage;
use crate::stream_dedup::StreamDedup;
use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use librefang_channels::types::SenderContext;
use librefang_kernel::LibreFangKernel;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::{AgentId, AgentIdentity, AgentManifest};
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

// ---------------------------------------------------------------------------
// Shared manifest resolution helper
// ---------------------------------------------------------------------------

/// Maximum manifest size (1MB) to prevent parser memory exhaustion.
const MAX_MANIFEST_SIZE: usize = 1024 * 1024;

/// Resolved manifest ready for spawning.
struct ResolvedManifest {
    manifest: AgentManifest,
    name: String,
}

/// Error from manifest resolution — carries a user-facing message.
struct ManifestError {
    message: String,
}

/// Resolve a `SpawnRequest` into a parsed `AgentManifest`.
///
/// Handles template lookup, path sanitization, size guard, signed manifest
/// verification, and TOML parsing — shared by both single and bulk spawn.
async fn resolve_manifest(
    state: &AppState,
    req: &SpawnRequest,
    lang: &'static str,
) -> Result<ResolvedManifest, ManifestError> {
    // Resolve template name → manifest_toml
    let manifest_toml = if req.manifest_toml.trim().is_empty() {
        if let Some(ref tmpl_name) = req.template {
            let safe_name: String = tmpl_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if safe_name.is_empty() || safe_name != *tmpl_name {
                let t = ErrorTranslator::new(lang);
                return Err(ManifestError {
                    message: t.t("api-error-template-invalid-name"),
                });
            }
            let tmpl_path = state
                .kernel
                .config_ref()
                .home_dir
                .join("workspaces")
                .join("agents")
                .join(&safe_name)
                .join("agent.toml");
            // Use tokio::fs to avoid blocking in an async context
            match tokio::fs::read_to_string(&tmpl_path).await {
                Ok(content) => content,
                Err(_) => {
                    let t = ErrorTranslator::new(lang);
                    return Err(ManifestError {
                        message: t.t_args("api-error-template-not-found", &[("name", &safe_name)]),
                    });
                }
            }
        } else {
            let t = ErrorTranslator::new(lang);
            return Err(ManifestError {
                message: t.t("api-error-template-required"),
            });
        }
    } else {
        req.manifest_toml.clone()
    };

    // Size guard
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        let t = ErrorTranslator::new(lang);
        return Err(ManifestError {
            message: t.t("api-error-manifest-too-large"),
        });
    }

    // SECURITY: Verify Ed25519 signature when provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                if verified_toml.trim() != manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    let t = ErrorTranslator::new(lang);
                    return Err(ManifestError {
                        message: t.t("api-error-manifest-signature-mismatch"),
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit().record(
                    "system",
                    librefang_runtime::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                let t = ErrorTranslator::new(lang);
                return Err(ManifestError {
                    message: t.t("api-error-manifest-signature-failed"),
                });
            }
        }
    }

    // Parse TOML
    let mut manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            let _ = e;
            let t = ErrorTranslator::new(lang);
            return Err(ManifestError {
                message: t.t("api-error-manifest-invalid-format"),
            });
        }
    };

    // Allow callers to override the manifest name, enabling multiple agents
    // from the same template with distinct names.
    if let Some(ref custom_name) = req.name {
        if !custom_name.trim().is_empty() {
            manifest.name = custom_name.trim().to_string();
        }
    }

    let name = manifest.name.clone();
    Ok(ResolvedManifest { manifest, name })
}

/// POST /api/agents — Spawn a new agent.
#[utoipa::path(
    post,
    path = "/api/agents",
    tag = "agents",
    request_body = crate::types::SpawnRequest,
    responses(
        (status = 200, description = "Agent spawned", body = crate::types::SpawnResponse),
        (status = 400, description = "Invalid manifest")
    )
)]
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());

    let resolved = match resolve_manifest(&state, &req, l).await {
        Ok(r) => r,
        Err(e) => {
            // Map specific errors to appropriate HTTP status codes
            let status = if e.message.contains("too large") {
                StatusCode::PAYLOAD_TOO_LARGE
            } else if e.message.contains("not found") && e.message.contains("Template") {
                StatusCode::NOT_FOUND
            } else if e.message.contains("signature verification failed") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(serde_json::json!({"error": e.message})));
        }
    };

    match state.kernel.spawn_agent(resolved.manifest) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(serde_json::json!(SpawnResponse {
                agent_id: id.to_string(),
                name: resolved.name,
            })),
        ),
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            let t = ErrorTranslator::new(l);
            let status = match &e {
                librefang_kernel::error::KernelError::LibreFang(
                    librefang_types::error::LibreFangError::AgentAlreadyExists(_),
                ) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-error", &[("error", &e.to_string())])}),
                ),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Bulk agent operations
// ---------------------------------------------------------------------------

/// Maximum number of agents allowed in a single bulk request.
const BULK_LIMIT: usize = 50;

/// Validate that a bulk request array is non-empty and within the limit.
fn validate_bulk_size(
    len: usize,
    lang: &'static str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let t = ErrorTranslator::new(lang);
    if len == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-agent-array-empty")})),
        ));
    }
    if len > BULK_LIMIT {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-agent-array-too-large", &[("max", &BULK_LIMIT.to_string())])}),
            ),
        ));
    }
    Ok(())
}

/// POST /api/agents/bulk — Create multiple agents at once.
#[utoipa::path(
    post,
    path = "/api/agents/bulk",
    tag = "agents",
    request_body(content = BulkCreateRequest, description = "Array of agent spawn requests"),
    responses(
        (status = 200, description = "Create multiple agents at once", body = serde_json::Value)
    )
)]
pub async fn bulk_create_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkCreateRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    if let Err(resp) = validate_bulk_size(req.agents.len(), l) {
        return resp;
    }

    let mut results: Vec<BulkCreateResult> = Vec::with_capacity(req.agents.len());

    for (index, spawn_req) in req.agents.iter().enumerate() {
        match resolve_manifest(&state, spawn_req, l).await {
            Err(e) => {
                results.push(BulkCreateResult {
                    index,
                    success: false,
                    agent_id: None,
                    name: None,
                    error: Some(e.message),
                });
            }
            Ok(resolved) => {
                let name = resolved.name.clone();
                match state.kernel.spawn_agent(resolved.manifest) {
                    Ok(id) => {
                        results.push(BulkCreateResult {
                            index,
                            success: true,
                            agent_id: Some(id.to_string()),
                            name: Some(name),
                            error: None,
                        });
                    }
                    Err(e) => {
                        let t = ErrorTranslator::new(l);
                        results.push(BulkCreateResult {
                            index,
                            success: false,
                            agent_id: None,
                            name: None,
                            error: Some(t.t_args(
                                "api-error-agent-clone-spawn-failed",
                                &[("error", &e.to_string())],
                            )),
                        });
                    }
                }
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// DELETE /api/agents/bulk — Delete multiple agents at once.
#[utoipa::path(
    delete,
    path = "/api/agents/bulk",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to delete"),
    responses(
        (status = 200, description = "Delete multiple agents at once", body = serde_json::Value)
    )
)]
pub async fn bulk_delete_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = validate_bulk_size(req.agent_ids.len(), l) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        // Same guard as the single-agent kill path: hand-spawned agents
        // must be removed by deactivating their owning hand, not directly.
        if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
            if entry.is_hand {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(
                        "Cannot delete a hand-spawned agent directly; deactivate or uninstall the owning hand instead.".to_string(),
                    ),
                });
                continue;
            }
        }
        match state.kernel.kill_agent(agent_id) {
            Ok(()) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some("Deleted".into()),
                    error: None,
                });
            }
            Err(e) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t_args("api-error-generic", &[("error", &e.to_string())])),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// POST /api/agents/bulk/start — Set multiple agents to Full mode.
#[utoipa::path(
    post,
    path = "/api/agents/bulk/start",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to start"),
    responses(
        (status = 200, description = "Start multiple agents (set to Full mode)", body = serde_json::Value)
    )
)]
pub async fn bulk_start_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    use librefang_types::agent::AgentMode;

    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = validate_bulk_size(req.agent_ids.len(), l) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        match state
            .kernel
            .agent_registry()
            .set_mode(agent_id, AgentMode::Full)
        {
            Ok(()) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some("Agent set to Full mode".into()),
                    error: None,
                });
            }
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-not-found")),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// POST /api/agents/bulk/stop — Stop multiple agents' current runs.
#[utoipa::path(
    post,
    path = "/api/agents/bulk/stop",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to stop"),
    responses(
        (status = 200, description = "Stop multiple agents' current runs", body = serde_json::Value)
    )
)]
pub async fn bulk_stop_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = validate_bulk_size(req.agent_ids.len(), l) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        match state.kernel.stop_agent_run(agent_id) {
            Ok(cancelled) => {
                let msg = if cancelled {
                    "Run cancelled"
                } else {
                    "No active run"
                };
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some(msg.into()),
                    error: None,
                });
            }
            Err(e) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t_args("api-error-generic", &[("error", &e.to_string())])),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// Enrich an `AgentEntry` into a JSON value with catalog data.
pub(crate) fn enrich_agent_json(
    e: &librefang_types::agent::AgentEntry,
    dm: &librefang_types::config::DefaultModelConfig,
    catalog: &Option<
        std::sync::RwLockReadGuard<'_, librefang_runtime::model_catalog::ModelCatalog>,
    >,
) -> serde_json::Value {
    let provider = if e.manifest.model.provider.is_empty() || e.manifest.model.provider == "default"
    {
        dm.provider.as_str()
    } else {
        e.manifest.model.provider.as_str()
    };
    let model = if e.manifest.model.model.is_empty() || e.manifest.model.model == "default" {
        dm.model.as_str()
    } else {
        e.manifest.model.model.as_str()
    };

    let (tier, auth_status, supports_thinking) = catalog
        .as_ref()
        .map(|cat| {
            let model_entry = cat.find_model(model);
            let tier = model_entry
                .map(|m| format!("{:?}", m.tier).to_lowercase())
                .unwrap_or_else(|| "unknown".to_string());
            let thinking = model_entry.map(|m| m.supports_thinking).unwrap_or(false);
            let auth = cat
                .get_provider(provider)
                .map(|p| p.auth_status.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            (tier, auth, thinking)
        })
        .unwrap_or(("unknown".to_string(), "unknown".to_string(), false));

    let ready =
        matches!(e.state, librefang_types::agent::AgentState::Running) && auth_status != "missing";

    serde_json::json!({
        "id": e.id.to_string(),
        "name": e.name,
        "is_hand": e.is_hand,
        "state": format!("{:?}", e.state),
        "mode": e.mode,
        "created_at": e.created_at.to_rfc3339(),
        "last_active": e.last_active.to_rfc3339(),
        "model_provider": provider,
        "model_name": model,
        "model_tier": tier,
        "auth_status": auth_status,
        "supports_thinking": supports_thinking,
        "ready": ready,
        "profile": e.manifest.profile,
        "identity": {
            "emoji": e.identity.emoji,
            "avatar_url": e.identity.avatar_url,
            "color": e.identity.color,
        },
        "web_search_augmentation": e.manifest.web_search_augmentation,
    })
}

pub(crate) fn effective_default_model(
    base: &librefang_types::config::DefaultModelConfig,
    override_dm: Option<&librefang_types::config::DefaultModelConfig>,
) -> librefang_types::config::DefaultModelConfig {
    override_dm.cloned().unwrap_or_else(|| base.clone())
}

/// GET /api/agents — List agents with optional filtering, pagination, and sorting.
///
/// Query parameters (all optional — omitting them returns all agents):
///   - `q`: free-text search across name and description (case-insensitive)
///   - `status`: filter by lifecycle state (e.g. "running", "suspended")
///   - `limit` / `offset`: pagination
///   - `sort`: field to sort by — "name", "created_at", "last_active", "state"
///   - `order`: "asc" (default) or "desc"
#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "agents",
    params(
        ("q" = Option<String>, Query, description = "Free-text search on name/description"),
        ("status" = Option<String>, Query, description = "Filter by agent state"),
        ("limit" = Option<usize>, Query, description = "Max items to return"),
        ("offset" = Option<usize>, Query, description = "Items to skip"),
        ("sort" = Option<String>, Query, description = "Sort field: name, created_at, last_active, state"),
        ("order" = Option<String>, Query, description = "Sort order: asc or desc"),
    ),
    responses(
        (status = 200, description = "Paginated list of agents")
    )
)]
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<AgentListQuery>,
) -> impl IntoResponse {
    let catalog = state.kernel.model_catalog_ref().read().ok();
    let dm = {
        let dm_override = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        effective_default_model(
            &state.kernel.config_ref().default_model,
            dm_override.as_ref(),
        )
    };

    let mut agents: Vec<librefang_types::agent::AgentEntry> = state.kernel.agent_registry().list();

    // -- Filtering --
    // Exclude hand agents by default; pass ?include_hands=true to include them.
    if !params.include_hands.unwrap_or(false) {
        agents.retain(|e| !e.is_hand);
    }

    if let Some(ref q) = params.q {
        let q_lower = q.to_lowercase();
        agents.retain(|e| {
            e.name.to_lowercase().contains(&q_lower)
                || e.manifest.description.to_lowercase().contains(&q_lower)
        });
    }

    if let Some(ref status) = params.status {
        let status_lower = status.to_lowercase();
        agents.retain(|e| format!("{:?}", e.state).to_lowercase() == status_lower);
    }

    let total = agents.len();

    // -- Sorting --
    const VALID_SORT_FIELDS: &[&str] = &["name", "created_at", "last_active", "state"];
    let sort_field = params.sort.as_deref().unwrap_or("name");
    if !VALID_SORT_FIELDS.contains(&sort_field) {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        let msg = t.t_args(
            "api-error-agent-invalid-sort",
            &[
                ("field", sort_field),
                ("valid", &format!("{:?}", VALID_SORT_FIELDS)),
            ],
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": msg
            })),
        )
            .into_response();
    }
    let descending = params
        .order
        .as_deref()
        .map(|o| o.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);

    agents.sort_by(|a, b| {
        let cmp = match sort_field {
            "created_at" => a.created_at.cmp(&b.created_at),
            "last_active" => a.last_active.cmp(&b.last_active),
            "state" => format!("{:?}", a.state).cmp(&format!("{:?}", b.state)),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // -- Pagination --
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.map(|l| l.min(100));
    let agents: Vec<librefang_types::agent::AgentEntry> = if let Some(lim) = limit {
        agents.into_iter().skip(offset).take(lim).collect()
    } else {
        agents.into_iter().skip(offset).collect()
    };

    let items: Vec<serde_json::Value> = agents
        .iter()
        .map(|e| enrich_agent_json(e, &dm, &catalog))
        .collect();

    Json(PaginatedResponse {
        items,
        total,
        offset,
        limit,
    })
    .into_response()
}

/// Hard cap on inlined text-attachment length (chars). Mirrors the PDF
/// truncation cap so a 5 MB `.log` paste doesn't blow the LLM context.
const MAX_TEXT_ATTACHMENT_CHARS: usize = 200_000;
const TEXT_TRUNCATION_MARKER: &str =
    "\n\n[…file truncated at 200K chars; content continues beyond this point…]";

/// Decide whether an attachment looks like a UTF-8 text/code/data file
/// the LLM can read directly. Browsers don't set `content_type` reliably
/// for code files (`.rs`, `.py` typically come through as empty or
/// `application/octet-stream`), so we fall back to extension matching.
fn is_text_like_attachment(content_type: &str, filename: &str) -> bool {
    if content_type.starts_with("text/") {
        return true;
    }
    let known_mime = matches!(
        content_type,
        "application/json"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "application/toml"
            | "application/x-toml"
            | "application/x-ipynb+json"
            | "application/javascript"
            | "application/x-javascript"
            | "application/typescript"
            | "application/sql"
            | "application/graphql"
    );
    if known_mime {
        return true;
    }
    let ext = filename
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        // Plain text & docs
        "txt" | "md" | "markdown" | "rst" | "csv" | "tsv" | "log"
        // Config & data
        | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "conf" | "cfg" | "env" | "properties"
        // Web
        | "html" | "htm" | "css" | "scss" | "sass" | "less"
        // JS/TS family
        | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "vue" | "svelte"
        // Other languages
        | "py" | "rs" | "go" | "java" | "kt" | "kts" | "swift" | "scala" | "clj" | "ex" | "exs"
        | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "m" | "mm"
        | "rb" | "php" | "pl" | "lua" | "r" | "jl" | "dart" | "zig" | "nim"
        // Shell
        | "sh" | "bash" | "zsh" | "fish" | "ps1"
        // Query / schema
        | "sql" | "graphql" | "gql" | "proto"
        // Notebooks
        | "ipynb"
        // Build files (no extension is rare; keep names like Dockerfile out — accept attribute can't match those)
        | "dockerfile" | "makefile"
    )
}

/// Resolve uploaded file attachments into content blocks.
///
/// Reads each file from the upload directory and produces blocks the
/// agent loop can consume:
///   - `image/*` → `ContentBlock::Image` (base64-encoded inline)
///   - `application/pdf` → `ContentBlock::Text` with a `[Attached PDF: <filename>]`
///     header followed by extracted plain text (truncated at 200K chars).
///     Scanned/image-only PDFs surface as a text note explaining no text
///     was extractable, so the LLM at least sees the attachment exists.
///   - text-like files (any `text/*`, `application/json|xml|yaml|toml|…`,
///     plus common code/data extensions) → `ContentBlock::Text` with a
///     `[Attached file: <filename>]` header. Read as UTF-8 lossy and
///     truncated at 200K chars.
///   - everything else → skipped with a warn log.
pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<librefang_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let (content_type, filename) = if let Some(ref m) = meta {
            (m.content_type.clone(), m.filename.clone())
        } else if !att.content_type.is_empty() {
            (att.content_type.clone(), att.file_id.clone())
        } else {
            continue; // Skip unknown attachments
        };

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);

        if content_type.starts_with("image/") {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        content_type = %content_type,
                        size_bytes = data.len(),
                        "Resolved image attachment into Image block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Image {
                        media_type: content_type,
                        data: b64,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read image upload");
                }
            }
        } else if content_type == "application/pdf" {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let header = format!("[Attached PDF: {} ({} bytes)]", filename, data.len());
                    let body = match librefang_runtime::pdf_text::extract_text_from_pdf(&data) {
                        Ok(text) => text,
                        Err(e) => {
                            tracing::warn!(
                                file_id = %att.file_id,
                                filename = %filename,
                                error = %e,
                                "PDF text extraction failed; surfacing as note to LLM"
                            );
                            format!("[Could not extract text: {e}]")
                        }
                    };
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        size_bytes = data.len(),
                        extracted_chars = body.chars().count(),
                        "Resolved PDF attachment into Text block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Text {
                        text: format!("{header}\n\n{body}"),
                        provider_metadata: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read PDF upload");
                }
            }
        } else if is_text_like_attachment(&content_type, &filename) {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let raw = String::from_utf8_lossy(&data);
                    let total_chars = raw.chars().count();
                    let (body, truncated) = if total_chars > MAX_TEXT_ATTACHMENT_CHARS {
                        let mut s: String = raw.chars().take(MAX_TEXT_ATTACHMENT_CHARS).collect();
                        s.push_str(TEXT_TRUNCATION_MARKER);
                        (s, true)
                    } else {
                        (raw.into_owned(), false)
                    };
                    let suffix = if truncated { ", truncated" } else { "" };
                    let header = format!(
                        "[Attached file: {} ({} bytes{})]",
                        filename,
                        data.len(),
                        suffix
                    );
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        content_type = %content_type,
                        size_bytes = data.len(),
                        kept_chars = body.chars().count(),
                        truncated,
                        "Resolved text attachment into Text block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Text {
                        text: format!("{header}\n\n{body}"),
                        provider_metadata: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read text upload");
                }
            }
        } else {
            tracing::warn!(
                file_id = %att.file_id,
                content_type = %content_type,
                filename = %filename,
                "Attachment type not yet wired into the agent loop; skipping"
            );
        }
    }

    blocks
}

/// Pre-insert attachment content blocks (image / extracted-text-from-PDF /
/// text files) into an agent's session so the LLM can see them.
///
/// Injects a single user-role message containing all blocks BEFORE the
/// kernel adds the user's text message, so the LLM receives:
/// `[..., User(attach_blocks), User(text)]`. session_repair will merge
/// those two consecutive user-role messages into one for the wire format.
pub fn inject_attachments_into_session(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    attachment_blocks: Vec<librefang_types::message::ContentBlock>,
) {
    use librefang_types::message::{Message, MessageContent, Role};

    let entry = match kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            tracing::warn!(agent_id = ?agent_id, "Cannot inject attachments: agent not found in registry");
            return;
        }
    };

    let mut session = match kernel.memory_substrate().get_session(entry.session_id) {
        Ok(Some(s)) => s,
        _ => librefang_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        },
    };

    let block_count = attachment_blocks.len();
    let block_kinds: Vec<&'static str> = attachment_blocks
        .iter()
        .map(|b| match b {
            librefang_types::message::ContentBlock::Image { .. } => "image",
            librefang_types::message::ContentBlock::Text { .. } => "text",
            librefang_types::message::ContentBlock::ImageFile { .. } => "image_file",
            librefang_types::message::ContentBlock::ToolUse { .. } => "tool_use",
            librefang_types::message::ContentBlock::ToolResult { .. } => "tool_result",
            librefang_types::message::ContentBlock::Thinking { .. } => "thinking",
            librefang_types::message::ContentBlock::Unknown => "unknown",
        })
        .collect();

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(attachment_blocks),
        pinned: false,
        timestamp: Some(chrono::Utc::now()),
    });

    let total_messages_after = session.messages.len();

    if let Err(e) = kernel.memory_substrate().save_session(&session) {
        tracing::warn!(
            agent_id = ?agent_id,
            session_id = ?entry.session_id,
            block_count,
            error = %e,
            "Failed to save session with attachment blocks"
        );
    } else {
        tracing::info!(
            agent_id = ?agent_id,
            session_id = ?entry.session_id,
            block_count,
            block_kinds = ?block_kinds,
            total_messages_after,
            "Injected attachment blocks into session"
        );
    }
}

/// Resolve URL-based attachments into image content blocks.
///
/// Downloads each attachment URL, base64-encodes images, and returns
/// content blocks ready to inject into a session. Non-image attachments
/// and download failures are skipped with a warning.
pub async fn resolve_url_attachments(
    attachments: &[librefang_types::comms::Attachment],
) -> Vec<librefang_types::message::ContentBlock> {
    use base64::Engine;

    let client = librefang_runtime::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP client build");
    let mut blocks = Vec::new();

    for att in attachments {
        // Determine MIME type from explicit field or guess from URL extension
        let content_type = if let Some(ref ct) = att.content_type {
            ct.clone()
        } else {
            mime_from_url(&att.url).unwrap_or_default()
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            tracing::debug!(url = %att.url, content_type, "Skipping non-image attachment");
            continue;
        }

        match client.get(&att.url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes().await {
                    Ok(data) => {
                        // Limit to 20MB to prevent OOM
                        if data.len() > 20 * 1024 * 1024 {
                            tracing::warn!(url = %att.url, size = data.len(), "Attachment too large, skipping");
                            continue;
                        }
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        blocks.push(librefang_types::message::ContentBlock::Image {
                            media_type: content_type,
                            data: b64,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(url = %att.url, error = %e, "Failed to read attachment body");
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(url = %att.url, status = %resp.status(), "Attachment download failed");
            }
            Err(e) => {
                tracing::warn!(url = %att.url, error = %e, "Failed to fetch attachment URL");
            }
        }
    }

    blocks
}

/// Guess MIME type from a URL file extension.
fn mime_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let ext = path.rsplit('.').next()?;
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg".into()),
        "png" => Some("image/png".into()),
        "gif" => Some("image/gif".into()),
        "webp" => Some("image/webp".into()),
        "svg" => Some("image/svg+xml".into()),
        _ => None,
    }
}

/// POST /api/agents/:id/message — Send a message to an agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/message",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::MessageRequest,
    responses(
        (status = 200, description = "Message response", body = crate::types::MessageResponse),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    // Pre-translate error messages before the `.await` point below.
    // `ErrorTranslator` wraps a `FluentBundle` which is `!Send`, so it must
    // not be held across an await boundary (axum requires `Send` futures).
    let l = super::resolve_lang(lang.as_ref());
    let (err_invalid_id, err_too_large, err_not_found, err_auth_missing) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-message-too-large"),
            t.t("api-error-agent-not-found"),
            t.t("api-error-auth-missing"),
        )
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            );
        }
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": err_too_large})),
        );
    }

    // Check agent exists before processing
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        );
    }

    // Reject messages when the agent's provider has no API key configured
    {
        let registry = state.kernel.agent_registry();
        if let Some(entry) = registry.get(agent_id) {
            let dm = {
                let dm_override = state
                    .kernel
                    .default_model_override_ref()
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                effective_default_model(
                    &state.kernel.config_ref().default_model,
                    dm_override.as_ref(),
                )
            };
            let provider = if entry.manifest.model.provider.is_empty()
                || entry.manifest.model.provider == "default"
            {
                &dm.provider
            } else {
                &entry.manifest.model.provider
            };
            if let Some(catalog) = state.kernel.model_catalog_ref().read().ok().as_ref() {
                if let Some(p) = catalog.get_provider(provider) {
                    if !p.auth_status.is_available() {
                        return (
                            StatusCode::PRECONDITION_FAILED,
                            Json(
                                serde_json::json!({"error": format!("{} (provider: {})", err_auth_missing, provider)}),
                            ),
                        );
                    }
                }
            }
        }
    }

    // Resolve file attachments into image content blocks
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    // Detect ephemeral mode: explicit flag OR `/btw ` prefix in the message text
    let (effective_message, is_ephemeral) = if req.ephemeral {
        (req.message.clone(), true)
    } else if let Some(stripped) = req.message.strip_prefix("/btw ") {
        (stripped.to_string(), true)
    } else {
        (req.message.clone(), false)
    };

    let thinking_override = req.thinking;
    let show_thinking = req.show_thinking.unwrap_or(true);

    // Parse optional explicit session_id override from the request body.
    let session_id_override = match req.session_id.as_deref() {
        None => None,
        Some(s) => match s.parse::<uuid::Uuid>() {
            Ok(id) => Some(librefang_types::agent::SessionId(id)),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "invalid session_id: must be a UUID"})),
                );
            }
        },
    };

    let result = if is_ephemeral {
        // Ephemeral "side question" — use a temp session, no persistence
        state
            .kernel
            .send_message_ephemeral(agent_id, &effective_message)
            .await
    } else {
        let sender_context = request_sender_context(&req);
        let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
        state
            .kernel
            .send_message_with_session_override(
                agent_id,
                &effective_message,
                Some(kernel_handle),
                sender_context.as_ref(),
                thinking_override,
                session_id_override,
            )
            .await
    };

    match result {
        Ok(result) => {
            // When the agent intentionally chose not to reply (NO_REPLY / [[silent]]),
            // return an empty response with the silent flag so callers can distinguish
            // intentional silence from a bug.
            if result.silent {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "response": "",
                        "silent": true,
                        "input_tokens": result.total_usage.input_tokens,
                        "output_tokens": result.total_usage.output_tokens,
                        "iterations": result.iterations,
                        "cost_usd": result.cost_usd,
                    })),
                );
            }

            // Extract reasoning trace (optional) and strip <think>...</think>
            // blocks from the final model output.
            let thinking_trace = if show_thinking {
                crate::ws::extract_think_content(&result.response)
            } else {
                None
            };
            let cleaned = crate::ws::strip_think_tags(&result.response);

            // Guard: ensure we never return an empty response to the client
            let response = if cleaned.trim().is_empty() {
                format!(
                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                cleaned
            };
            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                    decision_traces: result.decision_traces,
                    memories_saved: result.memories_saved,
                    memories_used: result.memories_used,
                    memory_conflicts: result.memory_conflicts,
                    thinking: thinking_trace,
                    owner_notice: result.owner_notice,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            let status = if format!("{e}").contains("Agent not found") {
                StatusCode::NOT_FOUND
            } else if format!("{e}").contains("quota") || format!("{e}").contains("Quota") {
                StatusCode::TOO_MANY_REQUESTS
            } else if format!("{e}").contains("belongs to a different agent") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, {
                let t = ErrorTranslator::new(l);
                Json(
                    serde_json::json!({"error": t.t_args("api-error-message-delivery-failed", &[("reason", &e.to_string())])}),
                )
            })
        }
    }
}

fn request_sender_context(req: &MessageRequest) -> Option<SenderContext> {
    let sender_id = req.sender_id.as_ref()?;
    Some(SenderContext {
        channel: req
            .channel_type
            .clone()
            .unwrap_or_else(|| "api".to_string()),
        user_id: sender_id.clone(),
        display_name: req.sender_name.clone().unwrap_or_else(|| sender_id.clone()),
        is_group: req.is_group,
        was_mentioned: req.was_mentioned,
        thread_id: None,
        account_id: None,
        // Phase 2 §C — forward the optional group participant roster from the
        // gateway POST body so the addressee guard can fire downstream. Empty
        // when the caller (Telegram, direct API) doesn't populate it; the
        // guard then becomes a no-op and cannot produce false positives.
        group_participants: req.group_participants.clone().unwrap_or_default(),
        ..Default::default()
    })
}

/// GET /api/agents/:id/session — Get agent session (conversation history).
#[utoipa::path(
    get,
    path = "/api/agents/{id}/session",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get agent conversation session history", body = serde_json::Value)
    )
)]
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    match state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
    {
        Ok(Some(session)) => {
            // Two-pass approach: ToolUse blocks live in Assistant messages while
            // ToolResult blocks arrive in subsequent User messages.  Pass 1
            // collects all tool_use entries keyed by id; pass 2 attaches results.

            // Pass 1: build messages and a lookup from tool_use_id → (msg_idx, tool_idx)
            use base64::Engine as _;
            let mut built_messages: Vec<serde_json::Value> = Vec::new();
            let mut tool_use_index: std::collections::HashMap<String, (usize, usize)> =
                std::collections::HashMap::new();

            for m in &session.messages {
                let mut tools: Vec<serde_json::Value> = Vec::new();
                let mut msg_images: Vec<serde_json::Value> = Vec::new();
                let content = match &m.content {
                    librefang_types::message::MessageContent::Text(t) => t.clone(),
                    librefang_types::message::MessageContent::Blocks(blocks) => {
                        let mut texts = Vec::new();
                        for b in blocks {
                            match b {
                                librefang_types::message::ContentBlock::Text { text, .. } => {
                                    texts.push(text.clone());
                                }
                                librefang_types::message::ContentBlock::Image {
                                    media_type,
                                    data,
                                } => {
                                    texts.push("[Image]".to_string());
                                    // Persist image to upload dir so it can be
                                    // served back when loading session history.
                                    let file_id = uuid::Uuid::new_v4().to_string();
                                    let upload_dir = std::env::temp_dir().join("librefang_uploads");
                                    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
                                        tracing::warn!("Failed to create upload directory: {e}");
                                    }
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(data)
                                    {
                                        if let Err(e) =
                                            std::fs::write(upload_dir.join(&file_id), &bytes)
                                        {
                                            tracing::warn!("Failed to write upload file: {e}");
                                        }
                                        UPLOAD_REGISTRY.insert(
                                            file_id.clone(),
                                            UploadMeta {
                                                filename: format!(
                                                    "image.{}",
                                                    media_type.rsplit('/').next().unwrap_or("png")
                                                ),
                                                content_type: media_type.clone(),
                                            },
                                        );
                                        msg_images.push(serde_json::json!({
                                            "file_id": file_id,
                                            "filename": format!("image.{}", media_type.rsplit('/').next().unwrap_or("png")),
                                        }));
                                    }
                                }
                                librefang_types::message::ContentBlock::ToolUse {
                                    id,
                                    name,
                                    input,
                                    ..
                                } => {
                                    let tool_idx = tools.len();
                                    tools.push(serde_json::json!({
                                        "name": name,
                                        "input": input,
                                        "running": false,
                                        "expanded": false,
                                    }));
                                    // Will be filled after this loop when we know msg_idx
                                    tool_use_index.insert(id.clone(), (usize::MAX, tool_idx));
                                }
                                // ToolResult blocks are handled in pass 2
                                librefang_types::message::ContentBlock::ToolResult { .. } => {}
                                _ => {}
                            }
                        }
                        texts.join("\n")
                    }
                };
                // Skip messages that are purely tool results (User role with only ToolResult blocks)
                if content.is_empty() && tools.is_empty() {
                    continue;
                }
                let msg_idx = built_messages.len();
                // Fix up the msg_idx for tool_use entries registered with sentinel
                for (_, (mi, _)) in tool_use_index.iter_mut() {
                    if *mi == usize::MAX {
                        *mi = msg_idx;
                    }
                }
                let mut msg = serde_json::json!({
                    "role": format!("{:?}", m.role),
                    "content": content,
                });
                if !tools.is_empty() {
                    msg["tools"] = serde_json::Value::Array(tools);
                }
                if !msg_images.is_empty() {
                    msg["images"] = serde_json::Value::Array(msg_images);
                }
                // Expose the real message timestamp so the dashboard can
                // render historical times correctly on resume instead of
                // falling back to render-time `Date.now()` (#2934). Serialized
                // as RFC 3339; messages persisted before the field existed
                // come through as `null`.
                if let Some(ts) = m.timestamp {
                    msg["timestamp"] = serde_json::Value::String(ts.to_rfc3339());
                }
                built_messages.push(msg);
            }

            // Pass 2: walk messages again and attach ToolResult to the correct tool
            for m in &session.messages {
                if let librefang_types::message::MessageContent::Blocks(blocks) = &m.content {
                    for b in blocks {
                        if let librefang_types::message::ContentBlock::ToolResult {
                            tool_use_id,
                            content: result,
                            is_error,
                            ..
                        } = b
                        {
                            if let Some(&(msg_idx, tool_idx)) = tool_use_index.get(tool_use_id) {
                                if let Some(msg) = built_messages.get_mut(msg_idx) {
                                    if let Some(tools_arr) =
                                        msg.get_mut("tools").and_then(|v| v.as_array_mut())
                                    {
                                        if let Some(tool_obj) = tools_arr.get_mut(tool_idx) {
                                            // Cap at 100 KB to keep session responses manageable
                                            let capped: String =
                                                result.chars().take(102_400).collect();
                                            tool_obj["result"] = serde_json::Value::String(capped);
                                            tool_obj["is_error"] =
                                                serde_json::Value::Bool(*is_error);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let messages = built_messages;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0,
                "context_window_tokens": 0,
                "messages": [],
            })),
        ),
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-session-load-failed")})),
            )
        }
    }
}

/// DELETE /api/agents/:id — Kill an agent.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent killed"),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Hand-spawned runtime agents are owned by their hand instance. Killing
    // one directly leaves the hand registry pointing at a dangling id that
    // can respawn or produce stale instance state — require callers to
    // deactivate or uninstall the owning hand instead. The dashboard hides
    // Delete for hand agents already; this closes the direct-API loophole.
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if entry.is_hand {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "Cannot delete a hand-spawned agent directly; deactivate or uninstall the owning hand instead."
                })),
            );
        }
    }

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        ),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found-or-terminated")})),
            )
        }
    }
}

/// PUT /api/agents/:id/suspend — Suspend an agent (stops cron, keeps in registry).
#[utoipa::path(put, path = "/api/agents/{id}/suspend", tag = "agents", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent suspended")))]
pub async fn suspend_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.suspend_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "suspended", "agent_id": id})),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// PUT /api/agents/:id/resume — Resume a suspended agent.
#[utoipa::path(put, path = "/api/agents/{id}/resume", tag = "agents", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent resumed")))]
pub async fn resume_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.resume_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "running", "agent_id": id})),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// PUT /api/agents/:id/mode — Change an agent's operational mode.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/mode",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = SetModeRequest, description = "New agent mode"),
    responses(
        (status = 200, description = "Change an agent's operational mode", body = serde_json::Value)
    )
)]
pub async fn set_agent_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    match state.kernel.agent_registry().set_mode(agent_id, body.mode) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "agent_id": id,
                "mode": body.mode,
            })),
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------

/// GET /api/agents/:id — Get a single agent's detailed info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent details", body = serde_json::Value),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let dm = {
        let dm_override = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        effective_default_model(
            &state.kernel.config_ref().default_model,
            dm_override.as_ref(),
        )
    };
    let resolved_provider =
        if entry.manifest.model.provider.is_empty() || entry.manifest.model.provider == "default" {
            dm.provider.as_str()
        } else {
            entry.manifest.model.provider.as_str()
        };
    let resolved_model =
        if entry.manifest.model.model.is_empty() || entry.manifest.model.model == "default" {
            dm.model.as_str()
        } else {
            entry.manifest.model.model.as_str()
        };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "is_hand": entry.is_hand,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "last_active": entry.last_active.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": resolved_provider,
                "model": resolved_model,
                "max_tokens": entry.manifest.model.max_tokens,
                "temperature": entry.manifest.model.temperature,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
            "system_prompt": entry.manifest.model.system_prompt,
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "skills": entry.manifest.skills,
            "skills_mode": skill_assignment_mode(&entry.manifest),
            "skills_disabled": entry.manifest.skills_disabled,
            "tools_disabled": entry.manifest.tools_disabled,
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
            "fallback_models": entry.manifest.fallback_models,
            "web_search_augmentation": entry.manifest.web_search_augmentation,
        })),
    )
}

/// POST /api/agents/:id/message/stream — SSE streaming response.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/message/stream",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::MessageRequest,
    responses(
        (status = 200, description = "Streaming message response (SSE)")
    )
)]
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use librefang_runtime::llm_driver::StreamEvent;

    let (err_too_large, err_invalid_id, err_not_found, err_streaming_failed) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-message-too-large"),
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-agent-not-found"),
            t.t("api-error-message-streaming-failed"),
        )
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": err_too_large})),
        )
            .into_response();
    }

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
                .into_response();
        }
    };

    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        )
            .into_response();
    }

    // Resolve file attachments into image content blocks (same as non-streaming)
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    // Parse optional explicit session_id override from the request body.
    let session_id_override = match req.session_id.as_deref() {
        None => None,
        Some(s) => match s.parse::<uuid::Uuid>() {
            Ok(id) => Some(librefang_types::agent::SessionId(id)),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "invalid session_id: must be a UUID"})),
                )
                    .into_response();
            }
        },
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle) = match state
        .kernel
        .send_message_streaming_with_routing_and_session_override(
            agent_id,
            &req.message,
            Some(kernel_handle),
            session_id_override,
        )
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("Streaming message failed for agent {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": err_streaming_failed})),
            )
                .into_response();
        }
    };

    // Defense against the agent loop emitting the same text span twice in a
    // single streaming turn (observed when multi-iteration loops re-assert a
    // final sentence after a tool step). The dedup window is per-request, so
    // legitimate repetitions across turns stay unaffected.
    let sse_stream = stream::unfold((rx, StreamDedup::new()), |(mut rx, mut dedup)| async move {
        loop {
            let event = rx.recv().await?;
            let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                StreamEvent::TextDelta { text } => {
                    if dedup.is_duplicate(&text) {
                        tracing::debug!(
                            len = text.len(),
                            preview = %text.chars().take(40).collect::<String>(),
                            "stream dedup: dropping duplicate TextDelta",
                        );
                        continue;
                    }
                    dedup.record_sent(&text);
                    Event::default()
                        .event("chunk")
                        .json_data(serde_json::json!({"content": text, "done": false}))
                        .unwrap_or_else(|_| Event::default().data("error"))
                }
                StreamEvent::ToolUseStart { name, .. } => Event::default()
                    .event("tool_use")
                    .json_data(serde_json::json!({"tool": name}))
                    .unwrap_or_else(|_| Event::default().data("error")),
                StreamEvent::ToolUseEnd { name, input, .. } => Event::default()
                    .event("tool_result")
                    .json_data(serde_json::json!({"tool": name, "input": input}))
                    .unwrap_or_else(|_| Event::default().data("error")),
                StreamEvent::ContentComplete { usage, .. } => Event::default()
                    .event("done")
                    .json_data(serde_json::json!({
                        "done": true,
                        "usage": {
                            "input_tokens": usage.input_tokens,
                            "output_tokens": usage.output_tokens,
                        }
                    }))
                    .unwrap_or_else(|_| Event::default().data("error")),
                StreamEvent::PhaseChange { phase, detail } => Event::default()
                    .event("phase")
                    .json_data(serde_json::json!({
                        "phase": phase,
                        "detail": detail,
                    }))
                    .unwrap_or_else(|_| Event::default().data("error")),
                StreamEvent::OwnerNotice { text } => Event::default()
                    .event("owner_notice")
                    .json_data(serde_json::json!({ "text": text }))
                    .unwrap_or_else(|_| Event::default().data("error")),
                _ => Event::default().comment("skip"),
            });
            return Some((sse_event, (rx, dedup)));
        }
    });

    Sse::new(sse_stream).into_response()
}

#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List all sessions for an agent", body = serde_json::Value)
    )
)]
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// POST /api/agents/{id}/sessions — Create a new session for an agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Optional label for the new session"),
    responses(
        (status = 200, description = "Create a new session for an agent", body = serde_json::Value)
    )
)]
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/switch — Switch to an existing session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions/{session_id}/switch",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to switch to"),
    ),
    responses(
        (status = 200, description = "Switch to an existing session", body = serde_json::Value)
    )
)]
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            )
        }
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ── Session Export / Import (Hibernation) ───────────────────────────────

/// GET /api/agents/{id}/sessions/{session_id}/export — Export a session for hibernation.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/sessions/{session_id}/export",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("session_id" = String, Path, description = "Session ID to export"),
    ),
    responses(
        (status = 200, description = "Exported session data", body = serde_json::Value)
    )
)]
pub async fn export_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            )
        }
    };
    match state.kernel.export_session(agent_id, session_id) {
        Ok(export) => (
            StatusCode::OK,
            Json(serde_json::to_value(export).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// POST /api/agents/{id}/sessions/import — Import a previously exported session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/sessions/import",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Exported session JSON"),
    responses(
        (status = 200, description = "Session imported successfully", body = serde_json::Value)
    )
)]
pub async fn import_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let export: librefang_memory::session::SessionExport = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid export format: {e}")})),
            )
        }
    };
    match state.kernel.import_session(agent_id, export) {
        Ok(new_session_id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "session_id": new_session_id.0.to_string(),
                "message": "Session imported successfully"
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ── Extended Chat Command API Endpoints ─────────────────────────────────

/// POST /api/agents/{id}/session/reset — Reset an agent's session.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/reset",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Reset an agent's current session", body = serde_json::Value)
    )
)]
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// POST /api/agents/{id}/session/reboot — Hard-reboot an agent's session (full clear, no summary).
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/reboot",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Hard-reboot an agent's session without saving summary", body = serde_json::Value)
    )
)]
pub async fn reboot_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    match state.kernel.reboot_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "message": "Session rebooted. Context cleared."}),
            ),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// DELETE /api/agents/{id}/history — Clear ALL conversation history for an agent.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/history",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Clear all conversation history for an agent", body = serde_json::Value)
    )
)]
pub async fn clear_agent_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
    match state.kernel.clear_agent_history(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// POST /api/agents/{id}/session/compact — Trigger LLM session compaction.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/session/compact",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Trigger LLM session compaction", body = serde_json::Value)
    )
)]
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let err_invalid_id = {
        let t = ErrorTranslator::new(l);
        t.t("api-error-agent-invalid-id")
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            )
        }
    };
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => {
            let t = ErrorTranslator::new(l);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            )
        }
    }
}

/// POST /api/agents/{id}/stop — Cancel an agent's current LLM run.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/stop",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Cancel an agent's current LLM run", body = serde_json::Value)
    )
)]
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

#[utoipa::path(
    put,
    path = "/api/agents/{id}/model",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Model name and optional provider"),
    responses(
        (status = 200, description = "Change an agent's LLM model", body = serde_json::Value)
    )
)]
pub async fn set_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-missing-model")})),
            )
        }
    };
    let explicit_provider = body["provider"].as_str();
    // Check agent exists — kernel returns a generic error for missing
    // agents that the match arm below would wrap as 500. Validate up
    // front so the caller gets a 404 for the common case.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
    match state
        .kernel
        .set_agent_model(agent_id, model, explicit_provider)
    {
        Ok(()) => {
            // Return the resolved model+provider so frontend stays in sync.
            // The model name may have been normalized (provider prefix stripped),
            // so we read it back from the registry instead of echoing the raw input.
            let (resolved_model, resolved_provider) = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| {
                    (
                        e.manifest.model.model.clone(),
                        e.manifest.model.provider.clone(),
                    )
                })
                .unwrap_or_else(|| (model.to_string(), String::new()));
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "ok", "model": resolved_model, "provider": resolved_provider}),
                ),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// GET /api/agents/{id}/traces — Get decision traces from the agent's most recent message.
///
/// Returns structured traces showing why each tool was selected during the last
/// agent loop execution. Useful for debugging, auditing, and optimization.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/traces",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get decision traces from the agent's most recent message", body = serde_json::Value)
    )
)]
pub async fn get_agent_traces(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };

    // Check agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let traces = state
        .kernel
        .traces()
        .get(&agent_id)
        .map(|entry| entry.value().clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({ "traces": traces })),
    )
}

/// GET /api/agents/{id}/tools — Get an agent's tool allowlist/blocklist.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's tool allowlist and blocklist", body = serde_json::Value)
    )
)]
pub async fn get_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
            "disabled": entry.manifest.tools_disabled,
        })),
    )
}

/// PUT /api/agents/{id}/tools — Update an agent's tool allowlist/blocklist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Tool allowlist and/or blocklist arrays"),
    responses(
        (status = 200, description = "Update an agent's tool allowlist and blocklist", body = serde_json::Value)
    )
)]
pub async fn set_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let allowlist = body
        .get("tool_allowlist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let blocklist = body
        .get("tool_blocklist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

    if allowlist.is_none() && blocklist.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-agent-missing-tools")})),
        );
    }

    // Check agent exists — kernel returns a generic error for missing
    // agents that the match arm below would wrap as 500. Validate up
    // front so the caller gets a 404 for the common case.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    match state
        .kernel
        .set_agent_tool_filters(agent_id, allowlist, blocklist)
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ── Per-Agent Skill & MCP Endpoints ────────────────────────────────────

/// GET /api/agents/{id}/skills — Get an agent's skill assignment info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/skills",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's skill assignment info", body = serde_json::Value)
    )
)]
pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": skill_assignment_mode(&entry.manifest),
            "disabled": entry.manifest.skills_disabled,
        })),
    )
}

/// PUT /api/agents/{id}/skills — Update an agent's skill allowlist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/skills",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Array of skill names"),
    responses(
        (status = 200, description = "Update an agent's skill allowlist", body = serde_json::Value)
    )
)]
pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let skills: Vec<String> = body["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "skills": skills})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers — Get an agent's MCP server assignment info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/mcp_servers",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's MCP server assignment info", body = serde_json::Value)
    )
)]
pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools_ref().lock() {
        let configured_servers: Vec<String> = state
            .kernel
            .effective_mcp_servers_ref()
            .read()
            .map(|servers| servers.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = librefang_runtime::mcp::resolve_mcp_server_from_known(
                &tool.name,
                configured_servers.iter().map(String::as_str),
            ) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = if entry.manifest.mcp_servers.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/mcp_servers",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Array of MCP server names"),
    responses(
        (status = 200, description = "Update an agent's MCP server allowlist", body = serde_json::Value)
    )
)]
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let servers: Vec<String> = body["mcp_servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/agents/:id — Update an agent (currently: re-set manifest fields).
#[utoipa::path(
    put,
    path = "/api/agents/{id}/update",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = AgentUpdateRequest, description = "New agent manifest TOML"),
    responses(
        (status = 200, description = "Update an agent's manifest", body = serde_json::Value)
    )
)]
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    // Parse the new manifest
    let _manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-invalid-manifest", &[("error", &e.to_string())])}),
                ),
            );
        }
    };

    // Note: Full manifest update requires kill + respawn. For now, acknowledge receipt.
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "acknowledged",
            "agent_id": id,
            "note": "Full manifest update requires agent restart. Use DELETE + POST to apply.",
        })),
    )
}

#[utoipa::path(
    patch,
    path = "/api/agents/{id}",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = serde_json::Value, description = "Partial agent fields to update"),
    responses(
        (status = 200, description = "Partially update an agent (name, description, model, system prompt)", body = serde_json::Value)
    )
)]
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    // Apply partial updates using dedicated registry methods
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_name(agent_id, name.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_description(agent_id, desc.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        let explicit_provider = body.get("provider").and_then(|v| v.as_str());
        if let Err(e) = state
            .kernel
            .set_agent_model(agent_id, model, explicit_provider)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(system_prompt) = body.get("system_prompt").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_system_prompt(agent_id, system_prompt.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(mcp_servers) = match patch_agent_mcp_servers(&body) {
        Ok(servers) => servers,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error})),
            );
        }
    } {
        if let Err(e) = state.kernel.set_agent_mcp_servers(agent_id, mcp_servers) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }

    // Persist updated entry to SQLite
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
            tracing::warn!("Failed to persist agent state: {e}");
        }
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "agent_id": entry.id.to_string(), "name": entry.name}),
            ),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": t.t("api-error-agent-vanished")})),
        )
    }
}

fn patch_agent_mcp_servers(body: &serde_json::Value) -> Result<Option<Vec<String>>, &'static str> {
    let raw = body.get("mcp_servers").or_else(|| {
        body.get("capabilities")
            .and_then(|caps| caps.get("mcp_servers"))
    });

    let Some(raw) = raw else {
        return Ok(None);
    };

    let items = raw
        .as_array()
        .ok_or("mcp_servers must be an array of strings")?;

    let mut servers = Vec::with_capacity(items.len());
    for item in items {
        let name = item
            .as_str()
            .ok_or("mcp_servers must be an array of strings")?;
        servers.push(name.to_string());
    }

    Ok(Some(servers))
}

// ---------------------------------------------------------------------------
// Agent Identity endpoint
// ---------------------------------------------------------------------------

/// Request body for updating agent visual identity.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/identity — Update an agent's visual identity.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}/identity",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = UpdateIdentityRequest, description = "Identity fields to update"),
    responses(
        (status = 200, description = "Update an agent's visual identity", body = serde_json::Value)
    )
)]
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-color-invalid")})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-avatar-invalid")})),
            );
        }
    }

    let identity = AgentIdentity {
        emoji: req.emoji,
        avatar_url: req.avatar_url,
        color: req.color,
        archetype: req.archetype,
        vibe: req.vibe,
        greeting_style: req.greeting_style,
    };

    match state
        .kernel
        .agent_registry()
        .update_identity(agent_id, identity)
    {
        Ok(()) => {
            // Persist identity to SQLite
            if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
                if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
                    tracing::warn!("Failed to persist agent state: {e}");
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "agent_id": id})),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Config Hot-Update
// ---------------------------------------------------------------------------

/// Request body for patching agent config (name, description, prompt, identity, model).
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    /// Maximum tokens for LLM response. Controls conversation window size.
    pub max_tokens: Option<u32>,
    /// Sampling temperature (0.0–2.0). Lower values are more deterministic.
    pub temperature: Option<f32>,
    #[schema(value_type = Option<Vec<serde_json::Value>>)]
    pub fallback_models: Option<Vec<librefang_types::agent::FallbackModel>>,
    /// Web search augmentation mode: "off", "auto", or "always".
    #[schema(value_type = Option<String>)]
    pub web_search_augmentation: Option<librefang_types::agent::WebSearchAugmentationMode>,
}

/// PATCH /api/agents/{id}/config — Hot-update agent name, description, system prompt, and identity.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}/config",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = PatchAgentConfigRequest, description = "Agent config fields to update"),
    responses(
        (status = 200, description = "Hot-update agent name, description, system prompt, identity, and model", body = serde_json::Value)
    )
)]
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Input length limits
    const MAX_NAME_LEN: usize = 256;
    const MAX_DESC_LEN: usize = 4096;
    const MAX_PROMPT_LEN: usize = 65_536;

    if let Some(ref name) = req.name {
        if name.len() > MAX_NAME_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-name-too-long", &[("max", &MAX_NAME_LEN.to_string())])}),
                ),
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-desc-too-long", &[("max", &MAX_DESC_LEN.to_string())])}),
                ),
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-prompt-too-long", &[("max", &MAX_PROMPT_LEN.to_string())])}),
                ),
            );
        }
    }

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-color-invalid")})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-avatar-invalid")})),
            );
        }
    }

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .agent_registry()
                .update_name(agent_id, new_name.clone())
            {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    // Update description
    if let Some(ref new_desc) = req.description {
        if state
            .kernel
            .agent_registry()
            .update_description(agent_id, new_desc.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message)
    if let Some(ref new_prompt) = req.system_prompt {
        if state
            .kernel
            .agent_registry()
            .update_system_prompt(agent_id, new_prompt.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update identity fields (merge — only overwrite provided fields)
    let has_identity_field = req.emoji.is_some()
        || req.avatar_url.is_some()
        || req.color.is_some()
        || req.archetype.is_some()
        || req.vibe.is_some()
        || req.greeting_style.is_some();

    if has_identity_field {
        // Read current identity, merge with provided fields
        let current = state
            .kernel
            .agent_registry()
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = AgentIdentity {
            emoji: req.emoji.or(current.emoji),
            avatar_url: req.avatar_url.or(current.avatar_url),
            color: req.color.or(current.color),
            archetype: req.archetype.or(current.archetype),
            vibe: req.vibe.or(current.vibe),
            greeting_style: req.greeting_style.or(current.greeting_style),
        };
        if state
            .kernel
            .agent_registry()
            .update_identity(agent_id, merged)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update model/provider — always go through set_agent_model so that
    // provider-change semantics (prefix stripping, canonical-session cleanup,
    // and clearing of stale per-agent api_key_env / base_url overrides) are
    // applied uniformly. Bypassing it via update_model_and_provider was the
    // root cause of #2380: switching to a non-default provider via the
    // dashboard left stale CLOUDVERSE_API_KEY / cloudverse base_url on the
    // manifest, so the new provider's request was sent to the old URL with
    // the old credentials and rejected with "Missing Authentication header".
    if let Some(ref new_model) = req.model {
        if !new_model.is_empty() {
            let explicit_provider = req.provider.as_deref().filter(|p| !p.is_empty());
            if let Err(e) = state
                .kernel
                .set_agent_model(agent_id, new_model, explicit_provider)
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    // Validate and update temperature (sampling randomness)
    if let Some(temperature) = req.temperature {
        if !(0.0..=2.0).contains(&temperature) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "temperature must be between 0.0 and 2.0"})),
            );
        }
        if state
            .kernel
            .agent_registry()
            .update_temperature(agent_id, temperature)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update max_tokens (response length / conversation window limit)
    if let Some(max_tokens) = req.max_tokens {
        if max_tokens == 0 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "max_tokens must be greater than 0"})),
            );
        }
        if state
            .kernel
            .agent_registry()
            .update_max_tokens(agent_id, max_tokens)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update fallback model chain
    if let Some(fallbacks) = req.fallback_models {
        if state
            .kernel
            .agent_registry()
            .update_fallback_models(agent_id, fallbacks)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update web search augmentation mode
    if let Some(mode) = req.web_search_augmentation {
        if state
            .kernel
            .agent_registry()
            .update_web_search_augmentation(agent_id, mode)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Persist updated manifest to database so changes survive restart
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
            tracing::warn!("Failed to persist agent config update: {e}");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
}

// ---------------------------------------------------------------------------
// Agent Cloning
// ---------------------------------------------------------------------------

/// Request body for cloning an agent.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct CloneAgentRequest {
    pub new_name: String,
    /// Whether to copy skills from the source agent (default: true).
    #[serde(default = "default_clone_true")]
    pub include_skills: bool,
    /// Whether to copy tools from the source agent (default: true).
    #[serde(default = "default_clone_true")]
    pub include_tools: bool,
}

fn default_clone_true() -> bool {
    true
}

fn apply_clone_inclusion_flags(
    manifest: &mut librefang_types::agent::AgentManifest,
    req: &CloneAgentRequest,
) {
    if !req.include_skills {
        manifest.skills.clear();
        manifest.skills_disabled = true;
    }
    if !req.include_tools {
        manifest.tools.clear();
        manifest.tool_allowlist.clear();
        manifest.tool_blocklist.clear();
        manifest.tools_disabled = true;
    }
}

fn skill_assignment_mode(manifest: &librefang_types::agent::AgentManifest) -> &'static str {
    if manifest.skills_disabled {
        "none"
    } else if manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    }
}

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/clone",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = CloneAgentRequest, description = "New name for the cloned agent"),
    responses(
        (status = 200, description = "Clone an agent with its workspace files", body = serde_json::Value)
    )
)]
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    if req.new_name.len() > 256 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": t.t_args("api-error-agent-name-too-long", &[("max", "256")])}),
            ),
        );
    }

    if req.new_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-agent-name-empty")})),
        );
    }

    let source = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Conditionally strip skills and tools based on request flags.
    apply_clone_inclusion_flags(&mut cloned_manifest, &req);

    // Spawn the cloned agent
    let new_id = match state.kernel.spawn_agent(cloned_manifest) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-clone-failed", &[("error", &e.to_string())])}),
                ),
            );
        }
    };

    // Copy workspace identity files from source to destination
    let new_entry = state.kernel.agent_registry().get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                let src_identity = src_can.join(".identity");
                let dst_identity = dst_can.join(".identity");
                let _ = std::fs::create_dir_all(&dst_identity);
                for &fname in KNOWN_IDENTITY_FILES {
                    // Source: prefer .identity/ (post-migration), fall back to workspace root
                    let src_file = if src_identity.join(fname).exists() {
                        src_identity.join(fname)
                    } else {
                        src_can.join(fname)
                    };
                    let dst_file = dst_identity.join(fname);
                    if src_file.exists() {
                        if let Err(e) = std::fs::copy(&src_file, &dst_file) {
                            tracing::warn!("Failed to copy identity file {fname}: {e}");
                        }
                    }
                }
            }
        }
    }

    // Copy identity from source
    if let Err(e) = state
        .kernel
        .agent_registry()
        .update_identity(new_id, source.identity.clone())
    {
        tracing::warn!("Failed to copy agent identity: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
}

/// POST /api/agents/{id}/reload — Re-read the agent's agent.toml from disk.
///
/// Picks up manual edits to fields like `skills`, `mcp_servers`, `tools`,
/// or `system_prompt` without restarting the daemon. Runtime-only fields
/// (workspace path, tags) are preserved.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/reload",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent manifest reloaded from agent.toml", body = serde_json::Value)
    )
)]
pub async fn reload_agent_manifest(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    match state.kernel.reload_agent_from_disk(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "reloaded", "agent_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Workspace File Editor endpoints
// ---------------------------------------------------------------------------

/// Whitelisted workspace identity files that can be read/written via API.
const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// GET /api/agents/{id}/files — List workspace identity files.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/files",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List workspace identity files for an agent", body = serde_json::Value)
    )
)]
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        // Check .identity/ first (current layout), then workspace root (pre-migration fallback)
        let identity_path = workspace.join(".identity").join(name);
        let path = if identity_path.exists() {
            identity_path
        } else {
            workspace.join(name)
        };
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    responses(
        (status = 200, description = "Read a workspace identity file", body = serde_json::Value)
    )
)]
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    // Resolve canonical path: prefer .identity/ (current layout), fall back to workspace root
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    let identity_path = workspace.join(".identity").join(&filename);
    let file_path = if identity_path.exists() {
        identity_path
    } else {
        workspace.join(&filename)
    };

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };

    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    request_body(content = SetAgentFileRequest, description = "File content to write"),
    responses(
        (status = 200, description = "Write a workspace identity file", body = serde_json::Value)
    )
)]
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": t.t("api-error-file-too-large")})),
        );
    }

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    // Always write to .identity/ (current layout)
    let identity_dir = workspace.join(".identity");
    if let Err(e) = std::fs::create_dir_all(&identity_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-file-write-failed", &[("error", &e.to_string())])}),
            ),
        );
    }
    let file_path = identity_dir.join(&filename);

    // Security: ensure .identity/ is inside the workspace
    let check_path = identity_dir
        .canonicalize()
        .map(|p| p.join(&filename))
        .unwrap_or_else(|_| file_path.clone());
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = identity_dir.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-file-write-failed", &[("error", &e.to_string())])}),
            ),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        if let Err(e) = std::fs::remove_file(&tmp_path) {
            tracing::warn!("Failed to remove temporary file: {e}");
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-file-rename-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}

/// DELETE /api/agents/{id}/files/{filename} — Delete a workspace identity file.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    responses(
        (status = 200, description = "File deleted successfully", body = serde_json::Value),
        (status = 404, description = "File not found", body = serde_json::Value)
    )
)]
pub async fn delete_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    let workspace = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => match e.manifest.workspace {
            Some(ref ws) => ws.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
                );
            }
        },
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    // Resolve path: prefer .identity/ (current layout), fall back to workspace root
    let identity_candidate = workspace.join(".identity").join(&filename);
    let file_path = if identity_candidate.exists() {
        identity_candidate
    } else {
        workspace.join(&filename)
    };

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    if let Err(e) = std::fs::remove_file(&canonical) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-file-delete-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
        })),
    )
}

// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------

/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

/// Metadata stored alongside uploaded files.
pub(crate) struct UploadMeta {
    #[allow(dead_code)]
    pub(crate) filename: String,
    pub(crate) content_type: String,
}

/// In-memory upload metadata registry.
pub(crate) static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> =
    LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
#[allow(dead_code)]
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Non-media MIME types also accepted on `/api/agents/{id}/upload` — text
/// files and PDFs that the agent loop consumes directly. Media types are
/// sourced from `librefang_types::media::{ALLOWED_IMAGE_TYPES,
/// ALLOWED_AUDIO_TYPES}` so the upload endpoint, the channel bridge, and
/// `MediaAttachment::validate()` can never drift.
///
/// Browsers send a wide variety of `Content-Type` values for the same file
/// kind (`.json` → `application/json`; `.yaml` → `application/x-yaml` /
/// `application/yaml`; `.ipynb` → `application/x-ipynb+json` / sometimes
/// `application/json`), so this list is intentionally exhaustive on the
/// safe subset.
const EXTRA_ALLOWED_UPLOAD_TYPES: &[&str] = &[
    "application/pdf",
    // Plain text + tables
    "text/plain",
    "text/markdown",
    "text/csv",
    "text/tab-separated-values",
    // Structured data
    "application/json",
    "application/x-ipynb+json",
    "application/xml",
    "application/yaml",
    "application/x-yaml",
    "application/toml",
    "application/x-toml",
    "application/sql",
    "application/graphql",
    // Code (often delivered with these MIMEs)
    "application/javascript",
    "application/x-javascript",
    "application/typescript",
];

/// MIME allowlist for `/api/agents/{id}/upload`.
///
/// Historically this was a permissive prefix list (`image/`, `text/`,
/// `application/pdf`, `audio/`) which accepted dangerous subtypes like
/// `image/svg+xml` (scriptable → XSS / SSRF), `text/html` (stored XSS
/// via downstream renderers), and `text/xml` (XXE / SSRF). That
/// contradicted the SECURITY.md promise of *"Media type whitelist
/// (png/jpeg/gif/webp)"*.
///
/// The check now combines:
///   1. Exact match against the canonical media constants
///      (`ALLOWED_IMAGE_TYPES`, `ALLOWED_AUDIO_TYPES`).
///   2. Exact match against `EXTRA_ALLOWED_UPLOAD_TYPES` (PDF + curated
///      text/data/code MIMEs).
///   3. **Any other `text/*` subtype** EXCEPT `text/html` and `text/xml`.
///      Browsers tag many code files (`.rs`, `.py`, `.go`, `.sh`, …) as
///      `text/x-rust`, `text/x-python`, `text/x-shellscript` etc. — those
///      are safe to inline because the agent loop reads them as plain
///      UTF-8 and never executes/renders them. HTML/XML stay blocked
///      because downstream consumers (markdown renderer, XML parsers)
///      could be tricked into XSS / XXE.
fn is_allowed_content_type(ct: &str) -> bool {
    use librefang_types::media::{mime_base, ALLOWED_AUDIO_TYPES, ALLOWED_IMAGE_TYPES};
    let base = mime_base(ct);
    if ALLOWED_IMAGE_TYPES.contains(&base.as_str())
        || ALLOWED_AUDIO_TYPES.contains(&base.as_str())
        || EXTRA_ALLOWED_UPLOAD_TYPES.contains(&base.as_str())
    {
        return true;
    }
    if let Some(subtype) = base.strip_prefix("text/") {
        // Anything text-like is fine to ingest as a plain-text attachment,
        // except formats that get rendered/parsed by downstream tooling
        // and could carry an exploit payload.
        return !matches!(subtype, "html" | "xml");
    }
    false
}

/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts raw body bytes. The client must set:
/// - `Content-Type` header (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `X-Filename` header (original filename)
#[utoipa::path(
    post,
    path = "/api/agents/{id}/upload",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = String, content_type = "application/octet-stream"),
    responses(
        (status = 200, description = "Upload a file attachment for an agent", body = serde_json::Value)
    )
)]
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let (
        err_invalid_id,
        err_unsupported_type,
        err_too_large_upload,
        err_empty_body,
        err_upload_dir_failed,
        err_upload_save_failed,
    ) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-file-unsupported-type"),
            t.t_args("api-error-file-too-large", &[("max", "10MB")]),
            t.t("api-error-file-empty-body"),
            t.t("api-error-file-upload-dir-failed"),
            t.t("api-error-file-save-failed"),
        )
    };
    // Validate agent ID format
    let _agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            );
        }
    };

    // Extract content type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_unsupported_type})),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Validate size (use config override or fall back to compiled default)
    let upload_limit = state.kernel.config_ref().max_upload_size_bytes;
    if body.len() > upload_limit {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": err_too_large_upload})),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_empty_body})),
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_dir_failed})),
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_save_failed})),
        );
    }

    let size = body.len();
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            filename: filename.clone(),
            content_type: content_type.clone(),
        },
    );

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = librefang_types::media::MediaAttachment {
            media_type: librefang_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: librefang_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state.kernel.media().transcribe_audio(&attachment).await {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}

/// GET /api/uploads/{file_id} — Serve an uploaded file.
#[utoipa::path(
    get,
    path = "/api/uploads/{file_id}",
    tag = "agents",
    params(("file_id" = String, Path, description = "Upload file ID (UUID)")),
    responses(
        (status = 200, description = "Serve an uploaded file by ID", body = serde_json::Value)
    )
)]
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let file_path = std::env::temp_dir()
        .join("librefang_uploads")
        .join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => m.content_type.clone(),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            "image/png".to_string()
        }
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Delivery tracking endpoints
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/deliveries — List recent delivery receipts for an agent.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/deliveries",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List recent delivery receipts for an agent", body = serde_json::Value)
    )
)]
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.agent_registry().find_by_name(&id) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                    );
                }
            }
        }
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let receipts = state.kernel.delivery().get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

// ---------------------------------------------------------------------------
// Mid-turn message injection (#956)
// ---------------------------------------------------------------------------

/// POST /api/agents/:id/inject — Inject a message into a running agent's tool loop.
///
/// If the agent is currently executing tools (mid-turn), the injected message
/// will be processed between tool calls, interrupting the remaining sequence.
/// Returns `{"injected": true}` if accepted, `{"injected": false}` if no
/// active tool loop is running for this agent.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/inject",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = crate::types::InjectMessageRequest,
    responses(
        (status = 200, description = "Injection result", body = crate::types::InjectMessageResponse),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found"),
        (status = 413, description = "Message too large")
    )
)]
pub async fn inject_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<InjectMessageRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("invalid agent ID").into_response();
        }
    };

    // Reject oversized injection messages
    const MAX_INJECT_SIZE: usize = 16 * 1024; // 16KB
    if req.message.len() > MAX_INJECT_SIZE {
        return ApiErrorResponse::bad_request("injection message too large")
            .with_status(StatusCode::PAYLOAD_TOO_LARGE)
            .into_response();
    }

    match state.kernel.inject_message(agent_id, &req.message).await {
        Ok(injected) => (
            StatusCode::OK,
            Json(serde_json::json!({"injected": injected})),
        )
            .into_response(),
        Err(e) => if e.to_string().contains("not found") {
            ApiErrorResponse::not_found(e.to_string())
        } else {
            ApiErrorResponse::internal(e.to_string())
        }
        .into_response(),
    }
}

// Push message — proactive outbound messaging via channel adapters
// ---------------------------------------------------------------------------

/// `POST /api/agents/:id/push` — push a proactive outbound message from an
/// agent to a channel recipient (e.g., Telegram chat, Slack channel, email).
///
/// The agent must exist, but the message is sent directly through the channel
/// adapter without going through the agent loop. This is the REST API
/// counterpart of the built-in `channel_send` tool that agents can self-invoke.
pub async fn push_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<crate::types::PushMessageRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let (err_invalid_id, err_not_found) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-agent-not-found"),
        )
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            );
        }
    };

    // Validate agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        );
    }

    // Validate request fields
    if req.channel.is_empty() || req.recipient.is_empty() || req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "channel, recipient, and message are required"})),
        );
    }

    // Delegate to the bridge manager if available, otherwise use kernel directly
    let thread_id = req.thread_id.as_deref();
    let result = {
        let bridge = state.bridge_manager.lock().await;
        if let Some(ref bm) = *bridge {
            bm.push_message(&req.channel, &req.recipient, &req.message, thread_id)
                .await
        } else {
            // No bridge manager — fall back to kernel's channel adapter registry
            state
                .kernel
                .send_channel_message(&req.channel, &req.recipient, &req.message, thread_id, None)
                .await
        }
    };

    match result {
        Ok(detail) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "detail": detail,
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "success": false,
                "detail": e,
                "agent_id": agent_id.to_string(),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use librefang_channels::types::ParticipantRef;

    /// The pre-fix prefix-match (`"image/"`) let SVG, BMP, TIFF, HEIC and
    /// friends through. Post-fix the allowlist is exact-match over the
    /// same four formats SECURITY.md advertises.
    #[test]
    fn test_upload_mime_allowlist_rejects_previously_accepted_types() {
        // Previously accepted via prefix match, now explicitly rejected.
        for bad in [
            "image/svg+xml",
            "image/svg+xml; charset=utf-8",
            "image/bmp",
            "image/tiff",
            "image/x-icon",
            "image/heic",
            "image/heif",
            "image/avif",
            "image/vnd.microsoft.icon",
            "text/html", // text/ prefix used to let this through
            "text/xml",
            "audio/vnd.rn-realaudio",
            "application/octet-stream",
        ] {
            assert!(
                !is_allowed_content_type(bad),
                "{bad} must be rejected by the upload allowlist"
            );
        }
    }

    #[test]
    fn test_upload_mime_allowlist_accepts_expected_formats() {
        for good in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/PNG",                 // case-insensitive
            "image/png; charset=binary", // MIME params stripped
            "audio/mpeg",
            "audio/wav",
            "audio/ogg",
            "audio/flac",
            "text/plain",
            "text/markdown",
            "text/csv",
            "application/pdf",
        ] {
            assert!(
                is_allowed_content_type(good),
                "{good} must be accepted by the upload allowlist"
            );
        }
    }

    #[test]
    fn test_clone_request_defaults() {
        let json = r#"{"new_name": "clone-1"}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.new_name, "clone-1");
        assert!(req.include_skills);
        assert!(req.include_tools);
    }

    #[test]
    fn test_clone_request_explicit_false() {
        let json = r#"{"new_name": "clone-2", "include_skills": false, "include_tools": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.new_name, "clone-2");
        assert!(!req.include_skills);
        assert!(!req.include_tools);
    }

    #[test]
    fn test_clone_request_partial_flags() {
        let json = r#"{"new_name": "clone-3", "include_skills": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert!(!req.include_skills);
        assert!(req.include_tools);

        let json = r#"{"new_name": "clone-4", "include_tools": false}"#;
        let req: CloneAgentRequest = serde_json::from_str(json).unwrap();
        assert!(req.include_skills);
        assert!(!req.include_tools);
    }

    #[test]
    fn test_clone_manifest_strips_skills_when_excluded() {
        let manifest = librefang_types::agent::AgentManifest {
            skills: vec!["skill-a".to_string(), "skill-b".to_string()],
            tools: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "tool-a".to_string(),
                    librefang_types::agent::ToolConfig {
                        params: std::collections::HashMap::new(),
                    },
                );
                m
            },
            ..Default::default()
        };

        let mut cloned = manifest.clone();
        apply_clone_inclusion_flags(
            &mut cloned,
            &CloneAgentRequest {
                new_name: "clone-1".to_string(),
                include_skills: false,
                include_tools: true,
            },
        );
        assert!(cloned.skills.is_empty());
        assert!(cloned.skills_disabled);
        assert_eq!(skill_assignment_mode(&cloned), "none");
        assert!(!cloned.tools.is_empty());
    }

    #[test]
    fn test_clone_manifest_disables_tools_when_excluded() {
        let manifest = librefang_types::agent::AgentManifest {
            tools: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "tool-a".to_string(),
                    librefang_types::agent::ToolConfig {
                        params: std::collections::HashMap::new(),
                    },
                );
                m
            },
            tool_allowlist: vec!["allowed-tool".to_string()],
            tool_blocklist: vec!["blocked-tool".to_string()],
            ..Default::default()
        };

        let mut cloned = manifest.clone();
        apply_clone_inclusion_flags(
            &mut cloned,
            &CloneAgentRequest {
                new_name: "clone-2".to_string(),
                include_skills: true,
                include_tools: false,
            },
        );
        assert!(cloned.tools.is_empty());
        assert!(cloned.tool_allowlist.is_empty());
        assert!(cloned.tool_blocklist.is_empty());
        assert!(cloned.tools_disabled);
    }

    #[test]
    fn test_request_sender_context_none_without_sender_id() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: None,
            sender_name: None,
            channel_type: Some("whatsapp".to_string()),
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
        };
        assert!(request_sender_context(&req).is_none());
    }

    #[test]
    fn test_request_sender_context_builds_defaults() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: Some("u-123".to_string()),
            sender_name: None,
            channel_type: None,
            is_group: false,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.user_id, "u-123");
        assert_eq!(sender.display_name, "u-123");
        assert_eq!(sender.channel, "api");
        assert!(sender.group_participants.is_empty());
    }

    #[test]
    fn test_request_sender_context_propagates_group_and_mention() {
        let req = MessageRequest {
            message: "hello".to_string(),
            attachments: Vec::new(),
            sender_id: Some("u-456".to_string()),
            sender_name: Some("Alice".to_string()),
            channel_type: Some("whatsapp".to_string()),
            is_group: true,
            was_mentioned: true,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: None,
            session_id: None,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert!(sender.is_group);
        assert!(sender.was_mentioned);
    }

    #[test]
    fn test_request_sender_context_threads_group_participants() {
        let roster = vec![
            ParticipantRef {
                jid: "111@s.whatsapp.net".to_string(),
                display_name: "Alice".to_string(),
            },
            ParticipantRef {
                jid: "222@s.whatsapp.net".to_string(),
                display_name: "Bob".to_string(),
            },
        ];
        let req = MessageRequest {
            message: "Bob, ciao".to_string(),
            attachments: Vec::new(),
            sender_id: Some("111@s.whatsapp.net".to_string()),
            sender_name: Some("Alice".to_string()),
            channel_type: Some("whatsapp".to_string()),
            is_group: true,
            was_mentioned: false,
            ephemeral: false,
            thinking: None,
            show_thinking: None,
            group_participants: Some(roster.clone()),
            session_id: None,
        };
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.group_participants, roster);
    }

    #[test]
    fn test_message_request_group_participants_default_when_missing() {
        // Backward compat: callers (Telegram, direct API) that omit
        // `group_participants` must still deserialize cleanly.
        let json = serde_json::json!({
            "message": "hi",
            "sender_id": "u-1",
            "channel_type": "telegram",
            "is_group": false,
        });
        let req: MessageRequest =
            serde_json::from_value(json).expect("deserialize without group_participants");
        assert!(req.group_participants.is_none());
        let sender = request_sender_context(&req).expect("sender context");
        assert!(sender.group_participants.is_empty());
    }

    #[test]
    fn test_message_request_group_participants_deserializes_from_json() {
        let json = serde_json::json!({
            "message": "hey Bob",
            "sender_id": "111@s.whatsapp.net",
            "sender_name": "Alice",
            "channel_type": "whatsapp:group-jid@g.us",
            "is_group": true,
            "group_participants": [
                {"jid": "111@s.whatsapp.net", "display_name": "Alice"},
                {"jid": "222@s.whatsapp.net", "display_name": "Bob"}
            ]
        });
        let req: MessageRequest =
            serde_json::from_value(json).expect("deserialize with group_participants");
        let sender = request_sender_context(&req).expect("sender context");
        assert_eq!(sender.group_participants.len(), 2);
        assert_eq!(sender.group_participants[1].display_name, "Bob");
    }

    #[test]
    fn test_effective_default_model_prefers_override() {
        let base = librefang_types::config::DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        let override_dm = librefang_types::config::DefaultModelConfig {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };

        let effective = effective_default_model(&base, Some(&override_dm));

        assert_eq!(effective.provider, "deepseek");
        assert_eq!(effective.model, "deepseek-chat");
        assert_eq!(effective.api_key_env, "DEEPSEEK_API_KEY");
    }

    #[test]
    fn test_effective_default_model_falls_back_to_base() {
        let base = librefang_types::config::DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };

        let effective = effective_default_model(&base, None);

        assert_eq!(effective.provider, "openai");
        assert_eq!(effective.model, "gpt-4.1");
        assert_eq!(effective.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn test_patch_config_request_temperature_deserialization() {
        let json = r#"{"temperature": 1.5}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(1.5));
        assert!(req.max_tokens.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn test_patch_config_request_temperature_range() {
        // Valid ranges
        for temp in [0.0, 0.5, 1.0, 1.5, 2.0] {
            let json = format!(r#"{{"temperature": {temp}}}"#);
            let req: PatchAgentConfigRequest = serde_json::from_str(&json).unwrap();
            assert_eq!(req.temperature, Some(temp));
        }

        // Out of range values still deserialize (validation happens in handler)
        let json = r#"{"temperature": 3.0}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(3.0));

        // Negative values still deserialize (validation happens in handler)
        let json = r#"{"temperature": -0.5}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, Some(-0.5));
    }

    #[test]
    fn test_patch_config_request_without_temperature() {
        let json = r#"{"max_tokens": 4096}"#;
        let req: PatchAgentConfigRequest = serde_json::from_str(json).unwrap();
        assert!(req.temperature.is_none());
        assert_eq!(req.max_tokens, Some(4096));
    }
}

// ---------------------------------------------------------------------------
// Agent monitoring and profiling endpoints (#181)
// ---------------------------------------------------------------------------

/// GET /api/agents/{id}/metrics — Returns aggregated metrics for an agent.
///
/// Includes message count, token usage, tool execution count, error count,
/// average response time (estimated), and cost data.
pub async fn agent_metrics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    // Session-level token/tool stats from the scheduler (in-memory, windowed).
    let sched_snap = state
        .kernel
        .scheduler_ref()
        .get_usage(agent_id)
        .unwrap_or_default();
    let (sched_tokens, sched_tool_calls) = (sched_snap.total_tokens, sched_snap.tool_calls);

    // Persistent usage summary from the UsageStore (SQLite).
    let usage_summary = state
        .kernel
        .memory_substrate()
        .usage()
        .query_summary(Some(agent_id))
        .ok();

    // Message count from the active session.
    let message_count: u64 = state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
        .ok()
        .flatten()
        .map(|s| s.messages.len() as u64)
        .unwrap_or(0);

    // Error count from the audit log (count entries with non-"ok" outcome for this agent).
    // NOTE: This scans the most recent 100k audit entries. Agents with errors beyond
    // this window will have under-reported error counts. A dedicated per-agent error
    // counter or index would eliminate this limitation.
    let agent_id_str = agent_id.to_string();
    let error_count: u64 = state
        .kernel
        .audit()
        .recent(100_000)
        .iter()
        .filter(|e| e.agent_id == agent_id_str && e.outcome != "ok" && e.outcome != "success")
        .count() as u64;

    // Uptime since the agent was created.
    let uptime_secs = (chrono::Utc::now() - entry.created_at).num_seconds().max(0) as u64;

    // Persistent usage values (fall back to scheduler data when no DB records exist).
    let (total_input_tokens, total_output_tokens, total_cost_usd, call_count, total_tool_calls) =
        match usage_summary {
            Some(ref s) => (
                s.total_input_tokens,
                s.total_output_tokens,
                s.total_cost_usd,
                s.call_count,
                s.total_tool_calls,
            ),
            None => (0, 0, 0.0, 0, 0),
        };

    // Average response time is not tracked yet; keep the field stable until
    // per-call timing is persisted in UsageStore.
    let avg_response_time_ms: Option<f64> = None;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "uptime_secs": uptime_secs,
            "message_count": message_count,
            "token_usage": {
                "session_tokens": sched_tokens,
                "total_input_tokens": total_input_tokens,
                "total_output_tokens": total_output_tokens,
                "total_tokens": total_input_tokens + total_output_tokens,
            },
            "tool_calls": {
                "session_tool_calls": sched_tool_calls,
                "total_tool_calls": total_tool_calls,
            },
            "cost_usd": total_cost_usd,
            "call_count": call_count,
            "error_count": error_count,
            "avg_response_time_ms": avg_response_time_ms,
        })),
    )
}

/// GET /api/agents/{id}/logs — Returns structured execution logs for an agent.
///
/// Supports optional query parameters:
/// - `n`: max number of log entries (default 100, max 1000)
/// - `level`: filter by outcome (e.g. "error", "ok")
/// - `offset`: number of matching entries to skip for pagination (default 0)
pub async fn agent_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Verify the agent exists.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let max_entries: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .min(1000);

    let offset: usize = params
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let level_filter = params
        .get("level")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let agent_id_str = agent_id.to_string();

    // Filter audit log entries belonging to this agent.
    let entries: Vec<serde_json::Value> = state
        .kernel
        .audit()
        .recent(100_000)
        .iter()
        .filter(|e| e.agent_id == agent_id_str)
        .filter(|e| {
            if level_filter.is_empty() {
                return true;
            }
            e.outcome.eq_ignore_ascii_case(&level_filter)
        })
        .skip(offset)
        .take(max_entries)
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id_str,
            "count": entries.len(),
            "offset": offset,
            "logs": entries,
        })),
    )
}

#[cfg(test)]
mod monitoring_tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use librefang_runtime::audit::AuditAction;
    use librefang_types::config::KernelConfig;

    fn monitoring_test_app_state() -> (Arc<AppState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("librefang-api-monitoring-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = Arc::new(librefang_kernel::LibreFangKernel::boot_with_config(config).unwrap());
        let state = Arc::new(AppState {
            kernel,
            started_at: std::time::Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(
                home_dir.join("data").join("webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            #[cfg(feature = "telemetry")]
            prometheus_handle: None,
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            config_write_lock: tokio::sync::Mutex::new(()),
        });
        (state, tmp)
    }

    fn spawn_monitoring_test_agent(state: &Arc<AppState>, name: &str) -> AgentId {
        let manifest = AgentManifest {
            name: name.to_string(),
            ..AgentManifest::default()
        };
        state.kernel.spawn_agent(manifest).unwrap()
    }

    async fn json_response(response: impl IntoResponse) -> (StatusCode, serde_json::Value) {
        let response = response.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_metrics_returns_json_shape_for_existing_agent() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "metrics-shape");

        let (status, body) =
            json_response(agent_metrics(State(state), Path(agent_id.to_string()), None).await)
                .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["agent_id"], agent_id.to_string());
        assert!(body["token_usage"].is_object());
        assert!(body["tool_calls"].is_object());
        assert!(body.get("avg_response_time_ms").is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_metrics_returns_not_found_for_unknown_agent() {
        let (state, _tmp) = monitoring_test_app_state();

        let (status, body) = json_response(
            agent_metrics(State(state), Path(AgentId::new().to_string()), None).await,
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Agent not found");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_agent_logs_filters_level_by_exact_match() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "logs-filter");
        let agent_id_str = agent_id.to_string();

        state.kernel.audit().record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "exact match target",
            "custom_error",
        );
        state.kernel.audit().record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "should not match substring filter",
            "not_custom_error",
        );

        let mut params = HashMap::new();
        params.insert("level".to_string(), "custom_error".to_string());

        let (status, body) =
            json_response(agent_logs(State(state), Path(agent_id_str), None, Query(params)).await)
                .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);

        let logs = body["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["outcome"], "custom_error");
    }

    #[test]
    fn test_patch_agent_mcp_servers_parses_top_level_and_nested_shapes() {
        let top_level = serde_json::json!({"mcp_servers": ["alpha", "beta"]});
        assert_eq!(
            patch_agent_mcp_servers(&top_level).unwrap(),
            Some(vec!["alpha".to_string(), "beta".to_string()])
        );

        let nested = serde_json::json!({"capabilities": {"mcp_servers": ["gamma"]}});
        assert_eq!(
            patch_agent_mcp_servers(&nested).unwrap(),
            Some(vec!["gamma".to_string()])
        );
    }

    #[test]
    fn test_patch_agent_mcp_servers_rejects_invalid_shape() {
        let invalid = serde_json::json!({"mcp_servers": [{}]});
        assert!(patch_agent_mcp_servers(&invalid).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_patch_agent_updates_top_level_mcp_servers_and_persists() {
        let (state, _tmp) = monitoring_test_app_state();
        let manifest = AgentManifest {
            name: "patch-top-level-mcp".to_string(),
            mcp_servers: vec!["server-a".to_string()],
            ..AgentManifest::default()
        };
        let agent_id = state.kernel.spawn_agent(manifest).unwrap();

        let (status, body) = json_response(
            patch_agent(
                State(state.clone()),
                Path(agent_id.to_string()),
                None,
                Json(serde_json::json!({"mcp_servers": []})),
            )
            .await,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(
            state
                .kernel
                .agent_registry()
                .get(agent_id)
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
        assert_eq!(
            state
                .kernel
                .memory_substrate()
                .load_agent(agent_id)
                .unwrap()
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_patch_agent_updates_nested_capabilities_mcp_servers_and_persists() {
        let (state, _tmp) = monitoring_test_app_state();
        let manifest = AgentManifest {
            name: "patch-nested-mcp".to_string(),
            mcp_servers: vec!["server-b".to_string()],
            ..AgentManifest::default()
        };
        let agent_id = state.kernel.spawn_agent(manifest).unwrap();

        let (status, body) = json_response(
            patch_agent(
                State(state.clone()),
                Path(agent_id.to_string()),
                None,
                Json(serde_json::json!({"capabilities": {"mcp_servers": []}})),
            )
            .await,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(
            state
                .kernel
                .agent_registry()
                .get(agent_id)
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
        assert_eq!(
            state
                .kernel
                .memory_substrate()
                .load_agent(agent_id)
                .unwrap()
                .unwrap()
                .manifest
                .mcp_servers,
            Vec::<String>::new()
        );
    }
}
