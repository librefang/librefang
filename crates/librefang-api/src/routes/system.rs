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

// ---------------------------------------------------------------------------
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
///
/// Auth (#3509): missing or invalid bearer token returns `401 Unauthorized`
/// with a `WWW-Authenticate: Bearer realm="librefang-webhook"` header per
/// RFC 9110 §11.6.1. The previous behaviour (400 Bad Request) confused
/// clients that tried to retry with a fixed body instead of fixing the
/// token.
#[utoipa::path(
    post,
    path = "/api/hooks/wake",
    tag = "webhooks",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Wake hook triggered", body = crate::types::JsonObject),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Webhook triggers not enabled")
    )
)]
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::WakePayload>,
) -> axum::response::Response {
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
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_response();
        }
    };

    // Validate bearer token (constant-time comparison). Invalid token is
    // an authentication failure, not a malformed request — return 401 with
    // the standard `WWW-Authenticate` challenge per RFC 9110 §11.6.1
    // (#3509).
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return webhook_unauthorized_response(err_invalid_token);
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
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
        return ApiErrorResponse::internal(err_msg).into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
        .into_response()
}

/// Build a `401 Unauthorized` response with the standard
/// `WWW-Authenticate: Bearer realm="librefang-webhook"` challenge header
/// (RFC 9110 §11.6.1). Used by webhook trigger endpoints whose bearer-token
/// check failed (#3509).
fn webhook_unauthorized_response(message: String) -> axum::response::Response {
    let body = ApiErrorResponse {
        error: message,
        code: Some("webhook_invalid_token".to_string()),
        r#type: Some("webhook_invalid_token".to_string()),
        details: None,
        status: StatusCode::UNAUTHORIZED,
    };
    let mut resp = body.into_response();
    resp.headers_mut().insert(
        axum::http::header::WWW_AUTHENTICATE,
        axum::http::HeaderValue::from_static("Bearer realm=\"librefang-webhook\""),
    );
    resp
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
///
/// Auth (#3509): missing or invalid bearer token returns `401 Unauthorized`
/// with a `WWW-Authenticate: Bearer realm="librefang-webhook"` header per
/// RFC 9110 §11.6.1, mirroring the `/hooks/wake` fix.
#[utoipa::path(
    post,
    path = "/api/hooks/agent",
    tag = "webhooks",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Agent hook triggered", body = crate::types::JsonObject),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Webhook triggers not enabled or agent not found")
    )
)]
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<librefang_types::webhook::AgentHookPayload>,
) -> axum::response::Response {
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
            return ApiErrorResponse::not_found(err_webhook_not_enabled).into_response();
        }
    };

    // Validate bearer token (#3509: 401 + WWW-Authenticate, not 400).
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return webhook_unauthorized_response(err_invalid_token);
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
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
                        return ApiErrorResponse::not_found(err_msg).into_response();
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent
            match state.kernel.agent_registry().list().first() {
                Some(entry) => entry.id,
                None => {
                    return ApiErrorResponse::not_found(err_no_agents).into_response();
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
        )
            .into_response(),
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            let msg = t.t_args(
                "api-error-webhook-agent-exec-failed",
                &[("error", &e.to_string())],
            );
            ApiErrorResponse::internal(msg).into_response()
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
