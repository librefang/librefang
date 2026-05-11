//! Verifies the Bedrock driver surfaces caller identity (agent / session /
//! step) on outbound HTTP requests as `x-librefang-*` headers.
//!
//! The Bedrock driver uses simple Bearer token auth (not SigV4 canonical
//! signing), so trace headers are appended alongside `Authorization: Bearer`
//! without any signing-scope concern. The negative test (emit flag disabled)
//! also acts as a SigV4-compatibility regression gate: if the driver is ever
//! migrated to SigV4, a future change should verify that unsigned custom
//! headers are excluded from the canonical-request hash — but that migration
//! has not happened.

mod common;

use common::*;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_llm_drivers::drivers::bedrock::BedrockDriver;
use librefang_types::message::Message;
use uuid::Uuid;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bedrock_200_body() -> serde_json::Value {
    serde_json::json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{"text": "hello"}]
            }
        },
        "stopReason": "end_turn",
        "usage": {
            "inputTokens": 5,
            "outputTokens": 3
        }
    })
}

fn request_with_trace_ids(
    agent_id: Option<&str>,
    session_id: Option<&str>,
    step_id: Option<&str>,
) -> CompletionRequest {
    CompletionRequest {
        model: "anthropic.claude-3-haiku-20240307-v1:0".to_string(),
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

fn driver_pointing_at(server: &MockServer) -> BedrockDriver {
    BedrockDriver::new_for_test(
        format!("test-bedrock-token-{}", Uuid::new_v4()),
        server.uri(),
    )
}

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path_regex(r"^/model/.*/converse$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(bedrock_200_body()))
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
        .and(path_regex(r"^/model/.*/converse$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(bedrock_200_body()))
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

/// Operator opt-out: `emit_caller_trace_headers(false)` must suppress the
/// three `x-librefang-*` headers even when caller-id fields are populated.
///
/// SigV4 compatibility gate: because Bedrock uses Bearer token auth (not
/// SigV4), headers added via `.headers(map)` travel outside any signing
/// scope. This test confirms the flag is honoured at the reqwest layer.
/// If the driver is ever migrated to SigV4 signing, a regression test
/// should be added to assert that trace headers are NOT included in the
/// canonical-request hash (unsigned pass-through headers only).
#[tokio::test]
#[serial_test::serial]
async fn complete_suppresses_trace_headers_when_emit_flag_disabled() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = driver_pointing_at(&server).with_emit_caller_trace_headers(false);

    Mock::given(method("POST"))
        .and(path_regex(r"^/model/.*/converse$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(bedrock_200_body()))
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
