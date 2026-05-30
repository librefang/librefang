//! Regression: content-block index alignment in the Anthropic SSE parser.
//!
//! Anthropic's streaming `index` is the absolute position in the response
//! `content` array. When a `content_block_start` carries a type this driver
//! does not recognise (e.g. `server_tool_use`), the parser must still occupy
//! that slot with a placeholder so every later block's vec position keeps
//! matching its API `index`. The old code routed unrecognized starts through
//! a no-op `_ => {}` arm and pushed nothing, so a recognized block that
//! followed an unknown one landed one slot too early and its
//! `content_block_delta` events were routed to the wrong accumulator (or
//! dropped entirely when the index ran past the vec length). This test drives
//! a stream where index 0 is an unknown block and asserts the recognized text
//! and tool_use at indices 1 and 2 reassemble intact.

mod common;

use common::*;
use librefang_llm_driver::StreamEvent;
use librefang_types::message::ContentBlock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Stream layout:
///   index 0 → unknown `server_tool_use` block (no deltas the driver emits)
///   index 1 → text block with two text_deltas ("Hello" + " world")
///   index 2 → tool_use block with input_json_deltas forming {"q":"rust"}
fn anthropic_sse_with_unknown_leading_block() -> ResponseTemplate {
    let mut body = String::new();

    let mut push = |event: &str, data: serde_json::Value| {
        body.push_str(&format!("event: {event}\ndata: {data}\n\n"));
    };

    push(
        "message_start",
        serde_json::json!({
            "type": "message_start",
            "message": {
                "id": "msg_align_test",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-test",
                "usage": {"input_tokens": 5, "output_tokens": 0}
            }
        }),
    );

    // index 0: a content_block type this driver does not recognise. Under the
    // pre-fix code this pushed nothing, shifting every later block left by one.
    push(
        "content_block_start",
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "server_tool_use", "id": "srv_1", "name": "web_search"}
        }),
    );
    push(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 0}),
    );

    // index 1: real text block.
    push(
        "content_block_start",
        serde_json::json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {"type": "text", "text": ""}
        }),
    );
    for piece in ["Hello", " world"] {
        push(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": {"type": "text_delta", "text": piece}
            }),
        );
    }
    push(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 1}),
    );

    // index 2: real tool_use block, input streamed as partial JSON.
    push(
        "content_block_start",
        serde_json::json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": {"type": "tool_use", "id": "tool_abc", "name": "search"}
        }),
    );
    for piece in [r#"{"q":"#, r#""rust"}"#] {
        push(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 2,
                "delta": {"type": "input_json_delta", "partial_json": piece}
            }),
        );
    }
    push(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 2}),
    );

    push(
        "message_delta",
        serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 7}
        }),
    );
    push("message_stop", serde_json::json!({"type": "message_stop"}));

    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

#[tokio::test]
#[serial_test::serial]
async fn unknown_leading_block_preserves_index_alignment() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_anthropic_driver(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse_with_unknown_leading_block())
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&driver, request_with_tools("claude-test")).await;
    let response = result.expect("stream should succeed");

    // --- Final assembled response ---------------------------------------
    // The text block (API index 1) must reassemble intact. Pre-fix, its
    // deltas were keyed by index 1 but the Text accumulator sat at vec[0]
    // (the unknown block having pushed nothing), so get_mut(1) missed and
    // the text came back empty.
    let text: String = response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text, "Hello world",
        "text block (index 1) must reassemble after the unknown block at index 0; got content {:?}",
        response.content
    );

    // The tool_use block (API index 2) must capture its full streamed input.
    // Pre-fix, the misalignment routed its input_json_deltas to the wrong
    // slot, yielding empty/garbled arguments.
    assert_eq!(
        response.tool_calls.len(),
        1,
        "expected exactly one tool call"
    );
    let call = &response.tool_calls[0];
    assert_eq!(call.id, "tool_abc");
    assert_eq!(call.name, "search");
    assert_eq!(
        call.input,
        serde_json::json!({"q": "rust"}),
        "tool_use input (index 2) must reassemble after the unknown block at index 0"
    );

    // The unknown block carries no emittable content and is dropped from the
    // final response — only the text + tool_use blocks survive.
    assert_eq!(
        response.content.len(),
        2,
        "unknown block must be dropped, leaving only text + tool_use; got {:?}",
        response.content
    );

    // --- Streamed events ------------------------------------------------
    // The text deltas must have been emitted to the consumer in order.
    let streamed_text: String = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        streamed_text, "Hello world",
        "streamed text deltas, got {events:?}"
    );

    // The tool_use start/end must reference the index-2 block, not a shifted
    // slot.
    let saw_tool_end = events.iter().any(|e| {
        matches!(
            e,
            StreamEvent::ToolUseEnd { id, name, input }
                if id == "tool_abc"
                    && name == "search"
                    && *input == serde_json::json!({"q": "rust"})
        )
    });
    assert!(
        saw_tool_end,
        "expected a ToolUseEnd for the index-2 tool_use, got {events:?}"
    );
}
