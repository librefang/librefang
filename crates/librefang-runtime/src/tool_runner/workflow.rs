//! Workflow execution tools — run / list / status / start / cancel / describe.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). Missing params -> `MissingParameter`; bad `input` object / bad
//! `_artifact` ref / non-UUID run_id -> `InvalidParameter`; unknown run/workflow
//! -> `NotFound`; kernel ops (`KernelOpError`) -> `ToolError::upstream`; JSON
//! serialization via `?`. The `prepare_workflow_input` /
//! `resolve_workflow_input_artifacts` / `build_workflow_run_result` helpers keep
//! their `Result<_, String>` / infallible shapes (shared, unit-tested directly).

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

/// Validate the optional `input` field on workflow_run / workflow_start
/// payloads and serialize it to the JSON-string form the workflow engine
/// expects.
///
/// Accepted shapes:
/// - absent / `null` → empty string (no parameters)
/// - JSON object → serialized after resolving any nested `_artifact`
///   references via [`resolve_workflow_input_artifacts`]
/// - anything else → `Err`
///
/// Centralised so `workflow_run` and `workflow_start` share one parse +
/// resolution code path (#4982 — gap 3 / file & image input). The agent
/// can pass `{"cover": {"_artifact": "sha256:<64hex>"}}` and the engine
/// receives `{"cover": "sha256:<64hex>"}` ready for `{{cover}}` template
/// substitution into a step prompt.
pub(super) fn prepare_workflow_input(raw: Option<&serde_json::Value>) -> Result<String, String> {
    match raw {
        Some(v) if v.is_object() => {
            let mut value = v.clone();
            resolve_workflow_input_artifacts(&mut value)?;
            serde_json::to_string(&value)
                .map_err(|e| format!("Failed to serialize workflow input: {e}"))
        }
        Some(v) if v.is_null() => Ok(String::new()),
        Some(_) => Err("'input' must be a JSON object or null".to_string()),
        None => Ok(String::new()),
    }
}

pub(super) fn resolve_workflow_input_artifacts(
    value: &mut serde_json::Value,
) -> Result<(), String> {
    const MAX_DEPTH: usize = 128;
    resolve_workflow_input_artifacts_impl(value, 0, MAX_DEPTH)
}

fn resolve_workflow_input_artifacts_impl(
    value: &mut serde_json::Value,
    depth: usize,
    max_depth: usize,
) -> Result<(), String> {
    if depth > max_depth {
        return Err("JSON nesting exceeds maximum depth".to_string());
    }
    match value {
        serde_json::Value::Object(map) => {
            if map.len() == 1 {
                if let Some(av) = map.get("_artifact") {
                    if let Some(handle) = av.as_str() {
                        let handle = handle.to_string();
                        crate::artifact_store::ArtifactHandle::parse(&handle).map_err(|e| {
                            format!(
                                "Invalid '_artifact' reference in workflow input: '{handle}' ({e})"
                            )
                        })?;
                        *value = serde_json::Value::String(handle);
                        return Ok(());
                    } else {
                        return Err("'_artifact' value must be a string".to_string());
                    }
                }
            }
            for (_k, v) in map.iter_mut() {
                resolve_workflow_input_artifacts_impl(v, depth + 1, max_depth)?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                resolve_workflow_input_artifacts_impl(v, depth + 1, max_depth)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Build the structured JSON body returned by `workflow_run`. Carries the
/// final `output` string plus `step_outputs` for stage navigation and
/// `output_json` when the final-step output parses as JSON (#4982 — gap 3
/// / structured results). When `summary` is `None` (run evicted between
/// completion and lookup), falls back to the legacy `{run_id, output}`
/// shape so the tool surface stays robust.
pub(super) fn build_workflow_run_result(
    run_id: &str,
    output: &str,
    summary: Option<&librefang_kernel_handle::WorkflowRunSummary>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "run_id": run_id,
        "output": output,
    });
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
        body["output_json"] = parsed;
    }
    if let Some(s) = summary {
        let step_outputs: Vec<serde_json::Value> = s
            .step_outputs
            .iter()
            .map(|so| {
                serde_json::json!({
                    "step_name": so.step_name,
                    "output": so.output,
                })
            })
            .collect();
        body["step_outputs"] = serde_json::Value::Array(step_outputs);
    }
    body
}

pub(super) async fn tool_workflow_run(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("workflow_id"))?;

    // Resolve any {"_artifact": "sha256:..."} references in the input
    // object before serializing for the workflow engine (#4982 — gap 3
    // / file & image input). See `resolve_workflow_input_artifacts`.
    let input_str = prepare_workflow_input(input.get("input")).map_err(|reason| {
        ToolError::InvalidParameter {
            name: "input",
            reason,
        }
    })?;

    let kh = require_kernel_typed(kernel)?;
    // A nesting-depth refusal arrives as `CapabilityDenied` (refs #6659) and must stay a policy error rather than becoming `Upstream`.
    // Two reasons, the same ones spelled out in `tool_agent_send`: `Upstream` lifts to a 5xx-class `ToolExecution` that reads as a downstream crash to retry logic, and `PermissionDenied` classifies as `ToolExecutionStatus::Denied` — a soft failure — so a capped agent that keeps trying does not burn through `MAX_CONSECUTIVE_ALL_FAILED` and lose the turn to an abort.
    // Every other kernel failure stays `Upstream`.
    let (run_id, output) = kh
        .run_workflow(workflow_id, &input_str)
        .await
        .map_err(|e| match e {
            librefang_types::error::LibreFangError::CapabilityDenied(msg) => {
                ToolError::PermissionDenied(msg)
            }
            other => ToolError::upstream(other),
        })?;

    // Fetch the structured run summary so the caller gets {step_outputs,
    // output_json?} alongside the final output string (#4982 — gap 3 /
    // structured results). When the run vanished between completion and
    // this lookup (eviction past MAX_RETAINED_RUNS) we still return the
    // legacy {run_id, output} shape rather than failing the tool.
    let summary = kh.get_workflow_run(&run_id).await;
    Ok(build_workflow_run_result(&run_id, &output, summary.as_ref()).to_string())
}

pub(super) async fn tool_workflow_list(kernel: Option<&Arc<dyn KernelHandle>>) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let mut summaries = kh.list_workflows().await;
    summaries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let json_array: Vec<serde_json::Value> = summaries
        .into_iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "name": w.name,
                "description": w.description,
                "step_count": w.step_count,
                "has_input_schema": w.has_input_schema,
            })
        })
        .collect();
    Ok(serde_json::to_string(&json_array)?)
}

pub(super) async fn tool_workflow_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let run_id = input["run_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("run_id"))?;

    // Validate UUID format before calling kernel — returns a clear error
    // rather than silently returning not-found for a malformed id.
    uuid::Uuid::parse_str(run_id).map_err(|_| ToolError::InvalidParameter {
        name: "run_id",
        reason: format!("must be a UUID: {run_id}"),
    })?;

    let kh = require_kernel_typed(kernel)?;
    let summary = kh
        .get_workflow_run(run_id)
        .await
        .ok_or_else(|| ToolError::NotFound {
            kind: "Workflow run",
            id: run_id.to_string(),
        })?;

    // Mirror the run_workflow tool's structured shape: alongside the raw
    // `output` string, surface a parsed `output_json` when applicable and
    // a trimmed `step_outputs` array so the agent can navigate stage
    // results without re-fetching (#4982 — gap 3 / structured results).
    let output_json = summary
        .output
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let step_outputs: Vec<serde_json::Value> = summary
        .step_outputs
        .iter()
        .map(|s| {
            serde_json::json!({
                "step_name": s.step_name,
                "output": s.output,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "run_id": summary.run_id,
        "workflow_id": summary.workflow_id,
        "workflow_name": summary.workflow_name,
        "state": summary.state,
        "started_at": summary.started_at,
        "completed_at": summary.completed_at,
        "output": summary.output,
        "error": summary.error,
        "step_count": summary.step_count,
        "last_step_name": summary.last_step_name,
        "step_outputs": step_outputs,
    });
    if let Some(json) = output_json {
        body["output_json"] = json;
    }
    Ok(serde_json::to_string(&body)?)
}

pub(super) async fn tool_workflow_start(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    caller_session_id: Option<librefang_types::agent::SessionId>,
) -> ToolResult {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("workflow_id"))?;

    // Resolve `_artifact` refs in the input object before serializing
    // (#4982 — gap 3 / file & image input).
    let input_str = prepare_workflow_input(input.get("input")).map_err(|reason| {
        ToolError::InvalidParameter {
            name: "input",
            reason,
        }
    })?;

    let kh = require_kernel_typed(kernel)?;

    // Forward caller context so the kernel can register the workflow on
    // its async-task tracker (#4983) and inject a `TaskCompletionEvent`
    // into the originating session when the run finishes. Falls back to
    // the historical fire-and-forget behaviour when either id is
    // missing (legacy / test call sites that don't carry context).
    let session_id_str = caller_session_id.map(|sid| sid.0.to_string());
    let run_id = kh
        .start_workflow_async_tracked(
            workflow_id,
            &input_str,
            caller_agent_id,
            session_id_str.as_deref(),
        )
        .await
        .map_err(ToolError::upstream)?;

    Ok(serde_json::json!({ "run_id": run_id }).to_string())
}

pub(super) async fn tool_workflow_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let run_id = input["run_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("run_id"))?;

    // Validate UUID format before calling kernel.
    uuid::Uuid::parse_str(run_id).map_err(|_| ToolError::InvalidParameter {
        name: "run_id",
        reason: format!("must be a UUID: {run_id}"),
    })?;

    let kh = require_kernel_typed(kernel)?;
    match kh.cancel_workflow_run(run_id).await {
        Ok(()) => Ok(serde_json::json!({
            "run_id": run_id,
            "state": "cancelled",
        })
        .to_string()),
        Err(e) => {
            let msg = e.to_string();
            let state = if msg.contains("not found") {
                "not_found"
            } else if msg.contains("already") {
                "already_terminal"
            } else {
                "error"
            };
            Err(ToolError::upstream_msg(format!(
                "cancel failed ({state}): {msg}"
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// workflow_describe — discover a workflow's input shape (#4982 — gap 2)
// ---------------------------------------------------------------------------

pub(super) async fn tool_workflow_describe(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("workflow_id"))?;

    let kh = require_kernel_typed(kernel)?;
    let description =
        kh.describe_workflow(workflow_id)
            .await
            .ok_or_else(|| ToolError::NotFound {
                kind: "Workflow",
                id: workflow_id.to_string(),
            })?;

    let input_schema: Vec<serde_json::Value> = description
        .input_schema
        .iter()
        .map(|p| {
            let mut entry = serde_json::json!({
                "name": p.name,
                "param_type": p.param_type,
                "required": p.required,
            });
            if let Some(desc) = &p.description {
                entry["description"] = serde_json::Value::String(desc.clone());
            }
            entry
        })
        .collect();

    Ok(serde_json::to_string(&serde_json::json!({
        "id": description.id,
        "name": description.name,
        "description": description.description,
        "step_names": description.step_names,
        "input_schema": input_schema,
    }))?)
}

/// Ceiling on the number of steps a single `workflow_create` call may register (#6943 review).
///
/// `workflow_create` sits in `ALWAYS_NATIVE_TOOLS`, so every agent has it regardless of configuration, and neither the tool's JSON schema nor the workflow engine enforced any ceiling on step count or timeout length.
/// A schema `maxItems` / `maximum` is advisory only — the model driving the call can simply ignore it — so the real cap has to live here, in the handler that actually persists the workflow.
/// Without it a single call could register an arbitrarily large step list, or a `total_timeout_secs` up to the one-year panic-safety clamp in `librefang_kernel::workflow`, tying up engine-tracked state and an async-task slot for as long as the caller likes.
pub(crate) const MAX_WORKFLOW_STEPS: usize = 50;

/// Per-step wall-clock ceiling enforced at creation time. One hour is generous for any legitimate step.
pub(crate) const MAX_STEP_TIMEOUT_SECS: u64 = 60 * 60;

/// Whole-workflow wall-clock ceiling enforced at creation time.
pub(crate) const MAX_TOTAL_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// Normalize one `input_schema` entry to the field names `WorkflowInputParam` actually deserializes.
///
/// The tool schema documents the parameter's type under `param_type`, matching what `workflow_describe` emits, so an agent can round-trip a described workflow back through `workflow_create`.
/// `type` is accepted as a tolerated alias because it is the spelling the schema advertised before this fix and it is the spelling an LLM reaches for by analogy with JSON Schema.
/// Without the remap the alias deserialized into nothing and `param_type` fell back to its `"string"` default, so every declared `number` / `boolean` / `file` / `image` / `agent_id` parameter was silently downgraded (#6943 review).
fn normalize_input_param(param: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
    let Some(obj) = param.as_object() else {
        return Err(ToolError::InvalidParameter {
            name: "input_schema",
            reason: "each input_schema entry must be an object".to_string(),
        });
    };

    let mut out = obj.clone();
    // `param_type` wins when both spellings are present; the alias is only a fallback.
    if let Some(alias) = out.remove("type") {
        out.entry("param_type").or_insert(alias);
    }

    if let Some(param_type) = out.get("param_type") {
        let Some(param_type) = param_type.as_str() else {
            return Err(ToolError::InvalidParameter {
                name: "input_schema",
                reason: "param_type must be a string".to_string(),
            });
        };
        if !WORKFLOW_INPUT_PARAM_TYPES.contains(&param_type) {
            return Err(ToolError::InvalidParameter {
                name: "input_schema",
                reason: format!(
                    "unknown param_type '{param_type}' (expected one of {})",
                    WORKFLOW_INPUT_PARAM_TYPES.join(", ")
                ),
            });
        }
    }

    Ok(serde_json::Value::Object(out))
}

/// The parameter types `WorkflowInputParam::param_type` recognizes, in the order the tool schema lists them.
pub(crate) const WORKFLOW_INPUT_PARAM_TYPES: &[&str] =
    &["string", "number", "boolean", "file", "image", "agent_id"];

pub(super) async fn tool_workflow_create(
    input: &serde_json::Value,
    kernel: Option<&std::sync::Arc<dyn crate::kernel_handle::KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let description = input["description"].as_str().unwrap_or("");

    // The canonical rule lives in `librefang_types::naming` so this check, the kernel-side one in `create_workflow`, and the API's `validate_template_name` cannot drift apart (#6943 review).
    librefang_types::naming::validate_resource_name(name).map_err(|e| {
        ToolError::InvalidParameter {
            name: "name",
            reason: format!("Workflow name {e}"),
        }
    })?;

    let steps = input["steps"]
        .as_array()
        .ok_or(ToolError::InvalidParameter {
            name: "steps",
            reason: "steps must be an array".to_string(),
        })?;

    if steps.is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "steps",
            reason: "Workflow must have at least one step".to_string(),
        });
    }
    if steps.len() > MAX_WORKFLOW_STEPS {
        return Err(ToolError::InvalidParameter {
            name: "steps",
            reason: format!(
                "Workflow may have at most {MAX_WORKFLOW_STEPS} steps (got {})",
                steps.len()
            ),
        });
    }
    for step in steps {
        if let Some(t) = step.get("timeout_secs").and_then(|v| v.as_u64()) {
            if t > MAX_STEP_TIMEOUT_SECS {
                return Err(ToolError::InvalidParameter {
                    name: "steps",
                    reason: format!(
                        "step timeout_secs must be at most {MAX_STEP_TIMEOUT_SECS}s (got {t}s)"
                    ),
                });
            }
        }
    }
    if let Some(t) = input.get("total_timeout_secs").and_then(|v| v.as_u64()) {
        if t > MAX_TOTAL_TIMEOUT_SECS {
            return Err(ToolError::InvalidParameter {
                name: "total_timeout_secs",
                reason: format!(
                    "total_timeout_secs must be at most {MAX_TOTAL_TIMEOUT_SECS}s (got {t}s)"
                ),
            });
        }
    }

    let input_schema = match input.get("input_schema") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(params)) => Some(
            params
                .iter()
                .map(normalize_input_param)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => {
            return Err(ToolError::InvalidParameter {
                name: "input_schema",
                reason: "input_schema must be an array".to_string(),
            })
        }
    };

    // Field-for-field the `Workflow` struct `create_workflow` deserializes — see the trait doc on `WorkflowRunner::create_workflow` for why that is not the same thing as the `POST /api/workflows` body.
    let workflow_json = serde_json::json!({
        "name": name,
        "description": description,
        "steps": steps,
        "input_schema": input_schema,
        "total_timeout_secs": input.get("total_timeout_secs"),
    });

    let workflow_json_str = serde_json::to_string(&workflow_json)
        .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

    kh.create_workflow(&workflow_json_str, caller_agent_id)
        .await
        .map_err(ToolError::upstream)
}
