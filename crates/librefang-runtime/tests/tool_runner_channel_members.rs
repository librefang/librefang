//! End-to-end wiring for the `channel_members` roster read (#7086).
//!
//! The channel bridge has persisted every group sender it sees through `ChannelBridgeHandle::roster_upsert` since the `group_roster` table landed, and `KernelHandle::roster_members` could always read it back — with no caller anywhere in the tree.
//! These tests cover the boundary that closes that gap: the dispatch arm resolves the conversation from the turn context, calls `roster_members` with the bare channel type, and refuses to read a conversation other than the one the turn arrived on.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Every `(channel, chat_id)` pair the tool asked the kernel about.
type RosterCallLog = Arc<Mutex<Vec<(String, String)>>>;

struct RosterKernel {
    calls: RosterCallLog,
}

impl RosterKernel {
    fn new() -> (Self, RosterCallLog) {
        let calls: RosterCallLog = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

#[async_trait]
impl AgentControl for RosterKernel {
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

impl MemoryAccess for RosterKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _key: &str,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    fn memory_list(
        &self,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

impl WikiAccess for RosterKernel {}

#[async_trait]
impl TaskQueue for RosterKernel {
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
impl EventBus for RosterKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for RosterKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
        _: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not implemented".into())
    }
}

impl CronControl for RosterKernel {}
impl ApprovalGate for RosterKernel {}
impl HandsControl for RosterKernel {}
impl A2ARegistry for RosterKernel {}
impl PromptStore for RosterKernel {}
impl WorkflowRunner for RosterKernel {}
impl GoalControl for RosterKernel {}
impl ToolPolicy for RosterKernel {}
impl librefang_kernel_handle::CatalogQuery for RosterKernel {}
impl librefang_kernel_handle::ApiAuth for RosterKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}
impl librefang_kernel_handle::SessionWriter for RosterKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl librefang_kernel_handle::AcpFsBridge for RosterKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for RosterKernel {}

/// The rows the real `ChannelSenderHandle` builds out of `RosterStore::members`, in the order that store returns them (display name, then user id).
impl ChannelSender for RosterKernel {
    fn roster_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.calls
            .lock()
            .unwrap()
            .push((channel.to_string(), chat_id.to_string()));
        if (channel, chat_id) != ("slack", "C0DESIGN") {
            return Ok(vec![]);
        }
        Ok(vec![
            json!({"user_id": "U001", "display_name": "Ana", "username": "ana", "source": "observed"}),
            json!({"user_id": "U002", "display_name": "Bo", "username": null, "source": "observed"}),
        ])
    }
}

fn make_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
    channel: Option<&'a str>,
    chat_id: Option<&'a str>,
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
        channel,
        chat_id,
        sender_account_id: None,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
        acting_principal: None,
    }
}

/// The use case from #7086: an agent in a shared Slack channel answers "who is in here?" with no arguments, and gets back the platform user id it needs to attribute the request to a person.
#[tokio::test]
async fn channel_members_reads_the_current_conversation_with_no_arguments() {
    let (kernel, calls) = RosterKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U002"));
    let result = execute_tool_raw("t1", "channel_members", &json!({}), &ctx).await;

    assert!(
        !result.is_error,
        "channel_members should succeed: {}",
        result.content
    );
    let body: serde_json::Value = serde_json::from_str(&result.content).expect("json body");
    assert_eq!(body["channel"], "slack");
    assert_eq!(body["chat_id"], "C0DESIGN");
    assert_eq!(body["count"], 2);
    assert_eq!(body["observed_count"], 2);
    assert_eq!(body["enumerated_count"], 0);
    assert_eq!(body["members"][0]["user_id"], "U001");
    assert_eq!(body["members"][0]["display_name"], "Ana");
    assert_eq!(body["members"][1]["user_id"], "U002");
    assert!(
        body.get("note").is_none(),
        "an all-observed roster needs no note: nothing here is unreachable by channel_dm: {}",
        result.content
    );

    assert_eq!(
        *calls.lock().unwrap(),
        vec![("slack".to_string(), "C0DESIGN".to_string())]
    );
}

/// An empty roster is the normal state for a DM and for a group nobody has spoken in yet, so it must not read as a broken tool.
#[tokio::test]
async fn channel_members_explains_an_empty_roster() {
    let (kernel, _calls) = RosterKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("slack"), None, Some("U777"));
    let result = execute_tool_raw("t2", "channel_members", &json!({}), &ctx).await;

    assert!(!result.is_error, "{}", result.content);
    let body: serde_json::Value = serde_json::from_str(&result.content).expect("json body");
    // No chat id stamped: the DM peer is the conversation, as for channel_send.
    assert_eq!(body["chat_id"], "U777");
    assert_eq!(body["count"], 0);
    assert!(body["members"].as_array().expect("array").is_empty());
    assert!(body["note"].as_str().expect("note").contains("roster"));
}

/// The inbound twin of the #6117 cross-chat dispatch leak: a member of one group must not be able to enumerate another group's membership, and the refusal has to happen before the kernel is asked.
#[tokio::test]
async fn channel_members_refuses_another_chat_on_the_same_channel() {
    let (kernel, calls) = RosterKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U002"));
    let input = json!({"channel": "slack", "chat_id": "C0SECRET"});
    let result = execute_tool_raw("t3", "channel_members", &input, &ctx).await;

    assert!(
        result.is_error,
        "cross-chat roster read must be refused: {}",
        result.content
    );
    assert!(result.content.contains("C0SECRET"), "{}", result.content);
    assert!(
        calls.lock().unwrap().is_empty(),
        "the kernel must not be queried for a refused read"
    );
}

/// The WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227) while the roster is keyed on the bare channel type, so the lookup key has to be the base type or every WhatsApp read misses.
#[tokio::test]
async fn channel_members_strips_the_channel_suffix_before_the_lookup() {
    let (kernel, calls) = RosterKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(
        &kernel,
        Some("whatsapp:123@g.us"),
        Some("123@g.us"),
        Some("44700@s.whatsapp.net"),
    );
    let result = execute_tool_raw("t4", "channel_members", &json!({}), &ctx).await;

    assert!(!result.is_error, "{}", result.content);
    let body: serde_json::Value = serde_json::from_str(&result.content).expect("json body");
    assert_eq!(body["channel"], "whatsapp");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("whatsapp".to_string(), "123@g.us".to_string())]
    );
}

/// Out-of-band callers (cron, triggers) have no conversation to default to, so the arguments become required instead of resolving to something arbitrary.
#[tokio::test]
async fn channel_members_requires_arguments_without_a_turn() {
    let (kernel, calls) = RosterKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, None, None, None);
    let missing = execute_tool_raw("t5", "channel_members", &json!({}), &ctx).await;
    assert!(missing.is_error, "{}", missing.content);
    assert!(missing.content.contains("channel"), "{}", missing.content);

    let explicit = json!({"channel": "slack", "chat_id": "C0DESIGN"});
    let ok = execute_tool_raw("t6", "channel_members", &explicit, &ctx).await;
    assert!(!ok.is_error, "{}", ok.content);
    let body: serde_json::Value = serde_json::from_str(&ok.content).expect("json body");
    assert_eq!(body["count"], 2);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("slack".to_string(), "C0DESIGN".to_string())]
    );
}
