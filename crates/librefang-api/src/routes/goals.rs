//! Goals endpoints — hierarchical goal tracking with CRUD operations.

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
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use std::collections::HashSet;
use std::sync::Arc;

use crate::types::ApiErrorResponse;
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
///
/// Goals are stored as a single JSON array in shared KV memory and returned
/// in one page — `offset=0` and `limit=None` always.
pub async fn list_goals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = goals_shared_agent_id();
    let items: Vec<serde_json::Value> = match state
        .kernel
        .memory_substrate()
        .structured_get(agent_id, GOALS_KEY)
    {
        Ok(Some(serde_json::Value::Array(arr))) => arr,
        Ok(_) => Vec::new(),
        Err(e) => {
            tracing::warn!("Failed to load goals: {e}");
            Vec::new()
        }
    };
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
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
                ApiErrorResponse::not_found(format!("Goal '{}' not found", id)).into_json_tuple()
            }
        }
        Ok(_) => ApiErrorResponse::not_found(format!("Goal '{}' not found", id)).into_json_tuple(),
        Err(e) => {
            tracing::warn!("Failed to load goals: {e}");
            ApiErrorResponse::internal(format!("Failed to load goals: {e}")).into_json_tuple()
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
            return ApiErrorResponse::bad_request("Missing or empty 'title' field")
                .into_json_tuple();
        }
    };

    if title.chars().count() > 256 {
        return ApiErrorResponse::bad_request("Title too long (max 256 chars)").into_json_tuple();
    }

    let description = req["description"].as_str().unwrap_or("").to_string();
    if description.chars().count() > 4096 {
        return ApiErrorResponse::bad_request("Description too long (max 4096 chars)")
            .into_json_tuple();
    }

    let parent_id = req["parent_id"].as_str().map(|s| s.to_string());

    let status = req["status"].as_str().unwrap_or("pending").to_string();
    if !["pending", "in_progress", "completed", "cancelled"].contains(&status.as_str()) {
        return ApiErrorResponse::bad_request(
            "Invalid status. Must be: pending, in_progress, completed, or cancelled",
        )
        .into_json_tuple();
    }

    let progress = req["progress"].as_u64().unwrap_or(0);
    if progress > 100 {
        return ApiErrorResponse::bad_request("Progress must be 0-100").into_json_tuple();
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

    // Atomic read-modify-write under BEGIN IMMEDIATE (#5138). Parent
    // validation, append, and persist all happen inside one transaction so
    // a concurrent create / update / delete cannot clobber this goal (the
    // last-writer-wins lost-update race the snapshot-then-set pattern had).
    let shared_id = goals_shared_agent_id();
    // Marker error so a missing parent maps to 404, not 500.
    const PARENT_MISSING: &str = "__goal_parent_missing__";
    let modify_result =
        state
            .kernel
            .memory_substrate()
            .structured_modify(shared_id, GOALS_KEY, |current| {
                let mut goals: Vec<serde_json::Value> = match current {
                    Some(serde_json::Value::Array(arr)) => arr,
                    _ => Vec::new(),
                };
                if let Some(ref pid) = parent_id {
                    let parent_exists =
                        goals.iter().any(|g| g["id"].as_str() == Some(pid.as_str()));
                    if !parent_exists {
                        return Err(librefang_types::error::LibreFangError::InvalidInput(
                            format!("{PARENT_MISSING}{pid}"),
                        ));
                    }
                }
                goals.push(entry.clone());
                Ok((serde_json::Value::Array(goals), ()))
            });

    if let Err(e) = modify_result {
        if let librefang_types::error::LibreFangError::InvalidInput(ref msg) = e {
            if let Some(pid) = msg.strip_prefix(PARENT_MISSING) {
                return ApiErrorResponse::not_found(format!("Parent goal '{}' not found", pid))
                    .into_json_tuple();
            }
        }
        tracing::warn!("Failed to save goal: {e}");
        return ApiErrorResponse::internal(format!("Failed to save goal: {e}")).into_json_tuple();
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

    // --- Stateless validation (no goals snapshot needed) ---

    if let Some(title) = req.get("title").and_then(|v| v.as_str()) {
        if title.is_empty() {
            return ApiErrorResponse::bad_request("Title must not be empty").into_json_tuple();
        }
        if title.chars().count() > 256 {
            return ApiErrorResponse::bad_request("Title too long (max 256 chars)")
                .into_json_tuple();
        }
    }

    if let Some(description) = req.get("description").and_then(|v| v.as_str()) {
        if description.chars().count() > 4096 {
            return ApiErrorResponse::bad_request("Description too long (max 4096 chars)")
                .into_json_tuple();
        }
    }

    if let Some(status) = req.get("status").and_then(|v| v.as_str()) {
        if !["pending", "in_progress", "completed", "cancelled"].contains(&status) {
            return ApiErrorResponse::bad_request("Invalid status").into_json_tuple();
        }
    }

    if let Some(progress) = req.get("progress").and_then(|v| v.as_u64()) {
        if progress > 100 {
            return ApiErrorResponse::bad_request("Progress must be 0-100").into_json_tuple();
        }
    }

    if let Some(parent_id) = req.get("parent_id") {
        if let Some(pid) = parent_id.as_str() {
            if pid == id {
                return ApiErrorResponse::bad_request("A goal cannot be its own parent")
                    .into_json_tuple();
            }
        }
    }

    // --- Atomic validate-then-mutate under BEGIN IMMEDIATE (#5138) ---
    //
    // Parent existence, cycle detection, the goal lookup, and the write all
    // run inside one transaction so a concurrent create / update / delete
    // can't slip a stale snapshot past validation or clobber this write
    // (the prior get→mutate→set was a lost-update race). Validation failures
    // are signalled via typed marker errors so they map back to the right
    // HTTP status without leaking a 500.
    const PARENT_MISSING: &str = "__goal_parent_missing__";
    const CIRCULAR: &str = "__goal_circular__";
    const NOT_FOUND: &str = "__goal_not_found__";
    use librefang_types::error::LibreFangError;

    let modify_result: Result<serde_json::Value, LibreFangError> = state
        .kernel
        .memory_substrate()
        .structured_modify(shared_id, GOALS_KEY, |current| {
            let mut goals: Vec<serde_json::Value> = match current {
                Some(serde_json::Value::Array(arr)) => arr,
                _ => Vec::new(),
            };

            // Parent existence + indirect-cycle detection on the live snapshot.
            if let Some(parent_id) = req.get("parent_id") {
                if !parent_id.is_null() {
                    if let Some(pid) = parent_id.as_str() {
                        if !goals.iter().any(|g| g["id"].as_str() == Some(pid)) {
                            return Err(LibreFangError::InvalidInput(format!(
                                "{PARENT_MISSING}{pid}"
                            )));
                        }
                        let mut ancestor = Some(pid.to_string());
                        let mut seen = HashSet::new();
                        seen.insert(id.clone());
                        while let Some(ref anc_id) = ancestor {
                            if !seen.insert(anc_id.clone()) {
                                break;
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
                                    return Err(LibreFangError::InvalidInput(CIRCULAR.to_string()));
                                }
                                Some(ap) => ancestor = Some(ap),
                                None => break,
                            }
                        }
                    }
                }
            }

            let mut updated: Option<serde_json::Value> = None;
            for g in goals.iter_mut() {
                if g["id"].as_str() == Some(&id) {
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
                    updated = Some(g.clone());
                    break;
                }
            }

            let Some(entity) = updated else {
                return Err(LibreFangError::InvalidInput(NOT_FOUND.to_string()));
            };

            Ok((serde_json::Value::Array(goals), entity))
        });

    let entity = match modify_result {
        Ok(e) => e,
        Err(LibreFangError::InvalidInput(ref msg)) if msg == NOT_FOUND => {
            return ApiErrorResponse::not_found("Goal not found").into_json_tuple();
        }
        Err(LibreFangError::InvalidInput(ref msg)) if msg == CIRCULAR => {
            return ApiErrorResponse::bad_request("Circular parent reference detected")
                .into_json_tuple();
        }
        Err(LibreFangError::InvalidInput(ref msg)) if msg.starts_with(PARENT_MISSING) => {
            let pid = msg.strip_prefix(PARENT_MISSING).unwrap_or_default();
            return ApiErrorResponse::not_found(format!("Parent goal '{}' not found", pid))
                .into_json_tuple();
        }
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to update goal: {e}"))
                .into_json_tuple();
        }
    };

    // Issue #3832: return the mutated entity so the dashboard can `setQueryData`
    // without an extra round-trip GET. Aligns with `create_goal`'s response shape.
    (StatusCode::OK, Json(entity))
}

/// DELETE /api/goals/{id} — Delete a goal and all its descendants.
pub async fn delete_goal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let shared_id = goals_shared_agent_id();
    // Atomic cascade-delete under BEGIN IMMEDIATE (#5138): collecting
    // descendants and the retain must see the same snapshot the write
    // commits, otherwise a concurrent create / update is lost.
    const NOT_FOUND: &str = "__goal_not_found__";
    use librefang_types::error::LibreFangError;

    let modify_result: Result<(), LibreFangError> = state
        .kernel
        .memory_substrate()
        .structured_modify(shared_id, GOALS_KEY, |current| {
            let mut goals: Vec<serde_json::Value> = match current {
                Some(serde_json::Value::Array(arr)) => arr,
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
                return Err(LibreFangError::InvalidInput(NOT_FOUND.to_string()));
            }

            Ok((serde_json::Value::Array(goals), ()))
        });

    if let Err(e) = modify_result {
        if let LibreFangError::InvalidInput(ref msg) = e {
            if msg == NOT_FOUND {
                return ApiErrorResponse::not_found("Goal not found")
                    .into_json_tuple()
                    .into_response();
            }
        }
        return ApiErrorResponse::internal(format!("Failed to delete goal: {e}"))
            .into_json_tuple()
            .into_response();
    }

    // Issue #3832: 204 No Content per RFC 9110 §15.3.5 — no body. The previous
    // `Json(null)` body violated the spec and tripped strict HTTP clients.
    StatusCode::NO_CONTENT.into_response()
}

/// GET /api/goals/templates — List built-in goal templates.
#[utoipa::path(
    get,
    path = "/api/goals/templates",
    tag = "goals",
    responses(
        (status = 200, description = "Goal templates", body = crate::types::JsonObject)
    )
)]
pub async fn list_goal_templates() -> impl IntoResponse {
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
        },
        {
            "id": "agent_deployment",
            "name": "Agent Deployment",
            "icon": "bot",
            "description": "Deploy and configure an autonomous agent from scratch.",
            "goals": [
                { "title": "Choose Model & Provider", "description": "Select the LLM provider and model for the agent.", "status": "pending" },
                { "title": "Configure Agent Manifest", "description": "Set system prompt, tools, and memory settings.", "status": "pending" },
                { "title": "Connect Channels", "description": "Wire up Slack, Discord, or other communication channels.", "status": "pending" },
                { "title": "Test Conversations", "description": "Run test dialogues and verify tool usage.", "status": "pending" },
                { "title": "Go Live", "description": "Enable the agent for end users and monitor performance.", "status": "pending" }
            ]
        },
        {
            "id": "security_audit",
            "name": "Security Audit",
            "icon": "shield",
            "description": "Conduct a security review of the system.",
            "goals": [
                { "title": "Dependency Scan", "description": "Audit all dependencies for known CVEs.", "status": "pending" },
                { "title": "API Security Review", "description": "Check authentication, authorization, and input validation.", "status": "pending" },
                { "title": "Secret Management", "description": "Verify no secrets are hardcoded or exposed.", "status": "pending" },
                { "title": "Penetration Testing", "description": "Run automated and manual penetration tests.", "status": "pending" },
                { "title": "Remediation Plan", "description": "Document findings and create fix timeline.", "status": "pending" }
            ]
        },
        {
            "id": "data_pipeline",
            "name": "Data Pipeline",
            "icon": "database",
            "description": "Build an end-to-end data processing pipeline.",
            "goals": [
                { "title": "Data Source Integration", "description": "Connect to data sources and define ingestion schedule.", "status": "pending" },
                { "title": "Transform & Clean", "description": "Build ETL jobs for data normalization.", "status": "pending" },
                { "title": "Storage & Indexing", "description": "Set up database schema and indexing strategy.", "status": "pending" },
                { "title": "Monitoring & Alerts", "description": "Add pipeline health checks and failure alerts.", "status": "pending" }
            ]
        },
        {
            "id": "team_onboarding",
            "name": "Team Onboarding",
            "icon": "users",
            "description": "Onboard a new team member step by step.",
            "goals": [
                { "title": "Access & Accounts", "description": "Set up email, VPN, Git, and internal tool access.", "status": "pending" },
                { "title": "Codebase Walkthrough", "description": "Review architecture, key modules, and coding conventions.", "status": "pending" },
                { "title": "First Task", "description": "Assign a starter task and pair with a mentor.", "status": "pending" },
                { "title": "First PR Merged", "description": "Complete code review cycle and merge first contribution.", "status": "pending" }
            ]
        },
        {
            "id": "incident_response",
            "name": "Incident Response",
            "icon": "alert",
            "description": "Handle a production incident from detection to postmortem.",
            "goals": [
                { "title": "Detect & Triage", "description": "Identify severity, assign incident commander.", "status": "pending" },
                { "title": "Investigate Root Cause", "description": "Analyze logs, traces, and metrics to find the cause.", "status": "pending" },
                { "title": "Mitigate", "description": "Apply hotfix or rollback to restore service.", "status": "pending" },
                { "title": "Communicate", "description": "Update stakeholders and post status page updates.", "status": "pending" },
                { "title": "Postmortem", "description": "Write incident report with timeline and action items.", "status": "pending" }
            ]
        }
    ]);

    Json(serde_json::json!({ "templates": templates }))
}
