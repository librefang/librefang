use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use librefang_kernel_handle::{AgentInfo, KernelHandle};

// ---------------------------------------------------------------------------
// Test 1: send_to_agent_as delegates to send_to_agent
// ---------------------------------------------------------------------------

struct TrackingSendHandle {
    send_called: AtomicBool,
}

#[async_trait]
impl KernelHandle for TrackingSendHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        self.send_called.store(true, Ordering::SeqCst);
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Ok(vec![])
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
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn knowledge_add_entity(
        &self,
        _entity: librefang_types::memory::Entity,
    ) -> Result<String, String> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: librefang_types::memory::Relation,
    ) -> Result<String, String> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_send_to_agent_as_delegates_to_send_to_agent() {
    let handle = TrackingSendHandle {
        send_called: AtomicBool::new(false),
    };

    let result = handle.send_to_agent_as("agent1", "msg", "parent1").await;

    assert!(handle.send_called.load(Ordering::SeqCst));
    assert_eq!(result, Ok("ok".into()));
}

// ---------------------------------------------------------------------------
// Test 2: spawn_agent_checked delegates to spawn_agent
// ---------------------------------------------------------------------------

struct TrackingSpawnHandle {
    spawn_called: AtomicBool,
}

#[async_trait]
impl KernelHandle for TrackingSpawnHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        self.spawn_called.store(true, Ordering::SeqCst);
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Ok(vec![])
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
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn knowledge_add_entity(
        &self,
        _entity: librefang_types::memory::Entity,
    ) -> Result<String, String> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: librefang_types::memory::Relation,
    ) -> Result<String, String> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_spawn_agent_checked_delegates_to_spawn_agent() {
    let handle = TrackingSpawnHandle {
        spawn_called: AtomicBool::new(false),
    };

    let result = handle.spawn_agent_checked("toml", None, &[]).await;

    assert!(handle.spawn_called.load(Ordering::SeqCst));
    assert_eq!(result, Ok(("id".into(), "name".into())));
}

// ---------------------------------------------------------------------------
// Test 3: requires_approval_with_context delegates to requires_approval
// ---------------------------------------------------------------------------

struct TrackingApprovalHandle {
    approval_checked: AtomicBool,
}

#[async_trait]
impl KernelHandle for TrackingApprovalHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Ok(vec![])
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
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn knowledge_add_entity(
        &self,
        _entity: librefang_types::memory::Entity,
    ) -> Result<String, String> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: librefang_types::memory::Relation,
    ) -> Result<String, String> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
        Ok(vec![])
    }

    fn requires_approval(&self, _tool_name: &str) -> bool {
        self.approval_checked.store(true, Ordering::SeqCst);
        true
    }
}

#[test]
fn test_requires_approval_with_context_delegates_to_requires_approval() {
    let handle = TrackingApprovalHandle {
        approval_checked: AtomicBool::new(false),
    };

    let result = handle.requires_approval_with_context("tool", Some("sender"), Some("channel"));

    assert!(handle.approval_checked.load(Ordering::SeqCst));
    assert!(result);
}
