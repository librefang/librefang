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
//! ### `LIBREFANG_HOME`
//!
//! The handlers resolve their storage under `librefang_home()`, which reads `LIBREFANG_HOME` live
//! on every call. One tempdir is pinned for the whole binary via `OnceLock`, the env var is set
//! exactly once, and the tests serialise behind a `Mutex` so listing assertions do not observe a
//! sibling test's fixtures. Same approach as `profiles_templates_routes_integration.rs`.

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

fn agent_types_dir() -> PathBuf {
    home().join("agent-types")
}

fn agent_type_file(name: &str) -> PathBuf {
    agent_types_dir().join(format!("{name}.toml"))
}

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
    // Force the home init before the kernel boots so nothing reaches the developer's `~/.librefang`.
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

async fn request(h: &Harness, method: &str, path: &str, body: Option<Json>) -> (StatusCode, Json) {
    let builder = axum::http::Request::builder().method(method).uri(path);
    let mut req = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
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

fn write_agent_type(name: &str, body: &str) {
    std::fs::create_dir_all(agent_types_dir()).expect("create agent-types dir");
    std::fs::write(agent_type_file(name), body).expect("write agent type");
}

fn write_workspace_agent(name: &str, body: &str) {
    let dir = home().join("workspaces").join("agents").join(name);
    std::fs::create_dir_all(&dir).expect("create agent workspace");
    std::fs::write(dir.join("agent.toml"), body).expect("write agent.toml");
}

fn cleanup(name: &str) {
    let _ = std::fs::remove_file(agent_type_file(name));
    let _ = std::fs::remove_dir_all(home().join("workspaces").join("agents").join(name));
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
    let _g = lock().lock().await;
    let name = "at_preserve";
    cleanup(name);
    write_agent_type(name, &manifest_with_non_form_fields(name));

    let h = boot().await;
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
        toml::from_str(&std::fs::read_to_string(agent_type_file(name)).unwrap()).unwrap();
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

    cleanup(name);
}

// ---------------------------------------------------------------------------
// Canned substitutions (#7740)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_writes_blank_fields_through_instead_of_substituting_canned_text() {
    let _g = lock().lock().await;
    let name = "at_blank";
    cleanup(name);
    write_agent_type(name, &manifest_with_non_form_fields(name));

    let h = boot().await;
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

    let stored = std::fs::read_to_string(agent_type_file(name)).unwrap();
    let parsed: toml::Value = toml::from_str(&stored).unwrap();
    assert_eq!(parsed["model"]["system_prompt"].as_str(), Some(""));
    assert_eq!(parsed["model"]["provider"].as_str(), Some(""));
    assert_eq!(parsed["model"]["model"].as_str(), Some(""));
    assert!(
        !stored.contains("You are a helpful AI agent."),
        "a deliberately blank system prompt was replaced with canned text: {stored}"
    );

    cleanup(name);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_writes_a_blank_system_prompt_through_unchanged() {
    let _g = lock().lock().await;
    let name = "at_blank_create";
    cleanup(name);

    let h = boot().await;
    let (status, body) = post(
        &h,
        "/api/templates",
        json!({ "name": name, "system_prompt": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let stored = std::fs::read_to_string(agent_type_file(name)).unwrap();
    assert!(
        !stored.contains("You are a helpful AI agent."),
        "create substituted canned text for an explicitly blank prompt: {stored}"
    );
    // A key the caller omitted entirely still gets the manifest's own documented default, which is
    // the sentinel the kernel resolves against `[default_model]`.
    assert_eq!(body["spec"]["provider"], "default");

    cleanup(name);
}

// ---------------------------------------------------------------------------
// Skills round-trip (#7740)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_round_trip_through_the_editor_shape() {
    let _g = lock().lock().await;
    let name = "at_skills";
    cleanup(name);

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

    cleanup(name);
}

// ---------------------------------------------------------------------------
// Dual-source guard (#7731)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_agent_row_is_readable_but_refuses_the_write_verbs() {
    let _g = lock().lock().await;
    let name = "at_liveagent";
    cleanup(name);
    write_workspace_agent(name, &manifest_with_non_form_fields(name));

    let h = boot().await;

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
        home()
            .join("workspaces")
            .join("agents")
            .join(name)
            .join("agent.toml"),
    )
    .unwrap();
    assert!(stored.contains("seeded"), "{stored}");

    cleanup(name);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_refuses_a_name_that_belongs_to_a_live_agent() {
    let _g = lock().lock().await;
    let name = "at_shadow";
    cleanup(name);
    write_workspace_agent(name, &manifest_with_non_form_fields(name));

    let h = boot().await;
    let (status, body) = post(&h, "/api/templates", json!({ "name": name })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "template_name_taken", "{body}");
    assert!(
        !agent_type_file(name).exists(),
        "a refused create still wrote a file"
    );

    cleanup(name);
}

// ---------------------------------------------------------------------------
// CRUD lifecycle and validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn crud_lifecycle_through_the_production_router() {
    let _g = lock().lock().await;
    let name = "at_lifecycle";
    cleanup(name);

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
    let after_refusal = std::fs::read_to_string(agent_type_file(name)).unwrap();
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

    cleanup(name);
}

#[tokio::test(flavor = "multi_thread")]
async fn write_verbs_reject_names_that_would_escape_the_agent_types_directory() {
    let _g = lock().lock().await;
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
    let _g = lock().lock().await;
    let name = "at_typo";
    cleanup(name);
    write_agent_type(name, &manifest_with_non_form_fields(name));

    let h = boot().await;
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

    let stored = std::fs::read_to_string(agent_type_file(name)).unwrap();
    assert!(stored.contains("Seeded prompt."), "{stored}");

    cleanup(name);
}

#[tokio::test(flavor = "multi_thread")]
async fn update_pins_identity_to_the_url_rather_than_the_body() {
    let _g = lock().lock().await;
    let name = "at_identity";
    cleanup(name);
    write_agent_type(name, &manifest_with_non_form_fields(name));

    let h = boot().await;
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
        !agent_type_file("somewhere-else").exists(),
        "a body `name` moved the document out from under the URL that addressed it"
    );

    cleanup(name);
    cleanup("somewhere-else");
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
    let _g = lock().lock().await;
    let name = "at_tool_created";
    cleanup(name);

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

    cleanup(name);
}

/// An operator's document is not something an agent may overwrite by guessing its name.
/// The refusal comes from the shared `File::create_new` claim, so it holds for the tool for the same
/// reason it holds for `POST` — which is exactly what routing both through one store buys.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_refuses_a_name_already_taken_without_clobbering_it() {
    let _g = lock().lock().await;
    let name = "at_tool_dupe";
    cleanup(name);
    write_agent_type(name, &manifest_with_non_form_fields(name));

    let h = boot().await;

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
    let raw = std::fs::read_to_string(agent_type_file(name)).unwrap();
    assert!(raw.contains("max_history_messages = 42"), "{raw}");
    assert!(raw.contains("[[triggers]]"), "{raw}");

    cleanup(name);
}

/// A name that would escape the store directory has to be refused before it is ever joined onto a
/// path, and the refusal has to say what a legal name looks like — the model is the one that has to
/// fix it.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_rejects_a_name_that_would_escape_the_store_directory() {
    let _g = lock().lock().await;
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
        !home().join("escape.toml").exists(),
        "a traversal attempt wrote a file outside the agent-types directory"
    );
}

/// A key the model invented is refused by name rather than dropped, and nothing reaches disk.
/// `AgentTypeSpec` is `deny_unknown_fields` precisely so a typo cannot be read as "keep the old
/// value" — the tool inherits that rather than re-deriving its own idea of the shape.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_rejects_a_spec_carrying_a_field_that_does_not_exist() {
    let _g = lock().lock().await;
    let name = "at_tool_typo";
    cleanup(name);

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

    cleanup(name);
}

/// A name that belongs to a live agent is refused for the tool the same way it is for `POST`:
/// an agent type shadowing it would win every later catalog read and make the agent unreachable.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_refuses_a_name_that_belongs_to_a_live_agent() {
    let _g = lock().lock().await;
    let name = "at_tool_shadow";
    cleanup(name);
    write_workspace_agent(name, &manifest_with_non_form_fields(name));

    let h = boot().await;

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
        !agent_type_file(name).exists(),
        "a refused create left a file behind"
    );

    cleanup(name);
}

/// Omitting provider and model is legal and resolves to the `"default"` sentinel the kernel later
/// maps onto `[default_model]`. The tool reports what was stored rather than echoing what was sent,
/// so a model that omitted them can see what it actually got.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_reports_the_defaults_it_resolved_rather_than_the_fields_it_was_given() {
    let _g = lock().lock().await;
    let name = "at_tool_defaults";
    cleanup(name);

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

    cleanup(name);
}
