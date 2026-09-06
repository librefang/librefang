//! Regression: extended thinking has to survive a full turn boundary.
//!
//! With thinking enabled, Anthropic requires the assistant turn that precedes a set of tool results to start with the thinking blocks it originally produced, echoed back unchanged together with the `signature` that authenticates them.
//! The driver used to fail both halves of that: the SSE parser dropped every `signature_delta` frame, and `convert_message` filtered thinking blocks out of the replayed history entirely — so an agent with thinking on could not complete a single tool round trip, the second request dying with `400 invalid_request_error`.
//!
//! This drives the real round trip: stream a signed thinking block plus a `tool_use`, feed the assembled turn back in as history, and inspect the body the driver puts on the wire the second time.

mod common;

use common::*;
use librefang_types::config::ThinkingConfig;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Stream layout, mirroring what a thinking model emits before a tool call:
///   index 0 → thinking block, text in `thinking_delta`s, signature split
///             across two `signature_delta`s
///   index 1 → tool_use block with `input_json_delta`s forming {"q":"rust"}
fn anthropic_sse_with_signed_thinking() -> ResponseTemplate {
    let mut body = String::new();

    let mut push = |event: &str, data: serde_json::Value| {
        body.push_str(&format!("event: {event}\ndata: {data}\n\n"));
    };

    push(
        "message_start",
        serde_json::json!({
            "type": "message_start",
            "message": {
                "id": "msg_thinking_test",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-test",
                "usage": {"input_tokens": 5, "output_tokens": 0}
            }
        }),
    );

    push(
        "content_block_start",
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "thinking", "thinking": "", "signature": ""}
        }),
    );
    for piece in ["Let me ", "check that."] {
        push(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "thinking_delta", "thinking": piece}
            }),
        );
    }
    // Anthropic chunks the signature like any other delta; the driver has to concatenate them, not take the last one.
    for piece in ["EqoBCkgIA", "RACGAIqSDk="] {
        push(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "signature_delta", "signature": piece}
            }),
        );
    }
    push(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 0}),
    );

    push(
        "content_block_start",
        serde_json::json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {"type": "tool_use", "id": "tool_abc", "name": "search"}
        }),
    );
    for piece in [r#"{"q":"#, r#""rust"}"#] {
        push(
            "content_block_delta",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": piece}
            }),
        );
    }
    push(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 1}),
    );

    push(
        "message_delta",
        serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 12}
        }),
    );
    push("message_stop", serde_json::json!({"type": "message_stop"}));

    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

#[tokio::test]
#[serial_test::serial]
async fn streamed_thinking_signature_is_captured_and_replayed_verbatim() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_anthropic_driver(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse_with_signed_thinking())
        .mount(&server)
        .await;

    let mut first = request_with_tools("claude-opus-4-6");
    first.thinking = Some(ThinkingConfig {
        budget_tokens: 8192,
        ..Default::default()
    });
    let (result, _events) = collect_stream(&driver, first).await;
    let response = result.expect("stream should succeed");

    // --- Half one: the signature must come off the wire --------------------
    let (thinking, signature) = response
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Thinking {
                thinking,
                provider_metadata,
            } => Some((
                thinking.clone(),
                provider_metadata
                    .as_ref()
                    .and_then(|m| m.get("signature"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            )),
            _ => None,
        })
        .expect("the assembled response must carry a thinking block");
    assert_eq!(thinking, "Let me check that.");
    assert_eq!(
        signature, "EqoBCkgIARACGAIqSDk=",
        "both signature_delta frames must be concatenated onto the block",
    );

    // --- Half two: the stored turn must go back out unchanged --------------
    let mut second = request_with_tools("claude-opus-4-6");
    second.thinking = Some(ThinkingConfig {
        budget_tokens: 8192,
        ..Default::default()
    });
    second.messages = std::sync::Arc::new(vec![
        Message::user("find something"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(response.content.clone()),
            pinned: false,
            timestamp: None,
        },
        Message::user_with_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "tool_abc".to_string(),
            tool_name: "search".to_string(),
            content: "results".to_string(),
            is_error: false,
            status: Default::default(),
            approval_request_id: None,
        }]),
    ]);
    let (result, _events) = collect_stream(&driver, second).await;
    result.expect("second stream should succeed");

    let received = server.received_requests().await.expect("requests");
    let body = request_json(received.last().expect("second request recorded"));
    let assistant_blocks = body["messages"][1]["content"]
        .as_array()
        .expect("the assistant turn must be sent in block form");
    assert_eq!(
        assistant_blocks[0],
        serde_json::json!({
            "type": "thinking",
            "thinking": "Let me check that.",
            "signature": "EqoBCkgIARACGAIqSDk="
        }),
        "the assistant turn must start with its thinking block, echoed unchanged; got {assistant_blocks:?}",
    );
    assert_eq!(
        assistant_blocks[1]["type"], "tool_use",
        "the tool_use block must still follow it",
    );

    // The turn that produced those blocks asked for thinking on a model that takes the adaptive form, so the removed budget spelling must not appear alongside them.
    assert_eq!(body["thinking"], serde_json::json!({"type": "adaptive"}));
    // No `reasoning_mode` was set, so no rung is invented and the API's own default effort applies.
    assert!(
        body.get("output_config").is_none(),
        "an unconfigured depth must be left to the API default, got {body}",
    );
    // The budget the request still carries governs nothing but the on/off decision on this wire.
    // The old code folded it into the ceiling here, sending `max_tokens: 9216` in place of the caller's 256.
    assert_eq!(
        body["max_tokens"], 256,
        "the adaptive form has no budget for max_tokens to clear, got {body}",
    );
}
