//! The prompt-cache metric must be emitted once for every turn that reached a model, with a label saying how the turn ended (#8754).
//! The helper tests pin the event itself; the loop tests pin the wiring, including the silent exits that used to log nothing at all.

use std::io;
use std::sync::{Arc, Mutex};

use librefang_types::message::TokenUsage;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

use super::integration::{test_manifest, AlwaysFailingToolDriver, NormalDriver};
use super::loop_accounting::{
    empty_session, run_loop_for_test, run_streaming_loop_for_test, BilledCascadeLeakDriver,
};
use super::*;
use crate::agent_loop::end_turn::log_cache_metrics;

const EVENT: &str = "prompt cache metrics for turn";

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Install a capturing subscriber for the duration of the guard.
fn install() -> (tracing::subscriber::DefaultGuard, CaptureWriter) {
    let writer = CaptureWriter::default();
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(false)
        .with_target(true);
    let subscriber = tracing_subscriber::registry().with(layer);
    (tracing::subscriber::set_default(subscriber), writer)
}

fn captured(writer: &CaptureWriter) -> String {
    String::from_utf8(writer.0.lock().unwrap().clone()).unwrap()
}

fn metric_lines(writer: &CaptureWriter) -> usize {
    captured(writer).matches(EVENT).count()
}

/// Always asks for a billed tool call, so the only way out is the iteration cap.
struct AlwaysToolCallDriver;

#[async_trait]
impl LlmDriver for AlwaysToolCallDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = ToolCall {
            id: "call_0".to_string(),
            name: "notify_owner".to_string(),
            input: serde_json::json!({ "reason": "status", "summary": "step" }),
        };
        Ok(CompletionResponse {
            text_synthesized_from_thinking: false,
            content: vec![ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![call],
            usage: TokenUsage {
                input_tokens: 150_000,
                output_tokens: 300,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

#[test]
fn logs_the_metric_for_a_turn_that_spent_tokens() {
    let usage = TokenUsage {
        input_tokens: 1000,
        output_tokens: 20,
        cache_creation_input_tokens: 100,
        cache_read_input_tokens: 700,
    };
    let (guard, writer) = install();
    log_cache_metrics("agent-A", &usage, "completed");
    drop(guard);
    let captured = captured(&writer);
    assert!(captured.contains(EVENT), "got: {captured}");
    assert!(captured.contains("agent=\"agent-A\""), "got: {captured}");
    assert!(
        captured.contains("outcome=\"completed\""),
        "got: {captured}"
    );
    assert!(captured.contains("hit_ratio="), "got: {captured}");
    // The totals are what tells "nothing cached" apart from a genuine 0 % hit, so all three must be on the line.
    assert!(captured.contains("input=1000"), "got: {captured}");
    assert!(captured.contains("creation=100"), "got: {captured}");
    assert!(captured.contains("read=700"), "got: {captured}");
}

#[test]
fn stays_silent_when_the_turn_never_reached_a_model() {
    let (guard, writer) = install();
    log_cache_metrics("agent-A", &TokenUsage::default(), "completed");
    drop(guard);
    assert_eq!(metric_lines(&writer), 0, "got: {}", captured(&writer));
}

#[test]
fn logs_a_turn_that_reported_only_cache_buckets() {
    // A partial usage block: no prompt size, but cache writes were reported.
    // The turn still spent tokens, so the line must appear with its totals.
    let usage = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_input_tokens: 5,
        cache_read_input_tokens: 0,
    };
    let (guard, writer) = install();
    log_cache_metrics("agent-A", &usage, "error");
    drop(guard);
    let captured = captured(&writer);
    assert!(captured.contains(EVENT), "got: {captured}");
    assert!(captured.contains("creation=5"), "got: {captured}");
}

#[tokio::test]
async fn completed_turn_emits_one_line() {
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let result = run_loop_for_test(
        &test_manifest(),
        "hello",
        &mut session,
        &memory,
        Arc::new(NormalDriver),
        &[],
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("a plain reply ends Ok");
    assert!(!result.silent);
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(captured(&writer).contains("outcome=\"completed\""));
    // The field has always carried the agent id, not the display name; the exit counter is the one that uses the readable name.
    assert!(
        captured(&writer).contains(&format!("agent=\"{}\"", session.agent_id.0)),
        "got: {}",
        captured(&writer)
    );
    assert!(
        !captured(&writer).contains("agent=\"test-agent\""),
        "the metric must not switch to the display name: {}",
        captured(&writer)
    );
}

#[tokio::test]
async fn tool_failure_turn_emits_one_line_labelled_repeated_tool_failures() {
    // The `Err` returns log from inside the loop, not from the wrapper, so this is the half of the change no `Ok` test can see.
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let err = run_loop_for_test(
        &test_manifest(),
        "Do something",
        &mut session,
        &memory,
        Arc::new(AlwaysFailingToolDriver),
        &[],
        None,
        &LoopOptions::default(),
    )
    .await
    .expect_err("repeated tool failures end the turn");
    assert!(
        matches!(err, LibreFangError::RepeatedToolFailures { .. }),
        "got: {err:?}"
    );
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(
        captured(&writer).contains("outcome=\"repeated_tool_failures\""),
        "got: {}",
        captured(&writer)
    );
}

#[tokio::test]
async fn silent_turn_still_emits_one_line() {
    // A cascade-leak abort is one of the silent exits: the reply is thrown away
    // and the turn used to log nothing, although the tokens were spent and billed.
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let result = run_loop_for_test(
        &test_manifest(),
        "hi",
        &mut session,
        &memory,
        Arc::new(BilledCascadeLeakDriver),
        &[],
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("a cascade-leak abort ends Ok");
    assert!(result.silent, "expected a silent result");
    assert_eq!(
        result.total_usage.input_tokens, 150_000,
        "the turn spent tokens, so it must be reported"
    );
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(
        captured(&writer).contains("outcome=\"silent\""),
        "got: {}",
        captured(&writer)
    );
}

#[tokio::test]
async fn capped_turn_emits_one_line_labelled_max_iterations() {
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let tool = fake_tool("notify_owner");
    let opts = LoopOptions {
        max_iterations: Some(2),
        ..LoopOptions::default()
    };
    let result = run_loop_for_test(
        &test_manifest(),
        "loop forever",
        &mut session,
        &memory,
        Arc::new(AlwaysToolCallDriver),
        std::slice::from_ref(&tool),
        None,
        &opts,
    )
    .await
    .expect("a capped turn ends Ok");
    assert!(result.hit_iteration_cap, "expected the cap exit");
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(
        captured(&writer).contains("outcome=\"max_iterations\""),
        "got: {}",
        captured(&writer)
    );
}

#[tokio::test]
async fn streaming_completed_turn_emits_one_line() {
    // The streaming loop runs its own wrapper; when that call went missing the
    // non-streaming tests stayed green while every channel-bot turn lost its line.
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let result = run_streaming_loop_for_test(
        &test_manifest(),
        "hello",
        &mut session,
        &memory,
        Arc::new(NormalDriver),
        &[],
        &LoopOptions::default(),
    )
    .await
    .expect("a plain streaming reply ends Ok");
    assert!(!result.silent);
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(captured(&writer).contains("outcome=\"completed\""));
}

#[tokio::test]
async fn streaming_silent_turn_emits_one_line() {
    let (_guard, writer) = install();
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
    .expect("a streaming cascade-leak abort ends Ok");
    assert!(result.silent, "expected a silent result");
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(captured(&writer).contains("outcome=\"silent\""));
}

/// A text-only reply that hits the output limit, which is the continuation exit.
struct TruncatedReplyDriver;

#[async_trait]
impl LlmDriver for TruncatedReplyDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            text_synthesized_from_thinking: false,
            content: vec![ContentBlock::Text {
                text: "a reply that ran out of room".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::MaxTokens,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 150_000,
                output_tokens: 4_096,
                ..Default::default()
            },
            actual_provider: None,
            actual_model: None,
        })
    }
}

#[tokio::test]
async fn continuation_truncated_turn_is_labelled_max_continuation() {
    // Without the flag this turn is indistinguishable from an ordinary
    // completed one, although its prompt is the largest kind in the system.
    let (_guard, writer) = install();
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = empty_session();
    let result = run_loop_for_test(
        &test_manifest(),
        "tell me everything",
        &mut session,
        &memory,
        Arc::new(TruncatedReplyDriver),
        &[],
        None,
        &LoopOptions::default(),
    )
    .await
    .expect("a truncated reply ends Ok");
    assert!(
        result.truncated_by_continuation,
        "expected the continuation flag"
    );
    assert!(!result.hit_iteration_cap);
    assert_eq!(metric_lines(&writer), 1, "got: {}", captured(&writer));
    assert!(
        captured(&writer).contains("outcome=\"max_continuation\""),
        "got: {}",
        captured(&writer)
    );
}
