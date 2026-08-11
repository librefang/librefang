//! Cross-user authorization coverage for agent, session, observability, and trigger routes.
//! Requests run through the real domain routers and auth middleware so `AuthenticatedApiUser` extraction is exercised end to end.

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
        // `true`, not the test-harness-typical `false`: production's own `derive_require_auth_for_reads` (server.rs) auto-enables this whenever any authentication is configured, which this harness's `[[users]]` always does.
        // Every dashboard-read-public route (including bare `GET /api/agents`) must go through the real bearer check here, or `AuthenticatedApiUser` is never populated for those routes and the ownership assertions below would pass vacuously — see `non_admin_cannot_override_owner_filter_on_list_agents`.
        require_auth_for_reads: true,
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

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_clone_agent() {
    // `agent_clone` in `middleware::user_role_allows_request` deliberately lets any `User`-role caller POST `/clone` on an arbitrary agent id (unlike most mutations, which require Admin+), so the ownership boundary has to be enforced in the handler itself.
    // Bob cannot read the resulting clone back afterwards either way (it keeps Alice's `author`), but without this check he could still trigger unauthorized cloning of her agent by guessing/enumerating its UUID.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let aid = agent_id.to_string();

    let status = request_status(
        &h.app,
        Method::POST,
        &format!("/api/agents/{aid}/clone"),
        BOB_KEY,
        Some(serde_json::json!({"new_name": "bob-should-not-get-this"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let status = request_status(
        &h.app,
        Method::POST,
        &format!("/api/agents/{aid}/clone"),
        ALICE_KEY,
        Some(serde_json::json!({"new_name": format!("alice-clone-{}", uuid::Uuid::new_v4())})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_message_another_users_agent() {
    // `agent_message` in `middleware::user_role_allows_request` deliberately lets any `User`-role caller POST `/message` and `/message/stream` on an arbitrary agent id (unlike most mutations, which require Admin+), so the ownership boundary has to be enforced in the handler itself — the same shape as `non_owner_cannot_clone_agent`.
    // Without it a non-owner could drive a full LLM turn — tool execution and budget spend included — on another user's agent by guessing/enumerating its UUID.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let aid = agent_id.to_string();

    for path in [
        format!("/api/agents/{aid}/message"),
        format!("/api/agents/{aid}/message/stream"),
    ] {
        let status = request_status(
            &h.app,
            Method::POST,
            &path,
            BOB_KEY,
            Some(serde_json::json!({"message": "hello"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must 404 for a non-owner"
        );

        // Alice, the real owner, must clear the ownership check and reach the provider-auth check below it, never a 404.
        // The test harness has no provider configured, so this is deterministic without a real LLM call.
        let status = request_status(
            &h.app,
            Method::POST,
            &path,
            ALICE_KEY,
            Some(serde_json::json!({"message": "hello"})),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must not 404 for the owner"
        );
    }
}

fn spawn_cron_job(
    state: &AppState,
    agent_id: AgentId,
    name: &str,
) -> librefang_types::scheduler::CronJobId {
    let job = librefang_types::scheduler::CronJob {
        id: librefang_types::scheduler::CronJobId::new(),
        agent_id,
        name: name.to_string(),
        enabled: true,
        schedule: librefang_types::scheduler::CronSchedule::Cron {
            expr: "* * * * *".to_string(),
            tz: None,
        },
        action: librefang_types::scheduler::CronAction::AgentTurn {
            message: "owner-scope cron probe".to_string(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        },
        delivery: librefang_types::scheduler::CronDelivery::None,
        delivery_targets: Vec::new(),
        peer_id: None,
        session_mode: None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };
    state
        .kernel
        .cron()
        .add_job(job, false)
        .expect("register test cron job")
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_and_schedule_routes_enforce_owner_read() {
    // #6753 follow-up: `/api/cron/jobs*` and `/api/schedules*` carry the same cross-owner disclosure class (user-authored `message`/`prompt_template` content) this PR closed for `/api/triggers/*`, but the GET handlers had no `can_access_agent` check at all.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let job_id = spawn_cron_job(&h.state, agent_id, "owner-scope-cron-job");
    let aid = agent_id.to_string();

    // Detail reads: non-owner gets 404, owner and admin get 200.
    for path in [
        format!("/api/cron/jobs/{job_id}"),
        format!("/api/cron/jobs/{job_id}/status"),
        format!("/api/schedules/{job_id}"),
    ] {
        let status = request_status(&h.app, Method::GET, &path, BOB_KEY, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        for key in [ALICE_KEY, ADMIN_KEY] {
            let status = request_status(&h.app, Method::GET, &path, key, None).await;
            assert_eq!(status, StatusCode::OK, "{path} as {key}");
        }
    }

    // Filtered list (?agent_id=): non-owner gets an empty list, not the job.
    let status_and_body = |bearer: &'static str, path: String| {
        let app = h.app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .header("authorization", format!("Bearer {bearer}"))
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("route response");
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("read body");
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let (status, body) = status_and_body(BOB_KEY, format!("/api/cron/jobs?agent_id={aid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["total"],
        serde_json::json!(0),
        "Bob must not see Alice's cron job"
    );

    // Unfiltered list: non-owner's result must not contain the other user's job.
    let (status, body) = status_and_body(BOB_KEY, "/api/cron/jobs".to_string()).await;
    assert_eq!(status, StatusCode::OK);
    let jobs = body["jobs"].as_array().expect("jobs[]");
    assert!(
        jobs.iter()
            .all(|j| j["id"] != serde_json::json!(job_id.to_string())),
        "Bob's unfiltered /api/cron/jobs must not include Alice's job"
    );

    let (status, body) = status_and_body(BOB_KEY, "/api/schedules".to_string()).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items[]");
    assert!(
        items
            .iter()
            .all(|j| j["id"] != serde_json::json!(job_id.to_string())),
        "Bob's unfiltered /api/schedules must not include Alice's job"
    );

    // Owner and admin still see it in the unfiltered lists.
    for key in [ALICE_KEY, ADMIN_KEY] {
        let (status, body) = status_and_body(key, "/api/cron/jobs".to_string()).await;
        assert_eq!(status, StatusCode::OK);
        let jobs = body["jobs"].as_array().expect("jobs[]");
        assert!(
            jobs.iter()
                .any(|j| j["id"] == serde_json::json!(job_id.to_string())),
            "{key} should still see the job in /api/cron/jobs"
        );
    }
}

async fn get_json(app: &Router, path: &str, bearer: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("route response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admin_cannot_override_owner_filter_on_list_agents() {
    // `list_agents` only auto-injected `?owner=<caller>` when the query param was absent — a non-admin caller supplying `?owner=<someone-else>` explicitly was trusted as-is, defeating the ownership scoping this PR enforces on every other agent-scoped route.
    let h = boot().await;
    let alice_agent = spawn_authored(&h.state, "Alice");

    // Bob explicitly asks for Alice's agents — must still be scoped to Bob, not Alice.
    let (status, body) = get_json(&h.app, "/api/agents?owner=Alice", BOB_KEY).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items[]");
    assert!(
        items
            .iter()
            .all(|a| a["id"] != serde_json::json!(alice_agent.to_string())),
        "Bob must not see Alice's agent even when explicitly requesting ?owner=Alice: {body}"
    );

    // Alice herself, and an Admin explicitly filtering by her name, still see it.
    for key in [ALICE_KEY, ADMIN_KEY] {
        let (status, body) = get_json(&h.app, "/api/agents?owner=Alice", key).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items[]");
        assert!(
            items
                .iter()
                .any(|a| a["id"] == serde_json::json!(alice_agent.to_string())),
            "{key} should still see Alice's agent via ?owner=Alice: {body}"
        );
    }
}
