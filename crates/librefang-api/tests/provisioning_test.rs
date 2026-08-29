//! Integration tests for declarative resource provisioning (#6695).
//!
//! In their own test binary for the same reason as `config_managed_mode_test.rs`: `LIBREFANG_PROVISIONING_PATH` and `LIBREFANG_PROVISIONING_PRUNE` are process-global, and a binary is the only isolation boundary Rust's test harness gives us.
//! Within the file, [`ProvisioningEnv`] serializes the cases and restores both variables on drop, panic included.
//!
//! Every case boots a real kernel through the real axum router and asserts over HTTP, because the wiring these tests exist to catch — a reconcile that never runs, a guard that is never reached, a status route that is never registered — is invisible to a unit test of the planner.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

const RESEARCHER: &str = r#"
name = "researcher"
description = "Provisioned by the deployment"
module = "builtin:chat"
"#;

const RESEARCHER_V2: &str = r#"
name = "researcher"
description = "Provisioned by the deployment, revised"
module = "builtin:chat"
"#;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct RouterHarness {
    app: axum::Router,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot a router against an explicit home directory.
///
/// Taking the home rather than making one lets a case boot twice over the same state, which is the only way to test what a *second* reconcile does — the unchanged no-op, the orphan release, the prune.
async fn boot_router(home: &Path) -> RouterHarness {
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);

    let config = KernelConfig {
        home_dir: home.to_path_buf(),
        data_dir: home.join("data"),
        api_key: API_KEY.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    RouterHarness { app, state }
}

/// `LIBREFANG_PROVISIONING_*` are process-global, so every case runs under this lock.
fn provisioning_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Sets the provisioning environment for its lifetime and restores both variables on drop.
struct ProvisioningEnv {
    previous_path: Option<String>,
    previous_prune: Option<String>,
    // A tokio mutex because the guard is held across the harness `.await`s.
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl ProvisioningEnv {
    /// `root = None` means "provisioning off", which is what an existing installation looks like.
    async fn set(root: Option<&Path>, prune: Option<&str>) -> Self {
        let lock = provisioning_lock().lock().await;
        let previous_path =
            std::env::var(librefang_kernel::provisioning::PROVISIONING_PATH_ENV).ok();
        let previous_prune =
            std::env::var(librefang_kernel::provisioning::PROVISIONING_PRUNE_ENV).ok();
        match root {
            Some(root) => std::env::set_var(
                librefang_kernel::provisioning::PROVISIONING_PATH_ENV,
                root.as_os_str(),
            ),
            None => std::env::remove_var(librefang_kernel::provisioning::PROVISIONING_PATH_ENV),
        }
        match prune {
            Some(p) => std::env::set_var(librefang_kernel::provisioning::PROVISIONING_PRUNE_ENV, p),
            None => std::env::remove_var(librefang_kernel::provisioning::PROVISIONING_PRUNE_ENV),
        }
        Self {
            previous_path,
            previous_prune,
            _lock: lock,
        }
    }
}

impl Drop for ProvisioningEnv {
    fn drop(&mut self) {
        restore(
            librefang_kernel::provisioning::PROVISIONING_PATH_ENV,
            self.previous_path.take(),
        );
        restore(
            librefang_kernel::provisioning::PROVISIONING_PRUNE_ENV,
            self.previous_prune.take(),
        );
    }
}

fn restore(key: &str, previous: Option<String>) {
    match previous {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// Create `<root>/agents/<file>` holding `body`, making the directory if needed.
fn declare_agent(root: &Path, file: &str, body: &str) -> PathBuf {
    let dir = root.join(librefang_kernel::provisioning::AGENTS_SUBDIR);
    std::fs::create_dir_all(&dir).expect("mkdir agents");
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write declaration");
    path
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.oneshot(req).await.expect("router response");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    (status, bytes)
}

fn auth(method: Method, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
}

fn auth_get(path: &str) -> Request<Body> {
    auth(Method::GET, path)
        .body(Body::empty())
        .expect("request")
}

fn auth_delete(path: &str) -> Request<Body> {
    auth(Method::DELETE, path)
        .body(Body::empty())
        .expect("request")
}

fn auth_json(method: Method, path: &str, body: serde_json::Value) -> Request<Body> {
    auth(method, path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn status_json(app: &axum::Router) -> serde_json::Value {
    let (status, body) = send(app.clone(), auth_get("/api/provisioning/status")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).expect("status body is JSON")
}

/// Resolve an agent id by name through the public listing, so the tests never reach past the API.
async fn agent_id(app: &axum::Router, name: &str) -> Option<String> {
    let (status, body) = send(app.clone(), auth_get("/api/agents")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v: serde_json::Value = serde_json::from_slice(&body).expect("agents body is JSON");
    let items = v["items"].as_array().cloned().unwrap_or_default();
    items
        .into_iter()
        .find(|a| a["name"] == name)
        .and_then(|a| a["id"].as_str().map(str::to_string))
}

/// Create a runtime-owned agent through `POST /api/agents` and return its id.
async fn spawn_agent(app: &axum::Router, manifest_toml: &str) -> String {
    let (status, body) = send(
        app.clone(),
        auth_json(
            Method::POST,
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "spawn must succeed: {}",
        String::from_utf8_lossy(&body)
    );
    let v: serde_json::Value = serde_json::from_slice(&body).expect("spawn body is JSON");
    v["agent_id"].as_str().expect("agent_id").to_string()
}

/// The one documented resource refusal shape, asserted in one place so a copy cannot drift.
fn assert_provisioned_refusal(status: StatusCode, body: &[u8], name: &str) {
    assert_eq!(
        status,
        StatusCode::LOCKED,
        "a provisioned resource must answer 423, got {status}: {}",
        String::from_utf8_lossy(body)
    );
    let v: serde_json::Value = serde_json::from_slice(body).expect("locked body is JSON");
    assert_eq!(v["ok"], false);
    assert_eq!(v["code"], "resource_provisioned");
    assert_eq!(v["kind"], "agent");
    assert_eq!(v["name"], name);
    assert!(
        v["source"].as_str().is_some_and(|s| s.ends_with(".toml")),
        "the refusal must name the declaring file so the operator knows what to edit; got {v}"
    );
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// Compatibility: an installation that never opts in sees the feature switched off and nothing else changes.
#[tokio::test(flavor = "multi_thread")]
async fn provisioning_is_disabled_and_inert_without_the_environment_variable() {
    let _env = ProvisioningEnv::set(None, None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;

    assert_eq!(v["enabled"], false);
    assert!(v["root"].is_null(), "{v}");
    assert_eq!(v["prune"], "keep");
    assert_eq!(v["resources"].as_array().expect("resources").len(), 0);
    assert!(v["applied_at"].is_null(), "{v}");
    assert!(
        !librefang_kernel::provisioning::state_path(home.path()).exists(),
        "a disabled reconcile must not write a state file"
    );
}

/// The headline case: a file in the tree becomes a live agent, and the status endpoint reports where it came from.
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_agent_is_created_at_boot_and_reported_with_its_provenance() {
    let tree = tempfile::tempdir().expect("tempdir");
    let source = declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;

    assert!(
        agent_id(&h.app, "researcher").await.is_some(),
        "the declared agent must exist after boot"
    );

    let v = status_json(&h.app).await;
    assert_eq!(v["enabled"], true);
    assert_eq!(v["root"], tree.path().display().to_string());
    assert_eq!(v["report"]["created"], 1);
    assert_eq!(v["report"]["failed"], 0);

    let resources = v["resources"].as_array().expect("resources");
    assert_eq!(resources.len(), 1, "{v}");
    let r = &resources[0];
    assert_eq!(r["kind"], "agent");
    assert_eq!(r["name"], "researcher");
    assert_eq!(r["source"], source.display().to_string());
    assert_eq!(r["present"], true);
    assert_eq!(
        r["drifted"], false,
        "a freshly applied declaration cannot have drifted"
    );
    assert_eq!(
        r["checksum"], r["source_checksum"],
        "applied and on-disk checksums must agree right after a reconcile"
    );
}

/// The lock: a provisioned agent's manifest cannot be rewritten through the API.
#[tokio::test(flavor = "multi_thread")]
async fn manifest_writes_to_a_provisioned_agent_are_refused_with_the_documented_shape() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;
    let id = agent_id(&h.app, "researcher")
        .await
        .expect("provisioned agent exists");

    for (method, path, body) in [
        (
            Method::PATCH,
            format!("/api/agents/{id}"),
            serde_json::json!({"description": "hijacked"}),
        ),
        (
            Method::PUT,
            format!("/api/agents/{id}/model"),
            serde_json::json!({"model": "gpt-4o"}),
        ),
        (
            Method::PUT,
            format!("/api/agents/{id}/skills"),
            serde_json::json!({"skills": []}),
        ),
        (
            Method::PUT,
            format!("/api/agents/{id}/mcp_servers"),
            serde_json::json!({"mcp_servers": []}),
        ),
        (
            Method::PUT,
            format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": []}),
        ),
        (
            Method::PUT,
            format!("/api/agents/{id}/tools"),
            serde_json::json!({"capabilities_tools": ["shell_exec"]}),
        ),
        (
            Method::PATCH,
            format!("/api/agents/{id}/config"),
            serde_json::json!({"description": "hijacked"}),
        ),
        (
            Method::PATCH,
            format!("/api/agents/{id}/identity"),
            serde_json::json!({"color": "#ff0000"}),
        ),
    ] {
        let label = format!("{method} {path}");
        let (status, resp) = send(h.app.clone(), auth_json(method, &path, body)).await;
        assert_eq!(
            status,
            StatusCode::LOCKED,
            "{label} must be refused: {}",
            String::from_utf8_lossy(&resp)
        );
        assert_provisioned_refusal(status, &resp, "researcher");
    }

    // And the delete path, which has its own return type.
    let (status, body) = send(
        h.app.clone(),
        auth_delete(&format!("/api/agents/{id}?confirm=true")),
    )
    .await;
    assert_provisioned_refusal(status, &body, "researcher");

    assert!(
        agent_id(&h.app, "researcher").await.is_some(),
        "a refused delete must leave the agent in place"
    );
}

/// The other half of the contract: operating a provisioned agent stays available.
///
/// The RFC's "operational actions and mutable runtime state remain usable" criterion, asserted rather than assumed — a guard placed one level too high would break this and nothing else would notice.
#[tokio::test(flavor = "multi_thread")]
async fn operational_routes_stay_open_on_a_provisioned_agent() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;
    let id = agent_id(&h.app, "researcher")
        .await
        .expect("provisioned agent exists");

    for path in [
        format!("/api/agents/{id}/suspend"),
        format!("/api/agents/{id}/resume"),
    ] {
        let (status, body) = send(
            h.app.clone(),
            auth_json(Method::PUT, &path, serde_json::json!({})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{path} must stay available on a provisioned agent: {}",
            String::from_utf8_lossy(&body)
        );
    }

    let (status, _) = send(h.app.clone(), auth_get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK, "reads are never locked");
}

/// Ownership is per resource, not global: an agent the operator created is still theirs.
#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_created_agent_stays_writable_beside_a_provisioned_one() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;
    let id = spawn_agent(
        &h.app,
        "name = \"handmade\"\ndescription = \"operator's own\"\nmodule = \"builtin:chat\"\n",
    )
    .await;

    let (status, body) = send(
        h.app.clone(),
        auth_json(
            Method::PATCH,
            &format!("/api/agents/{id}"),
            serde_json::json!({"description": "edited by the operator"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a runtime-created agent must remain editable while another agent is provisioned: {}",
        String::from_utf8_lossy(&body)
    );

    let v = status_json(&h.app).await;
    assert_eq!(
        v["resources"].as_array().expect("resources").len(),
        1,
        "only the declared agent is owned; {v}"
    );
}

/// A declaration edited after the reconcile shows as drift, which is how an operator confirms a rollout has not landed yet.
#[tokio::test(flavor = "multi_thread")]
async fn editing_a_declaration_after_boot_reports_drift_without_changing_the_applied_checksum() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;
    let applied = status_json(&h.app).await["resources"][0]["checksum"]
        .as_str()
        .expect("checksum")
        .to_string();

    declare_agent(tree.path(), "researcher.toml", RESEARCHER_V2);

    let r = status_json(&h.app).await["resources"][0].clone();
    assert_eq!(
        r["checksum"], applied,
        "the applied checksum records what is running, and nothing on disk may change it"
    );
    assert_eq!(r["drifted"], true);
    assert_ne!(r["source_checksum"], r["checksum"]);
}

/// A file the reconcile cannot use is reported, and takes nothing else down with it.
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_declaration_is_reported_without_failing_the_boot() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    declare_agent(tree.path(), "broken.toml", "name = \"unterminated\n");
    std::fs::create_dir_all(tree.path().join("channels")).expect("mkdir channels");
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let h = boot_router(home.path()).await;

    assert!(
        agent_id(&h.app, "researcher").await.is_some(),
        "one bad file must not stop the good ones applying"
    );

    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["created"], 1);
    assert_eq!(v["report"]["failed"], 2, "{v}");
    let failures = v["failures"].as_array().expect("failures");
    assert!(
        failures.iter().any(|f| f["source"]
            .as_str()
            .is_some_and(|s| s.ends_with("broken.toml"))),
        "{v}"
    );
    assert!(
        failures.iter().any(|f| f["error"]
            .as_str()
            .is_some_and(|e| e.contains("not a supported provisioning resource kind"))),
        "an unsupported resource kind must be named rather than silently ignored; {v}"
    );
}

/// Idempotence across restarts: an unchanged tree performs no work on the second boot, and the applied timestamp does not move.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_boot_over_an_unchanged_tree_changes_nothing() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    let first_applied = {
        let h = boot_router(home.path()).await;
        let v = status_json(&h.app).await;
        assert_eq!(v["report"]["created"], 1);
        v["resources"][0]["applied_at"]
            .as_str()
            .expect("applied_at")
            .to_string()
    };

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["created"], 0, "{v}");
    assert_eq!(v["report"]["applied"], 0, "{v}");
    assert_eq!(v["report"]["unchanged"], 1, "{v}");
    assert_eq!(
        v["resources"][0]["applied_at"], first_applied,
        "an untouched tree must not look freshly applied"
    );
}

/// A changed declaration is applied to the agent that already exists rather than duplicating it.
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_declaration_is_applied_to_the_existing_agent_on_the_next_boot() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    {
        let h = boot_router(home.path()).await;
        assert!(agent_id(&h.app, "researcher").await.is_some());
    }

    declare_agent(tree.path(), "researcher.toml", RESEARCHER_V2);

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["applied"], 1, "{v}");
    assert_eq!(v["report"]["created"], 0, "{v}");
    assert_eq!(
        v["report"]["adopted"], 0,
        "the agent was already owned; {v}"
    );
    assert_eq!(v["resources"][0]["drifted"], false);

    let id = agent_id(&h.app, "researcher")
        .await
        .expect("still one agent");
    let (status, body) = send(h.app.clone(), auth_get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let agent: serde_json::Value = serde_json::from_slice(&body).expect("agent body is JSON");
    assert_eq!(
        agent["description"], "Provisioned by the deployment, revised",
        "the new declaration must be in effect on the same agent"
    );
}

/// Removing a declaration under the default policy releases the agent instead of deleting it: it keeps running and becomes editable again.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_declaration_releases_the_agent_under_the_default_prune_policy() {
    let tree = tempfile::tempdir().expect("tempdir");
    let source = declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    {
        let h = boot_router(home.path()).await;
        let id = agent_id(&h.app, "researcher").await.expect("created");
        let (status, body) = send(
            h.app.clone(),
            auth_json(
                Method::PATCH,
                &format!("/api/agents/{id}"),
                serde_json::json!({"description": "nope"}),
            ),
        )
        .await;
        assert_provisioned_refusal(status, &body, "researcher");
    }

    std::fs::remove_file(&source).expect("remove declaration");

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["released"], 1, "{v}");
    assert_eq!(v["report"]["pruned"], 0, "{v}");
    assert_eq!(
        v["resources"].as_array().expect("resources").len(),
        0,
        "a released resource is no longer owned; {v}"
    );

    let id = agent_id(&h.app, "researcher")
        .await
        .expect("a released agent keeps running");
    let (status, body) = send(
        h.app.clone(),
        auth_json(
            Method::PATCH,
            &format!("/api/agents/{id}"),
            serde_json::json!({"description": "mine now"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "releasing must hand the agent back to the operator: {}",
        String::from_utf8_lossy(&body)
    );
}

/// The destructive policy is opt-in and does what it says.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_declaration_deletes_the_agent_under_the_delete_prune_policy() {
    let tree = tempfile::tempdir().expect("tempdir");
    let source = declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), Some("delete")).await;
    let home = tempfile::tempdir().expect("tempdir");

    {
        let h = boot_router(home.path()).await;
        assert!(agent_id(&h.app, "researcher").await.is_some());
    }

    std::fs::remove_file(&source).expect("remove declaration");

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["prune"], "delete");
    assert_eq!(v["report"]["pruned"], 1, "{v}");
    assert_eq!(v["report"]["released"], 0, "{v}");
    assert!(
        agent_id(&h.app, "researcher").await.is_none(),
        "the delete policy must actually remove the agent"
    );
}

/// An agent that already exists under a declared name is adopted rather than colliding, and the takeover is reported.
#[tokio::test(flavor = "multi_thread")]
async fn an_existing_agent_is_adopted_when_the_deployment_declares_its_name() {
    let tree = tempfile::tempdir().expect("tempdir");
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    // Boot once with an empty tree and create the agent by hand.
    {
        let h = boot_router(home.path()).await;
        spawn_agent(
            &h.app,
            "name = \"researcher\"\ndescription = \"made by hand\"\nmodule = \"builtin:chat\"\n",
        )
        .await;
    }

    declare_agent(tree.path(), "researcher.toml", RESEARCHER);

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["adopted"], 1, "{v}");
    assert_eq!(v["report"]["applied"], 1, "{v}");
    assert_eq!(v["report"]["created"], 0, "{v}");

    let id = agent_id(&h.app, "researcher")
        .await
        .expect("still exactly one researcher");
    let (status, body) = send(
        h.app.clone(),
        auth_json(
            Method::PATCH,
            &format!("/api/agents/{id}"),
            serde_json::json!({"description": "nope"}),
        ),
    )
    .await;
    assert_provisioned_refusal(status, &body, "researcher");
}

/// A provisioned agent that something removed out of band comes back on the next boot.
#[tokio::test(flavor = "multi_thread")]
async fn a_provisioned_agent_deleted_out_of_band_is_recreated_on_the_next_boot() {
    let tree = tempfile::tempdir().expect("tempdir");
    declare_agent(tree.path(), "researcher.toml", RESEARCHER);
    let _env = ProvisioningEnv::set(Some(tree.path()), None).await;
    let home = tempfile::tempdir().expect("tempdir");

    {
        let h = boot_router(home.path()).await;
        let id = agent_id(&h.app, "researcher").await.expect("created");
        // The API refuses, so this stands in for the out-of-band removal the guard cannot
        // prevent — a direct kernel call, as a CLI or a hand-edited database would do.
        h.state
            .kernel
            .kill_agent_typed(id.parse().expect("agent id"))
            .expect("kill");
        assert!(agent_id(&h.app, "researcher").await.is_none());
    }

    let h = boot_router(home.path()).await;
    let v = status_json(&h.app).await;
    assert_eq!(v["report"]["created"], 1, "{v}");
    assert!(agent_id(&h.app, "researcher").await.is_some());
}
