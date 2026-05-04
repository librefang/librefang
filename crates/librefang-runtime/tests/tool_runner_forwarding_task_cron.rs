use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

type TaskPostCalls = Arc<Mutex<Vec<Option<String>>>>;
type CronCreateCalls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

struct CapturedCalls {
    task_post: TaskPostCalls,
    cron_create: CronCreateCalls,
}

struct CapturingKernel {
    task_post_calls: TaskPostCalls,
    cron_create_calls: CronCreateCalls,
}

impl CapturingKernel {
    fn new() -> (Self, CapturedCalls) {
        let task_post = Arc::new(Mutex::new(Vec::new()));
        let cron_create = Arc::new(Mutex::new(Vec::new()));
        let kernel = Self {
            task_post_calls: Arc::clone(&task_post),
            cron_create_calls: Arc::clone(&cron_create),
        };
        (
            kernel,
            CapturedCalls {
                task_post,
                cron_create,
            },
        )
    }
}

#[async_trait]
impl AgentControl for CapturingKernel {
    async fn spawn_agent(&self, _: &str, _: Option<&str>) -> Result<(String, String), String> {
        Err("not implemented".into())
    }
    async fn send_to_agent(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not implemented".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), String> {
        Err("not implemented".into())
    }
    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for CapturingKernel {
    fn memory_store(&self, _: &str, _: serde_json::Value, _: Option<&str>) -> Result<(), String> {
        Err("not implemented".into())
    }
    fn memory_recall(&self, _: &str, _: Option<&str>) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    fn memory_list(&self, _: Option<&str>) -> Result<Vec<String>, String> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl TaskQueue for CapturingKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, String> {
        self.task_post_calls
            .lock()
            .unwrap()
            .push(created_by.map(|s| s.to_string()));
        Ok("task-id-1".to_string())
    }
    async fn task_claim(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_complete(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("not implemented".into())
    }
    async fn task_list(&self, _: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
    async fn task_get(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_update_status(&self, _: &str, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl EventBus for CapturingKernel {
    async fn publish_event(&self, _: &str, _: serde_json::Value) -> Result<(), String> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for CapturingKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
    ) -> Result<String, String> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
    ) -> Result<String, String> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl CronControl for CapturingKernel {
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
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
