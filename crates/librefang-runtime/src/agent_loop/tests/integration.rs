use super::*;

// --- Integration tests for empty response guards ---

pub(super) fn test_manifest() -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
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
        })
    }
}

#[tokio::test]
async fn test_empty_response_after_tool_use_returns_fallback() {
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
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

    let result = run_agent_loop_streaming(
        &manifest,
        "\u{1F934}",
        &mut session,
        &memory,
        driver,
        &[], // no tools registered — ensures any tool execution would panic/err
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
async fn test_streaming_max_continuations_with_directives_preserves_reply_directives() {
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
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
