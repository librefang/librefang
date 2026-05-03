use async_trait::async_trait;
use librefang_kernel_handle::{AgentInfo, KernelHandle};
use librefang_types::approval::ApprovalDecision;
use librefang_types::memory::{Entity, GraphMatch, GraphPattern, Relation};

struct NoopKernelHandle;

#[async_trait]
impl KernelHandle for NoopKernelHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("not implemented".into())
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Err("not implemented".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Err("not implemented".into())
    }

    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Err("not implemented".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Err("not implemented".into())
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
        Err("not implemented".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), String> {
        Err("not implemented".into())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Err("not implemented".into())
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }

    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        Err("not implemented".into())
    }

    async fn knowledge_add_entity(&self, _entity: &Entity) -> Result<String, String> {
        Err("not implemented".into())
    }

    async fn knowledge_add_relation(&self, _relation: &Relation) -> Result<String, String> {
        Err("not implemented".into())
    }

    async fn knowledge_query(&self, _pattern: GraphPattern) -> Result<Vec<GraphMatch>, String> {
        Err("not implemented".into())
    }
}

#[tokio::test]
async fn test_request_approval_default_auto_approves() {
    let handle = NoopKernelHandle;
    let result = handle
        .request_approval("agent1", "tool_name", "summary", None)
        .await;
    assert_eq!(result.unwrap(), ApprovalDecision::Approved);
}

#[test]
fn test_is_tool_denied_with_context_default_false() {
    let handle = NoopKernelHandle;
    assert!(!handle.is_tool_denied_with_context("any_tool", Some("sender"), Some("channel")));
}

#[test]
fn test_requires_approval_default_false() {
    let handle = NoopKernelHandle;
    assert!(!handle.requires_approval("any_tool"));
}
