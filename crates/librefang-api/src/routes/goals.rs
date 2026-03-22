//! Goals endpoints — hierarchical goal tracking with CRUD operations.

use super::AppState;

/// 构建目标管理领域的路由。
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/goals", axum::routing::get(list_goals).post(create_goal))
        .route(
            "/goals/{id}",
            axum::routing::get(get_goal)
                .put(update_goal_by_id)
                .delete(delete_goal),
        )
        .route(
            "/goals/{id}/children",
            axum::routing::get(get_goal_children),
        )
}
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use std::collections::HashSet;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Goals endpoints
// ---------------------------------------------------------------------------

/// The well-known shared-memory key for goals storage.
const GOALS_KEY: &str = "__librefang_goals";

/// Shared agent ID for goals KV storage.
fn goals_shared_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// GET /api/goals — List all goals.
pub async fn list_goals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = goals_shared_agent_id();
    match state
        .kernel
        .memory_substrate()
        .structured_get(agent_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => {
            let total = arr.len();
            Json(serde_json::json!({"goals": arr, "total": total}))
        }
        Ok(_) => Json(serde_json::json!({"goals": [], "total": 0})),
        Err(e) => {
            tracing::warn!("Failed to load goals: {e}");
            Json(serde_json::json!({"goals": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// GET /api/goals/{id} — Get a specific goal by ID.
pub async fn get_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = goals_shared_agent_id();
    match state
        .kernel
        .memory_substrate()
        .structured_get(agent_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => {
            if let Some(goal) = arr.iter().find(|g| g["id"].as_str() == Some(&id)) {
                (StatusCode::OK, Json(goal.clone()))
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Goal '{}' not found", id)})),
                )
            }
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Goal '{}' not found", id)})),
        ),
        Err(e) => {
            tracing::warn!("Failed to load goals: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to load goals: {e}")})),
            )
        }
    }
}

/// GET /api/goals/{id}/children — Get all direct children of a goal.
pub async fn get_goal_children(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = goals_shared_agent_id();
    match state
        .kernel
        .memory_substrate()
        .structured_get(agent_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => {
            let children: Vec<&serde_json::Value> = arr
                .iter()
                .filter(|g| g["parent_id"].as_str() == Some(&id))
                .collect();
            let total = children.len();
            Json(serde_json::json!({"children": children, "total": total}))
        }
        Ok(_) => Json(serde_json::json!({"children": [], "total": 0})),
        Err(e) => {
            tracing::warn!("Failed to load goals: {e}");
            Json(serde_json::json!({"children": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// POST /api/goals — Create a new goal.
pub async fn create_goal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let title = match req["title"].as_str() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'title' field"})),
            );
        }
    };

    if title.chars().count() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Title too long (max 256 chars)"})),
        );
    }

    let description = req["description"].as_str().unwrap_or("").to_string();
    if description.chars().count() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Description too long (max 4096 chars)"})),
        );
    }

    let parent_id = req["parent_id"].as_str().map(|s| s.to_string());

    let status = req["status"].as_str().unwrap_or("pending").to_string();
    if !["pending", "in_progress", "completed", "cancelled"].contains(&status.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Invalid status. Must be: pending, in_progress, completed, or cancelled"}),
            ),
        );
    }

    let progress = req["progress"].as_u64().unwrap_or(0);
    if progress > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Progress must be 0-100"})),
        );
    }

    let agent_id_str = req["agent_id"].as_str().map(|s| s.to_string());

    let now = chrono::Utc::now().to_rfc3339();
    let goal_id = uuid::Uuid::new_v4().to_string();
    let mut entry = serde_json::json!({
        "id": goal_id,
        "title": title,
        "description": description,
        "status": status,
        "progress": progress,
        "created_at": now,
        "updated_at": now,
    });

    if let Some(ref pid) = parent_id {
        entry["parent_id"] = serde_json::Value::String(pid.clone());
    }
    if let Some(ref aid) = agent_id_str {
        entry["agent_id"] = serde_json::Value::String(aid.clone());
    }

    // Single read-then-write to reduce TOCTOU window between parent validation and list append
    let shared_id = goals_shared_agent_id();
    let mut goals: Vec<serde_json::Value> = match state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        _ => Vec::new(),
    };

    // Validate parent_id exists within the same snapshot
    if let Some(ref pid) = parent_id {
        let parent_exists = goals.iter().any(|g| g["id"].as_str() == Some(pid.as_str()));
        if !parent_exists {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Parent goal '{}' not found", pid)})),
            );
        }
    }

    goals.push(entry.clone());
    if let Err(e) = state.kernel.memory_substrate().structured_set(
        shared_id,
        GOALS_KEY,
        serde_json::Value::Array(goals),
    ) {
        tracing::warn!("Failed to save goal: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save goal: {e}")})),
        );
    }

    (StatusCode::CREATED, Json(entry))
}

/// PUT /api/goals/{id} — Update a goal.
pub async fn update_goal_by_id(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let shared_id = goals_shared_agent_id();
    let mut goals: Vec<serde_json::Value> = match state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        _ => Vec::new(),
    };

    // --- Validate inputs before mutating ---

    if let Some(title) = req.get("title").and_then(|v| v.as_str()) {
        if title.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Title must not be empty"})),
            );
        }
        if title.chars().count() > 256 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Title too long (max 256 chars)"})),
            );
        }
    }

    if let Some(description) = req.get("description").and_then(|v| v.as_str()) {
        if description.chars().count() > 4096 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Description too long (max 4096 chars)"})),
            );
        }
    }

    if let Some(status) = req.get("status").and_then(|v| v.as_str()) {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&status) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid status"})),
            );
        }
    }

    if let Some(progress) = req.get("progress").and_then(|v| v.as_u64()) {
        if progress > 100 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Progress must be 0-100"})),
            );
        }
    }

    // Validate parent_id: detect self-reference, verify existence, and detect indirect cycles
    if let Some(parent_id) = req.get("parent_id") {
        if !parent_id.is_null() {
            if let Some(pid) = parent_id.as_str() {
                if pid == id {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "A goal cannot be its own parent"})),
                    );
                }
                // Verify the target parent exists
                if !goals.iter().any(|g| g["id"].as_str() == Some(pid)) {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(
                            serde_json::json!({"error": format!("Parent goal '{}' not found", pid)}),
                        ),
                    );
                }
                // Detect indirect cycles: walk ancestor chain from `pid` upward.
                // Use a seen set to guard against infinite loops on corrupted data.
                let mut ancestor = Some(pid.to_string());
                let mut seen = HashSet::new();
                seen.insert(id.clone());
                while let Some(ref anc_id) = ancestor {
                    if !seen.insert(anc_id.clone()) {
                        break; // already visited — corrupted data, stop walking
                    }
                    let anc_parent = goals.iter().find_map(|gg| {
                        if gg["id"].as_str() == Some(anc_id) {
                            gg["parent_id"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    });
                    match anc_parent {
                        Some(ref ap) if ap == &id => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(
                                    serde_json::json!({"error": "Circular parent reference detected"}),
                                ),
                            );
                        }
                        Some(ap) => ancestor = Some(ap),
                        None => break,
                    }
                }
            }
        }
    }

    // --- Apply mutations ---

    let mut found = false;
    for g in goals.iter_mut() {
        if g["id"].as_str() == Some(&id) {
            found = true;
            if let Some(title) = req.get("title").and_then(|v| v.as_str()) {
                g["title"] = serde_json::Value::String(title.to_string());
            }
            if let Some(description) = req.get("description").and_then(|v| v.as_str()) {
                g["description"] = serde_json::Value::String(description.to_string());
            }
            if let Some(status) = req.get("status").and_then(|v| v.as_str()) {
                g["status"] = serde_json::Value::String(status.to_string());
            }
            if let Some(progress) = req.get("progress").and_then(|v| v.as_u64()) {
                g["progress"] = serde_json::json!(progress);
            }
            if let Some(parent_id) = req.get("parent_id") {
                if parent_id.is_null() {
                    g.as_object_mut().map(|obj| obj.remove("parent_id"));
                } else if let Some(pid) = parent_id.as_str() {
                    g["parent_id"] = serde_json::Value::String(pid.to_string());
                }
            }
            if let Some(agent_id) = req.get("agent_id") {
                if agent_id.is_null() {
                    g.as_object_mut().map(|obj| obj.remove("agent_id"));
                } else if let Some(aid) = agent_id.as_str() {
                    g["agent_id"] = serde_json::Value::String(aid.to_string());
                }
            }
            g["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            break;
        }
    }

    if !found {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Goal not found"})),
        );
    }

    if let Err(e) = state.kernel.memory_substrate().structured_set(
        shared_id,
        GOALS_KEY,
        serde_json::Value::Array(goals),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update goal: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "updated", "goal_id": id})),
    )
}

/// DELETE /api/goals/{id} — Delete a goal and all its descendants.
pub async fn delete_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = goals_shared_agent_id();
    let mut goals: Vec<serde_json::Value> = match state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        _ => Vec::new(),
    };

    let before = goals.len();

    // Collect all IDs to remove: the target goal + all descendants
    let mut ids_to_remove: HashSet<String> = HashSet::new();
    ids_to_remove.insert(id.clone());
    let mut queue = vec![id.clone()];
    while let Some(current_id) = queue.pop() {
        for g in &goals {
            if g["parent_id"].as_str() == Some(&current_id) {
                if let Some(child_id) = g["id"].as_str() {
                    if ids_to_remove.insert(child_id.to_string()) {
                        queue.push(child_id.to_string());
                    }
                }
            }
        }
    }

    goals.retain(|g| {
        g["id"]
            .as_str()
            .map(|gid| !ids_to_remove.contains(gid))
            .unwrap_or(true)
    });

    if goals.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Goal not found"})),
        );
    }

    let removed = before - goals.len();

    if let Err(e) = state.kernel.memory_substrate().structured_set(
        shared_id,
        GOALS_KEY,
        serde_json::Value::Array(goals),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to delete goal: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "goal_id": id, "removed_count": removed})),
    )
}
