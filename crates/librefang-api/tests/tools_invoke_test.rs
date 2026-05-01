//! Integration tests for POST /api/tools/{name}/invoke.
//!
//! Each test boots a real kernel in a tempdir, installs a focused router
//! that mounts only the invoke route, and hits it via `tower::ServiceExt`.
//! The tests target the security-critical branches of the handler so a
//! future change that silently weakens any of them is caught:
//!
//!   - endpoint disabled → 403
//!   - tool not in allowlist → 403
//!   - unknown tool name → 404
//!   - approval-gated tool without `?agent_id=` → 400
//!   - malformed `?agent_id=` → 400
//!   - allowlisted non-approval tool → 200

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn build_harness(tool_invoke: librefang_types::config::ToolInvokeConfig) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".into(),
            model: "test-model".into(),
            api_key_env: "OLLAMA_API_KEY".into(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        cfg.tool_invoke = tool_invoke;
    }));

    let state = test.state.clone();
    let app = Router::new()
        .route(
            "/api/tools/{name}/invoke",
            axum::routing::post(routes::invoke_tool),
        )
        .with_state(state.clone());

    Harness {
        app,
        state,
        _test: test,
    }
}

async fn invoke(
    app: &Router,
    name: &str,
    agent_id: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut uri = format!("/api/tools/{name}/invoke");
    if let Some(id) = agent_id {
        uri.push_str(&format!("?agent_id={id}"));
    }
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("router oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_disabled_returns_403() {
    let h = build_harness(librefang_types::config::ToolInvokeConfig::default()).await;
    let (status, _) = invoke(&h.app, "web_search", None, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_tool_not_in_allowlist_returns_403() {
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["notify_owner".into()],
    })
    .await;
    let (status, _) = invoke(&h.app, "web_search", None, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_unknown_tool_returns_404() {
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["*".into()],
    })
    .await;
    let (status, _) = invoke(
        &h.app,
        "no_such_tool_exists_xyz",
        None,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_approval_gated_without_agent_id_returns_400() {
    // `shell_exec` is in the default `require_approval` list.
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["shell_exec".into()],
    })
    .await;
    let (status, _) = invoke(
        &h.app,
        "shell_exec",
        None,
        serde_json::json!({"command": "echo hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_malformed_agent_id_returns_400() {
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["notify_owner".into()],
    })
    .await;
    let (status, _) = invoke(
        &h.app,
        "notify_owner",
        Some("not-a-uuid"),
        serde_json::json!({"reason": "r", "summary": "s"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_allowlisted_non_approval_tool_succeeds() {
    // `notify_owner` does not require approval and succeeds without any
    // channel wiring (it returns a structured owner_notice in ToolResult).
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["notify_owner".into()],
    })
    .await;
    let (status, body) = invoke(
        &h.app,
        "notify_owner",
        None,
        serde_json::json!({"reason": "test", "summary": "smoke"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["is_error"], false, "body={body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_writes_audit_entry() {
    // Every direct invocation bypasses the agent loop's audit record, so the
    // handler must emit its own. Verify: on a successful call we get a
    // ToolInvoke entry tagged with the caller_agent_id, detail = tool name,
    // outcome starting with "ok".
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["notify_owner".into()],
    })
    .await;
    let before = h.state.kernel.audit().len();
    let agent_id = uuid::Uuid::new_v4().to_string();
    let (status, _) = invoke(
        &h.app,
        "notify_owner",
        Some(&agent_id),
        serde_json::json!({"reason": "r", "summary": "s"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after = h.state.kernel.audit().len();
    assert_eq!(
        after,
        before + 1,
        "exactly one audit entry should be appended"
    );
    let entry = h
        .state
        .kernel
        .audit()
        .recent(1)
        .into_iter()
        .next()
        .expect("at least one audit entry");
    // AuditAction does not implement PartialEq — match instead.
    assert!(
        matches!(
            entry.action,
            librefang_runtime::audit::AuditAction::ToolInvoke
        ),
        "expected ToolInvoke action, got {:?}",
        entry.action
    );
    assert_eq!(entry.detail, "notify_owner");
    assert_eq!(entry.agent_id, agent_id);
    assert!(
        entry.outcome.starts_with("ok"),
        "outcome should start with ok, got: {}",
        entry.outcome
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_invoke_file_read_uses_plumbed_workspace_root() {
    // Guards the sandbox-context plumbing: if `workspace_root` is ever
    // silently reverted to None in the handler, `file_read` returns
    // "Workspace sandbox not configured" and this test flips to a 400.
    let h = build_harness(librefang_types::config::ToolInvokeConfig {
        enabled: true,
        allowlist: vec!["file_read".into()],
    })
    .await;

    // `effective_workspaces_dir()` defaults to `home_dir.join("workspaces")`,
    // which the harness rooted at `tmp.path()`. Seed a file there and read it
    // back through the REST endpoint.
    let workspace_root = h.state.kernel.config_snapshot().effective_workspaces_dir();
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let file_path = workspace_root.join("hello.txt");
    std::fs::write(&file_path, "integration-ok").expect("seed test file");

    let (status, body) = invoke(
        &h.app,
        "file_read",
        None,
        serde_json::json!({"path": "hello.txt"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["is_error"], false, "body={body}");
    assert_eq!(
        body["content"].as_str(),
        Some("integration-ok"),
        "body={body}"
    );
}
