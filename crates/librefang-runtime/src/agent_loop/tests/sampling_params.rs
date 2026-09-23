//! #8290 — the typed sampling parameters, tested at the two places the agent loop builds a `CompletionRequest`.
//!
//! The driver wire tests prove each driver places `top_p` / `frequency_penalty` / `presence_penalty` correctly once they are on the request, and the `inference_params` tests prove the resolver puts them on the manifest.
//! Neither notices if `agent_loop/mod.rs` or `run_streaming.rs` stops copying them across: the fields are `Option`, so `top_p: None` (or a move to `..Default::default()`) compiles cleanly and every driver then sees `None`.
//! These tests drive both entry points end to end and assert on the request the driver was handed.

use super::*;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
use std::sync::Mutex;

/// Driver that answers immediately and keeps every request it was handed.
struct RecordingDriver {
    seen: Mutex<Vec<CompletionRequest>>,
}

impl RecordingDriver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }

    /// The first request the loop built.
    fn first_request(&self) -> CompletionRequest {
        let seen = self.seen.lock().unwrap();
        seen.first()
            .expect("the loop must have called the driver")
            .clone()
    }
}

#[async_trait]
impl LlmDriver for RecordingDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.seen.lock().unwrap().push(request);
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

fn manifest_with_sampling() -> AgentManifest {
    let mut manifest = super::integration::test_manifest();
    manifest.model.top_p = Some(0.9);
    manifest.model.frequency_penalty = Some(0.5);
    manifest.model.presence_penalty = Some(-0.25);
    manifest
}

fn blank_session() -> Session {
    Session {
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
    }
}

fn assert_typed_sampling(recorder: &RecordingDriver) {
    let request = recorder.first_request();
    assert_eq!(request.top_p, Some(0.9));
    assert_eq!(request.frequency_penalty, Some(0.5));
    assert_eq!(request.presence_penalty, Some(-0.25));
    // The values travel on the typed fields only; `extra_body` is the untyped escape hatch and must not carry a second copy.
    assert!(
        request.extra_body.is_none(),
        "extra_body: {:?}",
        request.extra_body
    );
}

#[tokio::test]
async fn non_streaming_loop_copies_sampling_params_onto_the_request() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = blank_session();
    let manifest = manifest_with_sampling();
    let recorder = RecordingDriver::new();
    let driver: Arc<dyn LlmDriver> = recorder.clone();

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
        None, // hooks
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

    assert_typed_sampling(&recorder);
}

#[tokio::test]
async fn streaming_loop_copies_sampling_params_onto_the_request() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = blank_session();
    let manifest = manifest_with_sampling();
    let recorder = RecordingDriver::new();
    let driver: Arc<dyn LlmDriver> = recorder.clone();
    let (stream_tx, mut stream_rx) = mpsc::channel(64);
    // Drain the stream so the driver's `tx.send` never blocks on a full channel.
    let drain = tokio::spawn(async move { while stream_rx.recv().await.is_some() {} });

    run_agent_loop_streaming(
        &manifest,
        "hello",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        stream_tx,
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
        None, // hooks
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
    .expect("the streaming loop must complete");
    drop(drain);

    assert_typed_sampling(&recorder);
}
