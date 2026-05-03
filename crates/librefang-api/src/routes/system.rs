//! Audit, logging, tools, memory, approvals, bindings, webhooks,
//! and miscellaneous system handlers.
//!
//! Tool profiles (`/profiles*`) and agent templates (`/templates*`) were
//! extracted to [`super::agent_templates`] per #3749.
//!
//! Device pairing (`/pairing/*`) was extracted to [`super::pairing`] per #3749.

use super::AppState;

/// Build routes for the system miscellaneous domain (audit, logs, tools, sessions, approvals, pairing, etc.).
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        // Tool profiles + agent templates live in `routes::agent_templates`
        // (#3749 sub-domain extraction). Public paths under `/profiles*` and
        // `/templates*` are unchanged.
        .merge(crate::routes::agent_templates::router())
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
        // Log streaming
        .route("/logs/stream", axum::routing::get(logs_stream))
        // Tools + Sessions — extracted into routes/tools_sessions.rs (#3749)
        .merge(crate::routes::tools_sessions::router())
        // Approvals + TOTP — extracted into routes/approvals.rs (#3749 6/N)
        .merge(crate::routes::approvals::router())
        // Webhook triggers + chat command catalog — extracted into
        // routes/hooks_commands.rs (#3749 4/N).
        .merge(crate::routes::hooks_commands::router())
        // Bindings
        .route(
            "/bindings",
            axum::routing::get(list_bindings).post(add_binding),
        )
        .route(
            "/bindings/{index}",
            axum::routing::delete(remove_binding),
        )
        // Pairing endpoints live in `routes::pairing` (#3749 sub-domain
        // extraction). Public paths under `/pairing/*` are unchanged.
        .merge(crate::routes::pairing::router())
        // Queue status
        .route("/queue/status", axum::routing::get(queue_status))
        // Task queue management routes moved to routes::task_queue (#3749 9/N).
        // Registry schema + content endpoints — extracted into
        // routes/registry.rs (#3749 10/N).
        .merge(crate::routes::registry::router())
        // Backup / restore (extracted to routes::backup, #3749)
        .merge(crate::routes::backup::router())
}
use crate::extractors::AgentIdPath;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Resolve the LibreFang home directory without depending on the kernel crate.
///
/// Mirrors `librefang_kernel::config::librefang_home`:
/// `LIBREFANG_HOME` env var takes priority, otherwise `~/.librefang`
/// (falling back to the system temp dir if no home directory is available).
pub(super) fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

// ---------------------------------------------------------------------------
// Memory endpoints
// ---------------------------------------------------------------------------

/// GET /api/memory/agents/:id/kv — List KV pairs for an agent.
#[utoipa::path(get, path = "/api/memory/agents/{id}/kv", tag = "memory", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent KV store", body = crate::types::JsonObject)))]
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Owner-scoping: non-admins can only read the KV store of agents
    // they authored. Without this, anyone authenticated could pull
    // user.preferences / oncall.contact / api.tokens out of any agent.
    if let Some(ref user) = api_user {
        use crate::middleware::UserRole;
        if user.0.role < UserRole::Admin {
            let entry = state.kernel.agent_registry().get(agent_id);
            let owned = entry
                .as_ref()
                .map(|e| e.manifest.author.eq_ignore_ascii_case(&user.0.name))
                .unwrap_or(false);
            if !owned {
                return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
                    .into_json_tuple();
            }
        }
    }
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
#[utoipa::path(get, path = "/api/memory/agents/{id}/kv/{key}", tag = "memory", params(("id" = String, Path, description = "Agent ID"), ("key" = String, Path, description = "Key name")), responses((status = 200, description = "KV value", body = crate::types::JsonObject)))]
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
#[utoipa::path(put, path = "/api/memory/agents/{id}/kv/{key}", tag = "memory", params(("id" = String, Path, description = "Agent ID"), ("key" = String, Path, description = "Key name")), request_body = crate::types::JsonObject, responses((status = 200, description = "KV value set", body = crate::types::JsonObject)))]
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
) -> axum::response::Response {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };
    match state
        .kernel
        .memory_substrate()
        .structured_delete(agent_id, &key)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("Memory delete failed for key '{key}': {e}");
            ApiErrorResponse::internal(t.t("api-error-memory-operation-failed"))
                .into_json_tuple()
                .into_response()
        }
    }
}

/// GET /api/agents/:id/memory/export — Export all KV memory for an agent as JSON.
#[utoipa::path(get, path = "/api/agents/{id}/memory/export", tag = "memory", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Exported memory", body = crate::types::JsonObject)))]
pub async fn export_agent_memory(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

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
#[utoipa::path(post, path = "/api/agents/{id}/memory/import", tag = "memory", params(("id" = String, Path, description = "Agent ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Memory imported", body = crate::types::JsonObject)))]
pub async fn import_agent_memory(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

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
#[utoipa::path(post, path = "/api/bindings", tag = "system", request_body = crate::types::JsonObject, responses((status = 200, description = "Binding added", body = crate::types::JsonObject)))]
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
        Some(_) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": t.t("api-error-binding-index-out-of-range") })),
        ),
    }
}

// ─── Queue status ──────────────────────────────────────────────────────

/// GET /api/queue/status — Command queue status and occupancy.
#[utoipa::path(get, path = "/api/queue/status", tag = "system", responses((status = 200, description = "Queue status", body = crate::types::JsonObject)))]
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
            "concurrency": {
                "main_lane": queue_cfg.concurrency.main_lane,
                "cron_lane": queue_cfg.concurrency.cron_lane,
                "subagent_lane": queue_cfg.concurrency.subagent_lane,
                "trigger_lane": queue_cfg.concurrency.trigger_lane,
                "default_per_agent": queue_cfg.concurrency.default_per_agent,
            },
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

// ---------------------------------------------------------------------------
// API versioning
// ---------------------------------------------------------------------------

/// GET /api/versions — List supported API versions and negotiation info.
#[utoipa::path(
    get,
    path = "/api/versions",
    tag = "system",
    responses(
        (status = 200, description = "API version info", body = crate::types::JsonObject)
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

// Webhook subscription handlers moved to `routes/webhooks.rs`.
