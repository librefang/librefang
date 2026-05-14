//! Integration tests for workflow operator-node step modes (#4980 step 1/N).
//!
//! Five new `StepMode` variants land here:
//!
//! * `Wait` — fully wired: sleeps for `duration_secs`, emits a structured
//!   `info!` log, returns success. Cancellation-aware via the run's
//!   `cancel_notify`.
//! * `Gate` / `Approval` / `Transform` / `Branch` — no-op-with-warn for
//!   V1. The wire format is locked so workflows can serialise these
//!   variants today; the executor bodies land in follow-up PRs once the
//!   deferred design questions (Gate.condition syntax,
//!   Approval operator-identity, Transform.code shape, Branch jump
//!   semantics) are resolved.
//!
//! The tests run the workflow engine directly (no HTTP) via
//! `kernel.workflow_engine().execute_run(...)` with a mock
//! `agent_resolver` / `send_message` pair — matching the kernel-only
//! pattern used by `workflow_pause_resume_test.rs::resume_with_wrong_token_returns_401`.
//! No agent is dispatched for operator nodes, so the mock sender is
//! never invoked on operator-node paths; we assert that fact by making
//! the mock panic on call.

use librefang_kernel::workflow::{
    BranchArm, ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowRunState, WorkflowStep,
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
// `Gate` / `Approval` / `Transform` / `Branch` — no-op-with-warn for V1
// ---------------------------------------------------------------------------
//
// These four log a structured `warn!` and return success. We can't
// easily capture `tracing` output from within an integration test
// without pulling in a subscriber dependency, so each test asserts the
// observable behaviour: the run completes successfully, exactly one
// step result is recorded with the matching `_operator:<kind>` agent
// name, and no agent was dispatched (mock resolver / sender would
// panic). The "not yet implemented" warn-log itself is exercised
// manually when the file is run with `RUST_LOG=warn cargo test ...`.

#[tokio::test(flavor = "multi_thread")]
async fn gate_step_is_noop_with_warn_and_completes() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "gate-stub",
        StepMode::Gate {
            condition: "score > 0.8".to_string(),
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
    assert!(result.is_ok(), "Gate stub must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    assert_eq!(run.step_results[0].agent_name, "_operator:gate");
    assert_eq!(run.step_results[0].input_tokens, 0);
    assert_eq!(run.step_results[0].output_tokens, 0);
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

#[tokio::test(flavor = "multi_thread")]
async fn transform_step_is_noop_with_warn_and_completes() {
    let test = boot();
    let engine = test.state.kernel.workflow_engine();
    let workflow = workflow_with_op_step(
        "transform-stub",
        StepMode::Transform {
            code: "# {{title}}\n\n{{body}}".to_string(),
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
    assert!(result.is_ok(), "Transform stub must succeed: {result:?}");

    let run = engine.get_run(run_id).await.expect("run exists");
    assert!(matches!(run.state, WorkflowRunState::Completed));
    assert_eq!(run.step_results.len(), 1);
    assert_eq!(run.step_results[0].agent_name, "_operator:transform");
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
