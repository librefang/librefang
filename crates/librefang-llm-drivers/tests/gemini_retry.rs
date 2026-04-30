mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use librefang_llm_driver::{LlmDriver, LlmError};
use librefang_llm_drivers::drivers::gemini::GeminiDriver;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use common::{
    collect_stream, gemini_200_body, gemini_sse_body, isolated_env, lockout_file_exists,
    simple_request,
};

struct SequencedResponder {
    responses: Vec<ResponseTemplate>,
    counter: Arc<AtomicUsize>,
}

impl SequencedResponder {
    fn new(responses: Vec<ResponseTemplate>) -> (Self, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let responder = Self {
            responses,
            counter: counter.clone(),
        };
        (responder, counter)
    }
}

impl Respond for SequencedResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let idx = self.counter.fetch_add(1, Ordering::SeqCst);
        self.responses[idx.min(self.responses.len() - 1)].clone()
    }
}

fn fast_gemini_429() -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("retry-after", "1")
        .insert_header("x-ratelimit-reset-requests-1h", "30")
        .set_body_json(serde_json::json!({
            "error": {
                "code": 429,
                "message": "Resource exhausted",
                "status": "RESOURCE_EXHAUSTED"
            }
        }))
}

fn fast_gemini_503() -> ResponseTemplate {
    ResponseTemplate::new(503)
        .insert_header("retry-after", "0")
        .set_body_json(serde_json::json!({
            "error": {
                "code": 503,
                "message": "The model is overloaded",
                "status": "UNAVAILABLE"
            }
        }))
}

fn gemini_403() -> ResponseTemplate {
    ResponseTemplate::new(403).set_body_json(serde_json::json!({
        "error": {
            "code": 403,
            "message": "API key not valid",
            "status": "PERMISSION_DENIED"
        }
    }))
}

#[tokio::test]
#[serial_test::serial]
async fn ag1_429_retry_then_success() {
    let server = MockServer::start().await;
    let api_key = "test-ag1-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![
        fast_gemini_429(),
        fast_gemini_429(),
        ResponseTemplate::new(200).set_body_json(gemini_200_body("retried ok")),
    ]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:generateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
    assert!(lockout_file_exists("gemini", &api_key));
}

#[tokio::test]
#[serial_test::serial]
async fn ag2_429_exhaustion() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let api_key = "test-ag2-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![
        fast_gemini_429(),
        fast_gemini_429(),
        fast_gemini_429(),
        fast_gemini_429(),
    ]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:generateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited, got {:?}",
        result
    );
    assert_eq!(counter.load(Ordering::SeqCst), 4);
}

#[tokio::test]
#[serial_test::serial]
async fn ag3_503_retry_then_success_no_lockout() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let api_key = "test-ag3-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![
        fast_gemini_503(),
        ResponseTemplate::new(200).set_body_json(gemini_200_body("back online")),
    ]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:generateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    assert!(
        !lockout_file_exists("gemini", &api_key),
        "503 must NOT create lockout file"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn ag4_auth_failure_403() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let api_key = "test-ag4-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![gemini_403()]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:generateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let result = driver.complete(simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::AuthenticationFailed(_))),
        "expected AuthenticationFailed, got {:?}",
        result
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn ag5_stream_429_retry() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let api_key = "test-ag5-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![
        fast_gemini_429(),
        fast_gemini_429(),
        gemini_sse_body("hello"),
    ]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:streamGenerateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&driver, simple_request("gpt-test")).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert!(!events.is_empty(), "stream should emit events");
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}
