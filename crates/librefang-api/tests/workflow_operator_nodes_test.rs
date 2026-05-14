//! Integration tests for workflow operator-node step modes (#4980 step
//! 1/N → step 3/N).
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
//! * `Approval` — no-op-with-warn; blocked on #4983 (async-task
//!   tracker). The executor will wire there once the dependency lands.
//! * `Branch` — no-op-with-warn; wired in step 4 of the #4980 series.
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
    WorkflowRunState, WorkflowStep,
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

#[tokio::test(flavor = "multi_thread")]
async fn branch_step_is_noop_with_warn_and_completes() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "branch-stub",
        StepMode::Branch {
            arms: vec![
                BranchArm {
                    match_value: serde_json::json!("approved"),
                    then: "publish".to_string(),
                },
                BranchArm {
                    match_value: serde_json::json!("rejected"),
                    then: "rewrite".to_string(),
                },
            ],
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
    assert!(result.is_ok(), "Branch stub must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    assert_eq!(run.step_results[0].agent_name, "_operator:branch");
}
