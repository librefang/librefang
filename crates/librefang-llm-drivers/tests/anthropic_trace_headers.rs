//! Verifies the Anthropic driver surfaces caller identity (agent / session /
//! step) on outbound HTTP requests as `x-librefang-*` headers, so any
//! observability sidecar in front of the Anthropic API can correlate request
//! log records without parsing the JSON body. Mirrors the pattern established
//! by `openai_trace_headers.rs` (#4548) — see that file for full rationale.

mod common;

use common::*;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_types::message::Message;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request_with_trace_ids(
    agent_id: Option<&str>,
    session_id: Option<&str>,
    step_id: Option<&str>,
) -> CompletionRequest {
    CompletionRequest {
        model: "claude-test".to_string(),
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

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_anthropic_driver(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("ok")))
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
    let driver = mock_anthropic_driver(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("ok")))
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
/// through to `AnthropicDriver::with_emit_caller_trace_headers(false)`,
/// the three `x-librefang-*` headers must NOT appear on the wire even when
/// the per-request caller-id fields ARE populated.
#[tokio::test]
#[serial_test::serial]
async fn complete_suppresses_trace_headers_when_emit_flag_disabled() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    use librefang_llm_drivers::drivers::anthropic::AnthropicDriver;
    use uuid::Uuid;
    let driver = AnthropicDriver::with_proxy_and_timeout(
        format!("sk-ant-test-{}", Uuid::new_v4()),
        server.uri(),
        None,
        Some(5),
    )
    .with_emit_caller_trace_headers(false);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("ok")))
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

#[tokio::test]
#[serial_test::serial]
async fn stream_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_anthropic_driver(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse_body("hi"))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("agent-stream"), Some("sess-stream"), Some("3"));
    let (result, _events) = collect_stream(&driver, req).await;
    assert!(result.is_ok(), "stream should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    assert_eq!(
        headers
            .get("x-librefang-agent-id")
            .map(|v| v.to_str().unwrap()),
        Some("agent-stream"),
    );
    assert_eq!(
        headers
            .get("x-librefang-session-id")
            .map(|v| v.to_str().unwrap()),
        Some("sess-stream"),
    );
    assert_eq!(
        headers
            .get("x-librefang-step-id")
            .map(|v| v.to_str().unwrap()),
        Some("3"),
    );
}
