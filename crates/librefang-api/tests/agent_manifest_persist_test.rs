//! `PATCH /api/agents/{id}` with `manifest_toml` is the only surface that reaches every manifest
//! field, so its durability is what "the API has full parity" actually rests on (refs #7742).
//!
//! These tests go through the production router and then force the manifest back off disk with
//! `reload_agent_from_disk`, which is the in-process stand-in for a daemon restart: boot
//! reconciliation re-reads each agent's `agent.toml` and overwrites the SQLite projection when the
//! two disagree, so an edit that reached only the database is an edit that will be lost.
//!
//! Run: cargo test -p librefang-api --test agent_manifest_persist_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
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

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn patch_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_json(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn manifest_of(state: &Arc<AppState>, id: AgentId) -> AgentManifest {
    state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent still registered")
        .manifest
        .clone()
}

/// A `[[triggers]]` block with no `pattern` used to make the agent's whole manifest unpersistable.
///
/// The struct-level `#[serde(default)]` on `ManifestTrigger` fills the missing key with
/// `Value::Null`, TOML has no null, and `toml::to_string_pretty` therefore failed for the *entire*
/// `AgentManifest`.
/// `persist_full_manifest_at` logged that and returned, so `update_manifest` still answered 200
/// while `agent.toml` froze at its previous contents — and the next restart restored the frozen
/// file over every edit made since.
/// One malformed trigger thus took all 58 fields down with it, on every surface at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_pattern_less_trigger_does_not_freeze_the_rest_of_the_manifest() {
    let h = boot().await;
    let id = spawn_named(&h.state, "trigger-poison");

    // A manifest an operator could plausibly hand-write: the trigger is missing its pattern.
    let manifest_toml = r#"
name = "trigger-poison"
description = "first write"
max_history_messages = 11

[model]
provider = "ollama"
model = "test-model"

[[triggers]]
prompt_template = "wake up: {{event}}"
"#;

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"manifest_toml": manifest_toml}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    // The trigger is kept in its inert state — the kernel skips a null pattern at reconcile with a
    // warning, and dropping the entry silently would be its own surprise.
    let in_memory = manifest_of(&h.state, id);
    assert_eq!(in_memory.triggers.len(), 1);
    assert!(in_memory.triggers[0].pattern.is_null());

    // The write that matters: a second edit, then back off disk.
    let second = manifest_toml.replace("first write", "second write");
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"manifest_toml": second}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    h.state
        .kernel
        .reload_agent_from_disk(id)
        .expect("agent.toml must be readable back");

    let reloaded = manifest_of(&h.state, id);
    assert_eq!(
        reloaded.description, "second write",
        "the edit reached agent.toml rather than only SQLite"
    );
    assert_eq!(
        reloaded.max_history_messages,
        Some(11),
        "a field with no other write path survives the same round trip"
    );
    assert_eq!(
        reloaded.triggers.len(),
        1,
        "the pattern-less trigger round-trips instead of being dropped"
    );
    assert!(
        reloaded.triggers[0].pattern.is_null(),
        "and comes back in the same inert state the kernel already skips"
    );
    assert_eq!(reloaded.triggers[0].prompt_template, "wake up: {{event}}");
}

/// Fields with no dedicated route are exactly the ones this issue inventoried as unreachable, so
/// the whole-manifest path has to carry them all the way to disk.
///
/// One representative from each family the issue listed as having zero graphical surface.
#[tokio::test(flavor = "multi_thread")]
async fn zero_surface_fields_survive_a_manifest_write_and_reload() {
    let h = boot().await;
    let id = spawn_named(&h.state, "zero-surface");

    let manifest_toml = r#"
name = "zero-surface"
max_history_messages = 42
cache_context = true
show_progress = false
reconcile_orphans = "delete"
tool_exec_backend = "docker"
assignee_wake = false

[model]
provider = "ollama"
model = "test-model"

[proactive_memory]
enabled = false

[compaction]
keep_recent = 7

[skill_workshop]
enabled = true

[async_tasks]
default_timeout_secs = 300

[metadata]
owner = "platform"

[workspaces]
library = { path = "shared/library", mode = "r" }
"#;

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"manifest_toml": manifest_toml}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    h.state
        .kernel
        .reload_agent_from_disk(id)
        .expect("agent.toml must be readable back");

    let m = manifest_of(&h.state, id);
    assert_eq!(m.max_history_messages, Some(42));
    assert!(m.cache_context);
    assert!(!m.show_progress);
    assert_eq!(
        m.reconcile_orphans,
        librefang_types::agent::OrphanPolicy::Delete
    );
    assert_eq!(
        m.tool_exec_backend,
        Some(librefang_types::tool_exec::BackendKind::Docker)
    );
    assert_eq!(m.assignee_wake, Some(false));
    assert_eq!(m.proactive_memory.enabled, Some(false));
    assert_eq!(m.compaction.as_ref().and_then(|c| c.keep_recent), Some(7));
    assert!(m.skill_workshop.enabled);
    assert_eq!(m.async_tasks.default_timeout_secs, Some(300));
    assert_eq!(
        m.metadata.get("owner").and_then(|v| v.as_str()),
        Some("platform")
    );
    assert!(
        m.workspaces.contains_key("library"),
        "named workspaces survive the round trip"
    );
}

/// `name` is deliberately not writable through `manifest_toml`, and must stay that way (#7742).
///
/// `update_manifest` pins it because the registry's `name_index` and `AgentEntry::name` are not
/// updated by a manifest swap, so a rename through this path would leave `find_by_name` pointing at
/// the old string. `PATCH {"name": …}` routes through `update_name`, which maintains both.
#[tokio::test(flavor = "multi_thread")]
async fn manifest_toml_cannot_rename_an_agent_out_from_under_the_registry() {
    let h = boot().await;
    let id = spawn_named(&h.state, "keeps-its-name");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({
                "manifest_toml": "name = \"renamed-behind-the-index\"\n\n[model]\nprovider = \"ollama\"\nmodel = \"test-model\"\n"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the write itself is accepted");

    assert_eq!(
        manifest_of(&h.state, id).name,
        "keeps-its-name",
        "the manifest path must not rename; that is what PATCH {{\"name\"}} is for"
    );

    // And the supported route does work, so this is a routing rule rather than a missing feature.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"name": "renamed-properly"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(manifest_of(&h.state, id).name, "renamed-properly");
    assert_eq!(
        h.state
            .kernel
            .agent_registry()
            .get(id)
            .expect("still registered")
            .name,
        "renamed-properly",
        "the registry entry tracks the rename, which is the invariant the pin protects"
    );
}

/// `GET /api/agents/{id}/manifest-history` exposes the snapshots the persist
/// path records, so an operator can see how an agent's config changed over
/// time. Viewing only — there is no restore of a prior version.
///
/// Covers the whole route contract: two PATCHes produce two rows (newest
/// first, with the fields the dashboard's diff view reads), `limit=0` is
/// clamped to at least one row instead of returning an empty array, and the
/// error paths use the standard `ApiErrorResponse` envelope with
/// machine-readable codes.
#[tokio::test(flavor = "multi_thread")]
async fn manifest_history_route_lists_recorded_versions_and_rejects_bad_ids() {
    let h = boot().await;
    let id = spawn_named(&h.state, "history-route");

    let first = r#"
name = "history-route"
description = "first write"

[model]
provider = "ollama"
model = "test-model"
"#;
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"manifest_toml": first}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    let second = first.replace("first write", "second write");
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({"manifest_toml": second}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    // Two writes, two rows, newest first.
    let (status, body) = send(
        h.app.clone(),
        get_json(&format!("/api/agents/{id}/manifest-history")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let versions = body["versions"]
        .as_array()
        .expect("versions array in response body")
        .clone();
    assert_eq!(versions.len(), 2, "body={body:?}");
    for v in &versions {
        for key in [
            "id",
            "agent_id",
            "agent_name",
            "timestamp",
            "manifest_toml",
            "change_source",
        ] {
            assert!(
                v.get(key).is_some(),
                "row is missing the '{key}' the dashboard diff view reads: {v:?}"
            );
        }
    }
    assert!(
        versions[0]["manifest_toml"]
            .as_str()
            .expect("manifest_toml is a string")
            .contains("second write"),
        "rows are newest first: {versions:?}"
    );
    assert_eq!(
        versions[0]["change_source"], "update",
        "a successful persist records the 'update' change_source"
    );

    // `limit=0` is clamped to at least one row rather than honoured literally.
    let (status, body) = send(
        h.app.clone(),
        get_json(&format!("/api/agents/{id}/manifest-history?limit=0")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(
        body["versions"].as_array().expect("versions array").len(),
        1,
        "limit=0 must clamp to 1, not return an empty array"
    );

    // A well-formed but unregistered id is a 404 in the standard envelope.
    let unregistered = "00000000-0000-0000-0000-000000000000";
    let (status, body) = send(
        h.app.clone(),
        get_json(&format!("/api/agents/{unregistered}/manifest-history")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");
    assert_eq!(
        body["error"]["code"], "agent_not_found",
        "the 404 carries the standard envelope code"
    );

    // A malformed id is a 400 in the standard envelope.
    let (status, body) = send(
        h.app.clone(),
        get_json("/api/agents/not-a-uuid/manifest-history"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert_eq!(
        body["error"]["code"], "invalid_agent_id",
        "the 400 carries the standard envelope code"
    );

    // A non-numeric limit is rejected at extraction rather than coerced.
    let (status, _) = send(
        h.app.clone(),
        get_json(&format!("/api/agents/{id}/manifest-history?limit=abc")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "non-numeric limit must 400"
    );
}
