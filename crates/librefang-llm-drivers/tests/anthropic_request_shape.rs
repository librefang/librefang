//! Integration tests for the Anthropic driver covering request shape,
//! tool-call response parsing, and streaming aggregation (#3696).
//!
//! These tests intentionally do NOT overlap with `anthropic_retry.rs`, which
//! exercises 429 / 529 / Retry-After retry behaviour. Here we lock in the
//! provider wire contract:
//!
//! 1. Request body & headers (model, max_tokens, system, tools, x-api-key,
//!    anthropic-version) are present and well-formed.
//! 2. A non-streaming response with a `tool_use` content block is parsed
//!    into `CompletionResponse.tool_calls` with `StopReason::ToolUse`.
//! 3. A streaming SSE response yields `TextDelta` events whose concatenation
//!    matches the final `CompletionResponse.text()`.

mod common;

use common::{
    anthropic_200_body, anthropic_sse_body, collect_stream, isolated_env, mock_anthropic_driver,
    request_json, request_with_tools, simple_request,
};
use librefang_llm_driver::{LlmDriver, StreamEvent};
use librefang_types::message::StopReason;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The driver POSTs to `/v1/messages` and the request body carries the
/// caller-supplied model, max_tokens, system prompt, and tool list. Required
/// headers (`x-api-key`, `anthropic-version`) are present.
#[tokio::test]
#[serial_test::serial]
async fn request_shape_includes_model_system_tools_and_required_headers() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("ok")))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_anthropic_driver(&server);
    let mut req = request_with_tools("claude-3-5-sonnet-20241022");
    req.system = Some("You are a helpful assistant.".to_string());

    let resp = driver.complete(req).await.expect("complete should succeed");
    assert_eq!(resp.text(), "ok");

    // Inspect the recorded request to lock in the wire contract.
    let received = &server.received_requests().await.expect("requests")[0];

    // Headers — the daemon will not work without these.
    let api_key = received
        .headers
        .get("x-api-key")
        .expect("x-api-key header must be set");
    assert!(api_key.to_str().unwrap().starts_with("sk-ant-test-"));
    assert_eq!(
        received
            .headers
            .get("anthropic-version")
            .expect("anthropic-version header must be set")
            .to_str()
            .unwrap(),
        "2023-06-01"
    );
    assert_eq!(
        received
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap()),
        Some("application/json")
    );

    // Body shape.
    let body = request_json(received);
    assert_eq!(body["model"], "claude-3-5-sonnet-20241022");
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["system"], "You are a helpful assistant.");
    assert!(
        body["messages"].as_array().is_some_and(|m| !m.is_empty()),
        "messages array must be populated"
    );
    let tools = body["tools"]
        .as_array()
        .expect("tools must serialise as a JSON array");
    assert_eq!(tools.len(), 1, "single tool was registered");
    assert_eq!(tools[0]["name"], "get_weather");
    assert!(
        tools[0]["input_schema"].is_object(),
        "input_schema must be a JSON object"
    );
}

/// A non-streaming `tool_use` response parses into a `ToolCall` with the
/// declared id / name / input, and `StopReason::ToolUse` propagates through.
/// This is the path the agent loop takes when the LLM asks to invoke a tool.
#[tokio::test]
#[serial_test::serial]
async fn tool_use_response_parses_into_tool_call_with_input() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "id": "msg_tool",
        "type": "message",
        "role": "assistant",
        "model": "claude-test",
        "stop_reason": "tool_use",
        "content": [
            {"type": "text", "text": "Let me check the weather."},
            {
                "type": "tool_use",
                "id": "toolu_abc123",
                "name": "get_weather",
                "input": {"location": "San Francisco"}
            }
        ],
        "usage": {"input_tokens": 12, "output_tokens": 7}
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_anthropic_driver(&server);
    let resp = driver
        .complete(request_with_tools("claude-3-5-sonnet-20241022"))
        .await
        .expect("complete should succeed");

    assert_eq!(
        resp.stop_reason,
        StopReason::ToolUse,
        "stop_reason must surface as ToolUse so the agent loop dispatches the tool"
    );
    assert_eq!(resp.text(), "Let me check the weather.");

    assert_eq!(
        resp.tool_calls.len(),
        1,
        "exactly one tool_call should have been parsed"
    );
    let call = &resp.tool_calls[0];
    assert_eq!(call.id, "toolu_abc123");
    assert_eq!(call.name, "get_weather");
    assert_eq!(call.input["location"], "San Francisco");

    // Usage accounting must propagate so the metering layer can charge.
    assert_eq!(resp.usage.input_tokens, 12);
    assert_eq!(resp.usage.output_tokens, 7);
}

/// Streaming SSE responses yield `TextDelta` events whose concatenation
/// matches the final `CompletionResponse.text()`. Locks in the contract that
/// streaming and non-streaming produce the same final text — a regression
/// here would split the user-visible UX (dashboard streaming vs. REST) from
/// the cost / history persistence path.
#[tokio::test]
#[serial_test::serial]
async fn streaming_sse_aggregates_text_deltas_into_final_response() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let text = "hello world";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse_body(text))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_anthropic_driver(&server);
    let req = simple_request("claude-3-5-sonnet-20241022");

    let (result, events) = collect_stream(&driver, req).await;
    let resp = result.expect("stream should succeed");

    // Final response carries the full text.
    assert_eq!(resp.text(), text);
    assert_eq!(resp.stop_reason, StopReason::EndTurn);

    // Concatenated TextDelta events must match the final text. This is the
    // invariant the dashboard relies on — the streamed deltas the user sees
    // and the final persisted text are the same string.
    let streamed: String = events
        .iter()
        .filter_map(|ev| match ev {
            StreamEvent::TextDelta { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, text);

    // A terminal `ContentComplete` event must close the stream so the agent
    // loop knows to advance.
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::ContentComplete { .. })),
        "stream must terminate with ContentComplete"
    );
}
