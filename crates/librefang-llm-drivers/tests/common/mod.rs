#![allow(dead_code)]

use std::path::PathBuf;
use std::time::SystemTime;

use librefang_llm_driver::{CompletionRequest, LlmDriver, LlmError, StreamEvent};
use librefang_types::message::Message;
use librefang_types::tool::ToolDefinition;
use tempfile::TempDir;
use uuid::Uuid;
use wiremock::{MockServer, Request, ResponseTemplate};

use librefang_llm_drivers::backoff;
use librefang_llm_drivers::drivers::anthropic::AnthropicDriver;
use librefang_llm_drivers::drivers::gemini::GeminiDriver;
use librefang_llm_drivers::drivers::openai::OpenAIDriver;
use librefang_llm_drivers::shared_rate_guard;

pub struct TestEnv {
    _tmp: TempDir,
    _backoff_guard: backoff::ZeroBackoffGuard,
}

pub fn isolated_env() -> TestEnv {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("LIBREFANG_HOME", tmp.path());
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");
    let backoff_guard = backoff::enable_test_zero_backoff();
    TestEnv {
        _tmp: tmp,
        _backoff_guard: backoff_guard,
    }
}

pub fn mock_openai_driver(server: &MockServer) -> OpenAIDriver {
    OpenAIDriver::with_proxy_and_timeout(
        format!("sk-test-{}", Uuid::new_v4()),
        server.uri(),
        None,
        Some(5),
    )
}

pub fn mock_anthropic_driver(server: &MockServer) -> AnthropicDriver {
    AnthropicDriver::with_proxy_and_timeout(
        format!("sk-ant-test-{}", Uuid::new_v4()),
        server.uri(),
        None,
        Some(5),
    )
}

pub fn mock_gemini_driver(server: &MockServer) -> GeminiDriver {
    GeminiDriver::with_proxy_and_timeout(
        format!("test-key-{}", Uuid::new_v4()),
        server.uri(),
        None,
        Some(5),
    )
}

pub fn simple_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message::user("hello")]),
        tools: Vec::new(),
        max_tokens: 16,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
    }
}

pub fn request_with_tools(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message::user("use a tool")]),
        tools: vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "Get current weather".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
        }],
        max_tokens: 256,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
    }
}

pub fn request_with_temperature(model: &str, temp: f32) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message::user("hello")]),
        tools: Vec::new(),
        max_tokens: 16,
        temperature: temp,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
    }
}

pub fn o_series_request() -> CompletionRequest {
    CompletionRequest {
        model: "o3-mini".to_string(),
        messages: std::sync::Arc::new(vec![Message::user("solve this")]),
        tools: Vec::new(),
        max_tokens: 1000,
        temperature: 1.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
    }
}

pub fn openai_200_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 3,
            "total_tokens": 8
        }
    })
}

pub fn openai_429_response(retry_after_secs: u64) -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("retry-after", retry_after_secs.to_string())
        .insert_header("x-ratelimit-reset-requests-1h", "30")
        .set_body_json(serde_json::json!({
            "error": {"message": "rate limited", "type": "rate_limit_error"}
        }))
}

pub fn openai_400_temperature_rejected() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": "Unsupported parameter: 'temperature' is not supported with this model.",
            "type": "invalid_request_error",
            "param": "temperature",
            "code": "unsupported_parameter"
        }
    }))
}

pub fn openai_400_max_tokens_unsupported() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": "Unsupported parameter: 'max_tokens' is not supported with this model. Please use 'max_completion_tokens' instead.",
            "type": "invalid_request_error",
            "param": "max_tokens",
            "code": "unsupported_parameter"
        }
    }))
}

pub fn openai_400_max_tokens_cap(limit: u32) -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": format!("max_completion_tokens must be less than or equal to `{limit}`"),
            "type": "invalid_request_error",
            "param": "max_completion_tokens"
        }
    }))
}

pub fn openai_400_tool_not_supported() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": "This model does not support tools. Please use a model that supports tools.",
            "type": "invalid_request_error"
        }
    }))
}

pub fn openai_500_tool_error() -> ResponseTemplate {
    ResponseTemplate::new(500).set_body_json(serde_json::json!({
        "error": {
            "message": "internal error",
            "type": "server_error"
        }
    }))
}

pub fn openai_400_tool_use_failed() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(serde_json::json!({
        "error": {
            "message": "tool_use_failed",
            "type": "invalid_request_error"
        }
    }))
}

pub fn openai_sse_body(chunks: &[&str]) -> ResponseTemplate {
    let mut body = String::new();
    for chunk in chunks {
        let data = serde_json::json!({
            "choices": [{"delta": {"content": chunk}}]
        });
        body.push_str(&format!("data: {}\n\n", data));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

pub fn anthropic_200_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "model": "claude-test",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 3}
    })
}

pub fn anthropic_429_response() -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("retry-after", "30")
        .insert_header("anthropic-ratelimit-requests-limit", "1000")
        .insert_header("anthropic-ratelimit-requests-remaining", "0")
        .insert_header("anthropic-ratelimit-requests-reset", "30")
        .set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "rate_limit_error", "message": "rate limited"}
        }))
}

pub fn anthropic_529_response() -> ResponseTemplate {
    ResponseTemplate::new(529)
        .insert_header("retry-after", "10")
        .set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "overloaded_error", "message": "Overloaded"}
        }))
}

pub fn anthropic_sse_body(text: &str) -> ResponseTemplate {
    let mut body = String::new();

    body.push_str(&format!(
        "event: message_start\ndata: {}\n\n",
        serde_json::json!({
            "type": "message_start",
            "message": {
                "id": "msg_sse_test",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-test",
                "usage": {"input_tokens": 5, "output_tokens": 0}
            }
        })
    ));

    body.push_str(&format!(
        "event: content_block_start\ndata: {}\n\n",
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""}
        })
    ));

    for ch in text.chars() {
        body.push_str(&format!(
            "event: content_block_delta\ndata: {}\n\n",
            serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": ch.to_string()}
            })
        ));
    }

    body.push_str(&format!(
        "event: content_block_stop\ndata: {}\n\n",
        serde_json::json!({"type": "content_block_stop", "index": 0})
    ));

    body.push_str(&format!(
        "event: message_delta\ndata: {}\n\n",
        serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 3}
        })
    ));

    body.push_str(&format!(
        "event: message_stop\ndata: {}\n\n",
        serde_json::json!({"type": "message_stop"})
    ));

    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

pub fn gemini_200_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{"text": text}]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 5,
            "candidatesTokenCount": 3
        }
    })
}

pub fn gemini_429_response() -> ResponseTemplate {
    ResponseTemplate::new(429)
        .insert_header("retry-after", "30")
        .set_body_json(serde_json::json!({
            "error": {
                "code": 429,
                "message": "Resource exhausted",
                "status": "RESOURCE_EXHAUSTED"
            }
        }))
}

pub fn gemini_503_response() -> ResponseTemplate {
    ResponseTemplate::new(503)
        .insert_header("retry-after", "10")
        .set_body_json(serde_json::json!({
            "error": {
                "code": 503,
                "message": "The model is overloaded",
                "status": "UNAVAILABLE"
            }
        }))
}

pub fn gemini_sse_body(text: &str) -> ResponseTemplate {
    let chunk = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{"text": text}]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 5,
            "candidatesTokenCount": 3
        }
    });
    let body = format!("data: {}\n\n", chunk);
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

pub fn lockout_file_exists(provider: &str, api_key: &str) -> bool {
    let kid = shared_rate_guard::key_id_hash(api_key);
    let dir = rate_limit_dir_path();
    let filename = format!("{provider}__{kid}.json");
    dir.join(filename).exists()
}

pub fn create_lockout_file(provider: &str, api_key: &str, until: SystemTime) {
    let kid = shared_rate_guard::key_id_hash(api_key);
    shared_rate_guard::record(provider, &kid, until, Some("test lockout".into()));
}

pub fn provider_for_openai_mock() -> &'static str {
    "openai-compat"
}

pub fn request_json(request: &Request) -> serde_json::Value {
    serde_json::from_slice(&request.body).expect("request body should be valid JSON")
}

pub async fn collect_stream(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
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
    let result = driver.stream(request, tx.clone()).await;
    drop(tx);
    let events = handle.await.unwrap();
    (result, events)
}

fn rate_limit_dir_path() -> PathBuf {
    if let Ok(custom) = std::env::var("LIBREFANG_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".librefang")
    }
    .join("rate_limits")
}
