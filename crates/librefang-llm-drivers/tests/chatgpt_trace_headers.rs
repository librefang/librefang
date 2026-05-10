//! Verifies the ChatGPT (Responses API) driver surfaces caller identity
//! (agent / session / step) on outbound HTTP requests as `x-librefang-*`
//! headers, so any observability sidecar in front of the ChatGPT API can
//! correlate request log records without parsing the JSON body. Mirrors the
//! pattern established by `openai_trace_headers.rs` (#4548) — see that file
//! for full rationale.

mod common;

use common::*;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_llm_drivers::drivers::chatgpt::ChatGptDriver;
use librefang_types::message::Message;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request_with_trace_ids(
    agent_id: Option<&str>,
    session_id: Option<&str>,
    step_id: Option<&str>,
) -> CompletionRequest {
    CompletionRequest {
        model: "chatgpt-test".to_string(),
        messages: std::sync::Arc::new(vec![Message::user("hello")]),
        tools: std::sync::Arc::new(Vec::new()),
        max_tokens: 16,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: agent_id.map(String::from),
        session_id: session_id.map(String::from),
        step_id: step_id.map(String::from),
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    }
}

/// Minimal SSE body the ChatGPT Responses API driver accepts.
/// The driver's `stream_sse` parser requires a `response.completed` event
/// to build a valid `CompletionResponse`; without it the driver returns an
/// `LlmError::Api` with "No response.completed event".
fn chatgpt_sse_body(text: &str) -> ResponseTemplate {
    let delta = serde_json::json!({
        "type": "response.output_text.delta",
        "delta": text,
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "status": "completed",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": text}]
            }],
            "usage": {"input_tokens": 5, "output_tokens": 3}
        }
    });
    let body = format!("data: {delta}\n\ndata: {completed}\n\ndata: [DONE]\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn mock_chatgpt_driver(server: &MockServer) -> ChatGptDriver {
    // ChatGPT driver authenticates via OAuth bearer; the session_token field
    // is used directly as the bearer until the API rejects it. We pass a
    // synthetic non-empty token so `ensure_token()` doesn't error before
    // the request fires — the wiremock server accepts any bearer value.
    ChatGptDriver::with_proxy("test-chatgpt-session-token".to_string(), server.uri(), None)
}

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_chatgpt_driver(&server);

    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(chatgpt_sse_body("ok"))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("agent-abc"), Some("sess-xyz"), Some("7"));
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    assert_eq!(
        headers
            .get("x-librefang-agent-id")
            .map(|v| v.to_str().unwrap()),
        Some("agent-abc"),
    );
    assert_eq!(
        headers
            .get("x-librefang-session-id")
            .map(|v| v.to_str().unwrap()),
        Some("sess-xyz"),
    );
    assert_eq!(
        headers
            .get("x-librefang-step-id")
            .map(|v| v.to_str().unwrap()),
        Some("7"),
    );
}

#[tokio::test]
#[serial_test::serial]
async fn complete_omits_trace_headers_when_ids_absent() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_chatgpt_driver(&server);

    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(chatgpt_sse_body("ok"))
        .mount(&server)
        .await;

    // All three IDs are None — driver must not emit any of the headers.
    let req = request_with_trace_ids(None, None, None);
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    assert!(headers.get("x-librefang-agent-id").is_none());
    assert!(headers.get("x-librefang-session-id").is_none());
    assert!(headers.get("x-librefang-step-id").is_none());
}

/// Operator opt-out: when `emit_caller_trace_headers = false` is plumbed
/// through to `ChatGptDriver::with_emit_caller_trace_headers(false)`,
/// the three `x-librefang-*` headers must NOT appear on the wire even when
/// the per-request caller-id fields ARE populated.
#[tokio::test]
#[serial_test::serial]
async fn complete_suppresses_trace_headers_when_emit_flag_disabled() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_chatgpt_driver(&server).with_emit_caller_trace_headers(false);

    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(chatgpt_sse_body("ok"))
        .mount(&server)
        .await;

    // Caller-id fields are populated; the driver-level opt-out must still suppress.
    let req = request_with_trace_ids(Some("agent-abc"), Some("sess-xyz"), Some("7"));
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    assert!(
        headers.get("x-librefang-agent-id").is_none(),
        "agent-id header must not appear when emit flag is false",
    );
    assert!(
        headers.get("x-librefang-session-id").is_none(),
        "session-id header must not appear when emit flag is false",
    );
    assert!(
        headers.get("x-librefang-step-id").is_none(),
        "step-id header must not appear when emit flag is false",
    );
}
