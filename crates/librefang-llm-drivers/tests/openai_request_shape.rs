//! Integration tests for the OpenAI driver covering request shape,
//! tool-call response parsing, and streaming aggregation (#3696).
//!
//! Distinct from `openai_retry_complete.rs` / `openai_retry_stream.rs` which
//! exercise 429 / 503 / Retry-After retry behaviour. Here we lock in the
//! provider wire contract:
//!
//! 1. Request body & headers (model, max_tokens, messages, Authorization).
//! 2. A non-streaming response with `tool_calls` is parsed into
//!    `CompletionResponse.tool_calls` with `StopReason::ToolUse`.
//! 3. A streaming SSE response yields `TextDelta` events whose concatenation
//!    matches the final `CompletionResponse.text()`.

mod common;

use common::{
    collect_stream, isolated_env, mock_openai_driver, openai_200_body, openai_sse_body,
    request_json, request_with_tools, simple_request,
};
use librefang_llm_driver::{LlmDriver, StreamEvent};
use librefang_types::message::StopReason;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The driver POSTs to `/chat/completions` with `Authorization: Bearer <key>`,
/// and the request body carries the caller-supplied model, messages, and
/// tool list (when present).
#[tokio::test]
#[serial_test::serial]
async fn request_shape_includes_model_messages_tools_and_auth_header() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_openai_driver(&server);
    let resp = driver
        .complete(request_with_tools("gpt-test"))
        .await
        .expect("complete should succeed");
    assert_eq!(resp.text(), "ok");

    let received = &server.received_requests().await.expect("requests")[0];

    // Bearer auth header is mandatory — the driver must not silently skip it
    // when the api_key is non-empty.
    let auth = received
        .headers
        .get("authorization")
        .expect("authorization header must be set")
        .to_str()
        .unwrap();
    assert!(
        auth.starts_with("Bearer sk-test-"),
        "expected Bearer prefix, got {auth:?}"
    );
    assert_eq!(
        received
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("application/json")
    );

    let body = request_json(received);
    assert_eq!(body["model"], "gpt-test");
    let messages = body["messages"]
        .as_array()
        .expect("messages must serialise as a JSON array");
    assert!(!messages.is_empty(), "messages array must be populated");
    // Tools serialise via the OpenAI `[{type: "function", function: {…}}]`
    // envelope — this regression-checks that the tool definitions actually
    // make it into the request rather than being silently dropped.
    let tools = body["tools"]
        .as_array()
        .expect("tools must serialise as a JSON array when ToolDefinitions are present");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "get_weather");
}

/// A non-streaming response with a `tool_calls` entry parses into a
/// `ToolCall` with the declared id / name / input. `StopReason::ToolUse`
/// propagates so the agent loop dispatches the tool.
#[tokio::test]
#[serial_test::serial]
async fn tool_calls_response_parses_into_tool_call_with_input() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "id": "chatcmpl-tool",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_xyz",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\": \"Paris\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 8,
            "completion_tokens": 4,
            "total_tokens": 12
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_openai_driver(&server);
    let resp = driver
        .complete(request_with_tools("gpt-test"))
        .await
        .expect("complete should succeed");

    assert_eq!(
        resp.stop_reason,
        StopReason::ToolUse,
        "finish_reason=tool_calls must map to StopReason::ToolUse"
    );
    assert_eq!(
        resp.tool_calls.len(),
        1,
        "exactly one tool_call should have been parsed"
    );
    let call = &resp.tool_calls[0];
    assert_eq!(call.id, "call_xyz");
    assert_eq!(call.name, "get_weather");
    // OpenAI sends arguments as a JSON-encoded string; the driver must
    // decode it into a structured value before handing to the agent loop.
    assert_eq!(call.input["location"], "Paris");

    assert_eq!(resp.usage.input_tokens, 8);
    assert_eq!(resp.usage.output_tokens, 4);
}

/// Streaming SSE deltas concatenate into the same final text as
/// `CompletionResponse.text()`. Locks in the contract that streaming and
/// non-streaming UX produce identical user-visible output.
#[tokio::test]
#[serial_test::serial]
async fn streaming_sse_aggregates_text_deltas_into_final_response() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let chunks = ["hel", "lo ", "world"];
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_sse_body(&chunks))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_openai_driver(&server);
    let req = simple_request("gpt-test");
    let (result, events) = collect_stream(&driver, req).await;
    let resp = result.expect("stream should succeed");

    let expected = chunks.concat();
    assert_eq!(resp.text(), expected);

    let streamed: String = events
        .iter()
        .filter_map(|ev| match ev {
            StreamEvent::TextDelta { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, expected);

    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::ContentComplete { .. })),
        "stream must terminate with ContentComplete"
    );
}
