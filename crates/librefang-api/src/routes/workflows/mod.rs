//! Workflow, trigger, schedule, and cron job handlers.

use super::AppState;

use crate::triggers::{Trigger, TriggerId, TriggerPatch, TriggerPattern};
use crate::workflow::{
    BranchArm, CancelRunError, ErrorMode, GateCondition, GateOp, PauseRunError, StepAgent,
    StepMode, Workflow, WorkflowId, WorkflowInputParam, WorkflowRun, WorkflowRunId,
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

mod cron;
mod schedules;
mod templates;
mod triggers;
mod workflow;

pub use cron::*;
pub use schedules::*;
pub use templates::*;
pub use triggers::*;
pub use workflow::*;

/// Extract the workflow run input string from a request body.
///
/// The workflow engine takes a single `input` string. When that string is
/// a JSON object, the workflow engine's `seed_input_vars_from_json`
/// explodes its top-level keys into `{{key}}` template variables (#4982's
/// contract), so a parameterised workflow whose steps reference
/// `{{challenge}}` resolves the value the user supplied instead of leaving
/// the literal placeholder in the prompt. This mirrors the runtime tool's
/// `prepare_workflow_input` so HTTP callers (the dashboard run / parameter
/// form) get the same per-key binding the `workflow_run` agent tool
/// already has.
///
/// Accepted `input` shapes:
/// - string → used verbatim (free-text `{{input}}`, backward-compatible)
/// - object → serialised to a JSON string so per-key `{{var}}` binding applies
/// - null / absent / other → empty string
fn workflow_run_input_string(req: &serde_json::Value) -> String {
    match req.get("input") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v @ serde_json::Value::Object(_)) => serde_json::to_string(v).unwrap_or_default(),
        _ => String::new(),
    }
}

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
            "/workflows/runs/{run_id}/rerun",
            axum::routing::post(rerun_workflow_run),
        )
        .route(
            "/workflows/runs/{run_id}/cancel",
            axum::routing::post(cancel_workflow_run),
        )
        .route(
            "/workflows/runs/{run_id}/pause",
            axum::routing::post(pause_workflow_run),
        )
        .route(
            "/workflows/runs/{run_id}/resume",
            axum::routing::post(resume_workflow_run),
        )
        .route(
            "/workflows/runs/{run_id}/operator",
            axum::routing::get(inspect_workflow_operator_pause)
                .post(operator_action_workflow_run),
        )
        .route(
            "/workflows/operator/pending",
            axum::routing::get(list_pending_operator_workflow_runs),
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
                "session_mode": serde_json::to_value(s.session_mode).unwrap_or(serde_json::Value::Null),
            })
        }).collect::<Vec<_>>(),
        "created_at": w.created_at.to_rfc3339(),
        "layout": w.layout,
        "total_timeout_secs": w.total_timeout_secs,
        "input_schema": w.input_schema.as_ref().map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null)),
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
            // Operator nodes (#4980). The flat-string forms read their
            // configuration from sibling fields on the step object —
            // mirrors the legacy `"conditional"` / `"loop"` shape, so
            // the dashboard / TOML examples in the issue body can keep
            // `mode = "wait"` with siblings `duration_secs = 5` etc.
            "wait" => {
                let duration_secs = step["duration_secs"].as_u64().unwrap_or_else(|| {
                    warn!("wait step missing 'duration_secs' field, defaulting to 0");
                    0
                });
                StepMode::Wait { duration_secs }
            }
            "gate" => {
                // The `condition` field is a typed comparator AST
                // (#4980 step 2). Parse it through serde so a malformed
                // shape (missing `op`, unknown operator, wrong types)
                // surfaces as a structured warn rather than silently
                // defaulting to a passing gate. We fail-closed on error
                // — `Eq` against `Value::Null` will fail any real input,
                // making the misconfiguration loud rather than silent.
                let condition: GateCondition = match step.get("condition") {
                    Some(c) => match serde_json::from_value(c.clone()) {
                        Ok(parsed) => parsed,
                        Err(e) => {
                            warn!(
                                "gate step 'condition' failed to parse: {e}; failing closed with Eq=null"
                            );
                            GateCondition {
                                field: None,
                                op: GateOp::Eq,
                                value: serde_json::Value::Null,
                            }
                        }
                    },
                    None => {
                        warn!("gate step missing 'condition' field; failing closed with Eq=null");
                        GateCondition {
                            field: None,
                            op: GateOp::Eq,
                            value: serde_json::Value::Null,
                        }
                    }
                };
                StepMode::Gate { condition }
            }
            "approval" => {
                let recipients: Vec<String> = step["recipients"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let timeout_secs = step["timeout_secs"].as_u64();
                StepMode::Approval {
                    recipients,
                    timeout_secs,
                }
            }
            "transform" => {
                let code = step["code"]
                    .as_str()
                    .unwrap_or_else(|| {
                        warn!("transform step missing 'code' field, defaulting to empty");
                        ""
                    })
                    .to_string();
                StepMode::Transform { code }
            }
            "branch" => {
                let arms: Vec<BranchArm> = step["arms"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                let then = v["then"].as_str()?.to_string();
                                let match_value = v.get("match_value").cloned()?;
                                Some(BranchArm { match_value, then })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                StepMode::Branch { arms }
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

/// Parse an optional per-step `session_mode` from a step JSON object.
///
/// Absent or null returns `None` so HTTP callers don't trip on minor schema
/// quirks; the kernel's resolver ("per-step > manifest > kernel default")
/// then falls back to the agent manifest or kernel default. Accepted values
/// for the field are `"persistent"` and `"new"` (the serde rename of
/// `SessionMode`). Malformed values (typos, wrong types) also fall back to
/// `None` but log a `WARN` so operators can spot a bad payload — silent
/// drop is the bug class this PR works to prevent.
fn parse_step_session_mode(
    step: &serde_json::Value,
) -> Option<librefang_types::agent::SessionMode> {
    let raw = step.get("session_mode")?;
    if raw.is_null() {
        return None;
    }
    match serde_json::from_value::<librefang_types::agent::SessionMode>(raw.clone()) {
        Ok(mode) => Some(mode),
        Err(err) => {
            tracing::warn!(
                field = ?raw,
                error = %err,
                "ignoring malformed session_mode on workflow step; expected \"persistent\" or \"new\""
            );
            None
        }
    }
}

/// Hard cap on declared workflow input parameters.
///
/// A workflow with hundreds of declared input parameters is almost
/// certainly malformed or attacker-crafted; the dashboard
/// parameter-discovery UI is unusable past a few dozen anyway. Bounds
/// the `Vec::with_capacity(arr.len())` allocation in
/// [`parse_input_schema`] below so a hostile
/// `"input_schema": [{}, {}, ...]` array within the 8 MiB body cap
/// cannot pre-allocate millions of entries
/// (`docs/issues/bulk-with-capacity-no-validate.md`).
const MAX_INPUT_SCHEMA_PARAMS: usize = 100;

/// Parse the optional `input_schema` JSON field on a workflow payload
/// (#4982 — gap 2 / parameter discovery).
///
/// Accepts:
/// - `None` / absent / explicit `null` → returns `None` (workflow has no
///   declared schema; the `workflow_describe` tool will auto-detect from
///   `{{var}}` placeholders).
/// - Empty array `[]` → returns `None` (no parameters declared).
/// - Array of param objects → returns `Some(vec)`. Each malformed entry
///   logs a `WARN` and is skipped rather than failing the whole request,
///   matching the lenient style of `parse_step_session_mode`. The kernel
///   stores whatever survives.
///
/// **Absent-vs-empty caveat:** the `None` return collapses three distinct
/// caller intents — "field absent in JSON", "explicit `null`", and
/// "explicit empty array `[]`". The PUT (`update_workflow`) handler can
/// therefore NOT distinguish "remove the schema entirely" from "set to an
/// empty list"; both clear `input_schema` on the persisted workflow. This
/// is acceptable because an empty schema is semantically a workflow with
/// no declared parameters — identical to "no schema declared" — so the
/// dashboard / agent surface behaves the same in either case. Callers
/// that need to *preserve* the existing schema MUST omit the key from
/// the PUT body entirely (see the "PATCH-style" branch in
/// `update_workflow`). If a future API ever needs to distinguish these
/// three states, change the return shape (e.g. `Result<Option<_>, _>`
/// or a custom three-state enum) and update both POST and PUT handlers.
fn parse_input_schema(val: Option<&serde_json::Value>) -> Option<Vec<WorkflowInputParam>> {
    let v = val?;
    if v.is_null() {
        return None;
    }
    let arr = v.as_array()?;
    if arr.is_empty() {
        return None;
    }
    // Cap the allocation BEFORE `Vec::with_capacity`. The parser is
    // lenient by design (`parse_step_session_mode` style — log + skip
    // malformed entries rather than failing the whole workflow), so an
    // oversize array is treated the same way: log a warning, take the
    // first `MAX_INPUT_SCHEMA_PARAMS` entries, and continue. Callers
    // that need stricter rejection can validate up front in the
    // top-level handler.
    let effective_len = arr.len().min(MAX_INPUT_SCHEMA_PARAMS);
    if arr.len() > MAX_INPUT_SCHEMA_PARAMS {
        warn!(
            requested = arr.len(),
            max = MAX_INPUT_SCHEMA_PARAMS,
            "input_schema exceeds maximum declared parameters; truncating",
        );
    }
    let mut params: Vec<WorkflowInputParam> = Vec::with_capacity(effective_len);
    for entry in arr.iter().take(effective_len) {
        match serde_json::from_value::<WorkflowInputParam>(entry.clone()) {
            Ok(p) => params.push(p),
            Err(err) => {
                warn!(
                    entry = ?entry,
                    error = %err,
                    "ignoring malformed input_schema entry on workflow payload",
                );
            }
        }
    }
    if params.is_empty() {
        None
    } else {
        Some(params)
    }
}

/// Render an [`OperatorPause`] paired with its run as the public JSON
/// shape the dashboard consumes. Centralised so the single-run inspector
/// and the worklist endpoint stay byte-identical per row — the dashboard
/// caches by run id and switching between the two surfaces should never
/// see a different shape for the same row.
fn operator_pause_row_json(
    run: &WorkflowRun,
    pause: &crate::workflow::OperatorPause,
) -> serde_json::Value {
    let paused_at = match &run.state {
        WorkflowRunState::Paused { paused_at, .. } => Some(paused_at.to_rfc3339()),
        _ => None,
    };
    serde_json::json!({
        "run_id": run.id.to_string(),
        "workflow_id": run.workflow_id.to_string(),
        "workflow_name": run.workflow_name,
        "step_name": pause.step_name,
        "operator_step_index": pause.operator_step_index,
        "artifact": pause.artifact,
        // Serialise actions through the existing serde derive so the wire
        // shape matches what the POST endpoint accepts (snake_case verbs;
        // `provide_input` carries the `field`).
        "actions": pause.actions.iter()
            .map(|a| serde_json::to_value(a).unwrap_or(serde_json::Value::Null))
            .collect::<Vec<_>>(),
        "started_at": run.started_at.to_rfc3339(),
        "paused_at": paused_at,
    })
}

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
/// `task_posted` (`assignee_match`), `task_claimed` and `task_completed`
/// (`creator_match`) are the struct variants with optional fields; extend
/// this match when other variants gain optional fields.
fn normalize_pattern_json(value: serde_json::Value) -> serde_json::Value {
    match value.as_str() {
        Some(tag @ ("task_posted" | "task_claimed" | "task_completed")) => {
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
