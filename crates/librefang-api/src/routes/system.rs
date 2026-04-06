//! Audit, logging, tools, profiles, templates, memory, approvals,
//! bindings, pairing, webhooks, and miscellaneous system handlers.

use super::AppState;

/// Build routes for the system miscellaneous domain (audit, logs, tools, sessions, approvals, pairing, etc.).
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        // Profiles and templates
        .route("/profiles", axum::routing::get(list_profiles))
        .route("/profiles/{name}", axum::routing::get(get_profile))
        .route("/templates", axum::routing::get(list_agent_templates))
        .route("/templates/{name}", axum::routing::get(get_agent_template))
        // Agent KV storage
        .route(
            "/memory/agents/{id}/kv",
            axum::routing::get(get_agent_kv),
        )
        .route(
            "/memory/agents/{id}/kv/{key}",
            axum::routing::get(get_agent_kv_key)
                .put(set_agent_kv_key)
                .delete(delete_agent_kv_key),
        )
        .route(
            "/agents/{id}/memory/export",
            axum::routing::get(export_agent_memory),
        )
        .route(
            "/agents/{id}/memory/import",
            axum::routing::post(import_agent_memory),
        )
        // Audit
        .route("/audit/recent", axum::routing::get(audit_recent))
        .route("/audit/verify", axum::routing::get(audit_verify))
        // Log streaming
        .route("/logs/stream", axum::routing::get(logs_stream))
        // Tools
        .route("/tools", axum::routing::get(list_tools))
        .route("/tools/{name}", axum::routing::get(get_tool))
        // Session management
        .route("/sessions", axum::routing::get(list_sessions))
        .route("/sessions/search", axum::routing::get(search_sessions))
        .route("/sessions/cleanup", axum::routing::post(session_cleanup))
        .route(
            "/sessions/{id}",
            axum::routing::get(get_session).delete(delete_session),
        )
        .route(
            "/sessions/{id}/label",
            axum::routing::put(set_session_label),
        )
        .route(
            "/agents/{id}/sessions/by-label/{label}",
            axum::routing::get(find_session_by_label),
        )
        // Approvals — static paths must precede the `{id}` wildcard
        .route(
            "/approvals",
            axum::routing::get(list_approvals).post(create_approval),
        )
        .route("/approvals/batch", axum::routing::post(batch_resolve))
        .route("/approvals/audit", axum::routing::get(audit_log))
        .route("/approvals/count", axum::routing::get(approval_count))
        .route("/approvals/totp/setup", axum::routing::post(totp_setup))
        .route(
            "/approvals/totp/confirm",
            axum::routing::post(totp_confirm),
        )
        .route(
            "/approvals/totp/status",
            axum::routing::get(totp_status),
        )
        .route(
            "/approvals/totp",
            axum::routing::delete(totp_revoke),
        )
        .route("/approvals/{id}", axum::routing::get(get_approval))
        .route(
            "/approvals/{id}/approve",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>,
                 body: Json<ApproveRequestBody>| async move {
                    approve_request(state, id, lang, body).await
                },
            ),
        )
        .route(
            "/approvals/{id}/reject",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>| async move {
                    reject_request(state, id, lang).await
                },
            ),
        )
        .route(
            "/approvals/{id}/modify",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>,
                 body: Json<ModifyRequestBody>| async move {
                    modify_request(state, id, body, lang).await
                },
            ),
        )
        // Webhook triggers (external event injection)
        .route("/hooks/wake", axum::routing::post(webhook_wake))
        .route("/hooks/agent", axum::routing::post(webhook_agent))
        // Chat command endpoints
        .route("/commands", axum::routing::get(list_commands))
        .route("/commands/{name}", axum::routing::get(get_command))
        // Bindings
        .route(
            "/bindings",
            axum::routing::get(list_bindings).post(add_binding),
        )
        .route(
            "/bindings/{index}",
            axum::routing::delete(remove_binding),
        )
        // Pairing
        .route("/pairing/request", axum::routing::post(pairing_request))
        .route(
            "/pairing/complete",
            axum::routing::post(pairing_complete),
        )
        .route("/pairing/devices", axum::routing::get(pairing_devices))
        .route(
            "/pairing/devices/{id}",
            axum::routing::delete(pairing_remove_device),
        )
        .route("/pairing/notify", axum::routing::post(pairing_notify))
        // Backup / restore
        .route("/backup", axum::routing::post(create_backup))
        .route("/backups", axum::routing::get(list_backups))
        .route(
            "/backups/{filename}",
            axum::routing::delete(delete_backup),
        )
        .route("/restore", axum::routing::post(restore_backup))
        // Queue status
        .route("/queue/status", axum::routing::get(queue_status))
        // Task queue management
        .route("/tasks/status", axum::routing::get(task_queue_status))
        .route("/tasks/list", axum::routing::get(task_queue_list))
        .route(
            "/tasks/{id}",
            axum::routing::delete(task_queue_delete),
        )
        .route(
            "/tasks/{id}/retry",
            axum::routing::post(task_queue_retry),
        )
        // Event webhook subscriptions
        .route(
            "/webhooks/events",
            axum::routing::get(list_event_webhooks).post(create_event_webhook),
        )
        .route(
            "/webhooks/events/{id}",
            axum::routing::put(update_event_webhook).delete(delete_event_webhook),
        )
        // Outbound webhook management
        .route(
            "/webhooks",
            axum::routing::get(list_webhooks).post(create_webhook),
        )
        .route(
            "/webhooks/{id}",
            axum::routing::get(get_webhook)
                .put(update_webhook)
                .delete(delete_webhook),
        )
        .route(
            "/webhooks/{id}/test",
            axum::routing::post(test_webhook),
        )
        // Registry schema (machine-parseable content type definitions)
        .route("/registry/schema", axum::routing::get(registry_schema))
        .route(
            "/registry/schema/{content_type}",
            axum::routing::get(registry_schema_by_type),
        )
        // Registry content creation / update
        .route(
            "/registry/content/{content_type}",
            axum::routing::post(create_registry_content)
                .put(update_registry_content),
        )
}
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_runtime::tool_runner::builtin_tool_definitions;
use librefang_types::agent::AgentId;
use librefang_types::agent::AgentManifest;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
#[utoipa::path(
    get,
    path = "/api/profiles",
    tag = "system",
    responses(
        (status = 200, description = "List tool profiles", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_profiles() -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// GET /api/profiles/:name — Get a single profile by name.
#[utoipa::path(get, path = "/api/profiles/{name}", tag = "system", params(("name" = String, Path, description = "Profile name")), responses((status = 200, description = "Profile details", body = serde_json::Value)))]
pub async fn get_profile(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let profiles: &[(&str, ToolProfile)] = &[
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    match profiles.iter().find(|(n, _)| *n == name) {
        Some((n, profile)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": n,
                "tools": profile.tools(),
            })),
        ),
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-profile-not-found", &[("name", &name)]))
                .into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
#[utoipa::path(get, path = "/api/templates", tag = "system", operation_id = "list_agent_templates", responses((status = 200, description = "List templates", body = Vec<serde_json::Value>)))]
pub async fn list_agent_templates() -> impl IntoResponse {
    let agents_dir = librefang_kernel::config::librefang_home()
        .join("workspaces")
        .join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("agent.toml");
                if manifest_path.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let description = std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                        .map(|m| m.description)
                        .unwrap_or_default();

                    templates.push(serde_json::json!({
                        "name": name,
                        "description": description,
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates/:name — Get template details.
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", operation_id = "get_agent_template", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template details", body = serde_json::Value)))]
pub async fn get_agent_template(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agents_dir = librefang_kernel::config::librefang_home()
        .join("workspaces")
        .join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "provider": manifest.model.provider,
                            "model": manifest.model.model,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                ApiErrorResponse::internal(t.t("api-error-template-invalid-manifest"))
                    .into_json_tuple()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            ApiErrorResponse::internal(t.t("api-error-template-read-failed")).into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Memory endpoints
// ---------------------------------------------------------------------------

/// GET /api/memory/agents/:id/kv — List KV pairs for an agent.
#[utoipa::path(get, path = "/api/memory/agents/{id}/kv", tag = "memory", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent KV store", body = serde_json::Value)))]
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };
    match state.kernel.memory_substrate().list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(e) => {
            tracing::warn!("Memory list_kv failed: {e}");
            ApiErrorResponse::internal(t.t("api-error-memory-operation-failed")).into_json_tuple()
        }
    }
}

/// GET /api/memory/agents/:id/kv/:key — Get a specific KV value.
#[utoipa::path(get, path = "/api/memory/agents/{id}/kv/{key}", tag = "memory", params(("id" = String, Path, description = "Agent ID"), ("key" = String, Path, description = "Key name")), responses((status = 200, description = "KV value", body = serde_json::Value)))]
pub async fn get_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };
    match state
        .kernel
        .memory_substrate()
        .structured_get(agent_id, &key)
    {
        Ok(Some(val)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": val})),
        ),
        Ok(None) => {
            ApiErrorResponse::not_found(t.t("api-error-kv-key-not-found")).into_json_tuple()
        }
        Err(e) => {
            tracing::warn!("Memory get failed for key '{key}': {e}");
            ApiErrorResponse::internal(t.t("api-error-memory-operation-failed")).into_json_tuple()
        }
    }
}

/// PUT /api/memory/agents/:id/kv/:key — Set a KV value.
#[utoipa::path(put, path = "/api/memory/agents/{id}/kv/{key}", tag = "memory", params(("id" = String, Path, description = "Agent ID"), ("key" = String, Path, description = "Key name")), request_body = serde_json::Value, responses((status = 200, description = "KV value set", body = serde_json::Value)))]
pub async fn set_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };
    let value = body.get("value").cloned().unwrap_or(body);

    match state
        .kernel
        .memory_substrate()
        .structured_set(agent_id, &key, value)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory set failed for key '{key}': {e}");
            ApiErrorResponse::internal(t.t("api-error-memory-operation-failed")).into_json_tuple()
        }
    }
}

/// DELETE /api/memory/agents/:id/kv/:key — Delete a KV value.
#[utoipa::path(delete, path = "/api/memory/agents/{id}/kv/{key}", tag = "memory", params(("id" = String, Path, description = "Agent ID"), ("key" = String, Path, description = "Key name")), responses((status = 200, description = "KV key deleted")))]
pub async fn delete_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };
    match state
        .kernel
        .memory_substrate()
        .structured_delete(agent_id, &key)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory delete failed for key '{key}': {e}");
            ApiErrorResponse::internal(t.t("api-error-memory-operation-failed")).into_json_tuple()
        }
    }
}

/// GET /api/agents/:id/memory/export — Export all KV memory for an agent as JSON.
#[utoipa::path(get, path = "/api/agents/{id}/memory/export", tag = "memory", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Exported memory", body = serde_json::Value)))]
pub async fn export_agent_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };

    // Verify agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return ApiErrorResponse::not_found(t.t("api-error-agent-not-found")).into_json_tuple();
    }

    match state.kernel.memory_substrate().list_kv(agent_id) {
        Ok(pairs) => {
            let kv_map: serde_json::Map<String, serde_json::Value> = pairs.into_iter().collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "agent_id": agent_id.0.to_string(),
                    "version": 1,
                    "kv": kv_map,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Memory export failed for agent {agent_id}: {e}");
            ApiErrorResponse::internal(t.t("api-error-kv-export-failed")).into_json_tuple()
        }
    }
}

/// POST /api/agents/:id/memory/import — Import KV memory from JSON into an agent.
///
/// Accepts a JSON body with a `kv` object mapping string keys to JSON values.
/// Optionally accepts `clear_existing: true` to wipe existing memory before import.
#[utoipa::path(post, path = "/api/agents/{id}/memory/import", tag = "memory", params(("id" = String, Path, description = "Agent ID")), request_body = serde_json::Value, responses((status = 200, description = "Memory imported", body = serde_json::Value)))]
pub async fn import_agent_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };

    // Verify agent exists
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return ApiErrorResponse::not_found(t.t("api-error-agent-not-found")).into_json_tuple();
    }

    let kv = match body.get("kv").and_then(|v| v.as_object()) {
        Some(obj) => obj.clone(),
        None => {
            return ApiErrorResponse::bad_request(t.t("api-error-kv-missing-kv-object"))
                .into_json_tuple();
        }
    };

    let clear_existing = body
        .get("clear_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Clear existing memory if requested
    if clear_existing {
        match state.kernel.memory_substrate().list_kv(agent_id) {
            Ok(existing) => {
                for (key, _) in existing {
                    if let Err(e) = state
                        .kernel
                        .memory_substrate()
                        .structured_delete(agent_id, &key)
                    {
                        tracing::warn!("Failed to delete key '{key}' during import clear: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to list existing KV during import clear: {e}");
                return ApiErrorResponse::internal(t.t("api-error-kv-import-clear-failed"))
                    .into_json_tuple();
            }
        }
    }

    let mut imported = 0u64;
    let mut errors = Vec::new();

    for (key, value) in &kv {
        match state
            .kernel
            .memory_substrate()
            .structured_set(agent_id, key, value.clone())
        {
            Ok(()) => imported += 1,
            Err(e) => {
                tracing::warn!("Memory import failed for key '{key}': {e}");
                errors.push(key.clone());
            }
        }
    }

    if errors.is_empty() {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "imported",
                "keys_imported": imported,
            })),
        )
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "partial",
                "keys_imported": imported,
                "failed_keys": errors,
            })),
        )
    }
}

// ---------------------------------------------------------------------------
// Audit endpoints
// ---------------------------------------------------------------------------

/// GET /api/audit/recent — Get recent audit log entries.
#[utoipa::path(get, path = "/api/audit/recent", tag = "system", responses((status = 200, description = "Recent audit entries", body = Vec<serde_json::Value>)))]
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000); // Cap at 1000

    let entries = state.kernel.audit().recent(n);
    let tip = state.kernel.audit().tip_hash();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit().len(),
        "tip_hash": tip,
    }))
}

/// GET /api/audit/verify — Verify the audit chain integrity.
#[utoipa::path(get, path = "/api/audit/verify", tag = "system", responses((status = 200, description = "Audit verification result", body = serde_json::Value)))]
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entry_count = state.kernel.audit().len();
    match state.kernel.audit().verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit().tip_hash(),
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit().tip_hash(),
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })),
    }
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///   - `token`  — auth token (for EventSource clients that cannot set headers)
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second and sends only new entries
/// (tracked by sequence number). On first connect, existing entries are sent
/// as a backfill so the client has immediate context.
#[utoipa::path(get, path = "/api/logs/stream", tag = "system", responses((status = 200, description = "SSE log stream")))]
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = state.kernel.audit().recent(200);

            for entry in &entries {
                // On first poll, send all existing entries as backfill.
                // After that, only send entries newer than last_seq.
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}

// ---------------------------------------------------------------------------
// Tools endpoint
// ---------------------------------------------------------------------------

/// GET /api/tools — List all tool definitions (built-in + MCP).
#[utoipa::path(
    get,
    path = "/api/tools",
    tag = "skills",
    responses(
        (status = 200, description = "List available tools", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tools: Vec<serde_json::Value> = builtin_tool_definitions()
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    // Include MCP tools so they're visible in Settings -> Tools
    if let Ok(mcp_tools) = state.kernel.mcp_tools_ref().lock() {
        for t in mcp_tools.iter() {
            tools.push(serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "source": "mcp",
            }));
        }
    }

    Json(serde_json::json!({"tools": tools, "total": tools.len()}))
}

/// GET /api/tools/:name — Get a single tool definition by name.
#[utoipa::path(get, path = "/api/tools/{name}", tag = "skills", params(("name" = String, Path, description = "Tool name")), responses((status = 200, description = "Tool details", body = serde_json::Value)))]
pub async fn get_tool(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let tr = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Search built-in tools first
    for t in builtin_tool_definitions() {
        if t.name == name {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })),
            );
        }
    }

    // Search MCP tools
    if let Ok(mcp_tools) = state.kernel.mcp_tools_ref().lock() {
        for t in mcp_tools.iter() {
            if t.name == name {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                        "source": "mcp",
                    })),
                );
            }
        }
    }

    ApiErrorResponse::not_found(tr.t_args("api-error-tool-not-found", &[("name", &name)]))
        .into_json_tuple()
}

// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// GET /api/sessions — List all sessions with metadata.
#[utoipa::path(
    get,
    path = "/api/sessions",
    tag = "sessions",
    responses(
        (status = 200, description = "List sessions", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory_substrate().list_sessions() {
        Ok(sessions) => Json(serde_json::json!({"sessions": sessions})),
        Err(_) => Json(serde_json::json!({"sessions": []})),
    }
}

/// GET /api/sessions/:id — Get a single session by ID.
#[utoipa::path(get, path = "/api/sessions/{id}", tag = "sessions", params(("id" = String, Path, description = "Session ID")), responses((status = 200, description = "Session found", body = serde_json::Value), (status = 404, description = "Session not found")))]
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => librefang_types::agent::SessionId(u),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .into_json_tuple();
        }
    };

    match state
        .kernel
        .memory_substrate()
        .get_session_with_created_at(session_id)
    {
        Ok(Some((session, created_at))) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "message_count": session.messages.len(),
                "messages": session.messages,
                "context_window_tokens": session.context_window_tokens,
                "label": session.label,
                "created_at": created_at,
            })),
        ),
        Ok(None) => {
            ApiErrorResponse::not_found(t.t("api-error-session-not-found")).into_json_tuple()
        }
        Err(e) => {
            ApiErrorResponse::internal(t.t_args("api-error-generic", &[("error", &e.to_string())]))
                .into_json_tuple()
        }
    }
}

/// DELETE /api/sessions/:id — Delete a session.
#[utoipa::path(delete, path = "/api/sessions/{id}", tag = "sessions", params(("id" = String, Path, description = "Session ID")), responses((status = 200, description = "Session deleted")))]
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => librefang_types::agent::SessionId(u),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .into_json_tuple();
        }
    };

    match state.kernel.memory_substrate().delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(e) => {
            ApiErrorResponse::internal(t.t_args("api-error-generic", &[("error", &e.to_string())]))
                .into_json_tuple()
        }
    }
}

/// PUT /api/sessions/:id/label — Set a session label.
#[utoipa::path(put, path = "/api/sessions/{id}/label", tag = "sessions", params(("id" = String, Path, description = "Session ID")), request_body = serde_json::Value, responses((status = 200, description = "Label set", body = serde_json::Value)))]
pub async fn set_session_label(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => librefang_types::agent::SessionId(u),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .into_json_tuple();
        }
    };

    let label = req.get("label").and_then(|v| v.as_str());

    // Validate label if present
    if let Some(lbl) = label {
        if let Err(e) = librefang_types::agent::SessionLabel::new(lbl) {
            return ApiErrorResponse::bad_request(
                t.t_args("api-error-generic", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    }

    match state
        .kernel
        .memory_substrate()
        .set_session_label(session_id, label)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
            })),
        ),
        Err(e) => {
            ApiErrorResponse::internal(t.t_args("api-error-generic", &[("error", &e.to_string())]))
                .into_json_tuple()
        }
    }
}

/// GET /api/sessions/by-label/:label — Find session by label (scoped to agent).
#[utoipa::path(get, path = "/api/agents/{id}/sessions/by-label/{label}", tag = "sessions", params(("id" = String, Path, description = "Agent ID"), ("label" = String, Path, description = "Session label")), responses((status = 200, description = "Session found", body = serde_json::Value)))]
pub async fn find_session_by_label(
    State(state): State<Arc<AppState>>,
    Path((agent_id_str, label)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id = match agent_id_str.parse::<uuid::Uuid>() {
        Ok(u) => librefang_types::agent::AgentId(u),
        Err(_) => {
            // Try name lookup
            match state.kernel.agent_registry().find_by_name(&agent_id_str) {
                Some(entry) => entry.id,
                None => {
                    return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
                        .into_json_tuple();
                }
            }
        }
    };

    match state
        .kernel
        .memory_substrate()
        .find_session_by_label(agent_id, &label)
    {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => {
            ApiErrorResponse::not_found(t.t("api-error-session-no-label")).into_json_tuple()
        }
        Err(e) => {
            ApiErrorResponse::internal(t.t_args("api-error-generic", &[("error", &e.to_string())]))
                .into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Session cleanup endpoint
// ---------------------------------------------------------------------------

/// POST /api/sessions/cleanup — Manually trigger session retention cleanup.
///
/// Runs both expired-session and excess-session cleanup using the configured
/// `[session]` policy. Returns `{"sessions_deleted": N}`.
#[utoipa::path(post, path = "/api/sessions/cleanup", tag = "sessions", responses((status = 200, description = "Cleanup result", body = serde_json::Value)))]
pub async fn session_cleanup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let kcfg = state.kernel.config_ref();
    let cfg = &kcfg.session;
    let mut total: u64 = 0;

    if cfg.retention_days > 0 {
        match state
            .kernel
            .memory_substrate()
            .cleanup_expired_sessions(cfg.retention_days)
        {
            Ok(n) => total += n,
            Err(e) => {
                return ApiErrorResponse::internal(t.t_args(
                    "api-error-session-cleanup-expired-failed",
                    &[("error", &e.to_string())],
                ))
                .into_json_tuple();
            }
        }
    }

    if cfg.max_sessions_per_agent > 0 {
        match state
            .kernel
            .memory_substrate()
            .cleanup_excess_sessions(cfg.max_sessions_per_agent)
        {
            Ok(n) => total += n,
            Err(e) => {
                return ApiErrorResponse::internal(t.t_args(
                    "api-error-session-cleanup-excess-failed",
                    &[("error", &e.to_string())],
                ))
                .into_json_tuple();
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"sessions_deleted": total})),
    )
}

/// GET /api/sessions/search?q=...&agent_id=... — Full-text search across session content.
#[utoipa::path(
    get,
    path = "/api/sessions/search",
    tag = "sessions",
    params(
        ("q" = String, Query, description = "FTS5 search query"),
        ("agent_id" = Option<String>, Query, description = "Optional agent ID filter"),
    ),
    responses(
        (status = 200, description = "Search results", body = serde_json::Value),
        (status = 400, description = "Missing query parameter"),
    )
)]
pub async fn search_sessions(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let query = match params.get("q") {
        Some(q) if !q.is_empty() => q.clone(),
        _ => {
            return ApiErrorResponse::bad_request("missing or empty 'q' parameter")
                .into_json_tuple();
        }
    };

    let agent_id = params.get("agent_id").and_then(|id| {
        uuid::Uuid::parse_str(id)
            .ok()
            .map(librefang_types::agent::AgentId)
    });

    match state
        .kernel
        .memory_substrate()
        .search_sessions(&query, agent_id.as_ref())
    {
        Ok(results) => (
            StatusCode::OK,
            Json(serde_json::json!({"results": results})),
        ),
        Err(e) => ApiErrorResponse::internal(e.to_string()).into_json_tuple(),
    }
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approvals()
// ---------------------------------------------------------------------------

/// Serialize an [`ApprovalRequest`] to the JSON shape expected by the dashboard.
///
/// Adds alias fields: `action` (= `action_summary`), `agent_name`, `created_at` (= `requested_at`).
fn approval_to_json(
    a: &librefang_types::approval::ApprovalRequest,
    registry_agents: &[librefang_types::agent::AgentEntry],
) -> serde_json::Value {
    let agent_name = registry_agents
        .iter()
        .find(|ag| ag.id.to_string() == a.agent_id || ag.name == a.agent_id)
        .map(|ag| ag.name.as_str())
        .unwrap_or(&a.agent_id);
    serde_json::json!({
        "id": a.id,
        "agent_id": a.agent_id,
        "agent_name": agent_name,
        "tool_name": a.tool_name,
        "description": a.description,
        "action_summary": a.action_summary,
        "action": a.action_summary,
        "risk_level": a.risk_level,
        "requested_at": a.requested_at,
        "created_at": a.requested_at,
        "timeout_secs": a.timeout_secs,
        "status": "pending"
    })
}

/// GET /api/approvals — List pending and recent approval requests.
///
/// Transforms field names to match the dashboard template expectations:
/// `action_summary` → `action`, `agent_id` → `agent_name`, `requested_at` → `created_at`.
#[utoipa::path(get, path = "/api/approvals", tag = "approvals", responses((status = 200, description = "List pending and recent approvals", body = Vec<serde_json::Value>)))]
pub async fn list_approvals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approvals().list_pending();
    let recent = state.kernel.approvals().list_recent(50);

    let registry_agents = state.kernel.agent_registry().list();
    let agent_name_for = |agent_id: &str| {
        registry_agents
            .iter()
            .find(|ag| ag.id.to_string() == agent_id || ag.name == agent_id)
            .map(|ag| ag.name.clone())
            .unwrap_or_else(|| agent_id.to_string())
    };

    let mut approvals: Vec<serde_json::Value> = pending
        .iter()
        .map(|a| approval_to_json(a, &registry_agents))
        .collect();

    approvals.extend(recent.into_iter().map(|record| {
        let request = record.request;
        let agent_name = agent_name_for(&request.agent_id);
        let status = match record.decision {
            librefang_types::approval::ApprovalDecision::Approved => "approved",
            librefang_types::approval::ApprovalDecision::Denied => "rejected",
            librefang_types::approval::ApprovalDecision::TimedOut => "expired",
            librefang_types::approval::ApprovalDecision::ModifyAndRetry { .. } => {
                "modify_and_retry"
            }
            librefang_types::approval::ApprovalDecision::Skipped => "skipped",
        };
        serde_json::json!({
            "id": request.id,
            "agent_id": request.agent_id,
            "agent_name": agent_name,
            "tool_name": request.tool_name,
            "description": request.description,
            "action_summary": request.action_summary,
            "action": request.action_summary,
            "risk_level": request.risk_level,
            "requested_at": request.requested_at,
            "created_at": request.requested_at,
            "timeout_secs": request.timeout_secs,
            "status": status,
            "decided_at": record.decided_at,
            "decided_by": record.decided_by,
        })
    }));

    approvals.sort_by(|a, b| {
        let a_pending = a["status"].as_str() == Some("pending");
        let b_pending = b["status"].as_str() == Some("pending");
        b_pending
            .cmp(&a_pending)
            .then_with(|| b["created_at"].as_str().cmp(&a["created_at"].as_str()))
    });

    let total = approvals.len();

    Json(serde_json::json!({"approvals": approvals, "total": total}))
}

/// GET /api/approvals/{id} — Get a single approval request by ID.
#[utoipa::path(get, path = "/api/approvals/{id}", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Single approval request", body = serde_json::Value), (status = 404, description = "Approval not found")))]
pub async fn get_approval(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple();
        }
    };

    match state.kernel.approvals().get_pending(uuid) {
        Some(a) => {
            let registry_agents = state.kernel.agent_registry().list();
            (StatusCode::OK, Json(approval_to_json(&a, &registry_agents)))
        }
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-approval-not-found", &[("id", &id)]))
                .into_json_tuple()
        }
    }
}

/// POST /api/approvals — Create a manual approval request (for external systems).
///
/// Note: Most approval requests are created automatically by the tool_runner
/// when an agent invokes a tool that requires approval. This endpoint exists
/// for external integrations that need to inject approval gates.
#[derive(serde::Deserialize)]
pub struct CreateApprovalRequest {
    pub agent_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub action_summary: String,
}

#[utoipa::path(post, path = "/api/approvals", tag = "approvals", request_body = serde_json::Value, responses((status = 200, description = "Approval created", body = serde_json::Value)))]
pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    use librefang_types::approval::{ApprovalRequest, RiskLevel};

    let policy = state.kernel.approvals().policy();
    let id = uuid::Uuid::new_v4();
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_summary: if req.action_summary.is_empty() {
            req.tool_name.clone()
        } else {
            req.action_summary
        },
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
        sender_id: None,
        channel: None,
        route_to: Vec::new(),
        escalation_count: 0,
    };

    // Spawn the request in the background (it will block until resolved or timed out)
    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approvals().request_approval(approval_req).await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve — Approve a pending request.
///
/// When TOTP is enabled, the request body must include a `totp_code` field.
#[derive(serde::Deserialize, Default)]
pub struct ApproveRequestBody {
    #[serde(default)]
    totp_code: Option<String>,
}

#[utoipa::path(post, path = "/api/approvals/{id}/approve", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), request_body = serde_json::Value, responses((status = 200, description = "Request approved", body = serde_json::Value)))]
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<ApproveRequestBody>,
) -> axum::response::Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    // Verify TOTP code or recovery code if second factor is enabled
    let totp_verified = if state.kernel.approvals().requires_totp() {
        if state.kernel.approvals().is_totp_locked_out("api_admin") {
            return ApiErrorResponse::bad_request(
                "Too many failed TOTP attempts. Try again later.",
            )
            .into_json_tuple()
            .into_response();
        }
        match body.totp_code.as_deref() {
            Some(code) => {
                if code.contains('-') {
                    match state.kernel.vault_get("totp_recovery_codes") {
                        Some(stored) => {
                            match librefang_kernel::approval::ApprovalManager::verify_recovery_code(
                                &stored, code,
                            ) {
                                Ok((true, updated)) => {
                                    let _ = state.kernel.vault_set("totp_recovery_codes", &updated);
                                    true
                                }
                                Ok((false, _)) => {
                                    return ApiErrorResponse::bad_request("Invalid recovery code")
                                        .into_json_tuple()
                                        .into_response();
                                }
                                Err(e) => {
                                    return ApiErrorResponse::bad_request(e)
                                        .into_json_tuple()
                                        .into_response();
                                }
                            }
                        }
                        None => {
                            return ApiErrorResponse::bad_request("No recovery codes configured")
                                .into_json_tuple()
                                .into_response();
                        }
                    }
                } else {
                    let secret = match state.kernel.vault_get("totp_secret") {
                        Some(s) => s,
                        None => {
                            return ApiErrorResponse::bad_request(
                                "TOTP not configured. Run POST /api/approvals/totp/setup first.",
                            )
                            .into_json_tuple()
                            .into_response();
                        }
                    };
                    match librefang_kernel::approval::ApprovalManager::verify_totp_code(
                        &secret, code,
                    ) {
                        Ok(true) => true,
                        Ok(false) => {
                            state.kernel.approvals().record_totp_failure("api_admin");
                            return ApiErrorResponse::bad_request("Invalid TOTP code")
                                .into_json_tuple()
                                .into_response();
                        }
                        Err(e) => {
                            return ApiErrorResponse::bad_request(e)
                                .into_json_tuple()
                                .into_response();
                        }
                    }
                }
            }
            None => false,
        }
    } else {
        false
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::Approved,
            Some("api".to_string()),
            totp_verified,
            Some("api_admin"),
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "approved", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_json_tuple().into_response(),
    }
}

/// POST /api/approvals/{id}/reject — Reject a pending request.
#[utoipa::path(post, path = "/api/approvals/{id}/reject", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Request rejected", body = serde_json::Value)))]
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::Denied,
            Some("api".to_string()),
            false,
            None,
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "rejected", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::not_found(e).into_json_tuple().into_response(),
    }
}

// ---------------------------------------------------------------------------
// Approval — modify, batch, audit, count
// ---------------------------------------------------------------------------

/// POST /api/approvals/{id}/modify — Return a pending request with feedback for modification.
#[derive(serde::Deserialize)]
pub struct ModifyRequestBody {
    #[serde(default)]
    feedback: String,
}

#[utoipa::path(post, path = "/api/approvals/{id}/modify", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), request_body = serde_json::Value, responses((status = 200, description = "Request modified", body = serde_json::Value)))]
pub async fn modify_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ModifyRequestBody>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    // Truncate feedback to prevent database bloat
    let feedback: String = body
        .feedback
        .chars()
        .take(librefang_types::approval::MAX_APPROVAL_FEEDBACK_LEN)
        .collect();
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::ModifyAndRetry { feedback },
            Some("api".to_string()),
            false,
            None,
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "modified", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::not_found(e).into_json_tuple().into_response(),
    }
}

/// POST /api/approvals/batch — Batch resolve multiple pending requests.
#[derive(serde::Deserialize)]
pub struct BatchResolveRequest {
    ids: Vec<String>,
    decision: String,
}

#[utoipa::path(post, path = "/api/approvals/batch", tag = "approvals", request_body = serde_json::Value, responses((status = 200, description = "Batch resolve results", body = serde_json::Value)))]
pub async fn batch_resolve(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BatchResolveRequest>,
) -> impl IntoResponse {
    const MAX_BATCH_SIZE: usize = 100;

    if body.ids.len() > MAX_BATCH_SIZE {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("batch size {} exceeds maximum {MAX_BATCH_SIZE}", body.ids.len())}),
            ),
        );
    }

    let decision = match body.decision.as_str() {
        "approve" => librefang_types::approval::ApprovalDecision::Approved,
        "reject" => librefang_types::approval::ApprovalDecision::Denied,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("invalid decision: {other}, expected 'approve' or 'reject'")}),
                ),
            );
        }
    };

    // Parse UUIDs, returning error entries for invalid ones
    let mut result_json: Vec<serde_json::Value> = Vec::with_capacity(body.ids.len());
    let mut valid_uuids = Vec::new();
    for id_str in &body.ids {
        match uuid::Uuid::parse_str(id_str) {
            Ok(uuid) => valid_uuids.push(uuid),
            Err(_) => {
                result_json.push(serde_json::json!({
                    "id": id_str, "status": "error", "message": "invalid UUID"
                }));
            }
        }
    }

    for uuid in valid_uuids {
        let id = uuid.to_string();
        match state
            .kernel
            .resolve_tool_approval(uuid, decision.clone(), Some("api".to_string()), false, None)
            .await
        {
            Ok((resp, _)) => result_json.push(serde_json::json!({
                "id": id,
                "status": "ok",
                "decision": resp.decision.as_str(),
                "decided_at": resp.decided_at.to_rfc3339(),
            })),
            Err(e) => {
                result_json.push(serde_json::json!({"id": id, "status": "error", "message": e}))
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"results": result_json})),
    )
}

/// GET /api/approvals/audit — Query the persistent approval audit log.
#[derive(serde::Deserialize)]
pub struct AuditQueryParams {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
    agent_id: Option<String>,
    tool_name: Option<String>,
}

fn default_audit_limit() -> usize {
    50
}

#[utoipa::path(get, path = "/api/approvals/audit", tag = "approvals", params(("limit" = Option<usize>, Query, description = "Max entries"), ("offset" = Option<usize>, Query, description = "Offset"), ("agent_id" = Option<String>, Query, description = "Filter by agent"), ("tool_name" = Option<String>, Query, description = "Filter by tool")), responses((status = 200, description = "Audit log entries", body = serde_json::Value)))]
pub async fn audit_log(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditQueryParams>,
) -> impl IntoResponse {
    const MAX_AUDIT_LIMIT: usize = 500;
    let limit = params.limit.min(MAX_AUDIT_LIMIT);
    let entries = state.kernel.approvals().query_audit(
        limit,
        params.offset,
        params.agent_id.as_deref(),
        params.tool_name.as_deref(),
    );
    let total = state
        .kernel
        .approvals()
        .audit_count(params.agent_id.as_deref(), params.tool_name.as_deref());

    Json(serde_json::json!({"entries": entries, "total": total}))
}

/// GET /api/approvals/count — Lightweight pending count for notification badges.
#[utoipa::path(get, path = "/api/approvals/count", tag = "approvals", responses((status = 200, description = "Pending approval count", body = serde_json::Value)))]
pub async fn approval_count(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approvals().pending_count();
    Json(serde_json::json!({"pending": pending}))
}

// ---------------------------------------------------------------------------
// TOTP setup endpoints
// ---------------------------------------------------------------------------

/// POST /api/approvals/totp/setup — Generate a new TOTP secret and return a provisioning URI.
///
/// The secret is stored in the vault but not yet active. The user must call
/// `/api/approvals/totp/confirm` with a valid code to activate TOTP.
///
/// If TOTP is already confirmed, the request body must include a valid
/// `current_code` (TOTP or recovery code) to authorize the reset.
#[derive(serde::Deserialize, Default)]
pub struct TotpSetupBody {
    /// Required when resetting an already-confirmed TOTP enrollment.
    #[serde(default)]
    current_code: Option<String>,
}

pub async fn totp_setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpSetupBody>,
) -> impl IntoResponse {
    // If TOTP is already confirmed, require verification of the old code
    let already_confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");

    if already_confirmed {
        match body.current_code.as_deref() {
            None => {
                return ApiErrorResponse::bad_request(
                    "TOTP is already enrolled. Provide current_code (TOTP or recovery code) to reset.",
                )
                .into_json_tuple();
            }
            Some(code) => {
                let verified = if code.contains('-') {
                    // Recovery code
                    match state.kernel.vault_get("totp_recovery_codes") {
                        Some(stored) => {
                            match librefang_kernel::approval::ApprovalManager::verify_recovery_code(
                                &stored, code,
                            ) {
                                Ok((true, updated)) => {
                                    let _ = state.kernel.vault_set("totp_recovery_codes", &updated);
                                    true
                                }
                                _ => false,
                            }
                        }
                        None => false,
                    }
                } else {
                    // TOTP code
                    match state.kernel.vault_get("totp_secret") {
                        Some(secret) => {
                            librefang_kernel::approval::ApprovalManager::verify_totp_code(
                                &secret, code,
                            )
                            .unwrap_or(false)
                        }
                        None => false,
                    }
                };
                if !verified {
                    return ApiErrorResponse::bad_request(
                        "Invalid current_code. Provide a valid TOTP or recovery code to reset.",
                    )
                    .into_json_tuple();
                }
            }
        }
    }

    let issuer = {
        let policy = state.kernel.approvals().policy();
        policy.totp_issuer.clone()
    };

    let (secret_base32, otpauth_uri, qr_base64) =
        match librefang_kernel::approval::ApprovalManager::generate_totp_secret(&issuer, "admin") {
            Ok(v) => v,
            Err(e) => {
                return ApiErrorResponse::internal(e).into_json_tuple();
            }
        };
    let qr_data_uri = format!("data:image/png;base64,{qr_base64}");

    // Generate recovery codes
    let recovery_codes = librefang_kernel::approval::ApprovalManager::generate_recovery_codes();
    let recovery_json = serde_json::to_string(&recovery_codes).unwrap_or_default();

    // Store secret and recovery codes in vault (not yet active — totp_confirmed = false)
    if let Err(e) = state.kernel.vault_set("totp_secret", &secret_base32) {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }
    if let Err(e) = state.kernel.vault_set("totp_confirmed", "false") {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }
    if let Err(e) = state
        .kernel
        .vault_set("totp_recovery_codes", &recovery_json)
    {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "otpauth_uri": otpauth_uri,
            "secret": secret_base32,
            "qr_code": qr_data_uri,
            "recovery_codes": recovery_codes,
            "message": "Scan the QR code or enter the secret in your authenticator app, then call POST /api/approvals/totp/confirm with a valid code. Save your recovery codes in a safe place."
        })),
    )
}

/// POST /api/approvals/totp/confirm — Confirm TOTP enrollment by verifying a code.
#[derive(serde::Deserialize)]
pub struct TotpConfirmBody {
    code: String,
}

pub async fn totp_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpConfirmBody>,
) -> impl IntoResponse {
    let secret = match state.kernel.vault_get("totp_secret") {
        Some(s) => s,
        None => {
            return ApiErrorResponse::bad_request(
                "No TOTP secret found. Run POST /api/approvals/totp/setup first.",
            )
            .into_json_tuple();
        }
    };

    match librefang_kernel::approval::ApprovalManager::verify_totp_code(&secret, &body.code) {
        Ok(true) => {
            if let Err(e) = state.kernel.vault_set("totp_confirmed", "true") {
                return ApiErrorResponse::internal(e).into_json_tuple();
            }
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "confirmed", "message": "TOTP is now active. Set second_factor = \"totp\" in your config to enforce it."}),
                ),
            )
        }
        Ok(false) => ApiErrorResponse::bad_request(
            "Invalid TOTP code. Check your authenticator app and try again.",
        )
        .into_json_tuple(),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// GET /api/approvals/totp/status — Check TOTP enrollment status.
pub async fn totp_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let has_secret = state.kernel.vault_get("totp_secret").is_some();
    let confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");
    let policy = state.kernel.approvals().policy();
    let enforced = policy.second_factor == librefang_types::approval::SecondFactor::Totp;

    let remaining_recovery = state
        .kernel
        .vault_get("totp_recovery_codes")
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.len())
        .unwrap_or(0);

    Json(serde_json::json!({
        "enrolled": has_secret,
        "confirmed": confirmed,
        "enforced": enforced,
        "remaining_recovery_codes": remaining_recovery,
    }))
}

/// DELETE /api/approvals/totp — Revoke TOTP enrollment.
///
/// Requires a valid TOTP or recovery code to authorize revocation.
#[derive(serde::Deserialize)]
pub struct TotpRevokeBody {
    code: String,
}

pub async fn totp_revoke(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpRevokeBody>,
) -> impl IntoResponse {
    let confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");

    if !confirmed {
        return ApiErrorResponse::bad_request("TOTP is not enrolled.").into_json_tuple();
    }

    // Verify the provided code (recovery codes are consumed on use)
    let verified = if body.code.contains('-') {
        match state.kernel.vault_get("totp_recovery_codes") {
            Some(stored) => {
                match librefang_kernel::approval::ApprovalManager::verify_recovery_code(
                    &stored, &body.code,
                ) {
                    Ok((true, updated)) => {
                        let _ = state.kernel.vault_set("totp_recovery_codes", &updated);
                        true
                    }
                    _ => false,
                }
            }
            None => false,
        }
    } else {
        match state.kernel.vault_get("totp_secret") {
            Some(secret) => {
                librefang_kernel::approval::ApprovalManager::verify_totp_code(&secret, &body.code)
                    .unwrap_or(false)
            }
            None => false,
        }
    };

    if !verified {
        return ApiErrorResponse::bad_request(
            "Invalid code. Provide a valid TOTP or recovery code.",
        )
        .into_json_tuple();
    }

    // Remove TOTP data from vault
    // vault_set to empty/false markers (vault doesn't expose remove via kernel helper)
    let _ = state.kernel.vault_set("totp_confirmed", "false");
    let _ = state.kernel.vault_set("totp_secret", "");
    let _ = state.kernel.vault_set("totp_recovery_codes", "[]");

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "revoked",
            "message": "TOTP has been revoked. Set second_factor = \"none\" in config to disable enforcement."
        })),
    )
}

// ---------------------------------------------------------------------------
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
#[utoipa::path(post, path = "/api/hooks/wake", tag = "webhooks", request_body = serde_json::Value, responses((status = 200, description = "Wake hook triggered", body = serde_json::Value)))]
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::WakePayload>,
) -> impl IntoResponse {
    let (err_webhook_not_enabled, err_invalid_token) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-triggers-not-enabled"),
            t.t("api-error-webhook-invalid-token"),
        )
    };
    // Check if webhook triggers are enabled — use config_snapshot()
    // because wh_config is held across .await below.
    let cfg = state.kernel.config_snapshot();
    let wh_config = match &cfg.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_json_tuple();
        }
    };

    // Validate bearer token (constant-time comparison)
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return ApiErrorResponse::bad_request(err_invalid_token).into_json_tuple();
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_json_tuple();
    }

    // Publish through the kernel's publish_event (KernelHandle trait), which
    // goes through the full event processing pipeline including trigger evaluation.
    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        let err_msg = {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            t.t_args(
                "api-error-webhook-publish-failed",
                &[("error", &e.to_string())],
            )
        };
        return ApiErrorResponse::internal(err_msg).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
#[utoipa::path(post, path = "/api/hooks/agent", tag = "webhooks", request_body = serde_json::Value, responses((status = 200, description = "Agent hook triggered", body = serde_json::Value)))]
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::AgentHookPayload>,
) -> impl IntoResponse {
    let (err_webhook_not_enabled, err_invalid_token, err_no_agents) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-triggers-not-enabled"),
            t.t("api-error-webhook-invalid-token"),
            t.t("api-error-webhook-no-agents"),
        )
    };
    // Check if webhook triggers are enabled — use config_snapshot()
    // because wh_config is held across .await below.
    let cfg2 = state.kernel.config_snapshot();
    let wh_config = match &cfg2.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_json_tuple();
        }
    };

    // Validate bearer token
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return ApiErrorResponse::bad_request(err_invalid_token).into_json_tuple();
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_json_tuple();
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.agent_registry().find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        let err_msg = {
                            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
                            t.t_args("api-error-webhook-agent-not-found", &[("id", agent_ref)])
                        };
                        return ApiErrorResponse::not_found(err_msg).into_json_tuple();
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent
            match state.kernel.agent_registry().list().first() {
                Some(entry) => entry.id,
                None => {
                    return ApiErrorResponse::not_found(err_no_agents).into_json_tuple();
                }
            }
        }
    };

    // Actually send the message to the agent and get the response
    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        ),
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            let msg = t.t_args(
                "api-error-webhook-agent-exec-failed",
                &[("error", &e.to_string())],
            );
            ApiErrorResponse::internal(msg).into_json_tuple()
        }
    }
}

// ─── Agent Bindings API ────────────────────────────────────────────────

/// GET /api/bindings — List all agent bindings.
#[utoipa::path(get, path = "/api/bindings", tag = "system", responses((status = 200, description = "List key bindings", body = Vec<serde_json::Value>)))]
pub async fn list_bindings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bindings = state.kernel.list_bindings();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": bindings })),
    )
}

/// POST /api/bindings — Add a new agent binding.
#[utoipa::path(post, path = "/api/bindings", tag = "system", request_body = serde_json::Value, responses((status = 200, description = "Binding added", body = serde_json::Value)))]
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    Json(binding): Json<librefang_types::config::AgentBinding>,
) -> impl IntoResponse {
    // Validate agent exists
    let agents = state.kernel.agent_registry().list();
    let agent_exists = agents.iter().any(|e| e.name == binding.agent)
        || binding.agent.parse::<uuid::Uuid>().is_ok();
    if !agent_exists {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    state.kernel.add_binding(binding);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created" })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
#[utoipa::path(delete, path = "/api/bindings/{index}", tag = "system", params(("index" = u32, Path, description = "Binding index")), responses((status = 200, description = "Binding removed")))]
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match state.kernel.remove_binding(index) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": t.t("api-error-binding-index-out-of-range") })),
        ),
    }
}

// ─── Device Pairing endpoints ───────────────────────────────────────────

/// POST /api/pairing/request — Create a new pairing request (returns token + QR URI).
#[utoipa::path(post, path = "/api/pairing/request", tag = "pairing", responses((status = 200, description = "Pairing request created", body = serde_json::Value)))]
pub async fn pairing_request(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    match state.kernel.pairing_ref().create_pairing_request() {
        Ok(req) => {
            let qr_uri = format!("librefang://pair?token={}", req.token);
            Json(serde_json::json!({
                "token": req.token,
                "qr_uri": qr_uri,
                "expires_at": req.expires_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => ApiErrorResponse::bad_request(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// POST /api/pairing/complete — Complete pairing with token + device info.
#[utoipa::path(post, path = "/api/pairing/complete", tag = "pairing", request_body = serde_json::Value, responses((status = 200, description = "Pairing completed", body = serde_json::Value)))]
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");
    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let platform = body
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let push_token = body
        .get("push_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let device_info = librefang_kernel::pairing::PairedDevice {
        device_id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        platform: platform.to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token,
    };
    match state
        .kernel
        .pairing_ref()
        .complete_pairing(token, device_info)
    {
        Ok(device) => Json(serde_json::json!({
            "device_id": device.device_id,
            "display_name": device.display_name,
            "platform": device.platform,
            "paired_at": device.paired_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// GET /api/pairing/devices — List paired devices.
#[utoipa::path(get, path = "/api/pairing/devices", tag = "pairing", responses((status = 200, description = "List paired devices", body = Vec<serde_json::Value>)))]
pub async fn pairing_devices(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing_ref()
        .list_devices()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "platform": d.platform,
                "paired_at": d.paired_at.to_rfc3339(),
                "last_seen": d.last_seen.to_rfc3339(),
            })
        })
        .collect();
    Json(serde_json::json!({"devices": devices})).into_response()
}

/// DELETE /api/pairing/devices/{id} — Remove a paired device.
#[utoipa::path(delete, path = "/api/pairing/devices/{id}", tag = "pairing", params(("id" = String, Path, description = "Device ID")), responses((status = 200, description = "Device removed")))]
pub async fn pairing_remove_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    match state.kernel.pairing_ref().remove_device(&device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => ApiErrorResponse::not_found(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// POST /api/pairing/notify — Push a notification to all paired devices.
#[utoipa::path(post, path = "/api/pairing/notify", tag = "pairing", request_body = serde_json::Value, responses((status = 200, description = "Notification sent", body = serde_json::Value)))]
pub async fn pairing_notify(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (err_pairing_not_enabled, err_message_required) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-pairing-not-enabled"),
            t.t("api-error-pairing-message-required"),
        )
    };
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(err_pairing_not_enabled)
            .into_json_tuple()
            .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("LibreFang");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return ApiErrorResponse::bad_request(err_message_required)
            .into_json_tuple()
            .into_response();
    }
    state
        .kernel
        .pairing_ref()
        .notify_devices(title, message)
        .await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing_ref().list_devices().len()}))
        .into_response()
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
#[utoipa::path(get, path = "/api/commands", tag = "system", responses((status = 200, description = "List chat commands", body = Vec<serde_json::Value>)))]
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Reset session (clear history)"}),
        serde_json::json!({"cmd": "/reboot", "desc": "Hard reset session (full context clear, no summary)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ];

    // Add skill-registered tool names as potential commands
    if let Ok(registry) = state.kernel.skill_registry_ref().read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// GET /api/commands/{name} — Lookup a single command by name.
#[utoipa::path(get, path = "/api/commands/{name}", tag = "system", params(("name" = String, Path, description = "Command name")), responses((status = 200, description = "Command details", body = serde_json::Value)))]
pub async fn get_command(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Normalise: ensure lookup key has a leading slash
    let lookup = if name.starts_with('/') {
        name.clone()
    } else {
        format!("/{name}")
    };

    // Built-in commands
    let builtins = [
        ("/help", "Show available commands"),
        ("/new", "Reset session (clear history)"),
        (
            "/reboot",
            "Hard reset session (full context clear, no summary)",
        ),
        ("/compact", "Trigger LLM session compaction"),
        ("/model", "Show or switch model (/model [name])"),
        ("/stop", "Cancel current agent run"),
        ("/usage", "Show session token usage & cost"),
        (
            "/think",
            "Toggle extended thinking (/think [on|off|stream])",
        ),
        ("/context", "Show context window usage & pressure"),
        (
            "/verbose",
            "Cycle tool detail level (/verbose [off|on|full])",
        ),
        ("/queue", "Check if agent is processing"),
        ("/status", "Show system status"),
        ("/clear", "Clear chat display"),
        ("/exit", "Disconnect from agent"),
    ];

    for (cmd, desc) in &builtins {
        if cmd.eq_ignore_ascii_case(&lookup) {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"cmd": cmd, "desc": desc})),
            );
        }
    }

    // Skill-registered commands
    if let Ok(registry) = state.kernel.skill_registry_ref().read() {
        for skill in registry.list() {
            let skill_cmd = format!("/{}", skill.manifest.skill.name);
            if skill_cmd.eq_ignore_ascii_case(&lookup) {
                let desc: String = skill.manifest.skill.description.chars().take(80).collect();
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "cmd": skill_cmd,
                        "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                        "source": "skill",
                    })),
                );
            }
        }
    }

    ApiErrorResponse::not_found(t.t_args("api-error-command-not-found", &[("name", &lookup)]))
        .into_json_tuple()
}

// ---------------------------------------------------------------------------
// Backup / Restore endpoints
// ---------------------------------------------------------------------------

/// Metadata stored inside every backup archive as `manifest.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct BackupManifest {
    version: u32,
    created_at: String,
    hostname: String,
    librefang_version: String,
    components: Vec<String>,
}

/// POST /api/backup — Create a backup archive of kernel state.
///
/// Returns the backup metadata including the filename. The archive is stored
/// in `<home_dir>/backups/` with a timestamped filename.
#[utoipa::path(post, path = "/api/backup", tag = "system", responses((status = 200, description = "Backup created", body = serde_json::Value)))]
pub async fn create_backup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let home_dir = &state.kernel.home_dir();
    let backups_dir = home_dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-create-dir-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("librefang_backup_{timestamp}.zip");
    let backup_path = backups_dir.join(&filename);

    let mut components: Vec<String> = Vec::new();

    // Create zip archive
    let file = match std::fs::File::create(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-create-file-failed",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Helper: add a single file to the zip relative to home_dir
    let add_file = |zip: &mut zip::ZipWriter<std::fs::File>,
                    src: &std::path::Path,
                    archive_name: &str|
     -> Result<(), String> {
        let data = std::fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        zip.start_file(archive_name, options)
            .map_err(|e| format!("zip start {archive_name}: {e}"))?;
        std::io::Write::write_all(zip, &data)
            .map_err(|e| format!("zip write {archive_name}: {e}"))?;
        Ok(())
    };

    // Helper: recursively add a directory to the zip
    let add_dir = |zip: &mut zip::ZipWriter<std::fs::File>,
                   dir: &std::path::Path,
                   prefix: &str|
     -> Result<u64, String> {
        let mut count = 0u64;
        if !dir.exists() {
            return Ok(0);
        }
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| format!("strip prefix: {e}"))?;
            let archive_name = if prefix.is_empty() {
                rel.to_string_lossy().to_string()
            } else {
                format!("{prefix}/{}", rel.to_string_lossy())
            };
            if path.is_file() {
                let data =
                    std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
                zip.start_file(&archive_name, options)
                    .map_err(|e| format!("zip start {archive_name}: {e}"))?;
                std::io::Write::write_all(zip, &data)
                    .map_err(|e| format!("zip write {archive_name}: {e}"))?;
                count += 1;
            }
        }
        Ok(count)
    };

    // 1. config.toml
    let config_path = home_dir.join("config.toml");
    if config_path.exists() {
        if let Err(e) = add_file(&mut zip, &config_path, "config.toml") {
            tracing::warn!("Backup: skipping config.toml: {e}");
        } else {
            components.push("config".to_string());
        }
    }

    // 2. cron_jobs.json
    let cron_path = home_dir.join("cron_jobs.json");
    if cron_path.exists() {
        if let Err(e) = add_file(&mut zip, &cron_path, "cron_jobs.json") {
            tracing::warn!("Backup: skipping cron_jobs.json: {e}");
        } else {
            components.push("cron_jobs".to_string());
        }
    }

    // 3. hand_state.json
    let hand_state_path = home_dir.join("hand_state.json");
    if hand_state_path.exists() {
        if let Err(e) = add_file(&mut zip, &hand_state_path, "hand_state.json") {
            tracing::warn!("Backup: skipping hand_state.json: {e}");
        } else {
            components.push("hand_state".to_string());
        }
    }

    // 4. custom_models.json
    let custom_models_path = home_dir.join("custom_models.json");
    if custom_models_path.exists() {
        if let Err(e) = add_file(&mut zip, &custom_models_path, "custom_models.json") {
            tracing::warn!("Backup: skipping custom_models.json: {e}");
        } else {
            components.push("custom_models".to_string());
        }
    }

    // 5. agents/ directory (user templates)
    let agents_dir = home_dir.join("workspaces").join("agents");
    if agents_dir.exists() {
        match add_dir(&mut zip, &agents_dir, "agents") {
            Ok(n) if n > 0 => components.push("agents".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping agents/: {e}"),
        }
    }

    // 6. skills/ directory
    let skills_dir = home_dir.join("skills");
    if skills_dir.exists() {
        match add_dir(&mut zip, &skills_dir, "skills") {
            Ok(n) if n > 0 => components.push("skills".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping skills/: {e}"),
        }
    }

    // 7. workflows/ directory
    let workflows_dir = home_dir.join("workflows");
    if workflows_dir.exists() {
        match add_dir(&mut zip, &workflows_dir, "workflows") {
            Ok(n) if n > 0 => components.push("workflows".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping workflows/: {e}"),
        }
    }

    // 8. data/ directory (SQLite DB, memory, etc.)
    let data_dir = home_dir.join("data");
    if data_dir.exists() {
        match add_dir(&mut zip, &data_dir, "data") {
            Ok(n) if n > 0 => components.push("data".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping data/: {e}"),
        }
    }

    // Write manifest
    let manifest = BackupManifest {
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        hostname: hostname_string(),
        librefang_version: env!("CARGO_PKG_VERSION").to_string(),
        components: components.clone(),
    };
    if let Ok(manifest_json) = serde_json::to_string_pretty(&manifest) {
        let _ = zip.start_file("manifest.json", options).and_then(|()| {
            std::io::Write::write_all(&mut zip, manifest_json.as_bytes())
                .map_err(zip::result::ZipError::Io)
        });
    }

    if let Err(e) = zip.finish() {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-finalize-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    let size = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    tracing::info!(
        "Backup created: {filename} ({} bytes, {} components)",
        size,
        components.len()
    );
    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("Backup created: {filename}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "filename": filename,
            "path": backup_path.to_string_lossy(),
            "size_bytes": size,
            "components": components,
            "created_at": manifest.created_at,
        })),
    )
}

/// GET /api/backups — List existing backups.
#[utoipa::path(get, path = "/api/backups", tag = "system", responses((status = 200, description = "List backups", body = Vec<serde_json::Value>)))]
pub async fn list_backups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let backups_dir = state.kernel.home_dir().join("backups");
    if !backups_dir.exists() {
        return Json(serde_json::json!({"backups": [], "total": 0}));
    }

    let mut backups: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&backups_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("zip") {
                continue;
            }
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                });

            // Try to read manifest from the zip
            let manifest = read_backup_manifest(&path);

            backups.push(serde_json::json!({
                "filename": filename,
                "path": path.to_string_lossy(),
                "size_bytes": size,
                "modified_at": modified,
                "components": manifest.as_ref().map(|m| &m.components),
                "librefang_version": manifest.as_ref().map(|m| &m.librefang_version),
                "created_at": manifest.as_ref().map(|m| &m.created_at),
            }));
        }
    }

    // Sort by filename descending (newest first since filenames contain timestamps)
    backups.sort_by(|a, b| {
        let fa = a["filename"].as_str().unwrap_or("");
        let fb = b["filename"].as_str().unwrap_or("");
        fb.cmp(fa)
    });

    let total = backups.len();
    Json(serde_json::json!({"backups": backups, "total": total}))
}

fn is_invalid_backup_filename(filename: &str) -> bool {
    if filename.is_empty() {
        return true;
    }
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return true;
    }
    std::path::Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        != Some(filename)
}

fn find_backup_path(
    backups_dir: &std::path::Path,
    filename: &str,
) -> std::io::Result<Option<std::path::PathBuf>> {
    let entries = std::fs::read_dir(backups_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("zip") {
            continue;
        }
        if entry.file_name().to_str() == Some(filename) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// DELETE /api/backups/{filename} — Delete a specific backup.
#[utoipa::path(delete, path = "/api/backups/{filename}", tag = "system", params(("filename" = String, Path, description = "Backup filename")), responses((status = 200, description = "Backup deleted")))]
pub async fn delete_backup(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Sanitize filename to prevent path traversal
    if is_invalid_backup_filename(&filename) {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-filename"))
            .into_json_tuple();
    }
    if !filename.ends_with(".zip") {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-must-be-zip"))
            .into_json_tuple();
    }

    let backups_dir = state.kernel.home_dir().join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) => {
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-delete-failed",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };

    if let Err(e) = std::fs::remove_file(&backup_path) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-delete-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    tracing::info!("Backup deleted: {filename}");
    (
        StatusCode::OK,
        Json(serde_json::json!({"deleted": filename})),
    )
}

/// POST /api/restore — Restore kernel state from a backup archive.
///
/// Accepts a JSON body with `{"filename": "librefang_backup_20260315_120000.zip"}`.
/// The file must exist in `<home_dir>/backups/`.
///
/// **Warning**: This overwrites existing state files. The daemon should be
/// restarted after a restore for all changes to take effect.
#[utoipa::path(post, path = "/api/restore", tag = "system", request_body = serde_json::Value, responses((status = 200, description = "Backup restored", body = serde_json::Value)))]
pub async fn restore_backup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let filename = match req.get("filename").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            return ApiErrorResponse::bad_request(t.t("api-error-backup-missing-filename"))
                .into_json_tuple();
        }
    };

    // Sanitize
    if is_invalid_backup_filename(&filename) {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-filename"))
            .into_json_tuple();
    }
    if !filename.ends_with(".zip") {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-must-be-zip"))
            .into_json_tuple();
    }

    let home_dir = &state.kernel.home_dir();
    let backups_dir = home_dir.join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) => {
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    };

    // Open zip
    let file = match std::fs::File::open(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            return ApiErrorResponse::bad_request(t.t_args(
                "api-error-backup-invalid-archive",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };

    // Validate manifest
    let manifest: Option<BackupManifest> = {
        match archive.by_name("manifest.json") {
            Ok(mut entry) => {
                let mut buf = String::new();
                if std::io::Read::read_to_string(&mut entry, &mut buf).is_ok() {
                    serde_json::from_str(&buf).ok()
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    };

    if manifest.is_none() {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-missing-manifest"))
            .into_json_tuple();
    }

    let mut restored: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Extract all files to home_dir, skipping manifest.json itself
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("Failed to read entry {i}: {e}"));
                continue;
            }
        };

        let entry_name = match entry.enclosed_name() {
            Some(name) => name.to_path_buf(),
            None => {
                errors.push(format!("Skipped unsafe entry name at index {i}"));
                continue;
            }
        };

        if entry_name.to_string_lossy() == "manifest.json" {
            continue;
        }

        let target = home_dir.join(&entry_name);

        if entry.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&target) {
                errors.push(format!("mkdir {}: {e}", entry_name.display()));
            }
            continue;
        }

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                errors.push(format!("mkdir parent for {}: {e}", entry_name.display()));
                continue;
            }
        }

        let mut data = Vec::new();
        if let Err(e) = std::io::Read::read_to_end(&mut entry, &mut data) {
            errors.push(format!("read {}: {e}", entry_name.display()));
            continue;
        }
        if let Err(e) = std::fs::write(&target, &data) {
            errors.push(format!("write {}: {e}", entry_name.display()));
            continue;
        }
        restored.push(entry_name.to_string_lossy().to_string());
    }

    let total_restored = restored.len();
    tracing::info!(
        "Restore from {filename}: {total_restored} files restored, {} errors",
        errors.len()
    );
    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("Backup restored: {filename} ({total_restored} files)"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "restored_files": total_restored,
            "errors": errors,
            "manifest": manifest,
            "message": "Restore complete. Restart the daemon for all changes to take effect.",
        })),
    )
}

/// Read the `manifest.json` from a backup zip without extracting everything.
fn read_backup_manifest(path: &std::path::Path) -> Option<BackupManifest> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("manifest.json").ok()?;
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut entry, &mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// GET /api/queue/status — Command queue status and occupancy.
#[utoipa::path(get, path = "/api/queue/status", tag = "system", responses((status = 200, description = "Queue status", body = serde_json::Value)))]
pub async fn queue_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let occupancy = state.kernel.command_queue_ref().occupancy();
    let lanes: Vec<serde_json::Value> = occupancy
        .iter()
        .map(|o| {
            serde_json::json!({
                "lane": o.lane.to_string(),
                "active": o.active,
                "capacity": o.capacity,
            })
        })
        .collect();

    let kcfg2 = state.kernel.config_ref();
    let queue_cfg = &kcfg2.queue;
    Json(serde_json::json!({
        "lanes": lanes,
        "config": {
            "max_depth_per_agent": queue_cfg.max_depth_per_agent,
            "max_depth_global": queue_cfg.max_depth_global,
            "task_ttl_secs": queue_cfg.task_ttl_secs,
        },
    }))
}

/// Get the machine hostname (best-effort).
pub(crate) fn hostname_string() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .map_err(|_| std::env::VarError::NotPresent)
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

/// SECURITY: Validate webhook bearer token using constant-time comparison.
fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(t) if t.len() >= 32 => t,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) if s.starts_with("Bearer ") => &s[7..],
            _ => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// API versioning
// ---------------------------------------------------------------------------

/// GET /api/versions — List supported API versions and negotiation info.
#[utoipa::path(
    get,
    path = "/api/versions",
    tag = "system",
    responses(
        (status = 200, description = "API version info", body = serde_json::Value)
    )
)]
pub async fn api_versions() -> impl IntoResponse {
    let supported: Vec<&str> = crate::versioning::SUPPORTED_VERSIONS.to_vec();
    let deprecated: Vec<&str> = crate::versioning::DEPRECATED_VERSIONS.to_vec();

    let details: Vec<serde_json::Value> = crate::server::API_VERSIONS
        .iter()
        .map(|(ver, status)| {
            serde_json::json!({
                "version": ver,
                "status": status,
                "url_prefix": format!("/api/{ver}"),
            })
        })
        .collect();

    Json(serde_json::json!({
        "current": crate::versioning::CURRENT_VERSION,
        "supported": supported,
        "deprecated": deprecated,
        "details": details,
        "negotiation": {
            "header": "Accept",
            "media_type_pattern": "application/vnd.librefang.<version>+json",
            "example": "application/vnd.librefang.v1+json",
        },
    }))
}

// ---------------------------------------------------------------------------
// Event Webhooks — subscribe to system events via HTTP callbacks (#185)
// ---------------------------------------------------------------------------

/// Supported event types for webhook subscriptions.
static VALID_EVENT_TYPES: &[&str] = &[
    "agent.spawned",
    "agent.terminated",
    "agent.error",
    "message.received",
    "workflow.completed",
    "workflow.failed",
];

/// In-memory store for event webhook subscriptions.
///
/// NOTE: subscriptions are lost on daemon restart. A future iteration should
/// persist these to the config/data directory.
static EVENT_WEBHOOKS: std::sync::LazyLock<
    tokio::sync::RwLock<HashMap<String, serde_json::Value>>,
> = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(HashMap::new()));

/// Validate an events JSON array against VALID_EVENT_TYPES.
fn validate_event_types(
    arr: &[serde_json::Value],
    lang: Option<&axum::Extension<RequestLanguage>>,
) -> Result<Vec<String>, (StatusCode, Json<serde_json::Value>)> {
    let t = ErrorTranslator::new(super::resolve_lang(lang));
    let mut event_list = Vec::new();
    for ev in arr {
        match ev.as_str() {
            Some(s) if VALID_EVENT_TYPES.contains(&s) => {
                event_list.push(s.to_string());
            }
            Some(s) => {
                let valid_str = format!("{VALID_EVENT_TYPES:?}");
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": t.t_args("api-error-webhook-unknown-event", &[("event", s), ("valid", &valid_str)])
                    })),
                ));
            }
            None => {
                return Err(ApiErrorResponse::bad_request(
                    t.t("api-error-webhook-event-not-string"),
                )
                .into_json_tuple());
            }
        }
    }
    if event_list.is_empty() {
        return Err(
            ApiErrorResponse::bad_request(t.t("api-error-webhook-events-empty")).into_json_tuple(),
        );
    }
    Ok(event_list)
}

/// Redact the secret field from a webhook JSON value before returning it.
fn redact_webhook_secret(webhook: &serde_json::Value) -> serde_json::Value {
    let mut w = webhook.clone();
    if let Some(obj) = w.as_object_mut() {
        if obj.contains_key("secret") {
            obj.insert("secret".to_string(), serde_json::json!("***"));
        }
    }
    w
}

/// GET /api/webhooks/events — List all event webhook subscriptions.
pub async fn list_event_webhooks() -> impl IntoResponse {
    let store = EVENT_WEBHOOKS.read().await;
    let list: Vec<serde_json::Value> = store.values().map(redact_webhook_secret).collect();
    Json(list)
}

/// POST /api/webhooks/events — Create a new event webhook subscription.
pub async fn create_event_webhook(
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Pre-translate error messages before .await to avoid holding !Send ErrorTranslator across await
    let (err_missing_url, err_invalid_url, err_missing_events) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-missing-url"),
            t.t("api-error-webhook-invalid-url"),
            t.t("api-error-webhook-missing-events"),
        )
    };

    let url = match req["url"].as_str() {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => {
            return ApiErrorResponse::bad_request(err_missing_url).into_json_tuple();
        }
    };

    if url::Url::parse(&url).is_err() {
        return ApiErrorResponse::bad_request(err_invalid_url).into_json_tuple();
    }

    let events = match req.get("events").and_then(|v| v.as_array()) {
        Some(arr) => match validate_event_types(arr, lang.as_ref()) {
            Ok(ev) => ev,
            Err(e) => return e,
        },
        None => {
            return ApiErrorResponse::bad_request(err_missing_events).into_json_tuple();
        }
    };

    let secret = req["secret"].as_str().map(|s| s.to_string());
    let enabled = req["enabled"].as_bool().unwrap_or(true);
    let id = uuid::Uuid::new_v4().to_string();

    let webhook = serde_json::json!({
        "id": id,
        "url": url,
        "events": events,
        "secret": secret,
        "enabled": enabled,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    EVENT_WEBHOOKS
        .write()
        .await
        .insert(id.clone(), webhook.clone());

    (StatusCode::CREATED, Json(redact_webhook_secret(&webhook)))
}

/// PUT /api/webhooks/events/{id} — Update an event webhook subscription.
pub async fn update_event_webhook(
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (err_webhook_not_found, err_invalid_url) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-not-found"),
            t.t("api-error-webhook-invalid-url"),
        )
    };
    let mut store = EVENT_WEBHOOKS.write().await;
    let existing = match store.get(&id) {
        Some(w) => w.clone(),
        None => {
            return ApiErrorResponse::not_found(err_webhook_not_found).into_json_tuple();
        }
    };

    let mut updated = existing;

    if let Some(url_val) = req.get("url").and_then(|v| v.as_str()) {
        if url::Url::parse(url_val).is_err() {
            return ApiErrorResponse::bad_request(err_invalid_url).into_json_tuple();
        }
        updated["url"] = serde_json::json!(url_val);
    }

    if let Some(arr) = req.get("events").and_then(|v| v.as_array()) {
        match validate_event_types(arr, lang.as_ref()) {
            Ok(ev) => updated["events"] = serde_json::json!(ev),
            Err(e) => return e,
        }
    }

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        updated["enabled"] = serde_json::json!(enabled);
    }

    if let Some(secret) = req.get("secret") {
        updated["secret"] = secret.clone();
    }

    store.insert(id, updated.clone());

    (StatusCode::OK, Json(redact_webhook_secret(&updated)))
}

/// DELETE /api/webhooks/events/{id} — Remove an event webhook subscription.
pub async fn delete_event_webhook(
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let err_webhook_not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-webhook-not-found")
    };
    let mut store = EVENT_WEBHOOKS.write().await;
    if store.remove(&id).is_some() {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "id": id})),
        )
    } else {
        ApiErrorResponse::not_found(err_webhook_not_found).into_json_tuple()
    }
}

// ---------------------------------------------------------------------------
// Outbound webhook management endpoints (file-persisted subscriptions)
// ---------------------------------------------------------------------------

/// GET /api/webhooks — List all webhook subscriptions (secrets redacted).
pub async fn list_webhooks(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let webhooks: Vec<_> = state
        .webhook_store
        .list()
        .iter()
        .map(crate::webhook_store::redact_webhook_secret)
        .collect();
    let total = webhooks.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({"webhooks": webhooks, "total": total})),
    )
}

/// GET /api/webhooks/{id} — Get a single webhook subscription (secret redacted).
pub async fn get_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let wh_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => crate::webhook_store::WebhookId(uuid),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id"))
                .into_json_tuple();
        }
    };
    match state.webhook_store.get(wh_id) {
        Some(wh) => {
            let redacted = crate::webhook_store::redact_webhook_secret(&wh);
            match serde_json::to_value(&redacted) {
                Ok(v) => (StatusCode::OK, Json(v)),
                Err(_) => ApiErrorResponse::internal(t.t("api-error-webhook-serialize-error"))
                    .into_json_tuple(),
            }
        }
        None => ApiErrorResponse::not_found(t.t("api-error-webhook-not-found")).into_json_tuple(),
    }
}

/// POST /api/webhooks — Create a new webhook subscription.
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<crate::webhook_store::CreateWebhookRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match state.webhook_store.create(req) {
        Ok(webhook) => {
            let redacted = crate::webhook_store::redact_webhook_secret(&webhook);
            match serde_json::to_value(&redacted) {
                Ok(v) => (StatusCode::CREATED, Json(v)),
                Err(_) => ApiErrorResponse::internal(t.t("api-error-webhook-serialize-error"))
                    .into_json_tuple(),
            }
        }
        Err(e) => ApiErrorResponse::bad_request(e).into_json_tuple(),
    }
}

/// PUT /api/webhooks/{id} — Update a webhook subscription.
pub async fn update_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<crate::webhook_store::UpdateWebhookRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let wh_id = crate::webhook_store::WebhookId(uuid);
            match state.webhook_store.update(wh_id, req) {
                Ok(webhook) => {
                    let redacted = crate::webhook_store::redact_webhook_secret(&webhook);
                    match serde_json::to_value(&redacted) {
                        Ok(v) => (StatusCode::OK, Json(v)),
                        Err(_) => {
                            ApiErrorResponse::internal(t.t("api-error-webhook-serialize-error"))
                                .into_json_tuple()
                        }
                    }
                }
                Err(e) => ApiErrorResponse::not_found(e).into_json_tuple(),
            }
        }
        Err(_) => {
            ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id")).into_json_tuple()
        }
    }
}

/// DELETE /api/webhooks/{id} — Delete a webhook subscription.
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let wh_id = crate::webhook_store::WebhookId(uuid);
            if state.webhook_store.delete(wh_id) {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "deleted"})),
                )
            } else {
                ApiErrorResponse::not_found(t.t("api-error-webhook-not-found")).into_json_tuple()
            }
        }
        Err(_) => {
            ApiErrorResponse::bad_request(t.t("api-error-webhook-invalid-id")).into_json_tuple()
        }
    }
}

/// POST /api/webhooks/{id}/test — Send a test event to a webhook.
///
/// Includes HMAC-SHA256 signature in `X-Webhook-Signature` header when
/// the webhook has a secret configured.
pub async fn test_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let (err_invalid_id, err_not_found) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-webhook-invalid-id"),
            t.t("api-error-webhook-not-found"),
        )
    };
    let wh_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => crate::webhook_store::WebhookId(uuid),
        Err(_) => {
            return ApiErrorResponse::bad_request(err_invalid_id).into_json_tuple();
        }
    };

    let webhook = match state.webhook_store.get(wh_id) {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found(err_not_found).into_json_tuple();
        }
    };

    // Re-validate the URL against SSRF rules before sending
    if let Err(e) = crate::webhook_store::validate_webhook_url(&webhook.url) {
        let err_msg = {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            t.t_args("api-error-webhook-url-unsafe", &[("error", &e.to_string())])
        };
        return ApiErrorResponse::bad_request(err_msg).into_json_tuple();
    }

    let test_payload = serde_json::json!({
        "event": "test",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "webhook_id": webhook.id.to_string(),
        "message": "This is a test event from LibreFang.",
    });

    let payload_bytes = serde_json::to_vec(&test_payload).unwrap_or_default();

    let client = librefang_runtime::http_client::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("HTTP client build");

    let mut request = client
        .post(&webhook.url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "LibreFang-Webhook/1.0");

    // Add HMAC signature if secret is configured
    if let Some(ref secret) = webhook.secret {
        let signature = crate::webhook_store::compute_hmac_signature(secret, &payload_bytes);
        request = request.header("X-Webhook-Signature", signature);
    }

    match request.body(payload_bytes).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "sent",
                    "response_status": status,
                    "webhook_id": id,
                })),
            )
        }
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            let msg = t.t_args(
                "api-error-webhook-reach-failed",
                &[("error", &e.to_string())],
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "status": "error",
                    "error": msg,
                    "webhook_id": id,
                })),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Task queue management endpoints (#184)
// ---------------------------------------------------------------------------

/// GET /api/tasks/status — Summary counts of tasks by status.
pub async fn task_queue_status(
    State(state): State<Arc<AppState>>,
    _lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    match state.kernel.task_list(None).await {
        Ok(tasks) => {
            let mut pending = 0u64;
            let mut in_progress = 0u64;
            let mut completed = 0u64;
            let mut failed = 0u64;
            for t in &tasks {
                match t["status"].as_str().unwrap_or("") {
                    "pending" => pending += 1,
                    "in_progress" => in_progress += 1,
                    "completed" => completed += 1,
                    "failed" => failed += 1,
                    _ => {}
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "total": tasks.len(),
                    "pending": pending,
                    "in_progress": in_progress,
                    "completed": completed,
                    "failed": failed,
                })),
            )
        }
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// GET /api/tasks/list — List tasks, optionally filtered by ?status=pending|in_progress|completed|failed.
pub async fn task_queue_list(
    State(state): State<Arc<AppState>>,
    _lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let status_filter = params.get("status").map(|s| s.as_str());
    match state.kernel.task_list(status_filter).await {
        Ok(tasks) => {
            let total = tasks.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({"tasks": tasks, "total": total})),
            )
        }
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// DELETE /api/tasks/{id} — Remove a task from the queue.
pub async fn task_queue_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let err_task_not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-task-not-found")
    };
    match state.kernel.task_delete(&id).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "id": id})),
        ),
        Ok(false) => ApiErrorResponse::not_found(err_task_not_found).into_json_tuple(),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// POST /api/tasks/{id}/retry — Re-queue a completed or failed task back to pending.
///
/// In-progress tasks cannot be retried to prevent duplicate execution.
pub async fn task_queue_retry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let err_task_not_retryable = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-task-not-retryable")
    };
    match state.kernel.task_retry(&id).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "retried", "id": id})),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": err_task_not_retryable
            })),
        ),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
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
        "integration" => home_dir
            .join("integrations")
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

    // Convert JSON values to TOML.
    // For providers: the catalog TOML format requires a `[provider]` section header.
    // If the body is a flat object (fields at the top level), restructure it so that
    // non-`models` fields are nested under a `"provider"` key, producing the correct
    // `[provider] … [[models]] …` layout that `ModelCatalogFile` expects.
    let body_for_toml = if content_type == "provider" {
        normalize_provider_body(&body)
    } else {
        body.clone()
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
        let mut catalog = state
            .kernel
            .model_catalog_ref()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if let Err(e) = catalog.load_catalog_file(&target) {
            tracing::warn!("Failed to merge provider file into catalog: {e}");
        }
        catalog.detect_auth();
        // Invalidate cached LLM drivers — URLs/keys may have changed.
        drop(catalog);
        state.kernel.clear_driver_cache();
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

// ---------------------------------------------------------------------------
// Event Webhook Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod event_webhook_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Serialize all webhook tests to avoid races on the shared EVENT_WEBHOOKS store.
    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn webhook_router() -> Router {
        Router::new()
            .route(
                "/api/webhooks/events",
                axum::routing::get(list_event_webhooks).post(create_event_webhook),
            )
            .route(
                "/api/webhooks/events/{id}",
                axum::routing::put(update_event_webhook).delete(delete_event_webhook),
            )
    }

    async fn clear_webhooks() {
        EVENT_WEBHOOKS.write().await.clear();
    }

    #[tokio::test]
    async fn test_list_empty() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned", "agent.error"],
            "secret": "my-secret-key",
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(created["id"].as_str().is_some());
        assert_eq!(created["url"], "https://example.com/hook");
        assert_eq!(created["enabled"], true);
        // Secret must be redacted in responses
        assert_eq!(created["secret"], "***");

        // List should contain the webhook with redacted secret
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["secret"], "***");
    }

    #[tokio::test]
    async fn test_create_invalid_event() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["nonexistent.event"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_missing_url() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "events": ["agent.spawned"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_invalid_url() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "not a valid url",
            "events": ["agent.spawned"],
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_update_webhook() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = created["id"].as_str().unwrap();

        let update_payload = serde_json::json!({
            "enabled": false,
            "events": ["agent.spawned", "workflow.completed"],
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/webhooks/events/{id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated["enabled"], false);
        assert_eq!(updated["events"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_delete_webhook() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({
            "url": "https://example.com/hook",
            "events": ["agent.spawned"],
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/events")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = created["id"].as_str().unwrap();

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/webhooks/events/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhooks/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/webhooks/events/nonexistent-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_not_found() {
        let _guard = TEST_LOCK.lock().await;
        clear_webhooks().await;
        let app = webhook_router();

        let payload = serde_json::json!({"enabled": false});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/webhooks/events/nonexistent-id")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
