//! End-to-end coverage for the agent manifest version history:
//! `GET /api/agents/{id}/manifest-history` and
//! `POST /api/agents/{id}/manifest-history/{version_id}/restore`.
//!
//! The harness boots a real `LibreFangKernel` through `MockKernelBuilder` and
//! mounts the production agent router, so persistence goes through the same
//! `update_manifest` / `persist_manifest_to_disk` funnel the daemon uses.
//!
//! Run: cargo test -p librefang-api --test agent_manifest_history_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::AuthenticatedApiUser;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole;
use librefang_kernel::provisioning::{AGENTS_SUBDIR, PROVISIONING_PATH_ENV};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use std::path::Path;
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
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: name.to_string(),
            source_template: None,
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn_agent")
}

fn manifest_with_prompt(state: &Arc<AppState>, id: AgentId, prompt: &str) -> AgentManifest {
    let mut manifest = state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent registered")
        .manifest
        .clone();
    manifest.model.system_prompt = prompt.to_string();
    manifest
}

fn user(name: &str, role: UserRole) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role,
        user_id: UserId::from_name(name),
    }
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    as_user: Option<AuthenticatedApiUser>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
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

async fn get_history(
    h: &Harness,
    id: &AgentId,
    as_user: Option<AuthenticatedApiUser>,
) -> (StatusCode, serde_json::Value) {
    send(
        h,
        Method::GET,
        &format!("/api/agents/{id}/manifest-history"),
        as_user,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn list_returns_recorded_versions_newest_first() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "history-list", "alice");

    // Two control-plane writes through the same funnel the API uses.
    let mut first = manifest_with_prompt(&h.state, id, "prompt v1");
    first.description = "first".to_string();
    h.state
        .kernel
        .update_manifest(id, first, "model")
        .expect("first persist");
    let mut second = manifest_with_prompt(&h.state, id, "prompt v2");
    second.description = "second".to_string();
    h.state
        .kernel
        .update_manifest(id, second, "skills")
        .expect("second persist");

    let (status, body) = get_history(&h, &id, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let versions = body["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 2, "body: {body}");
    assert_eq!(versions[0]["change_source"], "skills", "newest first");
    assert_eq!(versions[1]["change_source"], "model");
    assert_eq!(versions[0]["agent_id"], id.to_string());
    assert!(
        versions[0]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("prompt v2"),
        "the snapshot carries the full TOML: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_rejects_invalid_agent_id_and_non_numeric_limit() {
    let h = boot();

    let (status, body) = get_history(&h, &AgentId(uuid::Uuid::new_v4()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");

    let (status, body) = send(
        &h,
        Method::GET,
        "/api/agents/not-a-uuid/manifest-history",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"]["code"] == "invalid_agent_id", "body: {body}");

    let some_id = spawn_owned_by(&h.state, "history-bad-limit", "alice");
    let (status, body) = send(
        &h,
        Method::GET,
        &format!("/api/agents/{some_id}/manifest-history?limit=abc"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rolls_back_the_manifest_and_records_the_restore() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "history-restore", "alice");

    let mut v1 = manifest_with_prompt(&h.state, id, "prompt v1");
    v1.description = "v1".to_string();
    h.state
        .kernel
        .update_manifest(id, v1.clone(), "model")
        .expect("v1 persist");
    let mut v2 = manifest_with_prompt(&h.state, id, "prompt v2");
    v2.description = "v2".to_string();
    h.state
        .kernel
        .update_manifest(id, v2, "skills")
        .expect("v2 persist");

    let (_, body) = get_history(&h, &id, None).await;
    let version_id = body["versions"][1]["id"].as_i64().expect("v1 id");

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{id}/manifest-history/{version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["restored_version_id"], version_id);

    let restored = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent still registered")
        .manifest
        .clone();
    assert_eq!(
        restored.model.system_prompt, v1.model.system_prompt,
        "restore must apply the stored snapshot"
    );
    assert_eq!(restored.description, "v1");

    let (_, body) = get_history(&h, &id, None).await;
    let versions = body["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 3, "body: {body}");
    assert_eq!(
        versions[0]["change_source"], "restore",
        "the restore itself is a recorded version"
    );
    assert!(
        versions[0]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("prompt v1"),
        "the newest row holds the restored content: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_maps_missing_and_mismatched_versions() {
    let h = boot();
    let alice_agent = spawn_owned_by(&h.state, "history-alice", "alice");
    let bob_agent = spawn_owned_by(&h.state, "history-bob", "bob");

    let mut v1 = manifest_with_prompt(&h.state, alice_agent, "alice v1");
    v1.description = "alice v1".to_string();
    h.state
        .kernel
        .update_manifest(alice_agent, v1, "model")
        .expect("persist");
    let mut bob_v1 = manifest_with_prompt(&h.state, bob_agent, "bob v1");
    bob_v1.description = "bob v1".to_string();
    h.state
        .kernel
        .update_manifest(bob_agent, bob_v1, "model")
        .expect("persist");

    let (_, body) = get_history(&h, &alice_agent, None).await;
    let alice_version_id = body["versions"][0]["id"].as_i64().expect("alice id");
    let (_, body) = get_history(&h, &bob_agent, None).await;
    let bob_version_id = body["versions"][0]["id"].as_i64().expect("bob id");

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{alice_agent}/manifest-history/999999/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "version_not_found");

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{bob_agent}/manifest-history/{alice_version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["error"]["code"], "version_mismatch");

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/not-a-uuid/manifest-history/{bob_version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["error"]["code"], "invalid_agent_id");
}

/// A snapshot the server cannot deserialize must fail as its own static 500
/// rather than half-applying through the kernel or echoing parser internals.
#[tokio::test(flavor = "multi_thread")]
async fn restore_reports_a_corrupt_stored_version_as_a_static_500() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "history-corrupt", "alice");

    // A producer never writes unparseable TOML, but a hand-edited database or
    // a row from an older schema can still hold one. Inserting it through the
    // store is the only way to reach the parse guard.
    let store =
        librefang_memory::ManifestVersionStore::new(h.state.kernel.memory_substrate().pool());
    store
        .record_version(
            &id.to_string(),
            "history-corrupt",
            "name = [not-a-manifest",
            "test",
        )
        .expect("insert corrupt row");
    let version_id = store.list_for_agent(&id.to_string(), 10).unwrap()[0].id;

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{id}/manifest-history/{version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert_eq!(body["error"]["code"], "version_corrupt", "body: {body}");
    assert_eq!(
        body["error"]["message"], "stored version is corrupt and cannot be restored",
        "the message must stay static rather than echo the parser error: {body}"
    );
}

/// A snapshot is the agent's whole `agent.toml`, so the read and the restore
/// are owner-scoped. 404 rather than 403 stops id enumeration from telling
/// "not yours" apart from "does not exist".
#[tokio::test(flavor = "multi_thread")]
async fn history_and_restore_are_owner_scoped() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "history-owned", "alice");
    let mut v1 = manifest_with_prompt(&h.state, id, "alice prompt");
    v1.description = "alice".to_string();
    h.state
        .kernel
        .update_manifest(id, v1, "model")
        .expect("persist");
    let (_, body) = get_history(&h, &id, None).await;
    let version_id = body["versions"][0]["id"].as_i64().expect("version id");

    let mallory = Some(user("mallory", UserRole::Viewer));
    let (status, body) = get_history(&h, &id, mallory.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(
        body.get("versions").is_none(),
        "no snapshot payload may leak on the denied path: {body}"
    );

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{id}/manifest-history/{version_id}/restore"),
        mallory,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");

    let alice = Some(user("alice", UserRole::Viewer));
    let (status, body) = get_history(&h, &id, alice.clone()).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body["versions"].is_array(), "body: {body}");

    let (status, _) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{id}/manifest-history/{version_id}/restore"),
        alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let admin = Some(user("root", UserRole::Admin));
    let (status, body) = get_history(&h, &id, admin).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}

/// The loopback / no-auth deployment mode carries no `AuthenticatedApiUser`,
/// and the API compatibility contract keeps it allowed.
#[tokio::test(flavor = "multi_thread")]
async fn an_unauthenticated_trusted_request_is_still_served() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "history-no-auth", "alice");

    let (status, body) = get_history(&h, &id, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body["versions"].is_array(), "body: {body}");
}

/// `LIBREFANG_PROVISIONING_PATH` is process-global, so the provisioning case
/// holds this lock while it is set and restores the previous value on drop,
/// panic included.
fn provisioning_env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Sets `LIBREFANG_PROVISIONING_PATH` for its lifetime.
///
/// Other cases in this binary boot their kernels concurrently and may observe
/// the declaration — the reconcile is additive and runs in each kernel's own
/// temp home, so an extra provisioned agent cannot change what they assert.
struct ProvisioningEnv {
    previous: Option<std::ffi::OsString>,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl ProvisioningEnv {
    async fn set(root: &Path) -> Self {
        let lock = provisioning_env_lock().lock().await;
        let previous = std::env::var_os(PROVISIONING_PATH_ENV);
        std::env::set_var(PROVISIONING_PATH_ENV, root.as_os_str());
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for ProvisioningEnv {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(PROVISIONING_PATH_ENV, value),
            None => std::env::remove_var(PROVISIONING_PATH_ENV),
        }
    }
}

/// A deployment-provisioned agent is an input, not a file the API may roll
/// back: the restore refuses through `guard_provisioned_agent` (#6695) before
/// the version store is even consulted, because the next reconcile would
/// overwrite whatever the restore applied.
#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_a_deployment_provisioned_agent() {
    let root = tempfile::tempdir().expect("provisioning root");
    let agents_dir = root.path().join(AGENTS_SUBDIR);
    std::fs::create_dir_all(&agents_dir).expect("mkdir agents");
    std::fs::write(
        agents_dir.join("history-provisioned.toml"),
        "name = \"history-provisioned\"\ndescription = \"deployment-owned\"\nmodule = \"builtin:chat\"\n",
    )
    .expect("write declaration");

    let _env = ProvisioningEnv::set(root.path()).await;
    let h = boot();

    let id = h
        .state
        .kernel
        .agent_registry()
        .find_by_name("history-provisioned")
        .expect("boot reconcile must provision the declared agent")
        .id;

    // Version 1 does not exist: the guard must fire before the store lookup.
    // 423 Locked is the shared provisioned-write refusal; the 409 in the
    // restore handler's OpenAPI annotation is stale.
    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/agents/{id}/manifest-history/1/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::LOCKED, "body: {body}");
    assert_eq!(body["ok"], false, "body: {body}");
    assert_eq!(body["code"], "resource_provisioned", "body: {body}");
    assert_eq!(body["kind"], "agent", "body: {body}");
    assert_eq!(body["name"], "history-provisioned", "body: {body}");
    assert!(
        body["source"]
            .as_str()
            .is_some_and(|source| source.ends_with("history-provisioned.toml")),
        "the refusal must name the declaring file: {body}"
    );
}
