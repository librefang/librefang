use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

type TaskPostCalls = Arc<Mutex<Vec<Option<String>>>>;
type CronCreateCalls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;
type TaskGetCalls = Arc<Mutex<Vec<String>>>;

struct CapturedCalls {
    task_post: TaskPostCalls,
    cron_create: CronCreateCalls,
    task_get: TaskGetCalls,
}

struct CapturingKernel {
    task_post_calls: TaskPostCalls,
    cron_create_calls: CronCreateCalls,
    task_get_calls: TaskGetCalls,
    // When set, task_get returns Some(this) regardless of id; otherwise None.
    task_get_response: Mutex<Option<serde_json::Value>>,
}

impl CapturingKernel {
    fn new() -> (Self, CapturedCalls) {
        let task_post = Arc::new(Mutex::new(Vec::new()));
        let cron_create = Arc::new(Mutex::new(Vec::new()));
        let task_get = Arc::new(Mutex::new(Vec::new()));
        let kernel = Self {
            task_post_calls: Arc::clone(&task_post),
            cron_create_calls: Arc::clone(&cron_create),
            task_get_calls: Arc::clone(&task_get),
            task_get_response: Mutex::new(None),
        };
        (
            kernel,
            CapturedCalls {
                task_post,
                cron_create,
                task_get,
            },
        )
    }

    fn set_task_get_response(&self, value: Option<serde_json::Value>) {
        *self.task_get_response.lock().unwrap() = value;
    }
}

#[async_trait]
impl AgentControl for CapturingKernel {
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

impl MemoryAccess for CapturingKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

impl WikiAccess for CapturingKernel {}

#[async_trait]
impl TaskQueue for CapturingKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.task_post_calls
            .lock()
            .unwrap()
            .push(created_by.map(|s| s.to_string()));
        Ok("task-id-1".to_string())
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
        task_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.task_get_calls
            .lock()
            .unwrap()
            .push(task_id.to_string());
        Ok(self.task_get_response.lock().unwrap().clone())
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
impl EventBus for CapturingKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for CapturingKernel {
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

#[async_trait]
impl CronControl for CapturingKernel {
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.cron_create_calls
            .lock()
            .unwrap()
            .push((agent_id.to_string(), job_json));
        Ok("cron-id-1".to_string())
    }
}

impl ApprovalGate for CapturingKernel {}
impl HandsControl for CapturingKernel {}
impl A2ARegistry for CapturingKernel {}
impl ChannelSender for CapturingKernel {}
impl PromptStore for CapturingKernel {}
impl WorkflowRunner for CapturingKernel {}
impl GoalControl for CapturingKernel {}
impl ToolPolicy for CapturingKernel {}
impl librefang_kernel_handle::ApiAuth for CapturingKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}
impl librefang_kernel_handle::SessionWriter for CapturingKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

fn make_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
    sender_id: Option<&'a str>,
    caller_agent_id: Option<&'a str>,
) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id,
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
        sender_id,
        channel: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

#[tokio::test]
async fn test_task_post_forwards_caller_as_created_by() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, Some("agent-1"));
    let input = json!({"title": "Do something", "description": "Details here"});
    let result = execute_tool_raw("t1", "task_post", &input, &ctx).await;

    assert!(
        !result.is_error,
        "task_post should succeed: {}",
        result.content
    );
    let task_calls = calls.task_post.lock().unwrap();
    assert_eq!(task_calls.len(), 1);
    assert_eq!(task_calls[0], Some("agent-1".to_string()));
}

#[tokio::test]
async fn test_task_post_forwards_none_created_by() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, None);
    let input = json!({"title": "Do something", "description": "Details here"});
    let result = execute_tool_raw("t2", "task_post", &input, &ctx).await;

    assert!(
        !result.is_error,
        "task_post should succeed: {}",
        result.content
    );
    let task_calls = calls.task_post.lock().unwrap();
    assert_eq!(task_calls.len(), 1);
    assert_eq!(task_calls[0], None);
}

#[tokio::test]
async fn test_cron_create_injects_sender_peer_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("peer-xyz"), Some("agent-1"));
    let input = json!({"schedule": "0 * * * *", "payload": "tick"});
    let result = execute_tool_raw("t3", "cron_create", &input, &ctx).await;

    assert!(
        !result.is_error,
        "cron_create should succeed: {}",
        result.content
    );
    let cron_calls = calls.cron_create.lock().unwrap();
    assert_eq!(cron_calls.len(), 1);
    let job_json = &cron_calls[0].1;
    assert_eq!(job_json["peer_id"], "peer-xyz");
}

#[tokio::test]
async fn test_cron_create_preserves_existing_peer_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("peer-xyz"), Some("agent-1"));
    let input = json!({"schedule": "0 * * * *", "payload": "tick", "peer_id": "existing"});
    let result = execute_tool_raw("t4", "cron_create", &input, &ctx).await;

    assert!(
        !result.is_error,
        "cron_create should succeed: {}",
        result.content
    );
    let cron_calls = calls.cron_create.lock().unwrap();
    assert_eq!(cron_calls.len(), 1);
    let job_json = &cron_calls[0].1;
    assert_eq!(job_json["peer_id"], "existing");
}

#[tokio::test]
async fn test_cron_create_forwards_caller_as_agent_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("peer-xyz"), Some("agent-1"));
    let input = json!({"schedule": "0 * * * *", "payload": "tick"});
    let result = execute_tool_raw("t5", "cron_create", &input, &ctx).await;

    assert!(
        !result.is_error,
        "cron_create should succeed: {}",
        result.content
    );
    let cron_calls = calls.cron_create.lock().unwrap();
    assert_eq!(cron_calls.len(), 1);
    assert_eq!(cron_calls[0].0, "agent-1");
}

#[tokio::test]
async fn test_task_status_projects_six_canonical_fields() {
    let (kernel, calls) = CapturingKernel::new();
    // task_get returns the full row shape that librefang-memory's
    // substrate emits (id/description/created_by/result/claimed_at/
    // retry_count are present); task_status must project to exactly the
    // six fields the comms_task_status MCP bridge tool returns.
    kernel.set_task_get_response(Some(json!({
        "id": "task-42",
        "title": "Investigate flaky test",
        "description": "long form description",
        "status": "completed",
        "assigned_to": "worker-1",
        "created_by": "agent-1",
        "created_at": "2026-05-04T00:00:00Z",
        "completed_at": "2026-05-04T00:05:00Z",
        "result": "fixed by retrying",
        "claimed_at": null,
        "retry_count": 0,
    })));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, Some("agent-1"));
    let input = json!({"task_id": "task-42"});
    let result = execute_tool_raw("ts1", "task_status", &input, &ctx).await;

    assert!(
        !result.is_error,
        "task_status should succeed: {}",
        result.content
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&result.content).expect("task_status returns JSON");
    let obj = parsed.as_object().expect("object");
    let keys: std::collections::BTreeSet<&str> = obj.keys().map(|s| s.as_str()).collect();
    let expected: std::collections::BTreeSet<&str> = [
        "status",
        "result",
        "title",
        "assigned_to",
        "created_at",
        "completed_at",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "exactly the six canonical fields");
    assert_eq!(parsed["status"], "completed");
    assert_eq!(parsed["result"], "fixed by retrying");
    assert_eq!(parsed["title"], "Investigate flaky test");
    assert_eq!(parsed["assigned_to"], "worker-1");
    assert_eq!(parsed["created_at"], "2026-05-04T00:00:00Z");
    assert_eq!(parsed["completed_at"], "2026-05-04T00:05:00Z");

    let getters = calls.task_get.lock().unwrap();
    assert_eq!(getters.len(), 1);
    assert_eq!(getters[0], "task-42");
}

#[tokio::test]
async fn test_task_status_not_found_returns_message() {
    let (kernel, calls) = CapturingKernel::new();
    // No response set -> task_get returns None.
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, Some("agent-1"));
    let input = json!({"task_id": "task-missing"});
    let result = execute_tool_raw("ts2", "task_status", &input, &ctx).await;

    assert!(
        !result.is_error,
        "task_status should not error on missing task: {}",
        result.content
    );
    assert!(
        result.content.contains("not found"),
        "expected not-found message, got: {}",
        result.content
    );
    let getters = calls.task_get.lock().unwrap();
    assert_eq!(getters.len(), 1);
    assert_eq!(getters[0], "task-missing");
}

#[tokio::test]
async fn test_task_status_missing_task_id_errors() {
    let (kernel, _calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, Some("agent-1"));
    let input = json!({});
    let result = execute_tool_raw("ts3", "task_status", &input, &ctx).await;

    assert!(
        result.is_error,
        "task_status without task_id should error: {}",
        result.content
    );
}
