use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_types::memory::{Entity, GraphMatch, GraphPattern, Relation};
use librefang_types::user_policy::UserToolGate;

struct NoopKernelHandle;

#[async_trait]
impl AgentControl for NoopKernelHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("noop".into())
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Err("noop".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Err("noop".into())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for NoopKernelHandle {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Err("noop".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Err("noop".into())
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Err("noop".into())
    }
}

#[async_trait]
impl TaskQueue for NoopKernelHandle {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, String> {
        Err("noop".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Err("noop".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), String> {
        Err("noop".into())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Err("noop".into())
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
        Err("noop".into())
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
        Err("noop".into())
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, String> {
        Err("noop".into())
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, String> {
        Err("noop".into())
    }
}

#[async_trait]
impl EventBus for NoopKernelHandle {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("noop".into())
    }
}

#[async_trait]
impl KnowledgeGraph for NoopKernelHandle {
    async fn knowledge_add_entity(
        &self,
        _entity: &Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("noop".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("noop".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: GraphPattern,
    ) -> Result<Vec<GraphMatch>, librefang_kernel_handle::KernelOpError> {
        Err("noop".into())
    }
}

impl CronControl for NoopKernelHandle {}
impl ApprovalGate for NoopKernelHandle {}
impl HandsControl for NoopKernelHandle {}
impl A2ARegistry for NoopKernelHandle {}
impl ChannelSender for NoopKernelHandle {}
impl PromptStore for NoopKernelHandle {}
impl WorkflowRunner for NoopKernelHandle {}
impl GoalControl for NoopKernelHandle {}
impl ToolPolicy for NoopKernelHandle {}

#[test]
fn test_resolve_user_tool_decision_default_allow() {
    let handle = NoopKernelHandle;
    let result = handle.resolve_user_tool_decision("any_tool", Some("sender"), Some("channel"));
    assert_eq!(result, UserToolGate::Allow);
}

#[test]
fn test_memory_acl_for_sender_default_none() {
    let handle = NoopKernelHandle;
    let result = handle.memory_acl_for_sender(Some("sender"), Some("channel"));
    assert!(result.is_none());
}

#[tokio::test]
async fn test_cron_defaults_return_errors() {
    use librefang_kernel_handle::KernelOpError;

    let handle = NoopKernelHandle;

    // Each default impl now returns a typed Unavailable variant — callers
    // can match on the variant directly instead of substring-grepping on
    // the formatted message (#3541). Display still includes
    // "Cron scheduler not available" for log-output continuity.
    let result = handle.cron_create("agent", serde_json::json!({})).await;
    match result {
        Err(KernelOpError::Unavailable { capability: "Cron scheduler" }) => {}
        other => panic!("cron_create: expected Unavailable, got {other:?}"),
    }

    let result = handle.cron_list("agent").await;
    match result {
        Err(KernelOpError::Unavailable { capability: "Cron scheduler" }) => {}
        other => panic!("cron_list: expected Unavailable, got {other:?}"),
    }

    let result = handle.cron_cancel("job1").await;
    match result {
        Err(KernelOpError::Unavailable { capability: "Cron scheduler" }) => {}
        other => panic!("cron_cancel: expected Unavailable, got {other:?}"),
    }
}

#[test]
fn test_tool_timeout_defaults() {
    let handle = NoopKernelHandle;
    assert_eq!(handle.tool_timeout_secs(), 120);
    assert_eq!(handle.tool_timeout_secs_for("any_tool"), 120);
}

#[test]
fn test_max_agent_call_depth_default() {
    let handle = NoopKernelHandle;
    assert_eq!(handle.max_agent_call_depth(), 5);
}

#[test]
fn test_workspace_prefix_defaults_empty() {
    let handle = NoopKernelHandle;
    assert!(handle.readonly_workspace_prefixes("any_agent").is_empty());
    assert!(handle.named_workspace_prefixes("any_agent").is_empty());
}
