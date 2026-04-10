//! End-to-end workflow integration tests.
//!
//! Tests the full pipeline: boot kernel → spawn agents → create workflow →
//! execute workflow → verify outputs flow through the pipeline.
//!
//! LLM tests require GROQ_API_KEY. Non-LLM tests verify the kernel-level
//! workflow wiring without making real API calls.

use librefang_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowStep,
};
use librefang_kernel::LibreFangKernel;
use librefang_runtime::kernel_handle::KernelHandle;
use librefang_types::agent::AgentManifest;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;

fn test_config(provider: &str, model: &str, api_key_env: &str) -> KernelConfig {
    let tmp = tempfile::tempdir().unwrap();
    KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: api_key_env.to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    }
}

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
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);

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
            },
        ],
        created_at: chrono::Utc::now(),
        account_id: None,
        layout: None,
    };

    let wf_id = kernel.register_workflow(workflow).await;

    // Verify workflow is registered
    let workflows = kernel.workflow_engine().list_workflows().await;
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].name, "alpha-beta-pipeline");

    // Verify agents can be found by name
    let alpha = kernel.agent_registry().find_by_name("agent-alpha");
    assert!(alpha.is_some());
    assert_eq!(alpha.unwrap().id, alpha_id);

    let beta = kernel.agent_registry().find_by_name("agent-beta");
    assert!(beta.is_some());
    assert_eq!(beta.unwrap().id, beta_id);

    // Verify workflow run can be created
    let run_id = kernel
        .workflow_engine()
        .create_run(wf_id, "test input".to_string())
        .await;
    assert!(run_id.is_some());

    let run = kernel
        .workflow_engine()
        .get_run(run_id.unwrap())
        .await
        .unwrap();
    assert_eq!(run.input, "test input");

    kernel.shutdown();
}

/// Test workflow with agent referenced by ID.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_workflow_agent_by_id() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

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
        }],
        created_at: chrono::Utc::now(),
        account_id: None,
        layout: None,
    };

    let wf_id = kernel.register_workflow(workflow).await;

    // Can create run (agent resolution happens at execute time)
    let run_id = kernel
        .workflow_engine()
        .create_run(wf_id, "hello".to_string())
        .await;
    assert!(run_id.is_some());

    kernel.shutdown();
}

/// Test trigger registration and listing at kernel level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trigger_registration_with_kernel() {
    use librefang_kernel::triggers::TriggerPattern;

    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

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
            None,
            agent_id,
            TriggerPattern::Lifecycle,
            "Lifecycle event: {{event}}".to_string(),
            0,
        )
        .unwrap();

    let t2 = kernel
        .register_trigger(
            None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_resolve_workflow_reference_scoped_prefers_owning_tenant() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);

    let workflow_a = Workflow {
        id: WorkflowId::new(),
        name: "shared-name".to_string(),
        description: "".to_string(),
        steps: vec![],
        created_at: chrono::Utc::now(),
        account_id: Some("tenant-a".to_string()),
        layout: None,
    };
    let workflow_b = Workflow {
        id: WorkflowId::new(),
        name: "shared-name".to_string(),
        description: "".to_string(),
        steps: vec![],
        created_at: chrono::Utc::now(),
        account_id: Some("tenant-b".to_string()),
        layout: None,
    };

    let workflow_a_id = kernel.register_workflow(workflow_a).await;
    let workflow_b_id = kernel.register_workflow(workflow_b).await;

    assert_eq!(
        kernel
            .resolve_workflow_reference_scoped("shared-name", Some("tenant-a"))
            .await,
        Some(workflow_a_id)
    );
    assert_eq!(
        kernel
            .resolve_workflow_reference_scoped("shared-name", Some("tenant-b"))
            .await,
        Some(workflow_b_id)
    );
    assert_eq!(
        kernel
            .resolve_workflow_reference_scoped(&workflow_a_id.to_string(), Some("tenant-b"))
            .await,
        None
    );

    kernel.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_register_trigger_with_target_rejects_cross_tenant_target() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

    let owner = spawn_test_agent(&kernel, "owner-agent", "Owner.");
    let target = spawn_test_agent(&kernel, "target-agent", "Target.");
    kernel
        .agent_registry()
        .set_account_id(owner, Some("tenant-a".to_string()))
        .unwrap();
    kernel
        .agent_registry()
        .set_account_id(target, Some("tenant-b".to_string()))
        .unwrap();

    let err = kernel
        .register_trigger_with_target(
            Some("tenant-a".to_string()),
            owner,
            librefang_kernel::triggers::TriggerPattern::System,
            "wake target".to_string(),
            0,
            Some(target),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("owning account"));

    kernel.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_run_and_dry_run_workflow_scoped_reject_cross_tenant_agent_resolution() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

    let target = spawn_test_agent(&kernel, "tenant-b-agent", "Target.");
    kernel
        .agent_registry()
        .set_account_id(target, Some("tenant-b".to_string()))
        .unwrap();

    let by_name = Workflow {
        id: WorkflowId::new(),
        name: "cross-tenant-by-name".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "step-name".to_string(),
            agent: StepAgent::ByName {
                name: "tenant-b-agent".to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 30,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
        }],
        created_at: chrono::Utc::now(),
        account_id: Some("tenant-a".to_string()),
        layout: None,
    };
    let by_id = Workflow {
        id: WorkflowId::new(),
        name: "cross-tenant-by-id".to_string(),
        description: "".to_string(),
        steps: vec![WorkflowStep {
            name: "step-id".to_string(),
            agent: StepAgent::ById {
                id: target.to_string(),
            },
            prompt_template: "{{input}}".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 30,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
        }],
        created_at: chrono::Utc::now(),
        account_id: Some("tenant-a".to_string()),
        layout: None,
    };

    let by_name_id = kernel.register_workflow(by_name).await;
    let by_id_id = kernel.register_workflow(by_id).await;

    let run_name_err = kernel
        .run_workflow_scoped(by_name_id, "input".to_string(), Some("tenant-a"))
        .await
        .unwrap_err()
        .to_string();
    assert!(run_name_err.contains("Agent not found"));

    let run_id_err = kernel
        .run_workflow_scoped(by_id_id, "input".to_string(), Some("tenant-a"))
        .await
        .unwrap_err()
        .to_string();
    assert!(run_id_err.contains("Agent not found"));

    let dry_name = kernel
        .dry_run_workflow_scoped(by_name_id, "input".to_string(), Some("tenant-a"))
        .await
        .expect("dry run succeeds");
    assert_eq!(dry_name.len(), 1);
    assert!(!dry_name[0].agent_found);

    let dry_id = kernel
        .dry_run_workflow_scoped(by_id_id, "input".to_string(), Some("tenant-a"))
        .await
        .expect("dry run succeeds");
    assert_eq!(dry_id.len(), 1);
    assert!(!dry_id[0].agent_found);

    kernel.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cron_create_inherits_agent_account_scope() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

    let agent = spawn_test_agent(&kernel, "cron-owner", "Owner.");
    kernel
        .agent_registry()
        .set_account_id(agent, Some("tenant-a".to_string()))
        .unwrap();

    let result = KernelHandle::cron_create(
        &kernel,
        &agent.to_string(),
        serde_json::json!({
            "name": "tenant-owned-job",
            "schedule": {"kind": "every", "every_secs": 3600},
            "action": {"kind": "agent_turn", "message": "scheduled"},
            "delivery": {"kind": "none"},
            "one_shot": false
        }),
    )
    .await
    .expect("cron create");
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    let job_id = librefang_types::scheduler::CronJobId(
        uuid::Uuid::parse_str(json["job_id"].as_str().unwrap()).unwrap(),
    );
    let job = kernel.cron().get_job(job_id).expect("job should exist");
    assert_eq!(job.account_id.as_deref(), Some("tenant-a"));

    kernel.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cron_list_and_cancel_are_scoped_to_calling_owner() {
    let config = test_config("ollama", "test-model", "OLLAMA_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");

    let agent_a = spawn_test_agent(&kernel, "cron-owner-a", "Owner A.");
    let agent_b = spawn_test_agent(&kernel, "cron-owner-b", "Owner B.");
    kernel
        .agent_registry()
        .set_account_id(agent_a, Some("tenant-a".to_string()))
        .unwrap();
    kernel
        .agent_registry()
        .set_account_id(agent_b, Some("tenant-b".to_string()))
        .unwrap();

    let result_a = KernelHandle::cron_create(
        &kernel,
        &agent_a.to_string(),
        serde_json::json!({
            "name": "tenant-a-job",
            "schedule": {"kind": "every", "every_secs": 3600},
            "action": {"kind": "agent_turn", "message": "scheduled-a"},
            "delivery": {"kind": "none"},
            "one_shot": false
        }),
    )
    .await
    .expect("cron create a");
    let job_a_id = librefang_types::scheduler::CronJobId(
        uuid::Uuid::parse_str(
            serde_json::from_str::<serde_json::Value>(&result_a).unwrap()["job_id"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
    );

    KernelHandle::cron_create(
        &kernel,
        &agent_b.to_string(),
        serde_json::json!({
            "name": "tenant-b-job",
            "schedule": {"kind": "every", "every_secs": 3600},
            "action": {"kind": "agent_turn", "message": "scheduled-b"},
            "delivery": {"kind": "none"},
            "one_shot": false
        }),
    )
    .await
    .expect("cron create b");

    let jobs_a = KernelHandle::cron_list(&kernel, &agent_a.to_string())
        .await
        .expect("cron list a");
    assert_eq!(jobs_a.len(), 1);
    assert_eq!(jobs_a[0]["name"].as_str(), Some("tenant-a-job"));

    let cancel_err =
        KernelHandle::cron_cancel(&kernel, &agent_b.to_string(), &job_a_id.to_string())
            .await
            .unwrap_err();
    assert!(cancel_err.contains("not found"));
    assert!(kernel.cron().get_job(job_a_id).is_some());

    KernelHandle::cron_cancel(&kernel, &agent_a.to_string(), &job_a_id.to_string())
        .await
        .expect("tenant owner cancels own job");
    assert!(kernel.cron().get_job(job_a_id).is_none());

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

    let config = test_config("groq", "llama-3.3-70b-versatile", "GROQ_API_KEY");
    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

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
            },
        ],
        created_at: chrono::Utc::now(),
        account_id: None,
        layout: None,
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
    let run = kernel.workflow_engine().get_run(run_id).await.unwrap();
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
    let runs = kernel.workflow_engine().list_runs(None).await;
    assert_eq!(runs.len(), 1);

    kernel.shutdown();
}
