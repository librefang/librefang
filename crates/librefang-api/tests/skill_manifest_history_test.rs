//! End-to-end coverage for the skill version history: `GET /api/skills/{name}/history` and `POST /api/skills/{name}/history/{version_id}/restore` (#8021).
//!
//! The harness boots a real `LibreFangKernel` through `MockKernelBuilder` and mounts the production skills router, so writes go through the same evolve handlers the daemon serves and persistence lands in the same `skill_versions` table.
//!
//! Run: cargo test -p librefang-api --test skill_manifest_history_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    test: TestAppState,
}

impl Harness {
    fn home(&self) -> &Path {
        self.test.tmp_path()
    }

    fn skill_dir(&self, name: &str) -> std::path::PathBuf {
        self.home().join("skills").join(name)
    }

    fn store(&self) -> librefang_memory::SkillVersionStore {
        librefang_memory::SkillVersionStore::new(self.state.kernel.memory_substrate().pool())
    }
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::skills::router())
        .with_state(state.clone());
    Harness { app, state, test }
}

/// Same as [`boot`] but with the kernel booted in Stable mode, which freezes
/// the skill registry at boot (`KernelConfig::mode`) — the restore must refuse
/// through `reject_if_frozen` before touching disk.
fn boot_stable() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.mode = librefang_types::config::KernelMode::Stable;
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::skills::router())
        .with_state(state.clone());
    Harness { app, state, test }
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let request = builder.body(Body::from(body_bytes)).unwrap();
    let response = h.app.clone().oneshot(request).await.expect("oneshot");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read response body");
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn create_skill(h: &Harness, name: &str, prompt: &str) -> (StatusCode, serde_json::Value) {
    send(
        h,
        Method::POST,
        "/api/skills/create",
        Some(serde_json::json!({
            "name": name,
            "description": "history test skill",
            "prompt_context": prompt,
        })),
    )
    .await
}

async fn evolve_skill(h: &Harness, name: &str, prompt: &str) -> (StatusCode, serde_json::Value) {
    send(
        h,
        Method::POST,
        &format!("/api/skills/{name}/evolve/update"),
        Some(serde_json::json!({
            "prompt_context": prompt,
            "changelog": "test edit",
        })),
    )
    .await
}

async fn get_history(h: &Harness, name: &str) -> (StatusCode, serde_json::Value) {
    send(h, Method::GET, &format!("/api/skills/{name}/history"), None).await
}

/// Drop a skill on disk the way a manual install would (no history row), so
/// the frozen and corrupt tests can reach the store without going through a
/// mutation endpoint.
fn install_skill_on_disk(home: &Path, name: &str, prompt: &str) {
    let dir = home.join("skills").join(name);
    std::fs::create_dir_all(&dir).expect("mkdir skill dir");
    std::fs::write(
        dir.join("skill.toml"),
        format!(
            "[skill]\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"test\"\n\
             author = \"tester\"\n\n[runtime]\ntype = \"prompt_only\"\n"
        ),
    )
    .expect("write skill.toml");
    std::fs::write(dir.join("prompt_context.md"), prompt).expect("write prompt_context.md");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_returns_recorded_versions_newest_first() {
    let h = boot();
    let (status, body) = create_skill(&h, "hist_list", "prompt v1").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = evolve_skill(&h, "hist_list", "prompt v2").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = get_history(&h, "hist_list").await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let versions = body["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 2, "body: {body}");
    assert_eq!(versions[0]["change_source"], "evolve", "newest first");
    assert_eq!(versions[1]["change_source"], "create");
    assert_eq!(versions[0]["skill_name"], "hist_list");
    assert_eq!(versions[0]["author"], "dashboard");
    assert_eq!(
        versions[0]["prompt_context"], "prompt v2",
        "the snapshot carries the prompt the last write produced: {body}"
    );
    assert!(
        versions[1]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("hist_list"),
        "the snapshot carries the full skill.toml: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_rejects_invalid_name_and_non_numeric_limit() {
    let h = boot();

    let (status, body) = send(&h, Method::GET, "/api/skills/not%2Fvalid/history", None).await;
    // The route segment cannot carry `/`, so this exercises the validator's
    // rejection of other unsafe names; a valid-but-unknown name 404s below.
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "body: {body}"
    );

    let (status, body) = get_history(&h, "does_not_exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "skill_not_found", "body: {body}");

    let (status, body) = send(
        &h,
        Method::GET,
        "/api/skills/hist_limit/history?limit=abc",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_honours_the_limit_cap() {
    let h = boot();
    create_skill(&h, "hist_limit", "v1").await;
    evolve_skill(&h, "hist_limit", "v2").await;
    evolve_skill(&h, "hist_limit", "v3").await;

    let (status, body) = send(
        &h,
        Method::GET,
        "/api/skills/hist_limit/history?limit=2",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["versions"].as_array().unwrap().len(),
        2,
        "body: {body}"
    );
    assert_eq!(body["versions"][0]["prompt_context"], "v3");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rolls_back_the_skill_and_records_the_restore() {
    let h = boot();
    create_skill(&h, "hist_restore", "prompt v1").await;
    evolve_skill(&h, "hist_restore", "prompt v2").await;

    let (_, body) = get_history(&h, "hist_restore").await;
    let versions = body["versions"].as_array().expect("versions array");
    let v1_id = versions[1]["id"].as_i64().expect("v1 id");
    assert_eq!(versions[1]["prompt_context"], "prompt v1");

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/skills/hist_restore/history/{v1_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["restored_version_id"], v1_id);
    assert_eq!(body["skill_name"], "hist_restore");

    // The file on disk is back to v1.
    let on_disk =
        std::fs::read_to_string(h.skill_dir("hist_restore").join("prompt_context.md")).unwrap();
    assert_eq!(
        on_disk, "prompt v1",
        "restore must apply the stored snapshot"
    );

    // And the restore itself is a recorded version.
    let (_, body) = get_history(&h, "hist_restore").await;
    let versions = body["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 3, "body: {body}");
    assert_eq!(versions[0]["change_source"], "restore");
    assert_eq!(
        versions[0]["prompt_context"], "prompt v1",
        "the newest row holds the restored content: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_maps_missing_and_mismatched_versions() {
    let h = boot();
    create_skill(&h, "hist_alice", "alice v1").await;
    create_skill(&h, "hist_bob", "bob v1").await;

    let (_, body) = get_history(&h, "hist_alice").await;
    let alice_version_id = body["versions"][0]["id"].as_i64().expect("alice id");

    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/hist_alice/history/999999/restore",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "version_not_found");

    // Another skill's version id must be indistinguishable from a missing one,
    // or the status leaks which ids exist.
    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/skills/hist_bob/history/{alice_version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "version_not_found");

    // Restoring a name that is not installed is a 404, not a resurrection.
    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/skills/hist_gone/history/{alice_version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(body["error"]["code"], "skill_not_found");
}

/// A snapshot the server cannot deserialize must fail as its own static 500
/// rather than half-applying or echoing parser internals.
#[tokio::test(flavor = "multi_thread")]
async fn restore_reports_a_corrupt_stored_version_as_a_static_500() {
    let h = boot();
    create_skill(&h, "hist_corrupt", "prompt v1").await;

    // A producer never writes unparseable TOML, but a hand-edited database or
    // a row from an older schema can still hold one. Inserting it through the
    // store is the only way to reach the parse guard.
    h.store()
        .record_version(
            "hist_corrupt",
            "name = [not-a-skill",
            "broken",
            "test",
            "create",
        )
        .expect("insert corrupt row");
    let corrupt_id = h.store().list_for_skill("hist_corrupt", 10).unwrap()[0].id;

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/skills/hist_corrupt/history/{corrupt_id}/restore"),
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

/// Stable mode freezes the registry: the restore must refuse before touching
/// disk, the same way every other evolve endpoint does.
#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_under_stable_mode() {
    let h = boot_stable();
    install_skill_on_disk(h.home(), "hist_frozen", "frozen v1");
    h.store()
        .record_version("hist_frozen", "toml", "prompt", "test", "create")
        .unwrap();
    let version_id = h.store().list_for_skill("hist_frozen", 10).unwrap()[0].id;

    let (status, body) = send(
        &h,
        Method::POST,
        &format!("/api/skills/hist_frozen/history/{version_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Stable")),
        "the refusal must name the frozen registry: {body}"
    );
}

/// The headline acceptance item for the runtime half: a skill an agent authors
/// through `skill_evolve_create` shows up in the history, attributed to the
/// tool call, byte for byte the document the tool wrote.
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_authored_skill_records_agent_tool_in_history() {
    use librefang_kernel_handle::KernelHandle;
    use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};

    let h = boot();
    let skills_dir = h.home().join("skills");
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
    registry.load_all().expect("load registry");

    let kernel: Arc<dyn KernelHandle> = h.state.kernel.clone();
    let ctx = ToolExecContext {
        kernel: Some(&kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"),
        skill_registry: Some(&registry),
        allowed_skills: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        tts_config: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id: None,
        channel: None,
        chat_id: None,
        sender_account_id: None,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
        acting_principal: None,
    };

    let result = execute_tool_raw(
        "t1",
        "skill_evolve_create",
        &serde_json::json!({
            "name": "hist_tool",
            "description": "authored from a conversation",
            "prompt_context": "Do the thing",
        }),
        &ctx,
    )
    .await;
    assert!(
        !result.is_error,
        "skill_evolve_create failed: {}",
        result.content
    );

    let (status, history) = get_history(&h, "hist_tool").await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let versions = history["versions"].as_array().expect("versions array");
    assert_eq!(
        versions.len(),
        1,
        "one tool-authored create must leave exactly one snapshot: {history}"
    );
    assert_eq!(
        versions[0]["change_source"], "create",
        "the tool path must record its own source: {history}"
    );
    assert_eq!(
        versions[0]["author"], "agent:test-agent",
        "the tool path must attribute the change to the calling agent: {history}"
    );
    assert!(
        versions[0]["prompt_context"]
            .as_str()
            .expect("prompt_context")
            .contains("Do the thing"),
        "the snapshot must carry the body the tool actually wrote: {history}"
    );
}

/// A failed mutation must not record: `evolve/update` on a missing skill
/// answers 404 and leaves the history empty.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_mutation_records_nothing() {
    let h = boot();

    let (status, _) = evolve_skill(&h, "never_created", "prompt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = get_history(&h, "never_created").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(
        h.store()
            .list_for_skill("never_created", 10)
            .unwrap()
            .is_empty(),
        "a failed mutation must not leave a history row"
    );
}

/// Drop a complete skill into `<home>/registry/skills/<name>/` — the source
/// `POST /api/skills/install` copies from.
fn install_registry_skill(home: &Path, name: &str, prompt: &str) {
    let dir = home.join("registry").join("skills").join(name);
    std::fs::create_dir_all(&dir).expect("mkdir registry skill dir");
    std::fs::write(
        dir.join("skill.toml"),
        format!(
            "[skill]\nname = \"{name}\"\nversion = \"1.0.0\"\ndescription = \"registry skill\"\n\
             author = \"tester\"\n\n[runtime]\ntype = \"prompt_only\"\n\n\
             [source]\ntype = \"local\"\n"
        ),
    )
    .expect("write registry skill.toml");
    std::fs::write(dir.join("prompt_context.md"), prompt).expect("write registry prompt");
}

/// The rest of the funnel: every mutation surface records under its own
/// `change_source`, so the history can attribute each kind of change. The
/// removal paths are asserted straight off the store — their skill is gone
/// from disk by the time the row exists, so the list endpoint (which 404s for
/// an uninstalled name) cannot see them.
#[tokio::test(flavor = "multi_thread")]
async fn every_mutation_surface_records_its_own_source() {
    let h = boot();

    // update → `evolve`, patch → `patch`
    create_skill(&h, "hist_sources", "prompt v1").await;
    evolve_skill(&h, "hist_sources", "prompt v2").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/hist_sources/evolve/patch",
        Some(serde_json::json!({
            "old_string": "prompt v2",
            "new_string": "prompt v3",
            "changelog": "patch it",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // file write / file remove
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/hist_sources/evolve/file",
        Some(serde_json::json!({"path": "references/x.md", "content": "hi"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let (status, body) = send(
        &h,
        Method::DELETE,
        "/api/skills/hist_sources/evolve/file?path=references/x.md",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // rollback → `rollback` (the update/patch above left a snapshot)
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/hist_sources/evolve/rollback",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let sources: Vec<String> = h
        .store()
        .list_for_skill("hist_sources", 20)
        .unwrap()
        .into_iter()
        .map(|v| v.change_source)
        .collect();
    for expected in [
        "create",
        "evolve",
        "patch",
        "file-write",
        "file-remove",
        "rollback",
    ] {
        assert!(
            sources.iter().any(|s| s == expected),
            "expected a `{expected}` row, got: {sources:?}"
        );
    }

    // install → `install`
    install_registry_skill(h.home(), "hist_installed", "registry prompt");
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "hist_installed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let installed = h.store().list_for_skill("hist_installed", 10).unwrap();
    assert_eq!(installed.len(), 1, "{installed:?}");
    assert_eq!(installed[0].change_source, "install");

    // delete → `delete` (asserted off the store: the skill is gone from disk)
    create_skill(&h, "hist_deleted", "to be deleted").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/hist_deleted/evolve/delete",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let deleted = h.store().list_for_skill("hist_deleted", 10).unwrap();
    assert_eq!(deleted[0].change_source, "delete", "{deleted:?}");
    assert!(
        deleted[0].prompt_context.contains("to be deleted"),
        "the delete row keeps the content it removed: {deleted:?}"
    );

    // uninstall → `uninstall` (same read-from-the-store rule)
    create_skill(&h, "hist_uninstalled", "to be uninstalled").await;
    let (status, body) = send(
        &h,
        Method::POST,
        "/api/skills/uninstall",
        Some(serde_json::json!({"name": "hist_uninstalled"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let uninstalled = h.store().list_for_skill("hist_uninstalled", 10).unwrap();
    assert_eq!(uninstalled[0].change_source, "uninstall", "{uninstalled:?}");
    assert!(
        uninstalled[0].prompt_context.contains("to be uninstalled"),
        "the uninstall row keeps the content it removed: {uninstalled:?}"
    );
}
