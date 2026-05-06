use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

type PeerCallLog = Arc<Mutex<Vec<Option<String>>>>;

struct MemoryCallLogs {
    store: PeerCallLog,
    recall: PeerCallLog,
    list: PeerCallLog,
}

struct CapturingKernel {
    memory_store_calls: PeerCallLog,
    memory_recall_calls: PeerCallLog,
    memory_list_calls: PeerCallLog,
}

impl CapturingKernel {
    fn new() -> (Self, MemoryCallLogs) {
        let store = Arc::new(Mutex::new(Vec::new()));
        let recall = Arc::new(Mutex::new(Vec::new()));
        let list = Arc::new(Mutex::new(Vec::new()));
        let kernel = Self {
            memory_store_calls: Arc::clone(&store),
            memory_recall_calls: Arc::clone(&recall),
            memory_list_calls: Arc::clone(&list),
        };
        (
            kernel,
            MemoryCallLogs {
                store,
                recall,
                list,
            },
        )
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
        _key: &str,
        _value: serde_json::Value,
        peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.memory_store_calls
            .lock()
            .unwrap()
            .push(peer_id.map(|s| s.to_string()));
        Ok(())
    }
    fn memory_recall(
        &self,
        _key: &str,
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.memory_recall_calls
            .lock()
            .unwrap()
            .push(peer_id.map(|s| s.to_string()));
        Ok(None)
    }
    fn memory_list(
        &self,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        self.memory_list_calls
            .lock()
            .unwrap()
            .push(peer_id.map(|s| s.to_string()));
        Ok(vec![])
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

impl CronControl for CapturingKernel {}
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
) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"),
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
async fn test_memory_store_forwards_sender_id_as_peer_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("user-42"));
    let input = json!({"key": "k1", "value": "v1"});
    let result = execute_tool_raw("t1", "memory_store", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_store should succeed: {}",
        result.content
    );
    let store_calls = calls.store.lock().unwrap();
    assert_eq!(store_calls.len(), 1);
    assert_eq!(store_calls[0], Some("user-42".to_string()));
}

#[tokio::test]
async fn test_memory_store_forwards_none_when_no_sender() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None);
    let input = json!({"key": "k1", "value": "v1"});
    let result = execute_tool_raw("t1", "memory_store", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_store should succeed: {}",
        result.content
    );
    let store_calls = calls.store.lock().unwrap();
    assert_eq!(store_calls.len(), 1);
    assert_eq!(store_calls[0], None);
}

#[tokio::test]
async fn test_memory_recall_forwards_sender_id_as_peer_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("user-99"));
    let input = json!({"key": "k2"});
    let result = execute_tool_raw("t2", "memory_recall", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_recall should succeed: {}",
        result.content
    );
    let recall_calls = calls.recall.lock().unwrap();
    assert_eq!(recall_calls.len(), 1);
    assert_eq!(recall_calls[0], Some("user-99".to_string()));
}

#[tokio::test]
async fn test_memory_recall_forwards_none_when_no_sender() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None);
    let input = json!({"key": "k2"});
    let result = execute_tool_raw("t2", "memory_recall", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_recall should succeed: {}",
        result.content
    );
    let recall_calls = calls.recall.lock().unwrap();
    assert_eq!(recall_calls.len(), 1);
    assert_eq!(recall_calls[0], None);
}

#[tokio::test]
async fn test_memory_list_forwards_sender_id_as_peer_id() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("user-7"));
    let input = json!({});
    let result = execute_tool_raw("t3", "memory_list", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_list should succeed: {}",
        result.content
    );
    let list_calls = calls.list.lock().unwrap();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0], Some("user-7".to_string()));
}

#[tokio::test]
async fn test_memory_list_forwards_none_when_no_sender() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None);
    let input = json!({});
    let result = execute_tool_raw("t3", "memory_list", &input, &ctx).await;

    assert!(
        !result.is_error,
        "memory_list should succeed: {}",
        result.content
    );
    let list_calls = calls.list.lock().unwrap();
    assert_eq!(list_calls.len(), 1);
    assert_eq!(list_calls[0], None);
}

#[tokio::test]
async fn test_sender_id_not_leaked_between_calls() {
    let (kernel, calls) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let input = json!({"key": "k", "value": "v"});

    let ctx_a = make_ctx(&kernel, Some("alice"));
    let _ = execute_tool_raw("t1", "memory_store", &input, &ctx_a).await;

    let ctx_b = make_ctx(&kernel, Some("bob"));
    let _ = execute_tool_raw("t2", "memory_store", &input, &ctx_b).await;

    let ctx_none = make_ctx(&kernel, None);
    let _ = execute_tool_raw("t3", "memory_store", &input, &ctx_none).await;

    let store_calls = calls.store.lock().unwrap();
    assert_eq!(store_calls.len(), 3);
    assert_eq!(store_calls[0], Some("alice".to_string()));
    assert_eq!(store_calls[1], Some("bob".to_string()));
    assert_eq!(store_calls[2], None);
}
