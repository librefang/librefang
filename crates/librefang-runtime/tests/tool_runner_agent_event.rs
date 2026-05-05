//! Integration tests for `agent_*` and `event_publish` tool dispatch through
//! the runtime's `execute_tool_raw` (#3696).
//!
//! Complements the existing `tool_runner_forwarding.rs` (memory tools) and
//! `tool_runner_forwarding_task_cron.rs` (task / cron tools) which between
//! them already cover the memory and task dispatch paths. This file picks
//! up the remaining "easy" tool families that don't require a real kernel:
//!
//! - `agent_send`: payload + caller agent forwarded to `AgentControl`.
//! - `agent_list`: `KernelHandle::list_agents` is exercised and the
//!   resulting string carries each agent's id/name.
//! - `event_publish`: event_type + payload forwarded to `EventBus`.
//!
//! All tests use the same role-trait mock pattern as the sibling files —
//! a `CapturingKernel` that implements the full `KernelHandle` composition
//! by stubbing the unused traits and recording calls on the relevant ones.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

// --- Captured-call payloads ------------------------------------------------

#[derive(Debug, Clone)]
struct SentMessage {
    to_agent_id: String,
    body: String,
}

#[derive(Debug, Clone)]
struct PublishedEvent {
    event_type: String,
    payload: serde_json::Value,
}

struct Captures {
    sends: Arc<Mutex<Vec<SentMessage>>>,
    events: Arc<Mutex<Vec<PublishedEvent>>>,
}

// --- The mock kernel -------------------------------------------------------

struct CapturingKernel {
    sends: Arc<Mutex<Vec<SentMessage>>>,
    events: Arc<Mutex<Vec<PublishedEvent>>>,
    agents: Vec<AgentInfo>,
}

impl CapturingKernel {
    fn new(agents: Vec<AgentInfo>) -> (Self, Captures) {
        let sends = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let kernel = Self {
            sends: Arc::clone(&sends),
            events: Arc::clone(&events),
            agents,
        };
        (kernel, Captures { sends, events })
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
        agent_id: &str,
        message: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.sends.lock().unwrap().push(SentMessage {
            to_agent_id: agent_id.to_string(),
            body: message.to_string(),
        });
        Ok(format!("queued for {agent_id}"))
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents.clone()
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
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.events.lock().unwrap().push(PublishedEvent {
            event_type: event_type.to_string(),
            payload,
        });
        Ok(())
    }
}

// All remaining role traits use their default impls — no behaviour needed
// for these tests, just trait coverage so `dyn KernelHandle` is satisfied.
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
    fn auth_api_key(&self) -> String {
        String::new()
    }
    fn dashboard_raw_config(&self) -> librefang_kernel_handle::DashboardRawConfig {
        librefang_kernel_handle::DashboardRawConfig {
            user: String::new(),
            pass: String::new(),
            pass_hash: String::new(),
        }
    }
    fn auth_home_dir(&self) -> &std::path::Path {
        std::path::Path::new("/tmp")
    }
    fn auth_device_api_keys(&self) -> Vec<(String, String)> {
        vec![]
    }
    fn auth_config_users(&self) -> Vec<librefang_kernel_handle::ApiUserConfigSnapshot> {
        vec![]
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

// --- Helpers ---------------------------------------------------------------

fn make_ctx<'a>(kernel: &'a Arc<dyn KernelHandle>, caller: Option<&'a str>) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: caller,
        skill_registry: None,
        allowed_skills: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None as Option<&Path>,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id: None,
        channel: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

fn agent_info(id: &str, name: &str) -> AgentInfo {
    AgentInfo {
        id: id.to_string(),
        name: name.to_string(),
        state: "idle".to_string(),
        model_provider: "stub".to_string(),
        model_name: "stub-model".to_string(),
        description: String::new(),
        tags: vec![],
        tools: vec![],
    }
}

// --- agent_send ------------------------------------------------------------

#[tokio::test]
async fn agent_send_forwards_target_agent_id_and_message() {
    let (kernel, caps) = CapturingKernel::new(vec![
        agent_info("agent-A", "alice"),
        agent_info("agent-B", "bob"),
    ]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("agent-A"));
    let input = json!({"agent_id": "agent-B", "message": "hello bob"});
    let result = execute_tool_raw("t1", "agent_send", &input, &ctx).await;

    assert!(
        !result.is_error,
        "agent_send should succeed: {}",
        result.content
    );
    let sends = caps.sends.lock().unwrap();
    assert_eq!(sends.len(), 1, "exactly one send must be forwarded");
    assert_eq!(sends[0].to_agent_id, "agent-B");
    assert_eq!(sends[0].body, "hello bob");
}

#[tokio::test]
async fn agent_send_self_is_refused_to_avoid_deadlock() {
    // Agent self-send would deadlock on the per-agent message lock — the
    // dispatcher's job is to reject this BEFORE entering the kernel call,
    // so the mock should never see a recorded send.
    let (kernel, caps) = CapturingKernel::new(vec![agent_info("agent-A", "alice")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("agent-A"));
    let input = json!({"agent_id": "agent-A", "message": "to myself"});
    let result = execute_tool_raw("t1", "agent_send", &input, &ctx).await;

    assert!(result.is_error, "self-send must surface as an error result");
    assert!(
        caps.sends.lock().unwrap().is_empty(),
        "self-send must short-circuit BEFORE invoking AgentControl::send_to_agent"
    );
}

// --- agent_list ------------------------------------------------------------

#[tokio::test]
async fn agent_list_renders_kernel_provided_agents() {
    let (kernel, _caps) = CapturingKernel::new(vec![
        agent_info("a-1", "researcher"),
        agent_info("a-2", "writer"),
    ]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None);
    let result = execute_tool_raw("t1", "agent_list", &json!({}), &ctx).await;

    assert!(
        !result.is_error,
        "agent_list must succeed: {}",
        result.content
    );
    let out = result.content.to_lowercase();
    // Don't pin the exact rendering — just make sure both agents make it
    // into the output. The exact format is a UI-layer concern.
    assert!(
        out.contains("researcher") && out.contains("writer"),
        "agent_list output must include both registered names; got {}",
        result.content
    );
    assert!(
        result.content.contains("a-1") && result.content.contains("a-2"),
        "agent_list output must include both ids; got {}",
        result.content
    );
}

#[tokio::test]
async fn agent_list_when_no_agents_running_returns_friendly_string() {
    let (kernel, _caps) = CapturingKernel::new(vec![]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None);
    let result = execute_tool_raw("t1", "agent_list", &json!({}), &ctx).await;

    assert!(
        !result.is_error,
        "empty agent list must NOT be reported as an error: {}",
        result.content
    );
    let lower = result.content.to_lowercase();
    assert!(
        lower.contains("no agents") || lower.contains("0 agents") || lower.contains("none"),
        "empty result should clearly state no agents are running; got {}",
        result.content
    );
}

// --- event_publish ---------------------------------------------------------

#[tokio::test]
async fn event_publish_forwards_event_type_and_payload() {
    let (kernel, caps) = CapturingKernel::new(vec![]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("agent-A"));
    let input = json!({
        "event_type": "weather.update",
        "payload": {"city": "Tokyo", "tempC": 22}
    });
    let result = execute_tool_raw("t1", "event_publish", &input, &ctx).await;

    assert!(
        !result.is_error,
        "event_publish must succeed: {}",
        result.content
    );
    let events = caps.events.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event must be forwarded");
    assert_eq!(events[0].event_type, "weather.update");
    assert_eq!(events[0].payload["city"], "Tokyo");
    assert_eq!(events[0].payload["tempC"], 22);
}

#[tokio::test]
async fn event_publish_missing_event_type_errors_without_invoking_kernel() {
    let (kernel, caps) = CapturingKernel::new(vec![]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("agent-A"));
    let input = json!({"payload": {"x": 1}});
    let result = execute_tool_raw("t1", "event_publish", &input, &ctx).await;

    assert!(
        result.is_error,
        "missing event_type must surface as an error: {}",
        result.content
    );
    assert!(
        caps.events.lock().unwrap().is_empty(),
        "validation failures must short-circuit BEFORE EventBus::publish_event"
    );
}
