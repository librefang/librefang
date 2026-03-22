//! 示例测试 — 演示如何使用测试基础设施。

use crate::{assert_json_error, assert_json_ok, test_request, MockKernelBuilder, TestAppState};
use axum::http::{Method, StatusCode};
use tower::ServiceExt;

/// 测试 GET /api/health 端点返回 200 且包含 status 字段。
#[tokio::test]
async fn test_health_endpoint() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/health", None);
    let resp = router.oneshot(req).await.expect("请求失败");
    let json = assert_json_ok(resp).await;

    // health 端点应该返回 status 字段
    assert!(
        json.get("status").is_some(),
        "健康检查应包含 status 字段，实际返回: {json}"
    );
    let status = json["status"].as_str().unwrap();
    assert!(
        status == "ok" || status == "degraded",
        "status 应为 ok 或 degraded，实际: {status}"
    );
}

/// 测试 GET /api/agents 端点 — 返回 items 数组和 total 字段。
#[tokio::test]
async fn test_list_agents() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/agents", None);
    let resp = router.oneshot(req).await.expect("请求失败");
    let json = assert_json_ok(resp).await;

    // list_agents 返回 {"items": [...], "total": N, "offset": 0}
    assert!(
        json.get("items").is_some(),
        "list_agents 应返回 items 字段，实际: {json}"
    );
    assert!(
        json["items"].is_array(),
        "items 应为数组，实际: {}",
        json["items"]
    );
    assert!(
        json.get("total").is_some(),
        "list_agents 应返回 total 字段，实际: {json}"
    );
    // 验证 total 是一个合法的无符号整数
    assert!(
        json["total"].is_u64(),
        "total 应为无符号整数，实际: {}",
        json["total"]
    );
}

/// 测试 GET /api/agents/{id} — 使用无效 ID 应返回 400。
#[tokio::test]
async fn test_get_agent_invalid_id() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/agents/not-a-valid-uuid", None);
    let resp = router.oneshot(req).await.expect("请求失败");
    let json = assert_json_error(resp, StatusCode::BAD_REQUEST).await;

    assert!(
        json.get("error").is_some(),
        "错误响应应包含 error 字段，实际: {json}"
    );
}

/// 测试 GET /api/agents/{id} — 使用有效但不存在的 UUID 应返回 404。
#[tokio::test]
async fn test_get_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    // 使用一个有效的 UUID 但不存在于 registry 中
    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let req = test_request(Method::GET, &path, None);
    let resp = router.oneshot(req).await.expect("请求失败");
    let json = assert_json_error(resp, StatusCode::NOT_FOUND).await;

    assert!(
        json.get("error").is_some(),
        "404 响应应包含 error 字段，实际: {json}"
    );
}

/// 测试 MockLlmDriver 的调用记录功能。
#[tokio::test]
async fn test_mock_llm_driver_recording() {
    use crate::MockLlmDriver;
    use librefang_runtime::llm_driver::{CompletionRequest, LlmDriver};

    let driver = MockLlmDriver::new(vec!["回复1".into(), "回复2".into()]);

    let request = CompletionRequest {
        model: "test-model".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        temperature: 0.0,
        system: Some("test system prompt".into()),
        thinking: None,
        prompt_caching: false,
    };

    // 第一次调用
    let resp1 = driver.complete(request.clone()).await.unwrap();
    assert_eq!(resp1.text(), "回复1");

    // 第二次调用
    let resp2 = driver.complete(request).await.unwrap();
    assert_eq!(resp2.text(), "回复2");

    // 验证调用记录
    assert_eq!(driver.call_count(), 2);
    let calls = driver.recorded_calls();
    assert_eq!(calls[0].model, "test-model");
    assert_eq!(calls[0].system, Some("test system prompt".into()));
}

/// 测试使用自定义 config 构建 kernel。
#[tokio::test]
async fn test_custom_config_kernel() {
    let app = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.language = "zh".into();
    }));

    // 验证自定义配置生效
    assert_eq!(app.state.kernel.config_ref().language, "zh");
}

/// 测试 GET /api/version 端点。
#[tokio::test]
async fn test_version_endpoint() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/version", None);
    let resp = router.oneshot(req).await.expect("请求失败");
    let json = assert_json_ok(resp).await;

    assert!(
        json.get("version").is_some(),
        "version 端点应包含 version 字段，实际: {json}"
    );
}

// ── POST / PUT / DELETE 测试 ──────────────────────────────────────────

/// 测试 POST /api/agents — 使用 manifest_toml 创建 agent。
#[tokio::test]
async fn test_spawn_agent_post() {
    let app = TestAppState::new();
    let router = app.router();

    let manifest = r#"
[agent]
name = "test-bot"
system_prompt = "You are a test bot."
"#;
    let body = serde_json::json!({ "manifest_toml": manifest }).to_string();
    let req = test_request(Method::POST, "/api/agents", Some(&body));
    let resp = router.oneshot(req).await.expect("请求失败");
    let status = resp.status();

    // spawn 应返回 200 或 201（包含 agent id）
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "spawn_agent 应返回 200/201，实际: {status}"
    );
}

/// 测试 DELETE /api/agents/{id} — 删除不存在的 agent 应返回错误。
#[tokio::test]
async fn test_delete_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let req = test_request(Method::DELETE, &path, None);
    let resp = router.oneshot(req).await.expect("请求失败");

    // 删除不存在的 agent 应返回 404
    let json = assert_json_error(resp, StatusCode::NOT_FOUND).await;
    assert!(
        json.get("error").is_some(),
        "DELETE 404 响应应包含 error 字段，实际: {json}"
    );
}

/// 测试 PUT /api/agents/{id}/model — 为不存在的 agent 设置模型应返回错误。
#[tokio::test]
async fn test_set_model_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}/model");
    let body = serde_json::json!({ "model": "gpt-4" }).to_string();
    let req = test_request(Method::PUT, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("请求失败");

    // 不存在的 agent 应返回非 200 的错误状态码
    let status = resp.status();
    assert!(
        status.is_client_error() || status.is_server_error(),
        "set_model 对不存在的 agent 应返回错误状态码，实际: {status}"
    );
}

/// 测试 POST /api/agents/{id}/message — 向不存在的 agent 发消息应返回错误。
#[tokio::test]
async fn test_send_message_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}/message");
    let body = serde_json::json!({ "message": "hello" }).to_string();
    let req = test_request(Method::POST, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("请求失败");

    let status = resp.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
        "send_message 对不存在的 agent 应返回 404/400，实际: {status}"
    );
}

/// 测试 PATCH /api/agents/{id} — 更新不存在的 agent 应返回错误。
#[tokio::test]
async fn test_patch_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let body = serde_json::json!({ "name": "new-name" }).to_string();
    let req = test_request(Method::PATCH, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("请求失败");

    let status = resp.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
        "patch_agent 对不存在的 agent 应返回 404/400，实际: {status}"
    );
}

// ── MockLlmDriver builder 方法测试 ──────────────────────────────────

/// 测试 MockLlmDriver 的 with_tokens 和 with_stop_reason 自定义设置。
#[tokio::test]
async fn test_mock_llm_driver_custom_tokens_and_stop_reason() {
    use crate::MockLlmDriver;
    use librefang_runtime::llm_driver::{CompletionRequest, LlmDriver};
    use librefang_types::message::StopReason;

    let driver = MockLlmDriver::with_response("test")
        .with_tokens(200, 100)
        .with_stop_reason(StopReason::MaxTokens);

    let request = CompletionRequest {
        model: "test-model".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
    };

    let resp = driver.complete(request).await.unwrap();
    assert_eq!(
        resp.usage.input_tokens, 200,
        "input_tokens 应为自定义值 200"
    );
    assert_eq!(
        resp.usage.output_tokens, 100,
        "output_tokens 应为自定义值 100"
    );
    assert_eq!(
        resp.stop_reason,
        StopReason::MaxTokens,
        "stop_reason 应为 MaxTokens"
    );
}

// ── FailingLlmDriver 测试 ──────────────────────────────────────────

/// 测试 FailingLlmDriver 始终返回错误（用于错误处理场景）。
#[tokio::test]
async fn test_failing_llm_driver() {
    use crate::FailingLlmDriver;
    use librefang_runtime::llm_driver::{CompletionRequest, LlmDriver};

    let driver = FailingLlmDriver::new("模拟的 API 错误");

    let request = CompletionRequest {
        model: "test-model".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
    };

    let result = driver.complete(request).await;
    assert!(result.is_err(), "FailingLlmDriver 应始终返回错误");

    let err = result.unwrap_err();
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("模拟的 API 错误"),
        "错误消息应包含自定义内容，实际: {err_msg}"
    );

    // FailingLlmDriver 的 is_configured 应返回 false
    assert!(
        !driver.is_configured(),
        "FailingLlmDriver.is_configured() 应返回 false"
    );
}
