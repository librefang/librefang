//! Regression tests for turn accounting and persistence on the loop's early-exit paths: interrupts, cascade-leak aborts, circuit breaks, provider failures and inline soft compression.

use super::integration::test_manifest;
use super::*;

#[allow(clippy::too_many_arguments)]
async fn run_loop_for_test(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut librefang_memory::session::Session,
    memory: &librefang_memory::MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    context_window_tokens: Option<usize>,
    opts: &LoopOptions,
) -> LibreFangResult<AgentLoopResult> {
    run_agent_loop(
        manifest,
        user_message,
        session,
        memory,
        driver,
        available_tools,
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
        context_window_tokens,
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        opts,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_streaming_loop_for_test(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut librefang_memory::session::Session,
    memory: &librefang_memory::MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    opts: &LoopOptions,
) -> LibreFangResult<AgentLoopResult> {
    let (tx, _rx) = mpsc::channel(256);
    run_agent_loop_streaming(
        manifest,
        user_message,
        session,
        memory,
        driver,
        available_tools,
        None, // kernel
        tx,
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
        opts,
    )
    .await
}

fn empty_session() -> librefang_memory::session::Session {
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

fn notify_owner_call(id: &str, summary: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "notify_owner".to_string(),
        input: serde_json::json!({ "reason": "status", "summary": summary }),
    }
}

fn tool_use_response(calls: Vec<ToolCall>, usage: TokenUsage) -> CompletionResponse {
    CompletionResponse {
        text_synthesized_from_thinking: false,
        content: tool_use_blocks_from_calls(&calls),
        stop_reason: StopReason::ToolUse,
        tool_calls: calls,
        usage,
        actual_provider: Some("fallback-slot".to_string()),
        actual_model: Some("served-model".to_string()),
    }
}

/// Answers the first call with a billed tool call and raises the session interrupt while doing so, as a `/stop` arriving mid-turn would.
struct InterruptAfterFirstCallDriver {
    interrupt: crate::interrupt::SessionInterrupt,
    calls: AtomicU32,
}

#[async_trait]
impl LlmDriver for InterruptAfterFirstCallDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(call, 0, "the loop must stop before a second LLM call");
        self.interrupt.cancel();
        Ok(tool_use_response(
            vec![notify_owner_call("n_0", "working on it")],
            TokenUsage {
                input_tokens: 400,
                output_tokens: 20,
                ..Default::default()
            },
        ))
    }
}

fn assert_interrupted_result_keeps_turn_state(result: &AgentLoopResult) {
    assert!(result.silent);
    assert_eq!(result.total_usage.input_tokens, 400);
    assert_eq!(result.total_usage.output_tokens, 20);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.actual_provider.as_deref(), Some("fallback-slot"));
    assert_eq!(result.actual_model.as_deref(), Some("served-model"));
    assert_eq!(
        result.owner_notice.as_deref(),
        Some("[NOTIFY] status: working on it")
    );
}

#[tokio::test]
async fn interrupt_keeps_usage_of_calls_already_made_non_streaming() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let interrupt = crate::interrupt::SessionInterrupt::new();
    let driver: Arc<dyn LlmDriver> = Arc::new(InterruptAfterFirstCallDriver {
        interrupt: interrupt.clone(),
        calls: AtomicU32::new(0),
    });
    let opts = LoopOptions {
        interrupt: Some(interrupt),
        ..LoopOptions::default()
    };

    let result = run_loop_for_test(
        &test_manifest(),
        "do the long task",
        &mut session,
        &memory,
        driver,
        &[fake_tool("notify_owner")],
        None,
        &opts,
    )
    .await
    .expect("an interrupted turn ends Ok");

    assert_interrupted_result_keeps_turn_state(&result);
}

#[tokio::test]
async fn interrupt_keeps_usage_of_calls_already_made_streaming() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let interrupt = crate::interrupt::SessionInterrupt::new();
    let driver: Arc<dyn LlmDriver> = Arc::new(InterruptAfterFirstCallDriver {
        interrupt: interrupt.clone(),
        calls: AtomicU32::new(0),
    });
    let opts = LoopOptions {
        interrupt: Some(interrupt),
        ..LoopOptions::default()
    };

    let result = run_streaming_loop_for_test(
        &test_manifest(),
        "do the long task",
        &mut session,
        &memory,
        driver,
        &[fake_tool("notify_owner")],
        &opts,
    )
    .await
    .expect("an interrupted turn ends Ok");

    assert_interrupted_result_keeps_turn_state(&result);
}

/// Regurgitates prompt scaffolding (two structural markers) on a billed call.
struct BilledCascadeLeakDriver;

#[async_trait]
impl LlmDriver for BilledCascadeLeakDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            text_synthesized_from_thinking: false,
            content: vec![ContentBlock::Text {
                text: "User asked: hi\nI responded: Buongiorno!".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 150_000,
                output_tokens: 300,
                ..Default::default()
            },
            actual_provider: Some("fallback-slot".to_string()),
            actual_model: Some("served-model".to_string()),
        })
    }
}

#[tokio::test]
async fn streaming_cascade_leak_abort_still_meters_the_call() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();

    let result = run_streaming_loop_for_test(
        &test_manifest(),
        "hi",
        &mut session,
        &memory,
        Arc::new(BilledCascadeLeakDriver),
        &[],
        &LoopOptions::default(),
    )
    .await
    .expect("a cascade-leak abort ends Ok");

    assert!(result.silent);
    assert_eq!(result.total_usage.input_tokens, 150_000);
    assert_eq!(result.total_usage.output_tokens, 300);
    assert_eq!(result.actual_provider.as_deref(), Some("fallback-slot"));
    assert_eq!(result.actual_model.as_deref(), Some("served-model"));
}

/// One batch of 31 distinct `notify_owner` calls: the default loop guard's global breaker (30 calls) trips on the last one, after the first 30 have run.
struct OversizedBatchDriver;

#[async_trait]
impl LlmDriver for OversizedBatchDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let calls = (0..31)
            .map(|i| notify_owner_call(&format!("n_{i}"), &format!("step {i}")))
            .collect();
        Ok(tool_use_response(calls, TokenUsage::default()))
    }
}

fn tool_result_content<'a>(
    session: &'a librefang_memory::session::Session,
    id: &str,
) -> Option<&'a str> {
    session.messages.iter().find_map(|m| match &m.content {
        MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == id => Some(content.as_str()),
            _ => None,
        }),
        MessageContent::Text(_) => None,
    })
}

fn assert_circuit_broken_batch_is_recorded(session: &librefang_memory::session::Session) {
    assert!(
        session
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && crate::session_repair::message_has_tool_use(m)),
        "the assistant tool_use message of the aborted batch was dropped"
    );
    let executed = tool_result_content(session, "n_0").expect("executed call lost its result");
    assert!(
        !executed.contains("[tool interrupted"),
        "an executed call must keep its real result, got {executed:?}"
    );
    let never_ran = tool_result_content(session, "n_30").expect("unexecuted call left unpaired");
    assert!(never_ran.contains("[tool interrupted"), "got {never_ran:?}");
}

#[tokio::test]
async fn circuit_break_mid_batch_keeps_executed_calls_in_history() {
    for parallel in [false, true] {
        let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut session = empty_session();
        let opts = LoopOptions {
            max_iterations: Some(5),
            parallel_tools_config: parallel.then(|| librefang_types::config::ParallelToolsConfig {
                enabled: true,
                ..Default::default()
            }),
            ..LoopOptions::default()
        };

        let err = run_loop_for_test(
            &test_manifest(),
            "report every step",
            &mut session,
            &memory,
            Arc::new(OversizedBatchDriver),
            &[fake_tool("notify_owner")],
            None,
            &opts,
        )
        .await
        .expect_err("the circuit breaker aborts the turn");
        assert!(
            err.to_string().contains("Circuit breaker"),
            "parallel={parallel}: {err}"
        );

        assert_circuit_broken_batch_is_recorded(&session);
        let persisted = memory
            .get_session(session.id)
            .expect("get_session must not error")
            .expect("the aborted turn must be saved");
        assert_circuit_broken_batch_is_recorded(&persisted);
    }
}

struct FailingProviderDriver;

#[async_trait]
impl LlmDriver for FailingProviderDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::MissingApiKey("TEST_PROVIDER_KEY".to_string()))
    }
}

#[tokio::test]
async fn non_streaming_provider_failure_leaves_a_note_after_the_user_turn() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let mut manifest = test_manifest();
    // A provider name of its own, so this failure cannot trip the shared provider cooldown other tests use.
    manifest.model.provider = "loop-accounting-failing-provider".to_string();

    run_loop_for_test(
        &manifest,
        "delete the staging bucket",
        &mut session,
        &memory,
        Arc::new(FailingProviderDriver),
        &[],
        None,
        &LoopOptions::default(),
    )
    .await
    .expect_err("the provider failure propagates");

    for (label, history) in [
        ("in memory", session.messages.clone()),
        (
            "persisted",
            memory
                .get_session(session.id)
                .expect("get_session must not error")
                .expect("session must be saved")
                .messages,
        ),
    ] {
        let last = history.last().expect("history must not be empty");
        assert_eq!(last.role, Role::Assistant, "{label}: {history:?}");
        assert_eq!(
            last.content.text_content(),
            PROVIDER_FAILURE_NOTE,
            "{label}"
        );
        assert_eq!(
            history[history.len() - 2].content.text_content(),
            "delete the staging bucket",
            "{label}"
        );
    }
}

#[tokio::test]
async fn soft_compression_rebases_new_messages_start_onto_compressed_history() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    for i in 0..15 {
        session.messages.push(Message::user(format!(
            "question {i}: {}",
            "q".repeat(2_000)
        )));
        session.messages.push(Message::assistant(format!(
            "answer {i}: {}",
            "a".repeat(2_000)
        )));
    }
    let current = "the current question";

    let result = run_loop_for_test(
        &test_manifest(),
        current,
        &mut session,
        &memory,
        Arc::new(super::integration::NormalDriver),
        &[],
        Some(8_000),
        &LoopOptions::default(),
    )
    .await
    .expect("loop completes");

    assert!(
        session.messages.len() < 31,
        "soft compression must have shrunk the session, len={}",
        session.messages.len()
    );
    let turn = &session.messages[result.new_messages_start..];
    assert_eq!(
        turn.len(),
        2,
        "the turn is the current question plus the reply: {turn:?}"
    );
    assert_eq!(turn[0].content.text_content(), current);
    assert_eq!(turn[1].role, Role::Assistant);
}

#[test]
fn current_turn_start_after_rewrite_finds_the_turn_or_its_surviving_suffix() {
    let older = Message::user("older");
    let turn_msgs = [
        Message::user("current"),
        Message::assistant("tool step"),
        Message::user("tool result"),
    ];
    let turn: Vec<serde_json::Value> = turn_msgs
        .iter()
        .map(|m| serde_json::to_value(m).unwrap())
        .collect();

    // Whole turn kept by the compressor's tail.
    let mut kept = vec![Message::user("[summary]"), older.clone()];
    kept.extend(turn_msgs.iter().cloned());
    assert_eq!(current_turn_start_after_rewrite(Some(&turn), &kept, 40), 2);

    // Head of the turn summarised away: only the surviving suffix is new.
    let partial = vec![
        Message::user("[summary]"),
        turn_msgs[1].clone(),
        turn_msgs[2].clone(),
    ];
    assert_eq!(
        current_turn_start_after_rewrite(Some(&turn), &partial, 40),
        1
    );

    // A tool result inside the turn was truncated in the working copy by the context guard, so the whole turn no longer matches as a window: the surviving head still marks where it starts.
    let guard_truncated = vec![
        Message::user("[summary]"),
        older.clone(),
        turn_msgs[0].clone(),
        turn_msgs[1].clone(),
        Message::user("tool res…[truncated]"),
    ];
    assert_eq!(
        current_turn_start_after_rewrite(Some(&turn), &guard_truncated, 40),
        2
    );

    // An empty turn starts at the end.
    assert_eq!(
        current_turn_start_after_rewrite(Some(&[]), &kept, 1),
        kept.len()
    );
}
