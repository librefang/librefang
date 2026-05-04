//! Task-queue HTTP handlers.
//!
//! Extracted from `routes::system` (#3749). Public paths are unchanged:
//! `/tasks`, `/tasks/status`, `/tasks/list`, `/tasks/{id}`,
//! `/tasks/{id}/retry`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::kernel_handle::prelude::*;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::Arc;

/// Build routes for the task-queue domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
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
        .route("/tasks/{id}/retry", axum::routing::post(task_queue_retry))
        // Command-queue lane occupancy (#3749 11/N: moved from system.rs).
        .route("/queue/status", axum::routing::get(queue_status))
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
