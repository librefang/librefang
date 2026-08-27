// Integration tests for the `agent_type_create` tool (#7722).
//
// The handler is thin by design — every rule that decides whether the write is legal lives kernel-side in the shared `agent_type_store`, so what these tests pin is the seam: what the schema promises the model, what crosses the trait boundary, and whether a refusal comes back as something the model can act on next turn.
// The end-to-end proof that a type created here is the same document `GET /api/templates/{name}` serves is in `crates/librefang-api/tests/agent_types_routes_integration.rs`, which runs the real kernel against a real router.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Outcome the stub create_agent_type returns
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum StubCreateResult {
    Ok,
    /// The store refused the name — the kernel's `InvalidInput`.
    Invalid(&'static str),
    /// An agent type or a live agent already answers to the name — the kernel's `Conflict`.
    NameTaken(&'static str),
}

// ---------------------------------------------------------------------------
// Stub kernel
// ---------------------------------------------------------------------------

struct AgentTypeStubKernel {
    create_result: StubCreateResult,
    /// The `(name, spec)` pair the last `create_agent_type` call received, so the tests can assert what actually crossed the trait boundary rather than only what came back.
    create_seen: Mutex<Option<(String, librefang_types::agent_type::AgentTypeSpec)>>,
}

impl AgentTypeStubKernel {
    fn with_create(create_result: StubCreateResult) -> Self {
        Self {
            create_result,
            create_seen: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Boilerplate trait impls (same pattern as tool_runner_workflow_readonly.rs)
// ---------------------------------------------------------------------------

#[async_trait]
impl AgentControl for AgentTypeStubKernel {
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
    async fn create_agent_type(
        &self,
        name: &str,
        spec: librefang_types::agent_type::AgentTypeSpec,
    ) -> Result<AgentTypeSummary, librefang_kernel_handle::KernelOpError> {
        *self.create_seen.lock().unwrap() = Some((name.to_string(), spec.clone()));
        match &self.create_result {
            StubCreateResult::Ok => Ok(AgentTypeSummary {
                name: name.to_string(),
                description: spec.description.unwrap_or_default(),
                // The store substitutes the `"default"` sentinel for an omitted provider or model, and the tool has to report what was stored rather than what was sent — so the stub does the same substitution.
                provider: spec.provider.unwrap_or_else(|| "default".to_string()),
                model: spec.model.unwrap_or_else(|| "default".to_string()),
                tools: spec.tools.unwrap_or_default(),
                skills: spec.skills.unwrap_or_default(),
            }),
            StubCreateResult::Invalid(reason) => Err(
                librefang_kernel_handle::KernelOpError::InvalidInput(reason.to_string()),
            ),
            StubCreateResult::NameTaken(reason) => Err(
                librefang_kernel_handle::KernelOpError::Conflict(reason.to_string()),
            ),
        }
    }

    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for AgentTypeStubKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

impl WikiAccess for AgentTypeStubKernel {}

#[async_trait]
impl TaskQueue for AgentTypeStubKernel {
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
impl EventBus for AgentTypeStubKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for AgentTypeStubKernel {
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

impl CronControl for AgentTypeStubKernel {}
impl ApprovalGate for AgentTypeStubKernel {}
impl HandsControl for AgentTypeStubKernel {}
impl A2ARegistry for AgentTypeStubKernel {}
impl ChannelSender for AgentTypeStubKernel {}
impl PromptStore for AgentTypeStubKernel {}
impl GoalControl for AgentTypeStubKernel {}
impl ToolPolicy for AgentTypeStubKernel {}
impl librefang_kernel_handle::CatalogQuery for AgentTypeStubKernel {}

impl librefang_kernel_handle::ApiAuth for AgentTypeStubKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for AgentTypeStubKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for AgentTypeStubKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for AgentTypeStubKernel {}

// ---------------------------------------------------------------------------
// A kernel that never overrides `create_agent_type`, pinning the trait default
// ---------------------------------------------------------------------------

struct NoAgentTypesKernel;

#[async_trait]
impl AgentControl for NoAgentTypesKernel {
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

impl MemoryAccess for NoAgentTypesKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

impl WikiAccess for NoAgentTypesKernel {}

#[async_trait]
impl TaskQueue for NoAgentTypesKernel {
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
impl EventBus for NoAgentTypesKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl KnowledgeGraph for NoAgentTypesKernel {
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

impl CronControl for NoAgentTypesKernel {}
impl ApprovalGate for NoAgentTypesKernel {}
impl HandsControl for NoAgentTypesKernel {}
impl A2ARegistry for NoAgentTypesKernel {}
impl ChannelSender for NoAgentTypesKernel {}
impl PromptStore for NoAgentTypesKernel {}
impl GoalControl for NoAgentTypesKernel {}
impl ToolPolicy for NoAgentTypesKernel {}
impl librefang_kernel_handle::CatalogQuery for NoAgentTypesKernel {}

impl librefang_kernel_handle::ApiAuth for NoAgentTypesKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for NoAgentTypesKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for NoAgentTypesKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for NoAgentTypesKernel {}

// ---------------------------------------------------------------------------
// Helper: build a minimal ToolExecContext
// ---------------------------------------------------------------------------

fn make_ctx(kernel: &Arc<dyn KernelHandle>) -> ToolExecContext<'_> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"), // mock-only: non-UUID ok since mock kernel ignores agent_id
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

impl WorkflowRunner for AgentTypeStubKernel {}
impl WorkflowRunner for NoAgentTypesKernel {}

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

fn definition() -> librefang_types::tool::ToolDefinition {
    builtin_tool_definitions()
        .into_iter()
        .find(|d| d.name == "agent_type_create")
        .expect("agent_type_create missing from builtin_tool_definitions")
}

#[test]
fn agent_type_create_schema_exposes_exactly_the_flat_spec() {
    let def = definition();
    assert_eq!(def.input_schema["type"], "object");

    let keys: Vec<&str> = def.input_schema["properties"]
        .as_object()
        .expect("properties object")
        .keys()
        .map(String::as_str)
        .collect();
    // The seven keys are `AgentTypeSpec`'s fields. Advertising one the spec cannot deserialize would be refused by `deny_unknown_fields` at the boundary; omitting one the spec accepts would hide a settable field from the only caller that reads this schema.
    assert_eq!(
        keys,
        vec![
            "description",
            "model",
            "name",
            "provider",
            "skills",
            "system_prompt",
            "tools"
        ]
    );

    let required: Vec<&str> = def.input_schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    // Identity is the one field a create cannot invent; every other field has a documented manifest default.
    assert_eq!(required, vec!["name"]);
}

/// #3298: a tool definition is stringified into the LLM request on every turn that ships it, so a property order that varies between processes invalidates the provider's prompt cache while the content is unchanged.
///
/// Asserting the keys are sorted is stronger than asserting they are stable within one process: it holds whether `serde_json::Map` is backed by an insertion-ordered `IndexMap` (feature-unified `preserve_order`) or by a sorted `BTreeMap`, which is a property of the dependency graph rather than of this crate.
#[test]
fn agent_type_create_schema_property_order_is_deterministic() {
    let def = definition();
    let keys: Vec<String> = def.input_schema["properties"]
        .as_object()
        .expect("properties object")
        .keys()
        .cloned()
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "agent_type_create's schema properties must be written in sorted order"
    );

    // And the rendering itself is byte-identical across two independent serializations of the same definition.
    assert_eq!(
        serde_json::to_string(&definition().input_schema).unwrap(),
        serde_json::to_string(&def.input_schema).unwrap()
    );
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn create_payload() -> serde_json::Value {
    json!({
        "name": "release-notes-writer",
        "description": "turns merged PRs into release prose",
        "system_prompt": "You write release notes.",
        "provider": "anthropic",
        "model": "claude-sonnet-4-5",
        "tools": ["file_read", "web_search"],
        "skills": ["changelog-style"]
    })
}

#[tokio::test]
async fn agent_type_create_forwards_the_whole_spec_under_the_validated_name() {
    let stub = Arc::new(AgentTypeStubKernel::with_create(StubCreateResult::Ok));
    let kernel: Arc<dyn KernelHandle> = stub.clone();
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "agent_type_create", &create_payload(), &ctx).await;
    assert!(
        !result.is_error,
        "agent_type_create failed: {}",
        result.content
    );

    let v: serde_json::Value = serde_json::from_str(&result.content).expect("valid JSON");
    assert_eq!(v["name"], "release-notes-writer");
    assert_eq!(v["provider"], "anthropic");
    assert_eq!(v["model"], "claude-sonnet-4-5");
    assert_eq!(v["tools"], json!(["file_read", "web_search"]));
    assert_eq!(v["skills"], json!(["changelog-style"]));

    let (seen_name, seen_spec) = stub
        .create_seen
        .lock()
        .unwrap()
        .clone()
        .expect("the kernel handle must have been called");
    assert_eq!(seen_name, "release-notes-writer");
    assert_eq!(
        seen_spec.system_prompt.as_deref(),
        Some("You write release notes."),
        "every field the model sent must reach the kernel — the handler is not allowed to rebuild a subset of the spec"
    );
    assert_eq!(
        seen_spec.description.as_deref(),
        Some("turns merged PRs into release prose")
    );
    assert_eq!(
        seen_spec.tools,
        Some(vec!["file_read".to_string(), "web_search".to_string()])
    );
    assert_eq!(seen_spec.skills, Some(vec!["changelog-style".to_string()]));
}

/// An empty system prompt is a deliberate instruction, not an omission, and it has to survive the whole way to the kernel.
/// Collapsing `Some("")` into `None` is what turns a blank prompt into canned text on the operator's disk (#7740).
#[tokio::test]
async fn agent_type_create_preserves_a_deliberately_blank_system_prompt() {
    let stub = Arc::new(AgentTypeStubKernel::with_create(StubCreateResult::Ok));
    let kernel: Arc<dyn KernelHandle> = stub.clone();
    let ctx = make_ctx(&kernel);

    let payload = json!({ "name": "blank", "system_prompt": "" });
    let result = execute_tool_raw("t1", "agent_type_create", &payload, &ctx).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let (_, seen_spec) = stub.create_seen.lock().unwrap().clone().expect("called");
    assert_eq!(seen_spec.system_prompt.as_deref(), Some(""));
}

#[tokio::test]
async fn agent_type_create_missing_name_returns_error() {
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(AgentTypeStubKernel::with_create(StubCreateResult::Ok));
    let ctx = make_ctx(&kernel);

    let mut payload = create_payload();
    payload.as_object_mut().unwrap().remove("name");
    let result = execute_tool_raw("t1", "agent_type_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for missing name");
    assert!(
        result.content.contains("name"),
        "error should mention name: {}",
        result.content
    );
}

/// `AgentTypeSpec` is `deny_unknown_fields`, and that refusal has to reach the model by name.
/// Dropping an unrecognised key silently and answering 200 is how a model comes away believing it set a temperature it did not set.
#[tokio::test]
async fn agent_type_create_names_a_field_the_spec_does_not_have() {
    let stub = Arc::new(AgentTypeStubKernel::with_create(StubCreateResult::Ok));
    let kernel: Arc<dyn KernelHandle> = stub.clone();
    let ctx = make_ctx(&kernel);

    let mut payload = create_payload();
    payload["temperature"] = json!(0.2);
    let result = execute_tool_raw("t1", "agent_type_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for an unknown field");
    assert!(
        result.content.contains("temperature"),
        "the offending key must be named: {}",
        result.content
    );
    assert!(
        stub.create_seen.lock().unwrap().is_none(),
        "a spec that failed the boundary must never reach the kernel"
    );
}

/// A name collision is something the model can fix on its next turn — by picking another name — so the rejection has to arrive as a readable reason against the `name` field, not as an opaque upstream failure.
#[tokio::test]
async fn agent_type_create_relays_a_name_collision() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(AgentTypeStubKernel::with_create(
        StubCreateResult::NameTaken("an agent type with that name already exists"),
    ));
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "agent_type_create", &create_payload(), &ctx).await;
    assert!(result.is_error, "expected error for a taken name");
    assert!(
        result.content.contains("already exists"),
        "the collision reason must survive to the model: {}",
        result.content
    );
    assert!(
        result.content.contains("name"),
        "the collision must be attributed to the name field: {}",
        result.content
    );
}

#[tokio::test]
async fn agent_type_create_relays_an_unusable_name() {
    let kernel: Arc<dyn KernelHandle> =
        Arc::new(AgentTypeStubKernel::with_create(StubCreateResult::Invalid(
            "an agent type name must be 1-64 characters of letters, digits, '_' or '-'",
        )));
    let ctx = make_ctx(&kernel);

    let mut payload = create_payload();
    payload["name"] = json!("../../etc/passwd");
    let result = execute_tool_raw("t1", "agent_type_create", &payload, &ctx).await;
    assert!(result.is_error, "expected error for an unusable name");
    assert!(
        result.content.contains("letters, digits"),
        "the reason must tell the model what a legal name looks like: {}",
        result.content
    );
}

/// The trait method carries a default so adding it did not break every stub implementor in the workspace.
/// That default has to fail loudly rather than pretend the type was written.
#[tokio::test]
async fn agent_type_create_reports_a_kernel_that_cannot_author_agent_types() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(NoAgentTypesKernel);
    let ctx = make_ctx(&kernel);

    let result = execute_tool_raw("t1", "agent_type_create", &create_payload(), &ctx).await;
    assert!(result.is_error, "expected an error from the trait default");
    assert!(
        result.content.contains("create_agent_type"),
        "the unavailable capability must be named: {}",
        result.content
    );
}
