//! Regression coverage for #5139: the shared-KV (`memory_*`) and wiki
//! (`wiki_*`) tools must enforce the per-user `UserMemoryAccess` ACL at the
//! tool dispatch boundary, exactly like the proactive-retrieval path already
//! does. Before the fix a restricted user could drive `memory_store` /
//! `wiki_write` through the agent and reach cross-user shared state because
//! the dispatch site never consulted `memory_acl_for_sender`.
//!
//! These tests assert the SIDE EFFECT, not just the tool string: a denied
//! write must never reach the substrate (`store_calls` stays empty), a denied
//! read must never reach the substrate, and an allowed user's call must still
//! land. The ACL itself is the real `librefang_types::user_policy`
//! `UserMemoryAccess` evaluated through the real
//! `librefang_memory::namespace_acl::MemoryNamespaceGuard` — nothing about the
//! security predicate is mocked; only the substrate and the ACL *resolution*
//! (sender -> policy, normally done by the kernel `AuthManager`) are stubbed.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use librefang_types::user_policy::UserMemoryAccess;
use serde_json::json;
use std::sync::{Arc, Mutex};

type CallLog = Arc<Mutex<Vec<Option<String>>>>;

struct AclKernel {
    /// `peer_id` of every substrate write that actually landed.
    store_calls: CallLog,
    /// `peer_id` of every substrate recall that actually ran.
    recall_calls: CallLog,
    /// `peer_id` of every substrate list that actually ran.
    list_calls: CallLog,
    /// Number of wiki writes / reads that reached the vault.
    wiki_write_calls: Arc<Mutex<usize>>,
    wiki_read_calls: Arc<Mutex<usize>>,
    /// The ACL this kernel hands back from `memory_acl_for_sender`. `None`
    /// models "RBAC disabled / sender unattributed".
    acl: Option<UserMemoryAccess>,
}

struct AclProbes {
    store: CallLog,
    recall: CallLog,
    list: CallLog,
    wiki_write: Arc<Mutex<usize>>,
    wiki_read: Arc<Mutex<usize>>,
}

impl AclKernel {
    fn new(acl: Option<UserMemoryAccess>) -> (Self, AclProbes) {
        let store: CallLog = Arc::new(Mutex::new(Vec::new()));
        let recall: CallLog = Arc::new(Mutex::new(Vec::new()));
        let list: CallLog = Arc::new(Mutex::new(Vec::new()));
        let wiki_write = Arc::new(Mutex::new(0usize));
        let wiki_read = Arc::new(Mutex::new(0usize));
        let kernel = Self {
            store_calls: Arc::clone(&store),
            recall_calls: Arc::clone(&recall),
            list_calls: Arc::clone(&list),
            wiki_write_calls: Arc::clone(&wiki_write),
            wiki_read_calls: Arc::clone(&wiki_read),
            acl,
        };
        (
            kernel,
            AclProbes {
                store,
                recall,
                list,
                wiki_write,
                wiki_read,
            },
        )
    }
}

#[async_trait]
impl AgentControl for AclKernel {
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

impl MemoryAccess for AclKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.store_calls
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
        self.recall_calls
            .lock()
            .unwrap()
            .push(peer_id.map(|s| s.to_string()));
        Ok(Some(json!("secret-cross-user-value")))
    }
    fn memory_list(
        &self,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        self.list_calls
            .lock()
            .unwrap()
            .push(peer_id.map(|s| s.to_string()));
        Ok(vec!["leaked-key".to_string()])
    }
    fn memory_acl_for_sender(
        &self,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Option<UserMemoryAccess> {
        self.acl.clone()
    }
}

impl WikiAccess for AclKernel {
    fn wiki_get(
        &self,
        _topic: &str,
    ) -> Result<serde_json::Value, librefang_kernel_handle::KernelOpError> {
        *self.wiki_read_calls.lock().unwrap() += 1;
        Ok(json!({"topic": "secret", "body": "cross-user wiki page"}))
    }
    fn wiki_search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<serde_json::Value, librefang_kernel_handle::KernelOpError> {
        *self.wiki_read_calls.lock().unwrap() += 1;
        Ok(json!([{"topic": "secret", "snippet": "leak", "score": 1.0}]))
    }
    fn wiki_write(
        &self,
        _topic: &str,
        _body: &str,
        _provenance: serde_json::Value,
        _force: bool,
    ) -> Result<serde_json::Value, librefang_kernel_handle::KernelOpError> {
        *self.wiki_write_calls.lock().unwrap() += 1;
        Ok(json!({"status": "written"}))
    }
}

#[async_trait]
impl TaskQueue for AclKernel {
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
impl EventBus for AclKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for AclKernel {
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

impl CronControl for AclKernel {}
impl ApprovalGate for AclKernel {}
impl HandsControl for AclKernel {}
impl A2ARegistry for AclKernel {}
impl ChannelSender for AclKernel {}
impl PromptStore for AclKernel {}
impl WorkflowRunner for AclKernel {}
impl GoalControl for AclKernel {}
impl ToolPolicy for AclKernel {}
impl librefang_kernel_handle::CatalogQuery for AclKernel {}
impl librefang_kernel_handle::ApiAuth for AclKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}
impl librefang_kernel_handle::SessionWriter for AclKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl librefang_kernel_handle::AcpFsBridge for AclKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for AclKernel {}

fn make_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
    sender_id: Option<&'a str>,
    channel: Option<&'a str>,
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
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

/// A `viewer`-style ACL: may read `proactive` + `wiki` only; no writes at all.
/// This is the exact shape `default_memory_acl(Viewer)` produces.
fn viewer_acl() -> UserMemoryAccess {
    UserMemoryAccess {
        readable_namespaces: vec!["proactive".into(), "wiki".into()],
        writable_namespaces: vec![],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    }
}

/// A `user`-style ACL: read/write own `kv:*` + `wiki`. Mirrors
/// `default_memory_acl(User)`.
fn user_acl() -> UserMemoryAccess {
    UserMemoryAccess {
        readable_namespaces: vec!["proactive".into(), "kv:*".into(), "wiki".into()],
        writable_namespaces: vec!["kv:*".into(), "wiki".into()],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    }
}

// ── memory_store ─────────────────────────────────────────────────────────

#[tokio::test]
async fn restricted_user_memory_store_is_denied_and_does_not_land() {
    let (kernel, probes) = AclKernel::new(Some(viewer_acl()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("victim-peer"), Some("telegram"));
    let input = json!({"key": "k1", "value": "v1"});
    let result = execute_tool_raw("t1", "memory_store", &input, &ctx).await;

    assert!(
        result.is_error,
        "viewer (no writable namespaces) must be denied memory_store, got: {}",
        result.content
    );
    assert!(
        result.content.contains("Access denied"),
        "denial message should be explicit, got: {}",
        result.content
    );
    // SIDE EFFECT: the substrate write must NOT have happened.
    assert!(
        probes.store.lock().unwrap().is_empty(),
        "denied memory_store must never reach the substrate"
    );
}

#[tokio::test]
async fn allowed_user_memory_store_succeeds_and_lands() {
    let (kernel, probes) = AclKernel::new(Some(user_acl()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("alice"), Some("telegram"));
    let input = json!({"key": "k1", "value": "v1"});
    let result = execute_tool_raw("t1", "memory_store", &input, &ctx).await;

    assert!(
        !result.is_error,
        "user with kv:* write must be allowed: {}",
        result.content
    );
    let store_calls = probes.store.lock().unwrap();
    assert_eq!(store_calls.len(), 1, "allowed write must land");
    assert_eq!(store_calls[0], Some("alice".to_string()));
}

#[tokio::test]
async fn no_acl_means_no_restriction_store_still_lands() {
    // `memory_acl_for_sender` -> None models RBAC disabled / single-user.
    let (kernel, probes) = AclKernel::new(None);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("anyone"), None);
    let input = json!({"key": "k", "value": "v"});
    let result = execute_tool_raw("t1", "memory_store", &input, &ctx).await;

    assert!(!result.is_error, "no ACL => preserve pre-RBAC behaviour");
    assert_eq!(probes.store.lock().unwrap().len(), 1);
}

// ── memory_recall / memory_list ──────────────────────────────────────────

#[tokio::test]
async fn restricted_user_memory_recall_is_denied_and_does_not_read() {
    // ACL that can write its own kv but is NOT allowed to read kv at all
    // (e.g. an append-only inbox role). Recall of `kv:victim-peer` must fail.
    let acl = UserMemoryAccess {
        readable_namespaces: vec!["proactive".into()],
        writable_namespaces: vec!["kv:*".into()],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    };
    let (kernel, probes) = AclKernel::new(Some(acl));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("victim-peer"), Some("telegram"));
    let input = json!({"key": "k2"});
    let result = execute_tool_raw("t2", "memory_recall", &input, &ctx).await;

    assert!(
        result.is_error,
        "recall must be denied when kv read is not in the ACL: {}",
        result.content
    );
    assert!(
        probes.recall.lock().unwrap().is_empty(),
        "denied memory_recall must never reach the substrate (no cross-user leak)"
    );
}

#[tokio::test]
async fn restricted_user_memory_list_is_denied_and_does_not_enumerate() {
    let (kernel, probes) = AclKernel::new(Some(UserMemoryAccess {
        readable_namespaces: vec!["proactive".into()],
        writable_namespaces: vec![],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    }));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("victim-peer"), Some("telegram"));
    let result = execute_tool_raw("t3", "memory_list", &json!({}), &ctx).await;

    assert!(
        result.is_error,
        "list must be denied without kv read access: {}",
        result.content
    );
    assert!(
        probes.list.lock().unwrap().is_empty(),
        "denied memory_list must never enumerate the substrate"
    );
}

#[tokio::test]
async fn allowed_user_memory_recall_runs() {
    let (kernel, probes) = AclKernel::new(Some(user_acl()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("alice"), Some("telegram"));
    let result = execute_tool_raw("t2", "memory_recall", &json!({"key": "k2"}), &ctx).await;

    assert!(!result.is_error, "kv:* reader must be allowed recall");
    assert_eq!(probes.recall.lock().unwrap().len(), 1);
}

// ── wiki_get / wiki_search / wiki_write ──────────────────────────────────

#[tokio::test]
async fn restricted_user_wiki_write_is_denied_and_does_not_land() {
    // viewer has `wiki` read but no write.
    let (kernel, probes) = AclKernel::new(Some(viewer_acl()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("viewer-1"), Some("telegram"));
    let input = json!({"topic": "Roadmap", "body": "injected"});
    let result = execute_tool_raw("t4", "wiki_write", &input, &ctx).await;

    assert!(
        result.is_error,
        "viewer must be denied wiki_write: {}",
        result.content
    );
    assert_eq!(
        *probes.wiki_write.lock().unwrap(),
        0,
        "denied wiki_write must never reach the vault"
    );
}

#[tokio::test]
async fn restricted_user_wiki_read_denied_when_wiki_not_in_acl() {
    // An explicitly-configured ACL that omits `wiki` entirely: wiki reads
    // must fail closed (this is the path an operator uses to lock the vault).
    let acl = UserMemoryAccess {
        readable_namespaces: vec!["kv:*".into()],
        writable_namespaces: vec!["kv:*".into()],
        pii_access: false,
        export_allowed: false,
        delete_allowed: false,
    };
    let (kernel, probes) = AclKernel::new(Some(acl));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("u"), Some("telegram"));
    let get_res = execute_tool_raw("t5", "wiki_get", &json!({"topic": "X"}), &ctx).await;
    let search_res = execute_tool_raw("t6", "wiki_search", &json!({"query": "X"}), &ctx).await;

    assert!(
        get_res.is_error,
        "wiki_get must be denied: {}",
        get_res.content
    );
    assert!(
        search_res.is_error,
        "wiki_search must be denied: {}",
        search_res.content
    );
    assert_eq!(
        *probes.wiki_read.lock().unwrap(),
        0,
        "denied wiki reads must never reach the vault"
    );
}

#[tokio::test]
async fn allowed_user_wiki_roundtrip_works() {
    let (kernel, probes) = AclKernel::new(Some(user_acl()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = make_ctx(&kernel, Some("alice"), Some("telegram"));
    let w = execute_tool_raw(
        "t7",
        "wiki_write",
        &json!({"topic": "Doc", "body": "hello"}),
        &ctx,
    )
    .await;
    let g = execute_tool_raw("t8", "wiki_get", &json!({"topic": "Doc"}), &ctx).await;

    assert!(!w.is_error, "user may write wiki: {}", w.content);
    assert!(!g.is_error, "user may read wiki: {}", g.content);
    assert_eq!(*probes.wiki_write.lock().unwrap(), 1);
    assert_eq!(*probes.wiki_read.lock().unwrap(), 1);
}
