//! Goals endpoints — hierarchical goal tracking with tenant-owned CRUD operations.

use super::AppState;

/// Build routes for the goal management domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/goals", axum::routing::get(list_goals).post(create_goal))
        .route("/goals/templates", axum::routing::get(list_goal_templates))
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
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use librefang_kernel::goals::GoalUpdate;
use librefang_types::goal::{Goal, GoalId, GoalStatus};
use std::sync::Arc;

use crate::middleware::AccountId;
use crate::types::ApiErrorResponse;

fn scoped_account_id(account: &AccountId) -> Result<&str, axum::response::Response> {
    account
        .0
        .as_deref()
        .ok_or_else(|| ApiErrorResponse::bad_request("X-Account-Id required").into_response())
}

fn parse_goal_id(id: &str) -> Result<GoalId, axum::response::Response> {
    id.parse::<GoalId>()
        .map_err(|_| ApiErrorResponse::not_found("Goal not found").into_response())
}

fn parse_goal_status(value: Option<&str>) -> Result<GoalStatus, axum::response::Response> {
    match value.unwrap_or("pending") {
        "pending" => Ok(GoalStatus::Pending),
        "in_progress" => Ok(GoalStatus::InProgress),
        "completed" => Ok(GoalStatus::Completed),
        "cancelled" => Ok(GoalStatus::Cancelled),
        _ => Err(ApiErrorResponse::bad_request(
            "Invalid status. Must be: pending, in_progress, completed, or cancelled",
        )
        .into_response()),
    }
}

fn parse_optional_goal_id(
    value: Option<&serde_json::Value>,
) -> Result<Option<Option<GoalId>>, axum::response::Response> {
    match value {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(Some(None)),
        Some(v) => v
            .as_str()
            .ok_or_else(|| {
                ApiErrorResponse::bad_request("parent_id must be a string or null").into_response()
            })?
            .parse::<GoalId>()
            .map(Some)
            .map(Some)
            .map_err(|_| ApiErrorResponse::bad_request("Invalid parent_id").into_response()),
    }
}

fn parse_optional_agent_id(
    value: Option<&serde_json::Value>,
) -> Result<Option<Option<librefang_types::agent::AgentId>>, axum::response::Response> {
    match value {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(Some(None)),
        Some(v) => v
            .as_str()
            .ok_or_else(|| {
                ApiErrorResponse::bad_request("agent_id must be a string or null").into_response()
            })?
            .parse::<librefang_types::agent::AgentId>()
            .map(Some)
            .map(Some)
            .map_err(|_| ApiErrorResponse::bad_request("Invalid agent_id").into_response()),
    }
}

/// GET /api/goals — List all goals visible to the tenant.
pub async fn list_goals(
    account: AccountId,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let goals = state.kernel.goal_store().list_by_account(account_id).await;
    Json(serde_json::json!({"goals": goals, "total": goals.len()})).into_response()
}

/// GET /api/goals/{id} — Get a tenant-owned goal by ID.
pub async fn get_goal(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let goal_id = match parse_goal_id(&id) {
        Ok(goal_id) => goal_id,
        Err(resp) => return resp,
    };
    match state
        .kernel
        .goal_store()
        .get_scoped(goal_id, account_id)
        .await
    {
        Some(goal) => Json(goal).into_response(),
        None => ApiErrorResponse::not_found("Goal not found").into_response(),
    }
}

/// GET /api/goals/{id}/children — Get all direct children of a tenant-owned goal.
pub async fn get_goal_children(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let goal_id = match parse_goal_id(&id) {
        Ok(goal_id) => goal_id,
        Err(resp) => return resp,
    };
    if state
        .kernel
        .goal_store()
        .get_scoped(goal_id, account_id)
        .await
        .is_none()
    {
        return ApiErrorResponse::not_found("Goal not found").into_response();
    }
    let children = state
        .kernel
        .goal_store()
        .children_scoped(goal_id, account_id)
        .await;
    Json(serde_json::json!({"children": children, "total": children.len()})).into_response()
}

/// POST /api/goals — Create a tenant-owned goal.
pub async fn create_goal(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let title = match req["title"].as_str() {
        Some(title) if !title.is_empty() => title.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'title' field").into_response()
        }
    };
    let description = req["description"].as_str().unwrap_or("").to_string();
    let status = match parse_goal_status(req["status"].as_str()) {
        Ok(status) => status,
        Err(resp) => return resp,
    };
    let progress = req["progress"].as_u64().unwrap_or(0) as u8;
    if progress > 100 {
        return ApiErrorResponse::bad_request("Progress must be 0-100").into_response();
    }
    let parent_id = match parse_optional_goal_id(req.get("parent_id")) {
        Ok(Some(parent_id)) => parent_id,
        Ok(None) => None,
        Err(resp) => return resp,
    };
    let agent_id = match parse_optional_agent_id(req.get("agent_id")) {
        Ok(Some(agent_id)) => agent_id,
        Ok(None) => None,
        Err(resp) => return resp,
    };

    let goal = Goal {
        id: GoalId::new(),
        title,
        description,
        parent_id,
        status,
        progress,
        agent_id,
        account_id: account_id.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    match state.kernel.goal_store().create(goal).await {
        Ok(goal) => (axum::http::StatusCode::CREATED, Json(goal)).into_response(),
        Err(e) if e.contains("Parent goal") => ApiErrorResponse::not_found(e).into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// PUT /api/goals/{id} — Update a tenant-owned goal.
pub async fn update_goal_by_id(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let goal_id = match parse_goal_id(&id) {
        Ok(goal_id) => goal_id,
        Err(resp) => return resp,
    };

    if let Some(title) = req.get("title").and_then(|value| value.as_str()) {
        if title.is_empty() {
            return ApiErrorResponse::bad_request("Title must not be empty").into_response();
        }
    }
    if let Some(progress) = req.get("progress").and_then(|value| value.as_u64()) {
        if progress > 100 {
            return ApiErrorResponse::bad_request("Progress must be 0-100").into_response();
        }
    }

    let status = match req.get("status") {
        Some(value) => match parse_goal_status(value.as_str()) {
            Ok(status) => Some(status),
            Err(resp) => return resp,
        },
        None => None,
    };
    let parent_id = match parse_optional_goal_id(req.get("parent_id")) {
        Ok(parent_id) => parent_id,
        Err(resp) => return resp,
    };
    let agent_id = match parse_optional_agent_id(req.get("agent_id")) {
        Ok(agent_id) => agent_id,
        Err(resp) => return resp,
    };

    let updates = GoalUpdate {
        title: req
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        description: req
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        status,
        progress: req
            .get("progress")
            .and_then(|value| value.as_u64())
            .map(|value| value as u8),
        parent_id,
        agent_id,
    };
    match state
        .kernel
        .goal_store()
        .update_scoped(goal_id, account_id, updates)
        .await
    {
        Ok(goal) => Json(goal).into_response(),
        Err(e) if e.contains("not found") || e.contains("Parent goal") => {
            ApiErrorResponse::not_found(e).into_response()
        }
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// DELETE /api/goals/{id} — Delete a tenant-owned goal and all descendants.
pub async fn delete_goal(
    account: AccountId,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = match scoped_account_id(&account) {
        Ok(account_id) => account_id,
        Err(resp) => return resp,
    };
    let goal_id = match parse_goal_id(&id) {
        Ok(goal_id) => goal_id,
        Err(resp) => return resp,
    };
    match state
        .kernel
        .goal_store()
        .delete_scoped(goal_id, account_id)
        .await
    {
        Ok(removed_count) => Json(serde_json::json!({
            "status": "removed",
            "goal_id": id,
            "removed_count": removed_count
        }))
        .into_response(),
        Err(_) => ApiErrorResponse::not_found("Goal not found").into_response(),
    }
}

/// GET /api/goals/templates — List built-in goal templates.
#[utoipa::path(
    get,
    path = "/api/goals/templates",
    tag = "goals",
    responses(
        (status = 200, description = "Goal templates", body = serde_json::Value)
    )
)]
pub async fn list_goal_templates(
    account: AccountId,
    State(_state): State<Arc<AppState>>,
) -> axum::response::Response {
    if let Err(resp) = scoped_account_id(&account) {
        return resp;
    }
    let templates = serde_json::json!([
        {
            "id": "product_launch",
            "name": "Product Launch",
            "icon": "rocket",
            "description": "Plan and execute a product launch from ideation to release.",
            "goals": [
                { "title": "Define Product Requirements", "description": "Gather stakeholder input and finalize the PRD.", "status": "pending" },
                { "title": "Design & Prototyping", "description": "Create wireframes, mockups, and interactive prototypes.", "status": "pending" },
                { "title": "Development Sprint", "description": "Implement core features and integrate APIs.", "status": "pending" },
                { "title": "QA & Testing", "description": "Run integration tests, load tests, and UAT.", "status": "pending" },
                { "title": "Launch & Monitor", "description": "Deploy to production, monitor metrics, and collect feedback.", "status": "pending" }
            ]
        }
    ]);
    Json(templates).into_response()
}
