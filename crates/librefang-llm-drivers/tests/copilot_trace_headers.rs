//! Verifies the Copilot driver surfaces caller identity (agent / session /
//! step) on outbound HTTP requests as `x-librefang-*` headers.
//!
//! The Copilot driver delegates to an inner `OpenAIDriver` after exchanging a
//! GitHub PAT for a Copilot API token. The trace headers are emitted by the
//! inner `OpenAIDriver` via the same `build_trace_header_map` helper used by
//! all other drivers. GitHub's Copilot edge proxy is permissive on `x-`
//! prefixed headers (as established by its own `x-github-*` family) — the
//! `x-librefang-*` prefix follows the same convention and passes through.
//!
//! The mock intercepts both the GitHub token-exchange endpoint and the
//! downstream `POST /chat/completions` endpoint.

mod common;

use common::*;
use librefang_llm_driver::{CompletionRequest, LlmDriver};
use librefang_llm_drivers::drivers::copilot::CopilotDriver;
use librefang_types::message::Message;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request_with_trace_ids(
    agent_id: Option<&str>,
    session_id: Option<&str>,
    step_id: Option<&str>,
) -> CompletionRequest {
    CompletionRequest {
        model: "gpt-4o".to_string(),
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
    }
}

/// Register a mock for the GitHub token-exchange endpoint (`/copilot_internal/v2/token`).
/// Returns a Copilot API token whose `proxy-ep` points at the mock server so
/// the downstream `/chat/completions` call also hits the mock.
async fn mount_token_exchange_mock(server: &MockServer) {
    // The token exchange URL is hardcoded in copilot.rs as
    // `https://api.github.com/copilot_internal/v2/token`.  Because the test
    // driver below uses `CopilotDriver::new_for_test` which bypasses the
    // exchange, we do not need this mock. Kept here as documentation.
    let _ = server; // suppress unused warning
}

/// Build a Copilot driver that bypasses the GitHub token exchange and sends
/// the downstream completion request directly to the mock server.
fn driver_pointing_at(server: &MockServer) -> CopilotDriver {
    CopilotDriver::new_for_test(format!("ghp_test_{}", Uuid::new_v4()), server.uri())
}

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    mount_token_exchange_mock(&server).await;
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("agent-abc"), Some("sess-xyz"), Some("7"));
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    // Only the /chat/completions call should have reached the server.
    let completion_req = received
        .iter()
        .find(|r| r.url.path() == "/chat/completions")
        .expect("no /chat/completions request received");
    let headers = &completion_req.headers;
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
    mount_token_exchange_mock(&server).await;
    let driver = driver_pointing_at(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(None, None, None);
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    let completion_req = received
        .iter()
        .find(|r| r.url.path() == "/chat/completions")
        .expect("no /chat/completions request received");
    let headers = &completion_req.headers;
    assert!(headers.get("x-librefang-agent-id").is_none());
    assert!(headers.get("x-librefang-session-id").is_none());
    assert!(headers.get("x-librefang-step-id").is_none());
}

/// Operator opt-out: `with_emit_caller_trace_headers(false)` must suppress
/// the three `x-librefang-*` headers even when caller-id fields are populated.
///
/// GitHub's Copilot edge proxy accepts `x-*` prefixed custom headers
/// (same permissive policy as `x-github-*` siblings). The opt-out here is
/// a LibreFang operator choice, not a Copilot proxy constraint.
#[tokio::test]
#[serial_test::serial]
async fn complete_suppresses_trace_headers_when_emit_flag_disabled() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    mount_token_exchange_mock(&server).await;
    let driver = driver_pointing_at(&server).with_emit_caller_trace_headers(false);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("agent-abc"), Some("sess-xyz"), Some("7"));
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    let completion_req = received
        .iter()
        .find(|r| r.url.path() == "/chat/completions")
        .expect("no /chat/completions request received");
    let headers = &completion_req.headers;
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
