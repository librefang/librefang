//! Workflow, trigger, schedule, and cron job handlers.

use super::shared::require_admin;
use super::AppState;
use crate::middleware::{AccountId, ConcreteAccountId};

/// Build routes for the workflow/trigger/schedule/cron domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        // Triggers
        .route(
            "/triggers",
            axum::routing::get(list_triggers).post(create_trigger),
        )
        .route(
            "/triggers/{id}",
            axum::routing::delete(delete_trigger).put(update_trigger),
        )
        // Schedules
        .route(
            "/schedules",
            axum::routing::get(list_schedules).post(create_schedule),
        )
        .route(
            "/schedules/{id}",
            axum::routing::get(get_schedule)
                .delete(delete_schedule)
                .put(update_schedule),
        )
        .route(
            "/schedules/{id}/run",
            axum::routing::post(run_schedule),
        )
        // Workflows
        .route(
            "/workflows",
            axum::routing::get(list_workflows).post(create_workflow),
        )
        .route(
            "/workflows/{id}",
            axum::routing::get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
        .route(
            "/workflows/{id}/run",
            axum::routing::post(run_workflow),
        )
        .route(
            "/workflows/{id}/dry-run",
            axum::routing::post(dry_run_workflow),
        )
        .route(
            "/workflows/{id}/runs",
            axum::routing::get(list_workflow_runs),
        )
        .route(
            "/workflows/runs/{run_id}",
            axum::routing::get(get_workflow_run),
        )
        // Workflow templates (distinct from the agent templates in system.rs)
        .route(
            "/workflow-templates",
            axum::routing::get(list_workflow_templates),
        )
        .route(
            "/workflow-templates/{id}",
            axum::routing::get(get_workflow_template),
        )
        .route(
            "/workflow-templates/{id}/instantiate",
            axum::routing::post(instantiate_template),
        )
        .route(
            "/workflows/{id}/save-as-template",
            axum::routing::post(save_workflow_as_template),
        )
        // Cron jobs
        .route(
            "/cron/jobs",
            axum::routing::get(list_cron_jobs).post(create_cron_job),
        )
        .route(
            "/cron/jobs/{id}",
            axum::routing::get(get_cron_job)
                .delete(delete_cron_job)
                .put(update_cron_job),
        )
        .route(
            "/cron/jobs/{id}/enable",
            axum::routing::put(toggle_cron_job),
        )
        .route(
            "/cron/jobs/{id}/status",
            axum::routing::get(cron_job_status),
        )
}
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::triggers::{TriggerId, TriggerPattern};
use librefang_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowRunId, WorkflowStep,
};
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::AgentId;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

use crate::types::ApiErrorResponse;

fn workflow_schedule_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}
// ---------------------------------------------------------------------------
// Helpers – parse StepMode / ErrorMode from both flat-string and nested-object
// formats so the frontend can send either:
//   "sequential"                                     (flat string)
//   {"conditional": {"condition": "..."}}            (serde-serialised enum)
// ---------------------------------------------------------------------------

/// Parse a `StepMode` from a JSON value.
///
/// Accepts:
/// - A plain string: `"sequential"`, `"fan_out"`, `"collect"`, `"conditional"`, `"loop"`
/// - A serde-serialised tagged object: `{"conditional": {"condition": "..."}}`
fn parse_step_mode(val: &serde_json::Value, step: &serde_json::Value) -> StepMode {
    // 1) Try flat string first
    if let Some(s) = val.as_str() {
        return match s {
            "fan_out" => StepMode::FanOut,
            "collect" => StepMode::Collect,
            "conditional" => {
                let condition = step["condition"]
                    .as_str()
                    .unwrap_or_else(|| {
                        warn!("conditional step missing 'condition' field, defaulting to empty");
                        ""
                    })
                    .to_string();
                StepMode::Conditional { condition }
            }
            "loop" => {
                let max_iterations = match step["max_iterations"].as_u64() {
                    Some(v) => u32::try_from(v).unwrap_or_else(|_| {
                        warn!(
                            "loop step max_iterations value {v} exceeds u32 range, defaulting to 5"
                        );
                        5
                    }),
                    None => {
                        warn!("loop step missing 'max_iterations' field, defaulting to 5");
                        5
                    }
                };
                let until = step["until"]
                    .as_str()
                    .unwrap_or_else(|| {
                        warn!("loop step missing 'until' field, defaulting to empty");
                        ""
                    })
                    .to_string();
                StepMode::Loop {
                    max_iterations,
                    until,
                }
            }
            _ => StepMode::Sequential,
        };
    }

    // 2) Try nested object (serde-serialised enum representation)
    if let Some(obj) = val.as_object() {
        if let Some(inner) = obj.get("conditional") {
            let condition = inner["condition"]
                .as_str()
                .unwrap_or_else(|| {
                    warn!("conditional step missing 'condition' field in nested object, defaulting to empty");
                    ""
                })
                .to_string();
            return StepMode::Conditional { condition };
        }
        if let Some(inner) = obj.get("loop") {
            let max_iterations = match inner["max_iterations"].as_u64() {
                Some(v) => u32::try_from(v).unwrap_or_else(|_| {
                    warn!("loop step max_iterations value {v} exceeds u32 range, defaulting to 5");
                    5
                }),
                None => {
                    warn!(
                        "loop step missing 'max_iterations' field in nested object, defaulting to 5"
                    );
                    5
                }
            };
            let until = inner["until"]
                .as_str()
                .unwrap_or_else(|| {
                    warn!("loop step missing 'until' field in nested object, defaulting to empty");
                    ""
                })
                .to_string();
            return StepMode::Loop {
                max_iterations,
                until,
            };
        }
        if obj.contains_key("fan_out") {
            return StepMode::FanOut;
        }
        if obj.contains_key("collect") {
            return StepMode::Collect;
        }
        if obj.contains_key("sequential") {
            return StepMode::Sequential;
        }
    }

    // 3) Fallback: try serde deserialization directly
    if let Ok(mode) = serde_json::from_value::<StepMode>(val.clone()) {
        return mode;
    }

    StepMode::Sequential
}

/// Parse an `ErrorMode` from a JSON value.
///
/// Accepts:
/// - A plain string: `"fail"`, `"skip"`, `"retry"`
/// - A serde-serialised tagged object: `{"retry": {"max_retries": 3}}`
fn parse_error_mode(val: &serde_json::Value, step: &serde_json::Value) -> ErrorMode {
    // 1) Try flat string first
    if let Some(s) = val.as_str() {
        return match s {
            "skip" => ErrorMode::Skip,
            "retry" => ErrorMode::Retry {
                max_retries: step["max_retries"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(3),
            },
            _ => ErrorMode::Fail,
        };
    }

    // 2) Try nested object
    if let Some(obj) = val.as_object() {
        if let Some(inner) = obj.get("retry") {
            return ErrorMode::Retry {
                max_retries: inner["max_retries"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(3),
            };
        }
        if obj.contains_key("skip") {
            return ErrorMode::Skip;
        }
        if obj.contains_key("fail") {
            return ErrorMode::Fail;
        }
    }

    // 3) Fallback: try serde deserialization directly
    if let Ok(mode) = serde_json::from_value::<ErrorMode>(val.clone()) {
        return mode;
    }

    ErrorMode::Fail
}

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
    account: ConcreteAccountId,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let name = req["name"].as_str().unwrap_or("unnamed").to_string();
    let description = req["description"].as_str().unwrap_or("").to_string();

    let steps_json = match req["steps"].as_array() {
        Some(s) => s,
        None => {
            return ApiErrorResponse::bad_request("Missing 'steps' array")
                .into_json_tuple()
                .into_response();
        }
    };

    let mut steps = Vec::new();
    for s in steps_json {
        let step_name = s["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = s["agent_id"].as_str() {
            let agent_id = match id.parse::<AgentId>() {
                Ok(agent_id) => agent_id,
                Err(_) => {
                    return ApiErrorResponse::bad_request(format!(
                        "Step '{}' has invalid agent_id",
                        step_name
                    ))
                    .into_json_tuple()
                    .into_response();
                }
            };
            match state.kernel.agent_registry().get(agent_id) {
                Some(entry) if entry.account_id.as_deref() == Some(account_id) => {
                    StepAgent::ById { id: id.to_string() }
                }
                _ => {
                    return ApiErrorResponse::not_found(format!("Agent not found: {id}"))
                        .into_json_tuple()
                        .into_response();
                }
            }
        } else if let Some(name) = s["agent_name"].as_str() {
            if state
                .kernel
                .agent_registry()
                .list_by_account(account_id)
                .iter()
                .any(|entry| entry.name == name)
            {
                StepAgent::ByName {
                    name: name.to_string(),
                }
            } else {
                return ApiErrorResponse::not_found(format!("Agent not found: {name}"))
                    .into_json_tuple()
                    .into_response();
            }
        } else {
            return ApiErrorResponse::bad_request(format!(
                "Step '{}' needs 'agent_id' or 'agent_name'",
                step_name
            ))
            .into_json_tuple()
            .into_response();
        };

        let mode = parse_step_mode(&s["mode"], s);
        let error_mode = parse_error_mode(&s["error_mode"], s);

        let depends_on: Vec<String> = s["depends_on"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: s["output_var"].as_str().map(String::from),
            inherit_context: s["inherit_context"].as_bool(),
            depends_on,
        });
    }

    let layout = req.get("layout").cloned();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name,
        description,
        steps,
        created_at: chrono::Utc::now(),
        account_id: Some(account_id.to_string()),
        layout,
    };

    let id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"workflow_id": id.to_string()})),
    )
        .into_response()
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
pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let engine = state.kernel.workflow_engine();
    let workflows = engine.list_workflows_by_account(account_id).await;
    let all_runs = engine.list_runs_by_account(account_id, None).await;

    // Count runs per workflow
    let mut run_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in &all_runs {
        *run_counts.entry(r.workflow_id.to_string()).or_default() += 1;
    }

    // Load cron jobs to find workflow-bound schedules
    let all_cron_jobs = state.kernel.cron().list_jobs_by_account(account_id);

    let list: Vec<serde_json::Value> = workflows
        .iter()
        .map(|w| {
            let wid = w.id.to_string();
            let schedule = all_cron_jobs.iter().find(|j| {
                matches!(&j.action, librefang_types::scheduler::CronAction::Workflow { workflow_id, .. } if workflow_id == &wid)
            });
            let schedule_json = schedule.map(|j| {
                let cron_expr = match &j.schedule {
                    librefang_types::scheduler::CronSchedule::Cron { expr, .. } => expr.clone(),
                    librefang_types::scheduler::CronSchedule::Every { every_secs } => format!("every {every_secs}s"),
                    librefang_types::scheduler::CronSchedule::At { at } => format!("at {}", at.to_rfc3339()),
                };
                serde_json::json!({
                    "cron": cron_expr,
                    "enabled": j.enabled,
                    "last_run": j.last_run.map(|t| t.to_rfc3339()),
                })
            });
            serde_json::json!({
                "id": wid,
                "name": w.name,
                "description": w.description,
                "steps": w.steps.len(),
                "run_count": run_counts.get(&wid).copied().unwrap_or(0),
                "created_at": w.created_at.to_rfc3339(),
                "schedule": schedule_json,
            })
        })
        .collect();
    Json(serde_json::json!({ "workflows": list })).into_response()
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
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    match state
        .kernel
        .workflow_engine()
        .get_workflow_scoped(workflow_id, account_id)
        .await
    {
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
                        "depends_on": s.depends_on,
                    })
                }).collect::<Vec<_>>(),
                "created_at": w.created_at.to_rfc3339(),
                "layout": w.layout,
            })),
        ),
        None => {
            ApiErrorResponse::not_found(format!("Workflow '{}' not found", id)).into_json_tuple()
        }
    }
    .into_response()
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
    account: ConcreteAccountId,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    // Fetch existing workflow to preserve created_at
    let existing = match state
        .kernel
        .workflow_engine()
        .get_workflow_scoped(workflow_id, account_id)
        .await
    {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found("Workflow not found")
                .into_json_tuple()
                .into_response();
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
                let agent_id = match aid.parse::<AgentId>() {
                    Ok(agent_id) => agent_id,
                    Err(_) => {
                        return ApiErrorResponse::bad_request(format!(
                            "Step '{}' has invalid agent_id",
                            step_name
                        ))
                        .into_json_tuple()
                        .into_response();
                    }
                };
                match state.kernel.agent_registry().get(agent_id) {
                    Some(entry) if entry.account_id.as_deref() == Some(account_id) => {
                        StepAgent::ById {
                            id: aid.to_string(),
                        }
                    }
                    _ => {
                        return ApiErrorResponse::not_found(format!("Agent not found: {aid}"))
                            .into_json_tuple()
                            .into_response();
                    }
                }
            } else if let Some(aname) = s["agent_name"].as_str() {
                if state
                    .kernel
                    .agent_registry()
                    .list_by_account(account_id)
                    .iter()
                    .any(|entry| entry.name == aname)
                {
                    StepAgent::ByName {
                        name: aname.to_string(),
                    }
                } else {
                    return ApiErrorResponse::not_found(format!("Agent not found: {aname}"))
                        .into_json_tuple()
                        .into_response();
                }
            } else {
                return ApiErrorResponse::bad_request(format!(
                    "Step '{}' needs 'agent_id' or 'agent_name'",
                    step_name
                ))
                .into_json_tuple()
                .into_response();
            };

            let mode = parse_step_mode(&s["mode"], s);
            let error_mode = parse_error_mode(&s["error_mode"], s);

            let depends_on: Vec<String> = s["depends_on"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            parsed_steps.push(WorkflowStep {
                name: step_name,
                agent,
                prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
                mode,
                timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
                error_mode,
                output_var: s["output_var"].as_str().map(String::from),
                inherit_context: s["inherit_context"].as_bool(),
                depends_on,
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
        account_id: Some(account_id.to_string()),
        layout,
    };

    if !state
        .kernel
        .workflow_engine()
        .update_workflow_scoped(workflow_id, account_id, updated)
        .await
    {
        return ApiErrorResponse::not_found("Workflow not found")
            .into_json_tuple()
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "workflow_id": id,
        })),
    )
        .into_response()
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
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    if state
        .kernel
        .workflow_engine()
        .remove_workflow_scoped(workflow_id, account_id)
        .await
    {
        let workflow_id_str = workflow_id.to_string();
        for job in state.kernel.cron().list_jobs_by_account(account_id) {
            if matches!(
                &job.action,
                librefang_types::scheduler::CronAction::Workflow { workflow_id, .. }
                if workflow_id == &workflow_id_str
            ) {
                let _ = state.kernel.cron().remove_job_scoped(job.id, account_id);
            }
        }
        let _ = state.kernel.cron().persist();
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        ApiErrorResponse::not_found("Workflow not found").into_json_tuple()
    }
    .into_response()
}

/// POST /api/workflows/:id/run — Execute a workflow.
#[utoipa::path(post, path = "/api/workflows/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "Workflow run started", body = serde_json::Value)))]
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state
        .kernel
        .run_workflow_scoped(workflow_id, input, Some(account_id))
        .await
    {
        Ok((run_id, output)) => {
            // Include step-level detail in the response so callers can inspect I/O
            let run = state
                .kernel
                .workflow_engine()
                .get_run_scoped(run_id, account_id)
                .await;
            let step_results = run.as_ref().map(|r| {
                r.step_results
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "step_name": s.step_name,
                            "agent_name": s.agent_name,
                            "prompt": s.prompt,
                            "output": s.output,
                            "input_tokens": s.input_tokens,
                            "output_tokens": s.output_tokens,
                            "duration_ms": s.duration_ms,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "run_id": run_id.to_string(),
                    "output": output,
                    "status": "completed",
                    "step_results": step_results.unwrap_or_default(),
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Workflow run failed for {id}: {e}");
            // Return the actual error message, not a generic one, to aid debugging
            let detail = e.to_string();
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "workflow_failed",
                    "detail": detail,
                })),
            )
        }
    }
    .into_response()
}

/// POST /api/workflows/:id/dry-run — Validate and preview a workflow without executing it.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/dry-run",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Dry-run preview", body = serde_json::Value),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn dry_run_workflow(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state
        .kernel
        .dry_run_workflow_scoped(workflow_id, input, Some(account_id))
        .await
    {
        Ok(steps) => {
            let all_agents_found = steps.iter().all(|s| s.agent_found);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "valid": all_agents_found,
                    "steps": steps.iter().map(|s| serde_json::json!({
                        "step_name": s.step_name,
                        "agent_name": s.agent_name,
                        "agent_found": s.agent_found,
                        "resolved_prompt": s.resolved_prompt,
                        "skipped": s.skipped,
                        "skip_reason": s.skip_reason,
                    })).collect::<Vec<_>>(),
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Workflow dry-run failed for {id}: {e}");
            ApiErrorResponse::not_found("Workflow not found").into_json_tuple()
        }
    }
    .into_response()
}

/// GET /api/workflows/runs/:run_id — Get detailed info for a single workflow run.
#[utoipa::path(
    get,
    path = "/api/workflows/runs/{run_id}",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Workflow run details", body = serde_json::Value),
        (status = 404, description = "Run not found")
    )
)]
pub async fn get_workflow_run(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(run_id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID")
                .into_json_tuple()
                .into_response();
        }
    });

    match state
        .kernel
        .workflow_engine()
        .get_run_scoped(run_id, account_id)
        .await
    {
        Some(run) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": run.id.to_string(),
                "workflow_id": run.workflow_id.to_string(),
                "workflow_name": run.workflow_name,
                "account_id": run.account_id,
                "input": run.input,
                "state": serde_json::to_value(&run.state).unwrap_or_default(),
                "output": run.output,
                "error": run.error,
                "started_at": run.started_at.to_rfc3339(),
                "completed_at": run.completed_at.map(|t| t.to_rfc3339()),
                "step_results": run.step_results.iter().map(|s| serde_json::json!({
                    "step_name": s.step_name,
                    "agent_id": s.agent_id,
                    "agent_name": s.agent_name,
                    "prompt": s.prompt,
                    "output": s.output,
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "duration_ms": s.duration_ms,
                })).collect::<Vec<_>>(),
            })),
        ),
        None => ApiErrorResponse::not_found(format!("Run '{run_id}' not found")).into_json_tuple(),
    }
    .into_response()
}

/// GET /api/workflows/:id/runs — List runs for a workflow.
#[utoipa::path(get, path = "/api/workflows/{id}/runs", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "List workflow runs", body = Vec<serde_json::Value>)))]
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });
    if state
        .kernel
        .workflow_engine()
        .get_workflow_scoped(workflow_id, account_id)
        .await
        .is_none()
    {
        return ApiErrorResponse::not_found(format!("Workflow '{}' not found", id))
            .into_json_tuple()
            .into_response();
    }
    let runs = state
        .kernel
        .workflow_engine()
        .list_runs_by_account(account_id, None)
        .await;
    let list: Vec<serde_json::Value> = runs
        .iter()
        .filter(|run| run.workflow_id == workflow_id)
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "workflow_id": r.workflow_id.to_string(),
                "workflow_name": r.workflow_name,
                "state": serde_json::to_value(&r.state).unwrap_or_default(),
                "steps_completed": r.step_results.len(),
                "started_at": r.started_at.to_rfc3339(),
                "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    Json(list).into_response()
}

// ---------------------------------------------------------------------------
// Save workflow as reusable template
// ---------------------------------------------------------------------------

/// POST /api/workflows/:id/save-as-template — Convert a workflow into a reusable template.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/save-as-template",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Template created", body = serde_json::Value),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn save_workflow_as_template(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    use librefang_kernel::workflow::WorkflowEngine;

    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID")
                .into_json_tuple()
                .into_response();
        }
    });

    let workflow = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found(format!("Workflow '{}' not found", id))
                .into_json_tuple()
                .into_response();
        }
    };

    let template = WorkflowEngine::workflow_to_template(&workflow);

    // Persist template to TOML file under the active kernel home directory.
    let templates_dir = state.kernel.home_dir().join("workflows").join("templates");
    if let Err(e) = std::fs::create_dir_all(&templates_dir) {
        warn!("Failed to create templates directory: {e}");
    } else {
        let toml_path = templates_dir.join(format!("{}.toml", &template.id));
        match toml::to_string_pretty(&template) {
            Ok(toml_str) => {
                if let Err(e) = std::fs::write(&toml_path, toml_str) {
                    warn!(
                        path = %toml_path.display(),
                        "Failed to write template file: {e}"
                    );
                }
            }
            Err(e) => {
                warn!("Failed to serialize template to TOML: {e}");
            }
        }
    }

    // Register in the in-memory template registry
    state.kernel.templates().register(template.clone()).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "template": template,
        })),
    )
        .into_response()
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
    account: ConcreteAccountId,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let agent_id_str = match req["agent_id"].as_str() {
        Some(id) => id,
        None => {
            return ApiErrorResponse::bad_request("Missing 'agent_id'")
                .into_json_tuple()
                .into_response();
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid agent_id")
                .into_json_tuple()
                .into_response();
        }
    };
    match state.kernel.agent_registry().get(agent_id) {
        Some(entry) if entry.account_id.as_deref() == Some(account_id) => {}
        _ => {
            return ApiErrorResponse::not_found("Agent not found")
                .into_json_tuple()
                .into_response();
        }
    }

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => match serde_json::from_value(p.clone()) {
            Ok(pat) => pat,
            Err(e) => {
                tracing::warn!("Invalid trigger pattern: {e}");
                return ApiErrorResponse::bad_request("Invalid trigger pattern")
                    .into_json_tuple()
                    .into_response();
            }
        },
        None => {
            return ApiErrorResponse::bad_request("Missing 'pattern'")
                .into_json_tuple()
                .into_response();
        }
    };

    let prompt_template = req["prompt_template"]
        .as_str()
        .unwrap_or("Event: {{event}}")
        .to_string();
    let max_fires = req["max_fires"].as_u64().unwrap_or(0);

    // Optional cross-session target: route triggered message to a different agent.
    // If the caller supplied a value but it is malformed, reject explicitly —
    // otherwise the trigger would silently register without any target and the
    // caller would assume the routing was accepted.
    let target_agent: Option<AgentId> = match req.get("target_agent_id").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => match s.parse() {
            Ok(id) => Some(id),
            Err(_) => {
                return ApiErrorResponse::bad_request(format!(
                    "Invalid 'target_agent_id': '{s}' is not a valid UUID"
                ))
                .into_json_tuple()
                .into_response();
            }
        },
    };
    if let Some(target_agent) = target_agent {
        match state.kernel.agent_registry().get(target_agent) {
            Some(entry) if entry.account_id.as_deref() == Some(account_id) => {}
            _ => {
                return ApiErrorResponse::not_found("Target agent not found")
                    .into_json_tuple()
                    .into_response();
            }
        }
    }

    match state.kernel.register_trigger_with_target(
        Some(account_id.to_string()),
        agent_id,
        pattern,
        prompt_template,
        max_fires,
        target_agent,
    ) {
        Ok(trigger_id) => {
            let mut resp = serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            });
            if let Some(target) = target_agent {
                resp["target_agent_id"] = serde_json::json!(target.to_string());
            }
            (StatusCode::CREATED, Json(resp))
        }
        Err(e) => {
            tracing::warn!("Trigger registration failed: {e}");
            ApiErrorResponse::not_found("Trigger registration failed (agent not found?)")
                .into_json_tuple()
        }
    }
    .into_response()
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
    account: ConcreteAccountId,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    if let Some(agent_id) = agent_filter {
        match state.kernel.agent_registry().get(agent_id) {
            Some(entry) if entry.account_id.as_deref() == Some(account_id) => {}
            _ => {
                return ApiErrorResponse::not_found("Agent not found")
                    .into_json_tuple()
                    .into_response();
            }
        }
    }

    let triggers = state.kernel.list_triggers_scoped(account_id, agent_filter);
    let list: Vec<serde_json::Value> = triggers
        .iter()
        .map(|t| {
            let mut v = serde_json::json!({
                "id": t.id.to_string(),
                "agent_id": t.agent_id.to_string(),
                "pattern": serde_json::to_value(&t.pattern).unwrap_or_default(),
                "prompt_template": t.prompt_template,
                "enabled": t.enabled,
                "fire_count": t.fire_count,
                "max_fires": t.max_fires,
                "created_at": t.created_at.to_rfc3339(),
            });
            if let Some(target) = &t.target_agent {
                v["target_agent_id"] = serde_json::json!(target.to_string());
            }
            v
        })
        .collect();
    let total = list.len();
    Json(serde_json::json!({"triggers": list, "total": total})).into_response()
}

/// DELETE /api/triggers/:id — Remove a trigger.
#[utoipa::path(delete, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), responses((status = 200, description = "Trigger deleted")))]
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid trigger ID")
                .into_json_tuple()
                .into_response();
        }
    });

    if state
        .kernel
        .trigger_engine()
        .remove_scoped(trigger_id, account_id)
    {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        ApiErrorResponse::not_found("Trigger not found").into_json_tuple()
    }
    .into_response()
}

// ---------------------------------------------------------------------------
// Trigger update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/triggers/:id — Update a trigger (enable/disable toggle).
#[utoipa::path(put, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), request_body = serde_json::Value, responses((status = 200, description = "Trigger updated", body = serde_json::Value)))]
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid trigger ID")
                .into_json_tuple()
                .into_response();
        }
    });

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        if state
            .kernel
            .trigger_engine()
            .set_enabled_scoped(trigger_id, account_id, enabled)
        {
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "updated", "trigger_id": id, "enabled": enabled}),
                ),
            )
        } else {
            ApiErrorResponse::not_found("Trigger not found").into_json_tuple()
        }
    } else {
        ApiErrorResponse::bad_request("Missing 'enabled' field").into_json_tuple()
    }
    .into_response()
}

// ---------------------------------------------------------------------------
// Scheduled Jobs (cron) endpoints
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Schedule endpoints — backed by CronScheduler (unified with cron_* system)
// ---------------------------------------------------------------------------
// Previously these read/wrote a separate `__librefang_schedules` JSON blob in
// shared memory, which had no execution engine. Now they delegate to the real
// CronScheduler so scheduled jobs actually fire via the kernel tick loop (#2024).

/// Helper: parse a CronJobId from a string, returning an API error on failure.
fn parse_cron_job_id(
    id: &str,
) -> Result<librefang_types::scheduler::CronJobId, (StatusCode, Json<serde_json::Value>)> {
    id.parse::<librefang_types::scheduler::CronJobId>()
        .map_err(|_| {
            ApiErrorResponse::bad_request(format!("Invalid schedule ID: {id}")).into_json_tuple()
        })
}

/// Helper: serialize a CronJob to the JSON shape the dashboard expects.
fn cron_job_to_schedule_json(job: &librefang_types::scheduler::CronJob) -> serde_json::Value {
    let cron_expr = match &job.schedule {
        librefang_types::scheduler::CronSchedule::Cron { expr, .. } => expr.clone(),
        librefang_types::scheduler::CronSchedule::Every { every_secs } => {
            format!("every {every_secs}s")
        }
        librefang_types::scheduler::CronSchedule::At { at } => format!("at {}", at.to_rfc3339()),
    };
    let message = match &job.action {
        librefang_types::scheduler::CronAction::AgentTurn { message, .. } => message.clone(),
        librefang_types::scheduler::CronAction::Workflow {
            workflow_id, input, ..
        } => input
            .clone()
            .unwrap_or_else(|| format!("workflow:{workflow_id}")),
        librefang_types::scheduler::CronAction::SystemEvent { text } => text.clone(),
    };
    let workflow_id = match &job.action {
        librefang_types::scheduler::CronAction::Workflow { workflow_id, .. } => workflow_id.clone(),
        _ => String::new(),
    };
    serde_json::json!({
        "id": job.id.to_string(),
        "name": job.name,
        "cron": cron_expr,
        "agent_id": job.agent_id.to_string(),
        "account_id": job.account_id,
        "workflow_id": workflow_id,
        "message": message,
        "enabled": job.enabled,
        "created_at": job.created_at.to_rfc3339(),
        "last_run": job.last_run.map(|t| t.to_rfc3339()),
        "next_run": job.next_run.map(|t| t.to_rfc3339()),
    })
}

/// GET /api/schedules — List all scheduled jobs.
#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "workflows",
    responses(
        (status = 200, description = "List schedules", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_schedules(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let jobs = state.kernel.cron().list_jobs_by_account(account_id);
    let schedules: Vec<serde_json::Value> = jobs.iter().map(cron_job_to_schedule_json).collect();
    let total = schedules.len();
    Json(serde_json::json!({"schedules": schedules, "total": total})).into_response()
}

/// GET /api/schedules/{id} — Get a specific schedule by ID.
#[utoipa::path(get, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule details", body = serde_json::Value)))]
pub async fn get_schedule(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e.into_response(),
    };
    match state.kernel.cron().get_job_scoped(job_id, account_id) {
        Some(job) => (StatusCode::OK, Json(cron_job_to_schedule_json(&job))),
        None => ApiErrorResponse::not_found(format!("Schedule '{id}' not found")).into_json_tuple(),
    }
    .into_response()
}

/// POST /api/schedules — Create a new scheduled job (backed by CronScheduler).
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
    account: ConcreteAccountId,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let name = match req["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'name' field")
                .into_json_tuple()
                .into_response();
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'cron' field")
                .into_json_tuple()
                .into_response();
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return ApiErrorResponse::bad_request(
            "Invalid cron expression: must have 5 fields (min hour dom mon dow)",
        )
        .into_json_tuple()
        .into_response();
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    let workflow_id_str = req["workflow_id"].as_str().unwrap_or("").to_string();

    // Must have either agent_id or workflow_id
    if agent_id_str.is_empty() && workflow_id_str.is_empty() {
        return ApiErrorResponse::bad_request("Must provide either agent_id or workflow_id")
            .into_json_tuple()
            .into_response();
    }

    // Resolve agent_id to a UUID (tenant-scoped)
    let resolved_agent_id = if !agent_id_str.is_empty() {
        if let Ok(aid) = agent_id_str.parse::<AgentId>() {
            match state.kernel.agent_registry().get(aid) {
                Some(entry) => {
                    if entry.account_id.as_deref() != Some(account_id) {
                        return ApiErrorResponse::not_found(format!(
                            "Agent not found: {agent_id_str}"
                        ))
                        .into_json_tuple()
                        .into_response();
                    }
                    aid
                }
                None => {
                    return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                        .into_json_tuple()
                        .into_response();
                }
            }
        } else {
            let agent_list = state.kernel.agent_registry().list_by_account(account_id);
            if let Some(agent) = agent_list.iter().find(|a| a.name == agent_id_str) {
                agent.id
            } else {
                return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                    .into_json_tuple()
                    .into_response();
            }
        }
    } else {
        workflow_schedule_agent_id()
    };

    // Validate workflow exists if provided
    if !workflow_id_str.is_empty() {
        if let Ok(wid) = workflow_id_str.parse::<uuid::Uuid>() {
            if state
                .kernel
                .workflow_engine()
                .get_workflow_scoped(WorkflowId(wid), account_id)
                .await
                .is_none()
            {
                return ApiErrorResponse::not_found(format!(
                    "Workflow not found: {workflow_id_str}"
                ))
                .into_json_tuple()
                .into_response();
            }
        } else {
            return ApiErrorResponse::bad_request("Invalid workflow_id format")
                .into_json_tuple()
                .into_response();
        }
    }

    let message = req["message"].as_str().unwrap_or("").to_string();

    // Build the CronJob action
    let action = if !workflow_id_str.is_empty() {
        librefang_types::scheduler::CronAction::Workflow {
            workflow_id: workflow_id_str,
            input: if message.is_empty() {
                None
            } else {
                Some(message)
            },
            timeout_secs: None,
        }
    } else {
        let msg = if message.is_empty() {
            format!("[Scheduled task '{}' triggered]", name)
        } else {
            message
        };
        librefang_types::scheduler::CronAction::AgentTurn {
            message: msg,
            model_override: None,
            timeout_secs: None,
        }
    };

    let job = librefang_types::scheduler::CronJob {
        id: librefang_types::scheduler::CronJobId::new(),
        agent_id: resolved_agent_id,
        account_id: Some(account_id.to_string()),
        name,
        enabled: req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        schedule: librefang_types::scheduler::CronSchedule::Cron {
            expr: cron,
            tz: None,
        },
        action,
        delivery: librefang_types::scheduler::CronDelivery::None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    match state.kernel.cron().add_job(job.clone(), false) {
        Ok(job_id) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            let mut entry = cron_job_to_schedule_json(&job);
            entry["id"] = serde_json::Value::String(job_id.to_string());
            (StatusCode::CREATED, Json(entry)).into_response()
        }
        Err(e) => {
            tracing::warn!("Failed to create schedule: {e}");
            ApiErrorResponse::internal("Failed to create schedule")
                .with_code("schedule_create_failed")
                .into_json_tuple()
                .into_response()
        }
    }
}

/// PUT /api/schedules/:id — Update a scheduled job (toggle enabled, edit fields).
#[utoipa::path(put, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), request_body = serde_json::Value, responses((status = 200, description = "Schedule updated", body = serde_json::Value)))]
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e.into_response(),
    };

    // Build update payload compatible with CronScheduler::update_job
    let mut updates = serde_json::Map::new();
    if let Some(enabled) = req.get("enabled") {
        updates.insert("enabled".to_string(), enabled.clone());
    }
    if let Some(name) = req.get("name") {
        updates.insert("name".to_string(), name.clone());
    }
    if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
        let cron_parts: Vec<&str> = cron.split_whitespace().collect();
        if cron_parts.len() != 5 {
            return ApiErrorResponse::bad_request("Invalid cron expression")
                .into_json_tuple()
                .into_response();
        }
        updates.insert(
            "schedule".to_string(),
            serde_json::json!({"kind": "cron", "expr": cron}),
        );
    }
    if let Some(agent_id) = req.get("agent_id").and_then(|value| value.as_str()) {
        let parsed_agent_id = match agent_id.parse::<AgentId>() {
            Ok(agent_id) => agent_id,
            Err(_) => {
                return ApiErrorResponse::bad_request("Invalid agent_id")
                    .into_json_tuple()
                    .into_response();
            }
        };
        match state.kernel.agent_registry().get(parsed_agent_id) {
            Some(entry) if entry.account_id.as_deref() == Some(account_id) => {
                updates.insert("agent_id".to_string(), serde_json::json!(agent_id));
            }
            _ => {
                return ApiErrorResponse::not_found("Agent not found")
                    .into_json_tuple()
                    .into_response();
            }
        }
    }

    match state.kernel.cron().update_job_scoped(
        job_id,
        account_id,
        &serde_json::Value::Object(updates),
    ) {
        Ok(_job) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "updated", "schedule_id": id})),
            )
        }
        Err(e) => {
            tracing::warn!("Failed to update schedule {id}: {e}");
            ApiErrorResponse::not_found("Schedule not found").into_json_tuple()
        }
    }
    .into_response()
}

/// DELETE /api/schedules/:id — Remove a scheduled job.
#[utoipa::path(delete, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule deleted")))]
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e.into_response(),
    };

    match state.kernel.cron().remove_job_scoped(job_id, account_id) {
        Ok(_) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "removed", "schedule_id": id})),
            )
        }
        Err(e) => {
            tracing::warn!("Failed to delete schedule {id}: {e}");
            ApiErrorResponse::not_found("Schedule not found").into_json_tuple()
        }
    }
    .into_response()
}

/// POST /api/schedules/:id/run — Manually trigger a scheduled job now.
#[utoipa::path(post, path = "/api/schedules/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule triggered", body = serde_json::Value)))]
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    account: ConcreteAccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    let account_id = account.0.as_str();
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e.into_response(),
    };

    let job = match state.kernel.cron().get_job_scoped(job_id, account_id) {
        Some(j) => j,
        None => {
            return ApiErrorResponse::not_found("Schedule not found")
                .into_json_tuple()
                .into_response();
        }
    };

    let name = job.name.clone();
    let agent_id = job.agent_id;

    match &job.action {
        librefang_types::scheduler::CronAction::Workflow {
            workflow_id, input, ..
        } => {
            let wid = match workflow_id.parse::<uuid::Uuid>() {
                Ok(u) => WorkflowId(u),
                Err(_) => {
                    return ApiErrorResponse::bad_request("Invalid workflow_id")
                        .into_json_tuple()
                        .into_response();
                }
            };
            let wf_input = input
                .clone()
                .unwrap_or_else(|| format!("[Scheduled workflow '{}' triggered]", name));
            match state
                .kernel
                .run_workflow_scoped(wid, wf_input, job.account_id.as_deref())
                .await
            {
                Ok((run_id, output)) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "completed",
                        "schedule_id": id,
                        "workflow_id": workflow_id,
                        "run_id": run_id.to_string(),
                        "output": output,
                    })),
                ),
                Err(_e) => ApiErrorResponse::internal("Workflow execution failed")
                    .with_code("schedule_run_failed")
                    .into_json_tuple(),
            }
        }
        librefang_types::scheduler::CronAction::AgentTurn { message, .. } => {
            let kernel_handle: Arc<dyn KernelHandle> =
                state.kernel.clone() as Arc<dyn KernelHandle>;
            match state
                .kernel
                .send_message_with_handle(agent_id, message, Some(kernel_handle))
                .await
            {
                Ok(result) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "completed",
                        "schedule_id": id,
                        "agent_id": agent_id.to_string(),
                        "response": result.response,
                    })),
                ),
                Err(_e) => ApiErrorResponse::internal("Scheduled agent execution failed")
                    .with_code("schedule_run_failed")
                    .into_json_tuple(),
            }
        }
        librefang_types::scheduler::CronAction::SystemEvent { text } => {
            // Fire-and-forget system event
            let event = librefang_types::event::Event::new(
                AgentId::new(),
                librefang_types::event::EventTarget::Broadcast,
                librefang_types::event::EventPayload::Custom(text.as_bytes().to_vec()),
            )
            .with_account_id(job.account_id.clone());
            state.kernel.publish_event(event).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "schedule_id": id,
                    "type": "system_event",
                })),
            )
        }
    }
    .into_response()
}

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
#[utoipa::path(get, path = "/api/cron/jobs", tag = "workflows", responses((status = 200, description = "List cron jobs", body = Vec<serde_json::Value>)))]
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                state.kernel.cron().list_jobs(aid)
            }
            Err(_) => {
                return ApiErrorResponse::bad_request("Invalid agent_id")
                    .into_json_tuple()
                    .into_response();
            }
        }
    } else {
        state.kernel.cron().list_all_jobs()
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
        .into_response()
}

/// POST /api/cron/jobs — Create a new cron job.
#[utoipa::path(post, path = "/api/cron/jobs", tag = "workflows", request_body = serde_json::Value, responses((status = 200, description = "Cron job created", body = serde_json::Value)))]
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => {
            // cron_create returns a JSON string — parse it so the response
            // is a proper JSON object instead of a stringified blob.
            let parsed: serde_json::Value =
                serde_json::from_str(&result).unwrap_or(serde_json::json!({"id": result}));
            (StatusCode::CREATED, Json(parsed))
        }
        Err(e) => ApiErrorResponse::bad_request(e).into_json_tuple(),
    }
    .into_response()
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
#[utoipa::path(delete, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job deleted")))]
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().remove_job(job_id) {
                Ok(_) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::warn!("Failed to persist cron scheduler state: {e}");
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "deleted"})),
                    )
                }
                Err(e) => {
                    tracing::warn!("Failed to delete cron job {id}: {e}");
                    ApiErrorResponse::not_found("Cron job not found").into_json_tuple()
                }
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
    .into_response()
}

/// PUT /api/cron/jobs/{id} — Update a cron job's configuration.
#[utoipa::path(put, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = serde_json::Value, responses((status = 200, description = "Cron job updated", body = serde_json::Value)))]
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().update_job(job_id, &body) {
                Ok(job) => {
                    let _ = state.kernel.cron().persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::to_value(&job).unwrap_or_default()),
                    )
                }
                Err(e) => {
                    tracing::warn!("Failed to update cron job {id}: {e}");
                    ApiErrorResponse::not_found("Cron job not found").into_json_tuple()
                }
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
    .into_response()
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
#[utoipa::path(put, path = "/api/cron/jobs/{id}/enable", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = serde_json::Value, responses((status = 200, description = "Cron job toggled", body = serde_json::Value)))]
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().set_enabled(job_id, enabled) {
                Ok(()) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::warn!("Failed to persist cron scheduler state: {e}");
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => {
                    tracing::warn!("Failed to toggle cron job {id}: {e}");
                    ApiErrorResponse::not_found("Cron job not found").into_json_tuple()
                }
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
    .into_response()
}

/// GET /api/cron/jobs/{id} — Get a single cron job by ID.
#[utoipa::path(get, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job details", body = serde_json::Value), (status = 404, description = "Job not found")))]
pub async fn get_cron_job(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
    .into_response()
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
#[utoipa::path(get, path = "/api/cron/jobs/{id}/status", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job status", body = serde_json::Value)))]
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
    .into_response()
}

// ---------------------------------------------------------------------------
// Workflow template routes
// ---------------------------------------------------------------------------

/// Query parameters for listing workflow templates.
#[derive(Debug, Deserialize)]
pub struct TemplateListParams {
    /// Free-text search across name, description, and tags.
    pub q: Option<String>,
    /// Filter by category (exact match).
    pub category: Option<String>,
}

/// GET /api/workflow-templates — List all workflow templates with optional search/filter.
#[utoipa::path(
    get,
    path = "/api/workflow-templates",
    tag = "workflows",
    params(
        ("q" = Option<String>, Query, description = "Search name, description, tags"),
        ("category" = Option<String>, Query, description = "Filter by category"),
    ),
    responses(
        (status = 200, description = "List of workflow templates", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_workflow_templates(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Query(params): Query<TemplateListParams>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let all = state.kernel.templates().list().await;

    let filtered: Vec<_> = all
        .into_iter()
        .filter(|t| {
            // Category filter (exact match).
            if let Some(ref cat) = params.category {
                match &t.category {
                    Some(tc) if tc == cat => {}
                    _ => return false,
                }
            }
            // Free-text search across name, description, tags.
            if let Some(ref q) = params.q {
                let q_lower = q.to_lowercase();
                let matches_name = t.name.to_lowercase().contains(&q_lower);
                let matches_desc = t.description.to_lowercase().contains(&q_lower);
                let matches_tags = t
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&q_lower));
                if !matches_name && !matches_desc && !matches_tags {
                    return false;
                }
            }
            true
        })
        .collect();

    let list: Vec<serde_json::Value> = filtered
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    Json(serde_json::json!({ "templates": list })).into_response()
}

/// GET /api/workflow-templates/:id — Get full template details.
#[utoipa::path(
    get,
    path = "/api/workflow-templates/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    responses(
        (status = 200, description = "Template details", body = serde_json::Value),
        (status = 404, description = "Template not found")
    )
)]
pub async fn get_workflow_template(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    match state.kernel.templates().get(&id).await {
        Some(t) => (
            StatusCode::OK,
            Json(serde_json::to_value(&t).unwrap_or_default()),
        ),
        None => {
            ApiErrorResponse::not_found(format!("Template '{}' not found", id)).into_json_tuple()
        }
    }
    .into_response()
}

/// POST /api/workflow-templates/:id/instantiate — Create a live workflow from a template.
#[utoipa::path(
    post,
    path = "/api/workflow-templates/{id}/instantiate",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    request_body = HashMap<String, serde_json::Value>,
    responses(
        (status = 201, description = "Workflow created from template", body = serde_json::Value),
        (status = 400, description = "Invalid parameters"),
        (status = 404, description = "Template not found")
    )
)]
pub async fn instantiate_template(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(id): Path<String>,
    Json(params): Json<HashMap<String, serde_json::Value>>,
) -> axum::response::Response {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }
    let template = match state.kernel.templates().get(&id).await {
        Some(t) => t,
        None => {
            return ApiErrorResponse::not_found(format!("Template '{}' not found", id))
                .into_json_tuple()
                .into_response();
        }
    };

    let mut workflow = match state.kernel.templates().instantiate(&template, &params) {
        Ok(w) => w,
        Err(e) => {
            return ApiErrorResponse::bad_request(e)
                .into_json_tuple()
                .into_response();
        }
    };
    workflow.account_id = account.0.clone();

    let workflow_id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "workflow_id": workflow_id.to_string(),
            "template_id": id,
            "status": "instantiated",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode as HttpStatusCode;
    use http_body_util::BodyExt;
    use librefang_types::agent::{AgentEntry, AgentIdentity, AgentManifest, AgentMode, AgentState};
    use librefang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_app_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let config = librefang_types::config::KernelConfig {
            home_dir: home.clone(),
            data_dir: home.join("data"),
            ..Default::default()
        };
        let kernel =
            Arc::new(librefang_kernel::LibreFangKernel::boot_with_config(config).expect("kernel"));
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
            webhook_store: crate::webhook_store::WebhookStore::load(home.join("webhooks.json")),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            #[cfg(feature = "telemetry")]
            prometheus_handle: None,
            account_sig_secret: None,
        });
        (tmp, state)
    }

    fn tenant_agent(name: &str, account_id: &str) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            account_id: Some(account_id.to_string()),
            name: name.to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: vec![],
            session_id: Default::default(),
            source_toml_path: None,
            tags: vec![],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
        }
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        serde_json::from_slice(&body).expect("json")
    }

    // -----------------------------------------------------------------------
    // parse_step_mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn step_mode_flat_sequential() {
        let mode = parse_step_mode(&json!("sequential"), &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    #[test]
    fn step_mode_flat_fan_out() {
        let mode = parse_step_mode(&json!("fan_out"), &json!({}));
        assert!(matches!(mode, StepMode::FanOut));
    }

    #[test]
    fn step_mode_flat_collect() {
        let mode = parse_step_mode(&json!("collect"), &json!({}));
        assert!(matches!(mode, StepMode::Collect));
    }

    #[test]
    fn step_mode_flat_conditional_with_condition() {
        let step = json!({"condition": "status == ok"});
        let mode = parse_step_mode(&json!("conditional"), &step);
        match mode {
            StepMode::Conditional { condition } => {
                assert_eq!(condition, "status == ok");
            }
            other => panic!("expected Conditional, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_flat_conditional_missing_condition() {
        let mode = parse_step_mode(&json!("conditional"), &json!({}));
        match mode {
            StepMode::Conditional { condition } => {
                assert_eq!(condition, "", "should default to empty string");
            }
            other => panic!("expected Conditional, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_flat_loop_with_fields() {
        let step = json!({"max_iterations": 10, "until": "done"});
        let mode = parse_step_mode(&json!("loop"), &step);
        match mode {
            StepMode::Loop {
                max_iterations,
                until,
            } => {
                assert_eq!(max_iterations, 10);
                assert_eq!(until, "done");
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_flat_loop_missing_fields() {
        let mode = parse_step_mode(&json!("loop"), &json!({}));
        match mode {
            StepMode::Loop {
                max_iterations,
                until,
            } => {
                assert_eq!(max_iterations, 5, "should default to 5");
                assert_eq!(until, "", "should default to empty string");
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_flat_loop_large_max_iterations_clamped() {
        // u64 value exceeding u32::MAX should fall back to default (5)
        let step = json!({"max_iterations": u64::MAX, "until": "x"});
        let mode = parse_step_mode(&json!("loop"), &step);
        match mode {
            StepMode::Loop { max_iterations, .. } => {
                assert_eq!(max_iterations, 5, "should fall back to 5 on u32 overflow");
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_flat_unknown_string_defaults_sequential() {
        let mode = parse_step_mode(&json!("banana"), &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    #[test]
    fn step_mode_nested_conditional() {
        let val = json!({"conditional": {"condition": "x > 0"}});
        let mode = parse_step_mode(&val, &json!({}));
        match mode {
            StepMode::Conditional { condition } => assert_eq!(condition, "x > 0"),
            other => panic!("expected Conditional, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_nested_conditional_missing_condition() {
        let val = json!({"conditional": {}});
        let mode = parse_step_mode(&val, &json!({}));
        match mode {
            StepMode::Conditional { condition } => {
                assert_eq!(condition, "", "should default to empty string");
            }
            other => panic!("expected Conditional, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_nested_loop() {
        let val = json!({"loop": {"max_iterations": 3, "until": "finish"}});
        let mode = parse_step_mode(&val, &json!({}));
        match mode {
            StepMode::Loop {
                max_iterations,
                until,
            } => {
                assert_eq!(max_iterations, 3);
                assert_eq!(until, "finish");
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_nested_loop_missing_fields() {
        let val = json!({"loop": {}});
        let mode = parse_step_mode(&val, &json!({}));
        match mode {
            StepMode::Loop {
                max_iterations,
                until,
            } => {
                assert_eq!(max_iterations, 5);
                assert_eq!(until, "");
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_nested_loop_large_max_iterations() {
        let val = json!({"loop": {"max_iterations": u64::MAX}});
        let mode = parse_step_mode(&val, &json!({}));
        match mode {
            StepMode::Loop { max_iterations, .. } => {
                assert_eq!(max_iterations, 5);
            }
            other => panic!("expected Loop, got {other:?}"),
        }
    }

    #[test]
    fn step_mode_nested_fan_out() {
        let val = json!({"fan_out": {}});
        let mode = parse_step_mode(&val, &json!({}));
        assert!(matches!(mode, StepMode::FanOut));
    }

    #[test]
    fn step_mode_nested_collect() {
        let val = json!({"collect": {}});
        let mode = parse_step_mode(&val, &json!({}));
        assert!(matches!(mode, StepMode::Collect));
    }

    #[test]
    fn step_mode_nested_sequential() {
        let val = json!({"sequential": {}});
        let mode = parse_step_mode(&val, &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    #[test]
    fn step_mode_null_defaults_sequential() {
        let mode = parse_step_mode(&json!(null), &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    #[test]
    fn step_mode_number_defaults_sequential() {
        let mode = parse_step_mode(&json!(42), &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    #[test]
    fn step_mode_empty_object_defaults_sequential() {
        let mode = parse_step_mode(&json!({}), &json!({}));
        assert!(matches!(mode, StepMode::Sequential));
    }

    // -----------------------------------------------------------------------
    // parse_error_mode tests
    // -----------------------------------------------------------------------

    #[test]
    fn error_mode_flat_fail() {
        let mode = parse_error_mode(&json!("fail"), &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[test]
    fn error_mode_flat_skip() {
        let mode = parse_error_mode(&json!("skip"), &json!({}));
        assert!(matches!(mode, ErrorMode::Skip));
    }

    #[test]
    fn error_mode_flat_retry_with_value() {
        let step = json!({"max_retries": 7});
        let mode = parse_error_mode(&json!("retry"), &step);
        match mode {
            ErrorMode::Retry { max_retries } => assert_eq!(max_retries, 7),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_flat_retry_missing_max_retries() {
        let mode = parse_error_mode(&json!("retry"), &json!({}));
        match mode {
            ErrorMode::Retry { max_retries } => {
                assert_eq!(max_retries, 3, "should default to 3");
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_flat_retry_large_value_clamped() {
        let step = json!({"max_retries": u64::MAX});
        let mode = parse_error_mode(&json!("retry"), &step);
        match mode {
            ErrorMode::Retry { max_retries } => {
                assert_eq!(max_retries, 3, "should fall back to 3 on u32 overflow");
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_flat_unknown_defaults_fail() {
        let mode = parse_error_mode(&json!("explode"), &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[test]
    fn error_mode_nested_retry() {
        let val = json!({"retry": {"max_retries": 2}});
        let mode = parse_error_mode(&val, &json!({}));
        match mode {
            ErrorMode::Retry { max_retries } => assert_eq!(max_retries, 2),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_nested_retry_missing_max_retries() {
        let val = json!({"retry": {}});
        let mode = parse_error_mode(&val, &json!({}));
        match mode {
            ErrorMode::Retry { max_retries } => assert_eq!(max_retries, 3),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_nested_retry_large_value() {
        let val = json!({"retry": {"max_retries": u64::MAX}});
        let mode = parse_error_mode(&val, &json!({}));
        match mode {
            ErrorMode::Retry { max_retries } => assert_eq!(max_retries, 3),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_nested_skip() {
        let val = json!({"skip": {}});
        let mode = parse_error_mode(&val, &json!({}));
        assert!(matches!(mode, ErrorMode::Skip));
    }

    #[test]
    fn error_mode_nested_fail() {
        let val = json!({"fail": {}});
        let mode = parse_error_mode(&val, &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[test]
    fn error_mode_null_defaults_fail() {
        let mode = parse_error_mode(&json!(null), &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[test]
    fn error_mode_number_defaults_fail() {
        let mode = parse_error_mode(&json!(99), &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[test]
    fn error_mode_empty_object_defaults_fail() {
        let mode = parse_error_mode(&json!({}), &json!({}));
        assert!(matches!(mode, ErrorMode::Fail));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_schedule_system_event_does_not_trigger_other_tenants() {
        let (_tmp, state) = test_app_state();

        let tenant_a_source = tenant_agent("tenant-a-source", "tenant-a");
        let tenant_a_target = tenant_agent("tenant-a-target", "tenant-a");
        let tenant_b_target = tenant_agent("tenant-b-target", "tenant-b");
        let tenant_a_source_id = tenant_a_source.id;
        let tenant_a_target_id = tenant_a_target.id;
        let tenant_b_target_id = tenant_b_target.id;
        state
            .kernel
            .agent_registry()
            .register(tenant_a_source)
            .unwrap();
        state
            .kernel
            .agent_registry()
            .register(tenant_a_target)
            .unwrap();
        state
            .kernel
            .agent_registry()
            .register(tenant_b_target)
            .unwrap();

        let tenant_a_trigger = state
            .kernel
            .register_trigger(
                Some("tenant-a".to_string()),
                tenant_a_target_id,
                TriggerPattern::All,
                "tenant-a saw {{event}}".to_string(),
                0,
            )
            .unwrap();
        let tenant_b_trigger = state
            .kernel
            .register_trigger(
                Some("tenant-b".to_string()),
                tenant_b_target_id,
                TriggerPattern::All,
                "tenant-b saw {{event}}".to_string(),
                0,
            )
            .unwrap();

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: tenant_a_source_id,
            account_id: Some("tenant-a".to_string()),
            name: "tenant-a-system-event".to_string(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 60 },
            action: CronAction::SystemEvent {
                text: "scheduled wake".to_string(),
            },
            delivery: CronDelivery::None,
            created_at: chrono::Utc::now(),
            last_run: None,
            next_run: None,
        };
        let job_id = job.id;
        state.kernel.cron().add_job(job, false).unwrap();

        let response = run_schedule(
            State(state.clone()),
            ConcreteAccountId("tenant-a".to_string()),
            Path(job_id.to_string()),
        )
        .await;
        assert_eq!(response.status(), HttpStatusCode::OK);

        let tenant_a = state
            .kernel
            .trigger_engine()
            .get(tenant_a_trigger)
            .expect("tenant-a trigger");
        let tenant_b = state
            .kernel
            .trigger_engine()
            .get(tenant_b_trigger)
            .expect("tenant-b trigger");
        assert_eq!(
            tenant_a.fire_count, 1,
            "same-tenant trigger should still fire"
        );
        assert_eq!(
            tenant_b.fire_count, 0,
            "manual tenant-a schedule must not wake tenant-b trigger"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_schedule_sanitizes_internal_errors() {
        let (_tmp, state) = test_app_state();

        let tenant_a_source = tenant_agent("tenant-a-source", "tenant-a");
        let tenant_a_source_id = tenant_a_source.id;
        state
            .kernel
            .agent_registry()
            .register(tenant_a_source)
            .unwrap();

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: tenant_a_source_id,
            account_id: Some("tenant-a".to_string()),
            name: "tenant-a-failing-workflow".to_string(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 60 },
            action: CronAction::Workflow {
                workflow_id: uuid::Uuid::new_v4().to_string(),
                input: Some("hello".to_string()),
                timeout_secs: Some(30),
            },
            delivery: CronDelivery::None,
            created_at: chrono::Utc::now(),
            last_run: None,
            next_run: None,
        };
        let job_id = job.id;
        state.kernel.cron().add_job(job, false).unwrap();

        let response = run_schedule(
            State(state),
            ConcreteAccountId("tenant-a".to_string()),
            Path(job_id.to_string()),
        )
        .await;
        assert_eq!(response.status(), HttpStatusCode::INTERNAL_SERVER_ERROR);

        let body = response_json(response).await;
        let error = body["error"].as_str().unwrap_or_default();
        assert_eq!(
            error, "Workflow execution failed",
            "route should return the stable sanitized execution error"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dry_run_workflow_sanitizes_internal_errors() {
        let (_tmp, state) = test_app_state();

        let response = dry_run_workflow(
            State(state),
            ConcreteAccountId("tenant-a".to_string()),
            Path(uuid::Uuid::new_v4().to_string()),
            Json(serde_json::json!({"input": "hello"})),
        )
        .await;
        assert_eq!(response.status(), HttpStatusCode::NOT_FOUND);

        let body = response_json(response).await;
        let error = body["error"].as_str().unwrap_or_default();
        assert_eq!(
            error, "Workflow not found",
            "route should return the stable sanitized not-found message"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_schedule_workflow_succeeds_for_same_tenant() {
        let (_tmp, state) = test_app_state();

        let tenant_a_source = tenant_agent("tenant-a-source", "tenant-a");
        let tenant_a_source_id = tenant_a_source.id;
        state
            .kernel
            .agent_registry()
            .register(tenant_a_source)
            .unwrap();

        let workflow = Workflow {
            id: WorkflowId::new(),
            name: "tenant-a-empty-workflow".to_string(),
            description: "A workflow with no steps for deterministic testing".to_string(),
            steps: vec![],
            created_at: chrono::Utc::now(),
            account_id: Some("tenant-a".to_string()),
            layout: None,
        };
        let workflow_id = workflow.id;
        state.kernel.register_workflow(workflow).await;

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: tenant_a_source_id,
            account_id: Some("tenant-a".to_string()),
            name: "tenant-a-scheduled-workflow".to_string(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 60 },
            action: CronAction::Workflow {
                workflow_id: workflow_id.to_string(),
                input: Some("hello".to_string()),
                timeout_secs: Some(30),
            },
            delivery: CronDelivery::None,
            created_at: chrono::Utc::now(),
            last_run: None,
            next_run: None,
        };
        let job_id = job.id;
        state.kernel.cron().add_job(job, false).unwrap();

        let response = run_schedule(
            State(state),
            ConcreteAccountId("tenant-a".to_string()),
            Path(job_id.to_string()),
        )
        .await;
        assert_eq!(response.status(), HttpStatusCode::OK);

        let body = response_json(response).await;
        assert_eq!(body["status"], "completed");
        assert_eq!(body["workflow_id"], workflow_id.to_string());
        assert_eq!(body["output"], "hello");
    }
}
