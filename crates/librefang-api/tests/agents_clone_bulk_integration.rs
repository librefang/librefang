//! Integration tests for the `/api/agents` clone/reload/push + bulk-ops
//! route clusters.
//!
//! Refs the "agents-mutation-routes-untested" umbrella (Critical) — final
//! slice. Slice 1 (lifecycle) is PR #5628 (`agents_routes_integration.rs`);
//! slice 2 (files + capabilities) is in flight in its own file. This file
//! covers the remaining untested mutation clusters:
//!
//!   POST   /api/agents/{id}/clone   (201 + read-back; 409 on duplicate name;
//!                                     404 on unknown source; 400 invalid id)
//!   POST   /api/agents/{id}/reload  (200 + read-back side effect; negative)
//!   POST   /api/agents/{id}/push    (400 validation; 404 unknown agent;
//!                                     502 when no channel adapter is wired)
//!   POST   /api/agents/bulk         (multi-create + read-back)
//!   DELETE /api/agents/bulk         (multi-delete + read-back 404)
//!   POST   /api/agents/bulk/start   (set Full mode + read-back)
//!   POST   /api/agents/bulk/stop    (no active run → success result rows)
//!   bulk size guard (empty + oversize ids array → 400; refs validate_bulk_size)
//!
//! Like `agents_routes_integration.rs` these exercise the production router
//! (`server::build_router`) via `tower::ServiceExt::oneshot`, so real auth
//! middleware, route registration, and handler logic are all in play. The
//! kernel boots with provider `ollama` + a fake model, so no test makes a
//! real LLM call. The push happy path needs a live channel adapter the
//! hermetic harness can't provide, so it is covered at the validation /
//! not-found / no-adapter (502) status layer instead — see the note on
//! `test_push_message_*`.
//!
//! Run: cargo test -p librefang-api --test agents_clone_bulk_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest, AgentMode};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — boots the production router with a configurable api_key.
// Mirrors `boot()` in agents_routes_integration.rs; additionally retains the
// resolved `home_dir` so the reload test can plant an `agent.toml` at the
// canonical workspaces/agents/<name>/ fallback location the kernel reads.
// ---------------------------------------------------------------------------

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    home_dir: PathBuf,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot(api_key: &str) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Populate the registry cache so the kernel boots without network access.
    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
        None,
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
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

    let home_dir = tmp.path().to_path_buf();
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        home_dir,
        _tmp: tmp,
    }
}

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

/// Bearer token used by all authenticated test requests.
const TEST_TOKEN: &str = "test-secret";

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::empty())
        .unwrap()
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// A syntactically valid but non-existent agent id (random UUID).
fn unknown_agent_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ===========================================================================
// CLONE — POST /api/agents/{id}/clone
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_agent_creates_independent_copy() {
    let h = boot(TEST_TOKEN).await;
    let src = spawn_named(&h.state, "clone-source");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/clone", src),
            serde_json::json!({"new_name": "clone-dest"}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let new_id = body["agent_id"].as_str().expect("agent_id in response");
    assert_eq!(body["name"], "clone-dest");
    assert_ne!(new_id, src.to_string(), "clone must get a fresh id");

    // Read-back: the clone is independently addressable and carries the new name.
    let (status, got) = send(h.app.clone(), get(&format!("/api/agents/{}", new_id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["name"], "clone-dest");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_onto_existing_name_returns_409_not_500() {
    // Audit: agent-not-found-returns-500 — a duplicate-name clone must surface
    // as 409 Conflict (AgentAlreadyExists), NOT a blanket 500. Pre-fix the
    // handler returned 500 for every spawn error.
    let h = boot(TEST_TOKEN).await;
    let src = spawn_named(&h.state, "dup-source");
    spawn_named(&h.state, "already-taken");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/clone", src),
            serde_json::json!({"new_name": "already-taken"}),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "duplicate clone name must be 409, not 500; body: {body}"
    );
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_unknown_source_returns_404_not_500() {
    // Audit: agent-not-found-returns-500 — cloning from a non-existent source
    // must be 404 Not Found, NOT 500.
    let h = boot(TEST_TOKEN).await;

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/clone", unknown_agent_id()),
            serde_json::json!({"new_name": "orphan-clone"}),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown clone source must be 404, not 500; body: {body}"
    );
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/not-a-uuid/clone",
            serde_json::json!({"new_name": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_empty_name_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let src = spawn_named(&h.state, "blank-name-source");
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/clone", src),
            serde_json::json!({"new_name": "   "}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_clone_without_skills_strips_them() {
    // include_skills=false must clear the source's skills on the clone.
    let h = boot(TEST_TOKEN).await;
    let manifest = AgentManifest {
        name: "skilled-source".to_string(),
        skills: vec!["skill-a".to_string(), "skill-b".to_string()],
        ..AgentManifest::default()
    };
    let src = h
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn skilled source");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/clone", src),
            serde_json::json!({"new_name": "skilless-clone", "include_skills": false}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let new_id = body["agent_id"].as_str().expect("agent_id");

    let (status, got) = send(h.app.clone(), get(&format!("/api/agents/{}", new_id))).await;
    assert_eq!(status, StatusCode::OK);
    let skills = got["skills"].as_array().expect("skills array");
    assert!(
        skills.is_empty(),
        "skills should be stripped, got {skills:?}"
    );
    assert_eq!(got["skills_disabled"], true);
}

// ===========================================================================
// RELOAD — POST /api/agents/{id}/reload
// ===========================================================================

/// Plant a minimal valid `agent.toml` at the canonical
/// `<home>/workspaces/agents/<safe_name>/agent.toml` fallback location that
/// `reload_agent_from_disk` reads when an entry has no `source_toml_path`.
/// `safe_name` mirrors the kernel's `safe_path_component` (alphanumeric + `-_`
/// only); the names used here are already safe.
fn plant_agent_toml(home: &std::path::Path, safe_name: &str, body: &str) {
    let dir = home.join("workspaces").join("agents").join(safe_name);
    std::fs::create_dir_all(&dir).expect("create agent workspace dir");
    std::fs::write(dir.join("agent.toml"), body).expect("write agent.toml");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reload_applies_disk_manifest_changes() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "reloadable");

    // Confirm the pre-reload description (default manifest has none).
    let (status, before) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        before["description"].is_null() || before["description"].as_str() == Some(""),
        "expected no description before reload, got {:?}",
        before["description"]
    );

    // Write an agent.toml that changes an observable field, then reload.
    plant_agent_toml(
        &h.home_dir,
        "reloadable",
        r#"
name = "reloadable"
description = "reloaded-from-disk"
"#,
    );

    let (status, body) = send(
        h.app.clone(),
        post_json(&format!("/api/agents/{}/reload", id), serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "reloaded");

    // Read-back: the live manifest now reflects the on-disk description.
    let (status, after) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["description"], "reloaded-from-disk");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reload_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json("/api/agents/not-a-uuid/reload", serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reload_unknown_agent_is_rejected() {
    // The reload handler runs the shared `kernel_err_to_status` mapping, so an
    // unknown id surfaces as 404 NOT_FOUND — consistent with the clone/push
    // routes (which also return 404 for an unknown id). A non-UUID id is a
    // different case (malformed request → 400, see test_reload_invalid_id).
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/reload", unknown_agent_id()),
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "reload of an unknown agent should return 404; body: {body}"
    );
    assert!(body["error"].is_string());
}

// ===========================================================================
// PUSH — POST /api/agents/{id}/push
//
// The push happy path delivers through a live channel adapter (Telegram,
// Slack, …) which the hermetic harness deliberately does not wire up — with
// no adapter the handler returns 502 BAD_GATEWAY. So the success path can't be
// asserted here; instead we pin the status-code correctness of the three
// negative / no-adapter branches the handler owns: 400 (validation / invalid
// id), 404 (unknown agent), and 502 (no channel adapter). The agent-existence
// and field-validation gating is the route's responsibility and IS testable.
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_push_unknown_agent_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/push", unknown_agent_id()),
            serde_json::json!({
                "channel": "telegram",
                "recipient": "123",
                "message": "hi",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_push_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/not-a-uuid/push",
            serde_json::json!({"channel": "telegram", "recipient": "1", "message": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_push_missing_required_fields_returns_400() {
    // Empty channel/recipient/message is rejected with 400 — and crucially
    // this validation runs only AFTER the agent-exists check, so the agent
    // must exist for this branch to be reached.
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "push-validate");
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/push", id),
            serde_json::json!({"channel": "", "recipient": "", "message": ""}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_push_no_channel_adapter_returns_502() {
    // Valid agent + valid fields, but the harness wires no channel adapter for
    // "telegram", so delivery fails at the adapter layer → 502 BAD_GATEWAY
    // (not a 500). This pins the handler's adapter-error mapping.
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "push-no-adapter");
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{}/push", id),
            serde_json::json!({
                "channel": "telegram",
                "recipient": "chat-123",
                "message": "hello",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "body: {body}");
    assert_eq!(body["success"], false);
    assert_eq!(body["agent_id"], id.to_string());
}

// ===========================================================================
// BULK CREATE — POST /api/agents/bulk
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_create_spawns_all_listed_agents() {
    let h = boot(TEST_TOKEN).await;

    // Each bulk-create entry is a `SpawnRequest`: it needs a `manifest_toml`
    // (or a `template`), and `name` overrides the manifest name. A bare
    // `{"name": ...}` has no manifest source and `resolve_manifest` rejects it
    // per-row, so we supply a minimal manifest TOML for each.
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk",
            serde_json::json!({
                "agents": [
                    {"manifest_toml": "name = \"bulk-a\""},
                    {"manifest_toml": "name = \"bulk-b\""},
                    {"manifest_toml": "name = \"bulk-c\""},
                ]
            }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["total"], 3, "body: {body}");
    assert_eq!(body["succeeded"], 3, "body: {body}");
    assert_eq!(body["failed"], 0, "body: {body}");

    // Read-back: every created id is independently addressable.
    let results = body["results"].as_array().expect("results array");
    assert_eq!(results.len(), 3);
    for r in results {
        assert_eq!(r["success"], true, "result row: {r}");
        let id = r["agent_id"].as_str().expect("agent_id");
        let (status, got) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
        assert_eq!(status, StatusCode::OK, "clone {id} not addressable");
        assert!(got["name"].as_str().unwrap().starts_with("bulk-"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_create_empty_array_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json("/api/agents/bulk", serde_json::json!({"agents": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_create_oversize_array_is_rejected() {
    // DoS guard: validate_bulk_size rejects > BULK_LIMIT (50) entries with 400
    // BEFORE allocating with-capacity / spawning anything (refs
    // bulk-with-capacity-no-validate).
    let h = boot(TEST_TOKEN).await;
    let agents: Vec<serde_json::Value> = (0..51)
        .map(|i| serde_json::json!({"name": format!("oversize-{i}")}))
        .collect();
    let (status, body) = send(
        h.app.clone(),
        post_json("/api/agents/bulk", serde_json::json!({"agents": agents})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "oversize bulk create must be 400; body: {body}"
    );
    assert!(body["error"].is_string());

    // And nothing was spawned — the guard runs first.
    let (status, list) = send(h.app.clone(), get("/api/agents?q=oversize-&limit=200")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["total"], 0,
        "guard must short-circuit before any spawn; got {list}"
    );
}

// ===========================================================================
// BULK DELETE — DELETE /api/agents/bulk
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_delete_removes_listed_agents() {
    let h = boot(TEST_TOKEN).await;
    let a = spawn_named(&h.state, "del-a");
    let b = spawn_named(&h.state, "del-b");

    let (status, body) = send(
        h.app.clone(),
        delete_json(
            "/api/agents/bulk",
            serde_json::json!({"agent_ids": [a.to_string(), b.to_string()]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["total"], 2);
    assert_eq!(body["succeeded"], 2);

    // Read-back: both are gone (404 on GET).
    for id in [a, b] {
        let (status, _) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id} should be deleted");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_delete_reports_per_id_failures() {
    // A mix of a valid id, a malformed id, and an unknown id: the endpoint
    // returns 200 with per-row success flags rather than failing the whole
    // batch.
    let h = boot(TEST_TOKEN).await;
    let valid = spawn_named(&h.state, "mixed-del");

    let (status, body) = send(
        h.app.clone(),
        delete_json(
            "/api/agents/bulk",
            serde_json::json!({
                "agent_ids": [valid.to_string(), "not-a-uuid", unknown_agent_id()]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["total"], 3);
    assert_eq!(body["succeeded"], 1, "only the valid id should delete");

    let results = body["results"].as_array().expect("results");
    let valid_row = results
        .iter()
        .find(|r| r["agent_id"] == valid.to_string())
        .expect("valid row present");
    assert_eq!(valid_row["success"], true);
    // The malformed-id row carries an error and did not succeed.
    let bad_row = results
        .iter()
        .find(|r| r["agent_id"] == "not-a-uuid")
        .expect("bad row present");
    assert_eq!(bad_row["success"], false);
    assert!(bad_row["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_delete_oversize_array_is_rejected() {
    let h = boot(TEST_TOKEN).await;
    let ids: Vec<String> = (0..51).map(|_| unknown_agent_id()).collect();
    let (status, body) = send(
        h.app.clone(),
        delete_json("/api/agents/bulk", serde_json::json!({"agent_ids": ids})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

// ===========================================================================
// BULK START — POST /api/agents/bulk/start
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_start_sets_agents_to_full_mode() {
    let h = boot(TEST_TOKEN).await;
    let a = spawn_named(&h.state, "start-a");
    let b = spawn_named(&h.state, "start-b");

    // Precondition: drop both out of Full so the read-back proves the change.
    h.state
        .kernel
        .agent_registry()
        .set_mode(a, AgentMode::Observe)
        .expect("set observe a");
    h.state
        .kernel
        .agent_registry()
        .set_mode(b, AgentMode::Observe)
        .expect("set observe b");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/start",
            serde_json::json!({"agent_ids": [a.to_string(), b.to_string()]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["succeeded"], 2);

    // Read-back: both are now in Full mode (AgentMode serializes snake_case).
    for id in [a, b] {
        let (status, got) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["mode"], "full", "{id} should be Full after bulk start");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_start_unknown_id_reports_failure_row() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/start",
            serde_json::json!({"agent_ids": [unknown_agent_id()]}),
        ),
    )
    .await;
    // The batch endpoint returns 200 with a per-row failure, not a top-level error.
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["succeeded"], 0);
    assert_eq!(body["failed"], 1);
    let row = &body["results"][0];
    assert_eq!(row["success"], false);
    assert!(row["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_start_oversize_array_is_rejected() {
    let h = boot(TEST_TOKEN).await;
    let ids: Vec<String> = (0..51).map(|_| unknown_agent_id()).collect();
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/start",
            serde_json::json!({"agent_ids": ids}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

// ===========================================================================
// BULK STOP — POST /api/agents/bulk/stop
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_stop_with_no_active_runs_succeeds() {
    // No agent has an active LLM run in the hermetic harness, so each stop is
    // a successful no-op ("No active run"). The route still must return 200
    // with success rows for each id.
    let h = boot(TEST_TOKEN).await;
    let a = spawn_named(&h.state, "stop-a");
    let b = spawn_named(&h.state, "stop-b");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/stop",
            serde_json::json!({"agent_ids": [a.to_string(), b.to_string()]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["total"], 2);
    assert_eq!(body["succeeded"], 2);
    for r in body["results"].as_array().expect("results") {
        assert_eq!(r["success"], true, "row: {r}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_stop_empty_array_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/stop",
            serde_json::json!({"agent_ids": []}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_stop_oversize_array_is_rejected() {
    let h = boot(TEST_TOKEN).await;
    let ids: Vec<String> = (0..51).map(|_| unknown_agent_id()).collect();
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents/bulk/stop",
            serde_json::json!({"agent_ids": ids}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body["error"].is_string());
}

// ===========================================================================
// AUTH GATE — bulk + clone require a valid bearer token.
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_create_without_auth_is_401() {
    let h = boot(TEST_TOKEN).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/agents/bulk")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"agents": [{"name": "noauth"}]}).to_string(),
        ))
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
