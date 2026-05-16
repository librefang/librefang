//! End-to-end workflow integration tests.
//!
//! Tests the full pipeline: boot kernel → spawn agents → create workflow →
//! execute workflow → verify outputs flow through the pipeline.
//!
//! LLM tests require GROQ_API_KEY. Non-LLM tests verify the kernel-level
//! workflow wiring without making real API calls.

// The E2E test below builds a deeply-nested future through the kernel ->
// runtime -> agent_loop call chain. Each added field on LoopOptions /
// SessionInterrupt makes the type-layout query a little deeper; the default
// 128 overflows on some toolchains after issue #3044's interrupt cascade
// changes. 256 gives us headroom without hiding real regressions.
#![recursion_limit = "256"]

use librefang_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowInputParam, WorkflowStep,
};
use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::LibreFangKernel;
use librefang_kernel::WorkflowSubsystemApi;
use librefang_testing::MockKernelBuilder;
use librefang_types::agent::AgentManifest;
// kernel_handle is re-exported from librefang-kernel.
use librefang_kernel::kernel_handle::WorkflowRunner;

fn spawn_test_agent(
    kernel: &LibreFangKernel,
    name: &str,
    system_prompt: &str,
) -> librefang_types::agent::AgentId {
    let manifest_str = format!(
        r#"
name = "{name}"
version = "0.1.0"
description = "Workflow test agent: {name}"
author = "test"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "{system_prompt}"

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#
    );
    let manifest: AgentManifest = toml::from_str(&manifest_str).unwrap();
    kernel.spawn_agent(manifest).expect("Agent should spawn")
}

// ---------------------------------------------------------------------------
// Kernel-level workflow wiring tests (no LLM needed)
// ---------------------------------------------------------------------------

/// Test that workflow registration and agent resolution work at the kernel level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_workflow_register_and_resolve() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test-model".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    // Spawn agents
    let manifest: AgentManifest = toml::from_str(
        r#"
name = "agent-alpha"
version = "0.1.0"
description = "Alpha"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test"
system_prompt = "Alpha."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
    )
    .unwrap();
    let alpha_id = kernel.spawn_agent(manifest).unwrap();

    let manifest2: AgentManifest = toml::from_str(
        r#"
name = "agent-beta"
version = "0.1.0"
description = "Beta"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test"
system_prompt = "Beta."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
    )
    .unwrap();
    let beta_id = kernel.spawn_agent(manifest2).unwrap();

    // Create a 2-step workflow referencing agents by name
    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "alpha-beta-pipeline".to_string(),
        description: "Tests agent resolution by name".to_string(),
        steps: vec![
            WorkflowStep {
                name: "step-alpha".to_string(),
                agent: StepAgent::ByName {
                    name: "agent-alpha".to_string(),
                },
                prompt_template: "Analyze: {{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: Some("alpha_out".to_string()),
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "step-beta".to_string(),
                agent: StepAgent::ByName {
                    name: "agent-beta".to_string(),
                },
                prompt_template: "Summarize: {{input}} (alpha said: {{alpha_out}})".to_string(),
                mode: StepMode::Sequential,
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
        input_schema: None,
    };

    let wf_id = kernel.register_workflow(workflow).await;

    // Verify workflow is registered
    let workflows = kernel.engine_ref().list_workflows().await;
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].name, "alpha-beta-pipeline");

    // Verify agents can be found by name
    let alpha = kernel.agent_registry_ref().find_by_name("agent-alpha");
    assert!(alpha.is_some());
    assert_eq!(alpha.unwrap().id, alpha_id);

    let beta = kernel.agent_registry_ref().find_by_name("agent-beta");
    assert!(beta.is_some());
    assert_eq!(beta.unwrap().id, beta_id);

    // Verify workflow run can be created
    let run_id = kernel
        .engine_ref()
        .create_run(wf_id, "test input".to_string())
        .await;
    assert!(run_id.is_some());

    let run = kernel.engine_ref().get_run(run_id.unwrap()).await.unwrap();
    assert_eq!(run.input, "test input");

    kernel.shutdown();
}

/// Test workflow with agent referenced by ID.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_workflow_agent_by_id() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test-model".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let manifest: AgentManifest = toml::from_str(
        r#"
name = "id-agent"
version = "0.1.0"
description = "Test"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test"
system_prompt = "Test."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
    )
    .unwrap();
    let agent_id = kernel.spawn_agent(manifest).unwrap();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "by-id-test".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "step1".to_string(),
            agent: StepAgent::ById {
                id: agent_id.to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: None,
    };

    let wf_id = kernel.register_workflow(workflow).await;

    // Can create run (agent resolution happens at execute time)
    let run_id = kernel
        .engine_ref()
        .create_run(wf_id, "hello".to_string())
        .await;
    assert!(run_id.is_some());

    kernel.shutdown();
}

/// Test trigger registration and listing at kernel level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trigger_registration_with_kernel() {
    use librefang_kernel::triggers::TriggerPattern;

    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test-model".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let manifest: AgentManifest = toml::from_str(
        r#"
name = "trigger-agent"
version = "0.1.0"
description = "Trigger test"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test"
system_prompt = "Test."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
    )
    .unwrap();
    let agent_id = kernel.spawn_agent(manifest).unwrap();

    // Register triggers
    let t1 = kernel
        .register_trigger(
            agent_id,
            TriggerPattern::Lifecycle,
            "Lifecycle event: {{event}}".to_string(),
            0,
        )
        .unwrap();

    let t2 = kernel
        .register_trigger(
            agent_id,
            TriggerPattern::SystemKeyword {
                keyword: "deploy".to_string(),
            },
            "Deploy event: {{event}}".to_string(),
            5,
        )
        .unwrap();

    // List all triggers
    let all = kernel.list_triggers(None);
    assert_eq!(all.len(), 2);

    // List triggers for specific agent
    let agent_triggers = kernel.list_triggers(Some(agent_id));
    assert_eq!(agent_triggers.len(), 2);

    // Remove one
    assert!(kernel.remove_trigger(t1));
    let remaining = kernel.list_triggers(None);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, t2);

    kernel.shutdown();
}

// ---------------------------------------------------------------------------
// Full E2E with real LLM (skip if no GROQ_API_KEY)
// ---------------------------------------------------------------------------

/// End-to-end: boot kernel → spawn 2 agents → create 2-step workflow →
/// run it through the real Groq LLM → verify output flows from step 1 to step 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_workflow_e2e_with_groq() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set, skipping E2E workflow test");
        return;
    }

    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "groq".to_string();
            c.default_model.model = "llama-3.3-70b-versatile".to_string();
            c.default_model.api_key_env = "GROQ_API_KEY".to_string();
        })
        .build();

    // Spawn two agents with distinct roles
    let _analyst_id = spawn_test_agent(
        &kernel,
        "wf-analyst",
        "You are an analyst. When given text, respond with exactly: ANALYSIS: followed by a one-sentence analysis.",
    );
    let _writer_id = spawn_test_agent(
        &kernel,
        "wf-writer",
        "You are a writer. When given text, respond with exactly: SUMMARY: followed by a one-sentence summary.",
    );

    // Create a 2-step pipeline: analyst → writer
    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "analyst-writer-pipeline".to_string(),
        description: "E2E integration test workflow".to_string(),
        steps: vec![
            WorkflowStep {
                name: "analyze".to_string(),
                agent: StepAgent::ByName {
                    name: "wf-analyst".to_string(),
                },
                prompt_template: "Analyze the following: {{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 60,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "summarize".to_string(),
                agent: StepAgent::ByName {
                    name: "wf-writer".to_string(),
                },
                prompt_template: "Summarize this analysis: {{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 60,
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
        input_schema: None,
    };

    let wf_id = kernel.register_workflow(workflow).await;

    // Run the workflow
    let result = kernel
        .run_workflow(
            wf_id,
            "The Rust programming language is growing rapidly.".to_string(),
        )
        .await;

    assert!(
        result.is_ok(),
        "Workflow should complete: {:?}",
        result.err()
    );
    let (run_id, output) = result.unwrap();

    println!("\n=== WORKFLOW OUTPUT ===");
    println!("{output}");
    println!("======================\n");

    assert!(!output.is_empty(), "Workflow output should not be empty");

    // Verify the workflow run record
    let run = kernel.engine_ref().get_run(run_id).await.unwrap();
    assert!(matches!(
        run.state,
        librefang_kernel::workflow::WorkflowRunState::Completed
    ));
    assert_eq!(run.step_results.len(), 2);
    assert_eq!(run.step_results[0].step_name, "analyze");
    assert_eq!(run.step_results[1].step_name, "summarize");

    // Both steps should have used tokens
    assert!(run.step_results[0].input_tokens > 0);
    assert!(run.step_results[0].output_tokens > 0);
    assert!(run.step_results[1].input_tokens > 0);
    assert!(run.step_results[1].output_tokens > 0);

    // List runs
    let runs = kernel.engine_ref().list_runs(None).await;
    assert_eq!(runs.len(), 1);

    kernel.shutdown();
}

// ---------------------------------------------------------------------------
// #4982 — workflow_describe end-to-end through the kernel handle
// ---------------------------------------------------------------------------

/// Workflows with an explicit `input_schema` surface those parameters
/// verbatim through `WorkflowRunner::describe_workflow`. Verifies the
/// kernel-side resolution path (matches the runtime's `workflow_describe`
/// tool surface).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_describe_returns_explicit_input_schema() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "publish-article".to_string(),
        description: "Draft + cover image + post".to_string(),
        steps: vec![WorkflowStep {
            name: "draft".to_string(),
            agent: StepAgent::ByName {
                name: "writer".to_string(),
            },
            prompt_template: "Topic: {{topic}}. Cover ref: {{cover}}".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: Some(vec![
            WorkflowInputParam {
                name: "topic".to_string(),
                param_type: "string".to_string(),
                required: true,
                description: Some("Article topic".to_string()),
            },
            WorkflowInputParam {
                name: "cover".to_string(),
                param_type: "image".to_string(),
                required: false,
                description: None,
            },
        ]),
    };

    let _wf_id = kernel.register_workflow(workflow).await;

    let description = kernel
        .describe_workflow("publish-article")
        .await
        .expect("workflow found by name");
    assert_eq!(description.name, "publish-article");
    assert_eq!(description.step_names, vec!["draft".to_string()]);
    // Schema is sorted by name for deterministic prompt output (#3298).
    assert_eq!(description.input_schema.len(), 2);
    assert_eq!(description.input_schema[0].name, "cover");
    assert_eq!(description.input_schema[0].param_type, "image");
    assert!(!description.input_schema[0].required);
    assert_eq!(description.input_schema[1].name, "topic");
    assert_eq!(description.input_schema[1].param_type, "string");
    assert!(description.input_schema[1].required);
    assert_eq!(
        description.input_schema[1].description.as_deref(),
        Some("Article topic")
    );

    kernel.shutdown();
}

/// When no explicit `input_schema` is authored, `describe_workflow` falls
/// back to scanning `{{var}}` placeholders. The reserved `{{input}}`
/// (previous-step output) must NOT appear in the discovered parameters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_describe_auto_detects_from_template_placeholders() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "legacy-flow".to_string(),
        description: "Old-style flow with no input_schema block".to_string(),
        steps: vec![
            WorkflowStep {
                name: "step-1".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "Process {{user_text}} carefully".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 30,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "step-2".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "Translate to {{lang}}: {{input}}".to_string(),
                mode: StepMode::Sequential,
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
        input_schema: None,
    };

    let _wf_id = kernel.register_workflow(workflow).await;

    let description = kernel
        .describe_workflow("legacy-flow")
        .await
        .expect("workflow found");
    let names: Vec<&str> = description
        .input_schema
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["lang", "user_text"],
        "auto-detect must skip {{{{input}}}} and sort by name"
    );
    for p in &description.input_schema {
        assert_eq!(p.param_type, "string");
        assert!(p.required, "auto-detected params are required by default");
    }

    kernel.shutdown();
}

/// `list_workflows` exposes `has_input_schema=true` on entries that the
/// agent can describe — either via explicit schema OR via auto-detected
/// `{{var}}` placeholders. Drives the LLM heuristic of "should I call
/// workflow_describe before workflow_run?".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_list_reports_has_input_schema_for_both_paths() {
    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    // (a) Workflow with explicit schema.
    let explicit = Workflow {
        id: WorkflowId::new(),
        name: "explicit".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "s".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "fixed prompt, no placeholders".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: Some(vec![WorkflowInputParam {
            name: "x".to_string(),
            param_type: "string".to_string(),
            required: true,
            description: None,
        }]),
    };
    kernel.register_workflow(explicit).await;

    // (b) Workflow without schema, but step prompt has {{var}}.
    let implicit = Workflow {
        id: WorkflowId::new(),
        name: "implicit".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "s".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "Echo {{message}}".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: None,
    };
    kernel.register_workflow(implicit).await;

    // (c) Workflow with no parametric input at all.
    let none = Workflow {
        id: WorkflowId::new(),
        name: "none".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "s".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "Just say hi".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: None,
    };
    kernel.register_workflow(none).await;

    let summaries = kernel.list_workflows().await;
    assert_eq!(summaries.len(), 3);
    let by_name: std::collections::HashMap<&str, bool> = summaries
        .iter()
        .map(|s| (s.name.as_str(), s.has_input_schema))
        .collect();
    assert!(by_name["explicit"], "explicit schema must surface");
    assert!(
        by_name["implicit"],
        "auto-detected placeholders must surface"
    );
    assert!(
        !by_name["none"],
        "workflows with no parametric input must report false"
    );

    kernel.shutdown();
}

// ---------------------------------------------------------------------------
// #4982 — end-to-end: input_schema + {{var}} + JSON input → resolved prompt
// ---------------------------------------------------------------------------

/// Pin the BLOCKING claim of the #4982 PR: a workflow that declares
/// `[[input_schema]]` parameters and references them via `{{var}}` in a
/// step prompt, when run with an object-shaped JSON input, dispatches a
/// step prompt with the placeholders filled by the input values.
///
/// Uses the kernel-side `WorkflowEngine` accessor and a captured-sender
/// closure rather than a full LLM round-trip so the test is hermetic and
/// runs without GROQ credentials. The runtime's `_artifact` resolver runs
/// upstream (see `tool_runner::resolve_workflow_input_artifacts`) and lands
/// the handle string in the input JSON before the engine sees it, so the
/// test passes the resolved shape directly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_engine_substitutes_input_schema_vars_into_step_prompt() {
    use librefang_kernel::workflow::WorkflowEngine;
    use librefang_kernel::WorkflowSubsystemApi;
    use librefang_types::agent::{AgentId, SessionMode};
    use std::sync::{Arc, Mutex};

    let (kernel, _tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.default_model.provider = "ollama".to_string();
            c.default_model.model = "test".to_string();
            c.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        })
        .build();

    let workflow = Workflow {
        id: WorkflowId::new(),
        name: "publish-with-cover".to_string(),
        description: "Rich-input workflow for #4982 regression".to_string(),
        steps: vec![WorkflowStep {
            name: "draft".to_string(),
            agent: librefang_kernel::workflow::StepAgent::ByName {
                name: "writer".to_string(),
            },
            prompt_template: "Write about {{topic}} with cover {{cover}}.".to_string(),
            mode: StepMode::Sequential,
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
        input_schema: Some(vec![
            WorkflowInputParam {
                name: "topic".to_string(),
                param_type: "string".to_string(),
                required: true,
                description: Some("Article topic".to_string()),
            },
            WorkflowInputParam {
                name: "cover".to_string(),
                param_type: "file".to_string(),
                required: true,
                description: Some("Cover image artifact".to_string()),
            },
        ]),
    };
    let wf_id = kernel.register_workflow(workflow).await;

    // Resolve handle string for `{{cover}}` — matches the shape the
    // runtime's `_artifact` resolver produces upstream.
    let handle = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    let input_json = serde_json::json!({
        "topic": "Rust",
        "cover": handle,
    })
    .to_string();

    // Drive the engine directly with a stub sender: bypasses agent / LLM
    // wiring while still exercising the dispatch path that
    // `WorkflowRunner::run_workflow` invokes in production. The captured
    // prompt is what the agent loop would receive verbatim.
    let engine: &WorkflowEngine = kernel.engine_ref();
    let run_id = engine
        .create_run(wf_id, input_json)
        .await
        .expect("run created");
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
        let sink = Arc::clone(&sink);
        async move {
            sink.lock().unwrap().push(msg.clone());
            Ok::<(String, u64, u64), String>((format!("ack:{msg}"), 1u64, 1u64))
        }
    };
    let resolver = |_a: &librefang_kernel::workflow::StepAgent| {
        Some((AgentId::new(), "stub".to_string(), true))
    };
    let result = engine.execute_run(run_id, resolver, sender).await;
    assert!(result.is_ok(), "engine.execute_run failed: {:?}", result);

    let prompts = captured.lock().unwrap();
    assert_eq!(prompts.len(), 1, "exactly one step dispatched");
    let prompt = &prompts[0];
    assert!(
        prompt.contains("Write about Rust"),
        "{{topic}} must resolve to the input value 'Rust'; prompt was: {prompt}"
    );
    assert!(
        prompt.contains(&format!("with cover {handle}")),
        "{{cover}} must resolve to the input handle string; prompt was: {prompt}"
    );
    assert!(
        !prompt.contains("{{topic}}") && !prompt.contains("{{cover}}"),
        "no placeholders should remain unsubstituted; prompt was: {prompt}"
    );

    kernel.shutdown();
}
