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
    // Needs the isolated LIBREFANG_HOME so the lockout assertion below reads this run's rate_limits dir rather than whatever a previous run left in the real one.
    let _env = isolated_env();
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
    // A 429 the driver rode out itself must not persist a lockout: the call returned Ok, and only wall-clock expiry ever clears a recorded lockout, so keeping it would block this and every sibling process against a provider that is demonstrably healthy.
    assert!(
        !lockout_file_exists("gemini", &api_key),
        "a recovered 429 must not persist a cross-process lockout"
    );
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
    // The other half of the behaviour this PR redefined: once the driver has given up, the lockout must persist, so a sibling process short-circuits in `pre_request_check` instead of re-running the same four doomed requests.
    // Without this assertion, deleting the terminal `record_429_from_headers` branch would leave the whole suite green.
    assert!(
        lockout_file_exists("gemini", &api_key),
        "exhausted 429 must persist a cross-process lockout"
    );
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
    assert!(
        !lockout_file_exists("gemini", &api_key),
        "a recovered 429 must not persist a cross-process lockout on the streaming path either"
    );
}

/// The streaming twin of `ag2`: the stream path has its own 429 handling, so it needs its own assertion that the terminal attempt still persists the lockout.
#[tokio::test]
#[serial_test::serial]
async fn ag6_stream_429_exhaustion() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let api_key = "test-ag6-key".to_string();
    let driver = GeminiDriver::with_proxy_and_timeout(api_key.clone(), server.uri(), None, Some(5));

    let (responder, counter) = SequencedResponder::new(vec![
        fast_gemini_429(),
        fast_gemini_429(),
        fast_gemini_429(),
        fast_gemini_429(),
    ]);

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gpt-test:streamGenerateContent"))
        .respond_with(responder)
        .mount(&server)
        .await;

    let (result, events) = collect_stream(&driver, simple_request("gpt-test")).await;
    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "expected RateLimited, got {:?}",
        result
    );
    assert!(events.is_empty(), "expected no events, got {:?}", events);
    assert_eq!(counter.load(Ordering::SeqCst), 4);
    assert!(
        lockout_file_exists("gemini", &api_key),
        "exhausted 429 on the streaming path must persist a cross-process lockout"
    );
}
