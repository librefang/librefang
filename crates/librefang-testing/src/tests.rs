//! Example tests — demonstrates how to use the test infrastructure.

use crate::{
    agent_manifest_for_provider, assert_json_error, assert_json_ok, assert_json_status,
    seed_registry_skill, single_step_workflow, spawn_openai_fixture_server, test_request,
    MockKernelBuilder, TestAppState, FIXTURE_AGENT_MANIFEST,
};
use axum::http::{Method, StatusCode};
use tower::ServiceExt;

/// Tests that GET /api/health returns 200 and contains a status field.
#[tokio::test(flavor = "multi_thread")]
async fn test_health_endpoint() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/health", None);
    let resp = router.oneshot(req).await.expect("request failed");
    let json = assert_json_ok(resp).await;

    // Health endpoint should return a status field
    assert!(
        json.get("status").is_some(),
        "health check should contain a status field, got: {json}"
    );
    let status = json["status"].as_str().unwrap();
    assert!(
        status == "ok" || status == "degraded",
        "status should be ok or degraded, got: {status}"
    );
}

/// Tests that GET /api/agents returns an items array and a total field.
#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/agents", None);
    let resp = router.oneshot(req).await.expect("request failed");
    let json = assert_json_ok(resp).await;

    // list_agents returns {"items": [...], "total": N, "offset": 0}
    assert!(
        json.get("items").is_some(),
        "list_agents should return an items field, got: {json}"
    );
    assert!(
        json["items"].is_array(),
        "items should be an array, got: {}",
        json["items"]
    );
    assert!(
        json.get("total").is_some(),
        "list_agents should return a total field, got: {json}"
    );
    // Verify total is a valid unsigned integer
    assert!(
        json["total"].is_u64(),
        "total should be an unsigned integer, got: {}",
        json["total"]
    );
}

/// Tests that GET /api/agents/{id} with an invalid ID returns 400.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_invalid_id() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/agents/not-a-valid-uuid", None);
    let resp = router.oneshot(req).await.expect("request failed");
    let json = assert_json_error(resp, StatusCode::BAD_REQUEST).await;

    assert!(
        json.get("error").is_some(),
        "error response should contain an error field, got: {json}"
    );
}

/// Tests that GET /api/agents/{id} with a valid but nonexistent UUID returns 404.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    // Use a valid UUID that does not exist in the registry
    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let req = test_request(Method::GET, &path, None);
    let resp = router.oneshot(req).await.expect("request failed");
    let json = assert_json_error(resp, StatusCode::NOT_FOUND).await;

    assert!(
        json.get("error").is_some(),
        "404 response should contain an error field, got: {json}"
    );
}

/// Tests MockLlmDriver call recording functionality.
#[tokio::test]
async fn test_mock_llm_driver_recording() {
    use crate::MockLlmDriver;
    use librefang_runtime::llm_driver::{CompletionRequest, LlmDriver};

    let driver = MockLlmDriver::new(vec!["回复1".into(), "回复2".into()]);

    let request = CompletionRequest {
        model: "test-model".into(),
        messages: std::sync::Arc::new(vec![]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: 100,
        temperature: 0.0,
        system: Some("test system prompt".into()),
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    };

    // First call
    let resp1 = driver.complete(request.clone()).await.unwrap();
    assert_eq!(resp1.text(), "回复1");

    // Second call
    let resp2 = driver.complete(request).await.unwrap();
    assert_eq!(resp2.text(), "回复2");

    // Verify call recording
    assert_eq!(driver.call_count(), 2);
    let calls = driver.recorded_calls();
    assert_eq!(calls[0].model, "test-model");
    assert_eq!(calls[0].system, Some("test system prompt".into()));
}

/// Tests building a kernel with custom config.
#[tokio::test(flavor = "multi_thread")]
async fn test_custom_config_kernel() {
    let app = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.language = "zh".into();
    }));

    // Verify that the custom configuration took effect
    assert_eq!(app.state.kernel.config_ref().language, "zh");
}

/// Tests the GET /api/version endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn test_version_endpoint() {
    let app = TestAppState::new();
    let router = app.router();

    let req = test_request(Method::GET, "/api/version", None);
    let resp = router.oneshot(req).await.expect("request failed");
    let json = assert_json_ok(resp).await;

    assert!(
        json.get("version").is_some(),
        "version endpoint should contain a version field, got: {json}"
    );
}

// -- POST / PUT / DELETE tests ------------------------------------------------

/// Tests POST /api/agents — creates an agent using manifest_toml.
#[tokio::test(flavor = "multi_thread")]
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
    let resp = router.oneshot(req).await.expect("request failed");
    let status = resp.status();

    // spawn should return 200 or 201 (with agent id)
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "spawn_agent should return 200/201, got: {status}"
    );
}

/// Tests DELETE /api/agents/{id} — deleting a nonexistent agent is idempotent (#3509).
///
/// 404 is reserved for the malformed-UUID case; a valid UUID that doesn't
/// resolve to an agent succeeds with `200 OK` and a tagged body so retried
/// deletes (network blip, dashboard double-click) don't surface a phantom
/// error. The previous "DELETE → 404" assertion contradicted that contract
/// and was the upstream cause of the `Test / *` red on `main`.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_nonexistent_agent_is_idempotent() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let req = test_request(Method::DELETE, &path, None);
    let resp = router.oneshot(req).await.expect("request failed");

    let json = assert_json_ok(resp).await;
    assert_eq!(
        json["status"], "already-deleted",
        "missing-agent DELETE must surface idempotency tag, got: {json}"
    );
}

/// Tests PUT /api/agents/{id}/model — setting model for a nonexistent agent should return an error.
#[tokio::test(flavor = "multi_thread")]
async fn test_set_model_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}/model");
    let body = serde_json::json!({ "model": "gpt-4" }).to_string();
    let req = test_request(Method::PUT, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("request failed");

    // Nonexistent agent should return a non-200 error status code
    let status = resp.status();
    assert!(
        status.is_client_error() || status.is_server_error(),
        "set_model for a nonexistent agent should return an error status code, got: {status}"
    );
}

/// Tests POST /api/agents/{id}/message — sending a message to a nonexistent agent should return an error.
#[tokio::test(flavor = "multi_thread")]
async fn test_send_message_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}/message");
    let body = serde_json::json!({ "message": "hello" }).to_string();
    let req = test_request(Method::POST, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("request failed");

    let status = resp.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
        "send_message for a nonexistent agent should return 404/400, got: {status}"
    );
}

/// Tests install → reload → assign → uninstall for a local registry skill.
#[tokio::test(flavor = "multi_thread")]
async fn test_skill_lifecycle_and_agent_assignment() {
    let app = TestAppState::new();
    seed_registry_skill(app.home_dir(), "fixture-skill").expect("seed registry skill");
    let router = app.router();

    let body = serde_json::json!({ "name": "fixture-skill" }).to_string();
    let req = test_request(Method::POST, "/api/skills/install", Some(&body));
    let resp = router.clone().oneshot(req).await.expect("install request");
    let json = assert_json_ok(resp).await;
    assert_eq!(json["status"], "installed");
    assert_eq!(json["name"], "fixture-skill");

    let req = test_request(Method::GET, "/api/skills/registry", None);
    let resp = router.clone().oneshot(req).await.expect("registry request");
    let json = assert_json_ok(resp).await;
    assert!(json["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skill| skill["name"] == "fixture-skill"));

    let body = serde_json::json!({ "manifest_toml": FIXTURE_AGENT_MANIFEST }).to_string();
    let req = test_request(Method::POST, "/api/agents", Some(&body));
    let resp = router.clone().oneshot(req).await.expect("spawn request");
    let agent = assert_json_status(resp, StatusCode::CREATED).await;
    let agent_id = agent["agent_id"].as_str().expect("agent_id");

    let body = serde_json::json!({ "skills": ["fixture-skill"] }).to_string();
    let req = test_request(
        Method::PUT,
        &format!("/api/agents/{agent_id}/skills"),
        Some(&body),
    );
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("assign skill request");
    let json = assert_json_ok(resp).await;
    assert_eq!(json["skills"], serde_json::json!(["fixture-skill"]));

    let req = test_request(Method::GET, &format!("/api/agents/{agent_id}/skills"), None);
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("get agent skills request");
    let json = assert_json_ok(resp).await;
    assert_eq!(json["assigned"], serde_json::json!(["fixture-skill"]));

    let body = serde_json::json!({ "name": "fixture-skill" }).to_string();
    let req = test_request(Method::POST, "/api/skills/uninstall", Some(&body));
    let resp = router.oneshot(req).await.expect("uninstall request");
    let json = assert_json_ok(resp).await;
    assert_eq!(json["status"], "uninstalled");
}

/// Tests workflow create → dry-run → run → list runs using a local OpenAI-compatible fixture.
#[tokio::test(flavor = "multi_thread")]
async fn test_workflow_lifecycle_executes_and_records_runs() {
    let base_url = spawn_openai_fixture_server("fixture workflow output").await;
    let app = TestAppState::new();
    let router = app.router();

    let manifest = agent_manifest_for_provider("fixture-openai", "fixture-model", &base_url);
    let body = serde_json::json!({ "manifest_toml": manifest }).to_string();
    let req = test_request(Method::POST, "/api/agents", Some(&body));
    let resp = router.clone().oneshot(req).await.expect("spawn request");
    let agent = assert_json_status(resp, StatusCode::CREATED).await;
    let agent_name = agent["name"].as_str().expect("agent name");

    let workflow = single_step_workflow("fixture-workflow", agent_name).to_string();
    let req = test_request(Method::POST, "/api/workflows", Some(&workflow));
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("create workflow request");
    let created = assert_json_status(resp, StatusCode::CREATED).await;
    let workflow_id = created["workflow_id"].as_str().expect("workflow id");

    let body = serde_json::json!({ "input": "hello workflow" }).to_string();
    let req = test_request(
        Method::POST,
        &format!("/api/workflows/{workflow_id}/dry-run"),
        Some(&body),
    );
    let resp = router.clone().oneshot(req).await.expect("dry-run request");
    assert_json_ok(resp).await;

    let req = test_request(
        Method::POST,
        &format!("/api/workflows/{workflow_id}/run"),
        Some(&body),
    );
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("run workflow request");
    let run = assert_json_ok(resp).await;
    assert_eq!(run["status"], "completed");
    assert_eq!(run["output"], "fixture workflow output");
    let run_id = run["run_id"].as_str().expect("run id");
    assert!(!run["step_results"].as_array().unwrap().is_empty());

    let req = test_request(
        Method::GET,
        &format!("/api/workflows/{workflow_id}/runs"),
        None,
    );
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("list runs request");
    let runs = assert_json_ok(resp).await;
    assert!(runs
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == run_id));
}

/// Tests clone validation and audit endpoint contracts.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_clone_validation_and_audit_contracts() {
    let app = TestAppState::new();
    let router = app.router();

    let body = serde_json::json!({ "manifest_toml": FIXTURE_AGENT_MANIFEST }).to_string();
    let req = test_request(Method::POST, "/api/agents", Some(&body));
    let resp = router.clone().oneshot(req).await.expect("spawn request");
    let agent = assert_json_status(resp, StatusCode::CREATED).await;
    let agent_id = agent["agent_id"].as_str().expect("agent_id");

    let body = serde_json::json!({ "new_name": "fixture-agent-clone" }).to_string();
    let req = test_request(
        Method::POST,
        &format!("/api/agents/{agent_id}/clone"),
        Some(&body),
    );
    let resp = router.clone().oneshot(req).await.expect("clone request");
    let clone = assert_json_status(resp, StatusCode::CREATED).await;
    assert_ne!(clone["agent_id"], agent["agent_id"]);

    let body = serde_json::json!({ "new_name": "" }).to_string();
    let req = test_request(
        Method::POST,
        &format!("/api/agents/{agent_id}/clone"),
        Some(&body),
    );
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("invalid clone request");
    assert_json_error(resp, StatusCode::BAD_REQUEST).await;

    app.state.kernel.audit().record(
        agent_id,
        librefang_runtime::audit::AuditAction::AgentSpawn,
        "agent lifecycle test",
        "ok",
    );

    let req = test_request(Method::GET, "/api/audit/recent?n=10", None);
    let resp = router
        .clone()
        .oneshot(req)
        .await
        .expect("audit recent request");
    let audit = assert_json_ok(resp).await;
    assert!(audit["entries"].as_array().unwrap().iter().any(|entry| {
        entry["agent_id"] == agent_id && entry["action"].as_str() == Some("AgentSpawn")
    }));

    let req = test_request(Method::GET, "/api/audit/verify", None);
    let resp = router.oneshot(req).await.expect("audit verify request");
    let verify = assert_json_ok(resp).await;
    assert_eq!(verify["valid"], true);
}

/// Tests PATCH /api/agents/{id} — updating a nonexistent agent should return an error.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_not_found() {
    let app = TestAppState::new();
    let router = app.router();

    let fake_id = uuid::Uuid::new_v4();
    let path = format!("/api/agents/{fake_id}");
    let body = serde_json::json!({ "name": "new-name" }).to_string();
    let req = test_request(Method::PATCH, &path, Some(&body));
    let resp = router.oneshot(req).await.expect("request failed");

    let status = resp.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
        "patch_agent for a nonexistent agent should return 404/400, got: {status}"
    );
}

// -- MockLlmDriver builder method tests ---------------------------------------

/// Tests MockLlmDriver's with_tokens and with_stop_reason custom settings.
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
        messages: std::sync::Arc::new(vec![]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    };

    let resp = driver.complete(request).await.unwrap();
    assert_eq!(
        resp.usage.input_tokens, 200,
        "input_tokens should be the custom value 200"
    );
    assert_eq!(
        resp.usage.output_tokens, 100,
        "output_tokens should be the custom value 100"
    );
    assert_eq!(
        resp.stop_reason,
        StopReason::MaxTokens,
        "stop_reason should be MaxTokens"
    );
}

// -- FailingLlmDriver tests ---------------------------------------------------

/// Tests that FailingLlmDriver always returns errors (for error handling scenarios).
#[tokio::test]
async fn test_failing_llm_driver() {
    use crate::FailingLlmDriver;
    use librefang_runtime::llm_driver::{CompletionRequest, LlmDriver};

    let driver = FailingLlmDriver::new("模拟的 API 错误");

    let request = CompletionRequest {
        model: "test-model".into(),
        messages: std::sync::Arc::new(vec![]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    };

    let result = driver.complete(request).await;
    assert!(
        result.is_err(),
        "FailingLlmDriver should always return an error"
    );

    let err = result.unwrap_err();
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("模拟的 API 错误"),
        "error message should contain the custom content, got: {err_msg}"
    );

    // FailingLlmDriver's is_configured should return false
    assert!(
        !driver.is_configured(),
        "FailingLlmDriver.is_configured() should return false"
    );
}
