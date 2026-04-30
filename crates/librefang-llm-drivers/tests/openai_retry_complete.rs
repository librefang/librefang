mod common;

use common::*;
use librefang_llm_driver::{LlmDriver, LlmError};
use librefang_llm_drivers::drivers::openai::OpenAIDriver;
use std::time::{Duration, SystemTime};
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn request_json(request: &Request) -> serde_json::Value {
    serde_json::from_slice(&request.body).expect("request body should be valid JSON")
}

struct BodyContains(&'static str);

impl Match for BodyContains {
    fn matches(&self, request: &Request) -> bool {
        std::str::from_utf8(&request.body)
            .map(|s| s.contains(self.0))
            .unwrap_or(false)
    }
}

struct BodyNotContains(&'static str);

impl Match for BodyNotContains {
    fn matches(&self, request: &Request) -> bool {
        std::str::from_utf8(&request.body)
            .map(|s| !s.contains(self.0))
            .unwrap_or(false)
    }
}

#[tokio::test]
#[serial_test::serial]
async fn oc1_429_retry_then_success() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let api_key = "sk-test-oc1-fixed".to_string();
    let driver = OpenAIDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_429_response(1))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("recovered")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok after retry, got {:?}", result);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3, "expected 3 requests (2x429 + 1x200)");

    assert!(
        lockout_file_exists(provider_for_openai_mock(), &api_key),
        "lockout file should exist"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc2_429_exhaustion() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_429_response(1))
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        4,
        "expected 4 requests (max_retries=3 + initial)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc3_preexisting_lockout_blocks_request() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let api_key = "sk-test-oc3-fixed".to_string();
    let until = SystemTime::now() + Duration::from_secs(60);
    create_lockout_file(provider_for_openai_mock(), &api_key, until);

    let driver = OpenAIDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited from lockout, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 0, "no requests should reach the server");
}

#[tokio::test]
#[serial_test::serial]
async fn oc4_max_tokens_to_max_completion_tokens() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    let mut req = simple_request("my-custom-model");
    req.max_tokens = 1000;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(BodyContains("max_tokens"))
        .respond_with(openai_400_max_tokens_unsupported())
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(BodyContains("max_completion_tokens"))
        .and(BodyNotContains("max_tokens"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("reasoned")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver.complete(req).await;
    assert!(
        result.is_ok(),
        "expected Ok after max_tokens switch, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests (1x400 + 1x200)");
}

#[tokio::test]
#[serial_test::serial]
async fn oc5_temperature_strip() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_400_temperature_rejected())
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("no-temp")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver
        .complete(request_with_temperature("gpt-test", 0.7))
        .await;
    assert!(
        result.is_ok(),
        "expected Ok after temperature strip, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests");

    let first = request_json(&requests[0]);
    let second = request_json(&requests[1]);
    assert!(
        first.get("temperature").is_some(),
        "first request should include temperature: {first}"
    );
    assert!(
        second.get("temperature").is_none(),
        "retry request should omit temperature: {second}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc6_toolless_retry_on_500() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_500_tool_error())
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("no-tools")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver.complete(request_with_tools("gpt-test")).await;
    assert!(
        result.is_ok(),
        "expected Ok after tool strip, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests");

    let first = request_json(&requests[0]);
    let second = request_json(&requests[1]);
    assert!(
        first
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "first request should include tools: {first}"
    );
    assert!(
        first.get("tool_choice").is_some(),
        "first request should include tool_choice: {first}"
    );
    assert!(
        second
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty),
        "retry request should omit tools or send empty tools: {second}"
    );
    assert!(
        second.get("tool_choice").is_none(),
        "retry request should omit tool_choice: {second}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc7_max_tokens_auto_cap() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    let mut req = simple_request("gpt-test");
    req.max_tokens = 500;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "maximum value for `max_tokens` is `128`",
                "type": "invalid_request_error",
                "param": "max_tokens"
            }
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("capped")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver.complete(req).await;
    assert!(
        result.is_ok(),
        "expected Ok after auto-cap, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests");

    let first = request_json(&requests[0]);
    let second = request_json(&requests[1]);
    assert_eq!(
        first.get("max_tokens").and_then(serde_json::Value::as_u64),
        Some(500),
        "first request should send the original max_tokens: {first}"
    );
    let capped = second
        .get("max_tokens")
        .and_then(serde_json::Value::as_u64)
        .expect("retry request should include capped max_tokens");
    assert!(
        capped <= 128,
        "retry max_tokens should be capped to <= 128, got {capped}: {second}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc8_non_retryable_403() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": {"message": "Forbidden", "type": "permission_denied"}
        })))
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::Api { status: 403, .. })),
        "expected Api(403), got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "expected 1 request (no retry)");
}

#[tokio::test]
#[serial_test::serial]
async fn oc9_groq_tool_use_failed_retries() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_400_tool_use_failed())
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("recovered")))
        .with_priority(2)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        result.is_ok(),
        "expected Ok after tool_use_failed retry, got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        3,
        "expected 3 requests (2x tool_use_failed + 1x success)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn oc10_max_retries_exceeded_generic_500() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": {"message": "something went wrong", "type": "server_error"}
        })))
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::Api { status: 500, .. })),
        "expected Api(500), got {:?}",
        result
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "generic 500 without tools should not retry"
    );
}
