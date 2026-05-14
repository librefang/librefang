//! Verifies the Vertex AI driver surfaces caller identity (agent / session /
//! step) on outbound HTTP requests as `x-librefang-*` headers. Mirrors the
//! pattern established by `gemini_trace_headers.rs` — Vertex AI uses the same
//! Gemini generateContent wire format with Google Cloud OAuth2 Bearer auth
//! instead of an API key.

mod common;

use common::*;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_llm_drivers::drivers::vertex_ai::VertexAiDriver;
use librefang_types::message::Message;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request_with_trace_ids(
    agent_id: Option<&str>,
    session_id: Option<&str>,
    step_id: Option<&str>,
) -> CompletionRequest {
    CompletionRequest {
        model: "vertex-ai/gemini-2.0-flash".to_string(),
        messages: std::sync::Arc::new(vec![Message::user("hello")]),
        tools: std::sync::Arc::new(Vec::new()),
        max_tokens: 16,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: agent_id.map(String::from),
        session_id: session_id.map(String::from),
        step_id: step_id.map(String::from),
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    }
}

/// Build a Vertex AI driver pointing at the mock server.
///
/// Vertex AI endpoint format:
/// `https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/google/models/{model}:generateContent`
///
/// We build a `DriverConfig` with a service-account JSON that has a token_uri
/// pointing at the mock server so the OAuth2 token exchange hits the mock, and
/// a project_id so URL resolution succeeds. We then register a token-exchange
/// mock that returns a fixed access token.
///
/// Alternatively — and more simply — we use `VertexAiDriver::new_for_test`
/// which accepts a pre-set access token and a base URL override, bypassing
/// the OAuth2 flow entirely. We add that constructor to `vertex_ai.rs` for
/// tests.
fn driver_pointing_at(server: &MockServer) -> VertexAiDriver {
    VertexAiDriver::new_for_test("test-access-token".to_string(), server.uri())
}

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path_regex(r":generateContent$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_200_body("ok")))
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
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path_regex(r":generateContent$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_200_body("ok")))
        .mount(&server)
        .await;

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

/// Operator opt-out: `with_emit_caller_trace_headers(false)` must suppress
/// the three `x-librefang-*` headers even when caller-id fields are populated.
#[tokio::test]
#[serial_test::serial]
async fn complete_suppresses_trace_headers_when_emit_flag_disabled() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = driver_pointing_at(&server).with_emit_caller_trace_headers(false);

    Mock::given(method("POST"))
        .and(path_regex(r":generateContent$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gemini_200_body("ok")))
        .mount(&server)
        .await;

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
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path_regex(r":streamGenerateContent"))
        .respond_with(gemini_sse_body("hi"))
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
