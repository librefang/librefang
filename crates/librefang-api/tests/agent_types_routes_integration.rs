//! Integration tests for the agent-type write verbs on `/api/templates` (#7740, #7731).
//!
//! Every request goes through the real production router built by `server::build_router`, so a
//! handler that exists but was never merged into `server.rs` fails here rather than shipping.
//!
//! The test this file exists for is `update_preserves_every_field_the_dashboard_form_never_sends`.
//! The flat editor shape carries seven of `AgentManifest`'s fifty-eight fields; a `PUT` that
//! rebuilds the document from that shape resets the other fifty-one and answers 200. A suite that
//! only ever puts form-shaped manifests on disk before the `PUT` cannot see it — every assertion
//! passes because there was nothing outside the form to lose. So the fixture below deliberately
//! seeds `[[triggers]]`, `tool_allowlist`, `mcp_servers`, `max_history_messages`, `session_mode`
//! and `[compaction]` first, then saves through the exact body the dashboard sends.
//!
//! ### Two home directories, and why both exist
//!
//! The handlers resolve *agent-type* storage under `state.kernel.config_ref().home_dir` (#8112) —
//! NOT the process-wide `LIBREFANG_HOME` env var, which is what an embedder's `KernelConfig` can
//! point somewhere else entirely. Each [`boot`] call gets a fresh `MockKernelBuilder` tempdir as
//! its own `home_dir`, so fixtures are seeded under *that* harness's own directory, after `boot()`
//! returns it, rather than into one directory shared by the whole test binary.
//!
//! The *registry* side is the other spelling: `registry_cache_dir()` — which the registry-diff and
//! restore handlers call — reads the ambient `LIBREFANG_HOME`, not the kernel's `home_dir`. So
//! [`home`] pins that variable to a tempdir once ([`boot`] forces the init) and
//! [`write_registry_agent_type`] seeds there. Those ambient fixtures *are* shared by the whole
//! binary, which is what [`lock`] serialises; the per-harness paths need no lock, because each
//! harness is already isolated.

use axum::http::StatusCode;
use librefang_api::server;
use librefang_testing::{MockKernelBuilder, TestAppState};
use serde_json::{json, Value as Json};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tokio::sync::Mutex;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The process-wide home the *ambient* agent-type APIs resolve through
/// (`librefang_types::agent_type_store::registry_cache_dir`, used by the registry-diff and
/// restore handlers). Set once, to a tempdir, **before** the first kernel boots, so nothing
/// in this binary can reach the developer's real `~/.librefang`.
///
/// Kept alongside the per-harness [`home_dir`] because the two spellings answer different
/// questions: [#8112]'s `_in` functions take the kernel's own `home_dir`, while the ambient
/// helpers that other PRs' fixtures use read `LIBREFANG_HOME`. Both exist in the file because
/// both exist in production.
fn home() -> PathBuf {
    static HOME: OnceLock<TempDir> = OnceLock::new();
    let dir = HOME.get_or_init(|| {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Safety: env mutation. Setting it once, before any concurrent test reads it, is the
        // pattern the sibling template tests already use. The unsafe block is only required on
        // Rust 2024+.
        std::env::set_var("LIBREFANG_HOME", tmp.path());
        tmp
    });
    dir.path().to_path_buf()
}

/// Serialises the tests that write into the ambient [`home`] directory. The per-harness
/// paths need no lock — each `boot()` is its own tempdir — but the ambient registry fixtures
/// are shared, and `write_registry_agent_type` lands in one directory for the whole binary.
fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Remove what a test seeded under the ambient [`home`] directory.
///
/// The path is built here rather than through [`agent_type_file`] because that helper now
/// takes a `&Harness`: its `home_dir` is the kernel's tempdir, which is a *different*
/// directory from this one (see the module note on per-harness homes).
fn cleanup(name: &str) {
    let _ = std::fs::remove_file(home().join("agent-types").join(format!("{name}.toml")));
    let _ = std::fs::remove_dir_all(home().join("workspaces").join("agents").join(name));
    let _ = std::fs::remove_dir_all(home().join("registry").join("agent-types").join(name));
}

struct Harness {
    app: axum::Router,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    // Force the ambient home init before the kernel boots so nothing reaches the developer's
    // `~/.librefang`. The kernel gets its own tempdir from `MockKernelBuilder` regardless; this
    // pins the *ambient* spelling the registry helpers and the handlers' `registry_cache_dir()`
    // resolve through.
    let _ = home();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model.provider = "ollama".to_string();
        cfg.default_model.model = "test-model".to_string();
        cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
    }));
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();
    // Dropping the tempdir would wipe the SQLite file out from under the kernel mid-test.
    Box::leak(Box::new(tmp));
    let (app, _state) =
        server::build_router(state.kernel.clone(), "127.0.0.1:0".parse().unwrap()).await;
    Harness { app, state }
}

/// This harness kernel's own home directory — where [`write_agent_type`] /
/// [`write_workspace_agent`] seed fixtures, matching what the write verbs
/// and the tool actually resolve against (#8112).
fn home_dir(h: &Harness) -> PathBuf {
    h.state.kernel.config_ref().home_dir.clone()
}

fn agent_types_dir(h: &Harness) -> PathBuf {
    librefang_types::agent_type_store::agent_types_dir_in(&home_dir(h))
}

fn agent_type_file(h: &Harness, name: &str) -> PathBuf {
    agent_types_dir(h).join(format!("{name}.toml"))
}

async fn request(h: &Harness, method: &str, path: &str, body: Option<Json>) -> (StatusCode, Json) {
    send(
        h,
        method,
        path,
        body.is_some().then_some("application/json"),
        match &body {
            Some(json) => serde_json::to_vec(&json).unwrap(),
            None => Vec::new(),
        },
    )
    .await
}

/// A raw-body request — the shape `PUT /api/templates/{name}/toml` (`text/plain`) takes.
async fn send(
    h: &Harness,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> (StatusCode, Json) {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }
    let mut req = builder.body(axum::body::Body::from(body)).unwrap();
    // `MockKernelBuilder` leaves `api_key` empty; the auth middleware still requires a loopback
    // origin, and a `oneshot` call attaches no `ConnectInfo` of its own.
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));

    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Json::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Json::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, json)
}

async fn get(h: &Harness, path: &str) -> (StatusCode, Json) {
    request(h, "GET", path, None).await
}

async fn put(h: &Harness, path: &str, body: Json) -> (StatusCode, Json) {
    request(h, "PUT", path, Some(body)).await
}

async fn post(h: &Harness, path: &str, body: Json) -> (StatusCode, Json) {
    request(h, "POST", path, Some(body)).await
}

async fn delete(h: &Harness, path: &str) -> (StatusCode, Json) {
    request(h, "DELETE", path, None).await
}

fn write_agent_type(h: &Harness, name: &str, body: &str) {
    std::fs::create_dir_all(agent_types_dir(h)).expect("create agent-types dir");
    std::fs::write(agent_type_file(h, name), body).expect("write agent type");
}

fn write_workspace_agent(h: &Harness, name: &str, body: &str) {
    let dir = home_dir(h).join("workspaces").join("agents").join(name);
    std::fs::create_dir_all(&dir).expect("create agent workspace");
    std::fs::write(dir.join("agent.toml"), body).expect("write agent.toml");
}

/// The registry checkout root the diff/restore handlers resolve: `$LIBREFANG_HOME/registry`,
/// with each agent type stored directory-per-type as `agent-types/{name}/agent.toml`.
fn registry_agent_type_file(name: &str) -> PathBuf {
    home()
        .join("registry")
        .join("agent-types")
        .join(name)
        .join("agent.toml")
}

fn write_registry_agent_type(name: &str, body: &str) {
    let file = registry_agent_type_file(name);
    std::fs::create_dir_all(file.parent().unwrap()).expect("create registry agent-types dir");
    std::fs::write(file, body).expect("write registry agent type");
}

/// A minimal but valid manifest, parameterised on `max_history_messages` so a test can vary a
/// field `diff_manifests` deliberately does not compare (exercising the `unlisted_diffs` figure).
fn registry_manifest_body(name: &str, description: &str, max_history: usize) -> String {
    format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"
module = "builtin:chat"
max_history_messages = {max_history}

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Seeded."
"#
    )
}

/// The exact body the dashboard's agent-type editor sends on save: seven flat keys, nothing else.
fn dashboard_save_body(description: &str, system_prompt: &str, tools: &[&str]) -> Json {
    json!({
        "name": "ignored-by-the-route",
        "description": description,
        "system_prompt": system_prompt,
        "provider": "anthropic",
        "model": "claude-sonnet-4",
        "tools": tools,
        "skills": ["research"],
    })
}

/// A manifest carrying six things the flat editor shape cannot express.
fn manifest_with_non_form_fields(name: &str) -> String {
    format!(
        r#"name = "{name}"
version = "0.1.0"
description = "seeded"
module = "builtin:chat"
session_mode = "new"
max_history_messages = 42
mcp_servers = ["github"]
tool_allowlist = ["file_read"]
skills = ["research"]

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Seeded prompt."

[capabilities]
tools = ["file_read"]

[compaction]
threshold_messages = 7

[[triggers]]
pattern = "git.push"
prompt_template = "on push"
"#
    )
}

// ---------------------------------------------------------------------------
// The regression this whole file exists for (#7740)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_preserves_every_field_the_dashboard_form_never_sends() {
    let name = "at_preserve";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = put(
        &h,
        &format!("/api/templates/{name}"),
        dashboard_save_body("edited from the dashboard", "New prompt.", &["web_search"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The edit landed.
    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["spec"]["description"], "edited from the dashboard");
    assert_eq!(detail["spec"]["system_prompt"], "New prompt.");
    assert_eq!(detail["spec"]["tools"], json!(["web_search"]));

    // …and nothing the form could not express went with it. Each of these was reset to its
    // default by the rebuild-from-body implementation this endpoint replaces.
    let stored: toml::Value =
        toml::from_str(&std::fs::read_to_string(agent_type_file(&h, name)).unwrap()).unwrap();
    assert_eq!(
        stored["max_history_messages"].as_integer(),
        Some(42),
        "max_history_messages did not survive the save: {stored}"
    );
    assert_eq!(
        stored["tool_allowlist"].as_array().map(Vec::len),
        Some(1),
        "tool_allowlist did not survive the save: {stored}"
    );
    assert_eq!(
        stored["mcp_servers"].as_array().map(Vec::len),
        Some(1),
        "mcp_servers did not survive the save: {stored}"
    );
    assert_eq!(
        stored["session_mode"].as_str(),
        Some("new"),
        "session_mode did not survive the save: {stored}"
    );
    assert_eq!(
        stored["compaction"]["threshold_messages"].as_integer(),
        Some(7),
        "[compaction] did not survive the save: {stored}"
    );
    assert_eq!(
        stored["triggers"].as_array().map(Vec::len),
        Some(1),
        "[[triggers]] did not survive the save: {stored}"
    );
}

// ---------------------------------------------------------------------------
// Canned substitutions (#7740)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_writes_blank_fields_through_instead_of_substituting_canned_text() {
    let name = "at_blank";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({
            "system_prompt": "",
            "provider": "",
            "model": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    let parsed: toml::Value = toml::from_str(&stored).unwrap();
    assert_eq!(parsed["model"]["system_prompt"].as_str(), Some(""));
    assert_eq!(parsed["model"]["provider"].as_str(), Some(""));
    assert_eq!(parsed["model"]["model"].as_str(), Some(""));
    assert!(
        !stored.contains("You are a helpful AI agent."),
        "a deliberately blank system prompt was replaced with canned text: {stored}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_writes_a_blank_system_prompt_through_unchanged() {
    let name = "at_blank_create";
    let h = boot().await;
    let (status, body) = post(
        &h,
        "/api/templates",
        json!({ "name": name, "system_prompt": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(
        !stored.contains("You are a helpful AI agent."),
        "create substituted canned text for an explicitly blank prompt: {stored}"
    );
    // A key the caller omitted entirely still gets the manifest's own documented default, which is
    // the sentinel the kernel resolves against `[default_model]`.
    assert_eq!(body["spec"]["provider"], "default");
}

// ---------------------------------------------------------------------------
// Skills round-trip (#7740)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_round_trip_through_the_editor_shape() {
    let name = "at_skills";
    let h = boot().await;
    let (status, created) = post(
        &h,
        "/api/templates",
        json!({ "name": name, "skills": ["research", "summarize"] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    // Replay exactly what the GET handed the editor, the way a save that changed nothing does.
    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["spec"]["skills"], json!(["research", "summarize"]));
    let (status, saved) = put(
        &h,
        &format!("/api/templates/{name}"),
        detail["spec"].clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["spec"]["skills"], json!(["research", "summarize"]));

    // And a body that omits `skills` entirely — the shape the form used to send — leaves them alone
    // rather than reading as "clear the list".
    let (status, saved) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "description": "no skills key here" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["spec"]["skills"], json!(["research", "summarize"]));

    // An explicit empty list still clears, so "absent" and "empty" stay distinguishable.
    let (status, saved) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "skills": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["spec"]["skills"], json!([]));
}

// ---------------------------------------------------------------------------
// Dual-source guard (#7731)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_agent_row_is_readable_but_refuses_the_write_verbs() {
    let name = "at_liveagent";
    let h = boot().await;
    write_workspace_agent(&h, name, &manifest_with_non_form_fields(name));

    // The catalog still lists it — that is the dual-source behaviour clients depend on — but the
    // row says up front that this API cannot write it, so a client renders "managed elsewhere"
    // instead of an Edit button whose Save cannot succeed.
    let (status, list) = get(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("workspace agent missing from the catalog: {list}"));
    assert_eq!(row["source"], "agent");
    assert_eq!(row["editable"], false);

    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["source"], "agent");
    assert_eq!(detail["editable"], false);

    // A write aimed at it is refused with a reason, not the bare 404 a templates-dir-only lookup
    // would produce — which would tell an operator nothing about why a visible row will not save.
    let (status, body) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "description": "should not land" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");

    let (status, body) = delete(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");

    // The live agent's manifest is untouched.
    let stored = std::fs::read_to_string(
        home_dir(&h)
            .join("workspaces")
            .join("agents")
            .join(name)
            .join("agent.toml"),
    )
    .unwrap();
    assert!(stored.contains("seeded"), "{stored}");
}

/// `registry-diff` must refuse a name whose only local content is a live
/// agent's own workspace manifest, with the same 409 `restore_from_registry`
/// answers for it — otherwise the diff drawer offers a comparison for a
/// restore that can never succeed (#8042).
#[tokio::test(flavor = "multi_thread")]
async fn registry_diff_refuses_a_name_that_only_resolves_through_a_live_agent() {
    let _g = lock().lock().await;
    let name = "at_registry_diff_liveagent";
    cleanup(name);
    write_registry_agent_type(name, &registry_manifest_body(name, "from registry", 42));

    let h = boot().await;
    write_workspace_agent(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = get(&h, &format!("/api/templates/{name}/registry-diff")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");

    cleanup(name);
}

/// The 409 the test above claims `restore` answers, asserted on `restore`
/// itself rather than inferred from the diff route.
///
/// Both routes reach the refusal through different code — the diff route from
/// `read_agent_type` returning a `WorkspaceAgent` source, the restore route
/// from the local read missing and `workspace_agent_manifest_path` existing —
/// so a change to one leaves the other's guard untested.
/// Without this, dropping the restore guard turns a refusal into a write that
/// materialises an agent-type file shadowing a live agent's name, and the
/// suite stays green (#8054).
#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_a_name_that_only_resolves_through_a_live_agent() {
    let _g = lock().lock().await;
    let name = "at_registry_restore_liveagent";
    cleanup(name);
    write_registry_agent_type(name, &registry_manifest_body(name, "from registry", 42));

    let h = boot().await;
    write_workspace_agent(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = post(&h, &format!("/api/templates/{name}/restore"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");
    assert!(
        !agent_type_file(&h, name).exists(),
        "a refused restore must not materialise an agent type shadowing the live agent's name"
    );

    cleanup(name);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_refuses_a_name_that_belongs_to_a_live_agent() {
    let name = "at_shadow";
    let h = boot().await;
    write_workspace_agent(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = post(&h, "/api/templates", json!({ "name": name })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_name_taken", "{body}");
    assert!(
        !agent_type_file(&h, name).exists(),
        "a refused create still wrote a file"
    );
}

/// A name held by both sources lists once, as the writable copy (#8016).
///
/// `create_refuses_a_name_that_belongs_to_a_live_agent` above closes the door going forward, but it cannot close it behind: `read_agent_type` says outright that a collision "can still arise after the fact — an agent spawned under a name an agent type already uses".
/// Nothing in the suite covered that state, so the `dedup_by` in `list_agent_templates` was load-bearing and unguarded, and dropping it would have shipped two rows with the same name to the Agent Types page — which is what #8016 reports.
///
/// The source matters as much as the count: `editable` has to agree with what a `PUT` to that name would actually do, so the surviving row must be the agent-type one.
/// A dedup that kept the workspace row instead would still show one entry and would still be wrong, offering a control that cannot work (#7731).
#[tokio::test(flavor = "multi_thread")]
async fn a_name_held_by_both_sources_lists_once_as_the_writable_copy() {
    let name = "at_collision";
    let h = boot().await;

    // Both sources hold the name.
    // The descriptions differ so the assertion can tell which row survived, rather than only that one did.
    write_agent_type(
        &h,
        name,
        &manifest_with_non_form_fields(name).replace(
            r#"description = "seeded""#,
            r#"description = "the operator-authored agent type""#,
        ),
    );
    write_workspace_agent(
        &h,
        name,
        &manifest_with_non_form_fields(name).replace(
            r#"description = "seeded""#,
            r#"description = "a live agent's own manifest""#,
        ),
    );
    let (status, list) = get(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{list}");

    let rows: Vec<&Json> = list["templates"]
        .as_array()
        .expect("templates array")
        .iter()
        .filter(|r| r["name"] == name)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "a name held by both sources must list once, not once per source: {list}"
    );

    let row = rows[0];
    assert_eq!(
        row["source"], "agent-type",
        "the writable copy must be the one that survives: {row}"
    );
    assert_eq!(row["editable"], true, "{row}");
    assert_eq!(
        row["description"], "the operator-authored agent type",
        "the surviving row must carry the agent type's own content, not the live agent's: {row}"
    );

    // `total` is derived from the same deduplicated vector, so a regression that reintroduced the duplicate would otherwise be visible in the array while the count still looked right.
    assert_eq!(
        list["total"].as_u64().unwrap_or_default() as usize,
        list["templates"].as_array().expect("templates array").len(),
        "total must count the rows actually served: {list}"
    );
}

// ---------------------------------------------------------------------------
// CRUD lifecycle and validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn crud_lifecycle_through_the_production_router() {
    let name = "at_lifecycle";
    let h = boot().await;

    let (status, created) = post(
        &h,
        "/api/templates",
        json!({
            "name": name,
            "description": "a created type",
            "system_prompt": "Be terse.",
            "provider": "anthropic",
            "model": "claude-sonnet-4",
            "tools": ["web_search"],
            "skills": [],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["source"], "agent-type");
    assert_eq!(created["editable"], true);

    let (status, list) = get(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("created type missing from the catalog: {list}"));
    assert_eq!(row["editable"], true);
    assert_eq!(row["provider"], "anthropic");

    // A duplicate create is refused rather than overwriting the existing document. The refusal
    // comes from the `create_new` claim itself, not from a preceding `exists()` check, so the
    // guarantee holds under concurrency as well as here.
    let (status, body) = post(&h, "/api/templates", json!({ "name": name })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_exists", "{body}");
    let after_refusal = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(
        after_refusal.contains("Be terse."),
        "a refused duplicate create overwrote the existing document: {after_refusal}"
    );

    // `/toml` serves the same document the write verbs act on.
    let (status, raw) = get(&h, &format!("/api/templates/{name}/toml")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(raw.as_str().unwrap().contains("Be terse."), "{raw}");

    let (status, body) = delete(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = delete(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn write_verbs_reject_names_that_would_escape_the_agent_types_directory() {
    let h = boot().await;

    let (status, body) = post(&h, "/api/templates", json!({ "name": "../../etc/passwd" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = post(&h, "/api/templates", json!({ "description": "no name" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // The router itself will not match a path segment containing a slash, so the reachable
    // traversal shapes are the ones the name validator has to catch.
    let (status, body) = put(&h, "/api/templates/..", json!({ "description": "x" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = delete(&h, "/api/templates/..").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typo_in_a_save_body_is_rejected_rather_than_silently_ignored() {
    let name = "at_typo";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    // Under patch semantics an unrecognised key would deserialize to "field absent", which reads as
    // "keep the old value" — the edit would be dropped and the response would still be 200.
    let (status, body) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "systemPrompt": "camelCase typo" }),
    )
    .await;
    // axum's `Json` extractor reports a serde *data* error (which is what
    // `deny_unknown_fields` raises) as 422, reserving 400 for malformed JSON.
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(stored.contains("Seeded prompt."), "{stored}");
}

#[tokio::test(flavor = "multi_thread")]
async fn update_pins_identity_to_the_url_rather_than_the_body() {
    let name = "at_identity";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "name": "somewhere-else", "description": "renamed?" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], name);
    assert_eq!(body["spec"]["name"], name);
    assert!(
        !agent_type_file(&h, "somewhere-else").exists(),
        "a body `name` moved the document out from under the URL that addressed it"
    );
}

// ---------------------------------------------------------------------------
// The agent-facing `agent_type_create` tool (#7722)
// ---------------------------------------------------------------------------
//
// These run the real runtime dispatcher against the real kernel this harness booted, then read the
// result back through the production router. That pairing is the point: the tool and `POST
// /api/templates` write the same directory through the same `agent_type_store`, and the only way to
// prove they have not drifted is to have one of them write and the other read.

use librefang_kernel_handle::KernelHandle;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};

/// A tool context with nothing wired but the kernel — `agent_type_create` needs no workspace, no
/// skills and no MCP connections, so anything else here would be noise that hides which dependency
/// the tool actually has.
fn tool_ctx(kernel: &Arc<dyn KernelHandle>) -> ToolExecContext<'_> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"),
        skill_registry: None,
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
    }
}

async fn call_agent_type_create(h: &Harness, payload: Json) -> librefang_types::tool::ToolResult {
    let kernel: Arc<dyn KernelHandle> = h.state.kernel.clone();
    let ctx = tool_ctx(&kernel);
    execute_tool_raw("t1", "agent_type_create", &payload, &ctx).await
}

/// The headline acceptance item: a type an agent authors mid-conversation is a type the HTTP
/// catalog serves, byte for byte the same document.
#[tokio::test(flavor = "multi_thread")]
async fn a_type_the_tool_creates_is_the_type_the_api_serves() {
    let name = "at_tool_created";
    let h = boot().await;

    let result = call_agent_type_create(
        &h,
        json!({
            "name": name,
            "description": "authored from a conversation",
            "system_prompt": "Be terse.",
            "provider": "anthropic",
            "model": "claude-sonnet-4",
            "tools": ["web_search"],
            "skills": ["research"],
        }),
    )
    .await;
    assert!(
        !result.is_error,
        "agent_type_create failed: {}",
        result.content
    );

    let tool_view: Json = serde_json::from_str(&result.content).expect("tool result is JSON");
    assert_eq!(tool_view["name"], name);
    assert_eq!(tool_view["provider"], "anthropic");
    assert_eq!(tool_view["model"], "claude-sonnet-4");

    // The detail route serves the same seven-field projection a dashboard editor would open.
    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["source"], "agent-type");
    assert_eq!(
        detail["editable"], true,
        "a tool-authored type must be editable by an operator afterwards: {detail}"
    );
    assert_eq!(
        detail["spec"]["description"],
        "authored from a conversation"
    );
    assert_eq!(detail["spec"]["system_prompt"], "Be terse.");
    assert_eq!(detail["spec"]["provider"], "anthropic");
    assert_eq!(detail["spec"]["model"], "claude-sonnet-4");
    assert_eq!(detail["spec"]["tools"], json!(["web_search"]));
    assert_eq!(detail["spec"]["skills"], json!(["research"]));

    // And it is spawnable-from: the catalog lists it exactly as it lists an operator-authored one.
    let (status, list) = get(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("tool-created type missing from the catalog: {list}"));
    assert_eq!(row["source"], "agent-type");
    assert_eq!(row["editable"], true);
}

/// An operator's document is not something an agent may overwrite by guessing its name.
/// The refusal comes from the shared `File::create_new` claim, so it holds for the tool for the same
/// reason it holds for `POST` — which is exactly what routing both through one store buys.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_refuses_a_name_already_taken_without_clobbering_it() {
    let name = "at_tool_dupe";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    let result = call_agent_type_create(
        &h,
        json!({ "name": name, "system_prompt": "I am the replacement." }),
    )
    .await;
    assert!(
        result.is_error,
        "a duplicate name must be refused: {}",
        result.content
    );
    assert!(
        result.content.contains("already exists"),
        "the reason must tell the model to pick another name: {}",
        result.content
    );

    // Nothing was written: the operator's manifest still has its prompt and every field the flat
    // shape cannot express.
    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["spec"]["system_prompt"], "Seeded prompt.");
    let raw = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(raw.contains("max_history_messages = 42"), "{raw}");
    assert!(raw.contains("[[triggers]]"), "{raw}");
}

/// A name that would escape the store directory has to be refused before it is ever joined onto a
/// path, and the refusal has to say what a legal name looks like — the model is the one that has to
/// fix it.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_rejects_a_name_that_would_escape_the_store_directory() {
    let h = boot().await;

    for bad in ["../escape", "has space", "", &"a".repeat(65)] {
        let result = call_agent_type_create(&h, json!({ "name": bad })).await;
        assert!(
            result.is_error,
            "name {bad:?} must be refused: {}",
            result.content
        );
        assert!(
            result.content.contains("letters, digits"),
            "the refusal must describe a legal name for {bad:?}: {}",
            result.content
        );
    }

    assert!(
        !home_dir(&h).join("escape.toml").exists(),
        "a traversal attempt wrote a file outside the agent-types directory"
    );
}

/// A key the model invented is refused by name rather than dropped, and nothing reaches disk.
/// `AgentTypeSpec` is `deny_unknown_fields` precisely so a typo cannot be read as "keep the old
/// value" — the tool inherits that rather than re-deriving its own idea of the shape.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_rejects_a_spec_carrying_a_field_that_does_not_exist() {
    let name = "at_tool_typo";
    let h = boot().await;

    let result = call_agent_type_create(
        &h,
        json!({ "name": name, "sytsem_prompt": "note the typo" }),
    )
    .await;
    assert!(
        result.is_error,
        "an unknown key must be refused: {}",
        result.content
    );
    assert!(
        result.content.contains("sytsem_prompt"),
        "the offending key must be named: {}",
        result.content
    );

    let (status, body) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a rejected spec must not have created anything: {body}"
    );
}

/// A name that belongs to a live agent is refused for the tool the same way it is for `POST`:
/// an agent type shadowing it would win every later catalog read and make the agent unreachable.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_refuses_a_name_that_belongs_to_a_live_agent() {
    let name = "at_tool_shadow";
    let h = boot().await;
    write_workspace_agent(&h, name, &manifest_with_non_form_fields(name));

    let result = call_agent_type_create(&h, json!({ "name": name })).await;
    assert!(
        result.is_error,
        "a shadowing name must be refused: {}",
        result.content
    );
    assert!(
        result.content.contains("live agent"),
        "the reason must say why the name is unavailable: {}",
        result.content
    );
    assert!(
        !agent_type_file(&h, name).exists(),
        "a refused create left a file behind"
    );
}

/// Omitting provider and model is legal and resolves to the `"default"` sentinel the kernel later
/// maps onto `[default_model]`. The tool reports what was stored rather than echoing what was sent,
/// so a model that omitted them can see what it actually got.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_reports_the_defaults_it_resolved_rather_than_the_fields_it_was_given() {
    let name = "at_tool_defaults";
    let h = boot().await;

    let result = call_agent_type_create(&h, json!({ "name": name })).await;
    assert!(!result.is_error, "{}", result.content);

    let tool_view: Json = serde_json::from_str(&result.content).expect("tool result is JSON");
    assert_eq!(tool_view["provider"], "default");
    assert_eq!(tool_view["model"], "default");

    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["spec"]["provider"], tool_view["provider"],
        "the tool must report the provider the catalog will serve: {detail}"
    );
    assert_eq!(detail["spec"]["model"], tool_view["model"], "{detail}");
}

// ---------------------------------------------------------------------------
// Registry diff + restore (#8042)
// ---------------------------------------------------------------------------

/// A registry-diff request for a type that has a local copy but no registry
/// copy answers 404 with the stable `registry_type_not_found` code, so a
/// client can tell "not synced" apart from "unknown template".
#[tokio::test(flavor = "multi_thread")]
async fn registry_diff_reports_registry_type_not_found_when_the_registry_copy_is_absent() {
    let _g = lock().lock().await;
    let name = "at_registry_missing";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, &registry_manifest_body(name, "local only", 42));

    let (status, body) = get(&h, &format!("/api/templates/{name}/registry-diff")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "registry_type_not_found", "{body}");

    cleanup(name);
}

/// Byte-identical local and registry manifests report `identical: true` with
/// no diffs — the response must not invent a difference the projection missed.
#[tokio::test(flavor = "multi_thread")]
async fn registry_diff_reports_identical_when_local_and_registry_match_exactly() {
    let _g = lock().lock().await;
    let name = "at_registry_identical";
    cleanup(name);
    let body = registry_manifest_body(name, "same everywhere", 42);
    write_registry_agent_type(name, &body);

    let h = boot().await;
    write_agent_type(&h, name, &body);

    let (status, diff) = get(&h, &format!("/api/templates/{name}/registry-diff")).await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert_eq!(diff["identical"], true, "{diff}");
    assert_eq!(diff["unlisted_diffs"], 0, "{diff}");
    assert_eq!(diff["diffs"].as_array().map(Vec::len), Some(0), "{diff}");

    cleanup(name);
}

/// A field the operator-facing projection does not compare (here
/// `max_history_messages`) still drives `identical: false`, and the count of
/// differences outside the projection is reported rather than hidden.
#[tokio::test(flavor = "multi_thread")]
async fn registry_diff_marks_a_field_outside_the_projection_as_non_identical() {
    let _g = lock().lock().await;
    let name = "at_registry_hidden_diff";
    cleanup(name);
    write_registry_agent_type(name, &registry_manifest_body(name, "local", 99));

    let h = boot().await;
    write_agent_type(&h, name, &registry_manifest_body(name, "local", 42));

    let (status, diff) = get(&h, &format!("/api/templates/{name}/registry-diff")).await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert_eq!(diff["identical"], false, "{diff}");
    assert_eq!(
        diff["diffs"].as_array().map(Vec::len),
        Some(0),
        "the projection does not compare max_history_messages: {diff}"
    );
    let unlisted = diff["unlisted_diffs"].as_u64().expect("unlisted_diffs");
    assert!(
        unlisted > 0,
        "the out-of-projection difference must be counted: {diff}"
    );

    cleanup(name);
}

/// `unlisted_diffs` must not double-count a listed field that happens to be a
/// list. `tags` is one of the twelve fields the itemised `diffs` table
/// already covers; when it is the *only* difference, `unlisted_diffs` must
/// be zero rather than counting each differing array element as an
/// out-of-projection difference on top of the itemised row that already
/// shows it.
#[tokio::test(flavor = "multi_thread")]
async fn unlisted_diffs_does_not_double_count_a_differing_listed_list_field() {
    let _g = lock().lock().await;
    let name = "at_registry_list_field_diff";
    cleanup(name);
    let manifest_with_tags = |tags: &str| {
        format!(
            r#"name = "{name}"
description = "same everywhere"
module = "builtin:chat"
tags = {tags}

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Seeded."
"#
        )
    };
    write_registry_agent_type(name, &manifest_with_tags(r#"["c", "d"]"#));

    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_tags(r#"["a", "b"]"#));

    let (status, diff) = get(&h, &format!("/api/templates/{name}/registry-diff")).await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert_eq!(diff["identical"], false, "{diff}");
    assert_eq!(
        diff["diffs"].as_array().map(Vec::len),
        Some(1),
        "tags is the only itemised difference: {diff}"
    );
    assert_eq!(
        diff["unlisted_diffs"], 0,
        "tags is fully itemised already — its two differing elements must not \
         also be counted as unlisted: {diff}"
    );

    cleanup(name);
}

/// Restore overwrites the local copy with the registry version, and a follow-up
/// GET returns the registry content rather than the pre-restore local content.
#[tokio::test(flavor = "multi_thread")]
async fn restore_overwrites_the_local_copy_and_reads_back_the_registry_version() {
    let _g = lock().lock().await;
    let name = "at_registry_restore";
    cleanup(name);
    write_registry_agent_type(name, &registry_manifest_body(name, "from registry", 99));

    let h = boot().await;
    write_agent_type(&h, name, &registry_manifest_body(name, "local", 42));

    let (status, restored) = post(&h, &format!("/api/templates/{name}/restore"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["manifest"]["description"], "from registry",
        "{restored}"
    );

    let (status, detail) = get(&h, &format!("/api/templates/{name}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(
        detail["manifest"]["description"], "from registry",
        "{detail}"
    );
    assert_eq!(
        detail["manifest_toml"], restored["manifest_toml"],
        "{detail}"
    );

    cleanup(name);
}

/// The registry restore is the one write path that destroys the local copy by design, so it has to leave a snapshot behind like every other write path does.
/// A manifest whose current content came from a hand-edit of the file has no history row of its own, so recording only the post-restore content is not enough: the pre-restore content — the thing an operator actually wants back — must itself be recoverable from history, not merely implied by a row existing.
#[tokio::test(flavor = "multi_thread")]
async fn restore_from_registry_records_a_recoverable_pre_restore_snapshot() {
    let _g = lock().lock().await;
    let name = "at_registry_restore_history";
    cleanup(name);
    write_registry_agent_type(name, &registry_manifest_body(name, "from registry", 99));

    let h = boot().await;
    write_agent_type(
        &h,
        name,
        &registry_manifest_body(name, "hand edited on disk", 42),
    );

    let (status, restored) = post(&h, &format!("/api/templates/{name}/restore"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{restored}");

    let (status, history) = get(&h, &format!("/api/templates/{name}/history")).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let versions = history["versions"].as_array().expect("versions array");
    assert_eq!(
        versions.len(),
        2,
        "restore must record both the pre-restore and the post-restore content: {history}"
    );

    // Newest first (`ORDER BY timestamp DESC, id DESC`): the post-restore snapshot lands
    // after the pre-restore one within the same request, so it sorts first.
    assert_eq!(versions[0]["template_name"], name, "{history}");
    assert_eq!(
        versions[0]["change_source"], "registry-restore",
        "{history}"
    );
    assert!(
        versions[0]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("from registry"),
        "the post-restore snapshot must carry the content the restore wrote: {history}"
    );

    assert_eq!(
        versions[1]["change_source"], "pre-registry-restore",
        "{history}"
    );
    assert!(
        versions[1]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("hand edited on disk"),
        "the pre-restore content — what a restore actually needs to make recoverable — \
         must be readable back out of history, not just gone from disk: {history}"
    );

    cleanup(name);
}

/// A pre-restore snapshot that cannot be recorded has to abort the restore, not proceed without it.
///
/// The snapshot is best-effort at every other call site in the handler, and correctly so: those run *after* the write, so losing one costs a history row while the content is still on disk.
/// Here the ordering inverts it. The content on disk may have come from a hand-edit and exist nowhere else, and the very next statement overwrites it, so a swallowed snapshot failure destroys the operator's configuration and still answers 200.
///
/// Dropping `template_versions` is the deterministic stand-in for the failure that actually happens — a `SQLITE_BUSY` from a concurrent writer — and reaches `record_version` as the same `LibreFangError::Memory`.
#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_to_overwrite_when_the_pre_restore_snapshot_fails() {
    let _g = lock().lock().await;
    let name = "at_registry_restore_snapshot_failure";
    cleanup(name);
    let hand_edited = registry_manifest_body(name, "hand edited on disk", 42);
    write_registry_agent_type(name, &registry_manifest_body(name, "from registry", 99));

    let h = boot().await;
    write_agent_type(&h, name, &hand_edited);

    // Break the snapshot store through the pool the substrate already exposes — the same
    // route `goals_routes_integration.rs` uses to make a substrate read fail from out here.
    h.state
        .kernel
        .memory_substrate()
        .pool()
        .get()
        .expect("pool connection")
        .execute("DROP TABLE template_versions", [])
        .expect("drop template_versions");

    let (status, body) = post(&h, &format!("/api/templates/{name}/restore"), json!({})).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a restore that could not snapshot the current content must fail, not answer 200: {body}"
    );

    // The assertion the finding is about: the operator's content survived.
    let on_disk =
        std::fs::read_to_string(agent_type_file(&h, name)).expect("agent type still on disk");
    assert_eq!(
        on_disk, hand_edited,
        "the hand-edited manifest must still be on disk — it was the only copy, and the \
         snapshot meant to preserve it never landed"
    );

    cleanup(name);
}

/// Restoring must pin the manifest's own `name` field to the URL path
/// segment, the same way `update_agent_type` already does — otherwise a
/// registry document whose declared `name` disagrees with its directory
/// (the registry stores each type at `agent-types/<dir>/agent.toml`, and
/// `AgentManifest::name` is documented as the human-readable display name,
/// so the two are free to differ) persists that mismatch into the local
/// catalog.
#[tokio::test(flavor = "multi_thread")]
async fn restore_from_registry_pins_the_manifest_name_to_the_url_segment() {
    let _g = lock().lock().await;
    let name = "at_registry_restore_name_mismatch";
    cleanup(name);
    write_registry_agent_type(
        name,
        r#"name = "Some Other Display Name"
description = "from registry"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Seeded."
"#,
    );

    let h = boot().await;
    write_agent_type(&h, name, &registry_manifest_body(name, "local", 42));

    let (status, restored) = post(&h, &format!("/api/templates/{name}/restore"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["manifest"]["name"], name,
        "the persisted manifest's own `name` must match the URL segment it was \
         restored through, not the registry document's declared name: {restored}"
    );

    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(
        stored.contains(&format!("name = \"{name}\"")),
        "the file on disk must carry the pinned name too: {stored}"
    );

    cleanup(name);
}

// POST /api/templates/{name}/promote (#8043)
// ---------------------------------------------------------------------------

/// Promoting a template that does not exist resolves the manifest before any
/// token / network concern, so it must 404 regardless of GitHub credentials.
/// This is the network-free contract we can assert deterministically in CI.
#[tokio::test(flavor = "multi_thread")]
async fn promote_unknown_template_returns_404() {
    let h = boot().await;

    let (status, body) = post(&h, "/api/templates/at_promote_ghost/promote", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "error must mention 'not found': {body:?}"
    );
}

/// When the template exists but no GitHub token is configured (env or
/// vault), promoting returns 401. Guarded so it only runs when the test
/// process genuinely has no `GITHUB_TOKEN`, since env state is shared
/// across parallel test binaries and mutating it would be racy.
#[tokio::test(flavor = "multi_thread")]
async fn promote_without_token_returns_401() {
    if std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        return;
    }
    let name = "at_promote_no_token";
    let h = boot().await;
    write_agent_type(&h, name, &manifest_with_non_form_fields(name));

    let (status, body) = post(&h, &format!("/api/templates/{name}/promote"), json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("github"),
        "error must mention GitHub: {body:?}"
    );
}

// Template version history + restore (#8047)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn history_and_restore_round_trip() {
    let name = "at_history";
    let h = boot().await;
    let (status, created) = post(
        &h,
        "/api/templates",
        json!({ "name": name, "description": "first version" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, edited) = put(
        &h,
        &format!("/api/templates/{name}"),
        json!({ "description": "second version" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");

    // Two snapshots, newest first, each carrying the documented JSON keys.
    let (status, history) = get(&h, &format!("/api/templates/{name}/history")).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let versions = history["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 2, "{history}");
    assert_eq!(versions[0]["template_name"], name);
    assert_eq!(versions[0]["change_source"], "dashboard");
    assert_eq!(versions[1]["change_source"], "create");
    for v in versions {
        assert!(v.get("id").is_some(), "missing id: {v}");
        assert!(v.get("timestamp").is_some(), "missing timestamp: {v}");
        assert!(
            v.get("manifest_toml").is_some(),
            "missing manifest_toml: {v}"
        );
    }

    // Restore to the older snapshot and observe the description roll back.
    let first_id = versions[1]["id"].as_i64().unwrap();
    let (status, restored) = request(
        &h,
        "POST",
        &format!("/api/templates/{name}/history/{first_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["spec"]["description"], "first version",
        "{restored}"
    );
}

/// The server-side privacy gate: a manifest whose system prompt still
/// carries a personal-data literal (a field the sanitizer keeps) must be
/// refused with 409 and the findings returned, not published.
#[tokio::test(flavor = "multi_thread")]
async fn promote_refuses_a_manifest_with_retained_private_details() {
    let name = "at_promote_pii";
    let h = boot().await;
    write_agent_type(
        &h,
        name,
        &format!(
            r#"name = "{name}"
description = "seeded"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Escalate to priya.rao@acme.example when unsure."
"#
        ),
    );

    let (status, body) = post(&h, &format!("/api/templates/{name}/promote"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body:?}");
    assert_eq!(body["code"], "review_required", "{body:?}");
    let findings = body["details"]["findings"]
        .as_array()
        .expect("findings must be an array");
    assert!(
        findings.iter().any(|f| f["removed_by_sanitizer"] == false),
        "a retained finding must be present: {findings:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_rejects_foreign_versions_and_live_agents() {
    let name = "at_history_neg";
    let other = "at_history_other";
    let live = "at_history_live";
    let h = boot().await;

    post(
        &h,
        "/api/templates",
        json!({ "name": name, "description": "a" }),
    )
    .await;
    post(
        &h,
        "/api/templates",
        json!({ "name": other, "description": "b" }),
    )
    .await;

    // A version that belongs to a different template is refused, not silently applied.
    let (_, history) = get(&h, &format!("/api/templates/{other}/history")).await;
    let other_id = history["versions"][0]["id"].as_i64().unwrap();
    let (status, body) = request(
        &h,
        "POST",
        &format!("/api/templates/{name}/history/{other_id}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "version_mismatch", "{body}");

    // A version id that does not exist is a 404, not a crash.
    let (status, body) = request(
        &h,
        "POST",
        &format!("/api/templates/{name}/history/999999/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "version_not_found", "{body}");

    // A live agent's name is not editable through this route.
    write_workspace_agent(&h, live, &manifest_with_non_form_fields(live));
    let (status, body) = request(
        &h,
        "POST",
        &format!("/api/templates/{live}/history/1/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");
}
// ---------------------------------------------------------------------------
// The raw-TOML write path (#8028)
// ---------------------------------------------------------------------------

/// A manifest in the shape the TOML tab saves: sections the flat editor cannot
/// express, a known-table section, and keys no current schema knows about.
fn toml_tab_document(name: &str) -> String {
    format!(
        r#"name = "{name}"
description = "raw toml save"
session_mode = "new"
mcp_servers = ["github"]
tool_allowlist = ["file_read"]
future_field = "unknown to this daemon"

[model]
provider = "ollama"
model = "test-model"

[workspaces]
notes = {{ path = "notes", mode = "rw" }}

[compaction]
threshold_messages = 7

[[triggers]]
pattern = "git.push"
prompt_template = "on push"
"#
    )
}

/// PUT with a text/plain body through the production router.
async fn put_toml(h: &Harness, path: &str, body: &str) -> (StatusCode, Json) {
    send(h, "PUT", path, Some("text/plain"), body.as_bytes().to_vec()).await
}

/// The headline claim of #8028: the whole-document write is not lossy. Review asked
/// for exactly this round trip — triggers and compaction read back intact.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_round_trips_every_section_the_flat_editor_cannot_express() {
    let _g = lock().lock().await;
    let name = "at_toml_roundtrip";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    let doc = toml_tab_document(name);
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, raw) = get(&h, &format!("/api/templates/{name}/toml")).await;
    assert_eq!(status, StatusCode::OK);
    let stored: toml::Value = toml::from_str(raw.as_str().unwrap()).unwrap();
    assert_eq!(stored["session_mode"].as_str(), Some("new"), "{stored}");
    assert_eq!(
        stored["mcp_servers"][0].as_str(),
        Some("github"),
        "{stored}"
    );
    assert_eq!(
        stored["tool_allowlist"][0].as_str(),
        Some("file_read"),
        "{stored}"
    );
    assert_eq!(
        stored["workspaces"]["notes"]["path"].as_str(),
        Some("notes"),
        "{stored}"
    );
    assert_eq!(
        stored["compaction"]["threshold_messages"].as_integer(),
        Some(7),
        "[compaction] did not survive the save: {stored}"
    );
    assert_eq!(
        stored["triggers"][0]["pattern"].as_str(),
        Some("git.push"),
        "{stored}"
    );
    assert_eq!(
        stored["triggers"][0]["prompt_template"].as_str(),
        Some("on push"),
        "[[triggers]] did not survive the save: {stored}"
    );

    cleanup(name);
}

/// The raw-TOML tab is a write path like create and the flat `PUT`, so it snapshots like one.
/// Without the record, `GET /api/templates/{name}/history` reports the previous save as current while the file on disk is what this handler just wrote — a history that is silently incomplete rather than absent.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_records_a_version_snapshot() {
    let _g = lock().lock().await;
    let name = "at_toml_history";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    let doc = toml_tab_document(name);
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, history) = get(&h, &format!("/api/templates/{name}/history")).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let versions = history["versions"].as_array().expect("versions array");
    assert_eq!(
        versions.len(),
        1,
        "the raw-TOML save must record exactly one snapshot: {history}"
    );
    assert_eq!(versions[0]["template_name"], name, "{history}");
    assert_eq!(versions[0]["change_source"], "toml", "{history}");
    assert!(
        versions[0]["manifest_toml"]
            .as_str()
            .expect("manifest_toml")
            .contains("raw toml save"),
        "the snapshot must carry the content the save wrote: {history}"
    );

    cleanup(name);
}

/// A key the manifest does not recognize is reported in the response and dropped from
/// the file — the report is what keeps that drop from happening in silence.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_reports_keys_the_manifest_does_not_recognise() {
    let _g = lock().lock().await;
    let name = "at_toml_typo";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    let doc = format!("name = \"{name}\"\nsesion_mode = \"new\"\n");
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let unknown = body["unknown_keys"].as_array().expect("unknown_keys array");
    assert!(
        unknown.contains(&json!("sesion_mode")),
        "the typo key was not reported: {body}"
    );

    // And the stored document confirms what the report says: the unrecognised key is gone.
    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(
        !stored.contains("sesion_mode"),
        "the typo key survived the save despite the report: {stored}"
    );

    cleanup(name);
}

/// `triggers = []` — the explicit way to clear the list — must not be reported as a
/// key `AgentManifest` doesn't recognise. `Vec<Trigger>` carries `skip_serializing_if
/// = "Vec::is_empty"`, so the round-tripped comparison document drops the key exactly
/// the way it would for a genuinely unknown one; without excluding empty
/// arrays/tables the report (and its accompanying WARN) told the operator the schema
/// didn't know a key it understands perfectly well.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_does_not_misreport_an_explicitly_cleared_list_as_unrecognised() {
    let _g = lock().lock().await;
    let name = "at_toml_triggers_cleared";
    cleanup(name);

    let h = boot().await;
    write_agent_type(
        &h,
        name,
        &format!("name = \"{name}\"\n\n[[triggers]]\npattern = \"git.push\"\n"),
    );

    let doc = format!("name = \"{name}\"\ntriggers = []\n");
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let flagged_triggers = body
        .get("unknown_keys")
        .and_then(|k| k.as_array())
        .is_some_and(|arr| arr.contains(&json!("triggers")));
    assert!(
        !flagged_triggers,
        "triggers = [] is an explicit clear, not an unrecognised key: {body}"
    );

    cleanup(name);
}

/// A raw-body create — the shape `POST /api/templates/{name}/toml` (`text/plain`) takes.
async fn post_toml(h: &Harness, path: &str, body: &str) -> (StatusCode, Json) {
    send(
        h,
        "POST",
        path,
        Some("text/plain"),
        body.as_bytes().to_vec(),
    )
    .await
}

/// The dashboard's original create flow was two requests — `POST /api/templates` (a
/// name+description stub) then `PUT .../toml` (the manifest the operator actually
/// authored) — which left the stub on disk if the second call failed, with no way to
/// retry short of reopening the dialog, and recorded two version snapshots for one
/// user action even when both calls succeeded (#8028). This endpoint takes the full
/// manifest up front: one write, one snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn toml_post_creates_a_type_in_one_write_with_one_snapshot() {
    let _g = lock().lock().await;
    let name = "at_toml_post_create";
    cleanup(name);

    let h = boot().await;
    let doc = toml_tab_document(name);
    let (status, body) = post_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, raw) = get(&h, &format!("/api/templates/{name}/toml")).await;
    assert_eq!(status, StatusCode::OK);
    let stored: toml::Value = toml::from_str(raw.as_str().unwrap()).unwrap();
    assert_eq!(
        stored["triggers"][0]["pattern"].as_str(),
        Some("git.push"),
        "the manifest sent to create must land in full, not a name+description stub: {stored}"
    );

    let (status, history) = get(&h, &format!("/api/templates/{name}/history")).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let versions = history["versions"].as_array().expect("versions array");
    assert_eq!(
        versions.len(),
        1,
        "one user action must record one snapshot, not a phantom stub followed by the real save: {history}"
    );
    assert_eq!(versions[0]["change_source"], "create", "{history}");

    cleanup(name);
}

/// A second create for a name that already has an agent type must refuse, exactly
/// like the flat-shape create, and must not touch the file that is already there.
#[tokio::test(flavor = "multi_thread")]
async fn toml_post_refuses_a_name_that_already_exists() {
    let _g = lock().lock().await;
    let name = "at_toml_post_exists";
    cleanup(name);

    let h = boot().await;
    write_agent_type(
        &h,
        name,
        &format!("name = \"{name}\"\ndescription = \"already here\"\n"),
    );

    let doc = format!("name = \"{name}\"\n");
    let (status, body) = post_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_exists", "{body}");

    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(
        stored.contains("already here"),
        "a refused create must not touch the existing file: {stored}"
    );

    cleanup(name);
}

/// A name already claimed by a live agent is refused, exactly like the flat-shape
/// create, rather than shadowed by a type this catalog would list ahead of the
/// agent that actually answers to the name.
#[tokio::test(flavor = "multi_thread")]
async fn toml_post_refuses_a_name_that_belongs_to_a_live_agent() {
    let _g = lock().lock().await;
    let name = "at_toml_post_liveagent";
    cleanup(name);

    let h = boot().await;
    write_workspace_agent(&h, name, "name = \"seed\"\n");

    let doc = format!("name = \"{name}\"\n");
    let (status, body) = post_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_name_taken", "{body}");

    cleanup(name);
}

/// Malformed syntax and a type-violation both refuse with 400 and carry the parser
/// detail in `details.toml_error` rather than mixing it into the translated message.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_rejects_malformed_toml_with_a_400() {
    let _g = lock().lock().await;
    let name = "at_toml_bad";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    for doc in [
        "name = \nbroken[".to_string(),
        format!("name = \"{name}\"\nmax_history_messages = \"not-a-number\"\n"),
    ] {
        let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "template_invalid_toml", "{body}");
        assert!(
            body["details"]["toml_error"].is_string(),
            "parser detail missing: {body}"
        );
    }

    // The seeded document is untouched by the refusals.
    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(stored.contains("seed"), "{stored}");

    cleanup(name);
}

/// Unknown template name and live-agent name keep their distinct refusals on this verb too.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_refuses_unknown_and_live_agent_names() {
    let _g = lock().lock().await;
    let name = "at_toml_liveagent";
    cleanup(name);

    let h = boot().await;
    write_workspace_agent(&h, name, "name = \"seed\"\n");

    let (status, body) =
        put_toml(&h, "/api/templates/at_toml_missing/toml", "name = \"x\"\n").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "template_not_found", "{body}");

    let (status, body) = put_toml(
        &h,
        &format!("/api/templates/{name}/toml"),
        "name = \"at_toml_liveagent\"\n",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_not_editable", "{body}");

    cleanup(name);
}

/// The same 1MB manifest cap the agent spawn path enforces, checked before parsing.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_refuses_an_oversize_body_before_parsing() {
    let _g = lock().lock().await;
    let name = "at_toml_oversize";
    cleanup(name);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    let padding = "x".repeat(1024 * 1024);
    let doc = format!("name = \"{name}\"\ndescription = \"{padding}\"\n");
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "template_manifest_too_large", "{body}");

    // The seeded document is untouched.
    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(stored.contains("seed"), "{stored}");

    cleanup(name);
}

/// The name pin is deliberate (the UI copy says the name cannot change); it just has
/// to be visible. Same shape as `manifest_toml_cannot_rename_an_agent_out_from_under_
/// the_registry` on the agent side.
#[tokio::test(flavor = "multi_thread")]
async fn toml_put_pins_the_name_to_the_url_rather_than_the_body() {
    let _g = lock().lock().await;
    let name = "at_toml_pin";
    let renamed = "at_toml_pin_moved";
    cleanup(name);
    cleanup(renamed);

    let h = boot().await;
    write_agent_type(&h, name, "name = \"seed\"\n");

    let doc = format!("name = \"{renamed}\"\ndescription = \"kept\"\n");
    let (status, body) = put_toml(&h, &format!("/api/templates/{name}/toml"), &doc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], name, "{body}");
    assert!(
        !agent_type_file(&h, renamed).exists(),
        "a body name moved the document out from under the URL that addressed it"
    );
    let stored = std::fs::read_to_string(agent_type_file(&h, name)).unwrap();
    assert!(stored.contains("kept"), "{stored}");

    cleanup(name);
    cleanup(renamed);
}

/// The templates list must tell an editable row with a registry original apart
/// from one that has none, so the dashboard's restore control can be disabled
/// with an explanation instead of opening a drawer that can only ever answer
/// "this agent type does not exist in the registry" (#8042 review).
#[tokio::test(flavor = "multi_thread")]
async fn templates_list_flags_from_registry_per_row() {
    let _g = lock().lock().await;
    let synced = "at_list_from_registry_synced";
    let unsynced = "at_list_from_registry_unsynced";
    let live = "at_list_from_registry_live";
    cleanup(synced);
    cleanup(unsynced);
    cleanup(live);

    // An agent type with a registry original. The registry copy is ambient, so it is seeded
    // before `boot()`; the local copy belongs to the harness and is seeded after.
    let manifest = registry_manifest_body(synced, "synced with the registry", 42);
    write_registry_agent_type(synced, &manifest);
    let unsynced_manifest = registry_manifest_body(unsynced, "local only", 42);
    let live_manifest = registry_manifest_body(live, "a live agent", 42);

    let h = boot().await;

    // An agent type with a registry original.
    write_agent_type(&h, synced, &manifest);

    // Created locally with no registry counterpart — e.g. through `POST
    // /api/templates` or the `agent_type_create` tool.
    write_agent_type(&h, unsynced, &unsynced_manifest);

    // A live agent's own manifest: never editable, so never eligible to
    // restore from the registry either — the row must not even attempt the
    // lookup a registry-backed name might otherwise accidentally satisfy.
    write_workspace_agent(&h, live, &live_manifest);

    let (status, body) = get(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let templates = body["templates"].as_array().expect("templates array");

    let row = |name: &str| {
        templates
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from list: {body}"))
    };

    assert_eq!(row(synced)["from_registry"], true, "{body}");
    assert_eq!(row(unsynced)["from_registry"], false, "{body}");
    assert_eq!(row(live)["editable"], false, "{body}");
    assert_eq!(row(live)["from_registry"], false, "{body}");

    cleanup(synced);
    cleanup(unsynced);
    cleanup(live);
}
