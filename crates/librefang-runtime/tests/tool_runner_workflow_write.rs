// Integration tests for the workflow_start and workflow_cancel tools (#4844 section E).
//
// Uses the same hand-rolled stub kernel pattern as tool_runner_workflow_readonly.rs.
// The write-side stub extends WorkflowRunner with start_workflow_async and
// cancel_workflow_run implementations driven by per-test configuration.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Error sentinel returned by stub cancel_workflow_run
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum StubCancelResult {
    Ok,
    NotFound,
    AlreadyTerminal { state: &'static str },
}

// ---------------------------------------------------------------------------
// Stub kernel for write-tool tests
// ---------------------------------------------------------------------------

struct WorkflowWriteStubKernel {
    /// run_id returned by start_workflow_async (None → simulate resolution error)
    start_run_id: Option<String>,
    cancel_result: StubCancelResult,
    /// Every `(workflow_json, caller_agent_id)` pair `create_workflow` was handed,
    /// so a test can assert on the payload the tool built rather than only on the
    /// tool's own return value (#6943).
    created: std::sync::Mutex<Vec<(String, Option<String>)>>,
}

impl WorkflowWriteStubKernel {
    fn with_start(run_id: &str) -> Self {
        Self {
            start_run_id: Some(run_id.to_string()),
            cancel_result: StubCancelResult::Ok,
            created: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn start_error() -> Self {
        Self {
            start_run_id: None,
            cancel_result: StubCancelResult::Ok,
            created: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_cancel(cancel_result: StubCancelResult) -> Self {
        Self {
            start_run_id: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
            cancel_result,
            created: std::sync::Mutex::new(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Boilerplate trait impls (same pattern as tool_runner_workflow_readonly.rs)
// ---------------------------------------------------------------------------

#[async_trait]
impl AgentControl for WorkflowWriteStubKernel {
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

impl MemoryAccess for WorkflowWriteStubKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

impl WikiAccess for WorkflowWriteStubKernel {}

#[async_trait]
impl TaskQueue for WorkflowWriteStubKernel {
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
impl EventBus for WorkflowWriteStubKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for WorkflowWriteStubKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
        _: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not implemented".into())
    }
}

impl CronControl for WorkflowWriteStubKernel {}
impl ApprovalGate for WorkflowWriteStubKernel {}
impl HandsControl for WorkflowWriteStubKernel {}
impl A2ARegistry for WorkflowWriteStubKernel {}
impl ChannelSender for WorkflowWriteStubKernel {}
impl PromptStore for WorkflowWriteStubKernel {}
impl GoalControl for WorkflowWriteStubKernel {}
impl ToolPolicy for WorkflowWriteStubKernel {}
impl librefang_kernel_handle::CatalogQuery for WorkflowWriteStubKernel {}

impl librefang_kernel_handle::ApiAuth for WorkflowWriteStubKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for WorkflowWriteStubKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for WorkflowWriteStubKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for WorkflowWriteStubKernel {}

#[async_trait]
impl WorkflowRunner for WorkflowWriteStubKernel {
    async fn list_workflows(&self) -> Vec<WorkflowSummary> {
        vec![]
    }

    async fn get_workflow_run(&self, _run_id: &str) -> Option<WorkflowRunSummary> {
        None
    }

    async fn start_workflow_async_tracked(
        &self,
        _workflow_id: &str,
        _input: &str,
        _caller_agent_id: Option<&str>,
        _caller_session_id: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        match &self.start_run_id {
            Some(id) => Ok(id.clone()),
            None => Err(librefang_kernel_handle::KernelOpError::Internal(
                "workflow `unknown-workflow` not found".to_string(),
            )),
        }
    }

    async fn cancel_workflow_run(
        &self,
        run_id: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        match &self.cancel_result {
            StubCancelResult::Ok => Ok(()),
            StubCancelResult::NotFound => Err(librefang_kernel_handle::KernelOpError::Internal(
                format!("workflow run not found: {run_id}"),
            )),
            StubCancelResult::AlreadyTerminal { state } => {
                Err(librefang_kernel_handle::KernelOpError::Internal(format!(
                    "cannot cancel: run is already {state}"
                )))
            }
        }
    }

    async fn create_workflow(
        &self,
        workflow_json: &str,
        caller_agent_id: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.created
            .lock()
            .expect("create_workflow capture mutex must not be poisoned")
            .push((
                workflow_json.to_string(),
                caller_agent_id.map(str::to_string),
            ));
        let parsed: serde_json::Value =
            serde_json::from_str(workflow_json).expect("tool must emit valid JSON");
        Ok(parsed["name"].as_str().unwrap_or_default().to_lowercase())
    }
}

// ---------------------------------------------------------------------------
// Helper: build a minimal ToolExecContext
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
        chat_id: None,
        sender_account_id: None,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

// ---------------------------------------------------------------------------
// Tool definition presence tests
// ---------------------------------------------------------------------------

#[test]
fn workflow_start_and_workflow_cancel_appear_in_builtin_definitions() {
    let defs = builtin_tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&"workflow_start"),
        "workflow_start missing from builtin_tool_definitions"
    );
    assert!(
        names.contains(&"workflow_cancel"),
        "workflow_cancel missing from builtin_tool_definitions"
    );
}

// ---------------------------------------------------------------------------
// workflow_start tests
// ---------------------------------------------------------------------------

#[test]
fn workflow_start_definition_schema_correct() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_start")
        .expect("workflow_start definition");
    assert_eq!(def.input_schema["type"], "object");
    assert_eq!(
        def.input_schema["required"][0], "workflow_id",
        "workflow_id must be required"
    );
    // input parameter must be present but NOT required
    assert!(
        def.input_schema["properties"]["input"].is_object(),
        "input property should exist"
    );
    let required = def.input_schema["required"]
        .as_array()
        .expect("required array");
    assert!(
        !required.iter().any(|v| v == "input"),
        "input should not be required"
    );
}

#[tokio::test]
async fn workflow_start_returns_run_id_and_does_not_block() {
    let fixed_run_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_start(fixed_run_id));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_start",
        &json!({"workflow_id": "bug-triage"}),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "workflow_start failed: {}",
        result.content
    );

    let v: serde_json::Value = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(v["run_id"], fixed_run_id);
    // Only run_id field — no output field (fire-and-forget, not blocking).
    assert!(v.get("output").is_none(), "output should not be present");
}

#[tokio::test]
async fn workflow_start_missing_workflow_id_returns_error() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_start("any-run-id"));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_start", &json!({}), &ctx).await;
    assert!(result.is_error, "expected error for missing workflow_id");
    assert!(
        result.content.contains("workflow_id"),
        "error should mention workflow_id: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_start_definition_not_found_returns_error() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::start_error());
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_start",
        &json!({"workflow_id": "unknown-workflow"}),
        &ctx,
    )
    .await;
    assert!(result.is_error, "expected error for unknown workflow");
    assert!(
        result.content.contains("not found"),
        "error should mention not found: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_start_output_is_deterministic() {
    let fixed_run_id = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_start(fixed_run_id));
    let ctx = make_ctx(&kernel);

    let input = json!({"workflow_id": "code-review"});
    let r1 = execute_tool_raw("t1", "workflow_start", &input, &ctx).await;
    let r2 = execute_tool_raw("t2", "workflow_start", &input, &ctx).await;

    assert!(!r1.is_error);
    assert!(!r2.is_error);
    assert_eq!(
        r1.content, r2.content,
        "workflow_start output must be byte-identical across calls"
    );
}

// ---------------------------------------------------------------------------
// workflow_cancel tests
// ---------------------------------------------------------------------------

#[test]
fn workflow_cancel_definition_schema_correct() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_cancel")
        .expect("workflow_cancel definition");
    assert_eq!(def.input_schema["type"], "object");
    assert_eq!(
        def.input_schema["required"][0], "run_id",
        "run_id must be required"
    );
}

#[tokio::test]
async fn workflow_cancel_success_returns_state_cancelled() {
    let run_id = "dddddddd-dddd-dddd-dddd-dddddddddddd";
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(WorkflowWriteStubKernel::with_cancel(StubCancelResult::Ok));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_cancel", &json!({"run_id": run_id}), &ctx).await;
    assert!(
        !result.is_error,
        "workflow_cancel failed: {}",
        result.content
    );

    let v: serde_json::Value = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(v["run_id"], run_id);
    assert_eq!(v["state"], "cancelled");
}

#[tokio::test]
async fn workflow_cancel_not_found_returns_error_with_run_id() {
    let run_id = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_cancel(
        StubCancelResult::NotFound,
    ));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_cancel", &json!({"run_id": run_id}), &ctx).await;
    assert!(result.is_error, "expected error for not-found run");
    assert!(
        result.content.contains("not found"),
        "error should mention 'not found': {}",
        result.content
    );
    assert!(
        result.content.contains(run_id),
        "error should contain the run_id: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_cancel_invalid_uuid_returns_error_before_kernel_call() {
    // A malformed run_id must be rejected at the tool layer before the kernel
    // is ever called. The stub would succeed, so any error here is the UUID guard.
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(WorkflowWriteStubKernel::with_cancel(StubCancelResult::Ok));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_cancel",
        &json!({"run_id": "not-a-uuid"}),
        &ctx,
    )
    .await;
    assert!(result.is_error, "expected error for invalid UUID");
    assert!(
        result.content.contains("UUID") || result.content.contains("Invalid"),
        "error should mention UUID validation: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_cancel_already_terminal_returns_error_with_state() {
    let run_id = "ffffffff-ffff-ffff-ffff-ffffffffffff";
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_cancel(
        StubCancelResult::AlreadyTerminal { state: "completed" },
    ));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_cancel", &json!({"run_id": run_id}), &ctx).await;
    assert!(result.is_error, "expected error for already-terminal run");
    assert!(
        result.content.contains("already") && result.content.contains("completed"),
        "error should describe terminal state: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_cancel_missing_run_id_returns_error() {
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(WorkflowWriteStubKernel::with_cancel(StubCancelResult::Ok));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_cancel", &json!({}), &ctx).await;
    assert!(result.is_error, "expected error for missing run_id");
    assert!(
        result.content.contains("run_id"),
        "error should mention run_id: {}",
        result.content
    );
}

// ---------------------------------------------------------------------------
// workflow_create tests (#6943)
// ---------------------------------------------------------------------------

/// Build a stub plus the `Arc<dyn KernelHandle>` view of it, so a test can both
/// drive `execute_tool_raw` and inspect what reached `create_workflow`.
fn create_stub() -> (Arc<WorkflowWriteStubKernel>, Arc<dyn KernelHandle>) {
    let stub = Arc::new(WorkflowWriteStubKernel::with_start("unused-run-id"));
    let kernel: Arc<dyn KernelHandle> = stub.clone();
    (stub, kernel)
}

fn one_step() -> serde_json::Value {
    json!([{ "name": "only-step", "agent": "assistant", "prompt_template": "{{input}}" }])
}

/// The single `(workflow_json, caller)` pair the stub captured, parsed.
fn sole_capture(stub: &WorkflowWriteStubKernel) -> (serde_json::Value, Option<String>) {
    let captured = stub.created.lock().expect("capture mutex");
    assert_eq!(
        captured.len(),
        1,
        "expected exactly one create_workflow call"
    );
    let (payload, caller) = &captured[0];
    (
        serde_json::from_str(payload).expect("captured payload must be valid JSON"),
        caller.clone(),
    )
}

#[test]
fn workflow_create_appears_in_builtin_definitions() {
    let defs = builtin_tool_definitions();
    assert!(
        defs.iter().any(|d| d.name == "workflow_create"),
        "workflow_create missing from builtin_tool_definitions"
    );
}

#[test]
fn workflow_create_definition_schema_declares_required_fields_and_caps() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_create")
        .expect("workflow_create definition");
    let schema = &def.input_schema;

    assert_eq!(schema["type"], "object");
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|v| v.as_str().expect("required entries are strings"))
        .collect();
    assert_eq!(required, vec!["name", "steps"]);

    // The advertised caps must match what the handler enforces, otherwise the
    // model is told one ceiling and rejected at another.
    assert_eq!(schema["properties"]["steps"]["maxItems"], 50);
    assert_eq!(
        schema["properties"]["steps"]["items"]["properties"]["timeout_secs"]["maximum"],
        3600
    );
    assert_eq!(schema["properties"]["total_timeout_secs"]["maximum"], 86400);

    // input_schema entries are keyed on `param_type` — the spelling
    // workflow_describe reports — so a described workflow round-trips.
    let param = &schema["properties"]["input_schema"]["items"];
    assert!(
        param["properties"]["param_type"].is_object(),
        "input_schema items must declare param_type"
    );
    assert!(
        param["properties"]["type"].is_null(),
        "the bare `type` key must no longer be advertised"
    );
    let param_required: Vec<&str> = param["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|v| v.as_str().expect("required entries are strings"))
        .collect();
    assert_eq!(param_required, vec!["name", "param_type"]);
}

#[tokio::test]
async fn workflow_create_forwards_the_workflow_and_the_calling_agent() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({ "name": "bug-triage", "description": "Triage bugs", "steps": one_step() }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "workflow_create failed: {}",
        result.content
    );
    assert_eq!(result.content, "bug-triage");

    let (payload, caller) = sole_capture(&stub);
    assert_eq!(payload["name"], "bug-triage");
    assert_eq!(payload["description"], "Triage bugs");
    assert_eq!(payload["steps"], one_step());
    // The caller must survive the tool -> KernelHandle hop; the kernel impl logs
    // it as the workflow's only audit trail (#6943 review).
    assert_eq!(caller.as_deref(), Some("test-agent"));
}

#[tokio::test]
async fn workflow_create_missing_name_returns_error() {
    let (_stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({ "steps": one_step() }),
        &ctx,
    )
    .await;
    assert!(result.is_error, "expected error for missing name");
    assert!(
        result.content.contains("name"),
        "error should mention name: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_create_rejects_names_outside_the_shared_charset() {
    for bad in ["bad name", "../escape", "a/b", ""] {
        let (stub, kernel) = create_stub();
        let ctx = make_ctx(&kernel);

        let result = execute_tool_raw(
            "t1",
            "workflow_create",
            &json!({ "name": bad, "steps": one_step() }),
            &ctx,
        )
        .await;
        assert!(result.is_error, "{bad:?} must be rejected");
        assert!(
            result.content.contains("1-64") || result.content.contains("A-Za-z0-9"),
            "the rejection must name the shared length/charset rule: {}",
            result.content
        );
        assert!(
            stub.created.lock().expect("capture mutex").is_empty(),
            "a rejected name must never reach the kernel"
        );
    }
}

#[tokio::test]
async fn workflow_create_rejects_a_missing_or_empty_step_list() {
    for steps in [json!([]), json!("not-an-array")] {
        let (stub, kernel) = create_stub();
        let ctx = make_ctx(&kernel);

        let result = execute_tool_raw(
            "t1",
            "workflow_create",
            &json!({ "name": "wf", "steps": steps }),
            &ctx,
        )
        .await;
        assert!(result.is_error, "expected error for steps={steps}");
        assert!(
            result.content.contains("step"),
            "error should mention steps: {}",
            result.content
        );
        assert!(stub.created.lock().expect("capture mutex").is_empty());
    }
}

#[tokio::test]
async fn workflow_create_enforces_the_step_count_ceiling() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let steps: Vec<serde_json::Value> = (0..51)
        .map(|i| json!({ "name": format!("s{i}"), "agent": "assistant", "prompt_template": "go" }))
        .collect();
    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({ "name": "too-many", "steps": steps }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "51 steps must be rejected");
    assert!(
        result.content.contains("at most 50"),
        "error should name the ceiling: {}",
        result.content
    );
    assert!(
        stub.created.lock().expect("capture mutex").is_empty(),
        "an over-cap workflow must never be persisted"
    );
}

#[tokio::test]
async fn workflow_create_accepts_a_workflow_exactly_at_the_step_ceiling() {
    let (_stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let steps: Vec<serde_json::Value> = (0..50)
        .map(|i| json!({ "name": format!("s{i}"), "agent": "assistant", "prompt_template": "go" }))
        .collect();
    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({ "name": "exactly-fifty", "steps": steps }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "the cap must be inclusive: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_create_enforces_the_step_and_total_timeout_ceilings() {
    let cases = [
        (
            json!({
                "name": "slow-step",
                "steps": [{ "name": "s", "agent": "a", "prompt_template": "p", "timeout_secs": 3601 }],
            }),
            "3600",
        ),
        (
            json!({ "name": "slow-total", "steps": one_step(), "total_timeout_secs": 86_401 }),
            "86400",
        ),
    ];

    for (payload, ceiling) in cases {
        let (stub, kernel) = create_stub();
        let ctx = make_ctx(&kernel);

        let result = execute_tool_raw("t1", "workflow_create", &payload, &ctx).await;
        assert!(result.is_error, "expected rejection for {payload}");
        assert!(
            result.content.contains(ceiling),
            "error should name the {ceiling}s ceiling: {}",
            result.content
        );
        assert!(stub.created.lock().expect("capture mutex").is_empty());
    }
}

#[tokio::test]
async fn workflow_create_preserves_declared_input_parameter_types() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    // `param_type` is canonical; `type` is the alias the schema used to
    // advertise. Both must survive, because before this fix neither did:
    // WorkflowInputParam deserializes `param_type` only, so an entry keyed on
    // `type` fell back to the "string" default and the declared type was
    // silently lost (#6943 review).
    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({
            "name": "typed",
            "steps": one_step(),
            "input_schema": [
                { "name": "canonical", "param_type": "number", "required": true },
                { "name": "aliased", "type": "image", "description": "a picture" },
            ],
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "workflow_create failed: {}",
        result.content
    );

    let (payload, _) = sole_capture(&stub);
    let schema = payload["input_schema"]
        .as_array()
        .expect("input_schema must be forwarded as an array");
    assert_eq!(schema[0]["param_type"], "number");
    assert_eq!(schema[0]["required"], true);
    assert_eq!(schema[1]["param_type"], "image");
    assert_eq!(schema[1]["description"], "a picture");
    // The alias must be consumed, not forwarded alongside its replacement.
    assert!(
        schema.iter().all(|p| p.get("type").is_none()),
        "the `type` alias must be rewritten, not duplicated: {payload}"
    );
}

#[tokio::test]
async fn workflow_create_prefers_param_type_when_both_spellings_are_present() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({
            "name": "both",
            "steps": one_step(),
            "input_schema": [{ "name": "p", "param_type": "boolean", "type": "number" }],
        }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "workflow_create failed: {}",
        result.content
    );
    let (payload, _) = sole_capture(&stub);
    assert_eq!(payload["input_schema"][0]["param_type"], "boolean");
}

#[tokio::test]
async fn workflow_create_rejects_an_unknown_input_parameter_type() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({
            "name": "bad-type",
            "steps": one_step(),
            "input_schema": [{ "name": "p", "param_type": "integer" }],
        }),
        &ctx,
    )
    .await;

    assert!(result.is_error, "an unknown param_type must be rejected");
    assert!(
        result.content.contains("integer") && result.content.contains("agent_id"),
        "error should name the offending value and the accepted set: {}",
        result.content
    );
    assert!(stub.created.lock().expect("capture mutex").is_empty());
}

#[tokio::test]
async fn workflow_create_rejects_a_malformed_input_schema() {
    for schema in [json!("nope"), json!(["nope"])] {
        let (stub, kernel) = create_stub();
        let ctx = make_ctx(&kernel);

        let result = execute_tool_raw(
            "t1",
            "workflow_create",
            &json!({ "name": "wf", "steps": one_step(), "input_schema": schema }),
            &ctx,
        )
        .await;
        assert!(
            result.is_error,
            "expected rejection for input_schema={schema}"
        );
        assert!(
            result.content.contains("input_schema"),
            "error should mention input_schema: {}",
            result.content
        );
        assert!(stub.created.lock().expect("capture mutex").is_empty());
    }
}

#[tokio::test]
async fn workflow_create_omits_input_schema_when_none_was_declared() {
    let (stub, kernel) = create_stub();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw(
        "t1",
        "workflow_create",
        &json!({ "name": "no-params", "steps": one_step() }),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "workflow_create failed: {}",
        result.content
    );
    let (payload, _) = sole_capture(&stub);
    // Null rather than an empty array: `Workflow::input_schema` is an Option, and
    // an empty Vec would claim the workflow declares zero parameters instead of
    // leaving auto-detection from {{var}} placeholders enabled.
    assert!(
        payload["input_schema"].is_null(),
        "absent input_schema must stay absent: {payload}"
    );
}
