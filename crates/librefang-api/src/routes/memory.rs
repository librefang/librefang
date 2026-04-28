//! Proactive memory (mem0-style) API routes.

use std::sync::Arc;

use super::AppState;

/// Build routes for the memory/KV domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        // Global proactive memory endpoints
        .route(
            "/memory",
            axum::routing::get(memory_list).post(memory_add),
        )
        .route("/memory/search", axum::routing::get(memory_search))
        .route("/memory/stats", axum::routing::get(memory_stats))
        .route(
            "/memory/config",
            axum::routing::get(memory_config_get).patch(memory_config_patch),
        )
        .route("/memory/cleanup", axum::routing::post(memory_cleanup))
        .route("/memory/decay", axum::routing::post(memory_decay))
        .route(
            "/memory/bulk-delete",
            axum::routing::post(memory_bulk_delete),
        )
        .route(
            "/memory/items/{memory_id}",
            axum::routing::put(memory_update).delete(memory_delete),
        )
        .route(
            "/memory/items/{memory_id}/history",
            axum::routing::get(memory_history),
        )
        .route(
            "/memory/user/{user_id}",
            axum::routing::get(memory_get_user),
        )
        // Per-agent proactive memory endpoints
        .route(
            "/memory/agents/{id}",
            axum::routing::get(memory_list_agent).delete(memory_reset_agent),
        )
        .route(
            "/memory/agents/{id}/search",
            axum::routing::get(memory_search_agent),
        )
        .route(
            "/memory/agents/{id}/stats",
            axum::routing::get(memory_stats_agent),
        )
        .route(
            "/memory/agents/{id}/level/{level}",
            axum::routing::delete(memory_clear_level),
        )
        .route(
            "/memory/agents/{id}/duplicates",
            axum::routing::get(memory_duplicates),
        )
        .route(
            "/memory/agents/{id}/consolidate",
            axum::routing::post(memory_consolidate),
        )
        .route(
            "/memory/agents/{id}/count",
            axum::routing::get(memory_count_agent),
        )
        .route(
            "/memory/agents/{id}/relations",
            axum::routing::get(memory_query_relations).post(memory_store_relations),
        )
        .route(
            "/memory/agents/{id}/export",
            axum::routing::get(memory_export_agent),
        )
        .route(
            "/memory/agents/{id}/import",
            axum::routing::post(memory_import_agent),
        )
}
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::memory::ProactiveMemory;

use crate::types::ApiErrorResponse;
// ---------------------------------------------------------------------------
// Query / path helpers
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct MemorySearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(serde::Deserialize)]
pub struct MemoryListQuery {
    pub category: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(serde::Deserialize)]
pub struct MemoryAddBody {
    pub messages: Vec<serde_json::Value>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct MemoryUpdateBody {
    pub content: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_pm_store(
    state: &AppState,
) -> Result<Arc<librefang_memory::ProactiveMemoryStore>, (StatusCode, Json<serde_json::Value>)> {
    state
        .kernel
        .proactive_memory_store()
        .cloned()
        .ok_or_else(|| {
            ApiErrorResponse::internal("Proactive memory is not enabled").into_json_tuple()
        })
}

fn default_user_id() -> String {
    "00000000-0000-0000-0000-000000000000".to_string()
}

/// Log the full error server-side but return a generic message to the client.
fn internal_error(e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Memory operation failed: {e}");
    ApiErrorResponse::internal("Internal server error").into_json_tuple()
}

/// Build a [`MemoryNamespaceGuard`] for the current request from the
/// authenticated user's RBAC profile (RBAC M3, #3054 Phase 2).
///
/// Resolution order:
/// 1. `axum::Extension<AuthenticatedApiUser>` set by the auth middleware
///    when a per-user API key matched — look up that user by name in the
///    kernel's `AuthManager` and use their resolved `UserMemoryAccess`.
/// 2. No authenticated user (loopback dev / single-user mode, or any
///    request the auth middleware allowed through without binding a
///    user) → fall back to a **fail-closed Viewer-equivalent** guard:
///    read access is limited to the `proactive` namespace, all writes,
///    deletes, exports, and PII access are denied.
///
/// SECURITY (PR #3205 follow-up — Issue #6 fail-open fix): the previous
/// fallback granted **owner-equivalent** access (`readable=["*"]`,
/// `writable=["*"]`, `pii_access=true`, `export_allowed=true`,
/// `delete_allowed=true`) to anonymous loopback callers. That meant any
/// process with `127.0.0.1` access (or any deployment with
/// `LIBREFANG_ALLOW_NO_AUTH=1`) could exfiltrate every memory fragment
/// — including PII — and bulk-delete/export memories without
/// attribution. Other admin-gated RBAC endpoints (`/api/audit/query`,
/// `/api/budget/users/*`, `/api/authz/effective/*`) already reject
/// anonymous callers outright with `PermissionDenied` audit rows.
///
/// We pick the slightly looser "Viewer-equivalent" fallback (rather
/// than a hard 403) so the loopback dashboard SPA — which today hits
/// these endpoints with no Bearer token — keeps working for the
/// non-sensitive read path. Dangerous capabilities (PII, export, write,
/// delete, `kv:*`/`shared:*` namespaces) all fail closed: the guarded
/// store calls return `AuthDenied` → 403 to the client. To regain the
/// previous broad access, configure at least one user with an API key +
/// an `Owner`/`Admin` role and use that token; the auth middleware will
/// attach `AuthenticatedApiUser` and the matching ACL applies.
fn guard_for_request(
    state: &AppState,
    extensions: &axum::http::Extensions,
) -> librefang_memory::namespace_acl::MemoryNamespaceGuard {
    use librefang_memory::namespace_acl::MemoryNamespaceGuard;

    if let Some(api_user) = extensions.get::<crate::middleware::AuthenticatedApiUser>() {
        let user_id = librefang_types::agent::UserId::from_name(&api_user.name);
        if let Some(acl) = state.kernel.auth_manager().memory_acl_for(user_id) {
            return MemoryNamespaceGuard::new(acl);
        }
    }
    MemoryNamespaceGuard::new(anonymous_fallback_acl())
}

/// Least-privilege ACL handed out when the request has no authenticated
/// `AuthenticatedApiUser` (anonymous loopback / `LIBREFANG_ALLOW_NO_AUTH=1`).
///
/// Mirrors `librefang_kernel::auth::default_memory_acl(UserRole::Viewer)`
/// — read-only access to the `proactive` namespace, no PII, no writes,
/// no exports, no deletes. We deliberately do NOT call into the kernel
/// helper directly; inlining here keeps the API-layer fail-closed
/// contract self-contained and visible at the only call site. See the
/// SECURITY note on [`guard_for_request`].
fn anonymous_fallback_acl() -> librefang_types::user_policy::UserMemoryAccess {
    librefang_types::user_policy::UserMemoryAccess {
        readable_namespaces: vec!["proactive".into()],
        writable_namespaces: vec![],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    }
}

/// Convert an `AuthDenied` error to a 403 JSON response **and** record a
/// `PermissionDenied` audit row.
///
/// The reviewer of PR #3205 flagged that memory ACL denials at the API
/// layer were silently dropped from the audit chain, while the parallel
/// `routes/audit.rs`, `routes/budget.rs`, `routes/authz.rs`, and the
/// global auth middleware all emit a `PermissionDenied` row. This helper
/// closes that gap so a privilege probe against `/api/memory*` shows up
/// in `/api/audit` and the `audit.log` chain.
///
/// Anonymous (loopback / no-auth mode) callers are recorded with
/// `user_id = None` and the reason string in the detail field — same
/// shape as `routes/audit.rs::require_admin`.
fn auth_denied(
    state: &AppState,
    extensions: &axum::http::Extensions,
    reason: impl std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    let reason_str = reason.to_string();
    let api_user = extensions.get::<crate::middleware::AuthenticatedApiUser>();
    let (user_id, detail) = match api_user {
        Some(u) => (
            Some(u.user_id),
            format!(
                "memory denied for {} (role={}): {reason_str}",
                u.name, u.role
            ),
        ),
        None => (
            None,
            format!("memory denied for anonymous caller: {reason_str}"),
        ),
    };
    state.kernel.audit().record_with_context(
        "system",
        librefang_runtime::audit::AuditAction::PermissionDenied,
        detail,
        "denied",
        user_id,
        Some("api".to_string()),
    );
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": reason_str})),
    )
}

// ---------------------------------------------------------------------------
// GET /api/memory/search?q=...&limit=10
// ---------------------------------------------------------------------------

/// Search proactive memories by semantic similarity.
#[utoipa::path(
    get,
    path = "/api/memory/search",
    tag = "proactive-memory",
    params(
        ("q" = String, Query, description = "Search query"),
        ("limit" = usize, Query, description = "Max results (default 10)"),
    ),
    responses((status = 200, description = "Search results", body = serde_json::Value))
)]
pub async fn memory_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemorySearchQuery>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let guard = guard_for_request(&state, request.extensions());
    let limit = params.limit.min(100);
    // Search across ALL agents so the dashboard shows all memories
    match store.search_all_with_guard(&params.q, limit, &guard).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(librefang_types::error::LibreFangError::AuthDenied(reason)) => {
            auth_denied(&state, request.extensions(), reason)
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory?category=...
// ---------------------------------------------------------------------------

/// List all proactive memories, optionally filtered by category, with pagination.
///
/// When proactive memory is disabled in config, returns an empty list with
/// `proactive_enabled: false` (HTTP 200) so the dashboard can render an
/// explanatory note instead of treating a config state as a server error.
#[utoipa::path(
    get,
    path = "/api/memory",
    tag = "proactive-memory",
    params(
        ("category" = Option<String>, Query, description = "Optional category filter"),
        ("offset" = Option<usize>, Query, description = "Pagination offset (default 0)"),
        ("limit" = Option<usize>, Query, description = "Page size (default 10, max 100)"),
    ),
    responses((status = 200, description = "Paginated memory list", body = serde_json::Value))
)]
pub async fn memory_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryListQuery>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    // Graceful degradation: proactive memory disabled → empty list, not 500.
    let Some(store) = state.kernel.proactive_memory_store().cloned() else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "memories": [],
                "total": 0,
                "offset": params.offset,
                "limit": params.limit.min(100),
                "proactive_enabled": false,
            })),
        );
    };

    let guard = guard_for_request(&state, request.extensions());
    let limit = params.limit.min(100);
    let offset = params.offset;

    // List across ALL agents so the dashboard shows all memories
    match store
        .list_all_with_guard(params.category.as_deref(), &guard)
        .await
    {
        Ok(items) => {
            let total = items.len();
            let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "memories": page,
                    "total": total,
                    "offset": offset,
                    "limit": limit,
                    "proactive_enabled": true,
                })),
            )
        }
        Err(librefang_types::error::LibreFangError::AuthDenied(reason)) => {
            auth_denied(&state, request.extensions(), reason)
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/:user_id
// ---------------------------------------------------------------------------

/// Get all memories for a specific user.
#[utoipa::path(
    get,
    path = "/api/memory/user/{user_id}",
    tag = "proactive-memory",
    params(("user_id" = String, Path, description = "User ID")),
    responses((status = 200, description = "User memories", body = serde_json::Value))
)]
pub async fn memory_get_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let guard = guard_for_request(&state, request.extensions());
    match store.get_with_guard(&user_id, &guard).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(librefang_types::error::LibreFangError::AuthDenied(reason)) => {
            auth_denied(&state, request.extensions(), reason)
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory
// ---------------------------------------------------------------------------

/// Add memories from messages (uses extraction pipeline).
#[utoipa::path(
    post,
    path = "/api/memory",
    tag = "proactive-memory",
    request_body = serde_json::Value,
    responses((status = 201, description = "Memories added", body = serde_json::Value))
)]
pub async fn memory_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MemoryAddBody>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // In the proactive memory system, user_id maps to agent_id internally.
    // If agent_id is provided, prefer it; otherwise use user_id.
    let effective_id = body
        .agent_id
        .or(body.user_id)
        .unwrap_or_else(default_user_id);

    match store.add(&body.messages, &effective_id).await {
        Ok(items) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "added": items.len(), "memories": items })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// PUT /api/memory/:memory_id
// ---------------------------------------------------------------------------

/// Update a memory's content by ID.
#[utoipa::path(
    put,
    path = "/api/memory/items/{memory_id}",
    tag = "proactive-memory",
    params(("memory_id" = String, Path, description = "Memory ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Memory updated", body = serde_json::Value))
)]
pub async fn memory_update(
    State(state): State<Arc<AppState>>,
    Path(memory_id): Path<String>,
    Json(body): Json<MemoryUpdateBody>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    if body.content.trim().is_empty() {
        return ApiErrorResponse::bad_request("Content must not be empty").into_json_tuple();
    }

    // Look up the real agent_id that owns this memory so KV cleanup works correctly
    let real_agent_id = match store.find_agent_id_for_memory(&memory_id) {
        Ok(Some(aid)) => aid.0.to_string(),
        Ok(None) => {
            return ApiErrorResponse::not_found("Memory not found").into_json_tuple();
        }
        Err(e) => {
            return internal_error(e);
        }
    };

    match store
        .update(&memory_id, &real_agent_id, &body.content)
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"updated": true, "memory_id": memory_id})),
        ),
        Ok(false) => ApiErrorResponse::not_found("Memory not found").into_json_tuple(),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/memory/:memory_id
// ---------------------------------------------------------------------------

/// Delete a specific memory by ID.
#[utoipa::path(
    delete,
    path = "/api/memory/items/{memory_id}",
    tag = "proactive-memory",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses((status = 200, description = "Memory deleted", body = serde_json::Value))
)]
pub async fn memory_delete(
    State(state): State<Arc<AppState>>,
    Path(memory_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Look up the real agent_id that owns this memory so KV cleanup works correctly
    let real_agent_id = match store.find_agent_id_for_memory(&memory_id) {
        Ok(Some(aid)) => aid.0.to_string(),
        Ok(None) => {
            return ApiErrorResponse::not_found("Memory not found").into_json_tuple();
        }
        Err(e) => {
            return internal_error(e);
        }
    };

    match store.delete(&memory_id, &real_agent_id).await {
        Ok(true) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        Ok(false) => ApiErrorResponse::not_found("Memory not found").into_json_tuple(),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/bulk-delete — Delete multiple memories at once
// ---------------------------------------------------------------------------

/// Bulk-delete multiple memories by ID.
#[utoipa::path(
    post,
    path = "/api/memory/bulk-delete",
    tag = "proactive-memory",
    request_body = serde_json::Value,
    responses((status = 200, description = "Bulk delete results", body = serde_json::Value))
)]
pub async fn memory_bulk_delete(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let ids: Vec<String> = match body.get("ids").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => {
            return ApiErrorResponse::bad_request("missing 'ids' array").into_json_tuple();
        }
    };

    let mut deleted = 0usize;
    let mut failed = 0usize;
    for id in &ids {
        let agent_id = match store.find_agent_id_for_memory(id) {
            Ok(Some(aid)) => aid.0.to_string(),
            _ => {
                failed += 1;
                continue;
            }
        };
        match store.delete(id, &agent_id).await {
            Ok(true) => deleted += 1,
            _ => failed += 1,
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "deleted": deleted,
            "failed": failed,
            "total": ids.len(),
        })),
    )
}

// ---------------------------------------------------------------------------
// GET /api/memory/stats
// ---------------------------------------------------------------------------

/// Get memory statistics across all agents.
///
/// When proactive memory is disabled, returns `{stats: null, proactive_enabled: false}`
/// at HTTP 200 — disabled is a config state, not an error.
#[utoipa::path(
    get,
    path = "/api/memory/stats",
    tag = "proactive-memory",
    responses((status = 200, description = "Memory statistics", body = serde_json::Value))
)]
pub async fn memory_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Graceful degradation: proactive memory disabled → null stats, not 500.
    let Some(store) = state.kernel.proactive_memory_store().cloned() else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "stats": null,
                "proactive_enabled": false,
            })),
        );
    };

    // Aggregate stats across ALL agents so the dashboard shows global totals.
    // Merge `proactive_enabled: true` into the stats object so callers can
    // branch on a single field regardless of which path returned.
    match store.stats_all().await {
        Ok(stats) => {
            let mut value = serde_json::json!(stats);
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "proactive_enabled".to_string(),
                    serde_json::Value::Bool(true),
                );
            }
            (StatusCode::OK, Json(value))
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/memory/agents/:agent_id — Reset all memories for an agent
// ---------------------------------------------------------------------------

/// Delete all proactive memories for a specific agent.
#[utoipa::path(
    delete,
    path = "/api/memory/agents/{id}",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Memories reset", body = serde_json::Value))
)]
pub async fn memory_reset_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.reset(&agent_id) {
        Ok(_count) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/memory/agents/:agent_id/level/:level
// ---------------------------------------------------------------------------

/// Clear memories at a specific level (user/session/agent) for an agent.
#[utoipa::path(
    delete,
    path = "/api/memory/agents/{id}/level/{level}",
    tag = "proactive-memory",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("level" = String, Path, description = "Memory level: user, session, or agent"),
    ),
    responses((status = 200, description = "Memories cleared at level", body = serde_json::Value))
)]
pub async fn memory_clear_level(
    State(state): State<Arc<AppState>>,
    Path((agent_id, level_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Validate the level string before conversion to avoid silently
    // defaulting to Session and deleting the wrong memories.
    let level = match level_str.to_lowercase().as_str() {
        "user" | "user_memory" | "session" | "session_memory" | "agent" | "agent_memory" => {
            librefang_types::memory::MemoryLevel::from(level_str.as_str())
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!(
                        "Invalid memory level '{}'. Must be one of: user, session, agent",
                        level_str
                    )
                })),
            );
        }
    };

    match store.clear_level(&agent_id, level) {
        Ok(_count) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id?offset=0&limit=20
// ---------------------------------------------------------------------------

/// List memories for a specific agent with pagination.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}",
    tag = "proactive-memory",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("category" = Option<String>, Query, description = "Optional category filter"),
        ("offset" = Option<usize>, Query, description = "Pagination offset (default 0)"),
        ("limit" = Option<usize>, Query, description = "Page size (default 10, max 100)"),
    ),
    responses((status = 200, description = "Paginated agent memory list", body = serde_json::Value))
)]
pub async fn memory_list_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(params): Query<MemoryListQuery>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let guard = guard_for_request(&state, request.extensions());
    let limit = params.limit.min(100);
    let offset = params.offset;

    match store
        .list_with_guard(&agent_id, params.category.as_deref(), &guard)
        .await
    {
        Ok(items) => {
            let total = items.len();
            let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "memories": page,
                    "total": total,
                    "offset": offset,
                    "limit": limit,
                })),
            )
        }
        Err(librefang_types::error::LibreFangError::AuthDenied(reason)) => {
            auth_denied(&state, request.extensions(), reason)
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/search?q=...&limit=10
// ---------------------------------------------------------------------------

/// Search memories scoped to a specific agent.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/search",
    tag = "proactive-memory",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("q" = String, Query, description = "Search query"),
        ("limit" = usize, Query, description = "Max results (default 10)"),
    ),
    responses((status = 200, description = "Search results", body = serde_json::Value))
)]
pub async fn memory_search_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(params): Query<MemorySearchQuery>,
    request: axum::extract::Request,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let guard = guard_for_request(&state, request.extensions());
    let limit = params.limit.min(100);
    match store
        .search_with_guard(&params.q, &agent_id, limit, &guard)
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(librefang_types::error::LibreFangError::AuthDenied(reason)) => {
            auth_denied(&state, request.extensions(), reason)
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/stats
// ---------------------------------------------------------------------------

/// Get memory statistics for a specific agent.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/stats",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Agent memory statistics", body = serde_json::Value))
)]
pub async fn memory_stats_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.stats(&agent_id).await {
        Ok(stats) => (StatusCode::OK, Json(serde_json::json!(stats))),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/duplicates
// ---------------------------------------------------------------------------

/// Find duplicate/near-duplicate memories for an agent.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/duplicates",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Duplicate memory groups", body = serde_json::Value))
)]
pub async fn memory_duplicates(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.find_duplicates(&agent_id, None).await {
        Ok(groups) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "duplicate_groups": groups.len(),
                "groups": groups,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/:memory_id/history
// ---------------------------------------------------------------------------

/// Get the version history of a specific memory.
#[utoipa::path(
    get,
    path = "/api/memory/items/{memory_id}/history",
    tag = "proactive-memory",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses((status = 200, description = "Memory version history", body = serde_json::Value))
)]
pub async fn memory_history(
    State(state): State<Arc<AppState>>,
    Path(memory_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.history(&memory_id) {
        Ok(history) => {
            let count = history.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "memory_id": memory_id,
                    "versions": history,
                    "version_count": count,
                })),
            )
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/agents/:agent_id/consolidate
// ---------------------------------------------------------------------------

/// Consolidate memories for an agent: merge duplicates, cleanup stale entries.
#[utoipa::path(
    post,
    path = "/api/memory/agents/{id}/consolidate",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Consolidation result", body = serde_json::Value))
)]
pub async fn memory_consolidate(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.consolidate(&agent_id).await {
        Ok(merged) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "consolidated": true,
                "merged_count": merged,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/cleanup
// ---------------------------------------------------------------------------

/// Clean up expired session-level memories across all agents.
///
/// Deletes session memories older than `session_ttl_hours` (default 24).
/// Only session-level memories are affected — user and agent memories are persistent.
#[utoipa::path(
    post,
    path = "/api/memory/cleanup",
    tag = "proactive-memory",
    responses((status = 200, description = "Cleanup result", body = serde_json::Value))
)]
pub async fn memory_cleanup(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.cleanup_expired() {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "cleaned_up": true,
                "deleted_count": deleted,
                "session_ttl_hours": store.config().session_ttl_hours,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/export
// ---------------------------------------------------------------------------

/// Export all proactive memories for an agent as JSON.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/export",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Exported memories", body = serde_json::Value))
)]
pub async fn memory_export_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.export_all(&agent_id) {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "agent_id": agent_id,
                "count": items.len(),
                "memories": items,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/agents/:agent_id/import
// ---------------------------------------------------------------------------

/// Import proactive memories for an agent from JSON.
#[utoipa::path(
    post,
    path = "/api/memory/agents/{id}/import",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Import result", body = serde_json::Value))
)]
pub async fn memory_import_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(body): Json<Vec<librefang_memory::MemoryExportItem>>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.import_memories(&agent_id, body).await {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "imported": count,
                "agent_id": agent_id,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/decay
// ---------------------------------------------------------------------------

/// Trigger manual confidence decay across all memories.
///
/// Applies time-based exponential decay: memories not accessed recently
/// lose confidence, while frequently accessed memories get boosted.
/// Normally runs automatically during maintenance, but this endpoint
/// allows triggering it on demand.
#[utoipa::path(
    post,
    path = "/api/memory/decay",
    tag = "proactive-memory",
    responses((status = 200, description = "Decay result", body = serde_json::Value))
)]
pub async fn memory_decay(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.decay_confidence() {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "decayed": true,
                "decay_rate": store.config().confidence_decay_rate,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/count?level=user
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct MemoryCountQuery {
    pub level: Option<String>,
}

/// Count memories for an agent, optionally filtered by level (user/session/agent).
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/count",
    tag = "proactive-memory",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("level" = Option<String>, Query, description = "Memory level filter (user, session, agent)"),
    ),
    responses((status = 200, description = "Memory count", body = serde_json::Value))
)]
pub async fn memory_count_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(params): Query<MemoryCountQuery>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let level = params.level.as_deref().and_then(|l| match l {
        "user" => Some(librefang_types::memory::MemoryLevel::User),
        "session" => Some(librefang_types::memory::MemoryLevel::Session),
        "agent" => Some(librefang_types::memory::MemoryLevel::Agent),
        _ => None,
    });

    match store.count(&agent_id, level) {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "agent_id": agent_id,
                "count": count,
                "level": params.level,
            })),
        ),
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// POST /api/memory/agents/:agent_id/relations
// ---------------------------------------------------------------------------

/// Store relation triples into the knowledge graph for an agent.
///
/// Accepts an array of `{ subject, subject_type, relation, object, object_type }` triples.
/// Deduplicates automatically: existing identical relations are skipped.
#[utoipa::path(
    post,
    path = "/api/memory/agents/{id}/relations",
    tag = "proactive-memory",
    params(("id" = String, Path, description = "Agent ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Relations stored", body = serde_json::Value))
)]
pub async fn memory_store_relations(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(triples): Json<Vec<librefang_types::memory::RelationTriple>>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let count = triples.len();
    store.store_relations(&triples, &agent_id);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id,
            "triples_processed": count,
        })),
    )
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/:agent_id/relations?source=...&relation=...&target=...
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct RelationQueryParams {
    pub source: Option<String>,
    pub relation: Option<String>,
    pub target: Option<String>,
}

/// Query the knowledge graph for relations matching a pattern.
///
/// All query parameters are optional — omitting all returns up to 100 relations.
/// Results include full source entity, relation, and target entity for each match.
#[utoipa::path(
    get,
    path = "/api/memory/agents/{id}/relations",
    tag = "proactive-memory",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("source" = Option<String>, Query, description = "Source entity name or ID"),
        ("relation" = Option<String>, Query, description = "Relation type"),
        ("target" = Option<String>, Query, description = "Target entity name or ID"),
    ),
    responses((status = 200, description = "Matching relations", body = serde_json::Value))
)]
pub async fn memory_query_relations(
    State(state): State<Arc<AppState>>,
    Path(_agent_id): Path<String>,
    Query(params): Query<RelationQueryParams>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Parse optional relation type
    let relation_type = params.relation.as_deref().and_then(|r| {
        serde_json::from_str::<librefang_types::memory::RelationType>(&format!("\"{}\"", r)).ok()
    });

    let pattern = librefang_types::memory::GraphPattern {
        source: params.source,
        relation: relation_type,
        target: params.target,
        max_depth: 1,
    };

    match store.query_relations(pattern) {
        Ok(matches) => {
            let results: Vec<serde_json::Value> = matches
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "source": {
                            "id": m.source.id,
                            "name": m.source.name,
                            "entity_type": m.source.entity_type,
                        },
                        "relation": {
                            "type": m.relation.relation,
                            "confidence": m.relation.confidence,
                        },
                        "target": {
                            "id": m.target.id,
                            "name": m.target.name,
                            "entity_type": m.target.entity_type,
                        },
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "matches": results,
                    "count": results.len(),
                })),
            )
        }
        Err(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/config — Get memory configuration
// ---------------------------------------------------------------------------

#[utoipa::path(get, path = "/api/memory/config", tag = "memory", responses((status = 200, description = "Memory configuration", body = serde_json::Value)))]
pub async fn memory_config_get(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.kernel.config_ref();
    Json(serde_json::json!({
        "embedding_provider": config.memory.embedding_provider,
        "embedding_model": &config.memory.embedding_model,
        "embedding_api_key_env": config.memory.embedding_api_key_env,
        "decay_rate": config.memory.decay_rate,
        "proactive_memory": {
            "enabled": config.proactive_memory.enabled,
            "auto_memorize": config.proactive_memory.auto_memorize,
            "auto_retrieve": config.proactive_memory.auto_retrieve,
            "extraction_model": &config.proactive_memory.extraction_model,
            "max_retrieve": config.proactive_memory.max_retrieve,
        },
    }))
}

// ---------------------------------------------------------------------------
// PATCH /api/memory/config — Update memory configuration (writes config.toml)
// ---------------------------------------------------------------------------

#[utoipa::path(patch, path = "/api/memory/config", tag = "memory", request_body = serde_json::Value, responses((status = 200, description = "Memory configuration updated", body = serde_json::Value)))]
pub async fn memory_config_patch(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let config_path = state.kernel.home_dir().join("config.toml");

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to read config: {e}"))
                .into_json_tuple();
        }
    };
    let mut table: toml::Value = match toml::from_str(&content) {
        Ok(t) => t,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to parse config: {e}"))
                .into_json_tuple();
        }
    };

    let root = table.as_table_mut().unwrap();

    // Update [memory] section
    let memory_tbl = root
        .entry("memory")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();
    if let Some(v) = req.get("embedding_provider").and_then(|v| v.as_str()) {
        memory_tbl.insert(
            "embedding_provider".into(),
            toml::Value::String(v.to_string()),
        );
    }
    if let Some(v) = req.get("embedding_model").and_then(|v| v.as_str()) {
        memory_tbl.insert("embedding_model".into(), toml::Value::String(v.to_string()));
    }
    if let Some(v) = req.get("embedding_api_key_env").and_then(|v| v.as_str()) {
        memory_tbl.insert(
            "embedding_api_key_env".into(),
            toml::Value::String(v.to_string()),
        );
    }
    if let Some(v) = req.get("decay_rate").and_then(|v| v.as_f64()) {
        memory_tbl.insert("decay_rate".into(), toml::Value::Float(v));
    }

    // Update [proactive_memory] section
    if let Some(pm) = req.get("proactive_memory") {
        let pm_tbl = root
            .entry("proactive_memory")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .unwrap();
        if let Some(v) = pm.get("enabled").and_then(|v| v.as_bool()) {
            pm_tbl.insert("enabled".into(), toml::Value::Boolean(v));
        }
        if let Some(v) = pm.get("auto_memorize").and_then(|v| v.as_bool()) {
            pm_tbl.insert("auto_memorize".into(), toml::Value::Boolean(v));
        }
        if let Some(v) = pm.get("auto_retrieve").and_then(|v| v.as_bool()) {
            pm_tbl.insert("auto_retrieve".into(), toml::Value::Boolean(v));
        }
        if let Some(v) = pm.get("extraction_model").and_then(|v| v.as_str()) {
            pm_tbl.insert(
                "extraction_model".into(),
                toml::Value::String(v.to_string()),
            );
        }
        if let Some(v) = pm.get("max_retrieve").and_then(|v| v.as_u64()) {
            pm_tbl.insert("max_retrieve".into(), toml::Value::Integer(v as i64));
        }
    }

    let new_content = toml::to_string_pretty(&table).unwrap_or_default();
    if let Err(e) = std::fs::write(&config_path, &new_content) {
        return ApiErrorResponse::internal(format!("Failed to write config: {e}"))
            .into_json_tuple();
    }

    tracing::info!("Memory config updated via API");

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "updated", "note": "Restart required for full effect"})),
    )
}

#[cfg(test)]
mod tests {
    //! Regression tests for PR #3205 follow-ups.
    //!
    //! - **Issue #6 (fail-open)**: anonymous request (no
    //!   `AuthenticatedApiUser` extension) must get a Viewer-equivalent
    //!   ACL, NOT the historical owner-equivalent fallback.
    //!   `anonymous_fallback_*` tests pin that contract.
    //! - **Issue #8b (missing audit emit)**: a memory ACL denial at the
    //!   API layer must record a `PermissionDenied` audit row, matching
    //!   `routes/audit.rs`, `routes/budget.rs`, `routes/authz.rs`, and
    //!   the global auth middleware. `auth_denied_emits_audit_*` tests
    //!   pin that contract.
    //!
    //! `anonymous_fallback_*` tests exercise the helper directly because
    //! constructing a real [`AppState`] requires booting an entire
    //! kernel; `auth_denied_*` tests do boot a kernel because we need to
    //! observe the audit chain.
    use super::*;
    use librefang_memory::namespace_acl::{MemoryNamespaceGuard, NamespaceGate};
    use librefang_runtime::audit::AuditAction;
    use librefang_types::config::KernelConfig;

    #[test]
    fn anonymous_fallback_denies_pii_export_and_delete() {
        let acl = anonymous_fallback_acl();
        assert!(
            !acl.pii_access,
            "anonymous fallback must NOT expose PII (was true pre-fix — Issue #6)"
        );
        assert!(
            !acl.export_allowed,
            "anonymous fallback must NOT allow bulk export"
        );
        assert!(
            !acl.delete_allowed,
            "anonymous fallback must NOT allow delete"
        );
        assert!(
            acl.writable_namespaces.is_empty(),
            "anonymous fallback must NOT permit writes, got {:?}",
            acl.writable_namespaces
        );
        assert_eq!(
            acl.readable_namespaces,
            vec!["proactive".to_string()],
            "anonymous fallback must only allow reading the `proactive` namespace"
        );
    }

    #[test]
    fn anonymous_fallback_guard_gates_match_acl_intent() {
        let guard = MemoryNamespaceGuard::new(anonymous_fallback_acl());

        assert!(matches!(
            guard.check_read("proactive"),
            NamespaceGate::Allow
        ));

        assert!(matches!(
            guard.check_read("kv:secrets"),
            NamespaceGate::Deny(_)
        ));
        assert!(matches!(
            guard.check_read("shared:any"),
            NamespaceGate::Deny(_)
        ));
        assert!(matches!(guard.check_read("kg"), NamespaceGate::Deny(_)));
        assert!(matches!(
            guard.check_write("proactive"),
            NamespaceGate::Deny(_)
        ));
        assert!(matches!(
            guard.check_export("proactive"),
            NamespaceGate::Deny(_)
        ));
        assert!(matches!(
            guard.check_delete("proactive"),
            NamespaceGate::Deny(_)
        ));
        assert!(
            !guard.pii_access_allowed(),
            "fallback guard must redact PII"
        );
    }

    /// Minimal `AppState` for unit-testing the audit-emit path of
    /// [`auth_denied`]. Mirrors the fixture in `routes/agents.rs` but
    /// keeps fields to the bare minimum we touch here.
    fn audit_test_app_state() -> (Arc<AppState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("librefang-memory-audit-test");
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            config_write_lock: tokio::sync::Mutex::new(()),
        });
        (state, tmp)
    }

    /// Reviewer claim (PR #3205 follow-up #8b): a memory ACL denial at
    /// the API layer must emit a `PermissionDenied` audit row, matching
    /// `routes/audit.rs`, `routes/budget.rs`, `routes/authz.rs`, and the
    /// global auth middleware. Without this, a privilege probe against
    /// `/api/memory*` was silently dropped from the chain.
    ///
    /// Anonymous (loopback / no-auth) variant — the row is recorded with
    /// `user_id = None`.
    #[tokio::test(flavor = "multi_thread")]
    async fn auth_denied_emits_audit_row_for_anonymous() {
        let (state, _tmp) = audit_test_app_state();
        let extensions = axum::http::Extensions::new();

        let before = state.kernel.audit().len();
        let (status, _body) = auth_denied(
            &state,
            &extensions,
            "namespace 'kv:secrets' is not readable for the current user",
        );
        assert_eq!(status, StatusCode::FORBIDDEN);

        let entries = state.kernel.audit().recent(8);
        assert!(
            state.kernel.audit().len() > before,
            "auth_denied must append at least one audit entry"
        );
        let last = entries.last().expect("audit log must have a tail entry");
        assert!(matches!(last.action, AuditAction::PermissionDenied));
        assert_eq!(last.outcome, "denied");
        assert!(
            last.detail.contains("anonymous"),
            "anonymous detail should mark the caller: got {:?}",
            last.detail
        );
        assert!(
            last.detail.contains("kv:secrets"),
            "detail should carry the rejected namespace reason: got {:?}",
            last.detail
        );
        assert!(
            last.user_id.is_none(),
            "anonymous denial must not attribute a user_id"
        );
        assert_eq!(last.channel.as_deref(), Some("api"));
    }

    /// Authenticated-but-denied variant — the row carries the attributed
    /// `user_id` so an admin can see *who* tried to read what.
    #[tokio::test(flavor = "multi_thread")]
    async fn auth_denied_emits_audit_row_for_authenticated_user() {
        use crate::middleware::AuthenticatedApiUser;
        use librefang_kernel::auth::UserRole;
        use librefang_types::agent::UserId;

        let (state, _tmp) = audit_test_app_state();
        let mut extensions = axum::http::Extensions::new();
        let user_id = UserId::from_name("alice");
        extensions.insert(AuthenticatedApiUser {
            name: "alice".to_string(),
            role: UserRole::User,
            user_id,
        });

        let (status, _body) = auth_denied(&state, &extensions, "kv:secrets not readable");
        assert_eq!(status, StatusCode::FORBIDDEN);

        let last = state
            .kernel
            .audit()
            .recent(4)
            .into_iter()
            .last()
            .expect("audit must have a tail entry");
        assert!(matches!(last.action, AuditAction::PermissionDenied));
        assert_eq!(last.user_id, Some(user_id));
        assert!(
            last.detail.contains("alice"),
            "authenticated detail should name the user: got {:?}",
            last.detail
        );
    }
}
