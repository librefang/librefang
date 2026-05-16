//! Rich workflow invocation tests (#4982).
//!
//! Parameter discovery, file/image input via `_artifact` refs, and
//! structured `workflow_run` / `workflow_status` results. These tests pin
//! the wire shapes the agent sees so a future refactor cannot silently
//! change what the LLM reads.

use super::*;

// --- workflow_describe surface ---------------------------------------

#[test]
fn workflow_describe_is_registered_in_builtins() {
    let defs = builtin_tool_definitions();
    assert!(
        defs.iter().any(|d| d.name == "workflow_describe"),
        "workflow_describe must be exposed to the agent"
    );
}

#[test]
fn workflow_describe_schema_requires_workflow_id() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_describe")
        .expect("workflow_describe definition");
    assert_eq!(def.input_schema["type"], "object");
    let required = def.input_schema["required"]
        .as_array()
        .expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("workflow_id")));
}

#[test]
fn workflow_list_definition_advertises_has_input_schema_field() {
    // The agent reads workflow_list's description to know whether
    // calling workflow_describe is worthwhile. The descriptor text
    // is a load-bearing surface — assert the cue stays in place.
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_list")
        .expect("workflow_list definition");
    assert!(
        def.description.contains("has_input_schema"),
        "workflow_list description must mention has_input_schema so \
         the agent knows when to call workflow_describe"
    );
}

// --- _artifact reference resolution (#4982 — gap 3) ------------------

#[test]
fn artifact_ref_at_top_level_is_rewritten_to_handle_string() {
    let mut value = serde_json::json!({
        "cover_image": {
            "_artifact": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }
    });
    resolve_workflow_input_artifacts(&mut value).expect("valid handle");
    let s = value["cover_image"]
        .as_str()
        .expect("cover_image is now a string");
    assert!(s.starts_with("sha256:"), "got: {s}");
}

#[test]
fn artifact_ref_nested_in_array_is_rewritten() {
    let mut value = serde_json::json!({
        "attachments": [
            {"_artifact": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "plain string passes through",
            {"_artifact": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
        ]
    });
    resolve_workflow_input_artifacts(&mut value).expect("valid handles");
    let arr = value["attachments"].as_array().expect("attachments array");
    assert_eq!(
        arr[0].as_str().unwrap(),
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(arr[1].as_str().unwrap(), "plain string passes through");
    assert_eq!(
        arr[2].as_str().unwrap(),
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    );
}

#[test]
fn artifact_ref_with_invalid_handle_returns_clear_error() {
    let bad_handle = "not-a-valid-handle";
    let mut value = serde_json::json!({
        "image": {"_artifact": bad_handle}
    });
    let err = resolve_workflow_input_artifacts(&mut value).expect_err("malformed handle");
    assert!(
        err.contains("Invalid '_artifact'"),
        "error must say 'Invalid _artifact', got: {err}"
    );
    // The handle parser's error message is interpolated into the
    // outer error via the `format!(..., e)` at the call site
    // (see `resolve_workflow_input_artifacts`). Pinning the
    // offending value here catches a future refactor that drops
    // the inner error from the surfaced string — the agent uses
    // that string to self-correct on the next turn.
    assert!(
        err.contains(bad_handle),
        "error must include the offending handle string '{bad_handle}', got: {err}"
    );
}

#[test]
fn object_with_extra_keys_alongside_artifact_is_not_collapsed() {
    // {"_artifact": "...", "caption": "..."} is NOT a single-key
    // ref; pass through unchanged (caller can recurse into it but
    // the top-level object stays an object). This protects against
    // accidentally swallowing well-formed payloads that just happen
    // to contain the `_artifact` key alongside metadata.
    let mut value = serde_json::json!({
        "image": {
            "_artifact": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "caption": "a photo"
        }
    });
    resolve_workflow_input_artifacts(&mut value).expect("recursion only");
    // The object survives because it has 2 keys.
    assert!(value["image"].is_object());
}

#[test]
fn prepare_workflow_input_serializes_resolved_artifact_into_handle_string() {
    let input = serde_json::json!({
        "topic": "quantum",
        "cover": {"_artifact": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}
    });
    let serialized = prepare_workflow_input(Some(&input)).expect("valid input");
    // The resolved JSON must carry the bare handle string, not an
    // object — that's the form the workflow engine's {{var}}
    // substitution will splice into the step prompt.
    let parsed: serde_json::Value = serde_json::from_str(&serialized).expect("valid JSON output");
    assert_eq!(parsed["topic"], "quantum");
    assert_eq!(
        parsed["cover"].as_str().unwrap(),
        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    );
}

#[test]
fn prepare_workflow_input_rejects_non_object_input() {
    assert!(prepare_workflow_input(Some(&serde_json::json!("oops"))).is_err());
    assert!(prepare_workflow_input(Some(&serde_json::json!(42))).is_err());
}

#[test]
fn prepare_workflow_input_treats_absent_and_null_as_empty_string() {
    assert_eq!(prepare_workflow_input(None).unwrap(), "");
    assert_eq!(
        prepare_workflow_input(Some(&serde_json::Value::Null)).unwrap(),
        ""
    );
}

// --- structured workflow_run / workflow_status result shape ----------

#[test]
fn workflow_run_result_includes_output_json_when_output_parses() {
    let body = build_workflow_run_result("run-123", "{\"answer\": 42}", None);
    assert_eq!(body["run_id"], "run-123");
    assert_eq!(body["output"], "{\"answer\": 42}");
    assert_eq!(
        body["output_json"]["answer"], 42,
        "structured output must round-trip"
    );
}

#[test]
fn workflow_run_result_omits_output_json_for_plain_text() {
    let body = build_workflow_run_result("run-456", "just text", None);
    assert_eq!(body["run_id"], "run-456");
    assert_eq!(body["output"], "just text");
    assert!(
        body.get("output_json").is_none(),
        "plain-text output must not surface output_json"
    );
}

#[test]
fn workflow_run_result_carries_step_outputs_when_summary_present() {
    use librefang_kernel_handle::{StepOutputSummary, WorkflowRunSummary};
    let summary = WorkflowRunSummary::new(
        "run-789".to_string(),
        "wf-id".to_string(),
        "demo".to_string(),
        "completed".to_string(),
        "2026-01-01T00:00:00+00:00".to_string(),
        Some("2026-01-01T00:00:05+00:00".to_string()),
        Some("final".to_string()),
        None,
        2,
        Some("summarise".to_string()),
        vec![
            StepOutputSummary::new("analyse".to_string(), "draft analysis".to_string()),
            StepOutputSummary::new("summarise".to_string(), "final".to_string()),
        ],
    );
    let body = build_workflow_run_result("run-789", "final", Some(&summary));
    let steps = body["step_outputs"].as_array().expect("step_outputs array");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["step_name"], "analyse");
    assert_eq!(steps[0]["output"], "draft analysis");
    assert_eq!(steps[1]["step_name"], "summarise");
    assert_eq!(steps[1]["output"], "final");
}

#[test]
fn workflow_run_result_without_summary_keeps_legacy_shape() {
    // Eviction-during-completion race: the kernel completed the
    // workflow, returned the output, then evicted the run before the
    // runtime looked it up. We still ship a usable response.
    let body = build_workflow_run_result("run-evicted", "ok", None);
    assert_eq!(body["run_id"], "run-evicted");
    assert_eq!(body["output"], "ok");
    assert!(body.get("step_outputs").is_none());
}
