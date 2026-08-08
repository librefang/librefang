//! Cross-user authorization coverage for agent, session, observability, and
//! trigger routes. Requests run through the real domain routers and auth
//! middleware so `AuthenticatedApiUser` extraction is exercised end to end.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const ALICE_KEY: &str = "alice-owner-scope-key";
const BOB_KEY: &str = "bob-owner-scope-key";
const ADMIN_KEY: &str = "admin-owner-scope-key";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let users = [
        ("Alice", "user", ALICE_KEY),
        ("Bob", "user", BOB_KEY),
        ("Admin", "admin", ADMIN_KEY),
    ];
    let mut configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in users {
        let hash = librefang_api::password_hash::hash_password(key).expect("hash test key");
        configs.push(UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        auth_users.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = "owner-scope-master-key".to_string();
        cfg.users = configs;
    }))
    .with_api_key("owner-scope-master-key")
    .with_user_api_keys(auth_users);
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
    };
    let app = Router::new()
        .nest("/api", routes::agents::router())
        .nest("/api", routes::workflows::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_authored(state: &AppState, author: &str) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("owner-scope-{}", uuid::Uuid::new_v4()),
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored test agent")
}

async fn request_status(
    app: &Router,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("route response")
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_read_agent_scoped_resources() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let created = h
        .state
        .kernel
        .create_agent_session(agent_id, Some("owner-scope-test"))
        .expect("create materialized session");
    let session_id: librefang_types::agent::SessionId = created["session_id"]
        .as_str()
        .expect("created session id")
        .parse()
        .expect("parse created session id");
    let aid = agent_id.to_string();
    let sid = session_id.to_string();
    let cases = vec![
        (Method::GET, format!("/api/agents/{aid}"), None),
        (Method::GET, format!("/api/agents/{aid}/runtime"), None),
        (Method::GET, format!("/api/agents/{aid}/tools"), None),
        (Method::GET, format!("/api/agents/{aid}/skills"), None),
        (Method::GET, format!("/api/agents/{aid}/mcp_servers"), None),
        (Method::GET, format!("/api/agents/{aid}/channels"), None),
        (Method::GET, format!("/api/agents/{aid}/files"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/files/AGENT.md"),
            None,
        ),
        (Method::GET, format!("/api/agents/{aid}/deliveries"), None),
        (Method::GET, format!("/api/agents/{aid}/traces"), None),
        (Method::GET, format!("/api/agents/{aid}/metrics"), None),
        (Method::GET, format!("/api/agents/{aid}/logs"), None),
        (Method::GET, format!("/api/agents/{aid}/session"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/session/context"),
            None,
        ),
        (Method::GET, format!("/api/agents/{aid}/sessions"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/stream"),
            None,
        ),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/export"),
            None,
        ),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/trajectory"),
            None,
        ),
    ];

    let mut failures = Vec::new();
    for (method, path, body) in cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        if status != StatusCode::NOT_FOUND {
            failures.push(format!("{path}: expected 404, got {status}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admin_agent_session_mutations_are_blocked_by_rbac_middleware() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let created = h
        .state
        .kernel
        .create_agent_session(agent_id, Some("mutation-rbac-test"))
        .expect("create materialized session");
    let session_id = created["session_id"].as_str().expect("session id");
    let export = h
        .state
        .kernel
        .export_session(agent_id, session_id.parse().expect("parse session id"))
        .expect("export initial session");
    let aid = agent_id.to_string();
    let cases = vec![
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions"),
            Some(serde_json::json!({})),
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/{session_id}/switch"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/import"),
            Some(serde_json::to_value(export).expect("serialize export")),
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/reset"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/reboot"),
            None,
        ),
        (Method::DELETE, format!("/api/agents/{aid}/history"), None),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/compact"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/{session_id}/stop"),
            None,
        ),
    ];
    for (method, path, body) in cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_routes_enforce_owner_read_and_rbac_mutations() {
    use librefang_kernel::triggers::TriggerPattern;

    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let trigger_id = h
        .state
        .kernel
        .register_trigger_with_target(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "owner-scope".to_string(),
            },
            "{{event}}".to_string(),
            0,
            None,
            Some(0),
            None,
            None,
        )
        .expect("register trigger");
    let status = request_status(
        &h.app,
        Method::GET,
        &format!("/api/triggers/{trigger_id}"),
        BOB_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for key in [ALICE_KEY, ADMIN_KEY] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/triggers/{trigger_id}"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let mutation_cases = [
        (
            Method::POST,
            "/api/triggers".to_string(),
            Some(serde_json::json!({
                "agent_id": agent_id.to_string(),
                "pattern": {"content_match": {"substring": "blocked"}}
            })),
        ),
        (
            Method::PATCH,
            format!("/api/triggers/{trigger_id}"),
            Some(serde_json::json!({"enabled": false})),
        ),
        (Method::DELETE, format!("/api/triggers/{trigger_id}"), None),
    ];
    for (method, path, body) in mutation_cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_and_admin_can_access_agent_observability() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    for key in [ALICE_KEY, ADMIN_KEY] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/agents/{agent_id}/metrics"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let missing = AgentId::new();
    for endpoint in ["traces", "logs"] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/agents/{missing}/{endpoint}"),
            ADMIN_KEY,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{endpoint}");
    }
}
