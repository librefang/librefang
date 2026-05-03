//! Audit, logging, tools, memory, approvals, bindings, pairing, webhooks,
//! and miscellaneous system handlers.
//!
//! Tool profiles (`/profiles*`) and agent templates (`/templates*`) were
//! extracted to [`super::agent_templates`] per #3749.

use super::skills::write_secret_env;
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
        // Queue status
        .route("/queue/status", axum::routing::get(queue_status))
        // Task queue management
        .route(
            "/tasks",
            axum::routing::get(task_queue_list_root).post(task_queue_post_root),
        )
        .route("/tasks/status", axum::routing::get(task_queue_status))
        .route("/tasks/list", axum::routing::get(task_queue_list))
        .route(
            "/tasks/{id}",
            axum::routing::get(task_queue_get)
                .patch(task_queue_patch)
                .delete(task_queue_delete),
        )
        .route(
            "/tasks/{id}/retry",
            axum::routing::post(task_queue_retry),
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
        // Backup / restore (extracted to routes::backup, #3749)
        .merge(crate::routes::backup::router())
}
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use base64::Engine as _;
use librefang_runtime::kernel_handle::KernelHandle;
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
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(aid) => aid,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
    };
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
#[utoipa::path(post, path = "/api/agents/{id}/memory/import", tag = "memory", params(("id" = String, Path, description = "Agent ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Memory imported", body = crate::types::JsonObject)))]
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
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
#[utoipa::path(post, path = "/api/hooks/wake", tag = "webhooks", request_body = crate::types::JsonObject, responses((status = 200, description = "Wake hook triggered", body = crate::types::JsonObject)))]
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
#[utoipa::path(post, path = "/api/hooks/agent", tag = "webhooks", request_body = crate::types::JsonObject, responses((status = 200, description = "Agent hook triggered", body = crate::types::JsonObject)))]
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

// ─── Device Pairing endpoints ───────────────────────────────────────────

/// Resolve the daemon base_url that mobile clients should connect to,
/// embedded in the QR pairing payload.
///
/// Resolution order:
/// 1. `pairing.public_base_url` (operator-supplied, immune to header tampering)
/// 2. `Host` request header + scheme inferred from `X-Forwarded-Proto`
///
/// Returns `Err` only when neither path produces a usable URL — callers
/// surface that as 500 rather than emit a malformed QR.
fn resolve_pairing_base_url(
    configured: Option<&str>,
    headers: &axum::http::HeaderMap,
    host: &str,
) -> Result<String, String> {
    if let Some(url) = configured {
        let trimmed = url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            // Configured URL must carry a real http(s) scheme — silently
            // accepting `librefang.example.com` or `ftp://...` would
            // produce a QR the mobile client refuses with a vague
            // "unexpected base_url protocol" error.
            if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                return Err(format!(
                    "pairing.public_base_url must start with http:// or https:// (got: {trimmed:?})"
                ));
            }
            return Ok(trimmed.to_string());
        }
    }
    if host.is_empty() {
        return Err("Cannot resolve daemon base_url: missing Host header and \
                    pairing.public_base_url is not set"
            .to_string());
    }
    // Take the first comma-separated value, trim, and only accept it if
    // the result is non-empty — header value `""` or `, https` would
    // otherwise yield `://host`.
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http");
    Ok(format!("{scheme}://{host}"))
}

/// POST /api/pairing/request — Create a new pairing request (returns token + QR URI).
#[utoipa::path(post, path = "/api/pairing/request", tag = "pairing", responses((status = 200, description = "Pairing request created", body = crate::types::JsonObject)))]
pub async fn pairing_request(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Pull the Host header directly — axum 0.8 dropped the dedicated `Host`
    // extractor, and the project doesn't depend on `axum-extra`. The header
    // is mandatory in HTTP/1.1 so a missing one signals a malformed client.
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    // Resolve the base_url the mobile client should hit.
    //
    // Prefer the operator-configured `pairing.public_base_url` so the QR
    // payload is not influenced by request headers — trusting client
    // `X-Forwarded-Proto` would let any authenticated dashboard caller
    // forge `https://` even on a plain-HTTP daemon.
    //
    // When unset, fall back to `Host` + scheme inferred from
    // `X-Forwarded-Proto` (filtering blank values so we never emit
    // `://host`). If `Host` is also unusable, refuse rather than ship a
    // QR with a broken base_url.
    let base_url = match resolve_pairing_base_url(
        state.kernel.config_ref().pairing.public_base_url.as_deref(),
        &headers,
        &host,
    ) {
        Ok(url) => url,
        Err(msg) => {
            return ApiErrorResponse::internal(msg)
                .into_json_tuple()
                .into_response();
        }
    };
    match state.kernel.pairing_ref().create_pairing_request() {
        Ok(req) => {
            // Encode QR payload as base64 JSON so base_url (with "://") doesn't
            // need percent-encoding inside the outer librefang:// URI.
            let payload = serde_json::json!({
                "v": 1,
                "base_url": base_url,
                "token": req.token,
                "expires_at": req.expires_at.to_rfc3339(),
            });
            let payload_b64 =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
            let qr_uri = format!("librefang://pair?payload={payload_b64}");
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

/// Body of `POST /api/pairing/complete`. Typed so a missing/empty `token`
/// is rejected up front rather than silently degraded to an empty string
/// that the kernel pairing manager has to re-validate.
#[derive(serde::Deserialize)]
pub struct PairingCompleteRequest {
    pub token: String,
    #[serde(default = "default_unknown")]
    pub display_name: String,
    #[serde(default = "default_unknown")]
    pub platform: String,
    #[serde(default)]
    pub push_token: Option<String>,
}

fn default_unknown() -> String {
    "unknown".to_string()
}

/// POST /api/pairing/complete — Complete pairing with token + device info.
#[utoipa::path(post, path = "/api/pairing/complete", tag = "pairing", request_body = crate::types::JsonObject, responses((status = 200, description = "Pairing completed", body = crate::types::JsonObject)))]
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<PairingCompleteRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    // ErrorTranslator is !Send; drop before the .await on user_api_keys below
    // so the handler future remains Send.
    drop(t);
    let token = body.token.trim();
    if token.is_empty() {
        return ApiErrorResponse::bad_request("token is required")
            .into_json_tuple()
            .into_response();
    }
    let display_name = body.display_name.as_str();
    let platform = body.platform.as_str();
    let push_token = body.push_token.clone();
    // Mint a fresh per-device bearer token. The plaintext is returned
    // to the mobile client exactly once below; only the Argon2 hash is
    // persisted, so this token cannot be reconstructed from a database
    // dump and cannot be re-used by anyone except the holder.
    let plaintext_key = {
        let bytes: [u8; 32] = rand::random();
        hex::encode(bytes)
    };
    // Device bearers are 256-bit CSPRNG outputs — high enough entropy that
    // the Argon2 KDF cost is dead weight on every mobile request. Use a
    // plain SHA-256 hash; `verify_password` recognises the `$sha256$`
    // prefix and dispatches to the cheap path.
    let api_key_hash = crate::password_hash::hash_device_token(&plaintext_key);

    let device_id = uuid::Uuid::new_v4().to_string();
    let device_info = librefang_kernel::pairing::PairedDevice {
        device_id: device_id.clone(),
        display_name: display_name.to_string(),
        platform: platform.to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token,
        api_key_hash: api_key_hash.clone(),
    };

    match state
        .kernel
        .pairing_ref()
        .complete_pairing(token, device_info)
    {
        Ok(device) => {
            // Register this device's bearer with the live auth table so
            // the next request from the mobile app actually authenticates.
            // Devices are mapped to UserRole::User (chat with agents but no
            // admin-level mutations) — promote per-device privileges via a
            // future config knob if required.
            let device_user_name = format!("device:{}", device.device_id);
            let auth = crate::middleware::ApiUserAuth {
                name: device_user_name.clone(),
                role: crate::middleware::UserRole::User,
                api_key_hash,
                user_id: librefang_types::agent::UserId::from_name(&device_user_name),
            };
            state.user_api_keys.write().await.push(auth);

            tracing::info!(
                target: "pairing.audit",
                device_id = %device.device_id,
                display_name = %device.display_name,
                platform = %device.platform,
                "paired new device — bearer minted and registered"
            );

            Json(serde_json::json!({
                "device_id": device.device_id,
                // Plaintext bearer — the mobile client must store this; it
                // is never returned again. Replaces the daemon master
                // `api_key` that earlier revisions handed out, so revoking
                // a device via DELETE /api/pairing/devices/{id} now
                // genuinely cuts off its access.
                "api_key": plaintext_key,
                "display_name": device.display_name,
                "platform": device.platform,
                "paired_at": device.paired_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => {
            // Return 410 Gone for used/expired tokens to let the client
            // distinguish "token consumed" from a generic 400 input error.
            (
                axum::http::StatusCode::GONE,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
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
    let result = state.kernel.pairing_ref().remove_device(&device_id);
    // ErrorTranslator is !Send; drop before any .await below.
    drop(t);
    match result {
        Ok(()) => {
            // Drop this device's bearer from the live auth table so a
            // revoked device's stored key stops authenticating immediately
            // — the persisted device row was just deleted, but without
            // this the in-memory `Vec<ApiUserAuth>` would keep accepting
            // the token until the next process restart.
            let device_user_name = format!("device:{device_id}");
            state
                .user_api_keys
                .write()
                .await
                .retain(|u| u.name != device_user_name);
            tracing::info!(
                target: "pairing.audit",
                device_id = %device_id,
                "revoked paired device — bearer removed from live auth table"
            );
            // DELETE returns 204 No Content with no body (#3843).
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => ApiErrorResponse::not_found(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// POST /api/pairing/notify — Push a notification to all paired devices.
#[utoipa::path(post, path = "/api/pairing/notify", tag = "pairing", request_body = crate::types::JsonObject, responses((status = 200, description = "Notification sent", body = crate::types::JsonObject)))]
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
        serde_json::json!({"cmd": "/new", "desc": "Start a new session (new session id)"}),
        serde_json::json!({"cmd": "/reset", "desc": "Reset current session (clear history, same session id)"}),
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
#[utoipa::path(get, path = "/api/commands/{name}", tag = "system", params(("name" = String, Path, description = "Command name")), responses((status = 200, description = "Command details", body = crate::types::JsonObject)))]
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
        ("/new", "Start a new session (new session id)"),
        (
            "/reset",
            "Reset current session (clear history, same session id)",
        ),
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
        Ok(true) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
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

/// GET /api/tasks — List tasks with optional ?status=, ?assigned_to=, ?limit= filters.
///
/// This is the primary RESTful list endpoint. The legacy /api/tasks/list endpoint
/// remains for backwards compatibility.
pub async fn task_queue_list_root(
    State(state): State<Arc<AppState>>,
    _lang: Option<axum::Extension<RequestLanguage>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let status_filter = params.get("status").map(|s| s.as_str());
    match state.kernel.task_list(status_filter).await {
        Ok(mut tasks) => {
            // Filter by assigned_to if provided
            if let Some(assignee) = params.get("assigned_to") {
                tasks.retain(|t| t["assigned_to"].as_str().unwrap_or("") == assignee.as_str());
            }
            // Apply limit
            if let Some(limit_str) = params.get("limit") {
                if let Ok(limit) = limit_str.parse::<usize>() {
                    tasks.truncate(limit);
                }
            }
            let total = tasks.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({"tasks": tasks, "total": total})),
            )
        }
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// POST /api/tasks — Enqueue a task on behalf of an external caller.
///
/// Body: `{"title": "...", "description": "...", "assigned_to": "<agent-id>"?, "created_by": "<agent-id>"?}`
///
/// Wraps `KernelHandle::task_post` so HTTP clients (skill subprocesses,
/// cron scripts, external integrations) can enqueue tasks without a
/// runtime/agent context. The agent-side `task_post` tool keeps the
/// caller's agent id automatically; this HTTP form takes `created_by`
/// as an optional explicit field for provenance.
pub async fn task_queue_post_root(
    State(state): State<Arc<AppState>>,
    _lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let title = match body["title"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'title' field"})),
            );
        }
    };
    let description = match body["description"].as_str() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'description' field"})),
            );
        }
    };
    let assigned_to = body["assigned_to"].as_str();
    let created_by = body["created_by"].as_str();
    match state
        .kernel
        .task_post(title, description, assigned_to, created_by)
        .await
    {
        Ok(task_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"id": task_id, "status": "pending"})),
        ),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// GET /api/tasks/{id} — Get a single task by ID including its result.
pub async fn task_queue_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let err_not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-task-not-found")
    };
    match state.kernel.task_get(&id).await {
        Ok(Some(task)) => (StatusCode::OK, Json(task)),
        Ok(None) => ApiErrorResponse::not_found(err_not_found).into_json_tuple(),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// PATCH /api/tasks/{id} — Update task status.
///
/// Body: `{"status": "pending"}` or `{"status": "cancelled"}`
/// - `pending`: resets a failed/in_progress task so it can be re-claimed
/// - `cancelled`: cancels a pending/in_progress task
pub async fn task_queue_patch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let err_not_found = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        t.t("api-error-task-not-found")
    };
    let new_status = match body["status"].as_str() {
        Some(s @ ("pending" | "cancelled")) => s.to_string(),
        Some(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Invalid status '{other}': only 'pending' and 'cancelled' are allowed")
                })),
            );
        }
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'status' field"})),
            );
        }
    };
    match state.kernel.task_update_status(&id, &new_status).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "status": new_status})),
        ),
        Ok(false) => ApiErrorResponse::not_found(err_not_found).into_json_tuple(),
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
            // `std::env::set_var` is not thread-safe in an async context; delegate
            // to a blocking thread to avoid UB in the multithreaded tokio runtime.
            {
                let env_var_owned = env_var.clone();
                let key_value_owned = key_value.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    // SAFETY: single mutation on a dedicated blocking thread.
                    unsafe { std::env::set_var(&env_var_owned, &key_value_owned) };
                })
                .await;
            }
        }

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

#[cfg(test)]
mod pairing_tests {
    use super::*;
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn configured_url_takes_precedence_over_host_header() {
        let h = headers(&[("x-forwarded-proto", "https")]);
        let resolved =
            resolve_pairing_base_url(Some("https://configured.example.com"), &h, "host.local")
                .unwrap();
        assert_eq!(resolved, "https://configured.example.com");
    }

    #[test]
    fn configured_url_must_have_scheme() {
        let h = HeaderMap::new();
        let err =
            resolve_pairing_base_url(Some("librefang.example.com"), &h, "host.local").unwrap_err();
        assert!(err.contains("must start with http://"), "got: {err}");
    }

    #[test]
    fn configured_url_rejects_non_http_scheme() {
        let h = HeaderMap::new();
        let err =
            resolve_pairing_base_url(Some("ftp://nope.example.com"), &h, "host.local").unwrap_err();
        assert!(err.contains("must start with"), "got: {err}");
    }

    #[test]
    fn configured_url_trailing_slash_trimmed() {
        let h = HeaderMap::new();
        let resolved = resolve_pairing_base_url(Some("https://x.example.com/"), &h, "").unwrap();
        assert_eq!(resolved, "https://x.example.com");
    }

    #[test]
    fn empty_configured_falls_back_to_host_with_default_scheme() {
        let h = HeaderMap::new();
        let resolved = resolve_pairing_base_url(Some(""), &h, "host.local:4545").unwrap();
        assert_eq!(resolved, "http://host.local:4545");
    }

    #[test]
    fn host_fallback_honors_x_forwarded_proto_https() {
        let h = headers(&[("x-forwarded-proto", "https")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "https://host.local");
    }

    #[test]
    fn host_fallback_handles_multi_value_x_forwarded_proto() {
        // Some proxies append values: take the first.
        let h = headers(&[("x-forwarded-proto", "https, http")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "https://host.local");
    }

    #[test]
    fn host_fallback_blank_x_forwarded_proto_does_not_yield_double_colon() {
        // Header present but empty must NOT produce "://host".
        let h = headers(&[("x-forwarded-proto", "")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "http://host.local");
    }

    #[test]
    fn missing_host_and_configured_returns_err() {
        let h = HeaderMap::new();
        let err = resolve_pairing_base_url(None, &h, "").unwrap_err();
        assert!(err.contains("missing Host header"), "got: {err}");
    }

    #[test]
    fn pairing_complete_request_rejects_missing_token() {
        let json = serde_json::json!({"display_name": "x", "platform": "ios"});
        let parsed: Result<PairingCompleteRequest, _> = serde_json::from_value(json);
        assert!(parsed.is_err(), "missing token should fail to deserialize");
    }

    #[test]
    fn pairing_complete_request_defaults_unknown() {
        let json = serde_json::json!({"token": "abc"});
        let parsed: PairingCompleteRequest = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.token, "abc");
        assert_eq!(parsed.display_name, "unknown");
        assert_eq!(parsed.platform, "unknown");
        assert!(parsed.push_token.is_none());
    }

    #[test]
    fn pairing_complete_request_accepts_full_payload() {
        let json = serde_json::json!({
            "token": "tok",
            "display_name": "My iPhone",
            "platform": "ios",
            "push_token": "fcm-xyz",
        });
        let parsed: PairingCompleteRequest = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.token, "tok");
        assert_eq!(parsed.display_name, "My iPhone");
        assert_eq!(parsed.platform, "ios");
        assert_eq!(parsed.push_token.as_deref(), Some("fcm-xyz"));
    }
}
