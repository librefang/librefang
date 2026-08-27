//! Tool-runner boundary coverage for the semantic-memory tools (#7808).
//!
//! Before this surface existed the only agent-callable "memory" was three
//! exact-key KV operations against `kv_store`, and `memory_search` — the most
//! natural name a model reaches for — was aliased to `memory_recall`, so asking
//! to search memory returned a hashmap miss reported as "not found".
//!
//! These tests exercise `execute_tool_raw` (the real dispatch path, including
//! the compat-name normalisation and the typed-error boundary) against a stub
//! `KernelHandle`. What is asserted here is the contract the runtime owns:
//! argument forwarding, input validation, limit clamping, honest reporting of a
//! no-op, the soft-`Denied` status for a policy refusal, and byte-stable stats
//! rendering. The ACL predicate itself lives kernel-side (see the design note
//! on `MemoryAccess::memory_semantic_search`) and is covered by the
//! `librefang-memory` `*_with_guard` tests.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use librefang_types::memory::{MemoryItem, MemoryLevel};
use librefang_types::tool::ToolExecutionStatus;
use librefang_types::user_policy::UserMemoryAccess;
use serde_json::json;
use std::sync::{Arc, Mutex};

/// A caller agent id in the shape the real dispatcher passes (a UUID string).
const CALLER: &str = "11111111-2222-3333-4444-555555555555";

/// What the stub kernel recorded for one semantic call.
#[derive(Debug, Clone, PartialEq)]
struct SearchCall {
    query: String,
    agent_id: String,
    limit: usize,
    min_confidence: Option<f32>,
    min_similarity: Option<f32>,
    sender_id: Option<String>,
    channel: Option<String>,
}

#[derive(Default)]
struct Probes {
    searches: Mutex<Vec<SearchCall>>,
    adds: Mutex<Vec<(String, String)>>,
    forgets: Mutex<Vec<(String, String)>>,
    stats: Mutex<usize>,
    duplicates: Mutex<usize>,
    consolidates: Mutex<Vec<String>>,
}

/// Stub kernel: records every semantic call and replays a scripted outcome.
struct AclKernel {
    probes: Arc<Probes>,
    /// Fragments `memory_semantic_search` returns.
    search_result: Vec<MemoryItem>,
    /// Fragments `memory_semantic_add` reports as stored.
    add_result: Vec<MemoryItem>,
    /// What `memory_semantic_forget` reports.
    forget_result: bool,
    /// Category counts `memory_semantic_stats` reports, in insertion order.
    stat_categories: Vec<(String, usize)>,
    /// Groups `memory_semantic_duplicates` reports.
    duplicate_groups: Vec<Vec<MemoryItem>>,
    /// Count `memory_semantic_consolidate` reports as retracted.
    consolidate_result: u64,
    /// When set, `memory_semantic_consolidate` refuses with this reason —
    /// standing in for the kernel's `allow_self_consolidation` gate.
    consolidate_denied: Option<String>,
    /// When set, every semantic method fails with this ACL refusal instead.
    deny: Option<String>,
}

impl AclKernel {
    fn new() -> Self {
        Self {
            probes: Arc::new(Probes::default()),
            search_result: Vec::new(),
            add_result: Vec::new(),
            forget_result: false,
            stat_categories: Vec::new(),
            duplicate_groups: Vec::new(),
            consolidate_result: 0,
            consolidate_denied: None,
            deny: None,
        }
    }
}

fn fragment(id: &str, content: &str, confidence: Option<f32>) -> MemoryItem {
    let mut item = MemoryItem::new(content.to_string(), MemoryLevel::Agent);
    item.id = id.to_string();
    item.confidence = confidence;
    item.category = Some("preference".to_string());
    item
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

#[async_trait]
impl MemoryAccess for AclKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
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
    fn memory_acl_for_sender(
        &self,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Option<UserMemoryAccess> {
        None
    }

    async fn memory_semantic_search(
        &self,
        query: &str,
        agent_id: &str,
        limit: usize,
        min_confidence: Option<f32>,
        min_similarity: Option<f32>,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<MemoryItem>, librefang_kernel_handle::KernelOpError> {
        self.probes.searches.lock().unwrap().push(SearchCall {
            query: query.to_string(),
            agent_id: agent_id.to_string(),
            limit,
            min_confidence,
            min_similarity,
            sender_id: sender_id.map(str::to_string),
            channel: channel.map(str::to_string),
        });
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        Ok(self.search_result.clone())
    }

    async fn memory_semantic_add(
        &self,
        content: &str,
        agent_id: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Result<Vec<MemoryItem>, librefang_kernel_handle::KernelOpError> {
        self.probes
            .adds
            .lock()
            .unwrap()
            .push((content.to_string(), agent_id.to_string()));
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        Ok(self.add_result.clone())
    }

    async fn memory_semantic_forget(
        &self,
        memory_id: &str,
        agent_id: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        self.probes
            .forgets
            .lock()
            .unwrap()
            .push((memory_id.to_string(), agent_id.to_string()));
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        Ok(self.forget_result)
    }

    async fn memory_semantic_stats(
        &self,
        _agent_id: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Result<serde_json::Value, librefang_kernel_handle::KernelOpError> {
        *self.probes.stats.lock().unwrap() += 1;
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        // Mirrors the kernel impl: categories go through a BTreeMap so the
        // rendered result is byte-identical regardless of insertion order.
        let categories: std::collections::BTreeMap<&str, usize> = self
            .stat_categories
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        Ok(json!({
            "total": 3,
            "user_count": 1,
            "session_count": 0,
            "agent_count": 2,
            "categories": categories,
            "enabled": true,
            "auto_memorize_enabled": true,
            "auto_retrieve_enabled": true,
            "llm_extraction": false,
        }))
    }

    async fn memory_semantic_duplicates(
        &self,
        _agent_id: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Result<Vec<Vec<MemoryItem>>, librefang_kernel_handle::KernelOpError> {
        *self.probes.duplicates.lock().unwrap() += 1;
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        Ok(self.duplicate_groups.clone())
    }

    async fn memory_semantic_consolidate(
        &self,
        agent_id: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> Result<u64, librefang_kernel_handle::KernelOpError> {
        self.probes
            .consolidates
            .lock()
            .unwrap()
            .push(agent_id.to_string());
        if let Some(reason) = &self.consolidate_denied {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        if let Some(reason) = &self.deny {
            return Err(librefang_kernel_handle::KernelOpError::AuthDenied(
                reason.clone(),
            ));
        }
        Ok(self.consolidate_result)
    }
}

impl WikiAccess for AclKernel {}

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
        _session_id: librefang_types::agent::SessionId,
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
        caller_agent_id: Some(CALLER),
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

// ── memory_semantic_search ───────────────────────────────────────────────

#[tokio::test]
async fn semantic_search_forwards_query_caller_and_defaults() {
    let mut stub = AclKernel::new();
    stub.search_result = vec![fragment("m-1", "prefers dark mode", Some(0.9))];
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, Some("alice"), Some("telegram"));
    let result = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "what do I know about themes?"}),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "search must succeed: {}", result.content);
    let calls = probes.searches.lock().unwrap();
    assert_eq!(calls.len(), 1, "search must reach the kernel exactly once");
    assert_eq!(
        calls[0],
        SearchCall {
            query: "what do I know about themes?".to_string(),
            // The caller's own agent id — semantic search is per-agent, never
            // cross-agent.
            agent_id: CALLER.to_string(),
            // Default breadth matches the automatic recall's MEMORY_RECALL_LIMIT.
            limit: 5,
            min_confidence: None,
            // Unset per call: the kernel resolves the agent's / deployment's
            // configured floor rather than the runtime inventing one (#7808).
            min_similarity: None,
            sender_id: Some("alice".to_string()),
            channel: Some("telegram".to_string()),
        }
    );
    // The id must be in the output — it is the only handle the model has for
    // `memory_semantic_forget`.
    assert!(
        result.content.contains("m-1"),
        "fragment id must be returned so it can be retracted, got: {}",
        result.content
    );
    assert!(result.content.contains("prefers dark mode"));
}

#[tokio::test]
async fn semantic_search_clamps_limit_and_forwards_min_confidence() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    for (asked, expected) in [(0_u64, 1_usize), (7, 7), (5000, 50)] {
        let _ = execute_tool_raw(
            "t1",
            "memory_semantic_search",
            &json!({"query": "q", "limit": asked, "min_confidence": 0.25}),
            &ctx,
        )
        .await;
        let call = probes.searches.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            call.limit, expected,
            "limit {asked} must clamp to {expected}, not reach the store unbounded"
        );
        assert_eq!(call.min_confidence, Some(0.25));
    }
}

#[tokio::test]
async fn semantic_search_rejects_missing_and_empty_query() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    for input in [json!({}), json!({"query": "   "})] {
        let result = execute_tool_raw("t1", "memory_semantic_search", &input, &ctx).await;
        assert!(
            result.is_error,
            "query {input} must be rejected, got: {}",
            result.content
        );
    }
    assert!(
        probes.searches.lock().unwrap().is_empty(),
        "an invalid query must never reach the store"
    );
}

#[tokio::test]
async fn semantic_search_rejects_out_of_range_min_confidence() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "q", "min_confidence": 4.2}),
        &ctx,
    )
    .await;
    assert!(result.is_error, "4.2 is not a confidence");
    assert!(
        result.content.contains("min_confidence"),
        "error must name the offending parameter, got: {}",
        result.content
    );
    assert!(probes.searches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn semantic_search_empty_result_names_the_store_it_searched() {
    // #7808 core complaint: "not found" from a tool called `memory_*` is
    // indistinguishable from a KV key miss, so the model retries the wrong
    // tool. The empty-result message has to say which store was consulted.
    let stub = AclKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "nothing here"}),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "an empty result set is not an error");
    assert!(
        result.content.contains("semantic memory"),
        "must say which store was searched, got: {}",
        result.content
    );
    assert!(
        result.content.contains("memory_recall") || result.content.contains("memory_list"),
        "must point at the KV tools for the other store, got: {}",
        result.content
    );
}

// ── the `memory_search` trap (#7808) ─────────────────────────────────────

#[tokio::test]
async fn memory_search_alias_reaches_semantic_search_not_kv_recall() {
    // Before the fix, `tool_compat::map_tool_name("memory_search")` resolved to
    // `memory_recall` — an exact-key KV lookup. A model asking to search its
    // memory got a hashmap miss reported as "not found", with no error to
    // signal that the call had been silently rerouted.
    let mut stub = AclKernel::new();
    stub.search_result = vec![fragment("m-9", "the deploy key rotated in March", None)];
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result =
        execute_tool_raw("t1", "memory_search", &json!({"query": "deploy key"}), &ctx).await;

    assert!(!result.is_error, "alias must work: {}", result.content);
    let calls = probes.searches.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "`memory_search` must land on semantic search, not KV recall"
    );
    assert_eq!(calls[0].query, "deploy key");
    assert!(result.content.contains("the deploy key rotated in March"));
}

// ── memory_semantic_add ──────────────────────────────────────────────────

#[tokio::test]
async fn semantic_add_forwards_trimmed_content_and_reports_what_landed() {
    let mut stub = AclKernel::new();
    stub.add_result = vec![fragment("m-2", "the user's timezone is JST", Some(0.8))];
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_add",
        &json!({"content": "  the user's timezone is JST  "}),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "add must succeed: {}", result.content);
    let adds = probes.adds.lock().unwrap();
    assert_eq!(
        adds[0],
        ("the user's timezone is JST".to_string(), CALLER.to_string())
    );
    assert!(
        result.content.contains("m-2"),
        "the stored id must come back so the write can be retracted, got: {}",
        result.content
    );
}

#[tokio::test]
async fn semantic_add_reports_a_no_op_instead_of_claiming_success() {
    // The extraction pipeline legitimately declines input it finds fact-free or
    // already recorded. Reporting that as "stored" would reproduce the exact
    // failure mode this issue is about: a tool that answers confidently about
    // something it did not do.
    let stub = AclKernel::new(); // add_result stays empty
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_add",
        &json!({"content": "hello"}),
        &ctx,
    )
    .await;

    assert!(
        !result.is_error,
        "a declined extraction is not a tool error"
    );
    assert!(
        result.content.contains("Nothing was stored"),
        "must say nothing landed, got: {}",
        result.content
    );
    assert!(
        result.content.contains("memory_store"),
        "must point at the tool that does store verbatim, got: {}",
        result.content
    );
}

#[tokio::test]
async fn semantic_add_rejects_empty_and_oversized_content() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let huge = "x".repeat(8 * 1024 + 1);
    for input in [
        json!({}),
        json!({"content": "  "}),
        json!({"content": huge}),
    ] {
        let result = execute_tool_raw("t1", "memory_semantic_add", &input, &ctx).await;
        assert!(result.is_error, "must reject, got: {}", result.content);
    }
    assert!(
        probes.adds.lock().unwrap().is_empty(),
        "invalid content must never reach the store"
    );
}

// ── memory_semantic_forget ───────────────────────────────────────────────

#[tokio::test]
async fn semantic_forget_forwards_id_and_confirms_deletion() {
    let mut stub = AclKernel::new();
    stub.forget_result = true;
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_forget",
        &json!({"memory_id": "  7f4a1c30-0000-4000-8000-000000000003  "}),
        &ctx,
    )
    .await;

    assert!(!result.is_error, "forget must succeed: {}", result.content);
    assert_eq!(
        probes.forgets.lock().unwrap()[0],
        (
            "7f4a1c30-0000-4000-8000-000000000003".to_string(),
            CALLER.to_string()
        ),
        "the id must be trimmed and scoped to the calling agent"
    );
    assert!(result.content.contains("Forgot"));
}

#[tokio::test]
async fn semantic_forget_reports_a_miss_without_claiming_deletion() {
    let stub = AclKernel::new(); // forget_result = false
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw(
        "t1",
        "memory_semantic_forget",
        &json!({"memory_id": "7f4a1c30-0000-4000-8000-0000000000ff"}),
        &ctx,
    )
    .await;

    assert!(!result.is_error);
    assert!(
        result.content.contains("nothing was deleted"),
        "a miss must not read as a success, got: {}",
        result.content
    );
}

#[tokio::test]
async fn semantic_forget_rejects_a_malformed_id_as_a_caller_mistake() {
    // A hallucinated id must come back as a recoverable parameter error. Letting
    // the store reject it produces `Internal`, a hard failure that counts toward
    // the agent loop's consecutive-failure abort.
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    for id in ["", "   ", "the-one-about-dark-mode"] {
        let result = execute_tool_raw(
            "t1",
            "memory_semantic_forget",
            &json!({"memory_id": id}),
            &ctx,
        )
        .await;
        assert!(
            result.is_error,
            "{id:?} must be rejected: {}",
            result.content
        );
        assert!(
            result.content.contains("memory_semantic_search"),
            "{id:?}: the error must name the recovery step, got: {}",
            result.content
        );
    }
    assert!(
        probes.forgets.lock().unwrap().is_empty(),
        "a malformed id must never reach the store"
    );
}

// ── memory_semantic_stats ────────────────────────────────────────────────

#[tokio::test]
async fn semantic_stats_reports_counts_and_subsystem_flags() {
    let mut stub = AclKernel::new();
    stub.stat_categories = vec![("preference".to_string(), 2), ("fact".to_string(), 1)];
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw("t1", "memory_semantic_stats", &json!({}), &ctx).await;

    assert!(!result.is_error, "stats must succeed: {}", result.content);
    assert_eq!(*probes.stats.lock().unwrap(), 1);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).expect("stats is JSON");
    assert_eq!(parsed["total"], 3);
    assert_eq!(parsed["categories"]["preference"], 2);
    // The flags are the point: "search found nothing" and "semantic memory is
    // switched off" are different answers and the agent has to tell them apart.
    assert_eq!(parsed["enabled"], true);
    assert_eq!(parsed["llm_extraction"], false);
}

/// Determinism guard (#3298). `MemoryStats::categories` is a `HashMap`, so its
/// iteration order varies per process. This result lands in the message history
/// verbatim — a reordered one invalidates the provider prompt cache for every
/// later turn in the conversation even though nothing changed.
#[tokio::test]
async fn semantic_stats_rendering_is_byte_identical_across_insertion_orders() {
    let names: Vec<String> = (0..24).map(|i| format!("category_{i:02}")).collect();

    let forward: Vec<(String, usize)> =
        names.into_iter().enumerate().map(|(i, n)| (n, i)).collect();
    let mut reversed = forward.clone();
    reversed.reverse();
    // A third, interleaved order — reversal alone can accidentally pass for a
    // container that happens to sort on read.
    let mut shuffled = Vec::new();
    let (a, b) = forward.split_at(forward.len() / 2);
    for (x, y) in b.iter().zip(a.iter()) {
        shuffled.push(y.clone());
        shuffled.push(x.clone());
    }
    let mut renderings = Vec::new();
    for order in [forward, reversed, shuffled] {
        let mut stub = AclKernel::new();
        stub.stat_categories = order;
        let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
        let ctx = make_ctx(&kernel, None, None);
        let result = execute_tool_raw("t1", "memory_semantic_stats", &json!({}), &ctx).await;
        assert!(!result.is_error, "stats must succeed: {}", result.content);
        renderings.push(result.content);
    }

    assert_eq!(
        renderings[0], renderings[1],
        "stats rendering must not depend on category insertion order"
    );
    assert_eq!(
        renderings[1], renderings[2],
        "stats rendering must not depend on category insertion order"
    );
    // And the keys must actually be sorted, not merely stable.
    let parsed: serde_json::Value = serde_json::from_str(&renderings[0]).unwrap();
    let keys: Vec<&String> = parsed["categories"]
        .as_object()
        .expect("categories is an object")
        .keys()
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "category keys must be emitted in sorted order"
    );
}

// ── ACL refusal contract ─────────────────────────────────────────────────

/// A per-user policy refusal is permanent and non-fatal. It must arrive as the
/// soft `ToolExecutionStatus::Denied` so it does not count toward the agent
/// loop's consecutive-hard-failure abort — the regression #5984 fixed for the
/// KV tools. The semantic tools resolve their ACL kernel-side, so the refusal
/// comes back as `KernelOpError::AuthDenied` rather than from
/// `enforce_memory_acl`, and needs its own mapping.
#[tokio::test]
async fn semantic_tools_surface_an_acl_refusal_as_soft_denied() {
    for tool in [
        "memory_semantic_search",
        "memory_semantic_add",
        "memory_semantic_forget",
        "memory_semantic_stats",
    ] {
        let mut stub = AclKernel::new();
        stub.deny =
            Some("memory namespace 'proactive' is not writable for the current user".into());
        let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
        let ctx = make_ctx(&kernel, Some("viewer-user"), Some("telegram"));

        let input = json!({"query": "q", "content": "c", "memory_id": "7f4a1c30-0000-4000-8000-000000000001"});
        let result = execute_tool_raw("t1", tool, &input, &ctx).await;

        assert!(result.is_error, "{tool}: denial must be an error result");
        assert_eq!(
            result.status,
            ToolExecutionStatus::Denied,
            "{tool}: an ACL denial must be soft `Denied`, not a hard failure that \
             death-spirals the turn"
        );
        assert!(
            result.content.contains("proactive"),
            "{tool}: the reason must survive to the model, got: {}",
            result.content
        );
    }
}

// ── the runtime half of capability gating ────────────────────────────────

/// `execute_tool` (not the raw dispatcher) is where the `allowed_tools`
/// allowlist is enforced, so these go through it. Everything else is `None` /
/// zero — the semantic tools need only the kernel handle.
async fn execute_with_allowlist(
    tool: &str,
    input: &serde_json::Value,
    kernel: &Arc<dyn KernelHandle>,
    allowed: Option<&[String]>,
) -> librefang_types::tool::ToolResult {
    librefang_runtime::tool_runner::execute_tool(
        "t1",
        tool,
        input,
        Some(kernel),
        allowed,
        Some(CALLER),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0,
        0,
    )
    .await
}

/// A declaration that does not cover the semantic tools must block the call
/// outright, not merely hide the schema — an LLM can name a tool it was never
/// shown.
#[tokio::test]
async fn semantic_tools_respect_the_allowed_tools_allowlist() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let allowed = vec!["file_read".to_string(), "memory_recall".to_string()];
    for tool in [
        "memory_semantic_search",
        "memory_semantic_add",
        "memory_semantic_forget",
        "memory_semantic_stats",
    ] {
        let input = json!({"query": "q", "content": "c", "memory_id": "7f4a1c30-0000-4000-8000-000000000001"});
        let result = execute_with_allowlist(tool, &input, &kernel, Some(&allowed)).await;
        assert!(result.is_error, "{tool} must be denied: {}", result.content);
        assert!(
            result.content.contains("Permission denied"),
            "{tool}: got {}",
            result.content
        );
    }
    assert!(
        probes.searches.lock().unwrap().is_empty(),
        "a tool outside the allowlist must never reach the kernel"
    );
    assert!(probes.adds.lock().unwrap().is_empty());
    assert!(probes.forgets.lock().unwrap().is_empty());
    assert_eq!(*probes.stats.lock().unwrap(), 0);
}

/// The common `memory_*` declaration glob-matches the new names, so an operator
/// who granted the KV tools reaches the semantic ones through the runtime
/// allowlist too. That is precisely why the kernel gates them a second time on
/// the declared memory scopes — see the `available_tools` gate (#7808) and its
/// tests in `librefang-kernel`.
#[tokio::test]
async fn memory_glob_in_allowed_tools_admits_the_semantic_tools() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let allowed = vec!["memory_*".to_string()];
    let result = execute_with_allowlist(
        "memory_semantic_search",
        &json!({"query": "q"}),
        &kernel,
        Some(&allowed),
    )
    .await;

    assert!(!result.is_error, "got: {}", result.content);
    assert_eq!(
        probes.searches.lock().unwrap().len(),
        1,
        "`memory_*` glob-matches `memory_semantic_search`, so the runtime allowlist \
         alone cannot express \"KV yes, semantic no\""
    );
}

// ── min_similarity (#7808) ───────────────────────────────────────────────

/// The floor a caller names has to arrive at the kernel intact. Nothing else in
/// this file can tell a dropped argument from a store that had nothing to
/// filter, so pin the forwarding directly.
#[tokio::test]
async fn semantic_search_forwards_min_similarity() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    let _ = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "q", "min_similarity": 0.35}),
        &ctx,
    )
    .await;

    let call = probes.searches.lock().unwrap().last().cloned().unwrap();
    assert_eq!(call.min_similarity, Some(0.35));
    assert_eq!(
        call.min_confidence, None,
        "the two floors measure different things and must not be conflated"
    );
}

/// Cosine is signed, so the accepted range is [-1, 1] — and a number outside it
/// is a caller mistake worth naming rather than silently clamping, because
/// clamping 35 to 1.0 would return an empty set the model cannot explain.
#[tokio::test]
async fn semantic_search_rejects_out_of_range_min_similarity() {
    let stub = AclKernel::new();
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    for bad in [1.5, -2.0, 35.0] {
        let result = execute_tool_raw(
            "t1",
            "memory_semantic_search",
            &json!({"query": "q", "min_similarity": bad}),
            &ctx,
        )
        .await;
        assert!(result.is_error, "min_similarity {bad} must be rejected");
        assert!(
            result.content.contains("min_similarity"),
            "the refusal must name the parameter, got: {}",
            result.content
        );
    }
    assert!(
        probes.searches.lock().unwrap().is_empty(),
        "a rejected argument must never reach the store"
    );

    // -1.0 and 1.0 are both legitimate cosine values and must be accepted.
    for ok in [-1.0, 0.0, 1.0] {
        let result = execute_tool_raw(
            "t1",
            "memory_semantic_search",
            &json!({"query": "q", "min_similarity": ok}),
            &ctx,
        )
        .await;
        assert!(!result.is_error, "min_similarity {ok} is valid cosine");
    }
}

/// An empty result under a floor is ambiguous — "nothing is stored" versus
/// "nothing cleared the bar" — and a model that reads the first will stop
/// asking. The result has to say which.
#[tokio::test]
async fn semantic_search_empty_under_a_floor_says_the_floor_emptied_it() {
    let stub = AclKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let with_floor = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "themes", "min_similarity": 0.9}),
        &ctx,
    )
    .await;
    assert!(!with_floor.is_error);
    assert!(
        with_floor.content.contains("min_similarity"),
        "an empty result under a floor must name the floor, got: {}",
        with_floor.content
    );

    let without_floor = execute_tool_raw(
        "t1",
        "memory_semantic_search",
        &json!({"query": "themes"}),
        &ctx,
    )
    .await;
    assert!(
        !without_floor.content.contains("min_similarity"),
        "an unfiltered empty result must not blame a floor nobody set, got: {}",
        without_floor.content
    );
}

/// The score the ranker measured has to reach the model, or it cannot judge how
/// much a fragment is worth trusting or choose a floor for the next call.
#[tokio::test]
async fn semantic_search_reports_the_similarity_of_each_fragment() {
    let mut stub = AclKernel::new();
    let mut scored = fragment("m-1", "prefers dark mode", Some(0.9));
    scored.similarity = Some(0.82);
    let unscored = fragment("m-2", "lives in Berlin", Some(0.9));
    stub.search_result = vec![scored, unscored];
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    let result =
        execute_tool_raw("t1", "memory_semantic_search", &json!({"query": "q"}), &ctx).await;

    assert!(!result.is_error, "got: {}", result.content);
    let rows: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let reported = rows[0]["similarity"].as_f64().unwrap();
    assert!(
        (reported - 0.82).abs() < 1e-6,
        "the measured score must be reported, got {reported}"
    );
    assert!(
        rows[1].get("similarity").is_none(),
        "an unmeasured fragment must omit the field, never report 0.0 — 0.0 is a measured miss: {}",
        result.content
    );
}

// ── memory_semantic_duplicates ───────────────────────────────────────────

#[tokio::test]
async fn semantic_duplicates_reports_groups_and_what_merging_would_cost() {
    let mut stub = AclKernel::new();
    stub.duplicate_groups = vec![
        vec![
            fragment("d-1", "the deploy runs on Friday", Some(0.9)),
            fragment("d-2", "deploys happen Fridays", Some(0.8)),
            fragment("d-3", "we deploy each Friday", Some(0.7)),
        ],
        // A singleton is not a duplicate: `find_duplicates` seeds one group per
        // unabsorbed memory, so rendering these would bury the finding under
        // the whole store.
        vec![fragment("d-4", "unrelated fact", Some(0.9))],
    ];
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    let result = execute_tool_raw("t1", "memory_semantic_duplicates", &json!({}), &ctx).await;

    assert!(!result.is_error, "got: {}", result.content);
    assert_eq!(*probes.duplicates.lock().unwrap(), 1);
    assert!(result.content.contains("d-1") && result.content.contains("d-3"));
    assert!(
        !result.content.contains("d-4"),
        "a group of one is not a duplicate group: {}",
        result.content
    );
    assert!(
        result.content.contains("retract the other 2"),
        "the report must state what consolidating would delete, got: {}",
        result.content
    );
    // Reporting is not deleting.
    assert!(probes.consolidates.lock().unwrap().is_empty());
    assert!(probes.forgets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn semantic_duplicates_reports_a_clean_store_plainly() {
    let stub = AclKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw("t1", "memory_semantic_duplicates", &json!({}), &ctx).await;
    assert!(!result.is_error);
    assert!(
        result.content.contains("No near-duplicate memories"),
        "got: {}",
        result.content
    );
}

// ── memory_semantic_consolidate ──────────────────────────────────────────

#[tokio::test]
async fn semantic_consolidate_reports_how_many_memories_it_retracted() {
    let mut stub = AclKernel::new();
    stub.consolidate_result = 4;
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    let result = execute_tool_raw("t1", "memory_semantic_consolidate", &json!({}), &ctx).await;

    assert!(!result.is_error, "got: {}", result.content);
    assert_eq!(
        probes.consolidates.lock().unwrap().as_slice(),
        [CALLER.to_string()],
        "consolidation is scoped to the calling agent and no other"
    );
    assert!(
        result.content.contains('4'),
        "the count of retracted memories must be reported, got: {}",
        result.content
    );
}

/// A no-op has to read as a no-op. "Consolidated" with nothing merged would
/// teach the model that the pile it is worried about has been dealt with.
#[tokio::test]
async fn semantic_consolidate_reports_a_no_op_without_claiming_success() {
    let stub = AclKernel::new();
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);
    let ctx = make_ctx(&kernel, None, None);

    let result = execute_tool_raw("t1", "memory_semantic_consolidate", &json!({}), &ctx).await;
    assert!(!result.is_error);
    assert!(
        result.content.contains("Nothing to consolidate"),
        "got: {}",
        result.content
    );
}

/// The kernel's `allow_self_consolidation` refusal must arrive as the soft
/// `Denied` status, not a hard error.
///
/// This is the difference between "you are not allowed to do that, carry on"
/// and three consecutive hard failures aborting the turn — and a permanent
/// policy refusal will repeat every time the model retries, so routing it as a
/// hard failure death-spirals exactly the agents that most want the tool.
#[tokio::test]
async fn semantic_consolidate_surfaces_the_opt_in_refusal_as_soft_denied() {
    let mut stub = AclKernel::new();
    stub.consolidate_denied = Some(
        "agent may not consolidate its own semantic memory: set allow_self_consolidation".into(),
    );
    let probes = Arc::clone(&stub.probes);
    let kernel: Arc<dyn KernelHandle> = Arc::new(stub);

    let ctx = make_ctx(&kernel, None, None);
    let result = execute_tool_raw("t1", "memory_semantic_consolidate", &json!({}), &ctx).await;

    assert!(result.is_error, "a refusal is still an error result");
    assert_eq!(
        result.status,
        ToolExecutionStatus::Denied,
        "a policy refusal must be soft-Denied, not a hard failure: {}",
        result.content
    );
    assert!(
        result.content.contains("allow_self_consolidation"),
        "the refusal must name the setting that would allow it, got: {}",
        result.content
    );
    // The gate refused before anything was deleted.
    assert_eq!(probes.consolidates.lock().unwrap().len(), 1);
}

// ── tool declarations ────────────────────────────────────────────────────

#[tokio::test]
async fn semantic_tools_are_declared_and_search_ships_on_every_turn() {
    use librefang_runtime::tool_runner::{builtin_tool_definitions, select_native_tools};

    let defs = builtin_tool_definitions();
    for name in [
        "memory_semantic_search",
        "memory_semantic_add",
        "memory_semantic_forget",
        "memory_semantic_stats",
        "memory_semantic_duplicates",
        "memory_semantic_consolidate",
    ] {
        assert!(
            defs.iter().any(|d| d.name == name),
            "{name} must appear in builtin_tool_definitions or no agent can ever call it"
        );
    }

    // The destructive tool must say so in the schema the model reads. A
    // description that reads like maintenance invites a call the agent would
    // not have made if it knew what the call costs (#7808).
    let consolidate = defs
        .iter()
        .find(|d| d.name == "memory_semantic_consolidate")
        .unwrap();
    assert!(
        consolidate
            .description
            .contains("memory_semantic_duplicates")
            && consolidate.description.contains("allow_self_consolidation"),
        "the consolidate schema must point at the read-only alternative and name the opt-in: {}",
        consolidate.description
    );

    // #7808: the search tool has to be visible without a `tool_load`
    // round-trip, because the tool it competes with (`memory_recall`) already is.
    let native = select_native_tools(&defs);
    assert!(
        native.iter().any(|d| d.name == "memory_semantic_search"),
        "memory_semantic_search must be in ALWAYS_NATIVE_TOOLS"
    );

    // The KV descriptions must state what they do NOT do, so a model choosing
    // between the two stores can tell them apart from the schema alone.
    let recall = defs.iter().find(|d| d.name == "memory_recall").unwrap();
    assert!(
        recall.description.contains("EXACT")
            && recall.description.contains("memory_semantic_search"),
        "memory_recall's description must disclaim search and point at the real one, got: {}",
        recall.description
    );
}

/// Repeated calls must produce a byte-identical tool list. The definitions are
/// the prompt prefix — a reordered one invalidates the provider prompt cache on
/// every turn even when nothing changed (#3298).
#[test]
fn builtin_tool_definitions_are_byte_stable() {
    use librefang_runtime::tool_runner::builtin_tool_definitions;

    let first = serde_json::to_string(&builtin_tool_definitions()).unwrap();
    let second = serde_json::to_string(&builtin_tool_definitions()).unwrap();
    assert_eq!(first, second, "builtin tool definitions must be stable");

    // #3298: these strings are stringified into every request. The semantic
    // tools carry JSON-object schemas, and a `HashMap` anywhere in one would
    // reorder its keys between processes and invalidate the provider prompt
    // cache for the rest of the conversation even though nothing changed.
    // Serialize the semantic slice on its own so a regression names the tool
    // rather than pointing at the whole list.
    let semantic = |()| -> String {
        serde_json::to_string(
            &builtin_tool_definitions()
                .into_iter()
                .filter(|d| d.name.starts_with("memory_semantic_"))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    assert_eq!(
        semantic(()),
        semantic(()),
        "the memory_semantic_* definitions must be byte-identical across builds of the list"
    );

    let names: Vec<String> = builtin_tool_definitions()
        .iter()
        .map(|d| d.name.clone())
        .collect();
    let mut deduped = names.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(
        names.len(),
        deduped.len(),
        "a duplicate tool name would make the list order-dependent: {names:?}"
    );
}
