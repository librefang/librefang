//! Integration tests for the `/api/knowledge` route family.
//!
//! Refs #8327 — documents uploaded once and readable by chosen agents, built on
//! the named-workspace mechanism rather than a new store. Tests exercise the
//! production router (`server::build_router`) with `tower::ServiceExt::oneshot`,
//! so route registration, the real auth middleware and the kernel's manifest
//! write path are all in play. No LLM calls — every test is hermetic.
//!
//! Routes covered:
//!   GET    /api/knowledge
//!   POST   /api/knowledge                                  (create, duplicate 409, hostile name 400)
//!   DELETE /api/knowledge/{name}                           (removes files and revokes holders)
//!   GET    /api/knowledge/{name}/documents
//!   PUT    /api/knowledge/{name}/documents/{filename}      (write + read-back, oversize 413)
//!   DELETE /api/knowledge/{name}/documents/{filename}
//!   PUT    /api/knowledge/{name}/agents                    (grant, mode, revoke)
//!
//! Run: cargo test -p librefang-api --test knowledge_routes_test

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
// Harness
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

const TEST_TOKEN: &str = "test-secret";

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

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn json_req(method: Method, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn raw_put(path: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/octet-stream")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body))
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

async fn create_base(h: &Harness, name: &str) {
    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::POST,
            "/api/knowledge",
            serde_json::json!({ "name": name }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
}

// ---------------------------------------------------------------------------
// Bases
// ---------------------------------------------------------------------------

/// The whole lifecycle in one pass, because each step's evidence is the next
/// step's precondition: an empty listing, a create, the base showing up.
#[tokio::test(flavor = "multi_thread")]
async fn a_created_base_appears_in_the_listing() {
    let h = boot().await;

    let (status, body) = send(h.app.clone(), get("/api/knowledge")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(
        body["bases"],
        serde_json::json!([]),
        "a deployment with no bases must list none, not fail on a missing directory"
    );

    create_base(&h, "handbook").await;

    let (status, body) = send(h.app.clone(), get("/api/knowledge")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["bases"].as_array().expect("array").len(), 1);
    assert_eq!(body["bases"][0]["name"], "handbook");
    assert_eq!(
        body["bases"][0]["path"], "knowledge/handbook",
        "the listed path is what a manifest declaration has to carry"
    );
    assert_eq!(body["bases"][0]["document_count"], 0);
    assert_eq!(body["bases"][0]["agents"], serde_json::json!([]));
}

/// A name that would escape the prefix must be refused by the route, not only
/// by the helper — the helper is only load-bearing if the handler consults it.
#[tokio::test(flavor = "multi_thread")]
async fn a_traversing_name_is_refused_at_the_route() {
    let h = boot().await;

    for hostile in ["../escape", "..", "a/b", ".hidden"] {
        let (status, body) = send(
            h.app.clone(),
            json_req(
                Method::POST,
                "/api/knowledge",
                serde_json::json!({ "name": hostile }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "accepted {hostile:?}: body={body:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_the_same_base_twice_conflicts() {
    let h = boot().await;
    create_base(&h, "handbook").await;

    let (status, _) = send(
        h.app.clone(),
        json_req(
            Method::POST,
            "/api/knowledge",
            serde_json::json!({ "name": "handbook" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_document_written_is_a_document_listed_and_then_removed() {
    let h = boot().await;
    create_base(&h, "handbook").await;

    let (status, body) = send(
        h.app.clone(),
        raw_put(
            "/api/knowledge/handbook/documents/onboarding.md",
            b"# Onboarding\n\nRead this first.\n".to_vec(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["bytes"], 31);

    let (status, body) = send(h.app.clone(), get("/api/knowledge/handbook/documents")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["documents"].as_array().expect("array").len(), 1);
    assert_eq!(body["documents"][0]["filename"], "onboarding.md");
    assert_eq!(body["documents"][0]["bytes"], 31);

    // The listing's totals are what the operator sees on the base card, so they
    // must follow the documents rather than being counted once at create time.
    let (_, body) = send(h.app.clone(), get("/api/knowledge")).await;
    assert_eq!(body["bases"][0]["document_count"], 1);
    assert_eq!(body["bases"][0]["total_bytes"], 31);

    let (status, _) = send(
        h.app.clone(),
        delete("/api/knowledge/handbook/documents/onboarding.md"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(h.app.clone(), get("/api/knowledge/handbook/documents")).await;
    assert_eq!(body["documents"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_into_a_base_that_does_not_exist_is_404_not_a_new_directory() {
    let h = boot().await;

    let (status, _) = send(
        h.app.clone(),
        raw_put("/api/knowledge/ghost/documents/a.md", b"x".to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And the write must not have conjured the base into existence.
    let (_, body) = send(h.app.clone(), get("/api/knowledge")).await;
    assert_eq!(body["bases"], serde_json::json!([]));
}

/// The cap exists because these files land in an agent's context window, so an
/// oversize document has to fail at upload rather than at the turn that reads it.
///
/// Asserting on the message body, not just the status, is what makes this test
/// mean anything: until the route carried its own `DefaultBodyLimit`, axum's
/// 2 MiB extractor default answered 413 first and this test passed with the
/// handler's cap deleted entirely.
#[tokio::test(flavor = "multi_thread")]
async fn an_oversize_document_is_refused() {
    let h = boot().await;
    create_base(&h, "handbook").await;

    let (status, body) = send(
        h.app.clone(),
        raw_put(
            "/api/knowledge/handbook/documents/huge.md",
            vec![b'x'; 4 * 1024 * 1024 + 1],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("4194304"),
        "the 413 must come from the handler's own cap and name it, not from the \
         extractor default: {body:?}"
    );
}

/// The other side of the cap, and the half that fails without the route's own
/// `DefaultBodyLimit`: a document just under the ceiling has to be accepted.
#[tokio::test(flavor = "multi_thread")]
async fn a_document_just_under_the_cap_is_accepted() {
    let h = boot().await;
    create_base(&h, "handbook").await;

    let size = 4 * 1024 * 1024 - 1;
    let (status, body) = send(
        h.app.clone(),
        raw_put("/api/knowledge/handbook/documents/big.md", vec![b'x'; size]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["bytes"], serde_json::json!(size));
}

// ---------------------------------------------------------------------------
// Sharing
// ---------------------------------------------------------------------------

/// An agent may already declare a workspace under the alias a new base wants.
///
/// The rewrite in `set_holders` drops declarations by *path* and re-adds by
/// *alias*, so without a guard the operator's hand-written declaration is
/// replaced rather than kept — and once the base is later revoked, the alias
/// goes with it and the original is unrecoverable from the daemon. Refusing is
/// the only answer that does not decide on the operator's behalf.
///
/// Remove the conflict check in `set_holders` and this test goes red twice: the
/// status becomes 200, and `shared/handbook` is gone from the manifest.
#[tokio::test(flavor = "multi_thread")]
async fn a_base_that_collides_with_an_existing_alias_is_refused_and_changes_nothing() {
    use librefang_types::agent::{WorkspaceDecl, WorkspaceMode};
    use std::path::PathBuf;

    let h = boot().await;
    create_base(&h, "handbook").await;

    let agent = spawn_named(&h.state, "alice");
    let mut hand_written = std::collections::HashMap::new();
    hand_written.insert(
        "handbook".to_string(),
        WorkspaceDecl {
            path: Some(PathBuf::from("shared/handbook")),
            mount: None,
            mode: WorkspaceMode::ReadWrite,
        },
    );
    h.state
        .kernel
        .set_agent_workspaces(agent, hand_written)
        .expect("seed the pre-existing declaration");

    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [{ "agent_id": agent.to_string() }] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "body={body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("shared/handbook"),
        "the refusal must name the declaration it would have destroyed: {body:?}"
    );

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent)
        .expect("agent still registered");
    let kept = entry
        .manifest
        .workspaces
        .get("handbook")
        .expect("the hand-written declaration survives a refused request");
    assert_eq!(
        kept.path.as_deref(),
        Some(std::path::Path::new("shared/handbook"))
    );
    assert_eq!(kept.mode, WorkspaceMode::ReadWrite);
}

/// The point of the whole feature: granting a base to chosen agents writes a
/// real named-workspace declaration into each manifest, visible from both sides.
#[tokio::test(flavor = "multi_thread")]
async fn granting_a_base_writes_the_declaration_and_shows_on_both_sides() {
    let h = boot().await;
    create_base(&h, "handbook").await;
    let reader = spawn_named(&h.state, "reader");
    let writer = spawn_named(&h.state, "writer");
    let outsider = spawn_named(&h.state, "outsider");

    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [
                { "agent_id": reader.to_string() },
                { "agent_id": writer.to_string(), "mode": "rw" },
            ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    // Side one: the base knows who holds it, and in which mode.
    let holders = body["agents"].as_array().expect("array");
    assert_eq!(holders.len(), 2, "body={body:?}");
    let by_name = |name: &str| {
        holders
            .iter()
            .find(|h| h["agent_name"] == name)
            .unwrap_or_else(|| panic!("no holder {name} in {holders:?}"))
            .clone()
    };
    assert_eq!(
        by_name("reader")["mode"],
        "r",
        "an unspecified mode must default to read-only"
    );
    assert_eq!(by_name("writer")["mode"], "rw");
    assert_eq!(by_name("reader")["alias"], "handbook");

    // Side two: the manifest actually carries it, which is what the sandbox and
    // TOOLS.md read. A holder list that agreed with itself but not with the
    // manifest would be exactly the #8321 defect this feature was built to avoid.
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(reader)
        .expect("reader entry");
    let decl = entry
        .manifest
        .workspaces
        .get("handbook")
        .expect("declaration on the reader's manifest");
    assert_eq!(
        decl.path.as_deref(),
        Some(std::path::Path::new("knowledge/handbook"))
    );
    assert_eq!(decl.mode, librefang_types::agent::WorkspaceMode::ReadOnly);

    let outsider_entry = h
        .state
        .kernel
        .agent_registry()
        .get(outsider)
        .expect("outsider entry");
    assert!(
        outsider_entry.manifest.workspaces.is_empty(),
        "an agent left out of the list must not be granted anything"
    );
}

/// Sending a shorter list is how sharing is withdrawn, so the absent agent has
/// to lose the declaration rather than keep it from the previous call.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_left_out_of_a_later_call_loses_the_base() {
    let h = boot().await;
    create_base(&h, "handbook").await;
    let keep = spawn_named(&h.state, "keep");
    let drop_it = spawn_named(&h.state, "dropped");

    let (status, _) = send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [
                { "agent_id": keep.to_string() },
                { "agent_id": drop_it.to_string() },
            ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [ { "agent_id": keep.to_string() } ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["agents"].as_array().expect("array").len(), 1);
    assert_eq!(body["agents"][0]["agent_name"], "keep");

    assert!(
        h.state
            .kernel
            .agent_registry()
            .get(drop_it)
            .expect("entry")
            .manifest
            .workspaces
            .is_empty(),
        "the revoked agent must lose the declaration, not merely drop off the listing"
    );
}

/// Deleting a base has to revoke it too. `ensure_named_workspaces` recreates the
/// directory for any surviving declaration on the next spawn, so a delete that
/// left the manifests alone would come back empty and still be advertised in the
/// agent's TOOLS.md.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_base_revokes_it_from_every_holder() {
    let h = boot().await;
    create_base(&h, "handbook").await;
    let holder = spawn_named(&h.state, "holder");

    send(
        h.app.clone(),
        raw_put("/api/knowledge/handbook/documents/a.md", b"body".to_vec()),
    )
    .await;
    send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [ { "agent_id": holder.to_string() } ]}),
        ),
    )
    .await;

    let (status, body) = send(h.app.clone(), delete("/api/knowledge/handbook")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["revoked_from"], 1);

    assert!(
        h.state
            .kernel
            .agent_registry()
            .get(holder)
            .expect("entry")
            .manifest
            .workspaces
            .is_empty(),
        "a deleted base must not survive as a declaration the next spawn recreates"
    );

    let (_, body) = send(h.app.clone(), get("/api/knowledge")).await;
    assert_eq!(body["bases"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn granting_to_an_unknown_agent_is_404_and_changes_nothing() {
    let h = boot().await;
    create_base(&h, "handbook").await;
    let real = spawn_named(&h.state, "real");

    let (status, _) = send(
        h.app.clone(),
        json_req(
            Method::PUT,
            "/api/knowledge/handbook/agents",
            serde_json::json!({ "agents": [
                { "agent_id": real.to_string() },
                { "agent_id": AgentId::new().to_string() },
            ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert!(
        h.state
            .kernel
            .agent_registry()
            .get(real)
            .expect("entry")
            .manifest
            .workspaces
            .is_empty(),
        "a rejected request must not have half-applied the grant"
    );
}
