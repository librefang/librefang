//! Integration tests for workflow operator-node step modes (#4980 step
//! 1/N → step 4/N).
//!
//! Five new `StepMode` variants total:
//!
//! * `Wait` — fully wired: sleeps for `duration_secs`, emits a structured
//!   `info!` log, returns success. Cancellation-aware via the run's
//!   `cancel_notify`. (step 1)
//! * `Gate` — fully wired since step 2: a declarative comparator AST
//!   (`{field, op, value}`) evaluated against the previous step's
//!   output. Passing condition routes onwards; failing condition halts
//!   the run with a recorded reason; a malformed condition surfaces a
//!   serde deserialisation error at manifest-load time. The
//!   string-DSL alternative was rejected because it would have forced
//!   a one-shot wire-format commitment incompatible with a future
//!   richer expression language.
//! * `Transform` — fully wired since step 3: Tera templates rendered
//!   against the previous step's output (exposed as `prev` and, when
//!   the output parses as JSON, `prev_json`) plus the workflow's
//!   `vars` map. Syntax errors surface at manifest-load time via
//!   `Workflow::validate`; render errors halt the run with a recorded
//!   reason. Tera picked over `mlua` / `rhai` / a hand-rolled DSL
//!   because it ships sandboxed by default and is the smallest
//!   addition to the dependency tree.
//! * `Branch` — fully wired since step 4: exact-match dispatch.
//!   Previous step output is JSON-parsed when possible and compared
//!   against each arm's `match_value`; the first matching arm's
//!   `then` field names a later step the dispatcher forward-jumps
//!   to. No arm matches → halt with a recorded reason; target step
//!   missing or at/before the current index → halt with a typed
//!   reason (backward jumps forbidden — `Loop` exists for that
//!   semantic). Range / regex / in-set matchers will land as
//!   additive `BranchArm` fields in a follow-up.
//! * `Approval` — no-op-with-warn; blocked on #4983 (async-task
//!   tracker). The executor will wire there once the dependency lands.
//!
//! The tests run the workflow engine directly (no HTTP) via
//! `kernel.workflow_engine().execute_run(...)` with a mock
//! `agent_resolver` / `send_message` pair — matching the kernel-only
//! pattern used by `workflow_pause_resume_test.rs::resume_with_wrong_token_returns_401`.
//! No agent is dispatched for operator nodes, so the mock sender is
//! never invoked on operator-node paths; we assert that fact by making
//! the mock panic on call.

use librefang_kernel::workflow::{
    BranchArm, ErrorMode, GateCondition, GateOp, StepAgent, StepMode, Workflow, WorkflowId,
    WorkflowRunState, WorkflowStep, MAX_TRANSFORM_OUTPUT_BYTES, MAX_WAIT_SECS,
};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, SessionMode};

/// Boot a minimal AppState for engine-level testing. The HTTP router is
/// not needed here; we drive the engine directly.
fn boot() -> TestAppState {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let config_path = test.tmp_path().join("config.toml");
    test.with_config_path(config_path)
}

/// Build a single-step workflow whose only step uses the given operator
/// `mode`. The placeholder `agent` is never consulted by operator-node
/// executors, but the `WorkflowStep` field is required syntactically.
fn workflow_with_op_step(name: &str, mode: StepMode) -> Workflow {
    Workflow {
        id: WorkflowId::new(),
        name: name.to_string(),
        description: "operator-node integration test".to_string(),
        steps: vec![WorkflowStep {
            name: "op_step".to_string(),
            agent: StepAgent::ByName {
                name: "_operator_placeholder".to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode,
            timeout_secs: 120,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        }],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    }
}

/// Resolver closure that panics on call. Operator-node executors must
/// NEVER call `agent_resolver`; this enforces the contract.
fn panicking_agent_resolver(_agent: &StepAgent) -> Option<(AgentId, String, bool)> {
    panic!("operator-node executor must not call agent_resolver");
}

// ---------------------------------------------------------------------------
// `Wait` — fully wired
// ---------------------------------------------------------------------------

/// A workflow whose only step is `Wait { duration_secs: 1 }` completes
/// successfully after roughly 1 second. We assert:
///   * The run state transitions to Completed.
///   * The recorded step result carries the `_operator:wait` synthetic
///     agent name, an empty `agent_id` (no agent ran), and a `duration_ms`
///     ≥ 950ms (lower-bound only — the upper bound is intentionally
///     loose to keep the test non-flaky under CI load).
///   * Neither the agent resolver nor the message sender was invoked.
#[tokio::test(flavor = "multi_thread")]
async fn wait_step_completes_after_duration_and_skips_agent_dispatch() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step("wait-1s", StepMode::Wait { duration_secs: 1 });
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "seed input".to_string())
        .await
        .expect("create_run");

    let started = std::time::Instant::now();
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    assert!(result.is_ok(), "Wait step must succeed: {result:?}");
    assert!(
        elapsed_ms >= 950,
        "Wait(1s) must take at least ~1s; got {elapsed_ms}ms"
    );

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(
        matches!(run.state, WorkflowRunState::Completed),
        "run must be Completed, got {:?}",
        run.state
    );
    assert_eq!(run.step_results.len(), 1, "exactly one step recorded");
    let sr = &run.step_results[0];
    assert_eq!(sr.step_name, "op_step");
    assert_eq!(sr.agent_id, "", "operator nodes have no agent_id");
    assert_eq!(sr.agent_name, "_operator:wait");
    assert_eq!(sr.input_tokens, 0, "Wait burns zero tokens");
    assert_eq!(sr.output_tokens, 0, "Wait burns zero tokens");
    assert!(
        sr.duration_ms >= 950,
        "step duration_ms must reflect the sleep; got {}",
        sr.duration_ms
    );
    // current_input passes through unchanged so downstream {{input}} keeps working
    assert_eq!(sr.output, "seed input", "Wait must preserve current_input");
}

/// `Wait { 0 }` is a degenerate but legal config: completes immediately
/// without panicking on the zero-duration sleep, still records a step
/// result, still does not dispatch an agent.
#[tokio::test(flavor = "multi_thread")]
async fn wait_step_zero_duration_completes_immediately() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step("wait-0s", StepMode::Wait { duration_secs: 0 });
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "seed".to_string())
        .await
        .expect("create_run");

    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result.is_ok(), "Wait(0) must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
}

// ---------------------------------------------------------------------------
// `Approval` / `Transform` / `Branch` — no-op-with-warn for V1
// ---------------------------------------------------------------------------
//
// These three log a structured `warn!` and return success. We can't
// easily capture `tracing` output from within an integration test
// without pulling in a subscriber dependency, so each test asserts the
// observable behaviour: the run completes successfully, exactly one
// step result is recorded with the matching `_operator:<kind>` agent
// name, and no agent was dispatched (mock resolver / sender would
// panic). The "not yet implemented" warn-log itself is exercised
// manually when the file is run with `RUST_LOG=warn cargo test ...`.

// ---------------------------------------------------------------------------
// `Gate` — fully wired in #4980 step 2/N
// ---------------------------------------------------------------------------

/// A Gate whose comparator passes against the previous step's output
/// must route execution onwards: the run state is `Completed`, the
/// step result carries `_operator:gate` as the synthetic agent name,
/// and `current_input` flows through unchanged so downstream
/// `{{input}}` substitutions still see the producing step's output.
#[tokio::test(flavor = "multi_thread")]
async fn gate_step_passes_and_routes_onwards() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "gate-pass",
        StepMode::Gate {
            condition: GateCondition {
                field: Some("/score".to_string()),
                op: GateOp::Gt,
                value: serde_json::json!(0.8),
            },
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, r#"{"score": 0.95}"#.to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result.is_ok(), "Gate pass must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    assert_eq!(run.step_results[0].agent_name, "_operator:gate");
    assert_eq!(run.step_results[0].input_tokens, 0);
    assert_eq!(run.step_results[0].output_tokens, 0);
    assert_eq!(
        run.step_results[0].output, r#"{"score": 0.95}"#,
        "Gate must preserve current_input on pass"
    );
}

/// A Gate whose comparator fails halts the run with `Failed` state and
/// a human-readable error referencing the gate name. The
/// `step_results` history still carries the gate step (so the operator
/// can see *which* gate blocked the run in the dashboard) and its
/// `output` field carries the failure reason rather than the
/// previous-step output.
#[tokio::test(flavor = "multi_thread")]
async fn gate_step_fails_and_halts_workflow_with_recorded_reason() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "gate-block",
        StepMode::Gate {
            condition: GateCondition {
                field: Some("/score".to_string()),
                op: GateOp::Gt,
                value: serde_json::json!(0.8),
            },
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, r#"{"score": 0.4}"#.to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("Gate must halt failing runs");
    assert!(
        err.contains("Gate step 'op_step' blocked workflow"),
        "halt error must name the gate; got: {err}"
    );

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(
        matches!(run.state, WorkflowRunState::Failed),
        "run must be Failed, got {:?}",
        run.state
    );
    let recorded_err = run.error.as_deref().unwrap_or("");
    assert!(
        recorded_err.contains("Gate step 'op_step' blocked workflow"),
        "recorded run.error must carry the gate halt reason; got: {recorded_err}"
    );
    assert_eq!(
        run.step_results.len(),
        1,
        "the blocking gate step must still appear in run history"
    );
    let sr = &run.step_results[0];
    assert_eq!(sr.agent_name, "_operator:gate");
    assert!(
        sr.output.contains("gate condition failed"),
        "step_result.output must surface the comparator failure; got: {}",
        sr.output
    );
}

/// A manifest carrying a Gate condition that omits the `op` field must
/// fail at serde deserialisation time — never reach the executor. This
/// is the "malformed condition surfaces a deserialisation error at
/// manifest load" contract: the gate cannot default to passing, so a
/// missing operator MUST be a load-time error rather than a silent
/// runtime no-op.
#[test]
fn gate_step_malformed_condition_fails_deserialization_at_load_time() {
    let manifest = r#"{
        "gate": {
            "condition": { "field": "/score", "value": 0.8 }
        }
    }"#;
    let err = serde_json::from_str::<StepMode>(manifest)
        .expect_err("malformed gate condition must not deserialise");
    let msg = err.to_string();
    assert!(
        msg.contains("op") || msg.contains("missing"),
        "deserialisation error must flag the missing `op` field; got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn gate_step_completed_when_field_omitted_compares_whole_input() {
    // Sanity check that the `field: None` path works end-to-end (string
    // comparison against the raw previous-step output), so the typed
    // shape is not de-facto locking callers into JSON inputs only.
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "gate-root-eq",
        StepMode::Gate {
            condition: GateCondition {
                field: None,
                op: GateOp::Eq,
                value: serde_json::json!("approved"),
            },
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "approved".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result.is_ok(), "Gate (root, Eq) must pass: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_step_is_noop_with_warn_and_completes() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "approval-stub",
        StepMode::Approval {
            recipients: vec!["telegram:@pakman".into(), "email:foo@bar".into()],
            timeout_secs: Some(86400),
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "in".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result.is_ok(), "Approval stub must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    assert_eq!(run.step_results[0].agent_name, "_operator:approval");
}

// ---------------------------------------------------------------------------
// `Transform` — fully wired in #4980 step 3/N
// ---------------------------------------------------------------------------

/// Happy-path render: the Tera template references `prev` (the
/// previous step's output) and a workflow-level variable. The
/// rendered string becomes the run's `current_input` for downstream
/// consumers and is recorded as the step's `output`.
#[tokio::test(flavor = "multi_thread")]
async fn transform_step_renders_tera_template_and_replaces_current_input() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "transform-happy",
        StepMode::Transform {
            code: "# Report\n\n{{ prev }}".to_string(),
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "body content".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result.is_ok(), "Transform must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    let sr = &run.step_results[0];
    assert_eq!(sr.agent_name, "_operator:transform");
    assert_eq!(sr.input_tokens, 0);
    assert_eq!(sr.output_tokens, 0);
    assert_eq!(sr.output, "# Report\n\nbody content");
    // Run.output mirrors the rendered string (it's the final step's output).
    assert_eq!(run.output.as_deref(), Some("# Report\n\nbody content"));
}

/// Missing-variable error: rendering a template that references an
/// undefined Tera variable halts the run with the Tera error as the
/// recorded reason (Tera includes line/column info), and the failing
/// step still appears in `run.step_results` so the dashboard surfaces
/// which transform blew up.
#[tokio::test(flavor = "multi_thread")]
async fn transform_step_missing_variable_halts_workflow_with_recorded_reason() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "transform-missing",
        StepMode::Transform {
            code: "hello {{ undefined_var }}".to_string(),
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("Transform with missing variable must halt");
    assert!(
        err.contains("Transform step 'op_step' failed"),
        "halt error must name the step; got: {err}"
    );
    assert!(
        err.contains("transform render failed"),
        "halt error must carry the wrapper; got: {err}"
    );

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(
        matches!(run.state, WorkflowRunState::Failed),
        "run must be Failed, got {:?}",
        run.state
    );
    assert_eq!(
        run.step_results.len(),
        1,
        "the failing transform step must still appear in run history"
    );
    let sr = &run.step_results[0];
    assert_eq!(sr.agent_name, "_operator:transform");
    assert!(
        sr.output.contains("transform render failed"),
        "step_result.output must carry the Tera error; got: {}",
        sr.output
    );
}

/// A Transform template that expands beyond `MAX_TRANSFORM_OUTPUT_BYTES`
/// halts the run with a typed error instead of silently propagating a
/// huge payload into `current_input` and the persisted step_result.
#[tokio::test(flavor = "multi_thread")]
async fn transform_step_oversize_output_halts_workflow_with_recorded_reason() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    // Tera's `range()` builtin is enabled by default; this template
    // renders roughly 2 * MAX_TRANSFORM_OUTPUT_BYTES bytes — guaranteed
    // to trip the cap.
    let code = format!(
        "{{% for i in range(end={}) %}}xx{{% endfor %}}",
        MAX_TRANSFORM_OUTPUT_BYTES
    );
    let workflow = workflow_with_op_step("transform-huge", StepMode::Transform { code });
    let wf_id = workflow.id;
    engine.register(workflow).await;
    let run_id = engine
        .create_run(wf_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("oversize transform must halt");
    assert!(
        err.contains("rendered") && err.contains("cap"),
        "halt error must name the cap; got: {err}"
    );
    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Failed));
}

/// A Wait step whose `duration_secs` exceeds `MAX_WAIT_SECS` is
/// rejected before the sleep starts. The executor-level guard is the
/// last line of defence for persisted-pre-cap workflows reloaded after
/// an upgrade; route-level validation rejects the same shape on
/// register / update, but here we drive the engine directly.
#[tokio::test(flavor = "multi_thread")]
async fn wait_step_over_cap_halts_before_sleep() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "wait-huge",
        StepMode::Wait {
            duration_secs: MAX_WAIT_SECS + 1,
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;
    let run_id = engine
        .create_run(wf_id, "ignored".to_string())
        .await
        .expect("create_run");
    let start = std::time::Instant::now();
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let elapsed = start.elapsed();
    let err = result.expect_err("over-cap Wait must halt");
    assert!(
        err.contains("exceeds cap"),
        "halt error must name the cap; got: {err}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "guard must reject before any sleep; got {:?}",
        elapsed
    );
    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Failed));
}

/// `Workflow::validate()` catches Tera syntax errors at manifest-load
/// time so operators never discover a typo at run time. We do not
/// also call `register` — the kernel's `register` is fire-and-forget
/// today (returns `WorkflowId`, not `Result`); the `validate` method
/// is the load-time gate callers must invoke.
#[test]
fn transform_step_syntax_error_caught_by_workflow_validate_at_load_time() {
    use librefang_kernel::workflow::Workflow;
    let workflow: Workflow = workflow_with_op_step(
        "transform-bad-syntax",
        StepMode::Transform {
            code: "hello {{ prev".to_string(), // unterminated expression
        },
    );
    let errs = workflow.validate();
    assert_eq!(
        errs.len(),
        1,
        "expected exactly one validation error; got: {errs:?}"
    );
    let (step_name, reason) = &errs[0];
    assert_eq!(step_name, "op_step");
    assert!(
        reason.contains("transform template parse failed"),
        "expected parse-failed wrapper; got: {reason}"
    );
}

// ---------------------------------------------------------------------------
// `Branch` — fully wired in #4980 step 4/N
// ---------------------------------------------------------------------------

/// Build a multi-step workflow for Branch dispatch testing.
///
/// Two intermediate "skipped" Transform steps sit between the Branch
/// and the target Transform terminal. The point of the layout: we
/// can demonstrate that the Branch jumps directly to the named
/// target and *bypasses* the skipped steps (their `_operator:transform`
/// step result is NOT recorded). The target step is always the LAST
/// step in the workflow so sequential dispatch terminates there
/// naturally.
///
///   step 0: seed Transform — emits a literal string controlled by the
///           test, so we can drive different branch decisions without
///           dispatching an agent.
///   step 1: Branch — single arm `match_value: $literal => then: $target`.
///   step 2: skipped Transform — `marker:skipped_a:{{ prev }}` (must
///           NOT appear in step_results when the arm hits).
///   step 3: skipped Transform — `marker:skipped_b:{{ prev }}` (must
///           NOT appear in step_results when the arm hits).
///   step 4: target Transform — `terminal:$tag:{{ prev }}` (must
///           appear when the arm hits; is the last step so the
///           workflow naturally completes here).
fn branch_skip_workflow(literal: &str, target_name: &str, target_template: &str) -> Workflow {
    let step = |name: &str, code: &str, mode: StepMode| WorkflowStep {
        name: name.to_string(),
        agent: StepAgent::ByName {
            name: "_operator_placeholder".to_string(),
        },
        prompt_template: code.to_string(),
        mode,
        timeout_secs: 120,
        error_mode: ErrorMode::Fail,
        output_var: None,
        inherit_context: None,
        depends_on: vec![],
        session_mode: None,
    };
    Workflow {
        id: WorkflowId::new(),
        name: format!("branch-skip-{literal}"),
        description: "branch executor integration test".to_string(),
        steps: vec![
            step(
                "seed",
                "ignored",
                StepMode::Transform {
                    code: literal.to_string(),
                },
            ),
            step(
                "decide",
                "ignored",
                StepMode::Branch {
                    arms: vec![BranchArm {
                        match_value: serde_json::json!(literal),
                        then: target_name.to_string(),
                    }],
                },
            ),
            step(
                "skipped_a",
                "ignored",
                StepMode::Transform {
                    code: "marker:skipped_a:{{ prev }}".to_string(),
                },
            ),
            step(
                "skipped_b",
                "ignored",
                StepMode::Transform {
                    code: "marker:skipped_b:{{ prev }}".to_string(),
                },
            ),
            step(
                target_name,
                "ignored",
                StepMode::Transform {
                    code: target_template.to_string(),
                },
            ),
        ],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    }
}

/// An arm whose `match_value` matches the previous step's output
/// forward-jumps execution to the named target, bypassing the steps
/// in between. The test drives the same shape with two literals
/// against two workflows that name different terminals — the
/// "multiple workflows fan-out" case from the brief — and asserts
/// each run's step trail.
#[tokio::test(flavor = "multi_thread")]
async fn branch_step_arm_hit_routes_to_target_and_skips_intermediate_steps() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();

    // Run A — branch jumps to `publish` (the last step), skipping
    // `skipped_a` and `skipped_b`.
    let wf_a = branch_skip_workflow("approved", "publish", "published:{{ prev }}");
    let wf_a_id = wf_a.id;
    engine.register(wf_a).await;
    let run_a = engine
        .create_run(wf_a_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result_a = engine
        .execute_run(
            run_a,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result_a.is_ok(), "Run A must succeed: {result_a:?}");
    let run_a_full = engine.get_run(run_a).await.expect("run A exists");
    assert!(matches!(run_a_full.state, WorkflowRunState::Completed));
    assert_eq!(
        run_a_full.output.as_deref(),
        Some("published:approved"),
        "approved input must hit publish arm and skip both intermediates"
    );
    let step_names_a: Vec<&str> = run_a_full
        .step_results
        .iter()
        .map(|s| s.step_name.as_str())
        .collect();
    assert_eq!(
        step_names_a,
        vec!["seed", "decide", "publish"],
        "intermediates must be skipped; got step trail: {step_names_a:?}"
    );
    // Branch prompt slot records the dispatched arm for the dashboard.
    // The trace is a JSON object keyed by `op` (see the unified
    // operator-prompt-trace shape in `operator_prompt_trace` —
    // #4980 review nit #4); we assert the parsed fields so the test
    // doesn't depend on `serde_json::Map`'s key-ordering choice.
    let branch_sr = &run_a_full.step_results[1];
    assert_eq!(branch_sr.agent_name, "_operator:branch");
    let trace: serde_json::Value = serde_json::from_str(&branch_sr.prompt)
        .unwrap_or_else(|e| panic!("branch prompt must be JSON; got {}: {e}", branch_sr.prompt));
    assert_eq!(trace["op"], "branch");
    assert_eq!(trace["target"], "publish");
    assert_eq!(trace["matched"], true);
    assert_eq!(trace["arm_idx"], 0);
    // The decision input must be carried (#4980 review nit #5) so an
    // operator debugging a "wrong arm fired" report can see the value
    // the comparator saw, not just the arm index.
    assert!(
        trace["input"].is_string(),
        "branch prompt must carry the decision input; got: {trace}"
    );

    // Run B — same shape, different terminal — proves the routing
    // really depends on which arm hits, not workflow-shape coincidence.
    let wf_b = branch_skip_workflow("rejected", "rewrite", "rewritten:{{ prev }}");
    let wf_b_id = wf_b.id;
    engine.register(wf_b).await;
    let run_b = engine
        .create_run(wf_b_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result_b = engine
        .execute_run(
            run_b,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    assert!(result_b.is_ok(), "Run B must succeed: {result_b:?}");
    let run_b_full = engine.get_run(run_b).await.expect("run B exists");
    assert!(matches!(run_b_full.state, WorkflowRunState::Completed));
    assert_eq!(run_b_full.output.as_deref(), Some("rewritten:rejected"));
    let step_names_b: Vec<&str> = run_b_full
        .step_results
        .iter()
        .map(|s| s.step_name.as_str())
        .collect();
    assert_eq!(step_names_b, vec!["seed", "decide", "rewrite"]);
}

/// When no arm matches the previous step's output, the run halts
/// with `WorkflowRunState::Failed` and a recorded reason that names
/// the unmatched output. Downstream terminals must not execute.
#[tokio::test(flavor = "multi_thread")]
async fn branch_step_no_arm_match_halts_workflow_with_recorded_reason() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    // Use the same skip-workflow layout but with a literal the
    // single arm does NOT match — the arm reads `approved`; the seed
    // emits `needs_review`. No arm matches → halt; the three
    // downstream terminals (`skipped_a`, `skipped_b`, `publish`)
    // must not execute.
    let workflow = branch_skip_workflow("needs_review", "publish", "published:{{ prev }}");
    // Override the arm so it cannot match the seed output.
    let mut workflow = workflow;
    workflow.steps[1].mode = StepMode::Branch {
        arms: vec![BranchArm {
            match_value: serde_json::json!("approved"),
            then: "publish".to_string(),
        }],
    };
    let wf_id = workflow.id;
    engine.register(workflow).await;

    let run_id = engine
        .create_run(wf_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("Branch with no matching arm must halt");
    assert!(
        err.contains("Branch step 'decide' had no matching arm"),
        "halt error must name the branch step; got: {err}"
    );
    assert!(
        err.contains("needs_review"),
        "halt error must surface the unmatched output; got: {err}"
    );

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(
        matches!(run.state, WorkflowRunState::Failed),
        "run must be Failed, got {:?}",
        run.state
    );
    // step_results: seed, decide (branch failure). No terminals ran.
    let step_names: Vec<&str> = run
        .step_results
        .iter()
        .map(|s| s.step_name.as_str())
        .collect();
    assert_eq!(step_names, vec!["seed", "decide"]);
}

/// Single-step Branch (no preceding seed): when the only step is a
/// Branch with no matching arm, we still halt — the engine should
/// not silently complete because Branch is an explicit decision
/// point. Mirrors `gate_step_fails_and_halts_workflow_with_recorded_reason`
/// in spirit.
#[tokio::test(flavor = "multi_thread")]
async fn branch_step_no_match_solo_halts_workflow() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "branch-solo",
        StepMode::Branch {
            arms: vec![BranchArm {
                match_value: serde_json::json!("never"),
                then: "nowhere".to_string(),
            }],
        },
    );
    let wf_id = workflow.id;
    engine.register(workflow).await;
    let run_id = engine
        .create_run(wf_id, "actually-fed-in".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("solo Branch with no match must halt");
    assert!(err.contains("had no matching arm"), "got: {err}");
    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Failed));
}

/// A sequential workflow with two steps sharing the Branch arm's
/// target name must halt explicitly rather than silently jump to
/// whichever appears first.
///
/// Background: duplicate-name detection lives in
/// `build_dependency_graph`, which is only reached via
/// `topological_sort`. `execute_run` only calls the DAG path when at
/// least one step declares a `depends_on` edge — a workflow without
/// any `depends_on` (the historical sequential path) skips topo sort
/// entirely. Without the in-Branch uniqueness guard this exact case
/// silently routed to the first matching step.
#[tokio::test(flavor = "multi_thread")]
async fn branch_step_ambiguous_target_halts_with_recorded_reason() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();

    // No depends_on anywhere → sequential path, so topo sort (and its
    // duplicate-name guard) is skipped.
    let step = |name: &str, code: &str, mode: StepMode| WorkflowStep {
        name: name.to_string(),
        agent: StepAgent::ByName {
            name: "_operator_placeholder".to_string(),
        },
        prompt_template: code.to_string(),
        mode,
        timeout_secs: 120,
        error_mode: ErrorMode::Fail,
        output_var: None,
        inherit_context: None,
        depends_on: vec![],
        session_mode: None,
    };
    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "branch-ambiguous".to_string(),
        description: "duplicate target name guard".to_string(),
        steps: vec![
            step(
                "seed",
                "ignored",
                StepMode::Transform {
                    code: "go".to_string(),
                },
            ),
            step(
                "decide",
                "ignored",
                StepMode::Branch {
                    arms: vec![BranchArm {
                        match_value: serde_json::json!("go"),
                        then: "target".to_string(),
                    }],
                },
            ),
            // Two steps share the name "target" — the Branch arm's
            // target name resolves to both.
            step(
                "target",
                "ignored",
                StepMode::Transform {
                    code: "first:{{ prev }}".to_string(),
                },
            ),
            step(
                "target",
                "ignored",
                StepMode::Transform {
                    code: "second:{{ prev }}".to_string(),
                },
            ),
        ],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    };
    let wf_id = workflow.id;
    engine.register(workflow).await;
    let run_id = engine
        .create_run(wf_id, "ignored".to_string())
        .await
        .expect("create_run");
    let result = engine
        .execute_run(
            run_id,
            panicking_agent_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
                panic!("operator-node executor must not call send_message");
                #[allow(unreachable_code)]
                Ok::<_, String>(("unreachable".to_string(), 0u64, 0u64))
            },
        )
        .await;
    let err = result.expect_err("ambiguous branch target must halt");
    assert!(
        err.contains("ambiguous") && err.contains("target"),
        "ambiguous-target reason should be explicit; got: {err}"
    );
    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Failed));
    // Neither duplicate `target` step must have executed. The
    // `decide` Branch step itself fails before it pushes its own
    // synthetic StepResult — matching the existing "target not found"
    // and "backward jump" arms — so the trail naturally stops at
    // `seed`. If routing silently picked the first match, the trail
    // would extend into `target` (output `first:go`).
    let step_names: Vec<&str> = run
        .step_results
        .iter()
        .map(|s| s.step_name.as_str())
        .collect();
    assert_eq!(
        step_names,
        vec!["seed"],
        "ambiguous branch must not dispatch either duplicate target; got: {step_names:?}"
    );
    let output_strs: Vec<&str> = run.step_results.iter().map(|s| s.output.as_str()).collect();
    assert!(
        !output_strs
            .iter()
            .any(|o| o.contains("first:") || o.contains("second:")),
        "neither duplicate target's transform output should appear; got: {output_strs:?}"
    );
}

// ---------------------------------------------------------------------------
// Validate fail-closed (#4980 review blocking #1) — DAG + operator-node
// combinations are rejected at register time so the silent run-time
// failure from `execute_run_dag` calling `agent_resolver` on an operator
// node never reaches a real run.
// ---------------------------------------------------------------------------

/// Engine-level smoke test for the DAG+operator-node fail-closed rule.
/// The kernel-unit test
/// (`workflow_validate_rejects_operator_node_combined_with_dag_depends_on`)
/// covers each variant individually; here we just pin that the
/// integration boundary (the `Workflow::validate` call the HTTP route
/// makes before forwarding to `register_workflow`) sees the same
/// errors. Pre-fix, a DAG workflow containing operator nodes
/// serialised, persisted, round-tripped through pause/resume, and only
/// failed at run time with `format_missing_agent_error`.
#[tokio::test(flavor = "multi_thread")]
async fn validate_rejects_dag_workflow_with_operator_node_step() {
    let test = boot();
    let _engine = test.state.kernel.workflow_engine();

    // Producer is a vanilla Sequential step the operator node depends
    // on; the operator node itself is a Wait variant — but Gate,
    // Approval, Transform, Branch all behave identically (covered by
    // the kernel-unit cases). The presence of `depends_on` is what
    // triggers the rule.
    let producer = WorkflowStep {
        name: "producer".to_string(),
        agent: StepAgent::ByName {
            name: "_producer".to_string(),
        },
        prompt_template: "{{input}}".to_string(),
        mode: StepMode::Sequential,
        timeout_secs: 30,
        error_mode: ErrorMode::Fail,
        output_var: None,
        inherit_context: None,
        depends_on: vec![],
        session_mode: None,
    };
    let op = WorkflowStep {
        name: "op".to_string(),
        agent: StepAgent::ByName {
            name: "_operator_placeholder".to_string(),
        },
        prompt_template: "{{input}}".to_string(),
        mode: StepMode::Wait { duration_secs: 1 },
        timeout_secs: 30,
        error_mode: ErrorMode::Fail,
        output_var: None,
        inherit_context: None,
        depends_on: vec!["producer".to_string()],
        session_mode: None,
    };
    let wf = Workflow {
        id: WorkflowId::new(),
        name: "dag-plus-operator".to_string(),
        description: "must be rejected at validate time".to_string(),
        steps: vec![producer, op],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    };
    let errs = wf.validate();
    assert!(
        errs.iter().any(|(s, r)| s == "op" && r.contains("DAG")),
        "validate must reject DAG + operator-node combination; got: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// dry_run (#4980 review blocking #2) — operator-node steps must be
// previewed as `_operator:<kind>` with `agent_found = true`, not fall
// through to the agent-shaped branch and report as missing-agent.
// ---------------------------------------------------------------------------

/// Pre-fix, `dry_run` matched only on `Conditional` and fell through to
/// `agent_resolver` for everything else; operator-node steps surfaced
/// in the dashboard preview as `agent_found = false`, "missing agent"
/// rows. This test pins that the preview now names each operator
/// kind and reports it as found.
#[tokio::test(flavor = "multi_thread")]
async fn dry_run_reports_operator_nodes_as_found_with_synthetic_agent_names() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();

    // Build a workflow with one step per operator-node variant. No
    // `depends_on` (operator + DAG combination is rejected by
    // `validate` per blocking #1), so we exercise the sequential
    // dry-run path. Five operator variants, one terminal Sequential
    // step (the agent resolver is allowed to fail for that one — we
    // assert the operator nodes specifically).
    let mk = |name: &str, mode: StepMode| WorkflowStep {
        name: name.to_string(),
        agent: StepAgent::ByName {
            name: "_op_placeholder".to_string(),
        },
        prompt_template: "{{input}}".to_string(),
        mode,
        timeout_secs: 30,
        error_mode: ErrorMode::Fail,
        output_var: None,
        inherit_context: None,
        depends_on: vec![],
        session_mode: None,
    };
    let wf = Workflow {
        id: WorkflowId::new(),
        name: "dry-run-operators".to_string(),
        description: "dry_run operator-node preview test".to_string(),
        steps: vec![
            mk("w", StepMode::Wait { duration_secs: 5 }),
            mk(
                "g",
                StepMode::Gate {
                    condition: GateCondition {
                        field: Some("/score".to_string()),
                        op: GateOp::Gt,
                        value: serde_json::json!(0.8),
                    },
                },
            ),
            mk(
                "a",
                StepMode::Approval {
                    recipients: vec!["telegram:@pakman".into()],
                    timeout_secs: Some(3600),
                },
            ),
            mk(
                "t",
                StepMode::Transform {
                    code: "hello {{ prev }}".to_string(),
                },
            ),
            mk(
                "b",
                StepMode::Branch {
                    arms: vec![BranchArm {
                        match_value: serde_json::json!("ok"),
                        then: "fin".to_string(),
                    }],
                },
            ),
        ],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    };
    let wf_id = wf.id;
    engine.register(wf).await;

    // The agent resolver would fire on `Sequential` / `Conditional` /
    // `Loop` / `FanOut` / `Collect` steps; this workflow has none of
    // those, so it must never be called. We pass a panicking resolver
    // to enforce the contract.
    let preview = engine
        .dry_run(
            wf_id,
            "seed",
            |_agent: &StepAgent| -> Option<(AgentId, String, bool)> {
                panic!("dry_run must not call agent_resolver for operator nodes");
            },
        )
        .await
        .expect("dry_run");

    assert_eq!(preview.len(), 5, "one preview row per step");
    let expected: Vec<(&str, &str)> = vec![
        ("w", "_operator:wait"),
        ("g", "_operator:gate"),
        ("a", "_operator:approval"),
        ("t", "_operator:transform"),
        ("b", "_operator:branch"),
    ];
    for (preview_row, (expected_name, expected_kind)) in preview.iter().zip(expected.iter()) {
        assert_eq!(
            preview_row.step_name, *expected_name,
            "step name mismatch in dry_run preview"
        );
        assert!(
            preview_row.agent_found,
            "operator-node step `{expected_name}` must report agent_found=true; \
             pre-fix this was false because dry_run fell through to agent_resolver"
        );
        assert_eq!(
            preview_row.agent_name.as_deref(),
            Some(*expected_kind),
            "operator-node step `{expected_name}` must carry the `{expected_kind}` synthetic name"
        );
        assert!(
            !preview_row.skipped,
            "operator-node step `{expected_name}` must not be marked skipped on a clean dry_run"
        );
    }
}

/// A `Transform` step with a syntax-broken Tera template must surface
/// in the dry_run preview as a `skipped` row with a typed reason —
/// the same shape the run-time executor surfaces and the same shape
/// `Workflow::validate` rejects at register time. dry_run is reachable
/// for workflows loaded from disk that bypass the HTTP gate, so the
/// dry-run preview must double-check the template independently.
#[tokio::test(flavor = "multi_thread")]
async fn dry_run_marks_unparseable_transform_template_as_skipped() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();

    let wf = Workflow {
        id: WorkflowId::new(),
        name: "dry-run-bad-transform".to_string(),
        description: "dry_run surfaces template syntax errors".to_string(),
        steps: vec![WorkflowStep {
            name: "bad-transform".to_string(),
            agent: StepAgent::ByName {
                name: "_op_placeholder".to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode: StepMode::Transform {
                // Unterminated expression — Tera rejects at parse time.
                code: "hello {{ prev".to_string(),
            },
            timeout_secs: 30,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        }],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    };
    let wf_id = wf.id;
    engine.register(wf).await;

    let preview = engine
        .dry_run(
            wf_id,
            "seed",
            |_agent: &StepAgent| -> Option<(AgentId, String, bool)> {
                panic!("dry_run must not call agent_resolver for Transform");
            },
        )
        .await
        .expect("dry_run");

    assert_eq!(preview.len(), 1);
    let row = &preview[0];
    assert_eq!(row.step_name, "bad-transform");
    assert_eq!(row.agent_name.as_deref(), Some("_operator:transform"));
    assert!(row.agent_found);
    assert!(
        row.skipped,
        "Tera parse error must surface as skipped in dry_run"
    );
    let reason = row
        .skip_reason
        .as_deref()
        .expect("skip_reason must be set on Tera parse error");
    assert!(
        reason.contains("transform template parse failed"),
        "skip_reason should wrap the Tera parse error; got: {reason}"
    );
}

/// dry_run must advance `current_input` through a Transform step so
/// downstream operator nodes' previews reflect the post-Transform
/// value the run-time executor will see. The run-time Transform arm
/// sets `current_input = rendered`; without mirroring that in
/// `dry_run`, `{{input}}` previews on subsequent steps diverge from
/// what the real run produces. Wait / Gate / Approval / Branch stay
/// pass-through both at run time and in the preview — only Transform
/// rewrites `current_input`. Re-review nit #1.
#[tokio::test(flavor = "multi_thread")]
async fn dry_run_transform_advances_current_input_for_downstream_previews() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();

    // Two-node sequence: Transform → Wait. Wait's `prompt_template`
    // expands `{{input}}` against the post-Transform value; we
    // pre-fix this would have echoed the seed verbatim.
    let wf = Workflow {
        id: WorkflowId::new(),
        name: "dry-run-transform-feeds-wait".to_string(),
        description: "Transform output must flow into downstream {{input}}".to_string(),
        steps: vec![
            WorkflowStep {
                name: "uppercase".to_string(),
                agent: StepAgent::ByName {
                    name: "_op_placeholder".to_string(),
                },
                // Wait's `prompt_template` is the value under test; the
                // Transform step's `prompt_template` is irrelevant
                // because its row's `resolved_prompt` is the static
                // `"transform: <code>"` string.
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Transform {
                    code: "{{ prev | upper }}".to_string(),
                },
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "after-transform".to_string(),
                agent: StepAgent::ByName {
                    name: "_op_placeholder".to_string(),
                },
                // This is the assertion target: dry_run expands
                // `{{input}}` against `current_input`, which the
                // preceding Transform must have rewritten to
                // `"SEED"`.
                prompt_template: "after: {{input}}".to_string(),
                mode: StepMode::Wait { duration_secs: 5 },
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
        ],
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: None,
    };
    let wf_id = wf.id;
    engine.register(wf).await;

    let preview = engine
        .dry_run(
            wf_id,
            "seed",
            |_agent: &StepAgent| -> Option<(AgentId, String, bool)> {
                panic!("dry_run must not call agent_resolver for operator nodes");
            },
        )
        .await
        .expect("dry_run");

    assert_eq!(preview.len(), 2);

    // First row: Transform itself — sanity-check the existing
    // contract so a future regression here doesn't masquerade as the
    // bug under test.
    assert_eq!(preview[0].step_name, "uppercase");
    assert_eq!(
        preview[0].agent_name.as_deref(),
        Some("_operator:transform")
    );
    assert!(!preview[0].skipped, "valid template must not be skipped");

    // Second row: Wait. The expanded prompt must reflect the
    // Transform's rendered output ("SEED"), not the seed input
    // ("seed"). Pre-fix this would have been `"after: seed"` because
    // dry_run left `current_input` untouched on the Transform arm.
    assert_eq!(preview[1].step_name, "after-transform");
    assert_eq!(
        preview[1].resolved_prompt, "after: SEED",
        "downstream step's {{{{input}}}} preview must reflect the Transform's rendered output; \
         got {:?}",
        preview[1].resolved_prompt
    );
}
