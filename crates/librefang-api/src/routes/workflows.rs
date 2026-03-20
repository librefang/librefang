//! Workflow, trigger, schedule, and cron job handlers.

use super::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::triggers::{TriggerId, TriggerPattern};
use librefang_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowStep,
};
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::AgentId;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Workflow routes
// ---------------------------------------------------------------------------

/// POST /api/workflows — Register a new workflow.
#[utoipa::path(
    post,
    path = "/api/workflows",
    tag = "workflows",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Workflow created", body = serde_json::Value),
        (status = 400, description = "Invalid workflow definition")
    )
)]
pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = req["name"].as_str().unwrap_or("unnamed").to_string();
    let description = req["description"].as_str().unwrap_or("").to_string();

    let steps_json = match req["steps"].as_array() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'steps' array"})),
            );
        }
    };

    let mut steps = Vec::new();
    for s in steps_json {
        let step_name = s["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = s["agent_id"].as_str() {
            StepAgent::ById { id: id.to_string() }
        } else if let Some(name) = s["agent_name"].as_str() {
            StepAgent::ByName {
                name: name.to_string(),
            }
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("Step '{}' needs 'agent_id' or 'agent_name'", step_name)}),
                ),
            );
        };

        let mode = match s["mode"].as_str().unwrap_or("sequential") {
            "fan_out" => StepMode::FanOut,
            "collect" => StepMode::Collect,
            "conditional" => StepMode::Conditional {
                condition: s["condition"].as_str().unwrap_or("").to_string(),
            },
            "loop" => StepMode::Loop {
                max_iterations: s["max_iterations"].as_u64().unwrap_or(5) as u32,
                until: s["until"].as_str().unwrap_or("").to_string(),
            },
            _ => StepMode::Sequential,
        };

        let error_mode = match s["error_mode"].as_str().unwrap_or("fail") {
            "skip" => ErrorMode::Skip,
            "retry" => ErrorMode::Retry {
                max_retries: s["max_retries"].as_u64().unwrap_or(3) as u32,
            },
            _ => ErrorMode::Fail,
        };

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: s["output_var"].as_str().map(String::from),
        });
    }

    let layout = req.get("layout").cloned();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name,
        description,
        steps,
        created_at: chrono::Utc::now(),
        layout,
    };

    let id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"workflow_id": id.to_string()})),
    )
}

/// GET /api/workflows — List all workflows.
#[utoipa::path(
    get,
    path = "/api/workflows",
    tag = "workflows",
    responses(
        (status = 200, description = "List workflows", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_workflows(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let workflows = state.kernel.workflows.list_workflows().await;
    let list: Vec<serde_json::Value> = workflows
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id.to_string(),
                "name": w.name,
                "description": w.description,
                "steps": w.steps.len(),
                "created_at": w.created_at.to_rfc3339(),
            })
        })
        .collect();
    Json(list)
}

/// GET /api/workflows/:id — Get a single workflow by ID.
#[utoipa::path(
    get,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow details", body = serde_json::Value),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid workflow ID"})),
            );
        }
    });

    match state.kernel.workflows.get_workflow(workflow_id).await {
        Some(w) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": w.id.to_string(),
                "name": w.name,
                "description": w.description,
                "steps": w.steps.iter().map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "agent": match &s.agent {
                            StepAgent::ById { id } => serde_json::json!({"agent_id": id}),
                            StepAgent::ByName { name } => serde_json::json!({"agent_name": name}),
                        },
                        "prompt_template": s.prompt_template,
                        "mode": serde_json::to_value(&s.mode).unwrap_or_default(),
                        "timeout_secs": s.timeout_secs,
                        "error_mode": serde_json::to_value(&s.error_mode).unwrap_or_default(),
                        "output_var": s.output_var,
                    })
                }).collect::<Vec<_>>(),
                "created_at": w.created_at.to_rfc3339(),
                "layout": w.layout,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Workflow '{}' not found", id)})),
        ),
    }
}

/// PUT /api/workflows/:id — Update an existing workflow.
#[utoipa::path(
    put,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Workflow updated", body = serde_json::Value),
        (status = 400, description = "Invalid workflow definition"),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn update_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid workflow ID"})),
            );
        }
    });

    // Fetch existing workflow to preserve created_at
    let existing = match state.kernel.workflows.get_workflow(workflow_id).await {
        Some(w) => w,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Workflow not found"})),
            );
        }
    };

    let name = req["name"]
        .as_str()
        .map(String::from)
        .unwrap_or(existing.name.clone());
    let description = req["description"]
        .as_str()
        .map(String::from)
        .unwrap_or(existing.description.clone());

    // If steps are provided, parse them; otherwise keep existing steps
    let steps = if let Some(steps_json) = req["steps"].as_array() {
        let mut parsed_steps = Vec::new();
        for s in steps_json {
            let step_name = s["name"].as_str().unwrap_or("step").to_string();
            let agent = if let Some(aid) = s["agent_id"].as_str() {
                StepAgent::ById {
                    id: aid.to_string(),
                }
            } else if let Some(aname) = s["agent_name"].as_str() {
                StepAgent::ByName {
                    name: aname.to_string(),
                }
            } else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({"error": format!("Step '{}' needs 'agent_id' or 'agent_name'", step_name)}),
                    ),
                );
            };

            let mode = match s["mode"].as_str().unwrap_or("sequential") {
                "fan_out" => StepMode::FanOut,
                "collect" => StepMode::Collect,
                "conditional" => StepMode::Conditional {
                    condition: s["condition"].as_str().unwrap_or("").to_string(),
                },
                "loop" => StepMode::Loop {
                    max_iterations: s["max_iterations"].as_u64().unwrap_or(5) as u32,
                    until: s["until"].as_str().unwrap_or("").to_string(),
                },
                _ => StepMode::Sequential,
            };

            let error_mode = match s["error_mode"].as_str().unwrap_or("fail") {
                "skip" => ErrorMode::Skip,
                "retry" => ErrorMode::Retry {
                    max_retries: s["max_retries"].as_u64().unwrap_or(3) as u32,
                },
                _ => ErrorMode::Fail,
            };

            parsed_steps.push(WorkflowStep {
                name: step_name,
                agent,
                prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
                mode,
                timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
                error_mode,
                output_var: s["output_var"].as_str().map(String::from),
            });
        }
        parsed_steps
    } else {
        existing.steps.clone()
    };

    let layout = if req.get("layout").is_some() {
        req.get("layout").cloned()
    } else {
        existing.layout.clone()
    };

    let updated = Workflow {
        id: workflow_id,
        name,
        description,
        steps,
        created_at: existing.created_at,
        layout,
    };

    if !state
        .kernel
        .workflows
        .update_workflow(workflow_id, updated)
        .await
    {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Workflow not found"})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "workflow_id": id,
        })),
    )
}

/// DELETE /api/workflows/:id — Remove a workflow.
#[utoipa::path(
    delete,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow deleted"),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid workflow ID"})),
            );
        }
    });

    if state.kernel.workflows.remove_workflow(workflow_id).await {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Workflow not found"})),
        )
    }
}

/// POST /api/workflows/:id/run — Execute a workflow.
#[utoipa::path(post, path = "/api/workflows/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "Workflow run started", body = serde_json::Value)))]
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid workflow ID"})),
            );
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state.kernel.run_workflow(workflow_id, input).await {
        Ok((run_id, output)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                "output": output,
                "status": "completed",
            })),
        ),
        Err(e) => {
            tracing::warn!("Workflow run failed for {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workflow execution failed"})),
            )
        }
    }
}

/// GET /api/workflows/:id/runs — List runs for a workflow.
#[utoipa::path(get, path = "/api/workflows/{id}/runs", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "List workflow runs", body = Vec<serde_json::Value>)))]
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    let runs = state.kernel.workflows.list_runs(None).await;
    let list: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "workflow_name": r.workflow_name,
                "state": serde_json::to_value(&r.state).unwrap_or_default(),
                "steps_completed": r.step_results.len(),
                "started_at": r.started_at.to_rfc3339(),
                "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    Json(list)
}

// ---------------------------------------------------------------------------
// Trigger routes
// ---------------------------------------------------------------------------

/// POST /api/triggers — Register a new event trigger.
#[utoipa::path(
    post,
    path = "/api/triggers",
    tag = "workflows",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Trigger created", body = serde_json::Value),
        (status = 400, description = "Invalid trigger definition")
    )
)]
pub async fn create_trigger(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id_str = match req["agent_id"].as_str() {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'agent_id'"})),
            );
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent_id"})),
            );
        }
    };

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => match serde_json::from_value(p.clone()) {
            Ok(pat) => pat,
            Err(e) => {
                tracing::warn!("Invalid trigger pattern: {e}");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid trigger pattern"})),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'pattern'"})),
            );
        }
    };

    let prompt_template = req["prompt_template"]
        .as_str()
        .unwrap_or("Event: {{event}}")
        .to_string();
    let max_fires = req["max_fires"].as_u64().unwrap_or(0);

    match state
        .kernel
        .register_trigger(agent_id, pattern, prompt_template, max_fires)
    {
        Ok(trigger_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(e) => {
            tracing::warn!("Trigger registration failed: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({"error": "Trigger registration failed (agent not found?)"}),
                ),
            )
        }
    }
}

/// GET /api/triggers — List all triggers (optionally filter by ?agent_id=...).
#[utoipa::path(
    get,
    path = "/api/triggers",
    tag = "workflows",
    responses(
        (status = 200, description = "List triggers", body = serde_json::Value)
    )
)]
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    let triggers = state.kernel.list_triggers(agent_filter);
    let list: Vec<serde_json::Value> = triggers
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "agent_id": t.agent_id.to_string(),
                "pattern": serde_json::to_value(&t.pattern).unwrap_or_default(),
                "prompt_template": t.prompt_template,
                "enabled": t.enabled,
                "fire_count": t.fire_count,
                "max_fires": t.max_fires,
                "created_at": t.created_at.to_rfc3339(),
            })
        })
        .collect();
    let total = list.len();
    Json(serde_json::json!({"triggers": list, "total": total}))
}

/// DELETE /api/triggers/:id — Remove a trigger.
#[utoipa::path(delete, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), responses((status = 200, description = "Trigger deleted")))]
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid trigger ID"})),
            );
        }
    });

    if state.kernel.remove_trigger(trigger_id) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Trigger not found"})),
        )
    }
}

// ---------------------------------------------------------------------------
// Trigger update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/triggers/:id — Update a trigger (enable/disable toggle).
#[utoipa::path(put, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), request_body = serde_json::Value, responses((status = 200, description = "Trigger updated", body = serde_json::Value)))]
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid trigger ID"})),
            );
        }
    });

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        if state.kernel.set_trigger_enabled(trigger_id, enabled) {
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "updated", "trigger_id": id, "enabled": enabled}),
                ),
            )
        } else {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Trigger not found"})),
            )
        }
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'enabled' field"})),
        )
    }
}

// ---------------------------------------------------------------------------
// Scheduled Jobs (cron) endpoints
// ---------------------------------------------------------------------------

/// The well-known shared-memory agent ID used for cross-agent KV storage.
/// Must match the value in `librefang-kernel/src/kernel.rs::shared_memory_agent_id()`.
fn schedule_shared_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

const SCHEDULES_KEY: &str = "__librefang_schedules";

/// GET /api/schedules — List all cron-based scheduled jobs.
#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "workflows",
    responses(
        (status = 200, description = "List schedules", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = schedule_shared_agent_id();
    match state.kernel.memory.structured_get(agent_id, SCHEDULES_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => {
            let total = arr.len();
            Json(serde_json::json!({"schedules": arr, "total": total}))
        }
        Ok(_) => Json(serde_json::json!({"schedules": [], "total": 0})),
        Err(e) => {
            tracing::warn!("Failed to load schedules: {e}");
            Json(serde_json::json!({"schedules": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// GET /api/schedules/{id} — Get a specific schedule by ID.
#[utoipa::path(get, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule details", body = serde_json::Value)))]
pub async fn get_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = schedule_shared_agent_id();
    match state.kernel.memory.structured_get(agent_id, SCHEDULES_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => {
            if let Some(schedule) = arr.iter().find(|s| s["id"].as_str() == Some(&id)) {
                (StatusCode::OK, Json(schedule.clone()))
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Schedule '{}' not found", id)})),
                )
            }
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Schedule '{}' not found", id)})),
        ),
        Err(e) => {
            tracing::warn!("Failed to load schedules: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to load schedules: {e}")})),
            )
        }
    }
}

/// POST /api/schedules — Create a new cron-based scheduled job.
#[utoipa::path(
    post,
    path = "/api/schedules",
    tag = "workflows",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Schedule created", body = serde_json::Value),
        (status = 400, description = "Invalid schedule definition")
    )
)]
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match req["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'name' field"})),
            );
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'cron' field"})),
            );
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Invalid cron expression: must have 5 fields (min hour dom mon dow)"}),
            ),
        );
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    if agent_id_str.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: agent_id"})),
        );
    }
    // Validate agent exists (UUID or name lookup)
    let agent_exists = if let Ok(aid) = agent_id_str.parse::<AgentId>() {
        state.kernel.registry.get(aid).is_some()
    } else {
        state
            .kernel
            .registry
            .list()
            .iter()
            .any(|a| a.name == agent_id_str)
    };
    if !agent_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Agent not found: {agent_id_str}")})),
        );
    }
    let message = req["message"].as_str().unwrap_or("").to_string();
    let enabled = req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let schedule_id = uuid::Uuid::new_v4().to_string();
    let entry = serde_json::json!({
        "id": schedule_id,
        "name": name,
        "cron": cron,
        "agent_id": agent_id_str,
        "message": message,
        "enabled": enabled,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "last_run": null,
        "run_count": 0,
    });

    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    schedules.push(entry.clone());
    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        tracing::warn!("Failed to save schedule: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save schedule: {e}")})),
        );
    }

    (StatusCode::CREATED, Json(entry))
}

/// PUT /api/schedules/:id — Update a scheduled job (toggle enabled, edit fields).
#[utoipa::path(put, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), request_body = serde_json::Value, responses((status = 200, description = "Schedule updated", body = serde_json::Value)))]
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let mut found = false;
    for s in schedules.iter_mut() {
        if s["id"].as_str() == Some(&id) {
            found = true;
            if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
                s["enabled"] = serde_json::Value::Bool(enabled);
            }
            if let Some(name) = req.get("name").and_then(|v| v.as_str()) {
                s["name"] = serde_json::Value::String(name.to_string());
            }
            if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
                let cron_parts: Vec<&str> = cron.split_whitespace().collect();
                if cron_parts.len() != 5 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Invalid cron expression"})),
                    );
                }
                s["cron"] = serde_json::Value::String(cron.to_string());
            }
            if let Some(agent_id) = req.get("agent_id").and_then(|v| v.as_str()) {
                s["agent_id"] = serde_json::Value::String(agent_id.to_string());
            }
            if let Some(message) = req.get("message").and_then(|v| v.as_str()) {
                s["message"] = serde_json::Value::String(message.to_string());
            }
            break;
        }
    }

    if !found {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "updated", "schedule_id": id})),
    )
}

/// DELETE /api/schedules/:id — Remove a scheduled job.
#[utoipa::path(delete, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule deleted")))]
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let before = schedules.len();
    schedules.retain(|s| s["id"].as_str() != Some(&id));

    if schedules.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to delete schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "schedule_id": id})),
    )
}

/// POST /api/schedules/:id/run — Manually run a scheduled job now.
#[utoipa::path(post, path = "/api/schedules/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule triggered", body = serde_json::Value)))]
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let schedule = match schedules.iter().find(|s| s["id"].as_str() == Some(&id)) {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Schedule not found"})),
            );
        }
    };

    let agent_id_str = schedule["agent_id"].as_str().unwrap_or("");
    let message = schedule["message"]
        .as_str()
        .unwrap_or("Scheduled task triggered manually.");
    let name = schedule["name"].as_str().unwrap_or("(unnamed)");

    // Find the target agent — require explicit agent_id, no silent fallback
    let target_agent = if !agent_id_str.is_empty() {
        if let Ok(aid) = agent_id_str.parse::<AgentId>() {
            if state.kernel.registry.get(aid).is_some() {
                Some(aid)
            } else {
                None
            }
        } else {
            state
                .kernel
                .registry
                .list()
                .iter()
                .find(|a| a.name == agent_id_str)
                .map(|a| a.id)
        }
    } else {
        None
    };

    let target_agent = match target_agent {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({"error": "No target agent found. Specify an agent_id or start an agent first."}),
                ),
            );
        }
    };

    let run_message = if message.is_empty() {
        format!("[Scheduled task '{}' triggered manually]", name)
    } else {
        message.to_string()
    };

    // Update last_run and run_count
    let mut schedules_updated: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };
    for s in schedules_updated.iter_mut() {
        if s["id"].as_str() == Some(&id) {
            s["last_run"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            let count = s["run_count"].as_u64().unwrap_or(0);
            s["run_count"] = serde_json::json!(count + 1);
            break;
        }
    }
    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules_updated),
    ) {
        tracing::warn!("Failed to save structured data: {e}");
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(target_agent, &run_message, Some(kernel_handle))
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "schedule_id": id,
                "agent_id": target_agent.to_string(),
                "response": result.response,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "failed",
                "schedule_id": id,
                "error": format!("{e}"),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
#[utoipa::path(get, path = "/api/cron/jobs", tag = "workflows", responses((status = 200, description = "List cron jobs", body = Vec<serde_json::Value>)))]
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                state.kernel.cron_scheduler.list_jobs(aid)
            }
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid agent_id"})),
                );
            }
        }
    } else {
        state.kernel.cron_scheduler.list_all_jobs()
    };
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}

/// POST /api/cron/jobs — Create a new cron job.
#[utoipa::path(post, path = "/api/cron/jobs", tag = "workflows", request_body = serde_json::Value, responses((status = 200, description = "Cron job created", body = serde_json::Value)))]
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => {
            // cron_create returns a JSON string — parse it so the response
            // is a proper JSON object instead of a stringified blob.
            let parsed: serde_json::Value =
                serde_json::from_str(&result).unwrap_or(serde_json::json!({"id": result}));
            (StatusCode::CREATED, Json(parsed))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
#[utoipa::path(delete, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job deleted")))]
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.remove_job(job_id) {
                Ok(_) => {
                    if let Err(e) = state.kernel.cron_scheduler.persist() {
                        tracing::warn!("Failed to persist cron scheduler state: {e}");
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "deleted"})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// PUT /api/cron/jobs/{id} — Update a cron job's configuration.
#[utoipa::path(put, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = serde_json::Value, responses((status = 200, description = "Cron job updated", body = serde_json::Value)))]
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.update_job(job_id, &body) {
                Ok(job) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::to_value(&job).unwrap_or_default()),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
#[utoipa::path(put, path = "/api/cron/jobs/{id}/enable", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = serde_json::Value, responses((status = 200, description = "Cron job toggled", body = serde_json::Value)))]
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.set_enabled(job_id, enabled) {
                Ok(()) => {
                    if let Err(e) = state.kernel.cron_scheduler.persist() {
                        tracing::warn!("Failed to persist cron scheduler state: {e}");
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// GET /api/cron/jobs/{id} — Get a single cron job by ID.
#[utoipa::path(get, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job details", body = serde_json::Value), (status = 404, description = "Job not found")))]
pub async fn get_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Job not found"})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
#[utoipa::path(get, path = "/api/cron/jobs/{id}/status", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job status", body = serde_json::Value)))]
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Job not found"})),
                ),
            }
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        ),
    }
}
