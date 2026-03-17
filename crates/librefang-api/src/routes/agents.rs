//! Agent CRUD, messaging, sessions, files, and upload handlers.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use librefang_kernel::LibreFangKernel;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::{AgentId, AgentIdentity, AgentManifest};
use librefang_types::i18n::{self, ErrorTranslator};
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
                .config
                .home_dir
                .join("agents")
                .join(&safe_name)
                .join("agent.toml");
            // 使用 tokio::fs 避免在异步上下文中阻塞
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
                state.kernel.audit_log.record(
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
    let manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            let _ = e;
            let t = ErrorTranslator::new(lang);
            return Err(ManifestError {
                message: t.t("api-error-manifest-invalid-format"),
            });
        }
    };

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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-agent-spawn-failed")})),
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
fn validate_bulk_size(len: usize) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if len == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Array must not be empty"})),
        ));
    }
    if len > BULK_LIMIT {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Too many items (max {})", BULK_LIMIT)})),
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
    Json(req): Json<BulkCreateRequest>,
) -> impl IntoResponse {
    if let Err(resp) = validate_bulk_size(req.agents.len()) {
        return resp;
    }

    let mut results: Vec<BulkCreateResult> = Vec::with_capacity(req.agents.len());

    for (index, spawn_req) in req.agents.iter().enumerate() {
        match resolve_manifest(&state, spawn_req, i18n::DEFAULT_LANGUAGE).await {
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
                        results.push(BulkCreateResult {
                            index,
                            success: false,
                            agent_id: None,
                            name: None,
                            error: Some(format!("Spawn failed: {e}")),
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
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    if let Err(resp) = validate_bulk_size(req.agent_ids.len()) {
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
                    error: Some("Invalid agent ID".into()),
                });
                continue;
            }
        };
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
                    error: Some(format!("{e}")),
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
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    use librefang_types::agent::AgentMode;

    if let Err(resp) = validate_bulk_size(req.agent_ids.len()) {
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
                    error: Some("Invalid agent ID".into()),
                });
                continue;
            }
        };
        match state.kernel.registry.set_mode(agent_id, AgentMode::Full) {
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
                    error: Some("Agent not found".into()),
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
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    if let Err(resp) = validate_bulk_size(req.agent_ids.len()) {
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
                    error: Some("Invalid agent ID".into()),
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
                    error: Some(format!("{e}")),
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

/// GET /api/agents — List all agents.
#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "agents",
    responses(
        (status = 200, description = "List all agents", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Snapshot catalog once for enrichment
    let catalog = state.kernel.model_catalog.read().ok();
    let dm = &state.kernel.config.default_model;

    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            // Resolve "default" provider/model to actual kernel defaults
            let provider =
                if e.manifest.model.provider.is_empty() || e.manifest.model.provider == "default" {
                    dm.provider.as_str()
                } else {
                    e.manifest.model.provider.as_str()
                };
            let model = if e.manifest.model.model.is_empty() || e.manifest.model.model == "default"
            {
                dm.model.as_str()
            } else {
                e.manifest.model.model.as_str()
            };

            // Enrich from catalog
            let (tier, auth_status) = catalog
                .as_ref()
                .map(|cat| {
                    let tier = cat
                        .find_model(model)
                        .map(|m| format!("{:?}", m.tier).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    let auth = cat
                        .get_provider(provider)
                        .map(|p| format!("{:?}", p.auth_status).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    (tier, auth)
                })
                .unwrap_or(("unknown".to_string(), "unknown".to_string()));

            let ready = matches!(e.state, librefang_types::agent::AgentState::Running)
                && auth_status != "missing";

            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "last_active": e.last_active.to_rfc3339(),
                "model_provider": provider,
                "model_name": model,
                "model_tier": tier,
                "auth_status": auth_status,
                "ready": ready,
                "profile": e.manifest.profile,
                "identity": {
                    "emoji": e.identity.emoji,
                    "avatar_url": e.identity.avatar_url,
                    "color": e.identity.color,
                },
            })
        })
        .collect();

    Json(agents)
}

/// Resolve uploaded file attachments into ContentBlock::Image blocks.
///
/// Reads each file from the upload directory, base64-encodes it, and
/// returns image content blocks ready to insert into a session message.
pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<librefang_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let content_type = if let Some(ref m) = meta {
            m.content_type.clone()
        } else if !att.content_type.is_empty() {
            att.content_type.clone()
        } else {
            continue; // Skip unknown attachments
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            continue;
        }

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);
        match std::fs::read(&file_path) {
            Ok(data) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(librefang_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data: b64,
                });
            }
            Err(e) => {
                tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read upload for attachment");
            }
        }
    }

    blocks
}

/// Pre-insert image attachments into an agent's session so the LLM can see them.
///
/// This injects image content blocks into the session BEFORE the kernel
/// adds the text user message, so the LLM receives: [..., User(images), User(text)].
pub fn inject_attachments_into_session(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    image_blocks: Vec<librefang_types::message::ContentBlock>,
) {
    use librefang_types::message::{Message, MessageContent, Role};

    let entry = match kernel.registry.get(agent_id) {
        Some(e) => e,
        None => return,
    };

    let mut session = match kernel.memory.get_session(entry.session_id) {
        Ok(Some(s)) => s,
        _ => librefang_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        },
    };

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(image_blocks),
        pinned: false,
    });

    if let Err(e) = kernel.memory.save_session(&session) {
        tracing::warn!(error = %e, "Failed to save session with image attachments");
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
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    // Pre-translate error messages before the `.await` point below.
    // `ErrorTranslator` wraps a `FluentBundle` which is `!Send`, so it must
    // not be held across an await boundary (axum requires `Send` futures).
    let (err_invalid_id, err_too_large, err_not_found) = {
        let t = ErrorTranslator::new(i18n::DEFAULT_LANGUAGE);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-message-too-large"),
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

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": err_too_large})),
        );
    }

    // Check agent exists before processing
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_not_found})),
        );
    }

    // Resolve file attachments into image content blocks
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(agent_id, &req.message, Some(kernel_handle))
        .await
    {
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

            // Strip <think>...</think> blocks from model output
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
                })),
            )
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            let status = if format!("{e}").contains("Agent not found") {
                StatusCode::NOT_FOUND
            } else if format!("{e}").contains("quota") || format!("{e}").contains("Quota") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
            )
        }
    }
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    match state.kernel.memory.get_session(entry.session_id) {
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
                                            let preview: String =
                                                result.chars().take(2000).collect();
                                            tool_obj["result"] = serde_json::Value::String(preview);
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
                Json(serde_json::json!({"error": "Session load failed"})),
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        ),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found or already terminated"})),
            )
        }
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
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.registry.set_mode(agent_id, body.mode) {
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
            Json(serde_json::json!({"error": "Agent not found"})),
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": entry.manifest.model.provider,
                "model": entry.manifest.model.model,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
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
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use librefang_runtime::llm_driver::StreamEvent;

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        )
            .into_response();
    }

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
                .into_response();
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        )
            .into_response();
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle) =
        match state
            .kernel
            .send_message_streaming(agent_id, &req.message, Some(kernel_handle))
        {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("Streaming message failed for agent {id}: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Streaming message failed"})),
                )
                    .into_response();
            }
        };

    let sse_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => Event::default()
                        .event("chunk")
                        .json_data(serde_json::json!({"content": text, "done": false}))
                        .unwrap_or_else(|_| Event::default().data("error")),
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
                    _ => Event::default().comment("skip"),
                });
                Some((sse_event, rx))
            }
            None => None,
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
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
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
    Json(req): Json<serde_json::Value>,
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
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
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
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => librefang_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
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
            Json(serde_json::json!({"error": format!("{e}")})),
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
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
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
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }
    match state.kernel.clear_agent_history(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
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
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
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
            Json(serde_json::json!({"error": format!("{e}")})),
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
    Json(body): Json<serde_json::Value>,
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
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'model' field"})),
            )
        }
    };
    let explicit_provider = body["provider"].as_str();
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
                .registry
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
            Json(serde_json::json!({"error": format!("{e}")})),
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

    // Check agent exists
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }

    let traces = state
        .kernel
        .decision_traces
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
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
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
    Json(body): Json<serde_json::Value>,
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
            Json(serde_json::json!({"error": "Provide 'tool_allowlist' and/or 'tool_blocklist'"})),
        );
    }

    match state
        .kernel
        .set_agent_tool_filters(agent_id, allowlist, blocklist)
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
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
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry
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
    Json(body): Json<serde_json::Value>,
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
            Json(serde_json::json!({"error": format!("{e}")})),
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
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        let configured_servers: Vec<String> = state
            .kernel
            .effective_mcp_servers
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
    Json(body): Json<serde_json::Value>,
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
            Json(serde_json::json!({"error": format!("{e}")})),
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
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }

    // Parse the new manifest
    let _manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid manifest: {e}")})),
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
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }

    // Apply partial updates using dedicated registry methods
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_name(agent_id, name.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_description(agent_id, desc.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
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
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }
    if let Some(system_prompt) = body.get("system_prompt").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_system_prompt(agent_id, system_prompt.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            );
        }
    }

    // Persist updated entry to SQLite
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if let Err(e) = state.kernel.memory.save_agent(&entry) {
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
            Json(serde_json::json!({"error": "Agent vanished during update"})),
        )
    }
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
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
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
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
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

    match state.kernel.registry.update_identity(agent_id, identity) {
        Ok(()) => {
            // Persist identity to SQLite
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                if let Err(e) = state.kernel.memory.save_agent(&entry) {
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
            Json(serde_json::json!({"error": "Agent not found"})),
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
    #[schema(value_type = Option<Vec<serde_json::Value>>)]
    pub fallback_models: Option<Vec<librefang_types::agent::FallbackModel>>,
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
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
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
                    serde_json::json!({"error": format!("Name exceeds max length ({MAX_NAME_LEN} chars)")}),
                ),
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": format!("Description exceeds max length ({MAX_DESC_LEN} chars)")}),
                ),
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": format!("System prompt exceeds max length ({MAX_PROMPT_LEN} chars)")}),
                ),
            );
        }
    }

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
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
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
            );
        }
    }

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .registry
                .update_name(agent_id, new_name.clone())
            {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": format!("{e}")})),
                );
            }
        }
    }

    // Update description
    if let Some(ref new_desc) = req.description {
        if state
            .kernel
            .registry
            .update_description(agent_id, new_desc.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message)
    if let Some(ref new_prompt) = req.system_prompt {
        if state
            .kernel
            .registry
            .update_system_prompt(agent_id, new_prompt.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
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
            .registry
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
            .registry
            .update_identity(agent_id, merged)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update model/provider — use set_agent_model for catalog-based provider
    // resolution when provider is not explicitly provided (fixes #387/#466:
    // changing model from another provider without specifying provider now
    // auto-resolves the correct provider from the model catalog).
    if let Some(ref new_model) = req.model {
        if !new_model.is_empty() {
            if let Some(ref new_provider) = req.provider {
                if !new_provider.is_empty() {
                    // Explicit provider given — use it directly
                    if state
                        .kernel
                        .registry
                        .update_model_and_provider(
                            agent_id,
                            new_model.clone(),
                            new_provider.clone(),
                        )
                        .is_err()
                    {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({"error": "Agent not found"})),
                        );
                    }
                } else {
                    // Provider is empty string — resolve from catalog
                    if let Err(e) = state.kernel.set_agent_model(agent_id, new_model, None) {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": format!("{e}")})),
                        );
                    }
                }
            } else {
                // No provider field at all — resolve from catalog
                if let Err(e) = state.kernel.set_agent_model(agent_id, new_model, None) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": format!("{e}")})),
                    );
                }
            }
        }
    }

    // Update fallback model chain
    if let Some(fallbacks) = req.fallback_models {
        if state
            .kernel
            .registry
            .update_fallback_models(agent_id, fallbacks)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Persist updated manifest to database so changes survive restart
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if let Err(e) = state.kernel.memory.save_agent(&entry) {
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
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    if req.new_name.len() > 256 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Name exceeds max length (256 chars)"})),
        );
    }

    if req.new_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_name cannot be empty"})),
        );
    }

    let source = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
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
                Json(serde_json::json!({"error": format!("Clone spawn failed: {e}")})),
            );
        }
    };

    // Copy workspace files from source to destination
    let new_entry = state.kernel.registry.get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                for &fname in KNOWN_IDENTITY_FILES {
                    let src_file = src_can.join(fname);
                    let dst_file = dst_can.join(fname);
                    if src_file.exists() {
                        if let Err(e) = std::fs::copy(&src_file, &dst_file) {
                            tracing::warn!("Failed to copy file: {e}");
                        }
                    }
                }
            }
        }
    }

    // Copy identity from source
    if let Err(e) = state
        .kernel
        .registry
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        let path = workspace.join(name);
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
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
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "File content too large (max 32KB)"})),
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };

    let file_path = workspace.join(&filename);
    // For new files, check the parent directory instead
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        // Parent must be inside workspace
        file_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = workspace.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Write failed: {e}")})),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        if let Err(e) = std::fs::remove_file(&tmp_path) {
            tracing::warn!("Failed to remove temporary file: {e}");
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Rename failed: {e}")})),
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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    let workspace = match state.kernel.registry.get(agent_id) {
        Some(e) => match e.manifest.workspace {
            Some(ref ws) => ws.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Agent has no workspace"})),
                );
            }
        },
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    if let Err(e) = std::fs::remove_file(&canonical) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Delete failed: {e}")})),
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
struct UploadMeta {
    #[allow(dead_code)]
    filename: String,
    content_type: String,
}

/// In-memory upload metadata registry.
static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Allowed content type prefixes for upload.
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/", "text/", "application/pdf", "audio/"];

fn is_allowed_content_type(ct: &str) -> bool {
    ALLOWED_CONTENT_TYPES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
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
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Validate agent ID format
    let _agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
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
            Json(
                serde_json::json!({"error": "Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf"}),
            ),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Validate size
    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": format!("File too large (max {} MB)", MAX_UPLOAD_SIZE / (1024 * 1024))}),
            ),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty file body"})),
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to create upload directory"})),
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save file"})),
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
        match state
            .kernel
            .media_engine
            .transcribe_audio(&attachment)
            .await
        {
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
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&id) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Agent not found"})),
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

    let receipts = state.kernel.delivery_tracker.get_receipts(agent_id, limit);
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
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

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
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    // Session-level token/tool stats from the scheduler (in-memory, windowed).
    let (sched_tokens, sched_tool_calls) =
        state.kernel.scheduler.get_usage(agent_id).unwrap_or((0, 0));

    // Persistent usage summary from the UsageStore (SQLite).
    let usage_summary = state
        .kernel
        .memory
        .usage()
        .query_summary(Some(agent_id))
        .ok();

    // Message count from the active session.
    let message_count: u64 = state
        .kernel
        .memory
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
        .audit_log
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
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Verify the agent exists.
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
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
        .audit_log
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
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(home_dir.join("webhooks.json")),
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

    #[tokio::test]
    async fn test_agent_metrics_returns_json_shape_for_existing_agent() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "metrics-shape");

        let (status, body) =
            json_response(agent_metrics(State(state), Path(agent_id.to_string())).await).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["agent_id"], agent_id.to_string());
        assert!(body["token_usage"].is_object());
        assert!(body["tool_calls"].is_object());
        assert!(body.get("avg_response_time_ms").is_some());
    }

    #[tokio::test]
    async fn test_agent_metrics_returns_not_found_for_unknown_agent() {
        let (state, _tmp) = monitoring_test_app_state();

        let (status, body) =
            json_response(agent_metrics(State(state), Path(AgentId::new().to_string())).await)
                .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Agent not found");
    }

    #[tokio::test]
    async fn test_agent_logs_filters_level_by_exact_match() {
        let (state, _tmp) = monitoring_test_app_state();
        let agent_id = spawn_monitoring_test_agent(&state, "logs-filter");
        let agent_id_str = agent_id.to_string();

        state.kernel.audit_log.record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "exact match target",
            "custom_error",
        );
        state.kernel.audit_log.record(
            agent_id_str.clone(),
            AuditAction::AgentMessage,
            "should not match substring filter",
            "not_custom_error",
        );

        let mut params = HashMap::new();
        params.insert("level".to_string(), "custom_error".to_string());

        let (status, body) =
            json_response(agent_logs(State(state), Path(agent_id_str), Query(params)).await).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["count"], 1);

        let logs = body["logs"].as_array().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["outcome"], "custom_error");
    }
}
