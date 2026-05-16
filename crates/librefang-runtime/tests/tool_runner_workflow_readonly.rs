// Integration tests for the workflow_list and workflow_status tools (#4844).
//
// Uses the same hand-rolled stub kernel pattern as tool_runner_forwarding.rs —
// a struct that impls every KernelHandle role trait, with the WorkflowRunner
// methods overridden to return controlled data.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Stub kernel with controllable workflow data
// ---------------------------------------------------------------------------

struct WorkflowStubKernel {
    workflows: Vec<WorkflowSummary>,
    run: Option<WorkflowRunSummary>,
}

impl WorkflowStubKernel {
    fn new(workflows: Vec<WorkflowSummary>, run: Option<WorkflowRunSummary>) -> Self {
        Self { workflows, run }
    }
}

#[async_trait]
impl AgentControl for WorkflowStubKernel {
    async fn spawn_agent(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn send_to_agent(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for WorkflowStubKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _key: &str,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}
impl WikiAccess for WorkflowStubKernel {}

#[async_trait]
impl TaskQueue for WorkflowStubKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_claim(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_complete(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_get(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_update_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl EventBus for WorkflowStubKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for WorkflowStubKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not implemented".into())
    }
}

impl CronControl for WorkflowStubKernel {}
impl ApprovalGate for WorkflowStubKernel {}
impl HandsControl for WorkflowStubKernel {}
impl A2ARegistry for WorkflowStubKernel {}
impl ChannelSender for WorkflowStubKernel {}
impl PromptStore for WorkflowStubKernel {}
impl GoalControl for WorkflowStubKernel {}
impl ToolPolicy for WorkflowStubKernel {}
impl librefang_kernel_handle::CatalogQuery for WorkflowStubKernel {}

impl librefang_kernel_handle::ApiAuth for WorkflowStubKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for WorkflowStubKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for WorkflowStubKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for WorkflowStubKernel {}

#[async_trait]
impl WorkflowRunner for WorkflowStubKernel {
    async fn list_workflows(&self) -> Vec<WorkflowSummary> {
        self.workflows.clone()
    }

    async fn get_workflow_run(&self, _run_id: &str) -> Option<WorkflowRunSummary> {
        self.run.clone()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_ctx(kernel: &Arc<dyn KernelHandle>) -> ToolExecContext<'_> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"), // mock-only: non-UUID ok since mock kernel ignores agent_id
        skill_registry: None,
        allowed_skills: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id: None,
        channel: None,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

fn sample_run() -> WorkflowRunSummary {
    WorkflowRunSummary::new(
        "11111111-1111-1111-1111-111111111111".to_string(),
        "22222222-2222-2222-2222-222222222222".to_string(),
        "bug-triage".to_string(),
        "completed".to_string(),
        "2026-01-01T00:00:00+00:00".to_string(),
        Some("2026-01-01T00:01:00+00:00".to_string()),
        Some("triage complete".to_string()),
        None,
        2,
        Some("summarise".to_string()),
        vec![],
    )
}

// ---------------------------------------------------------------------------
// Tool definition presence tests
// ---------------------------------------------------------------------------

#[test]
fn test_workflow_list_and_status_appear_in_builtin_definitions() {
    let defs = builtin_tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&"workflow_list"),
        "workflow_list missing from builtin_tool_definitions"
    );
    assert!(
        names.contains(&"workflow_status"),
        "workflow_status missing from builtin_tool_definitions"
    );
}

#[test]
fn test_workflow_list_definition_schema() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_list")
        .expect("workflow_list definition");
    assert_eq!(def.input_schema["type"], "object");
}

#[test]
fn test_workflow_status_definition_schema() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_status")
        .expect("workflow_status definition");
    assert_eq!(def.input_schema["type"], "object");
    assert_eq!(
        def.input_schema["required"][0], "run_id",
        "run_id must be required"
    );
}

// ---------------------------------------------------------------------------
// workflow_list round-trip tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workflow_list_returns_sorted_by_name() {
    // Supply 3 workflows in reverse alphabetical order — output must be sorted.
    let workflows = vec![
        WorkflowSummary::new(
            "c".to_string(),
            "zebra-review".to_string(),
            "Last alphabetically".to_string(),
            1,
            false,
        ),
        WorkflowSummary::new(
            "a".to_string(),
            "alpha-pipeline".to_string(),
            "First alphabetically".to_string(),
            5,
            false,
        ),
        WorkflowSummary::new(
            "b".to_string(),
            "middle-flow".to_string(),
            "Middle".to_string(),
            3,
            false,
        ),
    ];
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(workflows, None));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_list", &json!({}), &ctx).await;
    assert!(!result.is_error, "workflow_list failed: {}", result.content);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&result.content).expect("valid JSON array");
    assert_eq!(parsed.len(), 3, "expected 3 workflows");
    assert_eq!(parsed[0]["name"], "alpha-pipeline");
    assert_eq!(parsed[1]["name"], "middle-flow");
    assert_eq!(parsed[2]["name"], "zebra-review");
    // Verify step_count is present
    assert_eq!(parsed[0]["step_count"], 5);
}

#[tokio::test]
async fn test_workflow_list_fields_present() {
    let workflows = vec![WorkflowSummary::new(
        "aabbccdd-0000-0000-0000-000000000000".to_string(),
        "code-review".to_string(),
        "Automated code review pipeline".to_string(),
        4,
        true,
    )];
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(workflows, None));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_list", &json!({}), &ctx).await;
    assert!(!result.is_error, "{}", result.content);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&result.content).expect("valid JSON array");
    let entry = &parsed[0];
    assert_eq!(entry["id"], "aabbccdd-0000-0000-0000-000000000000");
    assert_eq!(entry["name"], "code-review");
    assert_eq!(entry["description"], "Automated code review pipeline");
    assert_eq!(entry["step_count"], 4);
}

#[tokio::test]
async fn test_workflow_list_empty() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(vec![], None));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_list", &json!({}), &ctx).await;
    assert!(!result.is_error, "{}", result.content);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&result.content).expect("valid JSON array");
    assert!(parsed.is_empty());
}

// Determinism: calling workflow_list twice with the same (already-sorted) data
// produces byte-identical output. The kernel impl is responsible for sorting;
// the tool itself is a pure pass-through that must not reorder.
#[tokio::test]
async fn test_workflow_list_output_is_deterministic() {
    let workflows = vec![
        WorkflowSummary::new(
            "1".to_string(),
            "apple".to_string(),
            "d1".to_string(),
            1,
            false,
        ),
        WorkflowSummary::new(
            "2".to_string(),
            "banana".to_string(),
            "d2".to_string(),
            2,
            false,
        ),
    ];
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(workflows, None));
    let ctx = make_ctx(&kernel);

    let r1 = execute_tool_raw("t1", "workflow_list", &json!({}), &ctx).await;
    let r2 = execute_tool_raw("t2", "workflow_list", &json!({}), &ctx).await;
    assert!(!r1.is_error);
    assert!(!r2.is_error);
    assert_eq!(
        r1.content, r2.content,
        "workflow_list output must be byte-identical across calls"
    );
}

// ---------------------------------------------------------------------------
// workflow_status round-trip tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workflow_status_all_fields_map_correctly() {
    let run = sample_run();
    let run_id = run.run_id.clone();
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(vec![], Some(run)));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_status", &json!({"run_id": run_id}), &ctx).await;
    assert!(
        !result.is_error,
        "workflow_status failed: {}",
        result.content
    );

    let v: serde_json::Value = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(v["run_id"], "11111111-1111-1111-1111-111111111111");
    assert_eq!(v["workflow_id"], "22222222-2222-2222-2222-222222222222");
    assert_eq!(v["workflow_name"], "bug-triage");
    assert_eq!(v["state"], "completed");
    assert_eq!(v["started_at"], "2026-01-01T00:00:00+00:00");
    assert_eq!(v["completed_at"], "2026-01-01T00:01:00+00:00");
    assert_eq!(v["output"], "triage complete");
    assert!(v["error"].is_null());
    assert_eq!(v["step_count"], 2);
    assert_eq!(v["last_step_name"], "summarise");
}

#[tokio::test]
async fn test_workflow_status_not_found_returns_error() {
    // Stub returns None for any run — simulates run not found.
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(vec![], None));
    let ctx = make_ctx(&kernel);

    let valid_uuid = "33333333-3333-3333-3333-333333333333";
    let result = execute_tool_raw(
        "t1",
        "workflow_status",
        &json!({"run_id": valid_uuid}),
        &ctx,
    )
    .await;
    assert!(result.is_error, "expected error for unknown run");
    assert!(
        result.content.contains("not found"),
        "error message should mention 'not found': {}",
        result.content
    );
}

#[tokio::test]
async fn test_workflow_status_invalid_uuid_returns_error() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(vec![], None));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_status",
        &json!({"run_id": "not-a-uuid"}),
        &ctx,
    )
    .await;
    assert!(result.is_error, "expected error for invalid UUID");
    assert!(
        result.content.contains("Invalid run_id") || result.content.contains("UUID"),
        "error message should mention UUID: {}",
        result.content
    );
}

#[tokio::test]
async fn test_workflow_status_missing_run_id_returns_error() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowStubKernel::new(vec![], None));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_status", &json!({}), &ctx).await;
    assert!(result.is_error, "expected error for missing run_id");
}
