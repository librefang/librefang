//! Owner-scoping for `GET /api/agents/{id}/manifest-history`.
//!
//! A snapshot is the agent's entire `agent.toml` — `model.system_prompt`,
//! `capabilities`, `resources` budgets, tool / skill / MCP allowlists, `metadata`.
//! That is strictly more of another user's agent than the sibling reads in the same
//! module (`get_agent_traces`, `agent_metrics`, `agent_logs`) deliberately withhold,
//! and the auth middleware does not cover it: `user_role_allows_request` admits any
//! authenticated role on a GET and `min_role_for_privileged_get` has no pattern for
//! this path. The handler originally checked only that the id resolved in the
//! registry, so a Viewer key could read the full config history of an agent authored
//! by someone else.
//!
//! Run: cargo test -p librefang-api --test agent_manifest_history_authz_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::AuthenticatedApiUser;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::agents::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

fn spawn_owned_by(state: &Arc<AppState>, name: &str, author: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        author: author.to_string(),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

fn user(name: &str, role: UserRole) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role,
        user_id: UserId::from_name(name),
    }
}

async fn get_history(
    h: &Harness,
    id: &AgentId,
    as_user: Option<AuthenticatedApiUser>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/agents/{id}/manifest-history"))
        .body(Body::empty())
        .unwrap();
    if let Some(u) = as_user {
        request.extensions_mut().insert(u);
    }
    let response = h.app.clone().oneshot(request).await.expect("oneshot");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read response body");
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, body)
}

/// The finding: a Viewer key reading someone else's agent's whole config history.
///
/// 404 rather than 403 is deliberate and matches `can_access_agent`'s contract — it
/// is what stops id enumeration from distinguishing "exists but not yours" from
/// "does not exist".
#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_cannot_read_the_history_of_an_agent_it_does_not_author() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");

    let (status, body) = get_history(&h, &id, Some(user("mallory", UserRole::Viewer))).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-author Viewer must not receive another user's manifest snapshots, got body: {body}"
    );
    assert!(
        body.get("versions").is_none(),
        "no snapshot payload may leak on the denied path: {body}"
    );
}

/// The author of the agent is exactly who the pane is for, so scoping must not
/// close the endpoint to them.
#[tokio::test(flavor = "multi_thread")]
async fn the_author_can_read_its_own_agents_history() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");

    let (status, body) = get_history(&h, &id, Some(user("alice", UserRole::Viewer))).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.get("versions").is_some_and(|v| v.is_array()),
        "the author must still get the versions array: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_can_read_any_agents_history() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");

    let (status, body) = get_history(&h, &id, Some(user("root", UserRole::Admin))).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.get("versions").is_some_and(|v| v.is_array()), "{body}");
}

/// The loopback / no-auth deployment mode carries no `AuthenticatedApiUser`, and the
/// existing API compatibility contract keeps it allowed. Pinned so tightening the
/// scoping does not silently break single-user installs.
#[tokio::test(flavor = "multi_thread")]
async fn an_unauthenticated_trusted_request_is_still_served() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");

    let (status, body) = get_history(&h, &id, None).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.get("versions").is_some_and(|v| v.is_array()), "{body}");
}
