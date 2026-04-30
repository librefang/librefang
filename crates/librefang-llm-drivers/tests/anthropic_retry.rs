mod common;

use common::{
    anthropic_200_body, anthropic_sse_body, isolated_env, lockout_file_exists, simple_request,
};
use librefang_llm_driver::{LlmDriver, LlmError, StreamEvent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct DriverWithKey {
    driver: librefang_llm_drivers::drivers::anthropic::AnthropicDriver,
    api_key: String,
}

fn driver_with_key(server: &MockServer) -> DriverWithKey {
    let api_key = format!("sk-ant-test-{}", uuid::Uuid::new_v4());
    let driver = librefang_llm_drivers::drivers::anthropic::AnthropicDriver::with_proxy_and_timeout(
        api_key.clone(),
        server.uri(),
        None,
        Some(5),
    );
    DriverWithKey { driver, api_key }
}

async fn collect_stream(
    driver: &librefang_llm_drivers::drivers::anthropic::AnthropicDriver,
    request: librefang_llm_driver::CompletionRequest,
) -> (
    Result<librefang_llm_driver::CompletionResponse, LlmError>,
    Vec<StreamEvent>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let handle = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
        events
    });
    let result = driver.stream(request, tx).await;
    let events = handle.await.unwrap();
    (result, events)
}

fn anthropic_429_no_retry_after() -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("retry-after", "1")
        .insert_header("anthropic-ratelimit-requests-limit", "1000")
        .insert_header("anthropic-ratelimit-requests-remaining", "0")
        .insert_header("anthropic-ratelimit-requests-reset", "30")
        .set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "rate_limit_error", "message": "rate limited"}
        }))
}

fn anthropic_529_no_retry_after() -> ResponseTemplate {
    ResponseTemplate::new(529).set_body_json(serde_json::json!({
        "type": "error",
        "error": {"type": "overloaded_error", "message": "Overloaded"}
    }))
}

#[tokio::test]
#[serial_test::serial]
async fn aa1_429_retry_then_success() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let dk = driver_with_key(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_429_no_retry_after())
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("hello")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = dk.driver.complete(simple_request("claude-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    assert!(
        lockout_file_exists("anthropic", &dk.api_key),
        "lockout file should exist after 429"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn aa2_429_exhaustion() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let dk = driver_with_key(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_429_no_retry_after())
        .up_to_n_times(4)
        .mount(&server)
        .await;

    let result = dk.driver.complete(simple_request("claude-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited, got {:?}",
        result
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
    assert!(
        lockout_file_exists("anthropic", &dk.api_key),
        "lockout file should exist after 429 exhaustion"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn aa3_529_retry_then_success_no_lockout() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let dk = driver_with_key(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_529_no_retry_after())
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_200_body("hello")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = dk.driver.complete(simple_request("claude-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(
        !lockout_file_exists("anthropic", &dk.api_key),
        "lockout file must NOT exist after 529 (overloaded is not account-level rate limit)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn aa4_529_exhaustion_overloaded() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let dk = driver_with_key(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_529_no_retry_after())
        .up_to_n_times(4)
        .mount(&server)
        .await;

    let result = dk.driver.complete(simple_request("claude-test")).await;
    assert!(
        matches!(result, Err(LlmError::Overloaded { .. })),
        "expected Overloaded, got {:?}",
        result
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
#[serial_test::serial]
async fn aa5_stream_429_retry() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let dk = driver_with_key(&server);

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_429_no_retry_after())
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse_body("hello"))
        .with_priority(2)
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&dk.driver, simple_request("claude-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert!(!events.is_empty(), "stream events should not be empty");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}
