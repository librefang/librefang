use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;

// ---------------------------------------------------------------------------
// Test 1: send_to_agent_as delegates to send_to_agent
// ---------------------------------------------------------------------------

struct TrackingSendHandle {
    send_called: AtomicBool,
}

#[async_trait]
impl AgentControl for TrackingSendHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.send_called.store(true, Ordering::SeqCst);
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for TrackingSendHandle {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

#[async_trait]
impl TaskQueue for TrackingSendHandle {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for TrackingSendHandle {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for TrackingSendHandle {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

impl CronControl for TrackingSendHandle {}
impl ApprovalGate for TrackingSendHandle {}
impl HandsControl for TrackingSendHandle {}
impl A2ARegistry for TrackingSendHandle {}
impl ChannelSender for TrackingSendHandle {}
impl PromptStore for TrackingSendHandle {}
impl WorkflowRunner for TrackingSendHandle {}
impl GoalControl for TrackingSendHandle {}
impl ToolPolicy for TrackingSendHandle {}

#[tokio::test]
async fn test_send_to_agent_as_delegates_to_send_to_agent() {
    let handle = TrackingSendHandle {
        send_called: AtomicBool::new(false),
    };

    let result = handle.send_to_agent_as("agent1", "msg", "parent1").await;

    assert!(handle.send_called.load(Ordering::SeqCst));
    assert_eq!(result.unwrap(), "ok");
}

// ---------------------------------------------------------------------------
// Test 2: spawn_agent_checked delegates to spawn_agent
// ---------------------------------------------------------------------------

struct TrackingSpawnHandle {
    spawn_called: AtomicBool,
}

#[async_trait]
impl AgentControl for TrackingSpawnHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        self.spawn_called.store(true, Ordering::SeqCst);
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for TrackingSpawnHandle {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

#[async_trait]
impl TaskQueue for TrackingSpawnHandle {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for TrackingSpawnHandle {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for TrackingSpawnHandle {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

impl CronControl for TrackingSpawnHandle {}
impl ApprovalGate for TrackingSpawnHandle {}
impl HandsControl for TrackingSpawnHandle {}
impl A2ARegistry for TrackingSpawnHandle {}
impl ChannelSender for TrackingSpawnHandle {}
impl PromptStore for TrackingSpawnHandle {}
impl WorkflowRunner for TrackingSpawnHandle {}
impl GoalControl for TrackingSpawnHandle {}
impl ToolPolicy for TrackingSpawnHandle {}

#[tokio::test]
async fn test_spawn_agent_checked_delegates_to_spawn_agent() {
    let handle = TrackingSpawnHandle {
        spawn_called: AtomicBool::new(false),
    };

    let result = handle.spawn_agent_checked("toml", None, &[]).await;

    assert!(handle.spawn_called.load(Ordering::SeqCst));
    assert_eq!(result.unwrap(), ("id".to_string(), "name".to_string()));
}

// ---------------------------------------------------------------------------
// Test 3: requires_approval_with_context delegates to requires_approval
// ---------------------------------------------------------------------------

struct TrackingApprovalHandle {
    approval_checked: AtomicBool,
}

#[async_trait]
impl AgentControl for TrackingApprovalHandle {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Ok(("id".into(), "name".into()))
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("ok".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for TrackingApprovalHandle {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

#[async_trait]
impl TaskQueue for TrackingApprovalHandle {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("task".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }

    async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }

    async fn task_delete(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_retry(&self, _task_id: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }

    async fn task_get(&self, _task_id: &str) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }

    async fn task_update_status(&self, _task_id: &str, _new_status: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for TrackingApprovalHandle {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for TrackingApprovalHandle {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("entity".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok("relation".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

impl CronControl for TrackingApprovalHandle {}
#[async_trait]
impl ApprovalGate for TrackingApprovalHandle {
    fn requires_approval(&self, _tool_name: &str) -> bool {
        self.approval_checked.store(true, Ordering::SeqCst);
        true
    }
}
impl HandsControl for TrackingApprovalHandle {}
impl A2ARegistry for TrackingApprovalHandle {}
impl ChannelSender for TrackingApprovalHandle {}
impl PromptStore for TrackingApprovalHandle {}
impl WorkflowRunner for TrackingApprovalHandle {}
impl GoalControl for TrackingApprovalHandle {}
impl ToolPolicy for TrackingApprovalHandle {}

#[test]
fn test_requires_approval_with_context_delegates_to_requires_approval() {
    let handle = TrackingApprovalHandle {
        approval_checked: AtomicBool::new(false),
    };

    let result = handle.requires_approval_with_context("tool", Some("sender"), Some("channel"));

    assert!(handle.approval_checked.load(Ordering::SeqCst));
    assert!(result);
}
