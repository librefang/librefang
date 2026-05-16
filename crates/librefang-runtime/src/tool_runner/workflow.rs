//! Workflow execution tools — run / list / status / start / cancel / describe.

use super::require_kernel;
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

/// Recursively walk `value` and rewrite every `{"_artifact": "sha256:..."}`
/// object into the bare handle string. Anything else passes through
/// unchanged. Malformed handles fail fast with a clear error message so
/// the agent's tool-result includes the bad reference and can self-correct
/// on the next turn — silently coercing or stripping would leave the
/// downstream step rendering `[object Object]` into its prompt.
pub(super) fn resolve_workflow_input_artifacts(
    value: &mut serde_json::Value,
) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            // Single-key `_artifact` reference — replace this whole node.
            if map.len() == 1 {
                if let Some(serde_json::Value::String(handle)) = map.get("_artifact") {
                    let handle = handle.clone();
                    // Validate the handle format via the artifact_store
                    // parser — same shape (`sha256:<64hex>`) read_artifact
                    // accepts, so the agent's existing handle vocabulary
                    // works without translation. The offending handle is
                    // interpolated so the agent gets enough context in
                    // its tool-result to self-correct on the next turn
                    // (`ArtifactHandle::parse` itself only quotes the
                    // suffix on the "wrong length" path, not the "wrong
                    // prefix" path — surfacing it unconditionally
                    // closes that gap).
                    crate::artifact_store::ArtifactHandle::parse(&handle).map_err(|e| {
                        format!("Invalid '_artifact' reference in workflow input: '{handle}' ({e})")
                    })?;
                    *value = serde_json::Value::String(handle);
                    return Ok(());
                }
            }
            for (_k, v) in map.iter_mut() {
                resolve_workflow_input_artifacts(v)?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                resolve_workflow_input_artifacts(v)?;
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
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    // Resolve any {"_artifact": "sha256:..."} references in the input
    // object before serializing for the workflow engine (#4982 — gap 3
    // / file & image input). See `resolve_workflow_input_artifacts`.
    let input_str = prepare_workflow_input(input.get("input"))?;

    let kh = require_kernel(kernel)?;
    let (run_id, output) = kh
        .run_workflow(workflow_id, &input_str)
        .await
        .map_err(|e| e.to_string())?;

    // Fetch the structured run summary so the caller gets {step_outputs,
    // output_json?} alongside the final output string (#4982 — gap 3 /
    // structured results). When the run vanished between completion and
    // this lookup (eviction past MAX_RETAINED_RUNS) we still return the
    // legacy {run_id, output} shape rather than failing the tool.
    let summary = kh.get_workflow_run(&run_id).await;
    Ok(build_workflow_run_result(&run_id, &output, summary.as_ref()).to_string())
}

pub(super) async fn tool_workflow_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let mut summaries = kh.list_workflows().await;
    // Sort by name for deterministic LLM prompt output (#3298).
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
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
    serde_json::to_string(&json_array)
        .map_err(|e| format!("Failed to serialize workflow list: {e}"))
}

pub(super) async fn tool_workflow_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;

    // Validate UUID format before calling kernel — returns a clear error
    // rather than silently returning not-found for a malformed id.
    uuid::Uuid::parse_str(run_id)
        .map_err(|_| format!("Invalid run_id — must be a UUID: {run_id}"))?;

    let kh = require_kernel(kernel)?;
    let summary = kh
        .get_workflow_run(run_id)
        .await
        .ok_or_else(|| format!("workflow run not found: {run_id}"))?;

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
    serde_json::to_string(&body).map_err(|e| format!("Failed to serialize workflow status: {e}"))
}

pub(super) async fn tool_workflow_start(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    caller_session_id: Option<librefang_types::agent::SessionId>,
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    // Resolve `_artifact` refs in the input object before serializing
    // (#4982 — gap 3 / file & image input).
    let input_str = prepare_workflow_input(input.get("input"))?;

    let kh = require_kernel(kernel)?;

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
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "run_id": run_id }).to_string())
}

pub(super) async fn tool_workflow_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;

    // Validate UUID format before calling kernel.
    uuid::Uuid::parse_str(run_id)
        .map_err(|_| format!("Invalid run_id — must be a UUID: {run_id}"))?;

    let kh = require_kernel(kernel)?;
    kh.cancel_workflow_run(run_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "run_id": run_id,
        "state": "cancelled",
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// workflow_describe — discover a workflow's input shape (#4982 — gap 2)
// ---------------------------------------------------------------------------

pub(super) async fn tool_workflow_describe(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let workflow_id = input["workflow_id"]
        .as_str()
        .ok_or("Missing 'workflow_id' parameter")?;

    let kh = require_kernel(kernel)?;
    let description = kh
        .describe_workflow(workflow_id)
        .await
        .ok_or_else(|| format!("workflow not found: {workflow_id}"))?;

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

    serde_json::to_string(&serde_json::json!({
        "id": description.id,
        "name": description.name,
        "description": description.description,
        "step_names": description.step_names,
        "input_schema": input_schema,
    }))
    .map_err(|e| format!("Failed to serialize workflow description: {e}"))
}
