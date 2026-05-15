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
    let branch_sr = &run_a_full.step_results[1];
    assert_eq!(branch_sr.agent_name, "_operator:branch");
    assert!(
        branch_sr.prompt.contains("branch -> 'publish'"),
        "branch step's prompt slot must record the dispatched target; got: {}",
        branch_sr.prompt
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
