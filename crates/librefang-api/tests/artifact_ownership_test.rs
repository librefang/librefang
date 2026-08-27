//! Integration coverage for artifact ownership (#7744).
//!
//! Every test here drives the production `routes::workflows::router()` behind the real `middleware::auth` layer, so `AuthenticatedApiUser` is populated the way it is in production rather than injected.
//! That is what makes these regression tests instead of tautologies: the recorded owner is read back through an independent `GET`, so a handler that stamps a principal it then fails to persist — or a `PUT` that quietly re-owns what it edits — fails here even though the write returned the right status.
//!
//! The four properties asserted, in the order they matter:
//!
//! 1. A create records the **authenticated** caller, not the request body.
//! 2. A body field named `owner` cannot choose the owner — the same class of hole #7884 closed on `POST /api/agents/{id}/message`.
//! 3. An edit is not a transfer: `PUT /api/workflows/{id}` carries the stored owner over unchanged, even when a different, higher-privileged user performs it.
//! 4. `None` means unowned and visible to all, and `config.toml: default_owner` is what an unauthenticated deployment falls back to.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::{GroupConfig, UserConfig};
use librefang_types::principal::Principal;
use std::sync::Arc;
use tower::ServiceExt;

// Both users hold the `admin` role: `user_role_allows_request` gates every
// workflow / cron / schedule mutation at `Admin` and above, so a `user`-role
// bearer never reaches the handler whose stamping is under test. They are still
// two distinct identities, which is all the ownership assertions need — see
// `editing_a_workflow_is_not_a_transfer`, where the point is that the *editor*
// is not the *owner*, not that the editor outranks them.
const ALICE_KEY: &str = "alice-ownership-key";
const ADMIN_KEY: &str = "admin-ownership-key";
const MASTER_KEY: &str = "ownership-master-key";

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

/// An authenticated deployment with two distinct `admin` identities, plus an
/// `oncall` group so the reverse `owner_name` lookup has both arms to resolve
/// against.
async fn boot_authenticated(default_owner: Option<&str>) -> Harness {
    boot(true, default_owner).await
}

/// An unauthenticated deployment (`allow_no_auth`), which is what a single-user
/// or loopback install looks like. The middleware admits the caller as the
/// synthetic `root` sentinel, which `AuthenticatedApiUser::owner_principal`
/// declines to treat as a principal, so the create handlers fall through to
/// `default_owner`.
async fn boot_open(default_owner: Option<&str>) -> Harness {
    boot(false, default_owner).await
}

async fn boot(authenticated: bool, default_owner: Option<&str>) -> Harness {
    let mut configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in [("Alice", "admin", ALICE_KEY), ("Admin", "admin", ADMIN_KEY)] {
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

    // `allow_no_auth` only engages when there is genuinely nothing to
    // authenticate against — no master key, no `[[users]]` — so the open
    // harness declares neither. Both are configured for the authenticated one.
    if !authenticated {
        configs.clear();
        auth_users.clear();
    }

    let owner_spec = default_owner.map(str::to_string);
    let cfg_users = configs.clone();
    let master = if authenticated { MASTER_KEY } else { "" };
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = master.to_string();
        cfg.users = cfg_users;
        cfg.groups = vec![GroupConfig {
            name: "oncall".to_string(),
            members: vec!["Alice".to_string()],
            ..Default::default()
        }];
        cfg.default_owner = owner_spec;
    }))
    .with_api_key(master)
    .with_user_api_keys(auth_users);
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: authenticated,
        allow_no_auth: !authenticated,
        audit_log: Some(state.kernel.audit().clone()),
    };
    let app = Router::new()
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

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(b) = bearer {
        builder = builder.header("authorization", format!("Bearer {b}"));
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).expect("build request"))
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

/// A minimal but valid `POST /api/workflows` payload, plus whatever extra keys
/// the caller wants to smuggle in.
fn workflow_payload(name: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": name,
        "description": "ownership probe",
        "steps": [{ "name": "one", "agent_name": "writer", "prompt": "{{input}}" }],
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            body[k] = v.clone();
        }
    }
    body
}

async fn create_workflow(
    h: &Harness,
    bearer: Option<&str>,
    payload: serde_json::Value,
) -> serde_json::Value {
    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/workflows",
        bearer,
        Some(payload),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    body
}

// ---------------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_workflow_records_the_authenticated_caller_as_its_owner() {
    let h = boot_authenticated(None).await;
    let created = create_workflow(
        &h,
        Some(ALICE_KEY),
        workflow_payload("owned-by-alice", serde_json::json!({})),
    )
    .await;
    let id = created["workflow_id"].as_str().expect("workflow_id");

    // Independent read, per the integration-test rule: the create response
    // echoing the owner proves the handler computed it, not that it stored it.
    let (status, body) = send(
        &h.app,
        Method::GET,
        &format!("/api/workflows/{id}"),
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["owner"],
        serde_json::to_value(Principal::user_named("Alice")).unwrap(),
        "the workflow must belong to the authenticated caller"
    );
    assert_eq!(
        body["owner_name"], "Alice",
        "the recorded principal must resolve back to the name declared in `[[users]]`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_named_in_the_request_body_cannot_choose_the_owner() {
    // The security property, and the reason the handler reads the auth
    // extension rather than `req`: this is the same shape as the hole #7884
    // closed on `POST /api/agents/{id}/message`.
    let h = boot_authenticated(None).await;
    let forged = serde_json::json!({
        "owner": { "kind": "group", "id": "00000000-0000-0000-0000-000000000000" },
        "owner_name": "somebody-else",
    });
    let created = create_workflow(
        &h,
        Some(ALICE_KEY),
        workflow_payload("forged-owner", forged),
    )
    .await;
    let id = created["workflow_id"].as_str().expect("workflow_id");

    let (_, body) = send(
        &h.app,
        Method::GET,
        &format!("/api/workflows/{id}"),
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(
        body["owner"],
        serde_json::to_value(Principal::user_named("Alice")).unwrap(),
        "the body must not be able to name the owner"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_a_workflow_is_not_a_transfer() {
    // Admin outranks Alice and may PATCH her workflow, but an edit must not
    // move it — otherwise anyone who can edit can re-own, and the recorded
    // owner is worth less than the log line it replaced.
    let h = boot_authenticated(None).await;
    let created = create_workflow(
        &h,
        Some(ALICE_KEY),
        workflow_payload("edited-by-admin", serde_json::json!({})),
    )
    .await;
    let id = created["workflow_id"]
        .as_str()
        .expect("workflow_id")
        .to_string();

    let (status, body) = send(
        &h.app,
        Method::PUT,
        &format!("/api/workflows/{id}"),
        Some(ADMIN_KEY),
        Some(serde_json::json!({
            "description": "rewritten by admin",
            // …and an explicit attempt to take it while editing.
            "owner": { "kind": "user", "id": UserId::from_name("Admin").0 },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let (_, after) = send(
        &h.app,
        Method::GET,
        &format!("/api/workflows/{id}"),
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(after["description"], "rewritten by admin");
    assert_eq!(
        after["owner"],
        serde_json::to_value(Principal::user_named("Alice")).unwrap(),
        "an edit must carry the stored owner over unchanged"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unauthenticated_create_falls_back_to_the_configured_default_owner() {
    let h = boot_open(Some("group:oncall")).await;
    let created = create_workflow(
        &h,
        None,
        workflow_payload("default-owned", serde_json::json!({})),
    )
    .await;
    let id = created["workflow_id"].as_str().expect("workflow_id");

    let (_, body) = send(
        &h.app,
        Method::GET,
        &format!("/api/workflows/{id}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        body["owner"],
        serde_json::to_value(Principal::group_named("oncall")).unwrap(),
        "`default_owner` is the fallback when no caller is authenticated"
    );
    assert_eq!(
        body["owner_name"], "oncall",
        "a group principal resolves back through `[[groups]]`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_nothing_configured_a_workflow_is_recorded_unowned() {
    // The stated meaning of `None` — unowned, visible to all — asserted rather
    // than left as an accident of `#[serde(default)]`. Nothing restricts on
    // ownership in this increment, so an unowned workflow behaves exactly as it
    // did before the field existed.
    let h = boot_open(None).await;
    let created =
        create_workflow(&h, None, workflow_payload("unowned", serde_json::json!({}))).await;
    let id = created["workflow_id"].as_str().expect("workflow_id");

    let (status, body) = send(
        &h.app,
        Method::GET,
        &format!("/api/workflows/{id}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["owner"].is_null());
    assert!(body["owner_name"].is_null());
    // …and it is still readable, which is what "visible to all" means here.
    assert_eq!(body["name"], "unowned");
}

// ---------------------------------------------------------------------------
// Cron jobs
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_cron_job_records_the_authenticated_caller_as_its_owner() {
    let h = boot_authenticated(None).await;
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(librefang_types::agent::AgentManifest {
            name: format!("ownership-cron-{}", uuid::Uuid::new_v4()),
            author: "Alice".to_string(),
            ..Default::default()
        })
        .expect("spawn test agent");

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/cron/jobs",
        Some(ALICE_KEY),
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "name": "owned-cron",
            "schedule": { "kind": "cron", "expr": "0 * * * *" },
            "action": { "kind": "agent_turn", "message": "tick" },
            // Smuggled, and must be ignored: the body is forwarded to
            // `cron_create` whole.
            "owner": { "kind": "group", "id": "00000000-0000-0000-0000-000000000000" },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "cron create failed: {body}");
    assert_eq!(
        body["owner"],
        serde_json::json!(Principal::user_named("Alice").to_string())
    );

    let (status, listed) = send(
        &h.app,
        Method::GET,
        &format!("/api/cron/jobs?agent_id={agent_id}"),
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let job = listed["jobs"]
        .as_array()
        .expect("jobs array")
        .iter()
        .find(|j| j["name"] == "owned-cron")
        .expect("the created job must be listed");
    assert_eq!(
        job["owner"],
        serde_json::to_value(Principal::user_named("Alice")).unwrap(),
        "the owner must survive the scheduler's store, not just the response"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_schedule_records_the_authenticated_caller_as_its_owner() {
    // `/api/schedules` builds a `CronJob` by hand rather than going through
    // `cron_create`, so it is a second stamp site and needs its own assertion.
    let h = boot_authenticated(None).await;
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(librefang_types::agent::AgentManifest {
            name: format!("ownership-sched-{}", uuid::Uuid::new_v4()),
            author: "Alice".to_string(),
            ..Default::default()
        })
        .expect("spawn test agent");

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/schedules",
        Some(ALICE_KEY),
        Some(serde_json::json!({
            "name": "owned-schedule",
            "cron": "0 * * * *",
            "agent_id": agent_id.to_string(),
            "message": "tick",
        })),
    )
    .await;
    assert!(
        status.is_success(),
        "schedule create failed: {status} {body}"
    );

    let (status, listed) = send(
        &h.app,
        Method::GET,
        &format!("/api/cron/jobs?agent_id={agent_id}"),
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let job = listed["jobs"]
        .as_array()
        .expect("jobs array")
        .iter()
        .find(|j| j["name"] == "owned-schedule")
        .expect("the created schedule must be listed as a cron job");
    assert_eq!(
        job["owner"],
        serde_json::to_value(Principal::user_named("Alice")).unwrap()
    );
}
