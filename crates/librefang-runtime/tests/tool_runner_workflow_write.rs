// Integration tests for the workflow_start, workflow_cancel and workflow_create tools (#4844 section E, #6934).
//
// Uses the same hand-rolled stub kernel pattern as tool_runner_workflow_readonly.rs.
// The write-side stub extends WorkflowRunner with start_workflow_async, cancel_workflow_run and create_workflow implementations driven by per-test configuration.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

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
// Outcome the stub create_workflow returns
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum StubCreateResult {
    Ok,
    /// The spec could not become a workflow — the kernel's `InvalidInput`.
    Invalid(&'static str),
    /// The name is already registered — the kernel's `Conflict`.
    NameTaken(&'static str),
}

// ---------------------------------------------------------------------------
// Stub kernel for write-tool tests
// ---------------------------------------------------------------------------

struct WorkflowWriteStubKernel {
    /// run_id returned by start_workflow_async (None → simulate resolution error)
    start_run_id: Option<String>,
    cancel_result: StubCancelResult,
    create_result: StubCreateResult,
    /// The `(spec, caller_agent_id)` pair the last create_workflow call received, so the tests can assert what actually crossed the trait boundary rather than only what came back.
    create_seen: Mutex<Option<(serde_json::Value, Option<String>)>>,
}

impl WorkflowWriteStubKernel {
    fn base() -> Self {
        Self {
            start_run_id: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
            cancel_result: StubCancelResult::Ok,
            create_result: StubCreateResult::Ok,
            create_seen: Mutex::new(None),
        }
    }

    fn with_start(run_id: &str) -> Self {
        Self {
            start_run_id: Some(run_id.to_string()),
            ..Self::base()
        }
    }

    fn start_error() -> Self {
        Self {
            start_run_id: None,
            ..Self::base()
        }
    }

    fn with_cancel(cancel_result: StubCancelResult) -> Self {
        Self {
            cancel_result,
            ..Self::base()
        }
    }

    fn with_create(create_result: StubCreateResult) -> Self {
        Self {
            create_result,
            ..Self::base()
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

    async fn create_workflow(
        &self,
        spec: &serde_json::Value,
        caller_agent_id: Option<&str>,
    ) -> Result<WorkflowSummary, librefang_kernel_handle::KernelOpError> {
        *self.create_seen.lock().unwrap() =
            Some((spec.clone(), caller_agent_id.map(str::to_string)));
        match &self.create_result {
            StubCreateResult::Ok => Ok(WorkflowSummary::new(
                "11111111-1111-1111-1111-111111111111".to_string(),
                spec["name"].as_str().unwrap_or_default().to_string(),
                spec["description"].as_str().unwrap_or_default().to_string(),
                spec["steps"].as_array().map(|a| a.len()).unwrap_or(0),
                spec.get("input_schema").is_some(),
            )),
            StubCreateResult::Invalid(reason) => Err(
                librefang_kernel_handle::KernelOpError::InvalidInput(reason.to_string()),
            ),
            StubCreateResult::NameTaken(reason) => Err(
                librefang_kernel_handle::KernelOpError::Conflict(reason.to_string()),
            ),
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
// workflow_create tests (#6934)
// ---------------------------------------------------------------------------

/// A well-formed creation payload.
/// Individual tests mutate one field of it so the thing under test is the only difference from a known-good spec.
fn create_payload() -> serde_json::Value {
    json!({
        "name": "nightly-report",
        "description": "summarise the day",
        "steps": [
            { "name": "gather", "agent": "researcher", "prompt_template": "Collect today's {{topic}} news" },
            { "name": "write", "agent": "writer", "prompt_template": "Summarise {{input}}", "depends_on": ["gather"] }
        ],
        "input_schema": [ { "name": "topic", "param_type": "string" } ]
    })
}

#[test]
fn workflow_create_appears_in_builtin_definitions() {
    let defs = builtin_tool_definitions();
    let def = defs
        .iter()
        .find(|d| d.name == "workflow_create")
        .expect("workflow_create missing from builtin_tool_definitions");
    assert_eq!(def.input_schema["type"], "object");
    let required: Vec<&str> = def.input_schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        required,
        vec!["name", "steps"],
        "name and steps are the only fields a workflow cannot be built without"
    );
    // A step needs somewhere to run and something to say — the schema must say so, because the kernel rejects a step missing either and the model needs to know before it spends a turn on the call.
    let step_required: Vec<&str> = def.input_schema["properties"]["steps"]["items"]["required"]
        .as_array()
        .expect("step required array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(step_required, vec!["name", "agent", "prompt_template"]);
}

#[tokio::test]
async fn workflow_create_forwards_the_whole_spec_and_the_caller() {
    let stub = Arc::new(WorkflowWriteStubKernel::with_create(StubCreateResult::Ok));
    let kernel: Arc<dyn KernelHandle> = stub.clone();
    let ctx = make_ctx(&kernel);

    let payload = create_payload();
    let result = execute_tool_raw("t1", "workflow_create", &payload, &ctx).await;
    assert!(
        !result.is_error,
        "workflow_create failed: {}",
        result.content
    );

    let v: serde_json::Value = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(v["id"], "11111111-1111-1111-1111-111111111111");
    assert_eq!(v["name"], "nightly-report");
    assert_eq!(v["step_count"], 2);
    assert_eq!(v["has_input_schema"], true);

    let (seen_spec, seen_caller) = stub
        .create_seen
        .lock()
        .unwrap()
        .clone()
        .expect("the kernel handle must have been called");
    assert_eq!(
        seen_spec, payload,
        "the spec must reach the kernel unmodified — the handler is not allowed to rebuild a subset of it"
    );
    assert_eq!(
        seen_caller.as_deref(),
        Some("test-agent"),
        "the caller must be forwarded so the registration can be traced back to a turn"
    );
}

#[tokio::test]
async fn workflow_create_missing_name_returns_error() {
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(WorkflowWriteStubKernel::with_create(StubCreateResult::Ok));
    let ctx = make_ctx(&kernel);

    let mut payload = create_payload();
    payload.as_object_mut().unwrap().remove("name");
    let result = execute_tool_raw("t1", "workflow_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for missing name");
    assert!(
        result.content.contains("name"),
        "error should mention name: {}",
        result.content
    );
}

#[tokio::test]
async fn workflow_create_missing_or_malformed_steps_returns_error() {
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(WorkflowWriteStubKernel::with_create(StubCreateResult::Ok));
    let ctx = make_ctx(&kernel);

    let mut payload = create_payload();
    payload.as_object_mut().unwrap().remove("steps");
    let result = execute_tool_raw("t1", "workflow_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for missing steps");
    assert!(
        result.content.contains("steps"),
        "error should mention steps: {}",
        result.content
    );

    // A single step object instead of an array is the shape a model most often gets wrong, and it must be named as such rather than reaching the kernel as an opaque deserialization failure.
    let mut payload = create_payload();
    payload["steps"] = json!({ "name": "write", "agent": "writer", "prompt_template": "go" });
    let result = execute_tool_raw("t1", "workflow_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for a non-array steps");
    assert!(
        result.content.contains("array"),
        "error should say steps must be an array: {}",
        result.content
    );
}

/// A name collision is something the model can fix on its next turn — by picking another name — so the rejection has to arrive as a readable reason against the `name` field, not as an opaque upstream failure.
#[tokio::test]
async fn workflow_create_relays_a_name_collision() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_create(
        StubCreateResult::NameTaken("a workflow named 'nightly-report' already exists (id 42)"),
    ));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_create", &create_payload(), &ctx).await;
    assert!(result.is_error, "expected error for a taken name");
    assert!(
        result.content.contains("already exists"),
        "the collision reason must survive to the model: {}",
        result.content
    );
    assert!(
        result.content.contains("name"),
        "the collision must be attributed to the name field: {}",
        result.content
    );
}

/// Same for a spec the kernel refused: the reason names the field and the limit, and a model that cannot see which one it broke retries the same payload.
#[tokio::test]
async fn workflow_create_relays_a_validation_failure() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(WorkflowWriteStubKernel::with_create(
        StubCreateResult::Invalid(
            "step 'write' depends on 'research', which is not a step in this workflow",
        ),
    ));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "workflow_create", &create_payload(), &ctx).await;
    assert!(result.is_error, "expected error for an invalid spec");
    assert!(
        result.content.contains("not a step in this workflow"),
        "the validation reason must survive to the model: {}",
        result.content
    );
}
