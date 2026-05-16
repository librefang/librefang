use super::integration::{
    test_manifest, AlwaysFailingToolDriver, FailThenTextDriver, NormalDriver,
};
use super::*;

// --- Tests for strip_provider_prefix and model ID normalization ---

#[test]
fn test_strip_provider_prefix_basic() {
    assert_eq!(
        strip_provider_prefix("openrouter/google/gemini-2.5-flash", "openrouter"),
        "google/gemini-2.5-flash"
    );
    assert_eq!(
        strip_provider_prefix("openrouter:google/gemini-2.5-flash", "openrouter"),
        "google/gemini-2.5-flash"
    );
}

#[test]
fn test_strip_provider_prefix_no_prefix() {
    // Already qualified — should pass through unchanged
    assert_eq!(
        strip_provider_prefix("google/gemini-2.5-flash", "openrouter"),
        "google/gemini-2.5-flash"
    );
}

#[test]
fn test_strip_provider_prefix_non_openrouter() {
    // Non-OpenRouter providers: bare names should pass through
    assert_eq!(strip_provider_prefix("gpt-4o", "openai"), "gpt-4o");
    assert_eq!(strip_provider_prefix("sonnet", "anthropic"), "sonnet");
}

#[test]
fn test_normalize_bare_model_openrouter_gemini() {
    // Bare "gemini-2.5-flash" with openrouter → "google/gemini-2.5-flash"
    assert_eq!(
        strip_provider_prefix("gemini-2.5-flash", "openrouter"),
        "google/gemini-2.5-flash"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_claude() {
    assert_eq!(
        strip_provider_prefix("claude-sonnet-4", "openrouter"),
        "anthropic/claude-sonnet-4"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_gpt() {
    assert_eq!(
        strip_provider_prefix("gpt-4o", "openrouter"),
        "openai/gpt-4o"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_llama() {
    assert_eq!(
        strip_provider_prefix("llama-3.3-70b-instruct", "openrouter"),
        "meta-llama/llama-3.3-70b-instruct"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_deepseek() {
    assert_eq!(
        strip_provider_prefix("deepseek-chat", "openrouter"),
        "deepseek/deepseek-chat"
    );
    assert_eq!(
        strip_provider_prefix("deepseek-r1", "openrouter"),
        "deepseek/deepseek-r1"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_mistral() {
    assert_eq!(
        strip_provider_prefix("mistral-large-latest", "openrouter"),
        "mistralai/mistral-large-latest"
    );
}

#[test]
fn test_normalize_bare_model_openrouter_qwen() {
    assert_eq!(
        strip_provider_prefix("qwen-2.5-72b-instruct", "openrouter"),
        "qwen/qwen-2.5-72b-instruct"
    );
}

#[test]
fn test_normalize_bare_model_with_free_suffix() {
    assert_eq!(
        strip_provider_prefix("gemma-2-9b-it:free", "openrouter"),
        "google/gemma-2-9b-it:free"
    );
    assert_eq!(
        strip_provider_prefix("deepseek-r1:free", "openrouter"),
        "deepseek/deepseek-r1:free"
    );
}

#[test]
fn test_normalize_bare_model_together() {
    // Together also uses org/model format
    assert_eq!(
        strip_provider_prefix("llama-3.3-70b-instruct", "together"),
        "meta-llama/llama-3.3-70b-instruct"
    );
}

#[test]
fn test_normalize_unknown_bare_model_passes_through() {
    // Unknown model name should pass through with a warning (not panic)
    assert_eq!(
        strip_provider_prefix("my-custom-model", "openrouter"),
        "my-custom-model"
    );
}

#[test]
fn test_normalize_openai_o_series() {
    assert_eq!(
        strip_provider_prefix("o1-preview", "openrouter"),
        "openai/o1-preview"
    );
    assert_eq!(
        strip_provider_prefix("o3-mini", "openrouter"),
        "openai/o3-mini"
    );
}

#[test]
fn test_normalize_command_r() {
    assert_eq!(
        strip_provider_prefix("command-r-plus", "openrouter"),
        "cohere/command-r-plus"
    );
}

#[test]
fn test_needs_qualified_model_id() {
    assert!(needs_qualified_model_id("openrouter"));
    assert!(needs_qualified_model_id("together"));
    assert!(needs_qualified_model_id("fireworks"));
    assert!(needs_qualified_model_id("replicate"));
    assert!(needs_qualified_model_id("huggingface"));
    assert!(!needs_qualified_model_id("openai"));
    assert!(!needs_qualified_model_id("anthropic"));
    assert!(!needs_qualified_model_id("groq"));
}

// --- user_message_has_action_intent tests ---

#[test]
fn test_action_intent_send() {
    assert!(user_message_has_action_intent("send this to Telegram"));
    assert!(user_message_has_action_intent("Send the report via email"));
}

#[test]
fn test_action_intent_execute() {
    assert!(user_message_has_action_intent("execute the script"));
    assert!(user_message_has_action_intent(
        "please execute X and report"
    ));
}

#[test]
fn test_action_intent_create_delete() {
    assert!(user_message_has_action_intent("create a new file"));
    assert!(user_message_has_action_intent("delete the old records"));
}

#[test]
fn test_action_intent_combined() {
    assert!(user_message_has_action_intent(
        "fetch the news about AI and send to Telegram"
    ));
}

#[test]
fn test_action_intent_with_punctuation() {
    assert!(user_message_has_action_intent("send, please"));
    assert!(user_message_has_action_intent("can you deploy!"));
    assert!(user_message_has_action_intent("execute?"));
}

#[test]
fn test_action_intent_negative_plain_question() {
    // Simple questions without action keywords should not trigger
    assert!(!user_message_has_action_intent("what is the weather?"));
    assert!(!user_message_has_action_intent("explain how this works"));
    assert!(!user_message_has_action_intent("tell me about Rust"));
}

#[test]
fn test_action_intent_negative_no_keyword() {
    assert!(!user_message_has_action_intent("hello there"));
    assert!(!user_message_has_action_intent(
        "how do I configure logging?"
    ));
}

#[test]
fn test_action_intent_case_insensitive() {
    assert!(user_message_has_action_intent("SEND this now"));
    assert!(user_message_has_action_intent("Deploy the app"));
    assert!(user_message_has_action_intent("EXECUTE the tests"));
}

#[test]
fn test_action_intent_all_keywords() {
    let keywords = [
        "send", "execute", "create", "delete", "remove", "write", "publish", "deploy", "install",
        "upload", "download", "forward", "submit", "trigger", "launch", "notify", "schedule",
        "rename", "fetch",
    ];
    for kw in &keywords {
        let msg = format!("please {} the thing", kw);
        assert!(
            user_message_has_action_intent(&msg),
            "Expected action intent for keyword '{}'",
            kw
        );
    }
}

#[tokio::test]
async fn test_tool_failure_allows_retry_on_next_iteration() {
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
    let driver: Arc<dyn LlmDriver> = Arc::new(FailThenTextDriver::new());

    let result = run_agent_loop(
        &manifest,
        "Do something",
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
    .expect("Loop should complete after retry");

    assert_eq!(
        result.iterations, 2,
        "Loop must run 2 iterations (fail + retry), got {}",
        result.iterations
    );
    assert!(
        result.response.contains("Recovered after tool failure"),
        "Expected retry text response, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_repeated_tool_failures_cap_exits_loop() {
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
    let driver: Arc<dyn LlmDriver> = Arc::new(AlwaysFailingToolDriver);

    let err = run_agent_loop(
        &manifest,
        "Do something",
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
    .expect_err("Loop must exit with RepeatedToolFailures");

    match err {
        LibreFangError::RepeatedToolFailures { iterations, .. } => {
            assert_eq!(
                iterations, MAX_CONSECUTIVE_ALL_FAILED,
                "Cap should trigger after MAX_CONSECUTIVE_ALL_FAILED consecutive all-failed iterations"
            );
        }
        other => panic!("Expected RepeatedToolFailures, got {other:?}"),
    }
}

#[tokio::test]
async fn test_streaming_tool_failure_allows_retry() {
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
    let driver: Arc<dyn LlmDriver> = Arc::new(FailThenTextDriver::new());
    let (tx, _rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Do something",
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
    .expect("Streaming loop should complete after retry");

    assert_eq!(
        result.iterations, 2,
        "Streaming loop must run 2 iterations (fail + retry), got {}",
        result.iterations
    );
    assert!(
        result.response.contains("Recovered after tool failure"),
        "Expected retry text in streaming, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn test_streaming_repeated_tool_failures_cap_exits() {
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
    let driver: Arc<dyn LlmDriver> = Arc::new(AlwaysFailingToolDriver);
    let (tx, _rx) = mpsc::channel(64);

    let err = run_agent_loop_streaming(
        &manifest,
        "Do something",
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
    .expect_err("Streaming loop must exit with RepeatedToolFailures");

    match err {
        LibreFangError::RepeatedToolFailures { iterations, .. } => {
            assert_eq!(
                iterations, MAX_CONSECUTIVE_ALL_FAILED,
                "Cap should trigger after MAX_CONSECUTIVE_ALL_FAILED consecutive all-failed iterations"
            );
        }
        other => panic!("Expected RepeatedToolFailures, got {other:?}"),
    }
}

// -------------------------------------------------------------------
// StagedToolUseTurn invariants (closes #2381 by construction)
//
// These tests lock in the structural guarantees that make orphaned
// `tool_use_id`s impossible:
//   (a) pad_missing_results only fills ids that have no result at
//       all — real error content is never overwritten.
//   (b) commit is idempotent (safe to call twice).
//   (c) a StagedToolUseTurn dropped without commit leaves
//       session.messages untouched (drop-safety via ? propagation).
//   (d) commit atomically pushes exactly one assistant message plus
//       one user{tool_results} message in that order.
//   (e) the happy path batch case commits once and grows the
//       session by exactly 2 messages.
// -------------------------------------------------------------------

fn fresh_session() -> librefang_memory::session::Session {
    librefang_memory::session::Session {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    }
}

fn staged_two_tool_use(agent_id_str: String) -> StagedToolUseTurn {
    StagedToolUseTurn {
        assistant_msg: Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "tool-a".to_string(),
                    name: "tool_a".to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "tool-b".to_string(),
                    name: "tool_b".to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
            ]),
            pinned: false,
            timestamp: None,
        },
        tool_call_ids: vec![
            ("tool-a".to_string(), "tool_a".to_string()),
            ("tool-b".to_string(), "tool_b".to_string()),
        ],
        tool_result_blocks: Vec::new(),
        rationale_text: None,
        allowed_tool_names: Vec::new(),
        caller_id_str: agent_id_str,
        committed: false,
        per_result_threshold: crate::tool_budget::PER_RESULT_THRESHOLD,
        per_turn_budget: crate::tool_budget::PER_TURN_BUDGET,
        max_artifact_bytes: crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    }
}

#[test]
fn staged_pad_missing_results_fills_uncalled_ids_only() {
    // Real hard-error content on tool-a must survive pad untouched;
    // tool-b has no result so pad fabricates an "interrupted" one.
    let session = fresh_session();
    let mut staged = staged_two_tool_use(session.agent_id.to_string());
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-a".to_string(),
        tool_name: "tool_a".to_string(),
        content: "Permission denied: unknown tool".to_string(),
        is_error: true,
        status: librefang_types::tool::ToolExecutionStatus::Error,
        approval_request_id: None,
    });

    staged.pad_missing_results();

    assert_eq!(staged.tool_result_blocks.len(), 2);
    match &staged.tool_result_blocks[0] {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            ..
        } => {
            assert_eq!(tool_use_id, "tool-a");
            assert_eq!(content, "Permission denied: unknown tool");
            assert!(*is_error);
        }
        other => panic!("expected tool-a real error result, got {other:?}"),
    }
    match &staged.tool_result_blocks[1] {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            status,
            ..
        } => {
            assert_eq!(tool_use_id, "tool-b");
            assert!(content.contains("[tool interrupted"));
            assert!(*is_error);
            assert_eq!(*status, librefang_types::tool::ToolExecutionStatus::Error);
        }
        other => panic!("expected tool-b synthetic result, got {other:?}"),
    }
    // Session was never touched — pad is a staging-buffer operation.
    assert!(session.messages.is_empty());
}

#[test]
fn staged_pad_missing_results_noop_when_all_ids_have_results() {
    let mut staged = staged_two_tool_use("agent".to_string());
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-a".to_string(),
        tool_name: "tool_a".to_string(),
        content: "ok-a".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-b".to_string(),
        tool_name: "tool_b".to_string(),
        content: "ok-b".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });

    staged.pad_missing_results();

    assert_eq!(staged.tool_result_blocks.len(), 2);
    for block in &staged.tool_result_blocks {
        match block {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                assert!(!content.contains("[tool interrupted"));
                assert!(!*is_error);
            }
            other => panic!("expected tool result, got {other:?}"),
        }
    }
}

#[test]
fn staged_commit_is_idempotent() {
    let mut session = fresh_session();
    let mut messages = Vec::new();
    let mut staged = staged_two_tool_use(session.agent_id.to_string());
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-a".to_string(),
        tool_name: "tool_a".to_string(),
        content: "ok-a".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-b".to_string(),
        tool_name: "tool_b".to_string(),
        content: "ok-b".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });

    let first = staged.commit(&mut session, &mut messages);
    let len_after_first = session.messages.len();
    let msgs_after_first = messages.len();
    assert_eq!(first.success_count, 2);
    assert_eq!(first.hard_error_count, 0);
    assert_eq!(len_after_first, 2);
    assert_eq!(msgs_after_first, 2);
    assert!(staged.committed);

    // Second commit is a no-op: summary is default, no new messages.
    let second = staged.commit(&mut session, &mut messages);
    assert_eq!(second, ToolResultOutcomeSummary::default());
    assert_eq!(session.messages.len(), len_after_first);
    assert_eq!(messages.len(), msgs_after_first);
}

#[test]
fn staged_drop_without_commit_does_not_touch_session() {
    // This test simulates the `?`-propagation path: a caller builds
    // a StagedToolUseTurn, appends some results, then an error
    // propagates through the caller (in production via `?`) — the
    // staged turn is dropped without commit. Session state must be
    // byte-for-byte identical to the pre-stage snapshot; no orphan
    // ToolUse can have reached disk.
    let session = fresh_session();
    let snapshot = session.messages.clone();

    {
        let mut staged = staged_two_tool_use(session.agent_id.to_string());
        staged.append_result(ContentBlock::ToolResult {
            tool_use_id: "tool-a".to_string(),
            tool_name: "tool_a".to_string(),
            content: "ok-a".to_string(),
            is_error: false,
            status: librefang_types::tool::ToolExecutionStatus::Completed,
            approval_request_id: None,
        });
        // Intentionally drop `staged` here without commit.
        assert!(!staged.committed);
    }

    assert_eq!(session.messages.len(), snapshot.len());
    assert!(session.messages.is_empty());
}

#[test]
fn staged_batch_with_no_issues_commits_once() {
    // Happy path: 2 tool calls, both succeed, commit grows the
    // session by exactly 2 messages: [assistant{ToolUse×2},
    // user{ToolResult×2 + guidance text}].
    let mut session = fresh_session();
    let mut messages = Vec::new();
    let mut staged = staged_two_tool_use(session.agent_id.to_string());
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-a".to_string(),
        tool_name: "tool_a".to_string(),
        content: "ok-a".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "tool-b".to_string(),
        tool_name: "tool_b".to_string(),
        content: "ok-b".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });
    // pad_missing_results is a no-op on the happy path — guarantee
    // that explicitly, so a future refactor adding padding side
    // effects breaks this test.
    let before = staged.tool_result_blocks.len();
    staged.pad_missing_results();
    assert_eq!(staged.tool_result_blocks.len(), before);

    let summary = staged.commit(&mut session, &mut messages);

    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.hard_error_count, 0);
    assert_eq!(session.messages.len(), 2);
    assert_eq!(messages.len(), 2);
    assert!(matches!(
        &session.messages[0].content,
        MessageContent::Blocks(blocks)
            if matches!(
                blocks.as_slice(),
                [
                    ContentBlock::ToolUse { id: id_a, .. },
                    ContentBlock::ToolUse { id: id_b, .. },
                ] if id_a == "tool-a" && id_b == "tool-b"
            )
    ));
    assert!(matches!(
        &session.messages[1].content,
        MessageContent::Blocks(blocks)
            if blocks.iter().filter(|b| matches!(b, ContentBlock::ToolResult { .. })).count() == 2
    ));
}

#[test]
fn staged_hard_error_mid_batch_preserves_all_real_results() {
    // Three tool calls — tool 0 hard-errors, tools 1+2 succeed.
    // Under the pre-#2381 behaviour the `break;` after tool 0 would
    // have left tool 1 and tool 2 as orphan ids. Under the new
    // staged-commit contract, the caller is required to drive every
    // append_result before committing, so the final session carries
    // all three real results (real hard-error content preserved for
    // tool 0, real successes for tools 1+2) and zero synthetics.
    let mut session = fresh_session();
    let mut messages = Vec::new();
    let mut staged = StagedToolUseTurn {
        assistant_msg: Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "t0".to_string(),
                    name: "web_fetch".to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "web_fetch".to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "t2".to_string(),
                    name: "web_fetch".to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                },
            ]),
            pinned: false,
            timestamp: None,
        },
        tool_call_ids: vec![
            ("t0".to_string(), "web_fetch".to_string()),
            ("t1".to_string(), "web_fetch".to_string()),
            ("t2".to_string(), "web_fetch".to_string()),
        ],
        tool_result_blocks: Vec::new(),
        rationale_text: None,
        allowed_tool_names: Vec::new(),
        caller_id_str: session.agent_id.to_string(),
        committed: false,
        per_result_threshold: crate::tool_budget::PER_RESULT_THRESHOLD,
        per_turn_budget: crate::tool_budget::PER_TURN_BUDGET,
        max_artifact_bytes: crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    };

    // Simulate the batch executing end-to-end (no early break).
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "t0".to_string(),
        tool_name: "web_fetch".to_string(),
        content: "network error: Wikipedia unreachable".to_string(),
        is_error: true,
        status: librefang_types::tool::ToolExecutionStatus::Error,
        approval_request_id: None,
    });
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "t1".to_string(),
        tool_name: "web_fetch".to_string(),
        content: "fetched page 1".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });
    staged.append_result(ContentBlock::ToolResult {
        tool_use_id: "t2".to_string(),
        tool_name: "web_fetch".to_string(),
        content: "fetched page 2".to_string(),
        is_error: false,
        status: librefang_types::tool::ToolExecutionStatus::Completed,
        approval_request_id: None,
    });

    // pad is a no-op — every id already has a real result.
    staged.pad_missing_results();
    assert_eq!(staged.tool_result_blocks.len(), 3);

    let summary = staged.commit(&mut session, &mut messages);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.hard_error_count, 1);
    assert_eq!(session.messages.len(), 2);

    // Verify every real result content survived — no synthetic
    // "[tool interrupted" placeholders, because no id was skipped.
    match &session.messages[1].content {
        MessageContent::Blocks(blocks) => {
            let results: Vec<_> = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                        ..
                    } => Some((tool_use_id.clone(), content.clone(), *is_error)),
                    _ => None,
                })
                .collect();
            assert_eq!(results.len(), 3);
            assert_eq!(results[0].0, "t0");
            assert_eq!(results[0].1, "network error: Wikipedia unreachable");
            assert!(results[0].2);
            assert_eq!(results[1].0, "t1");
            assert_eq!(results[1].1, "fetched page 1");
            assert!(!results[1].2);
            assert_eq!(results[2].0, "t2");
            assert_eq!(results[2].1, "fetched page 2");
            assert!(!results[2].2);
            for (_, content, _) in &results {
                assert!(!content.contains("[tool interrupted"));
            }
        }
        other => panic!("expected blocks message, got {other:?}"),
    }
}

// ── Web search augmentation tests ───────────────────────────

#[test]
fn test_should_augment_web_search_off() {
    let manifest = AgentManifest {
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::Off,
        ..Default::default()
    };
    assert!(!should_augment_web_search(&manifest));
}

#[test]
fn test_should_augment_web_search_always() {
    let manifest = AgentManifest {
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::Always,
        ..Default::default()
    };
    assert!(should_augment_web_search(&manifest));
}

#[test]
fn test_should_augment_web_search_auto_with_tools() {
    let mut manifest = AgentManifest {
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::Auto,
        ..Default::default()
    };
    // model_supports_tools = true → don't augment
    manifest.metadata.insert(
        "model_supports_tools".to_string(),
        serde_json::Value::Bool(true),
    );
    assert!(!should_augment_web_search(&manifest));
}

#[test]
fn test_should_augment_web_search_auto_without_tools() {
    let mut manifest = AgentManifest {
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::Auto,
        ..Default::default()
    };
    // model_supports_tools = false → augment
    manifest.metadata.insert(
        "model_supports_tools".to_string(),
        serde_json::Value::Bool(false),
    );
    assert!(should_augment_web_search(&manifest));
}

#[test]
fn test_should_augment_web_search_auto_no_metadata() {
    let manifest = AgentManifest {
        web_search_augmentation: librefang_types::agent::WebSearchAugmentationMode::Auto,
        ..Default::default()
    };
    // No metadata → assume tools supported → don't augment (conservative)
    assert!(!should_augment_web_search(&manifest));
}

#[test]
fn test_search_query_gen_prompt_not_empty() {
    assert!(!SEARCH_QUERY_GEN_PROMPT.is_empty());
    assert!(SEARCH_QUERY_GEN_PROMPT.contains("queries"));
}

#[test]
fn test_web_search_augmentation_mode_serde_roundtrip() {
    use librefang_types::agent::WebSearchAugmentationMode;

    for mode in [
        WebSearchAugmentationMode::Off,
        WebSearchAugmentationMode::Auto,
        WebSearchAugmentationMode::Always,
    ] {
        let json = serde_json::to_string(&mode).unwrap();
        let back: WebSearchAugmentationMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, back);
    }
}

#[test]
fn test_web_search_augmentation_mode_toml_roundtrip() {
    #[derive(serde::Deserialize)]
    struct W {
        mode: librefang_types::agent::WebSearchAugmentationMode,
    }
    for label in ["off", "auto", "always"] {
        let toml_str = format!("mode = \"{label}\"");
        let w: W = toml::from_str(&toml_str).unwrap();
        let json = serde_json::to_string(&w.mode).unwrap();
        assert_eq!(json, format!("\"{label}\""));
    }
}

#[test]
fn test_manifest_default_web_search_augmentation_is_auto() {
    let manifest = AgentManifest::default();
    assert_eq!(
        manifest.web_search_augmentation,
        librefang_types::agent::WebSearchAugmentationMode::Auto,
    );
}

#[test]
fn test_manifest_with_web_search_augmentation_toml() {
    let toml_str = r#"
        name = "search-bot"
        web_search_augmentation = "always"
    "#;
    let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
    assert_eq!(
        manifest.web_search_augmentation,
        librefang_types::agent::WebSearchAugmentationMode::Always,
    );
}

#[test]
fn test_manifest_without_web_search_augmentation_toml() {
    let toml_str = r#"
        name = "plain-bot"
    "#;
    let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
    assert_eq!(
        manifest.web_search_augmentation,
        librefang_types::agent::WebSearchAugmentationMode::Auto,
    );
}

// -----------------------------------------------------------------------
// AgentLoopResult.owner_notice (§A — owner-notify channel)
// -----------------------------------------------------------------------

#[test]
fn agent_loop_result_owner_notice_defaults_none() {
    let r = AgentLoopResult::default();
    assert!(r.owner_notice.is_none());
}

#[test]
fn agent_loop_result_owner_notice_can_be_set() {
    let r = AgentLoopResult {
        owner_notice: Some("Sir, the appointment is at 3pm.".into()),
        ..AgentLoopResult::default()
    };
    assert_eq!(
        r.owner_notice.as_deref(),
        Some("Sir, the appointment is at 3pm.")
    );
}

#[test]
fn resolve_max_history_uses_manifest_when_set() {
    let manifest = AgentManifest {
        name: "agent-a".into(),
        max_history_messages: Some(7),
        ..AgentManifest::default()
    };
    let opts = LoopOptions {
        max_history_messages: Some(20),
        ..Default::default()
    };
    assert_eq!(resolve_max_history(&manifest, &opts), 7);
}

#[test]
fn resolve_max_history_falls_back_to_opts_when_manifest_unset() {
    let manifest = AgentManifest {
        name: "agent-b".into(),
        ..AgentManifest::default()
    };
    let opts = LoopOptions {
        max_history_messages: Some(20),
        ..Default::default()
    };
    assert_eq!(resolve_max_history(&manifest, &opts), 20);
}

#[test]
fn resolve_max_history_falls_back_to_default_when_both_unset() {
    let manifest = AgentManifest {
        name: "agent-c".into(),
        ..AgentManifest::default()
    };
    let opts = LoopOptions::default();
    assert_eq!(
        resolve_max_history(&manifest, &opts),
        DEFAULT_MAX_HISTORY_MESSAGES
    );
}

#[test]
fn resolve_max_history_clamps_below_floor() {
    let manifest = AgentManifest {
        name: "agent-d".into(),
        max_history_messages: Some(2),
        ..AgentManifest::default()
    };
    let opts = LoopOptions::default();
    assert_eq!(resolve_max_history(&manifest, &opts), MIN_HISTORY_MESSAGES);
}

#[test]
fn resolve_max_history_clamps_zero() {
    let manifest = AgentManifest {
        name: "agent-e".into(),
        max_history_messages: Some(0),
        ..AgentManifest::default()
    };
    let opts = LoopOptions::default();
    assert_eq!(resolve_max_history(&manifest, &opts), MIN_HISTORY_MESSAGES);
}

#[test]
fn resolve_max_history_passes_through_at_floor_and_above() {
    let opts = LoopOptions::default();

    let manifest_at_floor = AgentManifest {
        name: "agent-f".into(),
        max_history_messages: Some(MIN_HISTORY_MESSAGES),
        ..AgentManifest::default()
    };
    assert_eq!(
        resolve_max_history(&manifest_at_floor, &opts),
        MIN_HISTORY_MESSAGES
    );

    let manifest_above_floor = AgentManifest {
        name: "agent-f".into(),
        max_history_messages: Some(200),
        ..AgentManifest::default()
    };
    assert_eq!(resolve_max_history(&manifest_above_floor, &opts), 200);
}

#[test]
fn safe_trim_messages_respects_custom_cap() {
    // Build 20 alternating user/assistant messages so the history is
    // well above any reasonable small cap. Each pair is one "turn".
    let mut messages: Vec<Message> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                Message::user(format!("u{i}"))
            } else {
                Message::assistant(format!("a{i}"))
            }
        })
        .collect();
    let mut session_messages = messages.clone();

    safe_trim_messages(
        &mut messages,
        &mut session_messages,
        "test-agent",
        "current",
        10,
    );

    assert!(
        messages.len() <= 10,
        "messages should be trimmed to <= 10, got {}",
        messages.len()
    );
    assert!(
        session_messages.len() <= 10,
        "session_messages should be trimmed to <= 10, got {}",
        session_messages.len()
    );
    assert_eq!(
        messages.first().map(|m| m.role),
        Some(Role::User),
        "history must start with a user turn after trim+repair"
    );
}

// ── record_tool_call_metric covers failure paths ───────────────────────

/// Regression for #4560 — `record_tool_call_metric` must fire with
/// `outcome="failure"` even when `execute_single_tool_call` returns
/// `Err(...)` (e.g. circuit-break), not only on the `Ok` path.
///
/// We test `record_tool_call_metric` directly: call it with `is_error =
/// true` inside a `with_local_recorder` scope and assert the counter has
/// a "failure" label — mirroring the `DebuggingRecorder` pattern used in
/// `command_lane.rs::test_submit_records_queue_wait_histogram`.
#[test]
fn test_record_tool_call_metric_failure_outcome() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    metrics::with_local_recorder(&recorder, || {
        // Simulate what the wrapper does when execute_single_tool_call_inner
        // returns Err (circuit-break or any hard error).
        record_tool_call_metric("my_tool", true);
    });

    let snap = snapshotter.snapshot().into_vec();
    let failure_counter = snap.iter().find(|(ckey, _, _, val)| {
        ckey.key().name() == "librefang_tool_call_total"
            && ckey
                .key()
                .labels()
                .any(|l| l.key() == "tool" && l.value() == "my_tool")
            && ckey
                .key()
                .labels()
                .any(|l| l.key() == "outcome" && l.value() == "failure")
            && matches!(val, DebugValue::Counter(_))
    });
    assert!(
        failure_counter.is_some(),
        "outcome=failure counter must be recorded for error paths"
    );
    if let Some((_, _, _, DebugValue::Counter(count))) = failure_counter {
        assert_eq!(*count, 1, "counter must be incremented exactly once");
    }
}

/// Success path: `record_tool_call_metric` with `is_error = false` must
/// produce `outcome="success"`.
#[test]
fn test_record_tool_call_metric_success_outcome() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    metrics::with_local_recorder(&recorder, || {
        record_tool_call_metric("other_tool", false);
    });

    let snap = snapshotter.snapshot().into_vec();
    let success_counter = snap.iter().find(|(ckey, _, _, val)| {
        ckey.key().name() == "librefang_tool_call_total"
            && ckey
                .key()
                .labels()
                .any(|l| l.key() == "outcome" && l.value() == "success")
            && matches!(val, DebugValue::Counter(_))
    });
    assert!(
        success_counter.is_some(),
        "outcome=success counter must be recorded for successful tool calls"
    );
}

// ── Incognito persistence guards (refs #4073) ──────────────────────────
//
// These two tests prove the `LoopOptions::incognito` guard at the
// `finalize_successful_end_turn` save site actually skips the SQLite
// write. Replaces the earlier `test_incognito_message_does_not_persist_session`
// integration test which never reached the save site (it used a
// misconfigured provider so the LLM call failed before any save was
// attempted, making the assertion vacuously true regardless of whether
// the guard was wired in).

/// Control: a normal end-turn with `incognito: false` MUST persist the
/// session via `finalize_successful_end_turn`. If this fails, the
/// incognito test below loses its meaning (it might be passing because
/// the save path is broken, not because the guard worked).
#[tokio::test]
async fn test_normal_turn_persists_session_as_incognito_control() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let session_id = librefang_types::agent::SessionId::new();
    let mut session = librefang_memory::session::Session {
        id: session_id,
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

    run_agent_loop(
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
    .expect("loop should complete");

    let persisted = memory
        .get_session(session_id)
        .expect("get_session must not error");
    assert!(
        persisted.is_some(),
        "control: normal (non-incognito) end-turn MUST persist session — \
         if this fails, the incognito test below tests nothing",
    );
    let persisted = persisted.unwrap();
    assert!(
        persisted.messages.len() >= 2,
        "control: normal end-turn must persist user msg + assistant reply, got {} msgs",
        persisted.messages.len(),
    );
}

/// `LoopOptions::incognito = true` MUST suppress the SQLite write at
/// `finalize_successful_end_turn` even on a clean end-turn.
#[tokio::test]
async fn test_incognito_skips_session_save_on_end_turn() {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let agent_id = librefang_types::agent::AgentId::new();
    let session_id = librefang_types::agent::SessionId::new();
    let mut session = librefang_memory::session::Session {
        id: session_id,
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
    let opts = LoopOptions {
        incognito: true,
        ..LoopOptions::default()
    };

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
    .expect("loop should complete");

    // The LLM must still have produced a normal response — incognito
    // only suppresses persistence, not the turn itself.
    assert_eq!(result.response, "Hello from the agent!");

    // Session row must NOT exist in SQLite — `save_session_async` is
    // skipped at every site under the `incognito` guard.
    let persisted = memory
        .get_session(session_id)
        .expect("get_session must not error");
    assert!(
        persisted.is_none(),
        "incognito turn MUST NOT persist session to SQLite, got: {persisted:?}",
    );

    // The in-memory `session` object held by the caller still reflects
    // the turn — the LLM saw full context and the assistant reply was
    // appended in-process. Only the disk write was suppressed.
    assert!(
        session.messages.len() >= 2,
        "in-memory session must still contain user msg + assistant reply (only the \
         SQLite write is suppressed) — got {} msgs",
        session.messages.len(),
    );
}
