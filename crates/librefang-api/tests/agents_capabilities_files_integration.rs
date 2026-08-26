//! Integration tests for the `/api/agents/{id}` **files** and
//! **capabilities** route clusters.
//!
//! Refs `docs/issues/agents-mutation-routes-untested.md` (Critical umbrella) —
//! slice 2 of the agents-mutation backfill. Slice 1 (lifecycle
//! suspend/resume/mode) shipped as PR #5628 / issue #5627 in
//! `agents_routes_integration.rs`; this file is intentionally separate to
//! avoid a merge-append conflict with that still-open PR.
//!
//! Before this file, the files and capabilities mutation routes had **only a
//! registration check** in `dead_route_audit_test.rs` — no test asserted
//! behavior (mutation, status codes, input validation, path-traversal
//! defense). These tests close that gap.
//!
//! All tests run against the **production router** (`server::build_router`)
//! via `tower::ServiceExt::oneshot`, so the real auth middleware, route
//! registration, error envelope, and handler logic are all exercised — the
//! same setup as `boot()` in `agents_routes_integration.rs` and
//! `start_full_router` in `api_integration_test.rs`. No real LLM calls
//! (provider is `ollama` with a fake model); every test is hermetic.
//!
//! Routes covered:
//!   GET  /api/agents/{id}/files                — list identity files
//!   PUT  /api/agents/{id}/files/{filename}     — write (round-trip + traversal)
//!   GET  /api/agents/{id}/files/{filename}     — read back
//!   DELETE /api/agents/{id}/files/{filename}   — delete + read-back-gone
//!   GET/PUT /api/agents/{id}/tools             — tool allow/blocklist, plus the
//!                                                inert-`tool_allowlist`-entry
//!                                                warnings from #6609
//!   GET/PUT /api/agents/{id}/skills            — skill allowlist
//!   GET/PUT /api/agents/{id}/mcp_servers       — MCP server allowlist
//!
//! Run: cargo test -p librefang-api --test agents_capabilities_files_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — boots the production router. Mirrors `boot()` in
// agents_routes_integration.rs (same `server::build_router` path).
// ---------------------------------------------------------------------------

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

/// Bearer token used by all authenticated test requests.
const TEST_TOKEN: &str = "test-secret";

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Seed the pinned registry fixture so the kernel boots with content, offline.
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Spawn an agent with a real, on-disk workspace. `spawn_agent_inner`
/// resolves `manifest.workspace` to an absolute dir under the tempdir,
/// `ensure_workspace`s it, and `generate_identity_files`s the `.identity/`
/// layout — so the files endpoints have a canonicalizable workspace to
/// operate on.
fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

/// Serialize a whole-manifest fixture for `PATCH /api/agents/{id}` with
/// `manifest_toml`.
///
/// Built from a real `AgentManifest` rather than hand-written TOML, so a field
/// rename cannot leave these tests asserting against a document the production
/// parser no longer produces.
fn manifest_toml(name: &str, tags: &[&str]) -> String {
    let manifest = AgentManifest {
        name: name.to_string(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..AgentManifest::default()
    };
    toml::to_string_pretty(&manifest).expect("serialize manifest fixture")
}

/// Drop a minimal `skill.toml` into `<home>/skills/<name>/` so the kernel's
/// registry picks it up on the next `reload_skills()`. Mirrors the helper in
/// `skills_routes_test.rs` / `librefang_skills::registry::tests::create_test_skill`
/// so the schema matches what `SkillRegistry::load_all` accepts. Used by the
/// valid-allowlist round-trip below — the empty / rejection cases need no real
/// skill on disk, but exercising a successful non-empty assignment does.
fn install_skill(home: &std::path::Path, name: &str) {
    let skill_dir = home.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("mkdir skill dir");
    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "0.1.0"
description = "Test skill {name}"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
    );
    std::fs::write(skill_dir.join("skill.toml"), manifest).expect("write skill.toml");
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

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
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

fn delete(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

// ===========================================================================
// FILES cluster — highest-priority (path-traversal risk).
// ===========================================================================

/// Round-trip: PUT a whitelisted identity file, then GET it back and assert
/// the content survived. Exercises the real atomic write (.tmp + rename) and
/// the canonicalization guard on the happy path.
#[tokio::test(flavor = "multi_thread")]
async fn test_files_write_then_read_round_trip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "files-rw");

    let content = "# Soul\nI am a deterministic test agent.\n";
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/files/SOUL.md"),
            serde_json::json!({ "content": content }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "write should succeed: {body:?}");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["name"], "SOUL.md");
    assert_eq!(body["size_bytes"], content.len());

    // Read-after-write — the GET handler must return exactly what we wrote.
    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/files/SOUL.md")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read should succeed: {body:?}");
    assert_eq!(body["name"], "SOUL.md");
    assert_eq!(
        body["content"].as_str(),
        Some(content),
        "round-tripped content must match exactly: {body:?}"
    );
    assert_eq!(body["size_bytes"], content.len());

    // The list endpoint should now mark the file as existing with the
    // written byte length.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/files"))).await;
    assert_eq!(status, StatusCode::OK);
    let files = body["files"].as_array().expect("files array");
    let soul = files
        .iter()
        .find(|f| f["name"] == "SOUL.md")
        .expect("SOUL.md listed");
    assert_eq!(soul["exists"], true, "SOUL.md should now exist: {soul:?}");
    assert_eq!(soul["size_bytes"], content.len());
}

/// Concurrent requests must not share a staging path. Every request goes
/// through the production router and the final file must equal one complete
/// payload, never a truncated or interleaved combination.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_files_concurrent_writes_leave_one_payload_intact() {
    const WRITERS: usize = 8;
    let h = boot().await;
    let id = spawn_named(&h.state, "files-concurrent-write");
    let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS));
    let payloads: Vec<String> = (0..WRITERS)
        .map(|index| format!("payload-{index}").repeat(index * 500 + 1))
        .collect();

    let mut tasks = Vec::with_capacity(WRITERS);
    for payload in &payloads {
        let payload = payload.clone();
        let app = h.app.clone();
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            send(
                app,
                put_json(
                    &format!("/api/agents/{id}/files/SOUL.md"),
                    serde_json::json!({ "content": payload }),
                ),
            )
            .await
        }));
    }

    for task in tasks {
        let (status, body) = task.await.expect("writer task");
        assert_eq!(status, StatusCode::OK, "concurrent write failed: {body:?}");
    }

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/files/SOUL.md")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read should succeed: {body:?}");
    let written = body["content"].as_str().expect("content string");
    assert!(
        payloads.iter().any(|payload| payload == written),
        "content must equal exactly one complete payload; got {} bytes",
        written.len()
    );
}

/// SECURITY (highest-value test): a path-traversal filename must be rejected
/// with a 4xx — never written, never a 500. The filename whitelist
/// (`KNOWN_IDENTITY_FILES`) rejects `../../etc/passwd` before any path
/// resolution, so the expected status is 400. We additionally assert that no
/// write occurred by confirming the read-back of the same traversal path is
/// also a clean 4xx (no content leaked).
#[tokio::test(flavor = "multi_thread")]
async fn test_files_write_path_traversal_rejected_4xx() {
    let h = boot().await;
    let id = spawn_named(&h.state, "files-traversal-write");

    // Encoded so axum routes it to the {filename} segment rather than
    // splitting on raw `/`. `%2e%2e%2f` = `../`. This lands a single path
    // segment of `../../etc/passwd` into the handler, which the whitelist
    // check rejects.
    let traversal = "%2e%2e%2f%2e%2e%2fetc%2fpasswd";
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/files/{traversal}"),
            serde_json::json!({ "content": "root:x:0:0::/root:/bin/sh\n" }),
        ),
    )
    .await;
    assert!(
        status.is_client_error(),
        "path-traversal write must be a 4xx, got {status}: {body:?}"
    );
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "whitelist rejection is a 400 (fires before path resolution): {body:?}"
    );
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "must not be a 500"
    );
    assert!(
        body["error"].is_string() || body["error"].is_object(),
        "rejection must carry an error envelope: {body:?}"
    );

    // Confirm nothing was written outside the workspace: reading the same
    // traversal path back must also be a clean 4xx (no leaked content). A
    // 200 here would mean the write escaped the sandbox.
    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/files/{traversal}")),
    )
    .await;
    assert!(
        status.is_client_error(),
        "reading the traversal path must be a 4xx, got {status}: {body:?}"
    );
    assert!(
        body["content"].is_null(),
        "no file content may leak via the traversal path: {body:?}"
    );
}

/// A plain non-whitelisted filename (no traversal) is also rejected with 400
/// — the whitelist is the boundary, not just `..` detection.
#[tokio::test(flavor = "multi_thread")]
async fn test_files_write_non_whitelisted_name_rejected_400() {
    let h = boot().await;
    let id = spawn_named(&h.state, "files-non-whitelist");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/files/arbitrary.txt"),
            serde_json::json!({ "content": "nope" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "non-whitelisted filename must be 400: {body:?}"
    );
    assert!(body["error"].is_string() || body["error"].is_object());
}

/// DELETE removes a previously-written identity file; the follow-up read
/// must report it gone (404), and the list endpoint must flip `exists` back
/// to false.
#[tokio::test(flavor = "multi_thread")]
async fn test_files_delete_then_gone() {
    let h = boot().await;
    let id = spawn_named(&h.state, "files-delete");

    // Write first so there is something to delete.
    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/files/IDENTITY.md"),
            serde_json::json!({ "content": "name: deleteme\n" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        h.app.clone(),
        delete(&format!("/api/agents/{id}/files/IDENTITY.md")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete should succeed: {body:?}");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["name"], "IDENTITY.md");

    // Read-back: the file is gone.
    let (status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/files/IDENTITY.md")),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "deleted file must read back as 404"
    );
}

/// Unknown agent on the files write path → 404, not 500.
#[tokio::test(flavor = "multi_thread")]
async fn test_files_write_unknown_agent_returns_404() {
    let h = boot().await;
    let unknown = AgentId::new();

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unknown}/files/SOUL.md"),
            serde_json::json!({ "content": "hi" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown agent must be 404, not 500: {body:?}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// Invalid (non-UUID) agent id on the files write path → 400.
#[tokio::test(flavor = "multi_thread")]
async fn test_files_write_invalid_agent_id_returns_400() {
    let h = boot().await;
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/api/agents/not-a-uuid/files/SOUL.md",
            serde_json::json!({ "content": "hi" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

// ===========================================================================
// CAPABILITIES cluster — tools / skills / mcp_servers.
// ===========================================================================

/// Tools: PUT an allowlist + blocklist, assert the PUT response reflects the
/// change, then GET and assert read-back persistence.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_set_then_read_round_trip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-rw");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read", "file_write"],
                "tool_allowlist": ["file_read"],
                "tool_blocklist": ["file_write"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set tools should succeed: {body:?}");
    // The handler reads the agent back and returns the new shape directly.
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["file_read"]),
        "PUT response should echo the new allowlist: {body:?}"
    );
    assert_eq!(
        body["tool_blocklist"],
        serde_json::json!(["file_write"]),
        "PUT response should echo the new blocklist: {body:?}"
    );

    // Read-after-write via GET.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/tools"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["file_read"]),
        "GET must reflect the persisted allowlist: {body:?}"
    );
    assert_eq!(
        body["tool_blocklist"],
        serde_json::json!(["file_write"]),
        "GET must reflect the persisted blocklist: {body:?}"
    );
    assert_eq!(
        body["capabilities_tools"],
        serde_json::json!(["file_read", "file_write"])
    );
}

/// Tools: an empty body (no fields set) is a no-op request and must be
/// rejected with 400 rather than silently succeeding.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_empty_body_rejected_400() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-empty");

    let (status, body) = send(
        h.app.clone(),
        put_json(&format!("/api/agents/{id}/tools"), serde_json::json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty tools body must be 400: {body:?}"
    );
}

/// Tools: unknown agent → 404, not 500. The handler has an explicit
/// existence pre-check for this case.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_unknown_agent_returns_404() {
    let h = boot().await;
    let unknown = AgentId::new();

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unknown}/tools"),
            serde_json::json!({ "tool_allowlist": ["file_read"] }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown agent must be 404, not 500: {body:?}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// Skills: setting an empty allowlist (no restriction) succeeds and reads
/// back as empty. A non-empty list is validated against the skill registry,
/// so an empty list is the deterministic, network-free happy path.
#[tokio::test(flavor = "multi_thread")]
async fn test_skills_set_empty_then_read_round_trip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "skills-rw");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": [] }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "set skills should succeed: {body:?}"
    );
    assert_eq!(body["status"], "ok");
    assert_eq!(body["skills"], serde_json::json!([]));

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"],
        serde_json::json!([]),
        "GET must reflect the empty allowlist: {body:?}"
    );
    assert!(
        body["available"].is_array(),
        "GET should advertise available skills: {body:?}"
    );
}

/// Skills (#4917): assigning a *valid* non-empty allowlist persists and reads
/// back, and `mode` flips from "all" to "allowlist". This is the happy path
/// the dashboard's inline assignment UI drives — the empty round-trip above
/// only proves the clear/all path, not that a concrete skill survives a PUT.
#[tokio::test(flavor = "multi_thread")]
async fn test_skills_set_valid_allowlist_round_trip() {
    let h = boot().await;
    // Seed a real skill so the kernel's registry validation accepts it.
    install_skill(h._tmp.path(), "round_trip_skill");
    h.state.kernel.reload_skills();

    let id = spawn_named(&h.state, "skills-allowlist");

    // Before assignment the agent uses every registry skill → mode "all".
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["mode"], "all",
        "fresh agent should default to all: {body:?}"
    );
    assert!(
        body["available"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "round_trip_skill")),
        "seeded skill must surface in available: {body:?}"
    );

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": ["round_trip_skill"] }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "valid allowlist must succeed: {body:?}"
    );
    assert_eq!(body["skills"], serde_json::json!(["round_trip_skill"]));

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"],
        serde_json::json!(["round_trip_skill"]),
        "GET must reflect the assigned allowlist: {body:?}"
    );
    assert_eq!(
        body["mode"], "allowlist",
        "a non-empty allowlist flips mode to allowlist: {body:?}"
    );

    // Clearing it returns to all-mode — the dashboard's "Reset to all".
    let (status, _body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": [] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(
        body["mode"], "all",
        "empty PUT returns to all-mode: {body:?}"
    );
    assert_eq!(body["assigned"], serde_json::json!([]));
}

/// Skills: an unknown skill name in a non-empty allowlist is rejected by the
/// kernel's registry validation. This is the safe-rejection assertion the
/// scope calls for (no real network/skill install is performed) — it must be
/// a clean 4xx, never a 500.
#[tokio::test(flavor = "multi_thread")]
async fn test_skills_unknown_name_rejected_4xx() {
    let h = boot().await;
    let id = spawn_named(&h.state, "skills-unknown");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": ["__definitely_not_a_real_skill__"] }),
        ),
    )
    .await;
    assert!(
        status.is_client_error(),
        "unknown skill must be a 4xx, got {status}: {body:?}"
    );
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "must not be a 500: {body:?}"
    );
    assert!(body["error"].is_string() || body["error"].is_object());

    // The rejection must not have mutated the agent: GET still shows empty.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"],
        serde_json::json!([]),
        "rejected skill assignment must not persist: {body:?}"
    );
}

/// MCP servers: setting an empty allowlist (= no servers, #5855) succeeds and
/// reads back as empty with mode "none".
#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_servers_set_empty_then_read_round_trip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "mcp-rw");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/mcp_servers"),
            serde_json::json!({ "mcp_servers": [] }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "set mcp_servers should succeed: {body:?}"
    );
    assert_eq!(body["status"], "ok");
    assert_eq!(body["mcp_servers"], serde_json::json!([]));

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/mcp_servers"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"],
        serde_json::json!([]),
        "GET must reflect the empty allowlist: {body:?}"
    );
    assert_eq!(
        body["mode"], "none",
        "empty allowlist means no MCP servers (#5855): {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_servers_rejects_documented_wrong_and_malformed_body_shapes() {
    let h = boot().await;
    let id = spawn_named(&h.state, "mcp-invalid-body");
    h.state
        .kernel
        .agent_registry()
        .update_mcp_servers(id, vec!["keep-server".to_string()])
        .expect("seed assignment");

    for body in [
        serde_json::json!(["replacement"]),
        serde_json::json!({"mcp_servers": [1]}),
        serde_json::json!({"servers": []}),
    ] {
        let (status, _) = send(
            h.app.clone(),
            put_json(&format!("/api/agents/{id}/mcp_servers"), body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    assert_eq!(
        h.state
            .kernel
            .agent_registry()
            .get(id)
            .unwrap()
            .manifest
            .mcp_servers,
        vec!["keep-server"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mcp_servers_update_preserves_manifest_comments_and_omitted_defaults() {
    let h = boot().await;
    let id = spawn_named(&h.state, "mcp-format-preserve");
    h.state.kernel.persist_manifest_to_disk(id);

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("spawned agent");
    let manifest_path = entry
        .manifest
        .workspace
        .as_ref()
        .expect("spawned agent workspace")
        .join("agent.toml");
    let generated = std::fs::read_to_string(&manifest_path).expect("generated agent.toml");
    let customized = generated
        .replace(
            "mcp_servers = []",
            "mcp_servers = [\n    \"old-server\",\n] # keep assignment note",
        )
        .lines()
        .filter(|line| !line.trim_start().starts_with("full_mode_skips_approval ="))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        &manifest_path,
        format!("# operator decision: keep this note\n{customized}\n"),
    )
    .expect("customize agent.toml");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/mcp_servers"),
            serde_json::json!({ "mcp_servers": [] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body:?}");

    let updated = std::fs::read_to_string(&manifest_path).expect("updated agent.toml");
    assert!(
        updated.contains("# operator decision: keep this note"),
        "the field update must preserve operator comments:\n{updated}"
    );
    assert!(
        !updated.contains("full_mode_skips_approval ="),
        "the field update must not inject an omitted default:\n{updated}"
    );
    assert!(updated.contains("mcp_servers = [] # keep assignment note"));
    assert!(!updated.contains("old-server"));

    let generic_source = updated.replace(
        "mcp_servers = [] # keep assignment note",
        "mcp_servers = [\n    \"generic-old\",\n] # keep generic note",
    );
    std::fs::write(&manifest_path, &generic_source).expect("prepare generic PATCH manifest");
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "mcp_servers": [] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "generic PATCH failed: {body:?}");

    let generic_updated = std::fs::read_to_string(&manifest_path).expect("generic PATCH output");
    assert!(generic_updated.contains("# operator decision: keep this note"));
    assert!(generic_updated.contains("mcp_servers = [] # keep generic note"));
    assert!(!generic_updated.contains("generic-old"));
    assert!(!generic_updated.contains("full_mode_skips_approval ="));

    let nested_source = generic_updated.replace(
        "mcp_servers = [] # keep generic note",
        "mcp_servers = [\n    \"nested-old\",\n] # keep nested note",
    );
    std::fs::write(&manifest_path, &nested_source).expect("prepare nested PATCH manifest");
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "capabilities": { "mcp_servers": [] } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "nested PATCH failed: {body:?}");

    let nested_updated = std::fs::read_to_string(&manifest_path).expect("nested PATCH output");
    assert!(nested_updated.contains("# operator decision: keep this note"));
    assert!(nested_updated.contains("mcp_servers = [] # keep nested note"));
    assert!(!nested_updated.contains("nested-old"));
    assert!(!nested_updated.contains("full_mode_skips_approval ="));
}

/// Capabilities GET on an unknown agent → 404, not 500 (tools/skills/mcp all
/// share this branch shape on the read side).
#[tokio::test(flavor = "multi_thread")]
async fn test_capabilities_get_unknown_agent_returns_404() {
    let h = boot().await;
    let unknown = AgentId::new();

    for sub in ["tools", "skills", "mcp_servers"] {
        let (status, body) =
            send(h.app.clone(), get(&format!("/api/agents/{unknown}/{sub}"))).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "GET {sub} on unknown agent must be 404: {body:?}"
        );
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

/// Capabilities mutation routes reject unauthenticated requests (the auth
/// middleware is in play because we hit the real `build_router`). Confirms
/// these mutation routes are not accidentally on the public allowlist.
#[tokio::test(flavor = "multi_thread")]
async fn test_capabilities_put_requires_auth() {
    let h = boot().await;
    let id = spawn_named(&h.state, "auth-gate");

    // No Authorization header.
    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/agents/{id}/tools"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "tool_allowlist": ["file_read"] }).to_string(),
        ))
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "unauthenticated capabilities mutation must be 401"
    );
}

// ===========================================================================
// PUT /api/agents/{id}/tools — inert `tool_allowlist` entry warnings (#6609)
// ===========================================================================
//
// The kernel applies `tool_allowlist` in `available_tools` Step 4 as
// `all_tools.retain(...)`, after `capabilities.tools` has already filtered the
// candidate set in Step 1. `retain` can only remove, so an allowlist entry
// naming a builtin the declared set excludes grants nothing. Those semantics
// are deliberate and unchanged; what these tests pin is that the write API no
// longer accepts such an entry silently.

/// Collect the response's `warnings` array as plain strings. Absent key → empty.
fn warnings_of(body: &serde_json::Value) -> Vec<String> {
    body.get("warnings")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// An allowlist entry that `capabilities_tools` cannot admit is accepted and persisted (semantics unchanged) but named in the response's `warnings`.
/// Both fields go in one request so the check is proven to run against the declared set the request just wrote, not the one it replaced.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_warns_about_inert_allowlist_entry() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-inert");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read"],
                "tool_allowlist": ["web_search"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT must still succeed: {body:?}");

    let warnings = warnings_of(&body);
    assert_eq!(
        warnings.len(),
        1,
        "exactly one inert entry expected: {body:?}"
    );
    assert!(
        warnings[0].contains("web_search"),
        "the warning must name the inert entry: {body:?}"
    );

    // Semantics are unchanged — the value is still stored and echoed.
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["web_search"]),
        "the allowlist must still be persisted: {body:?}"
    );
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/tools"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tool_allowlist"], serde_json::json!(["web_search"]));
}

/// False-positive guard: an entry that `capabilities_tools` does admit produces no warning at all.
/// Without this a diagnostic that warned on every request would still satisfy the test above.
///
/// The declared entry is a glob (`file_*`) and the allowlist entry a literal (`file_read`), so a string-equality implementation of the check would flag a working configuration here — the match has to be glob-evaluated.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_does_not_warn_about_a_live_allowlist_entry() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-live");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_*"],
                "tool_allowlist": ["file_read"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "a live allowlist entry must produce no warnings key: {body:?}"
    );
}

/// An unbounded `capabilities_tools` makes the Step 1 filter a no-op, so no entry is provably inert and nothing may be reported — warning here would be a false positive on a configuration that grants every builtin.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_does_not_warn_when_capabilities_tools_is_unbounded() {
    let h = boot().await;

    // Explicit `*` wildcard grant.
    let wildcard = spawn_named(&h.state, "tools-wildcard");
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{wildcard}/tools"),
            serde_json::json!({
                "capabilities_tools": ["*"],
                "tool_allowlist": ["web_search", "definitely_not_a_tool"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "a `*` grant admits everything, so nothing is provably inert: {body:?}"
    );

    // Empty list — the kernel's other "unrestricted" spelling.
    let unrestricted = spawn_named(&h.state, "tools-unrestricted");
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unrestricted}/tools"),
            serde_json::json!({
                "capabilities_tools": [],
                "tool_allowlist": ["web_search"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "an empty capabilities_tools is unrestricted, so nothing is provably inert: {body:?}"
    );
}

/// MCP tools reach the candidate set without being filtered by `capabilities_tools`, and an `mcp_*` allowlist entry is how a caller selects among them (#6495).
/// Warning about one would break a working configuration.
/// A wildcard entry is likewise never reported: a later skill install or MCP connect can make it match.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_does_not_warn_about_mcp_or_glob_entries() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-mcp-glob");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read"],
                "tool_allowlist": ["mcp_github_create_issue", "mcp_*", "web_*"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "MCP-namespaced and glob entries are never provably inert: {body:?}"
    );
}

/// A request that touches neither the allowlist nor the grant surface says nothing about either, so it must stay quiet even when the stored allowlist is inert.
/// A blocklist-only request is the shape that qualifies; narrowing `capabilities_tools` does not, and is covered by the test below.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_only_warns_about_submitted_allowlist_entries() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-not-submitted");

    // Seed an inert entry (and take the warning).
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read"],
                "tool_allowlist": ["web_search"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(warnings_of(&body).len(), 1, "{body:?}");

    // A follow-up request that touches only the blocklist leaves the stored allowlist alone and must stay quiet about it.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "tool_blocklist": ["shell_exec"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["web_search"]),
        "the stored allowlist must be untouched: {body:?}"
    );
    assert!(
        body.get("warnings").is_none(),
        "a request that did not submit tool_allowlist must not warn about it: {body:?}"
    );
}

/// Narrowing `capabilities_tools` alone can silence a stored allowlist entry without the request ever mentioning the allowlist, and from the operator's side that is the same defect: their own request is what stopped the tool from being granted, and the response is a clean `200`.
/// Gating the diagnostic on a submitted `tool_allowlist` alone would stay quiet here.
///
/// The reverse shape — widening the grant surface back to unbounded, which is the most common real request that submits only `capabilities_tools` — must go quiet again rather than warn about the same stored entry.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_put_warns_when_narrowing_capabilities_makes_a_stored_entry_inert() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-narrowed");

    // A working configuration: an unbounded grant with a live allowlist.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["*"],
                "tool_allowlist": ["web_search"],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "a `*` grant admits web_search, so nothing is inert yet: {body:?}"
    );

    // Narrow the grant surface and nothing else. The stored allowlist stays at
    // ["web_search"], which the new declared set no longer admits.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "capabilities_tools": ["file_read"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let warnings = warnings_of(&body);
    assert_eq!(
        warnings.len(),
        1,
        "narrowing capabilities_tools must report the stored entry it silenced: {body:?}"
    );
    assert!(
        warnings[0].contains("web_search"),
        "the warning must name the silenced entry: {body:?}"
    );
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["web_search"]),
        "a capabilities-only request must leave the stored allowlist untouched: {body:?}"
    );

    // Widen the grant surface back — nothing is provably inert against an
    // unbounded declared set, so the warning must disappear.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "capabilities_tools": ["*"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "an unbounded grant admits everything, so nothing is provably inert: {body:?}"
    );
}

// ===========================================================================
// `tools_disabled` read/write parity, and `tags` reachability (#7742).
// ===========================================================================

/// `GET /api/agents/{id}/tools` has always returned `disabled`, but no request
/// could write it: `update_tool_config` forced it to `false` on every
/// successful write, so `PUT {"disabled": true}` was accepted and undone in the
/// same call. Assert both directions now round-trip.
#[tokio::test(flavor = "multi_thread")]
async fn test_tools_disabled_round_trips_in_both_directions() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-switch");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "disabled": true }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a body carrying only `disabled` is a real change, not an empty request: {body:?}"
    );
    assert_eq!(
        body["disabled"],
        serde_json::json!(true),
        "the PUT response must echo the stored flag: {body:?}"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/tools"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["disabled"],
        serde_json::json!(true),
        "GET must reflect the persisted flag: {body:?}"
    );

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "disabled": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["disabled"],
        serde_json::json!(false),
        "clearing the flag must be just as reachable as setting it: {body:?}"
    );

    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/tools"))).await;
    assert_eq!(body["disabled"], serde_json::json!(false), "{body:?}");
}

/// Regression for the destructive half of the same defect: an omitted field
/// means "leave it alone", for all four fields.
///
/// Editing a blocklist used to re-enable every tool on an agent whose operator
/// had deliberately switched them off, because the forced clear ran on every
/// successful write regardless of what the request contained.
#[tokio::test(flavor = "multi_thread")]
async fn test_omitted_tool_fields_are_left_unchanged() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-partial");

    // Seed all four fields in one request.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read", "file_write"],
                "tool_allowlist": ["file_read"],
                "tool_blocklist": ["file_write"],
                "disabled": true,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Touch exactly one of them.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "tool_blocklist": ["web_search"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/tools"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["tool_blocklist"],
        serde_json::json!(["web_search"]),
        "the submitted field must be written: {body:?}"
    );
    assert_eq!(
        body["disabled"],
        serde_json::json!(true),
        "a blocklist edit must not silently re-enable every tool (#7742): {body:?}"
    );
    assert_eq!(
        body["tool_allowlist"],
        serde_json::json!(["file_read"]),
        "an omitted allowlist must not be cleared: {body:?}"
    );
    assert_eq!(
        body["capabilities_tools"],
        serde_json::json!(["file_read", "file_write"]),
        "an omitted grant surface must not be cleared: {body:?}"
    );
}

/// While every tool is switched off, the #6609 inert-allowlist-entry
/// diagnostic has nothing useful to say: the master switch already admits
/// nothing, so naming a "silenced" entry would be advice about the second
/// stage of a pipeline whose first stage is closed.
#[tokio::test(flavor = "multi_thread")]
async fn test_inert_entry_warnings_suppressed_while_tools_disabled() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tools-quiet");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({
                "capabilities_tools": ["file_read"],
                "tool_allowlist": ["web_search"],
                "disabled": true,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body.get("warnings").is_none(),
        "no allowlist entry can admit anything while tools_disabled is set: {body:?}"
    );

    // Re-enable and the same stored allowlist entry is worth reporting again.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/tools"),
            serde_json::json!({ "capabilities_tools": ["file_read"], "disabled": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let warnings = warnings_of(&body);
    assert_eq!(
        warnings.len(),
        1,
        "warnings should be back once tools are on again: {body:?}"
    );
    assert!(warnings[0].contains("web_search"), "{body:?}");
}

/// `tags` used to be the one manifest field no route could reach: every write
/// path funnels into `update_manifest`, which pinned the incoming tags back to
/// the stored ones. The operator got a 200 and no change.
///
/// Assert the write lands, survives a hot-reload from the `agent.toml` that
/// `update_manifest` persisted, and that an operator can drop a tag as well as
/// add one.
#[tokio::test(flavor = "multi_thread")]
async fn test_manifest_toml_tags_are_writable_and_survive_reload() {
    let h = boot().await;
    let id = spawn_named(&h.state, "tags-rw");

    let manifest_with_tags = manifest_toml("tags-rw", &["research", "beta"]);

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "manifest_toml": manifest_with_tags }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "manifest update should succeed: {body:?}"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["tags"],
        serde_json::json!(["research", "beta"]),
        "tags submitted through manifest_toml must actually be stored (#7742): {body:?}"
    );

    // `update_manifest` persisted the manifest to agent.toml; reading it back
    // through the hot-reload path must not resurrect the old tags.
    h.state
        .kernel
        .reload_agent_from_disk(id)
        .expect("reload should succeed");

    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(
        body["tags"],
        serde_json::json!(["research", "beta"]),
        "tags must survive a reload from the manifest they were persisted into: {body:?}"
    );

    // Dropping a tag is as much a change as adding one.
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "manifest_toml": manifest_toml("tags-rw", &["beta"]) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(
        body["tags"],
        serde_json::json!(["beta"]),
        "an operator must be able to remove a tag, not only add one: {body:?}"
    );
}

/// The system-owned half of `tags` stays out of operator reach in both
/// directions.
///
/// `hand:` / `hand_role:` route an agent's workspace under `hands/<hand>/<role>`
/// and decide whether its tool calls need an approval gate, so letting a
/// manifest edit drop them would relocate a workspace and re-decide a security
/// boundary through a field that reads like free-form metadata — and letting
/// one add them would forge hand membership.
#[tokio::test(flavor = "multi_thread")]
async fn test_manifest_toml_cannot_forge_or_drop_system_tags() {
    let h = boot().await;
    let manifest = AgentManifest {
        name: "hand-tagged".to_string(),
        tags: vec![
            "hand:clipper".to_string(),
            "hand_role:editor".to_string(),
            "research".to_string(),
        ],
        ..AgentManifest::default()
    };
    let id = h
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent");

    let edited = manifest_toml("hand-tagged", &["hand:forged", "hand_role:admin", "prod"]);

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "manifest_toml": edited }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a submitted system tag is dropped, not rejected — the dashboard round-trips the tags it was shown: {body:?}"
    );

    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(
        body["tags"],
        serde_json::json!(["hand:clipper", "hand_role:editor", "prod"]),
        "live hand membership must survive, the forged one must not land, and the operator half must be replaced wholesale: {body:?}"
    );
}
