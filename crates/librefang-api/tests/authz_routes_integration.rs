//! Integration tests for the admin-only authz routes mounted under
//! `/api/authz/*` from `crates/librefang-api/src/routes/authz.rs`.
//!
//! Covers two endpoints:
//!   * `GET /api/authz/effective/{user_id}` — RBAC effective-permissions snapshot.
//!   * `GET /api/authz/check`               — user-policy-only decision query.
//!
//! Both endpoints are gated by `require_admin`: anonymous callers and
//! Viewer/User roles are denied; only Admin+ proceeds. The kernel boots
//! from a `MockKernelBuilder` config that seeds users + a per-user
//! `tool_policy`; we inject a synthetic `AuthenticatedApiUser` extension
//! when a request needs to clear the gate, since the bare router is
//! mounted without the auth middleware (mirrors `users_test.rs`).
//!
//! Filed against issue #3571 — partial coverage for the authz slice.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::AuthenticatedApiUser;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use librefang_types::user_policy::UserToolPolicy;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

fn boot_with_seed_users(seed: Vec<UserConfig>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        cfg.users = seed;
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::authz::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

/// Build a request with an injected `AuthenticatedApiUser` extension —
/// this bypasses the auth middleware (which is not mounted in this
/// harness) and lets the handler's `require_admin` see a caller of the
/// requested role.
fn req(method: Method, uri: &str, api_user: Option<AuthenticatedApiUser>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    if let Some(u) = api_user {
        request.extensions_mut().insert(u);
    }
    request
}

fn admin_user(name: &str) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role: UserRole::Admin,
        user_id: UserId::from_name(name),
    }
}

fn viewer_user(name: &str) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role: UserRole::Viewer,
        user_id: UserId::from_name(name),
    }
}

async fn run(h: &Harness, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = h.app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn seed_user(name: &str, role: &str) -> UserConfig {
    UserConfig {
        name: name.into(),
        role: role.into(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// /api/authz/effective/{user_id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn authz_effective_anonymous_caller_is_forbidden() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, body) = run(&h, req(Method::GET, "/api/authz/effective/Alice", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("admin"),
        "error should mention Admin role: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_effective_viewer_role_is_forbidden() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/effective/Alice",
            Some(viewer_user("watcher")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_effective_admin_returns_snapshot_by_name() {
    let seed = UserConfig {
        name: "Alice".into(),
        role: "user".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec!["shell_exec".into()],
        }),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/effective/Alice",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "Alice");
    assert_eq!(body["role"], "user");
    assert_eq!(
        body["tool_policy"]["allowed_tools"],
        serde_json::json!(["web_search"])
    );
    assert_eq!(
        body["tool_policy"]["denied_tools"],
        serde_json::json!(["shell_exec"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_effective_admin_resolves_uuid_user_id() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let alice_id = UserId::from_name("Alice");
    let uri = format!("/api/authz/effective/{alice_id}");
    let (status, body) = run(&h, req(Method::GET, &uri, Some(admin_user("admin")))).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "Alice");
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_effective_unknown_user_returns_404() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/effective/Ghost",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("no user"),
        "error must mention unknown user: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/authz/check
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_anonymous_caller_is_forbidden() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, _body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice&action=web_search",
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_viewer_role_is_forbidden() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, _body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice&action=web_search",
            Some(viewer_user("watcher")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_unknown_user_returns_404() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Ghost&action=web_search",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_allow_for_user_with_matching_policy() {
    let seed = UserConfig {
        name: "Alice".into(),
        role: "user".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec![],
        }),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice&action=web_search",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["user"], "Alice");
    assert_eq!(body["action"], "web_search");
    assert!(
        body["channel"].is_null(),
        "channel should echo null: {body:?}"
    );
    assert_eq!(body["decision"], "allow");
    assert_eq!(body["allowed"], true);
    assert!(
        body["reason"].is_null(),
        "reason must be null on allow: {body:?}"
    );
    assert_eq!(
        body["scope"], "user_policy_only",
        "scope marker MUST always be user_policy_only — runtime gate may differ"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_deny_for_explicitly_blocked_tool() {
    // A `viewer` whose policy explicitly denies `shell_exec` will
    // surface as `Deny` from Layer A — Layer B (admin escalation) does
    // not relax explicit denies, only fills in unspecified actions.
    let seed = UserConfig {
        name: "Alice".into(),
        role: "viewer".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec![],
            denied_tools: vec!["shell_exec".into()],
        }),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice&action=shell_exec",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["allowed"], false);
    assert!(
        body["reason"].as_str().is_some_and(|s| !s.is_empty()),
        "deny must carry a non-empty reason: {body:?}"
    );
    assert_eq!(body["scope"], "user_policy_only");
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_echoes_channel_query_param() {
    let seed = UserConfig {
        name: "Alice".into(),
        role: "user".into(),
        tool_policy: Some(UserToolPolicy {
            allowed_tools: vec!["web_search".into()],
            denied_tools: vec![],
        }),
        ..Default::default()
    };
    let h = boot_with_seed_users(vec![seed]);
    let (status, body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice&action=web_search&channel=telegram",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["channel"], "telegram");
}

#[tokio::test(flavor = "multi_thread")]
async fn authz_check_missing_required_query_param_returns_400() {
    let h = boot_with_seed_users(vec![seed_user("Alice", "user")]);
    // No `action=` — axum's Query extractor surfaces a 400 before the
    // handler runs.
    let (status, _body) = run(
        &h,
        req(
            Method::GET,
            "/api/authz/check?user=Alice",
            Some(admin_user("admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
