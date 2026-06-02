use async_trait::async_trait;

use super::*;
use std::sync::Arc;

/// Compile-only stub that implements every role trait, used to prove that:
///   1. `KernelHandle` is reachable purely via the blanket impl,
///   2. `Arc<dyn KernelHandle>` can be constructed from such a type,
///   3. each role trait is individually object-safe.
struct StubKernel;

#[async_trait]
impl AgentControl for StubKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), super::KernelOpError> {
        Err("stub".into())
    }
    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, super::KernelOpError> {
        Err("stub".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _agent_id: &str) -> Result<(), super::KernelOpError> {
        Err("stub".into())
    }
    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for StubKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), super::KernelOpError> {
        Err("stub".into())
    }
    fn memory_recall(
        &self,
        _key: &str,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
        Ok(None)
    }
    fn memory_list(
        &self,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, super::KernelOpError> {
        Ok(vec![])
    }
}

#[async_trait]
impl TaskQueue for StubKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, super::KernelOpError> {
        Err("stub".into())
    }
    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
        Ok(None)
    }
    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), super::KernelOpError> {
        Err("stub".into())
    }
    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, super::KernelOpError> {
        Ok(vec![])
    }
    async fn task_delete(&self, _task_id: &str) -> Result<bool, super::KernelOpError> {
        Ok(false)
    }
    async fn task_retry(&self, _task_id: &str) -> Result<bool, super::KernelOpError> {
        Ok(false)
    }
    async fn task_get(
        &self,
        _task_id: &str,
    ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
        Ok(None)
    }
    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, super::KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for StubKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), super::KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for StubKernel {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, super::KernelOpError> {
        Err("stub".into())
    }
    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, super::KernelOpError> {
        Err("stub".into())
    }
    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, super::KernelOpError> {
        Ok(vec![])
    }
}

impl CronControl for StubKernel {}
impl ApprovalGate for StubKernel {}
impl HandsControl for StubKernel {}
impl A2ARegistry for StubKernel {}
impl ChannelSender for StubKernel {}
impl PromptStore for StubKernel {}
impl WorkflowRunner for StubKernel {}
impl GoalControl for StubKernel {}
impl ToolPolicy for StubKernel {}
impl WikiAccess for StubKernel {}
impl CatalogQuery for StubKernel {}
impl ApiAuth for StubKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl SessionWriter for StubKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl AcpFsBridge for StubKernel {}
impl AcpTerminalBridge for StubKernel {}

#[test]
fn stub_satisfies_kernel_handle_via_blanket_impl() {
    fn assert_kernel_handle<T: KernelHandle + ?Sized>(_: &T) {}
    let s = StubKernel;
    assert_kernel_handle(&s);
}

#[test]
fn dyn_kernel_handle_is_object_safe() {
    let _arc: Arc<dyn KernelHandle> = Arc::new(StubKernel);
}

#[test]
fn role_traits_are_individually_object_safe() {
    // If any role trait gained a non-object-safe method (generic, Self by
    // value, etc.), this stops compiling. That's the point.
    let _agent: Arc<dyn AgentControl> = Arc::new(StubKernel);
    let _mem: Arc<dyn MemoryAccess> = Arc::new(StubKernel);
    let _tq: Arc<dyn TaskQueue> = Arc::new(StubKernel);
    let _ev: Arc<dyn EventBus> = Arc::new(StubKernel);
    let _kg: Arc<dyn KnowledgeGraph> = Arc::new(StubKernel);
    let _cron: Arc<dyn CronControl> = Arc::new(StubKernel);
    let _appr: Arc<dyn ApprovalGate> = Arc::new(StubKernel);
    let _hand: Arc<dyn HandsControl> = Arc::new(StubKernel);
    let _a2a: Arc<dyn A2ARegistry> = Arc::new(StubKernel);
    let _ch: Arc<dyn ChannelSender> = Arc::new(StubKernel);
    let _ps: Arc<dyn PromptStore> = Arc::new(StubKernel);
    let _wf: Arc<dyn WorkflowRunner> = Arc::new(StubKernel);
    let _goal: Arc<dyn GoalControl> = Arc::new(StubKernel);
    let _tp: Arc<dyn ToolPolicy> = Arc::new(StubKernel);
    let _auth: Arc<dyn ApiAuth> = Arc::new(StubKernel);
    let _sw: Arc<dyn SessionWriter> = Arc::new(StubKernel);
    let _cq: Arc<dyn CatalogQuery> = Arc::new(StubKernel);
}

#[test]
fn catalog_query_default_returns_none() {
    // Mocks / stubs that don't override `reasoning_echo_policy_for`
    // must return `None`, so drivers fall back to substring detection.
    // Without this guarantee the registry-driven dispatch could
    // accidentally activate against test fixtures that have no
    // catalog wired.
    use librefang_types::model_catalog::ReasoningEchoPolicy;
    let stub = StubKernel;
    assert_eq!(
        stub.reasoning_echo_policy_for("deepseek-v4-flash"),
        ReasoningEchoPolicy::None
    );
    assert_eq!(
        stub.reasoning_echo_policy_for("kimi-k2.6"),
        ReasoningEchoPolicy::None
    );
    assert_eq!(
        stub.reasoning_echo_policy_for("anything-else"),
        ReasoningEchoPolicy::None
    );
}
