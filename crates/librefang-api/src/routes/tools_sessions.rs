//! Tools and sessions sub-domain extracted from `routes/system.rs` (#3749).
//!
//! Mounts the `/tools/*` and `/sessions/*` routes plus the per-agent
//! `by-label` lookup. Public route paths are unchanged from the original
//! `system::router()` definition. The `PaginationParams` helper lives here
//! because both the session listing endpoints and the approvals listing
//! endpoint share it; `system.rs` references it via
//! `super::tools_sessions::PaginationParams`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool};
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build the tools + sessions sub-router. Mounted via `.merge(...)` from
/// `system::router()` so all paths remain rooted at `/api/...` exactly as
/// before.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        // Tools
        .route("/tools", axum::routing::get(list_tools))
        .route("/tools/{name}", axum::routing::get(get_tool))
        .route("/tools/{name}/invoke", axum::routing::post(invoke_tool))
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
#[utoipa::path(get, path = "/api/tools/{name}", tag = "skills", params(("name" = String, Path, description = "Tool name")), responses((status = 200, description = "Tool details", body = crate::types::JsonObject)))]
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

/// POST /api/tools/{name}/invoke — Invoke a kernel tool directly.
///
/// External integrations (MCP bridges, scripts, automations) can call kernel
/// tools without going through an agent loop. Fail-closed: the endpoint
/// rejects every request unless the tool is listed in
/// `[tool_invoke] allowlist` and `tool_invoke.enabled = true`. Pass
/// `?agent_id=<uuid>` when invoking approval-gated tools so the approval
/// callback can resolve the correct agent; without an `agent_id` those
/// tools are rejected to avoid orphaned deferred executions.
#[utoipa::path(
    post,
    path = "/api/tools/{name}/invoke",
    tag = "tools",
    params(
        ("name" = String, Path, description = "Tool name"),
        ("agent_id" = Option<String>, Query, description = "Caller agent UUID (required for approval-gated tools)")
    ),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Tool execution result", body = crate::types::JsonObject),
        (status = 400, description = "Tool invocation failed or requires an agent context"),
        (status = 403, description = "Endpoint disabled or tool not in allowlist"),
        (status = 404, description = "Tool not found")
    )
)]
pub async fn invoke_tool(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(input): Json<serde_json::Value>,
) -> impl IntoResponse {
    let lang_code = super::resolve_lang(lang.as_ref());

    // `agent_id`, if supplied, MUST be a well-formed UUID regardless of
    // whether the tool is approval-gated. A malformed id would flow into
    // `caller_agent_id` as opaque bytes and surface later as garbage
    // attribution on any tool that reads it for telemetry or audit.
    // Reject it once, at the edge.
    let caller_agent_id: Option<String> = match params.get("agent_id") {
        Some(raw) if raw.parse::<uuid::Uuid>().is_ok() => Some(raw.clone()),
        Some(_) => {
            let t = ErrorTranslator::new(lang_code);
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .into_json_tuple();
        }
        None => None,
    };

    // 1) Fail-closed allowlist check. Without an agent manifest gating which
    //    tools the caller may run, any API-key holder would otherwise be able
    //    to invoke every tool the kernel exposes.
    let cfg = state.kernel.config_snapshot();
    if !cfg.tool_invoke.permits(&name) {
        let t = ErrorTranslator::new(lang_code);
        let msg = if !cfg.tool_invoke.enabled {
            t.t("api-error-tool-invoke-disabled")
        } else {
            t.t_args("api-error-tool-invoke-denied", &[("name", &name)])
        };
        return ApiErrorResponse::forbidden(msg).into_json_tuple();
    }

    // 2) Deterministic existence check: builtin, connected MCP servers, and
    //    skill-provided tools are the three sources execute_tool dispatches
    //    to. Doing this up front lets us return a clean 404 instead of
    //    string-matching the downstream "Unknown tool:" error.
    let tool_exists = builtin_tool_definitions().iter().any(|t| t.name == name)
        || state
            .kernel
            .mcp_tools_ref()
            .lock()
            .map(|mcp_tools| mcp_tools.iter().any(|t| t.name == name))
            .unwrap_or(false)
        || state
            .kernel
            .skill_registry_ref()
            .read()
            .ok()
            .is_some_and(|reg| reg.find_tool_provider(&name).is_some());
    if !tool_exists {
        let t = ErrorTranslator::new(lang_code);
        return ApiErrorResponse::not_found(
            t.t_args("api-error-tool-not-found", &[("name", &name)]),
        )
        .into_json_tuple();
    }

    // 3) Approval-gated tools need a caller_agent_id that the approval
    //    subsystem can later look up. Without one, execute_tool would post
    //    the deferred request with `agent_id = "unknown"` and the approval
    //    could never resolve back to a real agent.
    if state.kernel.approvals().requires_approval(&name) && caller_agent_id.is_none() {
        let t = ErrorTranslator::new(lang_code);
        return ApiErrorResponse::bad_request(
            t.t_args("api-error-tool-requires-agent", &[("name", &name)]),
        )
        .into_json_tuple();
    }

    // 4) Snapshot the skill registry (so the RwLock guard does not cross the
    //    `.await`) and resolve kernel-level sandbox defaults before dispatch.
    let skill_snapshot = state
        .kernel
        .skill_registry_ref()
        .read()
        .ok()
        .map(|g| g.snapshot());
    let workspace_root = cfg.effective_workspaces_dir();
    let exec_policy = cfg.exec_policy.clone();
    let docker_config = cfg.docker.clone();
    let kernel: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;

    let result = execute_tool(
        "rest-api",
        &name,
        &input,
        Some(&kernel),
        None, // allowed_tools — already enforced by tool_invoke.allowlist above
        caller_agent_id.as_deref(),
        skill_snapshot.as_ref(),
        None, // allowed_skills — gated by allowlist above
        Some(state.kernel.mcp_connections_ref()),
        Some(state.kernel.web_tools()),
        Some(state.kernel.browser()),
        None, // allowed_env_vars
        Some(workspace_root.as_path()),
        Some(state.kernel.media()),
        Some(state.kernel.media_drivers()),
        Some(&exec_policy),
        Some(state.kernel.tts()),
        Some(&docker_config),
        Some(state.kernel.processes()),
        Some(state.kernel.process_registry()),
        None, // sender_id
        None, // channel
        None, // checkpoint_manager — snapshotting is wired into agent loops
        None, // interrupt — no session to cancel
        None, // session_id
        None, // dangerous_command_checker — session-scoped, not meaningful here
        None, // available_tools — lazy-load pool not applicable to REST bridge
    )
    .await;

    // Operator audit trail: every direct invocation bypasses the agent loop
    // (and therefore the agent-side audit record) so we log who called what
    // and how it finished. Detail carries the tool name; outcome carries
    // "ok" / the downstream error. The caller_agent_id is used when
    // supplied, otherwise a sentinel so the entry still attributes
    // to "REST caller" rather than appearing as an orphaned agent id.
    let audit_caller = caller_agent_id.as_deref().unwrap_or("rest-api:anonymous");
    let audit_outcome = if result.is_error {
        format!("error: {}", result.content)
    } else {
        "ok".to_string()
    };
    state.kernel.audit().record(
        audit_caller,
        librefang_runtime::audit::AuditAction::ToolInvoke,
        &name,
        &audit_outcome,
    );

    let status = if result.is_error {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    (
        status,
        Json(
            serde_json::to_value(result).unwrap_or_else(
                |_| serde_json::json!({"error": "Failed to serialize tool result"}),
            ),
        ),
    )
}

// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// Pagination query parameters shared by list endpoints. Defined here because
/// the session listing endpoints are the primary user; `system::list_approvals`
/// reuses it via `super::tools_sessions::PaginationParams`.
#[derive(serde::Deserialize, Default)]
pub struct PaginationParams {
    limit: Option<usize>,
    offset: Option<usize>,
}

impl PaginationParams {
    pub(super) const DEFAULT_LIMIT: usize = 50;
    pub(super) const MAX_LIMIT: usize = 500;

    pub(super) fn effective_limit(&self) -> usize {
        self.limit
            .unwrap_or(Self::DEFAULT_LIMIT)
            .min(Self::MAX_LIMIT)
    }

    pub(super) fn effective_offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

/// GET /api/sessions — List all sessions with metadata.
#[utoipa::path(
    get,
    path = "/api/sessions",
    tag = "sessions",
    params(
        ("limit" = Option<usize>, Query, description = "Max items (default 50, max 500)"),
        ("offset" = Option<usize>, Query, description = "Items to skip"),
    ),
    responses(
        (status = 200, description = "Paginated list of sessions", body = crate::types::JsonObject)
    )
)]
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(pagination): Query<PaginationParams>,
) -> impl IntoResponse {
    let offset = pagination.effective_offset();
    let limit = pagination.effective_limit();
    let substrate = state.kernel.memory_substrate();
    // Push pagination into SQLite so we don't deserialize every session blob (#3485).
    let total = substrate.count_sessions().unwrap_or(0);
    // Snapshot of in-flight session IDs from the kernel runtime — the
    // SQLite substrate has no view into liveness, so we merge it in here
    // (#4290). Taken once per request before the SQL call so every row
    // sees a consistent view.
    let running = state.kernel.running_session_ids();
    // Canonical paginated envelope (#3842): {items,total,offset,limit}.
    let (mut items, total, offset_out, limit_out) =
        match substrate.list_sessions_paginated(Some(limit), offset) {
            Ok(items) => (items, total, offset, limit),
            Err(_) => (Vec::new(), 0, 0, PaginationParams::DEFAULT_LIMIT),
        };
    annotate_sessions_active(&mut items, &running);
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: offset_out,
        limit: Some(limit_out),
    })
}

/// Inject an `"active": bool` field into each session JSON row by looking
/// up its `session_id` in the running-tasks snapshot. Rows whose
/// `session_id` doesn't parse as a UUID get `active: false` (a corrupt
/// row can't possibly be in the live registry keyed by `SessionId`).
/// Used by `/api/sessions` and `/api/sessions/search` (#4290).
fn annotate_sessions_active(
    items: &mut [serde_json::Value],
    running: &std::collections::HashSet<librefang_types::agent::SessionId>,
) {
    for item in items.iter_mut() {
        let active = item
            .get("session_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(librefang_types::agent::SessionId)
            .map(|id| running.contains(&id))
            .unwrap_or(false);
        if let Some(obj) = item.as_object_mut() {
            obj.insert("active".to_string(), serde_json::Value::Bool(active));
        }
    }
}

/// GET /api/sessions/:id — Get a single session by ID.
#[utoipa::path(get, path = "/api/sessions/{id}", tag = "sessions", params(("id" = String, Path, description = "Session ID")), responses((status = 200, description = "Session found", body = crate::types::JsonObject), (status = 404, description = "Session not found")))]
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
        Ok(Some((session, created_at))) => {
            // Mirror the list endpoint and surface the kernel runtime's
            // liveness bit (#4290) so single-session fetches don't lie
            // about idle state either.
            let active = state.kernel.running_session_ids().contains(&session.id);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "messages": session.messages,
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "created_at": created_at,
                    "active": active,
                })),
            )
        }
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
) -> axum::response::Response {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => librefang_types::agent::SessionId(u),
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-session-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    match state.kernel.memory_substrate().delete_session(session_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            ApiErrorResponse::internal(t.t_args("api-error-generic", &[("error", &e.to_string())]))
                .into_json_tuple()
                .into_response()
        }
    }
}

/// PUT /api/sessions/:id/label — Set a session label.
#[utoipa::path(put, path = "/api/sessions/{id}/label", tag = "sessions", params(("id" = String, Path, description = "Session ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Label set", body = crate::types::JsonObject)))]
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
#[utoipa::path(get, path = "/api/agents/{id}/sessions/by-label/{label}", tag = "sessions", params(("id" = String, Path, description = "Agent ID"), ("label" = String, Path, description = "Session label")), responses((status = 200, description = "Session found", body = crate::types::JsonObject)))]
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
#[utoipa::path(post, path = "/api/sessions/cleanup", tag = "sessions", responses((status = 200, description = "Cleanup result", body = crate::types::JsonObject)))]
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
        ("limit" = Option<usize>, Query, description = "Max items (default 50, max 500)"),
        ("offset" = Option<usize>, Query, description = "Items to skip"),
    ),
    responses(
        (status = 200, description = "Search results", body = crate::types::JsonObject),
        (status = 400, description = "Missing query parameter"),
    )
)]
pub async fn search_sessions(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    Query(pagination): Query<PaginationParams>,
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

    // Reuse the shared cap policy (default 50 / max 500) instead of
    // re-implementing it from the raw query map. Multiple `Query<T>`
    // extractors are fine — both read the same URI query string and
    // serde_urlencoded ignores fields the target type doesn't declare,
    // so `q`/`agent_id` don't interfere with PaginationParams.
    let limit = pagination.effective_limit();
    let offset = pagination.effective_offset();

    match state.kernel.memory_substrate().search_sessions_paginated(
        &query,
        agent_id.as_ref(),
        Some(limit),
        offset,
    ) {
        Ok(results) => {
            // Canonical paginated envelope (#3842): {items,total,offset,limit}.
            // The substrate has no count() for FTS5 search, so `total` is a
            // best-effort lower bound: when the page isn't full it is exact
            // (`offset + results.len()` == EOF), and when it is full it is at
            // least one greater than `offset + limit`. Clients MUST treat a
            // full page as "more may follow" and keep paginating until a
            // short page comes back.
            let total = if results.len() < limit {
                offset + results.len()
            } else {
                offset + results.len() + 1
            };
            // Project SessionSearchResult into untyped JSON so we can merge
            // the kernel runtime's `active` bit per row (#4290) — same
            // contract as /api/sessions.
            let mut items: Vec<serde_json::Value> = results
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                .collect();
            let running = state.kernel.running_session_ids();
            annotate_sessions_active(&mut items, &running);
            (
                StatusCode::OK,
                Json(
                    serde_json::to_value(crate::types::PaginatedResponse {
                        items,
                        total,
                        offset,
                        limit: Some(limit),
                    })
                    .unwrap_or(serde_json::Value::Null),
                ),
            )
        }
        Err(e) => ApiErrorResponse::internal(e.to_string()).into_json_tuple(),
    }
}
