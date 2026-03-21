//! Audit, logging, tools, profiles, templates, memory, approvals,
//! bindings, pairing, webhooks, and miscellaneous system handlers.

use super::AppState;
use crate::middleware::RequestLanguage;
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
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": t.t_args("api-error-profile-not-found", &[("name", &name)])}),
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
#[utoipa::path(get, path = "/api/templates", tag = "system", responses((status = 200, description = "List templates", body = Vec<serde_json::Value>)))]
pub async fn list_templates() -> impl IntoResponse {
    let agents_dir = librefang_kernel::config::librefang_home().join("agents");
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
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template details", body = serde_json::Value)))]
pub async fn get_template(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agents_dir = librefang_kernel::config::librefang_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-template-not-found")})),
        );
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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": t.t("api-error-template-invalid-manifest")})),
                )
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-template-read-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    match state.kernel.memory.list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(e) => {
            tracing::warn!("Memory list_kv failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-memory-operation-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    match state.kernel.memory.structured_get(agent_id, &key) {
        Ok(Some(val)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": val})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-kv-key-not-found")})),
        ),
        Err(e) => {
            tracing::warn!("Memory get failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-memory-operation-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    let value = body.get("value").cloned().unwrap_or(body);

    match state.kernel.memory.structured_set(agent_id, &key, value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory set failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-memory-operation-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    match state.kernel.memory.structured_delete(agent_id, &key) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory delete failed for key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-memory-operation-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Verify agent exists
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    match state.kernel.memory.list_kv(agent_id) {
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-kv-export-failed")})),
            )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Verify agent exists
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let kv = match body.get("kv").and_then(|v| v.as_object()) {
        Some(obj) => obj.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-kv-missing-kv-object")})),
            );
        }
    };

    let clear_existing = body
        .get("clear_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Clear existing memory if requested
    if clear_existing {
        match state.kernel.memory.list_kv(agent_id) {
            Ok(existing) => {
                for (key, _) in existing {
                    if let Err(e) = state.kernel.memory.structured_delete(agent_id, &key) {
                        tracing::warn!("Failed to delete key '{key}' during import clear: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to list existing KV during import clear: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": t.t("api-error-kv-import-clear-failed")})),
                );
            }
        }
    }

    let mut imported = 0u64;
    let mut errors = Vec::new();

    for (key, value) in &kv {
        match state
            .kernel
            .memory
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

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();

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
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    }))
}

/// GET /api/audit/verify — Verify the audit chain integrity.
#[utoipa::path(get, path = "/api/audit/verify", tag = "system", responses((status = 200, description = "Audit verification result", body = serde_json::Value)))]
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
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

            let entries = state.kernel.audit_log.recent(200);

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
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
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
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
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

    (
        StatusCode::NOT_FOUND,
        Json(
            serde_json::json!({"error": tr.t_args("api-error-tool-not-found", &[("name", &name)])}),
        ),
    )
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
    match state.kernel.memory.list_sessions() {
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            );
        }
    };

    match state.kernel.memory.get_session_with_created_at(session_id) {
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
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-session-not-found")})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            );
        }
    };

    match state.kernel.memory.delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-session-invalid-id")})),
            );
        }
    };

    let label = req.get("label").and_then(|v| v.as_str());

    // Validate label if present
    if let Some(lbl) = label {
        if let Err(e) = librefang_types::agent::SessionLabel::new(lbl) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }

    match state.kernel.memory.set_session_label(session_id, label) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
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
            match state.kernel.registry.find_by_name(&agent_id_str) {
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

    match state.kernel.memory.find_session_by_label(agent_id, &label) {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-session-no-label")})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
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
    let cfg = &state.kernel.config.session;
    let mut total: u64 = 0;

    if cfg.retention_days > 0 {
        match state
            .kernel
            .memory
            .cleanup_expired_sessions(cfg.retention_days)
        {
            Ok(n) => total += n,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-session-cleanup-expired-failed", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    if cfg.max_sessions_per_agent > 0 {
        match state
            .kernel
            .memory
            .cleanup_excess_sessions(cfg.max_sessions_per_agent)
        {
            Ok(n) => total += n,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-session-cleanup-excess-failed", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"sessions_deleted": total})),
    )
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approval_manager
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
    let pending = state.kernel.approval_manager.list_pending();
    let recent = state.kernel.approval_manager.list_recent(50);

    let registry_agents = state.kernel.registry.list();
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-approval-invalid-id")})),
            );
        }
    };

    match state.kernel.approval_manager.get_pending(uuid) {
        Some(a) => {
            let registry_agents = state.kernel.registry.list();
            (StatusCode::OK, Json(approval_to_json(&a, &registry_agents)))
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": t.t_args("api-error-approval-not-found", &[("id", &id)])}),
            ),
        ),
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

    let policy = state.kernel.approval_manager.policy();
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
    };

    // Spawn the request in the background (it will block until resolved or timed out)
    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approval_manager.request_approval(approval_req).await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve — Approve a pending request.
#[utoipa::path(post, path = "/api/approvals/{id}/approve", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Request approved", body = serde_json::Value)))]
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-approval-invalid-id")})),
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        librefang_types::approval::ApprovalDecision::Approved,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "approved", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
    }
}

/// POST /api/approvals/{id}/reject — Reject a pending request.
#[utoipa::path(post, path = "/api/approvals/{id}/reject", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Request rejected", body = serde_json::Value)))]
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-approval-invalid-id")})),
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        librefang_types::approval::ApprovalDecision::Denied,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "rejected", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
    }
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
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_webhook_not_enabled})),
            );
        }
    };

    // Validate bearer token (constant-time comparison)
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": err_invalid_token})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_msg})),
        );
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
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_webhook_not_enabled})),
            );
        }
    };

    // Validate bearer token
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": err_invalid_token})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.registry.find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        let err_msg = {
                            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
                            t.t_args("api-error-webhook-agent-not-found", &[("id", agent_ref)])
                        };
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({"error": err_msg})),
                        );
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent
            match state.kernel.registry.list().first() {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": err_no_agents})),
                    );
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": msg})),
            )
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
    let agents = state.kernel.registry.list();
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
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-pairing-not-enabled")})),
        )
            .into_response();
    }
    match state.kernel.pairing.create_pairing_request() {
        Ok(req) => {
            let qr_uri = format!("librefang://pair?token={}", req.token);
            Json(serde_json::json!({
                "token": req.token,
                "qr_uri": qr_uri,
                "expires_at": req.expires_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
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
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-pairing-not-enabled")})),
        )
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
    match state.kernel.pairing.complete_pairing(token, device_info) {
        Ok(device) => Json(serde_json::json!({
            "device_id": device.device_id,
            "display_name": device.display_name,
            "platform": device.platform,
            "paired_at": device.paired_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
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
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-pairing-not-enabled")})),
        )
            .into_response();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing
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
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-pairing-not-enabled")})),
        )
            .into_response();
    }
    match state.kernel.pairing.remove_device(&device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
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
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_pairing_not_enabled})),
        )
            .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("LibreFang");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_message_required})),
        )
            .into_response();
    }
    state.kernel.pairing.notify_devices(title, message).await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing.list_devices().len()}))
        .into_response()
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
#[utoipa::path(get, path = "/api/commands", tag = "system", responses((status = 200, description = "List chat commands", body = Vec<serde_json::Value>)))]
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Reset session (clear history)"}),
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
    if let Ok(registry) = state.kernel.skill_registry.read() {
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
    if let Ok(registry) = state.kernel.skill_registry.read() {
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

    (
        StatusCode::NOT_FOUND,
        Json(
            serde_json::json!({"error": t.t_args("api-error-command-not-found", &[("name", &lookup)])}),
        ),
    )
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
    let home_dir = &state.kernel.config.home_dir;
    let backups_dir = home_dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-backup-create-dir-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("librefang_backup_{timestamp}.zip");
    let backup_path = backups_dir.join(&filename);

    let mut components: Vec<String> = Vec::new();

    // Create zip archive
    let file = match std::fs::File::create(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-backup-create-file-failed", &[("error", &e.to_string())])}),
                ),
            );
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
    let agents_dir = home_dir.join("agents");
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-backup-finalize-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    let size = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    tracing::info!(
        "Backup created: {filename} ({} bytes, {} components)",
        size,
        components.len()
    );
    state.kernel.audit_log.record(
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
    let backups_dir = state.kernel.config.home_dir.join("backups");
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-backup-invalid-filename")})),
        );
    }
    if !filename.ends_with(".zip") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-backup-must-be-zip")})),
        );
    }

    let backups_dir = state.kernel.config.home_dir.join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-backup-not-found")})),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-backup-not-found")})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-backup-delete-failed", &[("error", &e.to_string())])}),
                ),
            );
        }
    };

    if let Err(e) = std::fs::remove_file(&backup_path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-backup-delete-failed", &[("error", &e.to_string())])}),
            ),
        );
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-backup-missing-filename")})),
            );
        }
    };

    // Sanitize
    if is_invalid_backup_filename(&filename) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-backup-invalid-filename")})),
        );
    }
    if !filename.ends_with(".zip") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-backup-must-be-zip")})),
        );
    }

    let home_dir = &state.kernel.config.home_dir;
    let backups_dir = home_dir.join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-backup-not-found")})),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-backup-not-found")})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())])}),
                ),
            );
        }
    };

    // Open zip
    let file = match std::fs::File::open(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())])}),
                ),
            );
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-backup-invalid-archive", &[("error", &e.to_string())])}),
                ),
            );
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-backup-missing-manifest")})),
        );
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
    state.kernel.audit_log.record(
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
    let occupancy = state.kernel.command_queue.occupancy();
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

    let queue_cfg = &state.kernel.config.queue;
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
fn hostname_string() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
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
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": t.t("api-error-webhook-event-not-string")})),
                ));
            }
        }
    }
    if event_list.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-webhook-events-empty")})),
        ));
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_missing_url})),
            );
        }
    };

    if url::Url::parse(&url).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_invalid_url})),
        );
    }

    let events = match req.get("events").and_then(|v| v.as_array()) {
        Some(arr) => match validate_event_types(arr, lang.as_ref()) {
            Ok(ev) => ev,
            Err(e) => return e,
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_missing_events})),
            );
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
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_webhook_not_found})),
            );
        }
    };

    let mut updated = existing;

    if let Some(url_val) = req.get("url").and_then(|v| v.as_str()) {
        if url::Url::parse(url_val).is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_url})),
            );
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
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_webhook_not_found})),
        )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-webhook-invalid-id")})),
            );
        }
    };
    match state.webhook_store.get(wh_id) {
        Some(wh) => {
            let redacted = crate::webhook_store::redact_webhook_secret(&wh);
            match serde_json::to_value(&redacted) {
                Ok(v) => (StatusCode::OK, Json(v)),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": t.t("api-error-webhook-serialize-error")})),
                ),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-webhook-not-found")})),
        ),
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
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": t.t("api-error-webhook-serialize-error")})),
                ),
            }
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
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
                        Err(_) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(
                                serde_json::json!({"error": t.t("api-error-webhook-serialize-error")}),
                            ),
                        ),
                    }
                }
                Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-webhook-invalid-id")})),
        ),
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
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-webhook-not-found")})),
                )
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-webhook-invalid-id")})),
        ),
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            );
        }
    };

    let webhook = match state.webhook_store.get(wh_id) {
        Some(w) => w,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": err_not_found})),
            );
        }
    };

    // Re-validate the URL against SSRF rules before sending
    if let Err(e) = crate::webhook_store::validate_webhook_url(&webhook.url) {
        let err_msg = {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            t.t_args("api-error-webhook-url-unsafe", &[("error", &e.to_string())])
        };
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_msg})),
        );
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
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
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err_task_not_found})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
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
