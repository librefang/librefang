//! The ChatGPT (Responses API) driver's 429 handling must read `Retry-After` through the shared parser.
//! A hand-rolled `secs * 1000` overflowed (panicking in debug builds) on a huge delta-seconds value and ignored the HTTP-date form.

mod common;

use common::*;
use librefang_llm_driver::{LlmDriver, LlmError};
use librefang_llm_drivers::drivers::chatgpt::ChatGptDriver;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn mock_chatgpt_driver(server: &MockServer) -> ChatGptDriver {
    ChatGptDriver::with_proxy("test-chatgpt-session-token".to_string(), server.uri(), None)
}

async fn complete_with_retry_after(value: &str) -> Result<(), LlmError> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", value))
        .mount(&server)
        .await;
    mock_chatgpt_driver(&server)
        .complete(simple_request("chatgpt-test"))
        .await
        .map(|_| ())
}

/// A delta-seconds value above `u64::MAX / 1000` saturates instead of overflowing.
/// The driver reports the hint as given; the agent loop caps the in-turn sleep it causes (`in_turn_retry_delay_ms`), and the uncapped value feeds only the message journal's defer window.
#[tokio::test]
#[serial_test::serial]
async fn huge_retry_after_saturates_instead_of_overflowing() {
    let _env = isolated_env();
    match complete_with_retry_after("18446744073709552").await {
        Err(LlmError::RateLimited { retry_after_ms, .. }) => {
            assert!(
                retry_after_ms >= 18_446_744_073_709_000,
                "huge Retry-After must saturate high, not wrap to a small value: {retry_after_ms}"
            );
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

/// The HTTP-date form is honoured rather than replaced with the 5 s default.
#[tokio::test]
#[serial_test::serial]
async fn http_date_retry_after_is_honoured() {
    let _env = isolated_env();
    let value = (chrono::Utc::now() + chrono::Duration::seconds(600)).to_rfc2822();
    match complete_with_retry_after(&value).await {
        Err(LlmError::RateLimited { retry_after_ms, .. }) => {
            assert!(
                retry_after_ms > 60_000,
                "HTTP-date Retry-After ten minutes out must not collapse to the 5 s default: {retry_after_ms}"
            );
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}
