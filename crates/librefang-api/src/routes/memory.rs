//! Proactive memory (mem0-style) API routes.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::memory::ProactiveMemory;

use super::AppState;

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
    state.kernel.proactive_memory.get().cloned().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "Proactive memory is not enabled"})),
        )
    })
}

fn default_user_id() -> String {
    "00000000-0000-0000-0000-000000000000".to_string()
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
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let user_id = default_user_id();
    let limit = params.limit.min(100);
    match store.search(&params.q, &user_id, limit).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory?category=...
// ---------------------------------------------------------------------------

/// List all proactive memories, optionally filtered by category.
#[utoipa::path(
    get,
    path = "/api/memory",
    tag = "proactive-memory",
    params(
        ("category" = Option<String>, Query, description = "Optional category filter"),
    ),
    responses((status = 200, description = "Memory list", body = serde_json::Value))
)]
pub async fn memory_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryListQuery>,
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let user_id = default_user_id();
    match store.list(&user_id, params.category.as_deref()).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match store.get(&user_id).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
    responses((status = 200, description = "Memories added", body = serde_json::Value))
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
            StatusCode::OK,
            Json(serde_json::json!({ "added": items.len(), "memories": items })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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

    let user_id = default_user_id();
    match store.update(&memory_id, &user_id, &body.content).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"updated": true, "memory_id": memory_id})),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Memory not found"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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

    let user_id = default_user_id();
    match store.delete(&memory_id, &user_id).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"deleted": true, "memory_id": memory_id})),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Memory not found"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /api/memory/stats
// ---------------------------------------------------------------------------

/// Get memory statistics for the default user.
#[utoipa::path(
    get,
    path = "/api/memory/stats",
    tag = "proactive-memory",
    responses((status = 200, description = "Memory statistics", body = serde_json::Value))
)]
pub async fn memory_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let user_id = default_user_id();
    match store.stats(&user_id).await {
        Ok(stats) => (StatusCode::OK, Json(serde_json::json!(stats))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({"reset": true, "deleted_count": count})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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

    let level = librefang_types::memory::MemoryLevel::from(level_str.as_str());

    match store.clear_level(&agent_id, level) {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "cleared": true,
                "level": level_str,
                "deleted_count": count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
) -> impl IntoResponse {
    let store = match get_pm_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let limit = params.limit.min(100);
    match store.search(&params.q, &agent_id, limit).await {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({ "memories": items })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
        Ok(history) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "memory_id": memory_id,
                "versions": history,
                "version_count": history.len(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}
