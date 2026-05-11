//! Workflow, trigger, schedule, and cron job handlers.

use super::AppState;

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
            axum::routing::get(get_trigger)
                .delete(delete_trigger)
                .patch(update_trigger),
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
        .route(
            "/workflows/runs/{run_id}/cancel",
            axum::routing::post(cancel_workflow_run),
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
use crate::triggers::{Trigger, TriggerId, TriggerPatch, TriggerPattern};
use crate::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowRun, WorkflowRunId,
    WorkflowRunState, WorkflowStep,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::kernel_handle::prelude::*;
use librefang_types::agent::AgentId;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

use crate::types::ApiErrorResponse;

/// Render a `Workflow` into the JSON shape used by the GET handler.
///
/// Centralized so that mutation handlers (PUT) can return the post-mutation
/// entity in the same shape the dashboard already consumes for GET, letting
/// the caller patch caches in place via `setQueryData` instead of a follow-up
/// refetch (#3832).
fn workflow_to_json(w: &Workflow) -> serde_json::Value {
    serde_json::json!({
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
        "total_timeout_secs": w.total_timeout_secs,
    })
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
                backoff_ms: step["backoff_ms"].as_u64(),
                jitter_pct: step["jitter_pct"]
                    .as_u64()
                    .and_then(|v| u8::try_from(v).ok()),
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
                backoff_ms: inner["backoff_ms"].as_u64(),
                jitter_pct: inner["jitter_pct"]
                    .as_u64()
                    .and_then(|v| u8::try_from(v).ok()),
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Workflow created", body = crate::types::JsonObject),
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
            return ApiErrorResponse::bad_request("Missing 'steps' array").into_json_tuple();
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
            return ApiErrorResponse::bad_request(format!(
                "Step '{}' needs 'agent_id' or 'agent_name'",
                step_name
            ))
            .into_json_tuple();
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
    let total_timeout_secs = req["total_timeout_secs"].as_u64();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name,
        description,
        steps,
        created_at: chrono::Utc::now(),
        layout,
        total_timeout_secs,
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
    let engine = state.kernel.workflow_engine();
    let workflows = engine.list_workflows().await;
    let all_runs = engine.list_runs(None).await;

    // Per-workflow run aggregates: total count, completed/failed/cancelled
    // counts, and the most recent run summary for the row badge. Computed in
    // one pass over `all_runs` to avoid N+1 scans across O(workflows × runs).
    //
    // `success_rate` = completed / (completed + failed). Cancelled runs are
    // NOT included in the denominator — a user-initiated cancel is not a
    // reliability signal for the workflow itself.
    struct RunAgg<'a> {
        total: usize,
        completed: usize,
        failed: usize,
        cancelled: usize,
        latest: Option<&'a WorkflowRun>,
    }
    let mut agg: std::collections::HashMap<String, RunAgg> = std::collections::HashMap::new();
    for r in &all_runs {
        let entry = agg.entry(r.workflow_id.to_string()).or_insert(RunAgg {
            total: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            latest: None,
        });
        entry.total += 1;
        match &r.state {
            WorkflowRunState::Completed => entry.completed += 1,
            WorkflowRunState::Failed => entry.failed += 1,
            WorkflowRunState::Cancelled => entry.cancelled += 1,
            _ => {}
        }
        match entry.latest {
            None => entry.latest = Some(r),
            Some(prev) if r.started_at > prev.started_at => entry.latest = Some(r),
            _ => {}
        }
    }

    let state_kind = |s: &WorkflowRunState| -> &'static str {
        match s {
            WorkflowRunState::Pending => "pending",
            WorkflowRunState::Running => "running",
            WorkflowRunState::Paused { .. } => "paused",
            WorkflowRunState::Completed => "completed",
            WorkflowRunState::Failed => "failed",
            WorkflowRunState::Cancelled => "cancelled",
        }
    };

    // Load cron jobs to find workflow-bound schedules
    let all_cron_jobs = state.kernel.cron().list_all_jobs();

    let items: Vec<serde_json::Value> = workflows
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
            let wf_agg = agg.get(&wid);
            let run_count = wf_agg.map(|a| a.total).unwrap_or(0);
            let last_run_json = wf_agg.and_then(|a| a.latest).map(|r| {
                serde_json::json!({
                    "state": state_kind(&r.state),
                    "started_at": r.started_at.to_rfc3339(),
                    "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
                })
            });
            // success_rate = completed / (completed + failed). Cancelled runs
            // are excluded from the denominator — they are not a reliability
            // signal. Null until at least one non-cancelled terminal run exists
            // (surfacing 0% on a workflow with only in-flight/cancelled runs
            // would be misleading).
            let success_rate = wf_agg.and_then(|a| {
                let terminal = a.completed + a.failed;
                (terminal > 0).then(|| a.completed as f32 / terminal as f32)
            });
            serde_json::json!({
                "id": wid,
                "name": w.name,
                "description": w.description,
                "steps": w.steps.len(),
                "run_count": run_count,
                "cancelled_count": wf_agg.map(|a| a.cancelled).unwrap_or(0),
                "created_at": w.created_at.to_rfc3339(),
                "schedule": schedule_json,
                "last_run": last_run_json,
                "success_rate": success_rate,
            })
        })
        .collect();
    // Workflows load from the engine in a single page (in-memory), so offset=0 / limit=None.
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/workflows/:id — Get a single workflow by ID.
#[utoipa::path(
    get,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow details", body = crate::types::JsonObject),
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
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => (StatusCode::OK, Json(workflow_to_json(&w))),
        None => {
            ApiErrorResponse::not_found(format!("Workflow '{}' not found", id)).into_json_tuple()
        }
    }
}

/// PUT /api/workflows/:id — Update an existing workflow.
#[utoipa::path(
    put,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Workflow updated", body = crate::types::JsonObject),
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
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    // Fetch existing workflow to preserve created_at
    let existing = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found("Workflow not found").into_json_tuple();
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
                return ApiErrorResponse::bad_request(format!(
                    "Step '{}' needs 'agent_id' or 'agent_name'",
                    step_name
                ))
                .into_json_tuple();
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

    // If the request contains "total_timeout_secs" (even null), use the new
    // value. If the key is absent, preserve the existing setting.
    let total_timeout_secs = if req.get("total_timeout_secs").is_some() {
        req["total_timeout_secs"].as_u64()
    } else {
        existing.total_timeout_secs
    };

    let updated = Workflow {
        id: workflow_id,
        name,
        description,
        steps,
        created_at: existing.created_at,
        layout,
        total_timeout_secs,
    };

    if !state
        .kernel
        .workflow_engine()
        .update_workflow(workflow_id, updated.clone())
        .await
    {
        return ApiErrorResponse::not_found("Workflow not found").into_json_tuple();
    }

    // Return the post-mutation entity in the same shape as GET so the
    // dashboard can `setQueryData` instead of round-tripping a refetch
    // (#3832). Read back from the engine in case the kernel normalized
    // anything during persist; fall back to `updated` if the row vanished
    // between write and read (narrow race — concurrent delete) so the
    // mutation still appears successful.
    let body = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(persisted) => workflow_to_json(&persisted),
        None => workflow_to_json(&updated),
    };
    (StatusCode::OK, Json(body))
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
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    if state
        .kernel
        .workflow_engine()
        .remove_workflow(workflow_id)
        .await
    {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        ApiErrorResponse::not_found("Workflow not found").into_json_tuple()
    }
}

/// POST /api/workflows/:id/run — Execute a workflow.
#[utoipa::path(post, path = "/api/workflows/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), request_body(content = crate::types::JsonObject, description = "Workflow input variables (free-form key/value object)"), responses((status = 200, description = "Workflow run started", body = crate::types::JsonObject)))]
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state.kernel.run_workflow_typed(workflow_id, input).await {
        Ok((run_id, output)) => {
            // Include step-level detail in the response so callers can inspect I/O
            let run = state.kernel.workflow_engine().get_run(run_id).await;
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
}

/// POST /api/workflows/:id/dry-run — Validate and preview a workflow without executing it.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/dry-run",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Dry-run preview", body = crate::types::JsonObject),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn dry_run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state.kernel.dry_run_workflow(workflow_id, input).await {
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
            ApiErrorResponse::not_found(e.to_string()).into_json_tuple()
        }
    }
}

/// GET /api/workflows/runs/:run_id — Get detailed info for a single workflow run.
#[utoipa::path(
    get,
    path = "/api/workflows/runs/{run_id}",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Workflow run details", body = crate::types::JsonObject),
        (status = 404, description = "Run not found")
    )
)]
pub async fn get_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    match state.kernel.workflow_engine().get_run(run_id).await {
        Some(run) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": run.id.to_string(),
                "workflow_id": run.workflow_id.to_string(),
                "workflow_name": run.workflow_name,
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
}

/// POST /api/workflows/runs/:run_id/cancel — Cancel a workflow run.
///
/// Transitions `Pending`, `Running`, or `Paused` runs to `Cancelled`.
/// Returns 200 with `{"run_id": ..., "state": "cancelled"}` on success,
/// 400 for a malformed run ID, 404 if the run does not exist, or 409 if
/// the run is already in a terminal state (includes `{"state": <state>}`
/// so callers can distinguish completed vs failed vs cancelled conflicts).
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/cancel",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Run cancelled", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID"),
        (status = 404, description = "Run not found"),
        (status = 409, description = "Run already in terminal state")
    )
)]
pub async fn cancel_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    use librefang_kernel::workflow::CancelRunError;

    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    match state.kernel.workflow_engine().cancel_run(run_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                "state": "cancelled",
            })),
        ),
        Err(CancelRunError::NotFound(_)) => {
            ApiErrorResponse::not_found(format!("Run '{run_id}' not found")).into_json_tuple()
        }
        Err(CancelRunError::AlreadyTerminal { state: s, .. }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "conflict",
                "state": s,
                "message": format!("Run '{run_id}' is already {s}"),
            })),
        ),
    }
}

/// GET /api/workflows/:id/runs — List runs for a workflow.
#[utoipa::path(get, path = "/api/workflows/{id}/runs", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "List workflow runs", body = Vec<serde_json::Value>)))]
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    let runs = state.kernel.workflow_engine().list_runs(None).await;
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
// Save workflow as reusable template
// ---------------------------------------------------------------------------

/// POST /api/workflows/:id/save-as-template — Convert a workflow into a reusable template.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/save-as-template",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Template created", body = crate::types::JsonObject),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn save_workflow_as_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
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
                .into_json_tuple();
        }
    };

    let template = workflow.to_template();

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
}

// ---------------------------------------------------------------------------
// Trigger routes
// ---------------------------------------------------------------------------

/// POST /api/triggers — Register a new event trigger.
#[utoipa::path(
    post,
    path = "/api/triggers",
    tag = "workflows",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Trigger created", body = crate::types::JsonObject),
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
            return ApiErrorResponse::bad_request("Missing 'agent_id'").into_json_tuple();
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid agent_id").into_json_tuple();
        }
    };

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => {
            // Legacy clients send `"task_posted"` as a bare string, but the
            // variant now carries an optional `assignee_match` field and
            // expects the struct form `{"task_posted": {...}}`. Rewrite the
            // bare strings to `{"<variant>": {}}` so both shapes parse.
            let normalized = normalize_pattern_json(p.clone());
            match serde_json::from_value(normalized) {
                Ok(pat) => pat,
                Err(e) => {
                    tracing::warn!("Invalid trigger pattern: {e}");
                    return ApiErrorResponse::bad_request("Invalid trigger pattern")
                        .into_json_tuple();
                }
            }
        }
        None => {
            return ApiErrorResponse::bad_request("Missing 'pattern'").into_json_tuple();
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
                .into_json_tuple();
            }
        },
    };

    let cooldown_secs: Option<u64> = req["cooldown_secs"].as_u64();

    let session_mode: Option<librefang_types::agent::SessionMode> =
        match req.get("session_mode").and_then(|v| v.as_str()) {
            None => None,
            Some(s) => match serde_json::from_value(serde_json::json!(s)) {
                Ok(m) => Some(m),
                Err(_) => {
                    return ApiErrorResponse::bad_request(format!(
                        "Invalid 'session_mode': '{s}' (expected 'persistent' or 'new')"
                    ))
                    .into_json_tuple();
                }
            },
        };

    // Optional workflow_id: if set, the trigger fires a workflow run instead
    // of dispatching a message to an agent via send_message_full.
    let workflow_id: Option<String> = match req.get("workflow_id").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => {
            if s.is_empty() {
                return ApiErrorResponse::bad_request(
                    "workflow_id must not be empty when provided",
                )
                .into_json_tuple();
            }
            if s.len() > librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN {
                return ApiErrorResponse::bad_request(format!(
                    "workflow_id too long ({} chars, max {})",
                    s.len(),
                    librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN
                ))
                .into_json_tuple();
            }
            Some(s.to_string())
        }
    };

    match state.kernel.register_trigger_with_target(
        agent_id,
        pattern,
        prompt_template,
        max_fires,
        target_agent,
        cooldown_secs,
        session_mode,
        workflow_id.clone(),
    ) {
        Ok(trigger_id) => {
            let mut resp = serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            });
            if let Some(target) = target_agent {
                resp["target_agent_id"] = serde_json::json!(target.to_string());
            }
            if let Some(wid) = workflow_id {
                resp["workflow_id"] = serde_json::json!(wid);
            }
            (StatusCode::CREATED, Json(resp))
        }
        Err(e) => {
            tracing::warn!("Trigger registration failed: {e}");
            ApiErrorResponse::not_found("Trigger registration failed (agent not found?)")
                .into_json_tuple()
        }
    }
}

/// GET /api/triggers — List all triggers (optionally filter by ?agent_id=...).
#[utoipa::path(
    get,
    path = "/api/triggers",
    tag = "workflows",
    responses(
        (status = 200, description = "List triggers", body = crate::types::JsonObject)
    )
)]
/// Serialize a `Trigger` to a JSON value (shared by list and get endpoints).
fn trigger_to_json(t: &Trigger) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": t.id.to_string(),
        "agent_id": t.agent_id.to_string(),
        "pattern": serde_json::to_value(&t.pattern).unwrap_or_default(),
        "prompt_template": t.prompt_template,
        "enabled": t.enabled,
        "fire_count": t.fire_count,
        "max_fires": t.max_fires,
        "created_at": t.created_at.to_rfc3339(),
        "cooldown_secs": t.cooldown_secs,
        "session_mode": serde_json::to_value(t.session_mode).unwrap_or(serde_json::Value::Null),
    });
    if let Some(target) = &t.target_agent {
        v["target_agent_id"] = serde_json::json!(target.to_string());
    }
    if let Some(wid) = &t.workflow_id {
        v["workflow_id"] = serde_json::json!(wid);
    }
    v
}

#[utoipa::path(get, path = "/api/triggers", tag = "workflows", params(("agent_id" = Option<String>, Query, description = "Filter by agent ID")), responses((status = 200, description = "List triggers", body = crate::types::JsonObject)))]
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    // Owner-scoping: non-admins can't see triggers for agents they don't
    // author. Two enforcement points:
    //   1. With ?agent_id=... — verify the caller owns that agent.
    //   2. Without — post-filter the trigger list by author.
    let restrict_to: Option<String> = match api_user.as_ref() {
        Some(u) if u.0.role < crate::middleware::UserRole::Admin => Some(u.0.name.clone()),
        _ => None,
    };
    if let (Some(user_name), Some(aid)) = (restrict_to.as_ref(), agent_filter) {
        let owns = state
            .kernel
            .agent_registry()
            .get(aid)
            .as_ref()
            .map(|e| e.manifest.author.eq_ignore_ascii_case(user_name))
            .unwrap_or(false);
        if !owns {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"triggers": [], "total": 0})),
            )
                .into_response();
        }
    }

    let triggers = state.kernel.list_triggers(agent_filter);
    let list: Vec<serde_json::Value> = if let Some(ref user_name) = restrict_to {
        // No explicit agent_id — fall back to per-trigger owner check.
        let owned_ids: std::collections::HashSet<librefang_types::agent::AgentId> = state
            .kernel
            .agent_registry()
            .list()
            .iter()
            .filter(|e| e.manifest.author.eq_ignore_ascii_case(user_name))
            .map(|e| e.id)
            .collect();
        triggers
            .iter()
            .filter(|tr| owned_ids.contains(&tr.agent_id))
            .map(trigger_to_json)
            .collect()
    } else {
        triggers.iter().map(trigger_to_json).collect()
    };
    let total = list.len();
    Json(serde_json::json!({"triggers": list, "total": total})).into_response()
}

#[utoipa::path(get, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), responses((status = 200, description = "Trigger detail", body = crate::types::JsonObject), (status = 404, description = "Not found")))]
/// GET /api/triggers/:id — Fetch a single trigger by ID.
pub async fn get_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple(),
    });
    match state.kernel.get_trigger(trigger_id) {
        Some(t) => (StatusCode::OK, Json(trigger_to_json(&t))),
        None => ApiErrorResponse::not_found("Trigger not found").into_json_tuple(),
    }
}

/// DELETE /api/triggers/:id — Remove a trigger.
///
/// Idempotent (RFC 9110 §9.2.2): deleting a trigger that is already gone
/// returns `200 OK` with `{"status": "already-deleted"}` instead of `404`.
/// `400` is reserved for the malformed-UUID case alone. Refs #3509.
#[utoipa::path(
    delete,
    path = "/api/triggers/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Trigger ID")),
    responses(
        (status = 200, description = "Trigger deleted (or was already absent — idempotent)"),
        (status = 400, description = "Malformed trigger ID")
    )
)]
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple();
        }
    });

    if state.kernel.remove_trigger(trigger_id) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        // Idempotent DELETE — replayed request, double-click, or already
        // removed by another caller. Surface success so clients don't have
        // to special-case 404 on a successful-state outcome.
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "already-deleted", "trigger_id": id})),
        )
    }
}

// ---------------------------------------------------------------------------
// Trigger update endpoint
// ---------------------------------------------------------------------------

#[utoipa::path(patch, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), request_body(content = crate::types::JsonObject, description = "Partial trigger fields: pattern, prompt_template, enabled, max_fires, cooldown_secs, session_mode, target_agent_id"), responses((status = 200, description = "Updated trigger", body = crate::types::JsonObject), (status = 404, description = "Not found")))]
/// PATCH /api/triggers/:id — Partially update a trigger.
///
/// All body fields are optional. Only provided fields are changed.
/// Supported fields: `pattern`, `prompt_template`, `enabled`, `max_fires`,
/// `cooldown_secs` (pass `null` to clear), `session_mode` (pass `null` to clear),
/// `target_agent_id` (pass `null` to clear, omit to leave unchanged).
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple(),
    });

    // Parse pattern if provided
    let pattern = if req.get("pattern").is_some() && !req["pattern"].is_null() {
        let normalized = normalize_pattern_json(req["pattern"].clone());
        match serde_json::from_value::<TriggerPattern>(normalized) {
            Ok(p) => Some(p),
            Err(e) => {
                return ApiErrorResponse::bad_request(format!("Invalid pattern: {e}"))
                    .into_json_tuple()
            }
        }
    } else {
        None
    };

    // Parse session_mode: absent = no change, null = clear, string = set
    let session_mode: Option<Option<librefang_types::agent::SessionMode>> =
        if req.get("session_mode").is_none() {
            None
        } else if req["session_mode"].is_null() {
            Some(None)
        } else {
            match serde_json::from_value(req["session_mode"].clone()) {
                Ok(m) => Some(Some(m)),
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!("Invalid session_mode: {e}"))
                        .into_json_tuple()
                }
            }
        };

    // Parse cooldown_secs: absent = no change, null = clear, number = set
    let cooldown_secs: Option<Option<u64>> = if req.get("cooldown_secs").is_none() {
        None
    } else if req["cooldown_secs"].is_null() {
        Some(None)
    } else {
        match req["cooldown_secs"].as_u64() {
            Some(n) => Some(Some(n)),
            None => {
                return ApiErrorResponse::bad_request(
                    "cooldown_secs must be a non-negative integer",
                )
                .into_json_tuple()
            }
        }
    };

    // Parse target_agent_id: absent = no change, null = clear, string = set
    let target_agent: Option<Option<AgentId>> = if req.get("target_agent_id").is_none() {
        None
    } else if req["target_agent_id"].is_null() {
        Some(None)
    } else {
        match req["target_agent_id"].as_str().and_then(|s| s.parse().ok()) {
            Some(id) => Some(Some(id)),
            None => {
                return ApiErrorResponse::bad_request("Invalid 'target_agent_id'").into_json_tuple()
            }
        }
    };

    // Validate target agent exists when being set (mirrors POST validation)
    if let Some(Some(target_id)) = target_agent {
        if state.kernel.agent_registry().get(target_id).is_none() {
            return ApiErrorResponse::bad_request(format!(
                "target_agent_id '{target_id}' does not exist"
            ))
            .into_json_tuple();
        }
    }

    // Parse workflow_id: absent = no change, null = clear, string = set
    let workflow_id: Option<Option<String>> = if req.get("workflow_id").is_none() {
        None
    } else if req["workflow_id"].is_null() {
        Some(None)
    } else {
        match req["workflow_id"].as_str() {
            Some(s) => {
                if s.is_empty() {
                    return ApiErrorResponse::bad_request(
                        "workflow_id must not be empty when provided",
                    )
                    .into_json_tuple();
                }
                if s.len() > librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN {
                    return ApiErrorResponse::bad_request(format!(
                        "workflow_id too long ({} chars, max {})",
                        s.len(),
                        librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN
                    ))
                    .into_json_tuple();
                }
                Some(Some(s.to_string()))
            }
            None => {
                return ApiErrorResponse::bad_request("workflow_id must be a string or null")
                    .into_json_tuple()
            }
        }
    };

    let patch = TriggerPatch {
        pattern,
        prompt_template: req["prompt_template"].as_str().map(|s| s.to_string()),
        enabled: req["enabled"].as_bool(),
        max_fires: req["max_fires"].as_u64(),
        cooldown_secs,
        session_mode,
        target_agent,
        workflow_id,
    };

    match state.kernel.update_trigger(trigger_id, patch) {
        Some(t) => (StatusCode::OK, Json(trigger_to_json(&t))),
        None => ApiErrorResponse::not_found("Trigger not found").into_json_tuple(),
    }
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

/// Normalize a trigger-pattern JSON value so legacy and new shapes both parse.
///
/// Variants that gained optional fields after shipping need to accept both
/// `"task_posted"` (the old bare-string form) and
/// `{"task_posted": {...}}` (the new struct form). Rewrite bare strings of
/// variants that carry optional data into empty-object form so serde
/// deserialises the `#[serde(default)]` fields cleanly.
///
/// Currently `task_posted` is the only struct variant with optional fields;
/// extend this match when other variants gain optional fields.
fn normalize_pattern_json(value: serde_json::Value) -> serde_json::Value {
    match value.as_str() {
        Some(tag @ "task_posted") => {
            serde_json::json!({ tag: {} })
        }
        _ => value,
    }
}

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
    let (cron_expr, tz) = match &job.schedule {
        librefang_types::scheduler::CronSchedule::Cron { expr, tz } => (expr.clone(), tz.clone()),
        librefang_types::scheduler::CronSchedule::Every { every_secs } => {
            (format!("every {every_secs}s"), None)
        }
        librefang_types::scheduler::CronSchedule::At { at } => {
            (format!("at {}", at.to_rfc3339()), None)
        }
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
    // Serialize delivery_targets so callers can round-trip the field through
    // the schedule view without a second GET on the raw cron-job endpoint.
    let delivery_targets = serde_json::to_value(&job.delivery_targets)
        .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    serde_json::json!({
        "id": job.id.to_string(),
        "name": job.name,
        "cron": cron_expr,
        "tz": tz,
        "agent_id": job.agent_id.to_string(),
        "workflow_id": workflow_id,
        "message": message,
        "enabled": job.enabled,
        "created_at": job.created_at.to_rfc3339(),
        "last_run": job.last_run.map(|t| t.to_rfc3339()),
        "next_run": job.next_run.map(|t| t.to_rfc3339()),
        "delivery_targets": delivery_targets,
    })
}

/// GET /api/schedules — List all scheduled jobs.
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// (#3842) so the generated SDK can share one list-response type across all
/// list endpoints. The legacy `schedules` key was renamed to `items`; offset
/// is always 0 and limit is null because this endpoint returns the full set.
#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "workflows",
    responses(
        (status = 200, description = "List schedules", body = crate::types::JsonObject)
    )
)]
pub async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let jobs = state.kernel.cron().list_all_jobs();
    let schedules: Vec<serde_json::Value> = jobs.iter().map(cron_job_to_schedule_json).collect();
    let total = schedules.len();
    Json(crate::types::PaginatedResponse {
        items: schedules,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/schedules/{id} — Get a specific schedule by ID.
#[utoipa::path(get, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule details", body = crate::types::JsonObject)))]
pub async fn get_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };
    match state.kernel.cron().get_job(job_id) {
        Some(job) => (StatusCode::OK, Json(cron_job_to_schedule_json(&job))),
        None => ApiErrorResponse::not_found(format!("Schedule '{id}' not found")).into_json_tuple(),
    }
}

/// POST /api/schedules — Create a new scheduled job (backed by CronScheduler).
#[utoipa::path(
    post,
    path = "/api/schedules",
    tag = "workflows",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Schedule created", body = crate::types::JsonObject),
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
            return ApiErrorResponse::bad_request("Missing 'name' field").into_json_tuple();
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'cron' field").into_json_tuple();
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return ApiErrorResponse::bad_request(
            "Invalid cron expression: must have 5 fields (min hour dom mon dow)",
        )
        .into_json_tuple();
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    let workflow_id_str = req["workflow_id"].as_str().unwrap_or("").to_string();

    // Must have either agent_id or workflow_id
    if agent_id_str.is_empty() && workflow_id_str.is_empty() {
        return ApiErrorResponse::bad_request("Must provide either agent_id or workflow_id")
            .into_json_tuple();
    }

    // Resolve agent_id to a UUID
    let resolved_agent_id = if !agent_id_str.is_empty() {
        if let Ok(aid) = agent_id_str.parse::<AgentId>() {
            if state.kernel.agent_registry().get(aid).is_some() {
                aid
            } else {
                return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                    .into_json_tuple();
            }
        } else if let Some(agent) = state
            .kernel
            .agent_registry()
            .list()
            .iter()
            .find(|a| a.name == agent_id_str)
        {
            agent.id
        } else {
            return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                .into_json_tuple();
        }
    } else {
        // For workflow-only schedules, use a system agent ID
        AgentId(uuid::Uuid::from_bytes([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]))
    };

    // Validate workflow exists if provided
    if !workflow_id_str.is_empty() {
        if let Ok(wid) = workflow_id_str.parse::<uuid::Uuid>() {
            if state
                .kernel
                .workflow_engine()
                .get_workflow(WorkflowId(wid))
                .await
                .is_none()
            {
                return ApiErrorResponse::not_found(format!(
                    "Workflow not found: {workflow_id_str}"
                ))
                .into_json_tuple();
            }
        } else {
            return ApiErrorResponse::bad_request("Invalid workflow_id format").into_json_tuple();
        }
    }

    let message = req["message"].as_str().unwrap_or("").to_string();
    let tz = req["tz"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    // Validate timezone string if provided
    if let Some(ref tz_str) = tz {
        if tz_str != "UTC" && tz_str.parse::<chrono_tz::Tz>().is_err() {
            return ApiErrorResponse::bad_request(format!(
                "Invalid timezone '{tz_str}'. Use IANA format (e.g. 'America/New_York', 'Europe/Rome')"
            ))
            .into_json_tuple();
        }
    }

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
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        }
    };

    // Optional fan-out delivery targets. Validated up front so a bad shape
    // returns a 400 rather than silently dropping targets later.
    let delivery_targets: Vec<librefang_types::scheduler::CronDeliveryTarget> =
        match req.get("delivery_targets") {
            Some(serde_json::Value::Null) | None => Vec::new(),
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(t) => t,
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!("Invalid delivery_targets: {e}"))
                        .into_json_tuple();
                }
            },
        };

    let job = librefang_types::scheduler::CronJob {
        id: librefang_types::scheduler::CronJobId::new(),
        agent_id: resolved_agent_id,
        name,
        enabled: req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        schedule: librefang_types::scheduler::CronSchedule::Cron { expr: cron, tz },
        action,
        delivery: librefang_types::scheduler::CronDelivery::None,
        delivery_targets,
        peer_id: None,
        session_mode: req["session_mode"]
            .as_str()
            .and_then(|s| serde_json::from_value(serde_json::Value::String(s.to_string())).ok()),
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
            (StatusCode::CREATED, Json(entry))
        }
        Err(e) => {
            ApiErrorResponse::internal(format!("Failed to create schedule: {e}")).into_json_tuple()
        }
    }
}

/// PUT /api/schedules/:id — Update a scheduled job (toggle enabled, edit fields).
#[utoipa::path(put, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Schedule updated", body = crate::types::JsonObject)))]
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    // Build update payload compatible with CronScheduler::update_job
    let mut updates = serde_json::Map::new();
    if let Some(enabled) = req.get("enabled") {
        updates.insert("enabled".to_string(), enabled.clone());
    }
    if let Some(name) = req.get("name") {
        updates.insert("name".to_string(), name.clone());
    }
    // Read tz from the request (if provided).  When the caller sends
    // a new `cron` expression we must carry over the timezone — otherwise
    // replacing the entire schedule object would reset tz to null.
    let req_tz = req
        .get("tz")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Validate timezone string if provided
    if let Some(ref tz_str) = req_tz {
        if tz_str != "UTC" && tz_str.parse::<chrono_tz::Tz>().is_err() {
            return ApiErrorResponse::bad_request(format!(
                "Invalid timezone '{tz_str}'. Use IANA format (e.g. 'America/New_York', 'Europe/Rome')"
            ))
            .into_json_tuple();
        }
    }

    if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
        let cron_parts: Vec<&str> = cron.split_whitespace().collect();
        if cron_parts.len() != 5 {
            return ApiErrorResponse::bad_request("Invalid cron expression").into_json_tuple();
        }
        // If tz not in this request, preserve the existing tz from the job.
        let tz = req_tz.clone().or_else(|| {
            state.kernel.cron().get_meta(job_id).and_then(|meta| {
                if let librefang_types::scheduler::CronSchedule::Cron { tz, .. } =
                    &meta.job.schedule
                {
                    tz.clone()
                } else {
                    None
                }
            })
        });
        updates.insert(
            "schedule".to_string(),
            serde_json::json!({"kind": "cron", "expr": cron, "tz": tz}),
        );
    } else if req_tz.is_some() {
        // Caller wants to change only the timezone — read current cron expr.
        if let Some(meta) = state.kernel.cron().get_meta(job_id) {
            if let librefang_types::scheduler::CronSchedule::Cron { expr, .. } = &meta.job.schedule
            {
                updates.insert(
                    "schedule".to_string(),
                    serde_json::json!({"kind": "cron", "expr": expr, "tz": req_tz}),
                );
            }
        }
    }
    if let Some(agent_id) = req.get("agent_id") {
        updates.insert("agent_id".to_string(), agent_id.clone());
    }
    // Multi-destination fan-out targets: full replacement when supplied.
    // Validation is done on the kernel side via serde, but reject obviously
    // malformed payloads (non-array) up front to give a clearer 400.
    //
    // Semantics intentionally differ between `null` and `[]`:
    //   * field omitted        — leave existing targets untouched.
    //   * `delivery_targets:null` — same as omitted (preserves the
    //     existing list). The kernel `update_job` checks `is_null()` and
    //     skips the patch.
    //   * `delivery_targets:[]` — explicit clear; kernel deserializes the
    //     empty array and replaces the list with `Vec::new()`.
    // Callers that want to clear all targets must send `[]`, not `null`.
    if let Some(targets) = req.get("delivery_targets") {
        if !targets.is_null() && !targets.is_array() {
            return ApiErrorResponse::bad_request(
                "delivery_targets must be an array of CronDeliveryTarget objects",
            )
            .into_json_tuple();
        }
        updates.insert("delivery_targets".to_string(), targets.clone());
    }

    match state
        .kernel
        .cron()
        .update_job(job_id, &serde_json::Value::Object(updates))
    {
        Ok(_job) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "updated", "schedule_id": id})),
            )
        }
        // SSRF / shape rejections must map to 400, not the catch-all 404
        // — see the parallel branch in `update_cron_job` (#4732).
        Err(librefang_types::error::LibreFangError::InvalidInput(msg)) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        Err(e) => ApiErrorResponse::not_found(format!("Schedule not found: {e}")).into_json_tuple(),
    }
}

/// DELETE /api/schedules/:id — Remove a scheduled job.
#[utoipa::path(delete, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule deleted")))]
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    match state.kernel.cron().remove_job(job_id) {
        Ok(_) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "removed", "schedule_id": id})),
            )
        }
        Err(e) => ApiErrorResponse::not_found(format!("Schedule not found: {e}")).into_json_tuple(),
    }
}

/// POST /api/schedules/:id/run — Manually trigger a scheduled job now.
#[utoipa::path(post, path = "/api/schedules/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule triggered", body = crate::types::JsonObject)))]
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    let job = match state.kernel.cron().get_job(job_id) {
        Some(j) => j,
        None => {
            return ApiErrorResponse::not_found("Schedule not found").into_json_tuple();
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
                    return ApiErrorResponse::bad_request("Invalid workflow_id").into_json_tuple();
                }
            };
            let wf_input = input
                .clone()
                .unwrap_or_else(|| format!("[Scheduled workflow '{}' triggered]", name));
            match state.kernel.run_workflow_typed(wid, wf_input).await {
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
        librefang_types::scheduler::CronAction::AgentTurn { message, .. } => {
            let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone();
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
        librefang_types::scheduler::CronAction::SystemEvent { text } => {
            // Fire-and-forget system event
            let event = librefang_types::event::Event::new(
                AgentId::new(),
                librefang_types::event::EventTarget::Broadcast,
                librefang_types::event::EventPayload::Custom(text.as_bytes().to_vec()),
            );
            state.kernel.publish_typed_event(event).await;
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
                state.kernel.cron().list_jobs(aid)
            }
            Err(_) => {
                return ApiErrorResponse::bad_request("Invalid agent_id").into_json_tuple();
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
}

/// POST /api/cron/jobs — Create a new cron job.
#[utoipa::path(post, path = "/api/cron/jobs", tag = "workflows", request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job created", body = crate::types::JsonObject)))]
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
        // #3541: route structured KernelOpError through the centralized
        // From impl so the status-code contract is consistent across all
        // routes. The earlier inline match mapped `Unavailable` to 500
        // (should be 503) and `Other` to 400 (should be 500), both fixed
        // here because the From impl is the single source of truth.
        Err(e) => ApiErrorResponse::from(e).into_json_tuple(),
    }
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
///
/// Idempotent (RFC 9110 §9.2.2): deleting a cron job that is already gone
/// returns `200 OK` with `{"status": "already-deleted"}` instead of `404`.
/// `400` is reserved for the malformed-UUID case alone (Refs #3509). Returns
/// `500` if the in-memory removal succeeds but persistence to disk fails —
/// without persistence, the deletion would silently revert on daemon restart
/// (issue #3515).
#[utoipa::path(
    delete,
    path = "/api/cron/jobs/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Cron job ID")),
    responses(
        (status = 200, description = "Cron job deleted (or was already absent — idempotent)"),
        (status = 400, description = "Malformed cron job ID"),
        (status = 500, description = "Persist failed; change will not survive restart")
    )
)]
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    };
    let job_id = librefang_types::scheduler::CronJobId(uuid);
    match state.kernel.cron().remove_job(job_id) {
        Ok(_) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::error!("Failed to persist cron scheduler state after delete: {e}");
                return cron_persist_failed_response("delete", &e.to_string());
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted", "job_id": id})),
            )
        }
        Err(_) => {
            // Idempotent DELETE — the cron job is already gone (replayed
            // request, double-click, or removed by another deleter). Treat
            // as success so clients don't have to special-case 404.
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "already-deleted", "job_id": id})),
            )
        }
    }
}

/// PUT /api/cron/jobs/{id} — Update a cron job's configuration.
///
/// Returns 500 if the in-memory update succeeds but persistence to disk
/// fails — without persistence, the new schedule runs in-memory until the
/// next restart, then silently reverts to the old schedule (issue #3515).
#[utoipa::path(put, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job updated", body = crate::types::JsonObject), (status = 500, description = "Persist failed; change will not survive restart")))]
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().update_job(job_id, &body) {
                Ok(job) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::error!("Failed to persist cron scheduler state after update: {e}");
                        return cron_persist_failed_response("update", &e.to_string());
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::to_value(&job).unwrap_or_default()),
                    )
                }
                // SSRF / shape rejections from `validate_cron_delivery*`
                // surface as `InvalidInput` and must map to 400, not the
                // catch-all 404 (#4732). 404 here would silently mask a
                // refused webhook host as "schedule not found", letting
                // attacker-controlled clients confuse the failure mode.
                Err(librefang_types::error::LibreFangError::InvalidInput(msg)) => {
                    ApiErrorResponse::bad_request(msg).into_json_tuple()
                }
                Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
///
/// Returns 500 if the in-memory toggle succeeds but persistence to disk
/// fails — without persistence, the new enabled state would silently
/// revert on daemon restart (issue #3515).
#[utoipa::path(put, path = "/api/cron/jobs/{id}/enable", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job toggled", body = crate::types::JsonObject), (status = 500, description = "Persist failed; change will not survive restart")))]
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().set_enabled(job_id, enabled) {
                Ok(()) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::error!("Failed to persist cron scheduler state after toggle: {e}");
                        return cron_persist_failed_response("toggle", &e.to_string());
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// Build a 500 response for cron persist failures.
///
/// The in-memory scheduler change has already been applied at this point,
/// so the response signals two things: (a) the change is live in-memory
/// right now, but (b) it will silently revert on daemon restart unless
/// the persist failure is resolved. Clients should surface this clearly
/// (it is *not* a routine 500).
fn cron_persist_failed_response(
    operation: &str,
    detail: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": "Failed to persist cron job change",
            "code": "cron_persist_failed",
            "type": "cron_persist_failed",
            "operation": operation,
            "in_memory_applied": true,
            "will_survive_restart": false,
            "detail": detail,
        })),
    )
}

/// Look up the persistent cron session for `agent_id` and return
/// `(message_count, estimated_tokens)`. Returns `(0, 0)` when no
/// session exists yet (job has never fired in `Persistent` mode).
///
/// #3693: surfaces session-size growth to operators via the cron
/// status / detail endpoints so the trend is visible in the
/// dashboard before the provider returns a hard context-window
/// 400. Estimation matches the kernel's prune path (system prompt
/// and tools are excluded) — under-counts slightly but is
/// consistent across calls.
fn cron_session_metrics(
    state: &AppState,
    agent_id: librefang_types::agent::AgentId,
) -> (usize, u64) {
    use librefang_kernel::compactor::estimate_token_count;
    use librefang_types::agent::SessionId;

    let cron_sid = SessionId::for_channel(agent_id, "cron");
    match state.kernel.memory_substrate().get_session(cron_sid) {
        Ok(Some(session)) => {
            let count = session.messages.len();
            let tokens = estimate_token_count(&session.messages, None, None) as u64;
            (count, tokens)
        }
        _ => (0, 0),
    }
}

/// Merge a cron `JobMeta` with `session_message_count` /
/// `session_token_count` into a JSON object response (#3693).
/// Falls back to the bare `meta` JSON if it does not serialize
/// to an object — the existing schema is forward-compatible
/// because both fields are additive.
fn cron_job_response_with_metrics(
    state: &AppState,
    meta: &librefang_kernel::cron::JobMeta,
) -> serde_json::Value {
    let mut value = serde_json::to_value(meta).unwrap_or(serde_json::Value::Null);
    let (msg_count, tok_count) = cron_session_metrics(state, meta.job.agent_id);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "session_message_count".to_string(),
            serde_json::Value::from(msg_count),
        );
        obj.insert(
            "session_token_count".to_string(),
            serde_json::Value::from(tok_count),
        );
    }
    value
}

/// GET /api/cron/jobs/{id} — Get a single cron job by ID.
///
/// Response carries the cron `JobMeta` plus two #3693 observability
/// fields:
/// - `session_message_count` (`usize`): messages in the persistent
///   `(agent, "cron")` session.
/// - `session_token_count` (`u64`): kernel-estimated tokens for those
///   messages (system prompt and tools excluded — same accounting as
///   the prune path).
#[utoipa::path(get, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job details", body = crate::types::JsonObject), (status = 404, description = "Job not found")))]
pub async fn get_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(cron_job_response_with_metrics(&state, &meta)),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
///
/// Same response shape as `GET /api/cron/jobs/{id}`, including the
/// #3693 `session_message_count` / `session_token_count` fields.
#[utoipa::path(get, path = "/api/cron/jobs/{id}/status", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job status", body = crate::types::JsonObject)))]
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(cron_job_response_with_metrics(&state, &meta)),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
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
    Query(params): Query<TemplateListParams>,
) -> impl IntoResponse {
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

    Json(serde_json::json!({ "templates": list }))
}

/// GET /api/workflow-templates/:id — Get full template details.
#[utoipa::path(
    get,
    path = "/api/workflow-templates/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    responses(
        (status = 200, description = "Template details", body = crate::types::JsonObject),
        (status = 404, description = "Template not found")
    )
)]
pub async fn get_workflow_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.templates().get(&id).await {
        Some(t) => (
            StatusCode::OK,
            Json(serde_json::to_value(&t).unwrap_or_default()),
        ),
        None => {
            ApiErrorResponse::not_found(format!("Template '{}' not found", id)).into_json_tuple()
        }
    }
}

/// POST /api/workflow-templates/:id/instantiate — Create a live workflow from a template.
#[utoipa::path(
    post,
    path = "/api/workflow-templates/{id}/instantiate",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    request_body = HashMap<String, serde_json::Value>,
    responses(
        (status = 201, description = "Workflow created from template", body = crate::types::JsonObject),
        (status = 400, description = "Invalid parameters"),
        (status = 404, description = "Template not found")
    )
)]
pub async fn instantiate_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    let template = match state.kernel.templates().get(&id).await {
        Some(t) => t,
        None => {
            return ApiErrorResponse::not_found(format!("Template '{}' not found", id))
                .into_json_tuple();
        }
    };

    let workflow = match state.kernel.templates().instantiate(&template, &params) {
        Ok(w) => w,
        Err(e) => {
            return ApiErrorResponse::bad_request(e).into_json_tuple();
        }
    };

    let workflow_id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "workflow_id": workflow_id.to_string(),
            "template_id": id,
            "status": "instantiated",
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            ErrorMode::Retry { max_retries, .. } => assert_eq!(max_retries, 7),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_flat_retry_missing_max_retries() {
        let mode = parse_error_mode(&json!("retry"), &json!({}));
        match mode {
            ErrorMode::Retry { max_retries, .. } => {
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
            ErrorMode::Retry { max_retries, .. } => {
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
            ErrorMode::Retry { max_retries, .. } => assert_eq!(max_retries, 2),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_nested_retry_missing_max_retries() {
        let val = json!({"retry": {}});
        let mode = parse_error_mode(&val, &json!({}));
        match mode {
            ErrorMode::Retry { max_retries, .. } => assert_eq!(max_retries, 3),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn error_mode_nested_retry_large_value() {
        let val = json!({"retry": {"max_retries": u64::MAX}});
        let mode = parse_error_mode(&val, &json!({}));
        match mode {
            ErrorMode::Retry { max_retries, .. } => assert_eq!(max_retries, 3),
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
}
