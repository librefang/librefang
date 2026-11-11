//! Per-agent memory isolation through the tool dispatch path.
//!
//! Regression tests for the fix to issue #5070: before the fix, the tool
//! dispatch layer passed `None` as `agent_id` to every `memory_store` /
//! `memory_recall` / `memory_list` call, so all agents shared a single
//! namespace keyed by the sentinel UUID. After the fix, `caller_agent_id`
//! from `ToolExecContext` is threaded through to the kernel handle, giving
//! each agent its own isolated memory.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type AgentIdLog = Arc<Mutex<Vec<Option<String>>>>;
type StorageMap = HashMap<(Option<String>, String), serde_json::Value>;

struct IsolationKernel {
    store_agent_ids: AgentIdLog,
    recall_agent_ids: AgentIdLog,
    list_agent_ids: AgentIdLog,
    prefix_agent_ids: AgentIdLog,
    storage: Arc<Mutex<StorageMap>>,
}

struct IsolationProbes {
    store_agent_ids: AgentIdLog,
    recall_agent_ids: AgentIdLog,
    list_agent_ids: AgentIdLog,
    prefix_agent_ids: AgentIdLog,
}

impl IsolationKernel {
    fn new() -> (Self, IsolationProbes) {
        let store = Arc::new(Mutex::new(Vec::new()));
        let recall = Arc::new(Mutex::new(Vec::new()));
        let list = Arc::new(Mutex::new(Vec::new()));
        let prefix = Arc::new(Mutex::new(Vec::new()));
        let storage = Arc::new(Mutex::new(HashMap::new()));
        let kernel = Self {
            store_agent_ids: Arc::clone(&store),
            recall_agent_ids: Arc::clone(&recall),
            list_agent_ids: Arc::clone(&list),
            prefix_agent_ids: Arc::clone(&prefix),
            storage: Arc::clone(&storage),
        };
        (
            kernel,
            IsolationProbes {
                store_agent_ids: store,
                recall_agent_ids: recall,
                list_agent_ids: list,
                prefix_agent_ids: prefix,
            },
        )
    }
}

#[async_trait]
impl AgentControl for IsolationKernel {
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

impl MemoryAccess for IsolationKernel {
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.store_agent_ids
            .lock()
            .unwrap()
            .push(agent_id.map(|s| s.to_string()));
        self.storage
            .lock()
            .unwrap()
            .insert((agent_id.map(|s| s.to_string()), key.to_string()), value);
        Ok(())
    }
    fn memory_recall(
        &self,
        key: &str,
        agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.recall_agent_ids
            .lock()
            .unwrap()
            .push(agent_id.map(|s| s.to_string()));
        Ok(self
            .storage
            .lock()
            .unwrap()
            .get(&(agent_id.map(|s| s.to_string()), key.to_string()))
            .cloned())
    }
    fn memory_list(
        &self,
        agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        self.list_agent_ids
            .lock()
            .unwrap()
            .push(agent_id.map(|s| s.to_string()));
        let guard = self.storage.lock().unwrap();
        Ok(guard
            .keys()
            .filter(|(aid, _)| *aid == agent_id.map(|s| s.to_string()))
            .map(|(_, k)| k.clone())
            .collect())
    }
    fn memory_acl_for_sender(
        &self,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Option<librefang_types::user_policy::UserMemoryAccess> {
        None
    }
    fn memory_keys_with_prefix(
        &self,
        prefix: &str,
        agent_id: Option<&str>,
        _peer_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        self.prefix_agent_ids
            .lock()
            .unwrap()
            .push(agent_id.map(|s| s.to_string()));
        let guard = self.storage.lock().unwrap();
        let mut keys: Vec<String> = guard
            .keys()
            .filter(|(aid, _)| *aid == agent_id.map(|s| s.to_string()))
            .map(|(_, k)| k.clone())
            .filter(|k| k.starts_with(prefix))
            .collect();
        keys.sort();
        keys.truncate(limit);
        Ok(keys)
    }
}

impl WikiAccess for IsolationKernel {}

#[async_trait]
impl TaskQueue for IsolationKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _opts: &librefang_kernel_handle::TaskPostOptions,
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
impl EventBus for IsolationKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for IsolationKernel {
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

impl CronControl for IsolationKernel {}
impl ApprovalGate for IsolationKernel {}
impl HandsControl for IsolationKernel {}
impl A2ARegistry for IsolationKernel {}
impl ChannelSender for IsolationKernel {}
impl PromptStore for IsolationKernel {}
impl SkillHistory for IsolationKernel {}
impl WorkflowRunner for IsolationKernel {}
impl GoalControl for IsolationKernel {}
impl ToolPolicy for IsolationKernel {}
impl librefang_kernel_handle::CatalogQuery for IsolationKernel {}
impl librefang_kernel_handle::ApiAuth for IsolationKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}
impl librefang_kernel_handle::SessionWriter for IsolationKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl librefang_kernel_handle::AcpFsBridge for IsolationKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for IsolationKernel {}

fn make_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
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
        tts_config: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id: None,
        channel: None,
        chat_id: None,
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

#[tokio::test]
async fn f1_cross_agent_negative_isolation_via_tool_path() {
    let (kernel, probes) = IsolationKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx_a = make_ctx(&kernel, Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
    let ctx_b = make_ctx(&kernel, Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));

    let store = execute_tool_raw(
        "t1",
        "memory_store",
        &json!({"key": "secret", "value": "agent-a-data"}),
        &ctx_a,
    )
    .await;
    assert!(!store.is_error, "store should succeed: {}", store.content);

    {
        let store_ids = probes.store_agent_ids.lock().unwrap();
        assert_eq!(
            store_ids.len(),
            1,
            "expected exactly one store call, got {store_ids:?}"
        );
        assert_eq!(
            store_ids[0],
            Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
            "store must pass agent A's caller_agent_id"
        );
    }

    let recall_b = execute_tool_raw("t2", "memory_recall", &json!({"key": "secret"}), &ctx_b).await;
    assert!(
        !recall_b.is_error,
        "recall by agent B should not error: {}",
        recall_b.content
    );
    assert!(
        recall_b.content.contains("No value found"),
        "agent B must not see agent A's key, got: {}",
        recall_b.content
    );

    let recall_a = execute_tool_raw("t3", "memory_recall", &json!({"key": "secret"}), &ctx_a).await;
    assert!(
        !recall_a.is_error,
        "recall by agent A should not error: {}",
        recall_a.content
    );
    assert!(
        recall_a.content.contains("agent-a-data"),
        "agent A must see its own stored value, got: {}",
        recall_a.content
    );

    let recall_ids = probes.recall_agent_ids.lock().unwrap();
    assert_eq!(recall_ids.len(), 2, "expected two recall calls");
    assert_eq!(
        recall_ids[0],
        Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".to_string()),
        "first recall must pass agent B's caller_agent_id"
    );
    assert_eq!(
        recall_ids[1],
        Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
        "second recall must pass agent A's caller_agent_id"
    );
}

#[tokio::test]
async fn f1_memory_list_scoped_to_agent() {
    let (kernel, probes) = IsolationKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx_a = make_ctx(&kernel, Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
    let ctx_b = make_ctx(&kernel, Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));

    let _ = execute_tool_raw(
        "t1",
        "memory_store",
        &json!({"key": "key-a", "value": "val-a"}),
        &ctx_a,
    )
    .await;
    let _ = execute_tool_raw(
        "t2",
        "memory_store",
        &json!({"key": "key-b", "value": "val-b"}),
        &ctx_b,
    )
    .await;

    let list_a = execute_tool_raw("t3", "memory_list", &json!({}), &ctx_a).await;
    assert!(!list_a.is_error, "list should succeed: {}", list_a.content);
    assert!(
        list_a.content.contains("key-a"),
        "agent A's list must include its own key, got: {}",
        list_a.content
    );
    assert!(
        !list_a.content.contains("key-b"),
        "agent A's list must NOT include agent B's key, got: {}",
        list_a.content
    );

    let list_ids = probes.list_agent_ids.lock().unwrap();
    assert_eq!(list_ids.len(), 1, "expected one list call");
    assert_eq!(
        list_ids[0],
        Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
        "memory_list must pass agent A's caller_agent_id"
    );
}

#[tokio::test]
async fn f2_memory_list_empty_returns_agent_scoped_message() {
    let (kernel, _probes) = IsolationKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("cccccccc-cccc-cccc-cccc-cccccccccccc"));

    let list = execute_tool_raw("t1", "memory_list", &json!({}), &ctx).await;
    assert!(!list.is_error, "list should succeed: {}", list.content);
    assert_eq!(
        list.content.replace('"', ""),
        "No entries found in this agent's memory.",
        "empty list must say 'this agent's memory', not 'shared memory'"
    );
}

// NOTE: f3 (e2e deferred approval with empty agent_id) is deferred —
// `cargo test -p librefang-kernel` fails to compile due to a pre-existing
// opentelemetry version mismatch in librefang-api (transitive dev-dependency).
// The kernel-side guard is covered by unit logic (early return on empty string
// in `handle_approval_resolution`), but a full e2e test requires the kernel
// test harness to build cleanly first.

#[tokio::test]
async fn f4_auto_memorize_via_memory_list() {
    let (kernel, probes) = IsolationKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("dddddddd-dddd-dddd-dddd-dddddddddddd"));

    let store = execute_tool_raw(
        "t1",
        "memory_store",
        &json!({"key": "auto.fact", "value": "the sky is blue"}),
        &ctx,
    )
    .await;
    assert!(!store.is_error, "store should succeed: {}", store.content);

    let list = execute_tool_raw("t2", "memory_list", &json!({}), &ctx).await;
    assert!(!list.is_error, "list should succeed: {}", list.content);
    assert!(
        list.content.contains("auto.fact"),
        "auto_memorize key must appear in memory_list, got: {}",
        list.content
    );

    let store_ids = probes.store_agent_ids.lock().unwrap();
    assert_eq!(store_ids.len(), 1);
    assert_eq!(
        store_ids[0],
        Some("dddddddd-dddd-dddd-dddd-dddddddddddd".to_string()),
        "memory_store must pass caller_agent_id"
    );
    let list_ids = probes.list_agent_ids.lock().unwrap();
    assert_eq!(list_ids.len(), 1);
    assert_eq!(
        list_ids[0],
        Some("dddddddd-dddd-dddd-dddd-dddddddddddd".to_string()),
        "memory_list must pass caller_agent_id"
    );
}

/// A recall miss must hand back the key space it searched (#8625).
///
/// Before the fix the miss was a bare `No value found for key '…'.`, so an agent convinced the key existed varied it — `…-docx-v32` … `-v41` — until the iteration cap ended the turn.
/// The hint has to name the keys that *do* exist near the request, and it has to do it inside the caller's own agent scope: a hint that surfaced another agent's key names would be a leak the exact-key lookup itself never had.
///
/// Peer scoping is deliberately not asserted here: this stub ignores `peer_id`, so it can only demonstrate the agent half.
/// That half is covered against the real kernel by `test_memory_keys_with_prefix_stays_inside_the_caller_scope`.
#[tokio::test]
async fn f5_recall_miss_names_nearby_keys_in_the_callers_own_scope() {
    let (kernel, probes) = IsolationKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx_a = make_ctx(&kernel, Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
    let ctx_b = make_ctx(&kernel, Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));

    // A owns the object under a shorter name; B owns a key that shares the whole requested prefix, so a probe issued without the caller's agent id would hand A a name that belongs to B.
    let _ = execute_tool_raw(
        "t1",
        "memory_store",
        &json!({"key": "file-acme-success-story", "value": "moat"}),
        &ctx_a,
    )
    .await;
    let _ = execute_tool_raw(
        "t2",
        "memory_store",
        &json!({"key": "file-acme-success-story-docx-v2", "value": "b-private"}),
        &ctx_b,
    )
    .await;

    let miss = execute_tool_raw(
        "t3",
        "memory_recall",
        &json!({"key": "file-acme-success-story-docx-v41"}),
        &ctx_a,
    )
    .await;

    assert!(!miss.is_error, "a miss is not an error: {}", miss.content);
    assert!(
        miss.content
            .starts_with("No value found for key 'file-acme-success-story-docx-v41'."),
        "the miss must still open with the exact-key sentence, got: {}",
        miss.content
    );
    assert!(
        miss.content.contains("'file-acme-success-story'"),
        "the miss must name the key that does exist, got: {}",
        miss.content
    );
    assert!(
        !miss.content.contains("docx-v2"),
        "the hint must not enumerate another agent's keys, got: {}",
        miss.content
    );
    assert!(
        miss.content.contains("memory_list") && miss.content.contains("memory_semantic_search"),
        "the miss must point at the full listing and the meaning-based store, got: {}",
        miss.content
    );

    let prefix_ids = probes.prefix_agent_ids.lock().unwrap();
    assert!(
        !prefix_ids.is_empty(),
        "the miss must probe the key space instead of answering bare"
    );
    assert!(
        prefix_ids
            .iter()
            .all(|id| id.as_deref() == Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")),
        "every hint probe must carry the caller's agent id, got: {prefix_ids:?}"
    );
}
