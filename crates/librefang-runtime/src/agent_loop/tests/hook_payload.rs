//! The `AgentLoopEnd` hook payload must carry the session the turn ran in.
//!
//! Turn-end consumers (skill workshop, RL export) read the turn from the session store, and the registry's `session_id` is not that session for channel-derived, conversation-keyed or `session_mode = "new"` turns.

use super::*;
use crate::hooks::{HookContext, HookHandler, HookRegistry};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
use librefang_types::agent::HookEvent;
use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
use std::sync::Mutex;

/// Driver that answers every request with a single text reply.
struct TextDriver;

#[async_trait]
impl LlmDriver for TextDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "ok".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 5,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// Records the `data` payload of every `AgentLoopEnd` event.
#[derive(Default)]
struct LoopEndRecorder {
    payloads: Mutex<Vec<serde_json::Value>>,
}

impl HookHandler for LoopEndRecorder {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
        if ctx.event == HookEvent::AgentLoopEnd {
            self.payloads.lock().unwrap().push(ctx.data.clone());
        }
        Ok(())
    }
}

#[tokio::test]
async fn agent_loop_end_payload_carries_turn_session_id() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        parent_session_id: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let session_id = session.id;
    let manifest = super::integration::test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(TextDriver);
    let recorder = Arc::new(LoopEndRecorder::default());
    let hooks = HookRegistry::new();
    hooks.register(HookEvent::AgentLoopEnd, recorder.clone());

    run_agent_loop(
        &manifest,
        "hello",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        Some(&hooks),
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("the loop must complete");

    let payloads = recorder.payloads.lock().unwrap();
    assert_eq!(payloads.len(), 1, "exactly one AgentLoopEnd per turn");
    assert_eq!(
        payloads[0].get("session_id").and_then(|v| v.as_str()),
        Some(session_id.to_string().as_str()),
        "payload: {}",
        payloads[0]
    );
}
