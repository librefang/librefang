mod common;

use common::*;
use librefang_llm_driver::{LlmDriver, LlmError, StreamEvent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn request_json(request: &Request) -> serde_json::Value {
    serde_json::from_slice(&request.body).expect("request body should be valid JSON")
}

async fn collect_stream(
    driver: &librefang_llm_drivers::drivers::openai::OpenAIDriver,
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

fn openai_400_stream_options_rejected() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": "Unrecognized request argument: stream_options",
            "type": "invalid_request_error",
            "param": "stream_options",
            "code": "unsupported_parameter"
        }
    }))
}

#[tokio::test]
#[serial_test::serial]
async fn os1_429_retry_then_success_stream() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_429_response(1))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_sse_body(&["hello", " world"]))
        .with_priority(2)
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&driver, simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);

    let has_text_delta = events
        .iter()
        .any(|e| matches!(e, StreamEvent::TextDelta { .. }));
    assert!(has_text_delta, "expected TextDelta event, got {:?}", events);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3, "expected 3 requests (2x429 + 1x200)");
}

#[tokio::test]
#[serial_test::serial]
async fn os2_stream_options_strip() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_400_stream_options_rejected())
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_sse_body(&["hello"]))
        .with_priority(2)
        .mount(&server)
        .await;

    let (result, _events) = collect_stream(&driver, simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests (1x400 + 1x200)");

    let first = request_json(&requests[0]);
    let second = request_json(&requests[1]);
    assert!(
        first.get("stream_options").is_some(),
        "first request should include stream_options: {first}"
    );
    assert!(
        second.get("stream_options").is_none(),
        "retry request should omit stream_options: {second}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn os3_429_exhaustion_stream() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_429_response(1))
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&driver, simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited, got {:?}",
        result
    );
    assert!(events.is_empty(), "expected no events, got {:?}", events);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        4,
        "expected 4 requests (initial + 3 retries)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn os4_temperature_strip_stream() {
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
        .respond_with(openai_sse_body(&["hello"]))
        .with_priority(2)
        .mount(&server)
        .await;

    let (result, _events) =
        collect_stream(&driver, request_with_temperature("gpt-test", 0.7)).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "expected 2 requests (1x400 + 1x200)");

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
