//! Integration tests for the native Ollama driver introduced by #4810.
//!
//! Locks in the wire contract against a `wiremock` server:
//!
//! 1. Request shape: POSTs to `/api/chat` with a native body
//!    (`messages`, `tools`, `think`, `options.{temperature,num_predict}`)
//!    and **never** to `/v1/chat/completions`.
//! 2. Bearer auth is sent only when an API key is configured.
//! 3. Non-streaming responses parse `message.content` /
//!    `message.thinking` / `message.tool_calls` correctly, synthesise
//!    stable tool-call IDs, and propagate `prompt_eval_count` /
//!    `eval_count` into [`TokenUsage`].
//! 4. NDJSON streaming aggregates incremental `content` and `thinking`
//!    deltas, surfaces tool calls on stream close, and reports usage
//!    from the final `done: true` envelope.
//! 5. 404 / 401 / 5xx error mapping (`ModelNotFound`,
//!    `AuthenticationFailed`, `Api`).
//! 6. Multi-modal: `ContentBlock::Image` is base64-attached to the
//!    user message via the native `images: [...]` field rather than the
//!    OpenAI `image_url` envelope.

mod common;

use common::{collect_stream, isolated_env, mock_ollama_driver, request_json, simple_request};
use librefang_llm_driver::{LlmDriver, LlmError, StreamEvent};
use librefang_llm_drivers::drivers::ollama::OllamaDriver;
use librefang_types::config::ThinkingConfig;
use librefang_types::message::{ContentBlock, Message, MessageContent, Role, StopReason};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ollama_200_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "llama3.1:8b",
        "created_at": "2026-01-01T00:00:00Z",
        "message": {
            "role": "assistant",
            "content": text
        },
        "done": true,
        "done_reason": "stop",
        "total_duration": 1_000_000_000_u64,
        "prompt_eval_count": 11,
        "eval_count": 7
    })
}

/// The driver POSTs to `/api/chat` (not `/v1/chat/completions`) and the
/// body carries the native shape: messages, tools-when-present, options
/// envelope for sampler params, no `max_tokens` at top level.
#[tokio::test]
#[serial_test::serial]
async fn request_targets_native_api_chat_with_native_body_shape() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("hi")))
        .expect(1)
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let resp = driver
        .complete(simple_request("llama3.1:8b"))
        .await
        .expect("complete should succeed");
    assert_eq!(resp.text(), "hi");

    let received = &server.received_requests().await.expect("requests")[0];
    assert_eq!(
        received.url.path(),
        "/api/chat",
        "driver MUST NOT hit /v1/chat/completions on native Ollama (regression: #4810)"
    );

    let body = request_json(received);
    assert_eq!(body["model"], "llama3.1:8b");
    assert!(
        body.get("max_tokens").is_none(),
        "max_tokens lives under options.num_predict on native Ollama"
    );
    let messages = body["messages"].as_array().expect("messages array");
    assert!(!messages.is_empty());
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "hello");
    let opts = body
        .get("options")
        .expect("options envelope must carry sampler params on native Ollama");
    assert!(opts.get("num_predict").is_some());
    assert!(opts.get("temperature").is_some());
}

/// No `Authorization` header when the key is empty (default localhost
/// setup); Bearer header when a key is configured (tunnelled / hosted
/// Ollama).
#[tokio::test]
#[serial_test::serial]
async fn auth_header_only_emitted_when_api_key_configured() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("ok")))
        .mount(&server)
        .await;

    // 1) Empty key → no Authorization header.
    let bare = mock_ollama_driver(&server);
    bare.complete(simple_request("m")).await.expect("ok");
    let req_no_auth = &server.received_requests().await.expect("reqs")[0];
    assert!(
        req_no_auth.headers.get("authorization").is_none(),
        "default localhost flow must not send Authorization"
    );

    // 2) Explicit key → Bearer header.
    let keyed = OllamaDriver::with_proxy_and_timeout(
        "secret-token-xyz".to_string(),
        server.uri(),
        None,
        Some(5),
    );
    keyed.complete(simple_request("m")).await.expect("ok");
    let req_keyed = server.received_requests().await.expect("reqs");
    let last = req_keyed.last().expect("last request");
    let auth = last
        .headers
        .get("authorization")
        .expect("auth header on tunnelled flow")
        .to_str()
        .unwrap();
    assert_eq!(auth, "Bearer secret-token-xyz");
}

/// `request.thinking = Some(_)` flips the native first-class `think`
/// boolean — no `extra_body.think` injection like the legacy compat
/// path.
#[tokio::test]
#[serial_test::serial]
async fn thinking_request_sets_native_think_true_field() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("ok")))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let mut req = simple_request("qwen3:8b");
    req.thinking = Some(ThinkingConfig::default());
    driver.complete(req).await.expect("ok");

    let received = &server.received_requests().await.expect("reqs")[0];
    let body = request_json(received);
    assert_eq!(body["think"], serde_json::Value::Bool(true));
}

/// Non-streaming `tool_calls` parse into `ToolCall` blocks with
/// synthesised IDs, and `done_reason: "stop"` plus a populated
/// `tool_calls` array maps to `StopReason::ToolUse`.
#[tokio::test]
#[serial_test::serial]
async fn non_streaming_tool_calls_parse_with_synthesised_ids() {
    let _env = isolated_env();
    let server = MockServer::start().await;

    let response = serde_json::json!({
        "model": "llama3.1:8b",
        "created_at": "2026-01-01T00:00:00Z",
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "function": {
                    "name": "get_weather",
                    "arguments": {"city": "Paris"}
                }
            }]
        },
        "done": true,
        "done_reason": "stop",
        "prompt_eval_count": 20,
        "eval_count": 4
    });
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let resp = driver.complete(simple_request("m")).await.expect("ok");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.tool_calls.len(), 1);
    let call = &resp.tool_calls[0];
    assert_eq!(call.name, "get_weather");
    assert_eq!(call.input["city"], "Paris");
    // ID is synthesised since native API doesn't return one.
    assert!(
        call.id.starts_with("ollama-call-"),
        "expected synthesised id, got {:?}",
        call.id
    );
    assert_eq!(resp.usage.input_tokens, 20);
    assert_eq!(resp.usage.output_tokens, 4);
}

/// Native API returns reasoning under `message.thinking` (first-class),
/// not embedded in the content with `<think>` tags. The driver routes
/// it to `ContentBlock::Thinking`.
#[tokio::test]
#[serial_test::serial]
async fn non_streaming_first_class_thinking_routes_to_thinking_block() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "model": "qwen3:8b",
        "message": {
            "role": "assistant",
            "content": "the answer is 42",
            "thinking": "we should consider the question carefully"
        },
        "done": true,
        "done_reason": "stop",
        "prompt_eval_count": 5,
        "eval_count": 8
    });
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let resp = driver
        .complete(simple_request("qwen3:8b"))
        .await
        .expect("ok");
    let has_thinking = resp.content.iter().any(|b| {
        matches!(
            b,
            ContentBlock::Thinking { thinking, .. }
                if thinking.contains("consider the question")
        )
    });
    assert!(
        has_thinking,
        "expected first-class message.thinking → Thinking block, got {:?}",
        resp.content
    );
    let has_text = resp
        .content
        .iter()
        .any(|b| matches!(b, ContentBlock::Text { text, .. } if text.contains("42")));
    assert!(has_text);
}

/// NDJSON streaming aggregates `content` deltas across chunks and emits
/// per-delta events; the final `done: true` chunk supplies token usage.
#[tokio::test]
#[serial_test::serial]
async fn streaming_ndjson_aggregates_text_and_reports_usage() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let mut body = String::new();
    for piece in ["He", "llo", " world"] {
        body.push_str(
            &serde_json::json!({
                "model": "llama3.1:8b",
                "message": {"role": "assistant", "content": piece},
                "done": false
            })
            .to_string(),
        );
        body.push('\n');
    }
    body.push_str(
        &serde_json::json!({
            "model": "llama3.1:8b",
            "message": {"role": "assistant", "content": ""},
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 9
        })
        .to_string(),
    );
    body.push('\n');

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let (result, events) = collect_stream(&driver, simple_request("llama3.1:8b")).await;
    let resp = result.expect("stream ok");
    assert_eq!(resp.text(), "Hello world");
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert_eq!(resp.usage.input_tokens, 12);
    assert_eq!(resp.usage.output_tokens, 9);

    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["He", "llo", " world"]);
    let saw_complete = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ContentComplete { .. }));
    assert!(
        saw_complete,
        "ContentComplete must be emitted at stream end"
    );
}

/// Streaming `thinking` deltas surface as `StreamEvent::ThinkingDelta`
/// without leaking into the visible text aggregate.
#[tokio::test]
#[serial_test::serial]
async fn streaming_thinking_deltas_route_to_thinking_event() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let mut body = String::new();
    body.push_str(
        &serde_json::json!({
            "message": {"role": "assistant", "thinking": "let me reason"},
            "done": false
        })
        .to_string(),
    );
    body.push('\n');
    body.push_str(
        &serde_json::json!({
            "message": {"role": "assistant", "content": "answer"},
            "done": false
        })
        .to_string(),
    );
    body.push('\n');
    body.push_str(
        &serde_json::json!({
            "message": {"role": "assistant", "content": ""},
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 3,
            "eval_count": 2
        })
        .to_string(),
    );
    body.push('\n');

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let (result, events) = collect_stream(&driver, simple_request("qwen3:8b")).await;
    let resp = result.expect("stream ok");
    assert_eq!(resp.text(), "answer");

    let saw_thinking = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ThinkingDelta { text } if text == "let me reason"));
    assert!(
        saw_thinking,
        "ThinkingDelta must surface for native message.thinking on streaming"
    );
    // Thinking text MUST NOT leak into the visible text channel.
    let leaked = events
        .iter()
        .any(|e| matches!(e, StreamEvent::TextDelta { text } if text.contains("let me reason")));
    assert!(!leaked, "thinking content must not appear in TextDelta");
}

/// Streaming tool calls (final chunk envelope) surface as paired
/// `ToolUseStart` / `ToolUseEnd` events, parse the object-shaped
/// arguments, and the final response carries the tool call.
#[tokio::test]
#[serial_test::serial]
async fn streaming_tool_calls_emit_start_end_pair() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    let mut body = String::new();
    body.push_str(
        &serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "get_weather",
                        "arguments": {"city": "Tokyo"}
                    }
                }]
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 10,
            "eval_count": 5
        })
        .to_string(),
    );
    body.push('\n');

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let (result, events) = collect_stream(&driver, simple_request("m")).await;
    let resp = result.expect("stream ok");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "get_weather");
    assert_eq!(resp.tool_calls[0].input["city"], "Tokyo");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);

    let starts = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::ToolUseStart { .. }))
        .count();
    let ends = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::ToolUseEnd { .. }))
        .count();
    assert_eq!(starts, 1, "exactly one ToolUseStart, got events {events:?}");
    assert_eq!(ends, 1, "exactly one ToolUseEnd, got events {events:?}");
}

/// 404 maps to `ModelNotFound` so the agent loop / fallback chain can
/// route around an unloaded tag.
#[tokio::test]
#[serial_test::serial]
async fn http_404_maps_to_model_not_found() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "model 'llama99' not found, try pulling it first"
        })))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let err = driver
        .complete(simple_request("llama99"))
        .await
        .unwrap_err();
    match err {
        LlmError::ModelNotFound(msg) => {
            assert!(msg.contains("llama99"), "msg: {msg}");
        }
        other => panic!("expected ModelNotFound, got {other:?}"),
    }
}

/// 401 maps to `AuthenticationFailed`. Hosted/tunnelled Ollama with a
/// rejected token surfaces correctly to the chain.
#[tokio::test]
#[serial_test::serial]
async fn http_401_maps_to_authentication_failed() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(serde_json::json!({"error": "unauthorized"})),
        )
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let err = driver.complete(simple_request("m")).await.unwrap_err();
    assert!(matches!(err, LlmError::AuthenticationFailed(_)));
}

/// Unstructured 5xx body (no JSON envelope) still produces an `Api`
/// error with the raw body as the message.
#[tokio::test]
#[serial_test::serial]
async fn http_502_passes_through_raw_body_in_api_error() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let err = driver.complete(simple_request("m")).await.unwrap_err();
    match err {
        LlmError::Api {
            status, message, ..
        } => {
            assert_eq!(status, 502);
            assert_eq!(message, "Bad Gateway");
        }
        other => panic!("expected Api(502), got {other:?}"),
    }
}

/// Multi-modal: a `ContentBlock::Image` user block lands in the native
/// `images: ["<base64>"]` list rather than being smuggled through the
/// OpenAI `image_url` envelope.
#[tokio::test]
#[serial_test::serial]
async fn multimodal_image_block_serialises_as_native_images_array() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("ok")))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let mut req = simple_request("llava:7b");
    req.messages = std::sync::Arc::new(vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![
            ContentBlock::Text {
                text: "what's in this picture?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "AAAA".to_string(),
            },
        ]),
        pinned: false,
        timestamp: None,
    }]);
    driver.complete(req).await.expect("ok");

    let received = &server.received_requests().await.expect("reqs")[0];
    let body = request_json(received);
    let user = body["messages"]
        .as_array()
        .and_then(|m| m.iter().find(|m| m["role"] == "user"))
        .expect("user message");
    assert_eq!(user["content"], "what's in this picture?");
    let images = user["images"].as_array().expect("images present");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0], "AAAA");
    assert!(
        user.get("image_url").is_none(),
        "must use native images[] not OpenAI image_url"
    );
}

/// `ContentBlock::ToolResult` round-trips as `role: "tool"` with
/// `tool_name` (Ollama's correlation key) — not as the OpenAI
/// `role: "tool"` + `tool_call_id` shape.
#[tokio::test]
#[serial_test::serial]
async fn tool_result_serialises_as_role_tool_with_tool_name() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("ok")))
        .mount(&server)
        .await;

    let driver = mock_ollama_driver(&server);
    let mut req = simple_request("m");
    req.messages = std::sync::Arc::new(vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "ollama-call-aaa".to_string(),
            tool_name: "get_weather".to_string(),
            content: "sunny, 72F".to_string(),
            is_error: false,
            status: Default::default(),
            approval_request_id: None,
        }]),
        pinned: false,
        timestamp: None,
    }]);
    driver.complete(req).await.expect("ok");

    let received = &server.received_requests().await.expect("reqs")[0];
    let body = request_json(received);
    let tool_msg = body["messages"]
        .as_array()
        .and_then(|m| m.iter().find(|m| m["role"] == "tool"))
        .expect("tool message present");
    assert_eq!(tool_msg["tool_name"], "get_weather");
    assert_eq!(tool_msg["content"], "sunny, 72F");
    assert!(
        tool_msg.get("tool_call_id").is_none(),
        "native API uses tool_name, not OpenAI's tool_call_id"
    );
}

/// Existing user configs that pin `base_url = "http://host:11434/v1"`
/// (a holdover from when the registry pointed at the OpenAI compat
/// shim) get the `/v1` silently stripped at driver construction so
/// `/api/chat` composes correctly. Without this migration, real users
/// would 404 on every turn after upgrade.
#[tokio::test]
#[serial_test::serial]
async fn legacy_v1_suffix_in_user_base_url_is_silently_stripped() {
    let _env = isolated_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ollama_200_body("ok")))
        .expect(1)
        .mount(&server)
        .await;

    // Construct the driver with the legacy URL form. If migration
    // doesn't trigger, the driver hits `{server.uri()}/v1/api/chat`
    // which the mock doesn't match → expect(1) fails with 0 hits.
    let driver = OllamaDriver::with_proxy_and_timeout(
        String::new(),
        format!("{}/v1", server.uri()),
        None,
        Some(5),
    );
    driver
        .complete(simple_request("m"))
        .await
        .expect("legacy URL must still reach /api/chat");
}
