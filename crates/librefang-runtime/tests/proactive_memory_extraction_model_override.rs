//! Integration test for the per-agent `[proactive_memory] extraction_model`
//! override (#5475).
//!
//! Verifies that `LlmMemoryExtractor::extract_memories_with_agent_id`
//! consults the installed `KernelHandle::proactive_memory_extraction_model_for`
//! and routes the LLM call through the resolved model name instead of
//! the boot-time `self.model`.
//!
//! Strategy:
//! 1. Build a recording `LlmDriver` that captures the `model` field of
//!    every `CompletionRequest`.
//! 2. Construct an `LlmMemoryExtractor` with model `"global-extractor"`.
//! 3. Install a stub `KernelHandle` that returns a different model
//!    (`"agent-override-model"`) from `proactive_memory_extraction_model_for`.
//! 4. Call `extract_memories_with_agent_id` and assert the recorded
//!    model on the captured request equals the override, not
//!    `"global-extractor"`.
//! 5. As a control, call with an agent the stub claims has no override
//!    and assert the request used `"global-extractor"`.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_types::memory::MemoryExtractor;
use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
use serde_json::json;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Minimal stub KernelHandle — only `CatalogQuery` carries the per-agent
// override; every other role trait stays at its default empty impl.
//
// The stub keys overrides on the agent_id string passed verbatim. Real
// kernel impls parse it as a `Uuid`, but the extractor's resolver only
// passes the string through, so a string-keyed map matches what the
// production kernel sees.
// ---------------------------------------------------------------------------

struct OverrideKernel {
    overrides: std::collections::HashMap<String, String>,
}

impl OverrideKernel {
    fn new(overrides: &[(&str, &str)]) -> Self {
        Self {
            overrides: overrides
                .iter()
                .map(|(a, m)| ((*a).to_string(), (*m).to_string()))
                .collect(),
        }
    }
}

#[async_trait]
impl AgentControl for OverrideKernel {
    async fn spawn_agent(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    async fn send_to_agent(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for OverrideKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
}

impl WikiAccess for OverrideKernel {}

#[async_trait]
impl TaskQueue for OverrideKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    async fn task_claim(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    async fn task_complete(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    async fn task_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
    async fn task_delete(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
    async fn task_retry(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
    async fn task_get(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    async fn task_update_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
}

#[async_trait]
impl EventBus for OverrideKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

#[async_trait]
impl KnowledgeGraph for OverrideKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("stub".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Ok(vec![])
    }
}

impl CronControl for OverrideKernel {}
impl ApprovalGate for OverrideKernel {}
impl HandsControl for OverrideKernel {}
impl A2ARegistry for OverrideKernel {}
impl ChannelSender for OverrideKernel {}
impl PromptStore for OverrideKernel {}
impl WorkflowRunner for OverrideKernel {}
impl GoalControl for OverrideKernel {}
impl ToolPolicy for OverrideKernel {}

impl librefang_kernel_handle::ApiAuth for OverrideKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for OverrideKernel {
    fn inject_attachment_blocks(
        &self,
        _: librefang_types::agent::AgentId,
        _: librefang_types::agent::SessionId,
        _: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for OverrideKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for OverrideKernel {}

impl librefang_kernel_handle::CatalogQuery for OverrideKernel {
    fn proactive_memory_extraction_model_for(&self, agent_id: &str) -> Option<String> {
        self.overrides.get(agent_id).cloned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn build_extractor_with_kernel(
    captured_models: Arc<Mutex<Vec<String>>>,
    overrides: &[(&str, &str)],
) -> Arc<librefang_runtime::proactive_memory::LlmMemoryExtractor> {
    let driver = Arc::new(SharedRecordingDriver {
        captured_models,
        // Minimal valid extraction JSON so parse_llm_extraction_response succeeds.
        response_json: r#"[]"#.to_string(),
    });

    let extractor = Arc::new(
        librefang_runtime::proactive_memory::LlmMemoryExtractor::with_prompt_caching(
            driver,
            "global-extractor".to_string(),
            true,
        ),
    );

    let kernel: Arc<dyn KernelHandle> = Arc::new(OverrideKernel::new(overrides));
    extractor.install_kernel_handle(Arc::downgrade(&kernel));
    // The kernel must outlive the extractor for the Weak ref to upgrade.
    // Leak it for the duration of the test — tests do not run long enough
    // for this to matter and the alternative (returning both) makes the
    // helper signature unwieldy.
    std::mem::forget(kernel);

    extractor
}

// Recording driver that writes into a shared captured-models vec.
struct SharedRecordingDriver {
    captured_models: Arc<Mutex<Vec<String>>>,
    response_json: String,
}

#[async_trait]
impl librefang_runtime::llm_driver::LlmDriver for SharedRecordingDriver {
    async fn complete(
        &self,
        request: librefang_runtime::llm_driver::CompletionRequest,
    ) -> Result<
        librefang_runtime::llm_driver::CompletionResponse,
        librefang_runtime::llm_driver::LlmError,
    > {
        self.captured_models
            .lock()
            .unwrap()
            .push(request.model.clone());
        Ok(librefang_runtime::llm_driver::CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.response_json.clone(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            actual_provider: None,
        })
    }
    fn is_configured(&self) -> bool {
        true
    }
}

fn sample_messages() -> Vec<serde_json::Value> {
    vec![
        json!({"role": "user", "content": "I love Rust."}),
        json!({"role": "assistant", "content": "Noted."}),
    ]
}

#[tokio::test]
async fn agent_override_wins_over_boot_time_model() {
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let agent_id = "agent-with-override";
    let extractor =
        build_extractor_with_kernel(Arc::clone(&captured), &[(agent_id, "agent-override-model")]);

    let result = extractor
        .extract_memories_with_agent_id(&sample_messages(), agent_id, &[])
        .await
        .expect("extraction should succeed");
    assert!(!result.has_content, "stub returned empty extraction list");

    let calls = captured.lock().unwrap();
    assert_eq!(calls.len(), 1, "expected exactly one driver.complete call");
    assert_eq!(
        calls[0], "agent-override-model",
        "per-agent override should replace boot-time model"
    );
}

#[tokio::test]
async fn no_override_falls_back_to_boot_time_model() {
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    // Stub returns None for this agent_id (not in the overrides map).
    let extractor = build_extractor_with_kernel(Arc::clone(&captured), &[]);

    extractor
        .extract_memories_with_agent_id(&sample_messages(), "agent-with-no-override", &[])
        .await
        .expect("extraction should succeed");

    let calls = captured.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0], "global-extractor",
        "missing override should fall back to boot-time model"
    );
}

#[tokio::test]
async fn provider_qualified_override_strips_prefix_at_request_time() {
    // Operators set `extraction_model = "anthropic/claude-haiku-4-5"`.
    // The boot path already strips the provider prefix on the global
    // model; the per-agent override path must do the same so the
    // upstream API sees a valid bare model name.
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let extractor = build_extractor_with_kernel(
        Arc::clone(&captured),
        &[("agent-x", "anthropic/claude-haiku-4-5")],
    );

    extractor
        .extract_memories_with_agent_id(&sample_messages(), "agent-x", &[])
        .await
        .expect("extraction should succeed");

    let calls = captured.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0], "claude-haiku-4-5",
        "provider/ prefix should be stripped before the wire request"
    );
}

#[tokio::test]
async fn colon_form_override_strips_prefix_at_request_time() {
    // `extraction_model = "openai:gpt-4o-mini"` (the `[llm.auxiliary]`
    // colon convention). Same stripping treatment as `provider/model`.
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let extractor =
        build_extractor_with_kernel(Arc::clone(&captured), &[("agent-y", "openai:gpt-4o-mini")]);

    extractor
        .extract_memories_with_agent_id(&sample_messages(), "agent-y", &[])
        .await
        .expect("extraction should succeed");

    let calls = captured.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "gpt-4o-mini");
}
