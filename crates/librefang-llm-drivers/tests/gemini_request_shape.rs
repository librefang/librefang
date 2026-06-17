//! Integration tests for the Gemini driver covering request shape,
//! tool-call response parsing, and streaming aggregation (#3696).
//!
//! Distinct from `gemini_retry.rs` which exercises 429 / 503 / Retry-After
//! retry behaviour. Here we lock in the provider wire contract:
//!
//! 1. Request body & URL (model embedded in path, `contents` / `generationConfig`,
//!    api key in the `x-goog-api-key` header — never in the query string).
//! 2. A non-streaming response with a `functionCall` part is parsed into
//!    `CompletionResponse.tool_calls` with `StopReason::ToolUse`.
//! 3. A streaming SSE response yields `TextDelta` events whose concatenation
//!    matches the final `CompletionResponse.text()`.

mod common;

use common::{
    collect_stream, gemini_sse_body, isolated_env, mock_gemini_driver, request_json,
    request_with_tools, simple_request,
};
use librefang_llm_driver::{LlmDriver, StreamEvent};
use librefang_types::message::StopReason;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The driver POSTs to `/v1beta/models/<model>:generateContent` with the
/// API key in the `x-goog-api-key` header (never in the query string) and
/// the request body carries the caller-supplied messages, tools, and
/// generationConfig.
#[tokio::test]
#[serial_test::serial]
async fn request_shape_targets_model_path_and_carries_contents_tools() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "ok"}]},
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_gemini_driver(&server);
    let resp = driver
        .complete(request_with_tools("gemini-2.0-flash"))
        .await
        .expect("complete should succeed");
    assert_eq!(resp.text(), "ok");

    let received = &server.received_requests().await.expect("requests")[0];

    // Gemini auth goes in the `x-goog-api-key` header, never in the URL.
    let api_key = received
        .headers
        .get("x-goog-api-key")
        .expect("x-goog-api-key header must be set")
        .to_str()
        .expect("api key header must be valid UTF-8");
    assert!(
        api_key.starts_with("test-key-"),
        "x-goog-api-key must carry the configured key, got {api_key}"
    );
    let url = received.url.to_string();
    assert!(
        !url.contains("key="),
        "api key must not leak into the URL query string, got {url}"
    );

    let body = request_json(received);
    // `contents` is the Gemini-flavoured message array.
    assert!(
        body["contents"].as_array().is_some_and(|c| !c.is_empty()),
        "contents array must be populated"
    );
    // Tools surface as `tools[0].functionDeclarations[]` in Gemini's wire
    // format. Lock that envelope in so a future driver refactor can't
    // silently drop the tool list.
    let tools = body["tools"]
        .as_array()
        .expect("tools must serialise as a JSON array");
    assert_eq!(tools.len(), 1);
    let decls = tools[0]["functionDeclarations"]
        .as_array()
        .expect("functionDeclarations must be an array");
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0]["name"], "get_weather");
    assert!(
        decls[0]["parameters"].is_object(),
        "tool parameters must be a JSON object"
    );
}

/// A non-streaming response with a `functionCall` part parses into a
/// `ToolCall` carrying the LLM's name + args. `StopReason::ToolUse` lets
/// the agent loop dispatch the tool.
#[tokio::test]
#[serial_test::serial]
async fn function_call_response_parses_into_tool_call_with_input() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let body = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"text": "Looking up weather."},
                    {
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "Tokyo"}
                        }
                    }
                ],
                "role": "model"
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 6}
    });

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_gemini_driver(&server);
    let resp = driver
        .complete(request_with_tools("gemini-2.0-flash"))
        .await
        .expect("complete should succeed");

    assert_eq!(
        resp.stop_reason,
        StopReason::ToolUse,
        "presence of a functionCall must surface as ToolUse"
    );
    assert_eq!(
        resp.tool_calls.len(),
        1,
        "exactly one tool_call should have been parsed"
    );
    let call = &resp.tool_calls[0];
    assert_eq!(call.name, "get_weather");
    assert_eq!(call.input["location"], "Tokyo");
    // Gemini does not natively expose a tool-use id; the driver must mint
    // one so the agent loop can correlate the result. Just assert a
    // non-empty id rather than the exact format (which is a driver impl
    // detail we don't want to pin).
    assert!(!call.id.is_empty(), "driver must mint a tool_use id");

    assert_eq!(resp.usage.input_tokens, 9);
    assert_eq!(resp.usage.output_tokens, 6);
}

/// Streaming SSE deltas concatenate into the same final text as
/// `CompletionResponse.text()`.
#[tokio::test]
#[serial_test::serial]
async fn streaming_sse_aggregates_text_deltas_into_final_response() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let text = "hello world";
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-2.0-flash:streamGenerateContent",
        ))
        .respond_with(gemini_sse_body(text))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_gemini_driver(&server);
    let req = simple_request("gemini-2.0-flash");
    let (result, events) = collect_stream(&driver, req).await;
    let resp = result.expect("stream should succeed");

    assert_eq!(resp.text(), text);

    let streamed: String = events
        .iter()
        .filter_map(|ev| match ev {
            StreamEvent::TextDelta { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, text);

    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::ContentComplete { .. })),
        "stream must terminate with ContentComplete"
    );
}

/// A streamed chunk carrying `cachedContentTokenCount` must populate
/// `cache_read_input_tokens` so metering can apply the cache-read rate.
/// Regression: the streaming per-chunk usage block previously set
/// `input_tokens` / `output_tokens` but dropped the cached-token field,
/// over-charging cached prompt tokens at the full input rate (matches the
/// `convert_response` / `stream_gemini_sse` non-streaming behaviour).
#[tokio::test]
#[serial_test::serial]
async fn streaming_usage_captures_cached_content_tokens() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let chunk = serde_json::json!({
        "candidates": [{
            "content": {"parts": [{"text": "ok"}]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 100,
            "candidatesTokenCount": 3,
            "cachedContentTokenCount": 80
        }
    });
    let body = format!("data: {chunk}\n\n");
    let sse = ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body);

    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-2.0-flash:streamGenerateContent",
        ))
        .respond_with(sse)
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_gemini_driver(&server);
    let req = simple_request("gemini-2.0-flash");
    let (result, _events) = collect_stream(&driver, req).await;
    let resp = result.expect("stream should succeed");

    assert_eq!(resp.usage.input_tokens, 100);
    assert_eq!(
        resp.usage.cache_read_input_tokens, 80,
        "cachedContentTokenCount from the streamed chunk must populate \
         cache_read_input_tokens"
    );
}
