use async_trait::async_trait;
use librefang_kernel_handle::{AgentInfo, KernelHandle};
use librefang_types::memory::{Entity, GraphMatch, GraphPattern, Relation};
use librefang_types::user_policy::UserToolGate;

struct NoopKernelHandle;

#[async_trait]
impl KernelHandle for NoopKernelHandle {
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

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }

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

    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        Err("noop".into())
    }

    async fn knowledge_add_entity(&self, _entity: Entity) -> Result<String, String> {
        Err("noop".into())
    }

    async fn knowledge_add_relation(&self, _relation: Relation) -> Result<String, String> {
        Err("noop".into())
    }

    async fn knowledge_query(&self, _pattern: GraphPattern) -> Result<Vec<GraphMatch>, String> {
        Err("noop".into())
    }
}

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
    let handle = NoopKernelHandle;

    let result = handle.cron_create("agent", serde_json::json!({})).await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Cron scheduler not available"),
        "cron_create error should mention 'Cron scheduler not available'"
    );

    let result = handle.cron_list("agent").await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Cron scheduler not available"),
        "cron_list error should mention 'Cron scheduler not available'"
    );

    let result = handle.cron_cancel("job1").await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Cron scheduler not available"),
        "cron_cancel error should mention 'Cron scheduler not available'"
    );
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
