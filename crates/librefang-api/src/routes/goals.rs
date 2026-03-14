//! Goals endpoints — hierarchical goal tracking with CRUD operations.

use super::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Goals endpoints
// ---------------------------------------------------------------------------

/// The well-known shared-memory key for goals storage.
const GOALS_KEY: &str = "__librefang_goals";

/// Shared agent ID for goals KV storage (same as schedules).
fn goals_shared_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// GET /api/goals — List all goals.
pub async fn list_goals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = goals_shared_agent_id();
    match state.kernel.memory.structured_get(agent_id, GOALS_KEY) {
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
    match state.kernel.memory.structured_get(agent_id, GOALS_KEY) {
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
    match state.kernel.memory.structured_get(agent_id, GOALS_KEY) {
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

    if title.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Title too long (max 256 chars)"})),
        );
    }

    let description = req["description"].as_str().unwrap_or("").to_string();
    if description.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Description too long (max 4096 chars)"})),
        );
    }

    let parent_id = req["parent_id"].as_str().map(|s| s.to_string());

    // If parent_id is specified, verify it exists
    if let Some(ref pid) = parent_id {
        let shared_id = goals_shared_agent_id();
        let parent_exists = match state.kernel.memory.structured_get(shared_id, GOALS_KEY) {
            Ok(Some(serde_json::Value::Array(ref arr))) => {
                arr.iter().any(|g| g["id"].as_str() == Some(pid))
            }
            _ => false,
        };
        if !parent_exists {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Parent goal '{}' not found", pid)})),
            );
        }
    }

    let status = req["status"].as_str().unwrap_or("pending").to_string();
    // Validate status
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

    let shared_id = goals_shared_agent_id();
    let mut goals: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, GOALS_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    goals.push(entry.clone());
    if let Err(e) =
        state
            .kernel
            .memory
            .structured_set(shared_id, GOALS_KEY, serde_json::Value::Array(goals))
    {
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
    let mut goals: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, GOALS_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let mut found = false;
    for g in goals.iter_mut() {
        if g["id"].as_str() == Some(&id) {
            found = true;
            if let Some(title) = req.get("title").and_then(|v| v.as_str()) {
                if title.is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Title must not be empty"})),
                    );
                }
                if title.len() > 256 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Title too long (max 256 chars)"})),
                    );
                }
                g["title"] = serde_json::Value::String(title.to_string());
            }
            if let Some(description) = req.get("description").and_then(|v| v.as_str()) {
                if description.len() > 4096 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Description too long (max 4096 chars)"})),
                    );
                }
                g["description"] = serde_json::Value::String(description.to_string());
            }
            if let Some(status) = req.get("status").and_then(|v| v.as_str()) {
                if !["pending", "in_progress", "completed", "cancelled"].contains(&status) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Invalid status"})),
                    );
                }
                g["status"] = serde_json::Value::String(status.to_string());
            }
            if let Some(progress) = req.get("progress").and_then(|v| v.as_u64()) {
                if progress > 100 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Progress must be 0-100"})),
                    );
                }
                g["progress"] = serde_json::json!(progress);
            }
            if let Some(parent_id) = req.get("parent_id") {
                if parent_id.is_null() {
                    g.as_object_mut().map(|obj| obj.remove("parent_id"));
                } else if let Some(pid) = parent_id.as_str() {
                    // Prevent self-reference
                    if pid == id {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({"error": "A goal cannot be its own parent"})),
                        );
                    }
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

    if let Err(e) =
        state
            .kernel
            .memory
            .structured_set(shared_id, GOALS_KEY, serde_json::Value::Array(goals))
    {
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

/// DELETE /api/goals/{id} — Delete a goal and optionally its children.
pub async fn delete_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = goals_shared_agent_id();
    let mut goals: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, GOALS_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let before = goals.len();

    // Collect all IDs to remove: the target goal + all descendants
    let mut ids_to_remove = vec![id.clone()];
    let mut i = 0;
    while i < ids_to_remove.len() {
        let current_id = ids_to_remove[i].clone();
        for g in &goals {
            if g["parent_id"].as_str() == Some(&current_id) {
                if let Some(child_id) = g["id"].as_str() {
                    if !ids_to_remove.contains(&child_id.to_string()) {
                        ids_to_remove.push(child_id.to_string());
                    }
                }
            }
        }
        i += 1;
    }

    goals.retain(|g| {
        g["id"]
            .as_str()
            .map(|gid| !ids_to_remove.contains(&gid.to_string()))
            .unwrap_or(true)
    });

    if goals.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Goal not found"})),
        );
    }

    let removed = before - goals.len();

    if let Err(e) =
        state
            .kernel
            .memory
            .structured_set(shared_id, GOALS_KEY, serde_json::Value::Array(goals))
    {
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
