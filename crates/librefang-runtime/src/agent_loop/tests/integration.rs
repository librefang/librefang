use super::*;

// --- Integration tests for empty response guards ---

pub(super) fn test_manifest() -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
        source_template: None,
        model: librefang_types::agent::ModelConfig {
            system_prompt: "You are a test agent.".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Mock driver that simulates: first call returns ToolUse with no text,
/// second call returns EndTurn with empty text. This reproduces the bug
/// where the LLM ends with no text after a tool-use cycle.
struct EmptyAfterToolUseDriver {
    call_count: AtomicU32,
}

impl EmptyAfterToolUseDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for EmptyAfterToolUseDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            // First call: LLM wants to use a tool (with no text block)
            Ok(CompletionResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "fake_tool".to_string(),
                    input: serde_json::json!({"query": "test"}),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ToolCall {
                    id: "tool_1".to_string(),
                    name: "fake_tool".to_string(),
                    input: serde_json::json!({"query": "test"}),
                }],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        } else {
            // Second call: LLM returns EndTurn with EMPTY text (the bug)
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

/// Mock driver: iteration 0 emits a tool call, iteration 1 emits text.
/// Used to verify the loop retries after a tool failure instead of exiting.
pub(super) struct FailThenTextDriver {
    call_count: AtomicU32,
}

impl FailThenTextDriver {
    pub(super) fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for FailThenTextDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            Ok(CompletionResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "fake_tool".to_string(),
                    input: serde_json::json!({"q": "test"}),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ToolCall {
                    id: "tool_1".to_string(),
                    name: "fake_tool".to_string(),
                    input: serde_json::json!({"q": "test"}),
                }],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        } else {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Recovered after tool failure".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

/// Mock driver: every iteration emits a tool call that will fail (unregistered tool).
/// Used to verify the consecutive_all_failed cap triggers RepeatedToolFailures.
pub(super) struct AlwaysFailingToolDriver;

#[async_trait]
impl LlmDriver for AlwaysFailingToolDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool_x".to_string(),
                name: "nonexistent_tool".to_string(),
                input: serde_json::json!({}),
                provider_metadata: None,
            }],
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![ToolCall {
                id: "tool_x".to_string(),
                name: "nonexistent_tool".to_string(),
                input: serde_json::json!({}),
            }],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// Mock driver that returns empty text with MaxTokens stop reason,
/// repeated MAX_CONTINUATIONS times to trigger the max continuations path.
struct EmptyMaxTokensDriver;

#[async_trait]
impl LlmDriver for EmptyMaxTokensDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![],
            stop_reason: StopReason::MaxTokens,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 0,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// Mock driver that returns normal text (sanity check).
pub(super) struct NormalDriver;

#[async_trait]
impl LlmDriver for NormalDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "Hello from the agent!".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 8,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

struct DirectiveDriver {
    text: &'static str,
    stop_reason: StopReason,
}

#[async_trait]
impl LlmDriver for DirectiveDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.text.to_string(),
                provider_metadata: None,
            }],
            stop_reason: self.stop_reason,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 8,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

struct NotifyOwnerThenMaxTokensDriver {
    call_count: AtomicU32,
    final_tool_calls: bool,
}

impl NotifyOwnerThenMaxTokensDriver {
    fn new(final_tool_calls: bool) -> Self {
        Self {
            call_count: AtomicU32::new(0),
            final_tool_calls,
        }
    }
}

#[async_trait]
impl LlmDriver for NotifyOwnerThenMaxTokensDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        match call {
            0 => Ok(CompletionResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "notify_1".to_string(),
                    name: "notify_owner".to_string(),
                    input: serde_json::json!({
                        "reason": "handoff_needed",
                        "summary": "Fallback provider needs owner visibility."
                    }),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ToolCall {
                    id: "notify_1".to_string(),
                    name: "notify_owner".to_string(),
                    input: serde_json::json!({
                        "reason": "handoff_needed",
                        "summary": "Fallback provider needs owner visibility."
                    }),
                }],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: Some("fallback-a".to_string()),
                actual_model: None,
            }),
            _ if self.final_tool_calls => Ok(CompletionResponse {
                content: vec![
                    ContentBlock::Text {
                        text: "Partial after owner notice".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::ToolUse {
                        id: format!("continue_{call}"),
                        name: "notify_owner".to_string(),
                        input: serde_json::json!({
                            "reason": "continuation_needed",
                            "summary": "Max tokens branch is continuing."
                        }),
                        provider_metadata: None,
                    },
                ],
                stop_reason: StopReason::MaxTokens,
                tool_calls: vec![ToolCall {
                    id: format!("continue_{call}"),
                    name: "notify_owner".to_string(),
                    input: serde_json::json!({
                        "reason": "continuation_needed",
                        "summary": "Max tokens branch is continuing."
                    }),
                }],
                usage: TokenUsage {
                    input_tokens: 12,
                    output_tokens: 6,
                    ..Default::default()
                },
                actual_provider: Some("fallback-b".to_string()),
                actual_model: Some("actual-model-x".to_string()),
            }),
            _ => Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Partial after owner notice".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::MaxTokens,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 12,
                    output_tokens: 6,
                    ..Default::default()
                },
                actual_provider: Some("fallback-b".to_string()),
                actual_model: Some("actual-model-x".to_string()),
            }),
        }
    }
}

struct CascadeLeakTimedOutDriver;

#[async_trait]
impl LlmDriver for CascadeLeakTimedOutDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        unreachable!("streaming test must use stream")
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        tx.send(StreamEvent::TextDelta {
            text: "User asked: hi\nI responded: secret".to_string(),
        })
        .await
        .expect("proxy stream receiver should be alive");
        Err(LlmError::TimedOut {
            inactivity_secs: 30,
            partial_text: Some(std::sync::Arc::<str>::from(
                "User asked: hi\nI responded: timed out secret",
            )),
            partial_text_len: 43,
            last_activity: "text_delta".to_string(),
        })
    }
}

fn fresh_session() -> librefang_memory::session::Session {
    librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    }
}

fn notify_owner_tool_definition() -> ToolDefinition {
    fake_tool("notify_owner")
}

fn session_texts(session: &librefang_memory::session::Session) -> Vec<&str> {
    session
        .messages
        .iter()
        .filter_map(|message| match &message.content {
            MessageContent::Text(text) => Some(text.as_str()),
            MessageContent::Blocks(_) => None,
        })
        .collect()
}

fn assert_saved_max_tokens_session(
    persisted: Option<librefang_memory::session::Session>,
    should_persist: bool,
    should_have_continue_prompt: bool,
    label: &str,
) {
    let Some(persisted) = persisted else {
        assert!(
            !should_persist,
            "{label}: session should have been persisted"
        );
        return;
    };

    assert!(
        should_persist,
        "{label}: session should not have been persisted"
    );
    let texts = session_texts(&persisted);
    assert!(
        texts.contains(&"Notify owner before hitting max tokens"),
        "{label}: persisted session lost original user message: {texts:?}"
    );
    assert!(
        texts.contains(&"Partial after owner notice"),
        "{label}: persisted session lost MaxTokens assistant partial: {texts:?}"
    );
    assert_eq!(
        texts.contains(&"Please continue."),
        should_have_continue_prompt,
        "{label}: persisted session continuation prompt mismatch: {texts:?}"
    );
}

#[allow(clippy::too_many_arguments)]
async fn run_streaming_for_test(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut librefang_memory::session::Session,
    memory: &librefang_memory::MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    stream_tx: mpsc::Sender<StreamEvent>,
    opts: &LoopOptions,
) -> LibreFangResult<AgentLoopResult> {
    run_agent_loop_streaming(
        manifest,
        user_message,
        session,
        memory,
        driver,
        available_tools,
        None,
        stream_tx,
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
        opts,
    )
    .await
}

#[tokio::test]
async fn test_empty_response_after_tool_use_returns_fallback() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());

    let result = run_agent_loop(
        &manifest,
        "Do something with tools",
        &mut session,
        &memory,
        driver,
        &[], // no tools registered — the tool call will fail, which is fine
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
    .expect("Loop should complete without error");

    // The response MUST NOT be empty — it should contain our fallback text
    assert!(
        !result.response.trim().is_empty(),
        "Response should not be empty after tool use, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("Permission denied") || result.response.contains("Task completed"),
        "Expected tool error or fallback message, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_empty_response_max_tokens_returns_fallback() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);

    let result = run_agent_loop(
        &manifest,
        "Tell me something long",
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
    .expect("Loop should complete without error");

    // Should hit MAX_CONTINUATIONS and return fallback instead of empty
    assert!(
        !result.response.trim().is_empty(),
        "Response should not be empty on max tokens, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("token limit"),
        "Expected max-tokens fallback message, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_normal_response_not_replaced_by_fallback() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

    let result = run_agent_loop(
        &manifest,
        "Say hello",
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
    .expect("Loop should complete without error");

    // Normal response should pass through unchanged
    assert_eq!(result.response, "Hello from the agent!");
}

#[tokio::test]
async fn test_success_response_preserves_reply_directives() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "[[reply:msg_123]] [[@current]] Visible reply",
        stop_reason: StopReason::EndTurn,
    });

    let result = run_agent_loop(
        &manifest,
        "Reply to this",
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
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // checkpoint_manager
        None, // process_registry
        None,
        None,
        None,
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("Loop should complete without error");

    assert_eq!(result.response, "Visible reply");
    assert_eq!(result.directives.reply_to.as_deref(), Some("msg_123"));
    assert!(result.directives.current_thread);
    assert!(!result.directives.silent);
}

#[tokio::test]
async fn test_max_tokens_partial_response_preserves_reply_directives() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "[[reply:msg_999]] [[@current]] Partial answer",
        stop_reason: StopReason::MaxTokens,
    });

    let result = run_agent_loop(
        &manifest,
        "Tell me more",
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
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // checkpoint_manager
        None, // process_registry
        None,
        None,
        None,
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("Loop should complete without error");

    assert_eq!(result.response, "Partial answer");
    // Pure-text max_tokens overflow short-circuits on iter 1 (#2310).
    assert_eq!(result.iterations, 1);
    assert_eq!(result.directives.reply_to.as_deref(), Some("msg_999"));
    assert!(result.directives.current_thread);
    assert!(!result.directives.silent);
}

async fn run_max_tokens_owner_notice_case(
    opts: LoopOptions,
    final_tool_calls: bool,
) -> (
    librefang_memory::MemorySubstrate,
    librefang_memory::session::Session,
    AgentLoopResult,
) {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> =
        Arc::new(NotifyOwnerThenMaxTokensDriver::new(final_tool_calls));
    let tool = notify_owner_tool_definition();

    let result = run_agent_loop(
        &manifest,
        "Notify owner before hitting max tokens",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&tool),
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
        &opts,
    )
    .await
    .expect("Loop should complete with max-tokens partial");

    (memory, session, result)
}

#[tokio::test]
async fn test_max_tokens_owner_notice_and_actual_provider_survive_non_streaming() {
    let (_memory, _session, result) =
        run_max_tokens_owner_notice_case(LoopOptions::default(), false).await;

    assert_eq!(result.response, "Partial after owner notice");
    assert_eq!(
        result.owner_notice.as_deref(),
        Some("[NOTIFY] handoff_needed: Fallback provider needs owner visibility.")
    );
    assert_eq!(result.actual_provider.as_deref(), Some("fallback-b"));
    // #6134: the model the call actually ran threads through to AgentLoopResult.
    assert_eq!(result.actual_model.as_deref(), Some("actual-model-x"));
}

#[tokio::test]
async fn test_max_tokens_session_save_respects_ephemeral_options() {
    for (label, opts, should_persist) in [
        ("default", LoopOptions::default(), true),
        (
            "incognito",
            LoopOptions {
                incognito: true,
                ..LoopOptions::default()
            },
            false,
        ),
        (
            "fork",
            LoopOptions {
                is_fork: true,
                ..LoopOptions::default()
            },
            false,
        ),
    ] {
        let (memory, session, result) = run_max_tokens_owner_notice_case(opts, false).await;
        assert_eq!(result.response, "Partial after owner notice", "{label}");
        let persisted = memory
            .get_session(session.id)
            .expect("get_session must not error");
        assert_saved_max_tokens_session(persisted, should_persist, false, label);
    }
}

#[tokio::test]
async fn test_max_tokens_session_save_respects_ephemeral_options_on_continuation() {
    for (label, opts, should_persist) in [
        ("default", LoopOptions::default(), true),
        (
            "incognito",
            LoopOptions {
                incognito: true,
                ..LoopOptions::default()
            },
            false,
        ),
        (
            "fork",
            LoopOptions {
                is_fork: true,
                ..LoopOptions::default()
            },
            false,
        ),
    ] {
        let (memory, session, result) = run_max_tokens_owner_notice_case(opts, true).await;
        assert_eq!(result.response, "Partial after owner notice", "{label}");
        assert_eq!(result.iterations, MAX_CONTINUATIONS + 1, "{label}");
        let persisted = memory
            .get_session(session.id)
            .expect("get_session must not error");
        assert_saved_max_tokens_session(persisted, should_persist, true, label);
    }
}

// ── History-fold integration test ────────────────────────────────────────
//
// Drives `run_agent_loop` through enough tool-use / tool-result cycles to
// push earlier turns past the `history_fold_after_turns` boundary, then
// asserts that the fold stub was observed in a CompletionRequest sent to
// the primary driver.  A mock aux driver returns deterministic summaries
// so the test does not require a live LLM key.
//
// The fold operates on the local `messages` slice used for LLM calls (not
// `session.messages` directly), so the assertion captures the request that
// the primary driver received: at least one message in that request must
// start with the "[history-fold]" prefix.

/// Driver that emits `N` tool-use rounds then finishes with EndTurn text.
/// Each tool-use call hits the meta-tool `tool_search` (which always succeeds
/// with `is_error=false` even against an empty registry — see
/// `tool_runner::tool_meta_search`). A succeeding tool keeps
/// `consecutive_all_failed = 0` so the `MAX_CONSECUTIVE_ALL_FAILED = 3`
/// circuit breaker does not abort the loop before the fold path runs.
/// Earlier draft used `probe_tool` (unknown → hard error returned by
/// the loop), accumulating tool-result messages in the working history.
/// Also records all CompletionRequest message lists it receives so the
/// test can assert that fold stubs appeared in a request.
struct MultiToolCycleDriver {
    call_count: AtomicU32,
    tool_cycles: u32,
    // Flattened snapshot of all messages seen across all complete() calls.
    seen_messages: std::sync::Mutex<Vec<librefang_types::message::Message>>,
}

impl MultiToolCycleDriver {
    fn new(tool_cycles: u32) -> Self {
        Self {
            call_count: AtomicU32::new(0),
            tool_cycles,
            seen_messages: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LlmDriver for MultiToolCycleDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Record the messages this call received.
        {
            let mut guard = self.seen_messages.lock().unwrap();
            guard.extend(request.messages.iter().cloned());
        }
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call < self.tool_cycles {
            Ok(CompletionResponse {
                content: vec![ContentBlock::ToolUse {
                    id: format!("tid_{call}"),
                    name: "tool_search".to_string(),
                    input: serde_json::json!({"query": format!("probe-{call}")}),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ToolCall {
                    id: format!("tid_{call}"),
                    name: "tool_search".to_string(),
                    input: serde_json::json!({"query": format!("probe-{call}")}),
                }],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 3,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        } else {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "All done after many tool cycles.".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 8,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

/// Deterministic aux driver for fold summarisation: returns a fixed
/// summary string without any network call.
struct FoldSummaryDriver;

#[async_trait]
impl LlmDriver for FoldSummaryDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "probe_tool ran and returned output.".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 5,
                output_tokens: 8,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// Verifies that the history-fold path is exercised end-to-end:
/// after enough tool-use cycles the fold path replaces stale tool-result
/// messages with compact `[history-fold]` stubs that are visible in the
/// CompletionRequest messages delivered to the primary driver.
#[tokio::test]
async fn test_history_fold_stub_appears_in_llm_request_after_enough_tool_cycles() {
    use crate::aux_client::AuxClient;
    use librefang_types::config::ToolResultsConfig;

    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();

    // Primary driver: 10 tool-use rounds then EndTurn; records all
    // CompletionRequest.messages it receives.
    let primary = Arc::new(MultiToolCycleDriver::new(10));
    let driver: Arc<dyn LlmDriver> = Arc::clone(&primary) as Arc<dyn LlmDriver>;

    // Aux driver: deterministic fold summariser (no live LLM required).
    // Wire it as the primary driver of an AuxClient that has no chain
    // configuration, so every AuxTask resolves directly to FoldSummaryDriver.
    let aux_driver: Arc<dyn LlmDriver> = Arc::new(FoldSummaryDriver);
    let aux_client = AuxClient::with_primary_only(aux_driver);

    // fold_after_turns=3 so turns 0..6 are stale by the time we have 10
    // assistant turns, guaranteeing at least one fold group before the
    // final LLM call that returns EndTurn.  `fold_min_batch_size: 1`
    // disables the cost amortiser so the test exercises fold on the
    // first eligible turn instead of waiting for 4 stale messages.
    let tool_results_cfg = ToolResultsConfig {
        history_fold_after_turns: 3,
        fold_min_batch_size: 1,
        ..ToolResultsConfig::default()
    };

    let loop_opts = LoopOptions {
        aux_client: Some(Arc::new(aux_client)),
        tool_results_config: Some(tool_results_cfg),
        ..LoopOptions::default()
    };

    // `tool_search` is dispatched by name in `tool_runner::execute_tool_raw`,
    // but the outer `execute_tool` enforces the capability allowlist
    // (`available_tool_names`) which is built from this `&[ToolDefinition]`
    // slice — so the meta-tool name still has to appear here, otherwise
    // the agent_loop returns a "Permission denied" hard error.
    let tool_search_def = fake_tool("tool_search");
    let result = run_agent_loop(
        &manifest,
        "Run many tool cycles",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&tool_search_def),
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
        &loop_opts,
    )
    .await
    .expect("Loop should complete without error");

    // The loop must finish and produce a non-empty final response.
    assert!(
        !result.response.trim().is_empty(),
        "expected non-empty final response, got: {:?}",
        result.response
    );

    // At least one message that the primary driver received across all
    // calls must be a [history-fold] stub — this proves fold_stale_tool_results
    // ran and replaced stale tool-result entries before the LLM call.
    // The prefix "[history-fold]" mirrors `history_fold::FOLD_PREFIX`.
    // Post-#1 review: fold now rewrites `ContentBlock::ToolResult.content`
    // in place (preserving tool_use_id pairing), so we look for the
    // prefix inside ToolResult blocks rather than in a Text-content
    // message.
    const FOLD_PREFIX_STR: &str = "[history-fold]";
    let seen = primary.seen_messages.lock().unwrap();
    let fold_stub_found = seen.iter().any(|m| match &m.content {
        librefang_types::message::MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
            matches!(
                b,
                librefang_types::message::ContentBlock::ToolResult { content, .. }
                    if content.starts_with(FOLD_PREFIX_STR)
            )
        }),
        _ => false,
    });
    assert!(
        fold_stub_found,
        "expected at least one [history-fold] ToolResult stub in a CompletionRequest after 10 \
         tool cycles with fold_after_turns=3; messages seen by primary driver: {:#?}",
        seen.iter()
            .map(|m| format!("{:?}: {:?}", m.role, m.content))
            .collect::<Vec<_>>()
    );
}

/// Axis-2 wiring regression for #4866: `maybe_fold_stale_tool_results`
/// must replay the fold rewrites onto `session.messages` (not just the
/// working clone) AND advance `messages_generation` via
/// `mark_messages_mutated()` so `save_session_async` persists the
/// rewrite.  A future refactor in this wrapper could silently drop
/// either of those steps; the unit tests inside `history_fold` cover
/// the function in isolation and would not catch that.
#[tokio::test]
async fn maybe_fold_stale_tool_results_persists_rewrites_to_session_messages() {
    use crate::history_fold::FoldConfig;
    use librefang_types::tool::ToolExecutionStatus;

    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    // 10 turns of (assistant, tool_result) — under fold_after=2 every
    // tool_result older than the last two assistant turns is stale.
    session
        .messages
        .push(librefang_types::message::Message::user("start"));
    for i in 0..10 {
        session
            .messages
            .push(librefang_types::message::Message::assistant(format!(
                "asst {i}"
            )));
        session.messages.push(librefang_types::message::Message {
            role: librefang_types::message::Role::User,
            content: librefang_types::message::MessageContent::Blocks(vec![
                librefang_types::message::ContentBlock::ToolResult {
                    tool_use_id: format!("tid_{i}"),
                    tool_name: "shell".to_string(),
                    content: format!("output {i}"),
                    is_error: false,
                    status: ToolExecutionStatus::Completed,
                    approval_request_id: None,
                },
            ]),
            pinned: false,
            timestamp: None,
        });
    }
    let pre_generation = session.messages_generation;
    let working = session.messages.clone();

    // Aux driver returns plain prose — fold falls back to bulk
    // summary across every block.  The persistence wiring is what
    // we are testing, NOT the JSON path; using the prose driver
    // makes the assertion shape independent of JSON formatting.
    let aux_driver: Arc<dyn LlmDriver> = Arc::new(FoldSummaryDriver);
    let aux_client = crate::aux_client::AuxClient::with_primary_only(Arc::clone(&aux_driver));

    let folded = maybe_fold_stale_tool_results(
        working,
        &mut session,
        FoldConfig {
            fold_after_turns: 2,
            min_batch_size: 1,
        },
        "test-model",
        Some(&aux_client),
        aux_driver,
        false,
        librefang_types::model_catalog::ReasoningEchoPolicy::None,
    )
    .await;

    // (1) The working clone must carry the stubs (sanity).
    let working_stubs = folded
        .iter()
        .filter(|m| match &m.content {
            librefang_types::message::MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
                matches!(
                    b,
                    librefang_types::message::ContentBlock::ToolResult { content, .. }
                        if content.starts_with("[history-fold]")
                )
            }),
            _ => false,
        })
        .count();
    assert!(working_stubs >= 8, "expected working copy to be folded");

    // (2) `session.messages` must ALSO carry the stubs — without
    // this, every subsequent turn refolds from scratch (the bug).
    let durable_stubs = session
        .messages
        .iter()
        .filter(|m| match &m.content {
            librefang_types::message::MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
                matches!(
                    b,
                    librefang_types::message::ContentBlock::ToolResult { content, .. }
                        if content.starts_with("[history-fold]")
                )
            }),
            _ => false,
        })
        .count();
    assert!(
        durable_stubs >= 8,
        "fold must replay rewrites onto session.messages — without this the \
         durable record stays raw and every subsequent turn refolds from scratch \
         (issue #4866 axis 2). durable_stubs={durable_stubs}"
    );

    // (3) `messages_generation` must have advanced — without this
    // `save_session_async` would NOT detect the mutation and the
    // rewrite would be lost across save / reload.
    assert!(
        session.messages_generation > pre_generation,
        "mark_messages_mutated must fire when fold rewrites are replayed; \
         pre={pre_generation} post={post}",
        post = session.messages_generation,
    );

    // (4) Every original `tool_use_id` must still be present in
    // `session.messages` — pairing invariant.
    let durable_ids: std::collections::BTreeSet<String> = session
        .messages
        .iter()
        .flat_map(|m| match &m.content {
            librefang_types::message::MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    librefang_types::message::ContentBlock::ToolResult { tool_use_id, .. } => {
                        Some(tool_use_id.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    for i in 0..10 {
        let expected = format!("tid_{i}");
        assert!(
            durable_ids.contains(&expected),
            "fold must preserve every original tool_use_id in session.messages, \
             missing {expected}"
        );
    }

    // (5) Second-call no-op: now that session.messages carries fold
    // stubs, calling the wrapper again on a fresh working clone must
    // NOT rewrite session.messages a second time — the
    // `is_already_folded` short-circuit fires inside
    // `collect_stale_indices`, no aux-LLM call, no new rewrites, and
    // `messages_generation` MUST stay where it is.  Without this
    // invariant the persistence fix only saves one round-trip per
    // session lifetime instead of all subsequent ones.
    let gen_after_first = session.messages_generation;
    let working_after_first = session.messages.clone();
    let aux_driver_2: Arc<dyn LlmDriver> = Arc::new(FoldSummaryDriver);
    let aux_client_2 = crate::aux_client::AuxClient::with_primary_only(Arc::clone(&aux_driver_2));
    let _ = maybe_fold_stale_tool_results(
        working_after_first,
        &mut session,
        FoldConfig {
            fold_after_turns: 2,
            min_batch_size: 1,
        },
        "test-model",
        Some(&aux_client_2),
        aux_driver_2,
        false,
        librefang_types::model_catalog::ReasoningEchoPolicy::None,
    )
    .await;
    assert_eq!(
        session.messages_generation,
        gen_after_first,
        "second fold pass on already-folded session must NOT advance \
         messages_generation; pre={gen_after_first} post={post}",
        post = session.messages_generation,
    );
}

#[tokio::test]
async fn test_streaming_max_continuations_return_preserves_reply_directives() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);
    let (tx, _rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Tell me more",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        tx,
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
        None, // checkpoint_manager
        None, // process_registry
        None,
        None,
        None,
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    assert_eq!(
        result.response,
        "[Partial response — token limit reached with no text output.]"
    );
    // Pure-text max_tokens overflow short-circuits on iter 1 (#2310).
    assert_eq!(result.iterations, 1);
    assert!(result.directives.reply_to.is_none());
    assert!(!result.directives.current_thread);
    assert!(!result.directives.silent);
}

#[tokio::test]
async fn test_streaming_max_tokens_owner_notice_and_actual_provider_survive_result() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(NotifyOwnerThenMaxTokensDriver::new(false));
    let tool = notify_owner_tool_definition();
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Notify owner before hitting max tokens",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&tool),
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete with max-tokens partial");

    assert_eq!(result.response, "Partial after owner notice");
    assert_eq!(
        result.owner_notice.as_deref(),
        Some("[NOTIFY] handoff_needed: Fallback provider needs owner visibility.")
    );
    assert_eq!(result.actual_provider.as_deref(), Some("fallback-b"));
    // #6134: the model the call actually ran threads through to AgentLoopResult.
    assert_eq!(result.actual_model.as_deref(), Some("actual-model-x"));
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    assert!(events.iter().any(
        |event| matches!(event, StreamEvent::OwnerNotice { text } if text.contains("Fallback provider"))
    ));
}

async fn run_streaming_max_tokens_owner_notice_case(
    opts: LoopOptions,
    final_tool_calls: bool,
) -> (
    librefang_memory::MemorySubstrate,
    librefang_memory::session::Session,
    AgentLoopResult,
) {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> =
        Arc::new(NotifyOwnerThenMaxTokensDriver::new(final_tool_calls));
    let tool = notify_owner_tool_definition();
    let (tx, _rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Notify owner before hitting max tokens",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&tool),
        tx,
        &opts,
    )
    .await
    .expect("Streaming loop should complete with max-tokens partial");

    (memory, session, result)
}

#[tokio::test]
async fn test_streaming_max_tokens_session_save_respects_ephemeral_options() {
    for (label, opts, should_persist) in [
        ("default", LoopOptions::default(), true),
        (
            "incognito",
            LoopOptions {
                incognito: true,
                ..LoopOptions::default()
            },
            false,
        ),
        (
            "fork",
            LoopOptions {
                is_fork: true,
                ..LoopOptions::default()
            },
            false,
        ),
    ] {
        let (memory, session, result) =
            run_streaming_max_tokens_owner_notice_case(opts, false).await;
        assert_eq!(result.response, "Partial after owner notice", "{label}");
        let persisted = memory
            .get_session(session.id)
            .expect("get_session must not error");
        assert_saved_max_tokens_session(persisted, should_persist, false, label);
    }
}

#[tokio::test]
async fn test_streaming_max_tokens_session_save_respects_ephemeral_options_on_continuation() {
    for (label, opts, should_persist) in [
        ("default", LoopOptions::default(), true),
        (
            "incognito",
            LoopOptions {
                incognito: true,
                ..LoopOptions::default()
            },
            false,
        ),
        (
            "fork",
            LoopOptions {
                is_fork: true,
                ..LoopOptions::default()
            },
            false,
        ),
    ] {
        let (memory, session, result) =
            run_streaming_max_tokens_owner_notice_case(opts, true).await;
        assert_eq!(result.response, "Partial after owner notice", "{label}");
        assert_eq!(result.iterations, MAX_CONTINUATIONS + 1, "{label}");
        let persisted = memory
            .get_session(session.id)
            .expect("get_session must not error");
        assert_saved_max_tokens_session(persisted, should_persist, true, label);
    }
}

/// Cascade-leak fixture: a fresh in-memory `MemorySubstrate` and a
/// `Session` ready to drive a one-shot agent-loop turn. Both new
/// integration tests below share this setup; only the loop entry
/// point (`run_agent_loop` vs `run_agent_loop_streaming`) differs.
fn cascade_leak_fixture() -> (
    librefang_memory::MemorySubstrate,
    librefang_memory::session::Session,
    AgentManifest,
    Arc<dyn LlmDriver>,
) {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    // Re-use DirectiveDriver: two structural markers (envelope + turn
    // frame) reproduce the real-incident leak shape exactly.
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "[Group message from Alice]\nUser asked: hi\nI responded: Buongiorno!",
        stop_reason: StopReason::EndTurn,
    });
    (memory, session, test_manifest(), driver)
}

#[tokio::test]
async fn cascade_leak_guard_drops_endturn_in_non_streaming_path() {
    let (memory, mut session, manifest, driver) = cascade_leak_fixture();
    let result = run_agent_loop(
        &manifest,
        "\u{1F934}", // emoji-only inbound, mirroring the real incident
        &mut session,
        &memory,
        driver,
        &[],
        None,
        None,
        None, // skill_registry
        None, // mcp_connections
        None, // web_ctx
        None, // browser_ctx
        None, // embedding_driver
        None, // workspace_root
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
        None, // pending_messages
        &LoopOptions::default(),
    )
    .await
    .expect("Loop should complete without error");
    assert!(result.silent, "got response: {:?}", result.response);
    assert!(result.response.is_empty(), "got: {:?}", result.response);
}

#[tokio::test]
async fn cascade_leak_guard_drops_endturn_in_streaming_path() {
    let (memory, mut session, manifest, driver) = cascade_leak_fixture();
    let (tx, _rx) = mpsc::channel(64);
    let result = run_agent_loop_streaming(
        &manifest,
        "\u{1F934}",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        tx,
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
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");
    assert!(result.silent, "got response: {:?}", result.response);
    assert!(result.response.is_empty(), "got: {:?}", result.response);
}

/// M-2: Regression lock for the streaming short-circuit in
/// `run_agent_loop_streaming`. When the incremental cascade-leak guard fires
/// mid-stream, the caller must treat the entire turn as a silent drop even
/// if the driver's final `ContentComplete` carries `stop_reason = ToolUse`.
///
/// This test drives `run_agent_loop_streaming` end-to-end (not just the
/// forwarding task) and asserts:
/// - `result.silent == true` (the turn was silently dropped)
/// - `result.response.is_empty()` (no text reached the caller)
/// - No tool was invoked (the ToolUse stop_reason must not trigger
///   tool execution when cascade_leak_aborted is set).
#[tokio::test]
async fn cascade_leak_guard_aborts_tool_use_stop_reason_in_streaming_path() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    // A driver that emits two structural markers (triggering the cascade-leak
    // guard) and then signals ToolUse as the stop reason. Without the
    // cascade_leak_aborted short-circuit in run_agent_loop_streaming the loop
    // would proceed to tool execution — which this test must prevent.
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "User asked: hi\nI responded: Buongiorno!",
        stop_reason: StopReason::ToolUse,
    });
    let manifest = test_manifest();
    let (tx, _rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "\u{1F934}",
        &mut session,
        &memory,
        driver,
        &[], // no tools registered — ensures any tool execution would panic/err
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    assert!(
        result.silent,
        "cascade-leak + ToolUse stop_reason must yield a silent result; got: {:?}",
        result.response
    );
    assert!(
        result.response.is_empty(),
        "no text must reach the caller when cascade leak fires; got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn cascade_leak_guard_suppresses_timeout_partial_text_delta_in_streaming_path() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(CascadeLeakTimedOutDriver);
    let (tx, mut rx) = mpsc::channel(64);

    let err = run_streaming_for_test(
        &manifest,
        "hi",
        &mut session,
        &memory,
        driver,
        &[],
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect_err("timeout should propagate");

    let events = {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    };

    assert!(
        err.to_string().contains("Task timed out after 30s"),
        "unexpected error: {err}"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, StreamEvent::TextDelta { .. })),
        "cascade leak timeout partial text must not emit TextDelta events: {events:?}"
    );
}

#[tokio::test]
async fn test_streaming_max_continuations_with_directives_preserves_reply_directives() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "[[reply:msg_999]] [[@current]] Partial answer",
        stop_reason: StopReason::MaxTokens,
    });
    let (tx, _rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Tell me more",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        tx,
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
        None, // checkpoint_manager
        None, // process_registry
        None,
        None,
        None,
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    assert_eq!(result.response, "Partial answer");
    // Pure-text max_tokens overflow short-circuits on iter 1 (#2310).
    assert_eq!(result.iterations, 1);
    assert_eq!(result.directives.reply_to.as_deref(), Some("msg_999"));
    assert!(result.directives.current_thread);
    assert!(!result.directives.silent);
}

#[tokio::test]
async fn test_streaming_empty_response_after_tool_use_returns_fallback() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());
    let (tx, _rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Do something with tools",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        tx,
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
    .expect("Streaming loop should complete without error");

    assert!(
        !result.response.trim().is_empty(),
        "Streaming response should not be empty after tool use, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("Permission denied") || result.response.contains("Task completed"),
        "Expected tool error or fallback message in streaming, got: {:?}",
        result.response
    );
}

/// Mock driver that returns empty text on first call (EndTurn), then normal text on second.
/// This tests the one-shot retry logic for iteration 0 empty responses.
struct EmptyThenNormalDriver {
    call_count: AtomicU32,
}

impl EmptyThenNormalDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for EmptyThenNormalDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            // First call: empty EndTurn (triggers retry)
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        } else {
            // Second call (retry): normal response
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Recovered after retry!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 15,
                    output_tokens: 8,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

/// Mock driver that always returns empty EndTurn (no recovery on retry).
/// Tests that the fallback message appears when retry also fails.
struct AlwaysEmptyDriver;

#[async_trait]
impl LlmDriver for AlwaysEmptyDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 0,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

#[tokio::test]
async fn test_empty_first_response_retries_and_recovers() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyThenNormalDriver::new());

    let result = run_agent_loop(
        &manifest,
        "Hello",
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
        None,
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
    .expect("Loop should recover via retry");

    assert_eq!(result.response, "Recovered after retry!");
    assert_eq!(
        result.iterations, 2,
        "Should have taken 2 iterations (retry)"
    );
}

#[tokio::test]
async fn test_empty_first_response_fallback_when_retry_also_empty() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(AlwaysEmptyDriver);

    let result = run_agent_loop(
        &manifest,
        "Hello",
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
        None,
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
    .expect("Loop should complete with fallback");

    // No tools were executed, so should get the empty response message
    assert!(
        result.response.contains("empty response"),
        "Expected empty response fallback (no tools executed), got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_max_history_messages_constant() {
    assert_eq!(DEFAULT_MAX_HISTORY_MESSAGES, 60);
}

#[tokio::test]
async fn test_streaming_empty_response_max_tokens_returns_fallback() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);
    let (tx, _rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Tell me something long",
        &mut session,
        &memory,
        driver,
        &[],
        None,
        tx,
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
    .expect("Streaming loop should complete without error");

    assert!(
        !result.response.trim().is_empty(),
        "Streaming response should not be empty on max tokens, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("token limit"),
        "Expected max-tokens fallback in streaming, got: {:?}",
        result.response
    );
}

#[test]
fn test_recover_text_tool_calls_basic() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[0].input["query"], "rust async");
    assert!(calls[0].id.starts_with("recovered_"));
}

#[test]
fn test_recover_text_tool_calls_unknown_tool() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function=hack_system>{"cmd":"rm -rf /"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Unknown tools should be rejected");
}

#[test]
fn test_recover_text_tool_calls_invalid_json() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = r#"<function=web_search>not valid json</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty(), "Invalid JSON should be skipped");
}

#[test]
fn test_recover_text_tool_calls_multiple() {
    let tools = vec![
        ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    let text = r#"<function=web_search>{"query":"hello"}</function> then <function=read_file>{"path":"a.txt"}</function>"#;
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "web_search");
    assert_eq!(calls[1].name, "read_file");
}

// #8235: models served behind OpenAI-compatible proxies (Hermes/Llama-3
// tool-call format) emit the parameters as <parameter=NAME>value</parameter>
// pairs instead of a JSON object. Verbatim shape from a production host:
// the command value spans multiple lines and one parameter is numeric.
#[test]
fn test_recover_parameter_style_tool_call() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Run a shell command".into(),
        // #8236: the schema is what tells `coerce_scalar` `timeout_seconds`
        // is really numeric — an empty schema now means "stay a string" (see
        // test_recover_parameter_style_respects_string_schema_type below for
        // the case that motivated the change).
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout_seconds": {"type": "integer"}
            }
        }),
    }];
    let text = concat!(
        "<function=shell_exec>\n",
        "<parameter=command>\n",
        "export PATH=\"/home/u/.nvm/versions/node/v22/bin:$PATH\"\n",
        "mercadona product 3024 --json\n",
        "</parameter>\n",
        "<parameter=timeout_seconds>\n",
        "180\n",
        "</parameter>\n",
        "</function>"
    );
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1, "a parameter-style call must be recovered");
    assert_eq!(calls[0].name, "shell_exec");
    assert_eq!(
        calls[0].input["command"],
        concat!(
            "export PATH=\"/home/u/.nvm/versions/node/v22/bin:$PATH\"\n",
            "mercadona product 3024 --json"
        ),
        "a multi-line parameter value must survive intact"
    );
    assert_eq!(
        calls[0].input["timeout_seconds"],
        serde_json::json!(180),
        "a numeric parameter must arrive as a number, not a string"
    );
}

#[test]
fn test_recover_parameter_style_variant2() {
    // Variant 2 shape (<function>NAME…</function>) with a parameter body and
    // no JSON object. Before #8235 the search for '{' failed and the call
    // was dropped.
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Run a shell command".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<function>shell_exec<parameter=command>ls -la</parameter></function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input["command"], "ls -la");
}

#[test]
fn test_parameter_style_unknown_tool_is_still_rejected() {
    let tools = vec![ToolDefinition {
        name: "shell_exec".into(),
        description: "Run a shell command".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "<function=hack_system><parameter=command>rm -rf /</parameter></function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(
        calls.is_empty(),
        "the parameter fallback must not bypass the name check"
    );
}

#[test]
fn test_pure_tool_call_markup_is_replaced_with_honest_reply() {
    // A model that answered with nothing but an unparseable tool call would
    // otherwise deliver the raw syntax to the channel (#8235).
    let text = "<function=shell_exec>\n<parameter=command>ls</parameter>\n<parameter=timeout_seconds>5</parameter>\n</function>";
    let out = replace_unrecoverable_tool_call_reply(text);
    assert!(
        !out.contains("<function=") && !out.contains("<parameter="),
        "raw tool-call syntax must never reach the user, got: {out:?}"
    );
    assert!(
        !out.trim().is_empty(),
        "the replacement must say something honest"
    );
}

#[test]
fn test_markup_mixed_with_prose_is_delivered_unchanged() {
    let text =
        "I could not run that. <function=shell_exec><parameter=command>ls</parameter></function>";
    let out = replace_unrecoverable_tool_call_reply(text);
    assert_eq!(out, text, "a reply that starts with prose passes through");
}

#[test]
fn test_normal_text_untouched_by_reply_guard() {
    let text = "Here are the prices I found: piña 1,20 €.";
    assert_eq!(replace_unrecoverable_tool_call_reply(text), text);
}

#[test]
fn test_recover_text_tool_calls_no_pattern() {
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = "Just a normal response with no tool calls.";
    let calls = recover_text_tool_calls(text, &tools);
    assert!(calls.is_empty());
}

#[test]
fn test_recover_text_tool_calls_empty_tools() {
    let text = r#"<function=web_search>{"query":"hello"}</function>"#;
    let calls = recover_text_tool_calls(text, &[]);
    assert!(calls.is_empty(), "No tools = no recovery");
}

// --- #8236 review fixes ------------------------------------------------

#[test]
fn test_unrecoverable_reply_guard_actually_substitutes_at_call_site() {
    // Regression for the #8236 review finding on text_recovery.rs:668: the
    // replacement branch used to return `Cow::Borrowed`, the same variant as
    // the pass-through branch, so the `match` mirrored below (the one both
    // mod.rs and run_streaming.rs actually run) always kept the caller's
    // original `text` local and the raw markup reached the channel — even
    // though this exact case already passed the pre-existing test above,
    // because that test only inspects the *returned value*, never the
    // discriminated variant.
    let text = "<function=shell_exec>\n<parameter=command>ls</parameter>\n</function>";
    let delivered = match replace_unrecoverable_tool_call_reply(text) {
        std::borrow::Cow::Borrowed(_) => text.to_string(),
        std::borrow::Cow::Owned(replacement) => replacement,
    };
    assert!(
        !delivered.contains("<function="),
        "the call-site match must receive the substituted text, got: {delivered:?}"
    );
}

#[test]
fn test_strip_tool_call_spans_preserves_trailing_prose_after_last_span() {
    // Regression for text_recovery.rs:700: once the last recognized span is
    // consumed, `strip_tool_call_spans` used to drop everything after it
    // instead of keeping it as prose — so a reply that opens with a tool
    // call and closes with a real answer got judged "100% unparseable" and
    // had the user's actual answer replaced by the apology sentence.
    let text = "<function=shell_exec><parameter=command>ls</parameter></function>\nLos precios son: piña 1,20 €.";
    let out = replace_unrecoverable_tool_call_reply(text);
    assert_eq!(
        out, text,
        "trailing prose after the last markup span must survive, not be swallowed"
    );
}

#[test]
fn test_tool_call_tag_uses_correct_closer() {
    // Regression for text_recovery.rs:683: `strip_tool_call_spans` paired
    // `<tool_call>` with a markdown fence ("```") instead of `</tool_call>`,
    // so the real closer was never found. Two well-formed <tool_call> spans
    // around genuine middle prose isolate the bug from the trailing-prose
    // fix above: with the wrong closer, the very first span already reads
    // as "unterminated" and drops everything — including the second span
    // and the prose between them — before the loop ever gets a chance to
    // treat the end of the text as an "opener not found" case.
    let text = concat!(
        "<tool_call>{\"name\":\"a\",\"arguments\":{}}</tool_call>",
        "middle prose",
        "<tool_call>{\"name\":\"b\",\"arguments\":{}}</tool_call>",
    );
    let out = replace_unrecoverable_tool_call_reply(text);
    assert_eq!(
        out, text,
        "the middle prose between two properly closed <tool_call> spans must survive"
    );
}

#[test]
fn test_recover_parameter_style_respects_string_schema_type() {
    // Regression for text_recovery.rs:755: `coerce_scalar` used to guess the
    // JSON type from the value alone, so a `chat_id` that merely looks
    // numeric was coerced to an integer — which `serde` then rejects on the
    // tool's actual `String` field ("invalid type: integer, expected a
    // string"), and which silently loses leading zeros or numeric precision
    // for other string-typed IDs. The schema is now consulted first.
    let tools = vec![ToolDefinition {
        name: "telegram_send".into(),
        description: "Send a Telegram message".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "chat_id": {"type": "string"},
                "text": {"type": "string"}
            }
        }),
    }];
    let text = "<function=telegram_send><parameter=chat_id>-1001234567890</parameter><parameter=text>hi</parameter></function>";
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].input["chat_id"],
        serde_json::json!("-1001234567890"),
        "a string-typed schema property must not be coerced to a number"
    );
}

#[test]
fn test_parameter_value_keeps_deliberate_blank_lines_and_indentation() {
    // Regression for text_recovery.rs:735: the parameter value was
    // `.trim()`ed, which the doc-comment above `parse_parameter_style_body`
    // already called "verbatim" — but `.trim()` strips ALL surrounding
    // whitespace, not just the single newline the markup convention puts
    // right after `>` and right before `</parameter>`. A content-bearing
    // parameter (e.g. `content` for a file-writing tool) loses a deliberate
    // trailing blank line or leading indentation on its first line.
    let tools = vec![ToolDefinition {
        name: "write_file".into(),
        description: "Write a file".into(),
        input_schema: serde_json::json!({}),
    }];
    let text = concat!(
        "<function=write_file>\n",
        "<parameter=content>\n",
        "  indented first line\n",
        "\n",
        "</parameter>\n",
        "</function>"
    );
    let calls = recover_text_tool_calls(text, &tools);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].input["content"], "  indented first line\n",
        "leading indentation and the deliberate trailing blank line must survive — \
         only the single markup newline on each end is stripped"
    );
}

#[tokio::test]
async fn test_max_tokens_pure_markup_overflow_replaced_with_honest_reply() {
    // Regression for mod.rs:1450's second gap: a token cap can cut a
    // tool-call markup span off mid-call, leaving it permanently
    // unterminated — the honest-reply guard used to run only on the EndTurn
    // path, so this pure-text MaxTokens overflow delivered the raw
    // `<function=...>` syntax to the channel verbatim.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "<function=shell_exec><parameter=command>ls -la /very/long/path/that/got/cut/off",
        stop_reason: StopReason::MaxTokens,
    });

    let result = run_agent_loop(
        &manifest,
        "Run something",
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
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // checkpoint_manager
        None, // process_registry
        None,
        None,
        None,
        None,
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("Loop should complete without error");

    assert!(
        !result.response.contains("<function="),
        "raw tool-call markup must never reach the channel, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_streaming_pure_markup_reply_never_appears_on_the_wire() {
    // Regression for the run_streaming.rs:1011 review finding: the honest-
    // reply guard rewrites the *final* text used for history/delivery, but
    // by the time it runs, `stream_with_retry`'s forwarding task has already
    // relayed every `TextDelta` live — a markup-only reply showed the raw
    // `<function=...>` syntax to any client rendering deltas as they
    // arrived, and the guard's substitution never reached the wire at all.
    // `stream_with_retry` now withholds a delta while it might still resolve
    // to pure markup, and `run_streaming.rs` sends the corrected text as a
    // single catch-up delta once the stream ends without ever resolving.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "<function=shell_exec><parameter=command>ls</parameter></function>",
        stop_reason: StopReason::EndTurn,
    });
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Run something",
        &mut session,
        &memory,
        driver,
        &[],
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    let mut delta_texts = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::TextDelta { text } = event {
            delta_texts.push(text);
        }
    }
    let all_deltas = delta_texts.join("");
    assert!(
        !all_deltas.contains("<function="),
        "raw tool-call markup must never reach the wire, got deltas: {delta_texts:?}"
    );
    assert!(
        !result.response.contains("<function="),
        "the final response must also be the honest reply, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_streaming_markup_with_trailing_prose_reaches_the_wire_unchanged() {
    // The counterpart to the test above: once real content follows the
    // markup, the withholding buffer must flush it — this is not a case the
    // honest-reply guard replaces (see test_markup_mixed_with_prose_is_delivered_unchanged),
    // so it must still be visible to a streaming client, not silently
    // swallowed by the new buffer.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let full_text =
        "<function=shell_exec><parameter=command>ls</parameter></function>\nHere is your answer.";
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: full_text,
        stop_reason: StopReason::EndTurn,
    });
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Run something",
        &mut session,
        &memory,
        driver,
        &[],
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    let mut delta_texts = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::TextDelta { text } = event {
            delta_texts.push(text);
        }
    }
    let all_deltas = delta_texts.join("");
    assert_eq!(
        all_deltas, full_text,
        "markup resolved by real trailing content must still reach the wire, unchanged"
    );
    assert_eq!(result.response, full_text);
}

/// A streaming driver that emits the given chunks as separate `TextDelta`s
/// before reporting `stop_reason` — the shape a real provider streams in,
/// and the only way to exercise a withholding decision that is made *after*
/// earlier deltas already went out.
struct ChunkedDeltasDriver {
    chunks: &'static [&'static str],
    stop_reason: StopReason,
}

#[async_trait]
impl LlmDriver for ChunkedDeltasDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.chunks.concat(),
                provider_metadata: None,
            }],
            stop_reason: self.stop_reason,
            tool_calls: vec![],
            usage: TokenUsage::default(),
            actual_provider: None,
            actual_model: None,
        })
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        for chunk in self.chunks {
            tx.send(StreamEvent::TextDelta {
                text: (*chunk).to_string(),
            })
            .await
            .map_err(|_| LlmError::Http("stream receiver dropped".to_string()))?;
        }
        let response = CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.chunks.concat(),
                provider_metadata: None,
            }],
            stop_reason: self.stop_reason,
            tool_calls: vec![],
            usage: TokenUsage::default(),
            actual_provider: None,
            actual_model: None,
        };
        tx.send(StreamEvent::ContentComplete {
            stop_reason: response.stop_reason,
            usage: response.usage,
        })
        .await
        .map_err(|_| LlmError::Http("stream receiver dropped".to_string()))?;
        Ok(response)
    }
}

#[tokio::test]
async fn test_streaming_prose_then_markup_is_not_delivered_twice() {
    // Regression (#8236 review G1): the withholding buffer was re-evaluated on
    // every delta instead of being anchored to the first one, so a reply whose
    // *later* delta happened to start with a known opener opened a buffer even
    // though earlier prose had already gone out on the wire. The stream then
    // ended with `withheld_markup = Some(..)`, and the catch-up send in
    // `run_streaming.rs` re-sent the *whole* text — delivering the opening
    // prose to the client a second time. The honest-reply guard cannot rescue
    // it either: the full text starts with prose, so it is not a pure-markup
    // reply and is returned borrowed, unchanged.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let chunks: &[&str] = &[
        "Hello there.\n",
        "<function=nonexistent_tool><parameter=a>1</parameter></function>",
    ];
    let full_text = chunks.concat();
    let driver: Arc<dyn LlmDriver> = Arc::new(ChunkedDeltasDriver {
        chunks,
        stop_reason: StopReason::EndTurn,
    });
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Say hello",
        &mut session,
        &memory,
        driver,
        &[],
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    let mut delta_texts = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::TextDelta { text } = event {
            delta_texts.push(text);
        }
    }
    let all_deltas = delta_texts.join("");
    assert_eq!(
        all_deltas, full_text,
        "every byte of the reply must reach the wire exactly once, got deltas: {delta_texts:?}"
    );
    assert_eq!(
        result.response, full_text,
        "the final response is unchanged — a reply that opens with real prose is not \
         a pure-markup reply the honest-reply guard replaces"
    );
}

#[tokio::test]
async fn test_streaming_max_tokens_pure_markup_overflow_replaced_with_honest_reply() {
    // Regression (#8236 review M3): the token-cap counterpart of
    // `test_max_tokens_pure_markup_overflow_replaced_with_honest_reply`. The
    // honest-reply guard was added to the non-streaming `MaxTokens` arm only,
    // so a truncated `<function=...>` still reached the channel verbatim on
    // the streaming path — the one the dashboard and every channel bridge
    // take. The withheld first delta also has to be released here: without a
    // catch-up send this turn delivers no delta at all.
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(DirectiveDriver {
        text: "<function=shell_exec><parameter=command>ls -la /very/long/path/that/got/cut/off",
        stop_reason: StopReason::MaxTokens,
    });
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_streaming_for_test(
        &manifest,
        "Run something",
        &mut session,
        &memory,
        driver,
        &[],
        tx,
        &LoopOptions::default(),
    )
    .await
    .expect("Streaming loop should complete without error");

    let mut delta_texts = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::TextDelta { text } = event {
            delta_texts.push(text);
        }
    }
    let all_deltas = delta_texts.join("");
    assert!(
        !result.response.contains("<function="),
        "raw tool-call markup must never reach the channel, got: {:?}",
        result.response
    );
    assert!(
        !all_deltas.contains("<function="),
        "raw tool-call markup must never reach the wire, got deltas: {delta_texts:?}"
    );
    assert_eq!(
        all_deltas, result.response,
        "the corrected text must be delivered as the one delta of this turn, not swallowed"
    );
}

// --- Parallel tool-dispatch integration (#3129 PR-4) -------------------
//
// These exercise the real `run_agent_loop` ToolUse branch end to end with
// `file_read` over a tempdir, asserting that (1) the flag-off path is the
// unchanged serial dispatch, (2) the flag-on path runs every member of a
// safe group and (3) results land in original tool-call index order — the
// hard provider contract that `tool_result` blocks line up positionally
// with `tool_use` blocks.

/// Driver that emits a fixed batch of `file_read` calls on the first turn,
/// then `EndTurn` on the second.
struct BatchFileReadDriver {
    call_count: AtomicU32,
    calls: Vec<(String, String)>, // (tool_use_id, relative path)
}

impl BatchFileReadDriver {
    fn new(calls: Vec<(String, String)>) -> Self {
        Self {
            call_count: AtomicU32::new(0),
            calls,
        }
    }
}

#[async_trait]
impl LlmDriver for BatchFileReadDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            let content: Vec<ContentBlock> = self
                .calls
                .iter()
                .map(|(id, path)| ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "file_read".to_string(),
                    input: serde_json::json!({ "path": path }),
                    provider_metadata: None,
                })
                .collect();
            let tool_calls: Vec<ToolCall> = self
                .calls
                .iter()
                .map(|(id, path)| ToolCall {
                    id: id.clone(),
                    name: "file_read".to_string(),
                    input: serde_json::json!({ "path": path }),
                })
                .collect();
            Ok(CompletionResponse {
                content,
                stop_reason: StopReason::ToolUse,
                tool_calls,
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 3,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        } else {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "All reads done.".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

/// Pull the committed `(tool_use_id, content)` tool-result pairs out of the
/// session in wire order. The single user message that pairs with the
/// assistant tool_use turn carries every result block in append order, so
/// this directly reflects the order the provider will see.
fn committed_tool_results(session: &librefang_memory::session::Session) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in &session.messages {
        if let librefang_types::message::MessageContent::Blocks(blocks) = &m.content {
            for b in blocks {
                if let librefang_types::message::ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = b
                {
                    out.push((tool_use_id.clone(), content.clone()));
                }
            }
        }
    }
    out
}

async fn run_batch_read_loop(
    calls: Vec<(String, String)>,
    workspace_root: &std::path::Path,
    parallel: Option<librefang_types::config::ParallelToolsConfig>,
) -> librefang_memory::session::Session {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(BatchFileReadDriver::new(calls));
    let file_read_def = fake_tool("file_read");

    let loop_opts = LoopOptions {
        parallel_tools_config: parallel,
        ..LoopOptions::default()
    };

    run_agent_loop(
        &manifest,
        "read files",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&file_read_def),
        None,                 // kernel
        None,                 // skill_registry
        None,                 // mcp_connections
        None,                 // web_ctx
        None,                 // browser_ctx
        None,                 // embedding_driver
        Some(workspace_root), // workspace_root — enables file tools
        None,                 // on_phase
        None,                 // media_engine
        None,                 // media_drivers
        None,                 // tts_engine
        None,                 // docker_config
        None,                 // hooks
        None,                 // context_window_tokens
        None,                 // process_manager
        None,                 // checkpoint_manager
        None,                 // process_registry
        None,                 // user_content_blocks
        None,                 // proactive_memory
        None,                 // context_engine
        None,                 // pending_messages
        &loop_opts,
    )
    .await
    .expect("loop should complete without error");

    session
}

fn enabled_parallel_cfg(max_concurrent: u32) -> librefang_types::config::ParallelToolsConfig {
    librefang_types::config::ParallelToolsConfig {
        enabled: true,
        max_concurrent,
        ..librefang_types::config::ParallelToolsConfig::default()
    }
}

/// Flag-on: four independent read-only `file_read` calls all execute and
/// their results are committed in original tool-call index order.
#[tokio::test]
async fn parallel_dispatch_runs_all_reads_in_index_order() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut calls = Vec::new();
    for i in 0..4 {
        let name = format!("f{i}.txt");
        std::fs::write(dir.path().join(&name), format!("content-{i}")).unwrap();
        calls.push((format!("tid_{i}"), name));
    }

    let session =
        run_batch_read_loop(calls.clone(), dir.path(), Some(enabled_parallel_cfg(4))).await;

    let results = committed_tool_results(&session);
    assert_eq!(results.len(), 4, "every read must produce a result");
    // Index order: result[i] pairs with tool_use tid_i and carries content-i.
    for (i, (id, content)) in results.iter().enumerate() {
        assert_eq!(id, &format!("tid_{i}"), "result {i} out of index order");
        assert!(
            content.contains(&format!("content-{i}")),
            "result {i} content mismatch: {content:?}"
        );
    }
}

/// Flag-off (default): identical batch on the serial path must produce the
/// same results in the same index order — zero behaviour change.
#[tokio::test]
async fn serial_dispatch_unchanged_when_flag_off() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut calls = Vec::new();
    for i in 0..4 {
        let name = format!("f{i}.txt");
        std::fs::write(dir.path().join(&name), format!("content-{i}")).unwrap();
        calls.push((format!("tid_{i}"), name));
    }

    // `None` (flag absent) and explicit `enabled = false` must both stay serial.
    for cfg in [
        None,
        Some(librefang_types::config::ParallelToolsConfig::default()),
    ] {
        let session = run_batch_read_loop(calls.clone(), dir.path(), cfg).await;
        let results = committed_tool_results(&session);
        assert_eq!(results.len(), 4);
        for (i, (id, content)) in results.iter().enumerate() {
            assert_eq!(id, &format!("tid_{i}"));
            assert!(content.contains(&format!("content-{i}")));
        }
    }
}

/// Flag-on with a read + write + read mix: the planner keeps disjoint
/// reads/writes in one group, so all three run and results stay in index
/// order. Drives `file_read` only (writes need a writable sandbox) but
/// asserts the *plan* groups the mix as expected via `plan_batch`, then
/// confirms the loop preserves order for the runnable reads.
#[tokio::test]
async fn parallel_dispatch_write_read_mix_groups_and_orders() {
    use crate::parallel_dispatch::plan_batch;

    // Planner-level assertion: read(/a) + write(/b) + read(/c) on disjoint
    // paths collapse into one parallel group, preserving 0..3 order.
    let mix = vec![
        ToolCall {
            id: "r0".into(),
            name: "file_read".into(),
            input: serde_json::json!({"path": "/a"}),
        },
        ToolCall {
            id: "w1".into(),
            name: "file_write".into(),
            input: serde_json::json!({"path": "/b", "content": "x"}),
        },
        ToolCall {
            id: "r2".into(),
            name: "file_read".into(),
            input: serde_json::json!({"path": "/c"}),
        },
    ];
    let plan = plan_batch(&mix, &[]);
    assert_eq!(
        plan.groups,
        vec![vec![0, 1, 2]],
        "disjoint read/write/read must form one group"
    );

    // End-to-end ordering for the runnable subset (three reads, with the
    // middle one on a distinct file standing in for the disjoint write):
    // confirms the loop appends results 0..3 in order under the flag.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let mut calls = Vec::new();
    for (i, id) in ["a", "b", "c"].iter().enumerate() {
        let name = format!("{id}.txt");
        std::fs::write(dir.path().join(&name), format!("content-{i}")).unwrap();
        calls.push((format!("tid_{i}"), name));
    }
    let session = run_batch_read_loop(calls, dir.path(), Some(enabled_parallel_cfg(2))).await;
    let results = committed_tool_results(&session);
    assert_eq!(results.len(), 3);
    for (i, (id, content)) in results.iter().enumerate() {
        assert_eq!(id, &format!("tid_{i}"));
        assert!(content.contains(&format!("content-{i}")));
    }
}

// --- Outcome-aware loop guard, end to end ------------------------------
//
// The guard's outcome half only exists if the agent loop feeds results back into it after execution.
// These drive the real `run_agent_loop` over a tool whose answer is byte-identical every time and assert where the block lands: on the fourth identical call (the outcome rule) rather than the fifth (`block_threshold`).
// Both dispatch paths are covered, because the recording site differs between them — the serial path records per call, the parallel dispatcher once its concurrent phase has drained.

/// Driver that issues the same `tool_search` call — same name, same parameters — `calls_per_turn` times per turn for `turns` turns, then finishes with text.
/// A query that matches nothing in the pool makes `tool_search` return a byte-identical result every time, which is what the outcome hash keys on.
struct RepeatIdenticalCallDriver {
    call_count: AtomicU32,
    turns: u32,
    calls_per_turn: usize,
}

impl RepeatIdenticalCallDriver {
    fn new(turns: u32, calls_per_turn: usize) -> Self {
        Self {
            call_count: AtomicU32::new(0),
            turns,
            calls_per_turn,
        }
    }
}

#[async_trait]
impl LlmDriver for RepeatIdenticalCallDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let turn = self.call_count.fetch_add(1, Ordering::Relaxed);
        if turn >= self.turns {
            return Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Giving up on that approach.".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
                actual_provider: None,
                actual_model: None,
            });
        }
        let input = serde_json::json!({"query": "zzqq"});
        let ids: Vec<String> = (0..self.calls_per_turn)
            .map(|i| format!("tid_{turn}_{i}"))
            .collect();
        Ok(CompletionResponse {
            content: ids
                .iter()
                .map(|id| ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "tool_search".to_string(),
                    input: input.clone(),
                    provider_metadata: None,
                })
                .collect(),
            stop_reason: StopReason::ToolUse,
            tool_calls: ids
                .iter()
                .map(|id| ToolCall {
                    id: id.clone(),
                    name: "tool_search".to_string(),
                    input: input.clone(),
                })
                .collect(),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 3,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

/// The definition `run_repeat_call_loop` hands `run_agent_loop`.
///
/// `tool_search` is dispatched by name, but the outer capability allowlist is built from the tool slice, so the meta-tool still has to appear in it.
/// The `x-parallel-safety` annotation is what makes the parallel test meaningful: `classify_by_name` does not know `tool_search`, so without it the call classifies `Unknown` → `WriteShared`, and `plan_batch` gives every `WriteShared` call a bucket of its own.
/// The batch would then be three groups of one — `execute_tool_group` entered three times with nothing to order — and the phase-3 hazard the test exists for (results arriving in whichever order the futures finished) would never arise.
/// `explicit_parallel_safety_from_schema` reads the annotation ahead of the name heuristic, which puts all three calls in one group.
fn repeat_call_tool_def() -> ToolDefinition {
    ToolDefinition {
        name: "tool_search".to_string(),
        description: "fake tool_search".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "x-parallel-safety": "read_only"
        }),
    }
}

async fn run_repeat_call_loop(
    turns: u32,
    calls_per_turn: usize,
    parallel: Option<librefang_types::config::ParallelToolsConfig>,
) -> librefang_memory::session::Session {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = fresh_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> =
        Arc::new(RepeatIdenticalCallDriver::new(turns, calls_per_turn));
    let tool_search_def = repeat_call_tool_def();

    let loop_opts = LoopOptions {
        parallel_tools_config: parallel,
        ..LoopOptions::default()
    };

    run_agent_loop(
        &manifest,
        "keep searching",
        &mut session,
        &memory,
        driver,
        std::slice::from_ref(&tool_search_def),
        None, // kernel
        None, // skill_registry
        None, // mcp_connections
        None, // web_ctx
        None, // browser_ctx
        None, // embedding_driver
        None, // workspace_root
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
        &loop_opts,
    )
    .await
    .expect("loop should complete without error");

    session
}

/// Serial path: a tool answering identically to identical parameters is cut off on the fourth call.
/// Without the post-execution recording the outcome counters stay empty and the fourth call runs like the three before it, with the block arriving only on the fifth.
#[tokio::test]
async fn identical_results_block_the_fourth_call_on_the_serial_path() {
    let session = run_repeat_call_loop(5, 1, None).await;
    let results = committed_tool_results(&session);
    assert_eq!(results.len(), 5, "every tool_use must have a result");

    for (i, (_, content)) in results.iter().take(3).enumerate() {
        assert!(
            content.contains("No tools matched"),
            "call {i} should have run the tool, got: {content:?}"
        );
    }
    // The second identical result already carries the advisory — the guard says so before it acts on it.
    assert!(
        results[1].1.contains("[LOOP GUARD]") && results[1].1.contains("identical results"),
        "second identical result must carry the outcome advisory, got: {:?}",
        results[1].1
    );
    assert!(
        results[3].1.starts_with("Blocked:") && results[3].1.contains("identical results"),
        "fourth call must be blocked by the outcome rule, got: {:?}",
        results[3].1
    );
}

/// Parallel path: the dispatcher records outcomes after its concurrent phase, so one batch of three identical calls — dispatched together, in a single group — arms the block for the next batch.
/// Without that recording the next batch's first call runs and only its second, the fifth identical call overall, is blocked.
///
/// The group really has to be multi-member for this to test anything: recording per call in a group of one is the serial path with extra steps, and the ordering the phase-3 sort exists to fix cannot go wrong.
/// The planner assertion below pins that, so a future classification change that splits the batch fails here instead of quietly turning this into a duplicate of the serial test.
#[tokio::test]
async fn identical_results_block_the_next_batch_on_the_parallel_path() {
    use crate::parallel_dispatch::plan_batch_with_mcp;

    let cfg = enabled_parallel_cfg(3);
    let def = repeat_call_tool_def();
    let batch: Vec<ToolCall> = (0..3)
        .map(|i| ToolCall {
            id: format!("tid_0_{i}"),
            name: "tool_search".to_string(),
            input: serde_json::json!({"query": "zzqq"}),
        })
        .collect();
    assert_eq!(
        plan_batch_with_mcp(&batch, std::slice::from_ref(&def), Some(&cfg)).groups,
        vec![vec![0, 1, 2]],
        "the three calls must dispatch as one group, or the parallel path is untested"
    );

    let session = run_repeat_call_loop(2, 3, Some(cfg)).await;
    let results = committed_tool_results(&session);
    assert_eq!(results.len(), 6, "every tool_use must have a result");

    for (i, (_, content)) in results.iter().take(3).enumerate() {
        assert!(
            content.contains("No tools matched"),
            "call {i} in the first batch should have run the tool, got: {content:?}"
        );
    }
    for (i, (_, content)) in results.iter().skip(3).enumerate() {
        assert!(
            content.starts_with("Blocked:") && content.contains("identical results"),
            "call {} in the second batch must be blocked by the outcome rule, got: {content:?}",
            i + 3
        );
    }
}

/// Proxy measurement for the fat-frame half of #6659, deliberately labelled as such: it would NOT have caught the missing depth accounting on the workflow path, and it cannot fail the way a stack overflow does.
///
/// `run_agent_loop`'s future is the state machine a nested agent turn stacks once per level, and the #6659 crash report works out to roughly 40 KB per frame — which reads as bounded depth with very large frames rather than deep recursion.
/// So the size of this future is the number worth watching: if it grows by an order of magnitude, a legal-depth delegation chain starts aborting a 2 MiB tokio worker even with the depth cap in place.
///
/// An `async fn` does nothing until polled, so constructing the future without awaiting it is enough to measure it — no runtime needed.
/// The bound is generous on purpose: this pins the order of magnitude, not a byte count, so ordinary feature work does not have to re-baseline it.
#[test]
fn run_agent_loop_future_size_stays_within_its_order_of_magnitude() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let mut session = librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id,
        parent_session_id: None,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());
    let options = LoopOptions::default();

    let fut = run_agent_loop(
        &manifest,
        "measure only — never polled",
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
        &options,
    );

    let size = std::mem::size_of_val(&fut);
    // Printed so a `--nocapture` run reports the current figure for a PR body or an issue comment without anyone having to rebuild with nightly and `-Zprint-type-sizes`.
    eprintln!("run_agent_loop future size: {size} bytes");
    assert!(
        size < 512 * 1024,
        "run_agent_loop's future grew to {size} bytes. Each nested agent turn \
         (agent_send, a workflow step) stacks one of these on the polling \
         thread, and the daemon's tokio workers get 8 MiB — so a 512 KiB \
         future leaves room for only a handful of levels. Shrink it (box a \
         large await site) rather than raising this bound."
    );
    drop(fut);
}
