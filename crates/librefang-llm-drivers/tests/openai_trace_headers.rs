//! Verifies the OpenAI driver surfaces caller identity (agent / session /
//! step) on outbound HTTP requests as `x-librefang-*` headers, so any
//! observability sidecar in front of an OpenAI-compatible upstream can
//! correlate request log records without parsing the JSON body.

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
        model: "gpt-test".to_string(),
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

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
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
async fn complete_omits_trace_headers_when_unset() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
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

#[tokio::test]
#[serial_test::serial]
async fn complete_emits_partial_trace_headers() {
    // Only some fields populated — only those headers should appear.
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("agent-only"), None, None);
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    let headers = &received[0].headers;
    assert_eq!(
        headers
            .get("x-librefang-agent-id")
            .map(|v| v.to_str().unwrap()),
        Some("agent-only"),
    );
    assert!(headers.get("x-librefang-session-id").is_none());
    assert!(headers.get("x-librefang-step-id").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn complete_trace_headers_override_same_named_extra_headers() {
    // Per the medium review finding on PR #4548: when an operator has set
    // `x-librefang-agent-id` via the driver-level extra_headers escape
    // hatch (e.g. for diagnostics or a sidecar shim), the per-request
    // trace header from CompletionRequest.agent_id MUST replace it on the
    // wire — not duplicate. Without `insert`-semantics, downstream log
    // correlation gets two values for the same name and has to guess
    // which one the upstream actually billed.
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server).with_extra_headers(vec![
        // Duplicate trace-name → MUST be overwritten by request value.
        (
            "x-librefang-agent-id".to_string(),
            "stale-extra".to_string(),
        ),
        // Unrelated extra → MUST be preserved verbatim.
        ("x-vendor-trace".to_string(), "keep-me".to_string()),
    ]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("request-wins"), None, None);
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;

    // Only one value, and it's the per-request trace ID.
    let agent_values: Vec<_> = headers
        .get_all("x-librefang-agent-id")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(
        agent_values,
        vec!["request-wins".to_string()],
        "trace header must replace same-named extras header, not duplicate",
    );
    // Unrelated extra header survives.
    assert_eq!(
        headers.get("x-vendor-trace").map(|v| v.to_str().unwrap()),
        Some("keep-me"),
    );
}

#[tokio::test]
#[serial_test::serial]
async fn complete_skips_malformed_trace_header_value() {
    // Per the medium review finding on PR #4548: trace ID values containing
    // \r, \n, NUL, or other non-visible bytes are rejected by
    // `reqwest::header::HeaderValue::from_str`. The driver MUST swallow
    // those validation errors (with a `warn!`) and let the request proceed
    // — failing the LLM call because of a malformed observability hint
    // would be far worse than dropping the hint. Exotic-but-valid Unicode
    // (printable bytes, non-ASCII) is rejected by HeaderValue too, so this
    // case also exercises the same path.
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    // Each value here trips `HeaderValue::from_str`. The http crate
    // rejects bytes < 0x20 (except \t) and 0x7F (DEL), so:
    //   * `\r\n` in agent_id triggers CRLF-injection rejection
    //   * `\0` in session_id triggers NUL rejection
    //   * `\x07` (bell) in step_id triggers control-char rejection
    // Note: extended-ASCII / UTF-8 bytes >= 0x80 are actually permitted
    // by HeaderValue, so a Chinese-character step_id like "步骤-7" would
    // pass through (the wire would carry the raw UTF-8 bytes). The
    // sanitization path is specifically about CRLF injection and other
    // structurally-invalid characters that cause `.send()` to fail
    // opaquely, not about character-set policy.
    let req = request_with_trace_ids(
        Some("agent\r\nInjected: header"),
        Some("sess\0null"),
        Some("step\x07bell"),
    );
    let result = driver.complete(req).await;
    assert!(
        result.is_ok(),
        "complete must still succeed even when trace IDs are malformed: {result:?}",
    );

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    // None of the malformed values should have made it onto the wire.
    assert!(
        headers.get("x-librefang-agent-id").is_none(),
        "agent_id with CRLF must not be sent",
    );
    assert!(
        headers.get("x-librefang-session-id").is_none(),
        "session_id with NUL must not be sent",
    );
    assert!(
        headers.get("x-librefang-step-id").is_none(),
        "step_id with control char must not be sent",
    );
}

#[tokio::test]
#[serial_test::serial]
async fn complete_passes_through_extended_ascii_trace_header_value() {
    // Companion to the malformed-value test: the sanitization path is
    // narrow on purpose. Bytes >= 0x80 (extended ASCII / UTF-8) are
    // accepted by `HeaderValue::from_str` and therefore make it onto the
    // wire as-is. We assert this so a future "tighten validation to
    // ASCII-only" change comes with a deliberate review, not as a
    // surprise side-effect of the CRLF fix.
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some("步骤-7"), None, None);
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    let headers = &received[0].headers;
    // Header is present; we don't assert the exact bytes because
    // wiremock decodes through `HeaderValue::to_str`, which fails on
    // non-ASCII. Presence is enough — the point is "did NOT get
    // dropped".
    assert!(
        headers.get("x-librefang-agent-id").is_some(),
        "non-ASCII UTF-8 agent_id must NOT be dropped — only structurally-invalid bytes are sanitized",
    );
}

#[tokio::test]
#[serial_test::serial]
async fn complete_skips_empty_string_trace_header_value() {
    // Empty-string values are treated as absent: `Some("")` is a common
    // shape for external callers that haven't decided whether they want
    // the field to be set, and emitting `x-librefang-agent-id:` (header
    // name with empty value) would just pollute downstream log
    // correlation. In-tree call sites format UUIDs / integers so this
    // can't fire today, but we lock the contract for external
    // consumers.
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_200_body("ok")))
        .mount(&server)
        .await;

    let req = request_with_trace_ids(Some(""), Some(""), Some(""));
    let result = driver.complete(req).await;
    assert!(result.is_ok(), "complete should succeed: {result:?}");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let headers = &received[0].headers;
    assert!(
        headers.get("x-librefang-agent-id").is_none(),
        "empty agent_id must not emit a header",
    );
    assert!(
        headers.get("x-librefang-session-id").is_none(),
        "empty session_id must not emit a header",
    );
    assert!(
        headers.get("x-librefang-step-id").is_none(),
        "empty step_id must not emit a header",
    );
}

#[tokio::test]
#[serial_test::serial]
async fn stream_emits_trace_headers_when_set() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let driver = mock_openai_driver(&server);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_sse_body(&["hi"]))
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
