//! Integration tests for the prompts router (#3571 — prompts slice).
//!
//! Mounts `routes::prompts::router()` directly under `/api` against a
//! `TestAppState` + `MockKernelBuilder`-built `LibreFangKernel`. The kernel
//! has a real prompt store wired in, so mutating endpoints persist data
//! that subsequent reads can observe. Tests cover happy-path round trips
//! plus the path-parsing rejection paths (non-UUID `agent_id`) and the
//! body-validation path (`activate` requires `agent_id`).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::prompts::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(
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
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

const AGENT_UUID: &str = "11111111-1111-1111-1111-111111111111";
const VERSION_ID: &str = "22222222-2222-2222-2222-222222222222";
const EXPERIMENT_ID: &str = "33333333-3333-3333-3333-333333333333";

fn seeded_agent_id(h: &Harness) -> String {
    h.state
        .kernel
        .agent_registry()
        .list()
        .into_iter()
        .find(|agent| agent.name == "assistant")
        .expect("mock kernel must seed the assistant agent")
        .id
        .to_string()
}

async fn create_prompt_version(
    h: &Harness,
    agent_id: &str,
    system_prompt: &str,
) -> serde_json::Value {
    let (status, body) = json_request(
        h,
        Method::POST,
        &format!("/api/agents/{agent_id}/prompts/versions"),
        Some(serde_json::json!({ "system_prompt": system_prompt })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    body
}

// ----- repository overview -----

#[tokio::test(flavor = "multi_thread")]
async fn prompts_overview_returns_paginated_envelope() {
    // The MockKernelBuilder seeds the default "assistant" agent, so the
    // overview is non-empty. Assert the standard paginated envelope (200 +
    // `items`/`total`/`offset`/`limit`) and that the seeded agent appears as
    // one summary row with the expected shape: its live system prompt is
    // surfaced, and with no rows in the prompt-version store it reports zero
    // versions and no active version. This pins the cross-agent aggregation
    // route wiring and its per-agent row contract.
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/prompts/overview", None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let items = body["items"].as_array().expect("items is array");
    assert!(
        !items.is_empty(),
        "expected at least the seeded agent: {body:?}"
    );
    assert_eq!(body["total"], items.len());
    assert_eq!(body["offset"], 0);
    assert!(body.get("limit").is_some(), "limit field present: {body:?}");

    let row = &items[0];
    // agent_id parses as a UUID; agent_name is non-empty.
    let agent_id = row["agent_id"].as_str().expect("agent_id string");
    assert!(
        uuid::Uuid::parse_str(agent_id).is_ok(),
        "agent_id must be a UUID: {row:?}"
    );
    assert!(
        !row["agent_name"].as_str().unwrap_or_default().is_empty(),
        "agent_name must be non-empty: {row:?}"
    );
    // The live system prompt is surfaced from the manifest.
    assert!(
        row["live_system_prompt"].is_string(),
        "live_system_prompt must be present: {row:?}"
    );
    // No versions persisted for the seeded agent → zero count, null active.
    assert_eq!(row["version_count"], 0, "row={row:?}");
    assert_eq!(
        row["active_version"],
        serde_json::Value::Null,
        "row={row:?}"
    );
    assert_eq!(
        row["active_version_id"],
        serde_json::Value::Null,
        "row={row:?}"
    );
}

// ----- prompt versions -----

#[tokio::test(flavor = "multi_thread")]
async fn list_prompt_versions_empty_for_unknown_agent() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert!(body.get("limit").is_some(), "limit field present: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_prompt_versions_rejects_non_uuid_agent_id() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/agents/not-a-uuid/prompts/versions",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(
        body.get("error").is_some(),
        "expected error envelope: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_round_trips_through_get_and_list() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "system_prompt": "You are a helpful assistant.",
            "description": "initial",
        })),
    )
    .await;
    // Issue #3832: POST /versions creates a resource — must be 201 Created.
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    assert_eq!(body["agent_id"], AGENT_UUID);
    assert_eq!(body["system_prompt"], "You are a helpful assistant.");
    // Server must compute a sha256 content_hash from system_prompt and
    // assign a fresh UUID + creation timestamp.
    let hash = body["content_hash"].as_str().expect("content_hash string");
    assert_eq!(hash.len(), 64, "sha256 hex = 64 chars, got {hash:?}");
    let new_id = body["id"].as_str().expect("id string").to_string();
    assert_ne!(new_id, "00000000-0000-0000-0000-000000000000");

    // List should now contain it.
    let (status, listed) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = listed["items"].as_array().expect("items is array");
    assert!(
        arr.iter().any(|v| v["id"] == new_id),
        "expected new version in list: {listed:?}"
    );
    assert_eq!(listed["total"], arr.len());

    // GET single should return the same record.
    let (status, fetched) = json_request(
        &h,
        Method::GET,
        &format!("/api/prompts/versions/{new_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"], new_id);
    assert_eq!(fetched["system_prompt"], "You are a helpful assistant.");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_rejects_non_uuid_agent_id() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/agents/not-a-uuid/prompts/versions",
        Some(serde_json::json!({"system_prompt": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_prompt_version_returns_null_for_unknown_id() {
    // Default KernelHandle::get_prompt_version returns Ok(None) which the
    // route serializes as JSON `null` with status 200.
    let h = boot().await;
    let path = format!("/api/prompts/versions/{VERSION_ID}");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body, serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_prompt_version_for_unknown_id_succeeds_idempotently() {
    let h = boot().await;
    let path = format!("/api/prompts/versions/{VERSION_ID}");
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(&path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "idempotent delete must preserve the empty 204 contract"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_prompt_version_requires_agent_id_in_body() {
    let h = boot().await;
    let path = format!("/api/prompts/versions/{VERSION_ID}/activate");
    let (status, body) = json_request(&h, Method::POST, &path, Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(body.get("error").is_some(), "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn activate_created_prompt_version_returns_entity() {
    let h = boot().await;
    let created = create_prompt_version(&h, AGENT_UUID, "Activate this prompt.").await;
    let version_id = created["id"].as_str().expect("created version id");
    let path = format!("/api/prompts/versions/{version_id}/activate");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"agent_id": AGENT_UUID})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["id"], version_id, "activated entity: {body:?}");
    assert_eq!(body["agent_id"], AGENT_UUID, "activated entity: {body:?}");
    assert_eq!(body["is_active"], true, "activated entity: {body:?}");
}

/// #6195: deleting the active (bound) prompt version must be refused at the
/// HTTP layer with 400, and the version must survive. Drives the full
/// create → activate → delete path through the real SQLite-backed store so
/// the guard (`PromptStore::delete_version` → `InvalidState`) is exercised
/// end to end, not just at the store layer.
#[tokio::test(flavor = "multi_thread")]
async fn delete_active_prompt_version_returns_400() {
    let h = boot().await;
    let create_path = format!("/api/agents/{AGENT_UUID}/prompts/versions");

    // Create two versions so one can be made active while another remains.
    let (status, first) = json_request(
        &h,
        Method::POST,
        &create_path,
        Some(serde_json::json!({"system_prompt": "First.", "description": "v1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create v1: {first:?}");
    let first_id = first["id"].as_str().expect("v1 id").to_string();

    let (status, second) = json_request(
        &h,
        Method::POST,
        &create_path,
        Some(serde_json::json!({"system_prompt": "Second.", "description": "v2"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create v2: {second:?}");

    // Activate the first version.
    let (status, _) = json_request(
        &h,
        Method::POST,
        &format!("/api/prompts/versions/{first_id}/activate"),
        Some(serde_json::json!({"agent_id": AGENT_UUID})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Deleting the active version must be refused with 400.
    let (status, body) = json_request(
        &h,
        Method::DELETE,
        &format!("/api/prompts/versions/{first_id}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "deleting the active version must be refused: {body:?}"
    );

    // The version still exists — the refused delete was a no-op.
    let (status, _) = json_request(
        &h,
        Method::GET,
        &format!("/api/prompts/versions/{first_id}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the active version must survive the refused delete"
    );
}

// ----- experiments -----

#[tokio::test(flavor = "multi_thread")]
async fn list_experiments_empty_for_unknown_agent() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/experiments");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert!(body.get("limit").is_some(), "limit field present: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_experiments_rejects_non_uuid_agent_id() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::GET,
        "/api/agents/not-a-uuid/prompts/experiments",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_experiment_with_unknown_agent_surfaces_store_error() {
    // The experiments table has FK constraints on the agent. Posting an
    // experiment for an agent_id that has no rows in the agents/prompt
    // store yields a 500 with the FK violation surfaced through the
    // structured error envelope. This pins the contract that the route
    // does NOT panic on store failure and that the bad_request path is
    // distinguishable (4xx) from the store-failure path (5xx).
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/experiments");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "name": "exp-1",
            "variants": [
                {"name": "control"},
                {"name": "treatment"},
            ]
        })),
    )
    .await;
    // Variants without a `prompt_version_id` default to the nil UUID; completing such an experiment would deactivate the agent's prompt and activate nothing, so the route rejects it before the store is touched.
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(body.get("error").is_some(), "error envelope: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_experiment_rejects_missing_or_foreign_variant_versions() {
    let h = boot().await;
    let agent_id = seeded_agent_id(&h);
    let own = create_prompt_version(&h, &agent_id, "Own prompt.").await;
    let foreign = create_prompt_version(&h, AGENT_UUID, "Foreign prompt.").await;
    let path = format!("/api/agents/{agent_id}/prompts/experiments");
    for (label, variants) in [
        ("no variants", serde_json::json!([])),
        (
            "nil version",
            serde_json::json!([{"name": "control", "prompt_version_id": own["id"]}, {"name": "treatment"}]),
        ),
        (
            "unknown version",
            serde_json::json!([{"name": "control", "prompt_version_id": VERSION_ID}]),
        ),
        (
            "another agent's version",
            serde_json::json!([{"name": "control", "prompt_version_id": own["id"]}, {"name": "treatment", "prompt_version_id": foreign["id"]}]),
        ),
    ] {
        let (status, body) = json_request(
            &h,
            Method::POST,
            &path,
            Some(serde_json::json!({"name": "exp", "variants": variants})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "[{label}] body={body:?}");
    }
    let (_, listed) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(
        listed["total"], 0,
        "no experiment may be persisted: {listed:?}"
    );
}

/// Activating a version under another agent's id used to clear that agent's active version and leave the owning agent with two active rows; an unknown version id cleared the agent's active version and still answered `{"success": true}`.
#[tokio::test(flavor = "multi_thread")]
async fn activate_rejects_foreign_or_unknown_version_without_touching_active_state() {
    let h = boot().await;
    let agent_a = seeded_agent_id(&h);
    let a_version = create_prompt_version(&h, &agent_a, "Agent A prompt.").await;
    let a_version_id = a_version["id"].as_str().unwrap().to_string();
    let b_version = create_prompt_version(&h, AGENT_UUID, "Agent B prompt.").await;
    let b_version_id = b_version["id"].as_str().unwrap().to_string();
    let (status, _) = json_request(
        &h,
        Method::POST,
        &format!("/api/prompts/versions/{a_version_id}/activate"),
        Some(serde_json::json!({"agent_id": agent_a})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // B's version under A's id.
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/prompts/versions/{b_version_id}/activate"),
        Some(serde_json::json!({"agent_id": agent_a})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");

    // A version id that does not exist.
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/prompts/versions/{VERSION_ID}/activate"),
        Some(serde_json::json!({"agent_id": agent_a})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");

    // A keeps its one active version; B gained none.
    let (_, a_list) = json_request(
        &h,
        Method::GET,
        &format!("/api/agents/{agent_a}/prompts/versions"),
        None,
    )
    .await;
    let active: Vec<_> = a_list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["is_active"] == true)
        .map(|v| v["id"].clone())
        .collect();
    assert_eq!(active, vec![serde_json::json!(a_version_id)], "{a_list:?}");
    let (_, b_list) = json_request(
        &h,
        Method::GET,
        &format!("/api/agents/{AGENT_UUID}/prompts/versions"),
        None,
    )
    .await;
    assert!(
        b_list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["is_active"] == false),
        "{b_list:?}"
    );
}

// ----- owner-scoping (#6753) -----

async fn request_as(
    h: &Harness,
    path: &str,
    user: librefang_api::middleware::AuthenticatedApiUser,
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(user);
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn user(
    name: &str,
    role: librefang_kernel::auth::UserRole,
) -> librefang_api::middleware::AuthenticatedApiUser {
    librefang_api::middleware::AuthenticatedApiUser {
        name: name.to_string(),
        role,
        user_id: librefang_types::agent::UserId::from_name(name),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_reads_are_scoped_to_the_agent_author_for_non_admins() {
    use librefang_kernel::auth::UserRole;
    let h = boot().await;
    let spawn = |name: &str, author: &str| {
        h.state
            .kernel
            .spawn_agent_typed(librefang_types::agent::AgentManifest {
                name: name.to_string(),
                author: author.to_string(),
                model: librefang_types::agent::ModelConfig {
                    system_prompt: format!("SECRET-{name}"),
                    ..Default::default()
                },
                ..Default::default()
            })
            .expect("spawn")
            .to_string()
    };
    let alice_agent = spawn("prompt-alice", "alice");
    let bob_agent = spawn("prompt-bob", "bob");
    let alice_version = create_prompt_version(&h, &alice_agent, "Alice v1.").await;
    let alice_version_id = alice_version["id"].as_str().unwrap().to_string();
    let bob_version = create_prompt_version(&h, &bob_agent, "Bob v1.").await;
    let (status, alice_exp) = json_request(
        &h,
        Method::POST,
        &format!("/api/agents/{alice_agent}/prompts/experiments"),
        Some(serde_json::json!({"name": "e", "variants": [{"name": "c", "prompt_version_id": alice_version_id}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{alice_exp:?}");
    let alice_exp_id = alice_exp["id"].as_str().unwrap().to_string();

    let bob = || user("bob", UserRole::User);

    let (status, body) = request_as(&h, "/api/prompts/overview", bob()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.to_string().contains("SECRET-prompt-alice"),
        "{body:?}"
    );
    assert!(body.to_string().contains("SECRET-prompt-bob"), "{body:?}");

    for path in [
        format!("/api/agents/{alice_agent}/prompts/versions"),
        format!("/api/agents/{alice_agent}/prompts/experiments"),
    ] {
        let (status, body) = request_as(&h, &path, bob()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body:?}");
    }
    let (status, body) = request_as(
        &h,
        &format!("/api/agents/{bob_agent}/prompts/versions"),
        bob(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"][0]["id"], bob_version["id"], "{body:?}");

    let (status, body) = request_as(
        &h,
        &format!("/api/prompts/versions/{alice_version_id}"),
        bob(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::Value::Null,
        "foreign version must look missing"
    );
    let (_, body) = request_as(
        &h,
        &format!("/api/prompts/experiments/{alice_exp_id}"),
        bob(),
    )
    .await;
    assert_eq!(
        body,
        serde_json::Value::Null,
        "foreign experiment must look missing"
    );
    let (status, body) = request_as(
        &h,
        &format!("/api/prompts/experiments/{alice_exp_id}/metrics"),
        bob(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        serde_json::json!([]),
        "foreign metrics must look empty"
    );

    // Admin is unrestricted.
    let (_, body) = request_as(&h, "/api/prompts/overview", user("ops", UserRole::Admin)).await;
    assert!(body.to_string().contains("SECRET-prompt-alice"), "{body:?}");
    let (_, body) = request_as(
        &h,
        &format!("/api/prompts/versions/{alice_version_id}"),
        user("ops", UserRole::Admin),
    )
    .await;
    assert_eq!(body["id"], alice_version_id);
    // Creation seeds one zeroed metrics row per variant, so the empty list bob got above is the scoping, not an absence of data.
    let (_, body) = request_as(
        &h,
        &format!("/api/prompts/experiments/{alice_exp_id}/metrics"),
        user("ops", UserRole::Admin),
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1), "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_experiment_rejects_non_uuid_agent_id() {
    let h = boot().await;
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/agents/not-a-uuid/prompts/experiments",
        Some(serde_json::json!({"name": "exp-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_experiment_returns_null_for_unknown_id() {
    let h = boot().await;
    let path = format!("/api/prompts/experiments/{EXPERIMENT_ID}");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body, serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn start_pause_complete_status_transitions_succeed() {
    // The status-transition endpoints all dispatch through a single
    // `update_experiment_status` call on the kernel. Against the real
    // prompt store wired into TestAppState's kernel, an unknown id is
    // accepted as a no-op success — we assert the route plumbing only
    // (status 200 + `success: true` JSON body), not store semantics.
    let h = boot().await;
    for verb in ["start", "pause", "complete"] {
        let path = format!("/api/prompts/experiments/{EXPERIMENT_ID}/{verb}");
        let (status, body) = json_request(&h, Method::POST, &path, None).await;
        assert_eq!(status, StatusCode::OK, "{verb}: body={body:?}");
        assert_eq!(body["success"], true, "{verb}: body={body:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_experiment_metrics_empty_for_unknown_id() {
    let h = boot().await;
    let path = format!("/api/prompts/experiments/{EXPERIMENT_ID}/metrics");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body, serde_json::json!([]));
}

/// A malformed version / experiment id in the path is a client error: it must reach the caller as 400 with the parse message, not a scrubbed 500.
#[tokio::test(flavor = "multi_thread")]
async fn prompt_routes_reject_non_uuid_path_ids_with_400() {
    let h = boot().await;
    let agent_id = seeded_agent_id(&h);
    let cases = [
        (Method::GET, "/api/prompts/versions/not-a-uuid", None),
        (Method::DELETE, "/api/prompts/versions/not-a-uuid", None),
        (
            Method::POST,
            "/api/prompts/versions/not-a-uuid/activate",
            Some(serde_json::json!({ "agent_id": agent_id })),
        ),
        (Method::GET, "/api/prompts/experiments/not-a-uuid", None),
        (
            Method::GET,
            "/api/prompts/experiments/not-a-uuid/metrics",
            None,
        ),
        (
            Method::POST,
            "/api/prompts/experiments/not-a-uuid/start",
            None,
        ),
        (
            Method::POST,
            "/api/prompts/experiments/not-a-uuid/pause",
            None,
        ),
        (
            Method::POST,
            "/api/prompts/experiments/not-a-uuid/complete",
            None,
        ),
    ];
    for (method, path, body) in cases {
        let (status, resp) = json_request(&h, method.clone(), path, body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{method} {path}: body={resp:?}"
        );
        assert!(
            resp.to_string().contains("Invalid"),
            "{method} {path}: expected the parse message, got {resp:?}"
        );
    }
}

// ----- create-handler input-validation guards -----
//
// Audit: `docs/issues/prompt-version-system-prompt-no-cap.md`. The create
// handler is the only path that mints a `PromptVersion`; once active, its
// `system_prompt` rides every LLM call. These tests pin the three guards
// the audit prescribes:
//   1. byte / character caps on `system_prompt`,
//   2. client-supplied `is_active` is ignored (only `/activate` flips it),
//   3. client-supplied `version` is ignored (server monotonic numbering).

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_rejects_oversize_system_prompt_bytes() {
    // 33 KiB of ASCII = 33 KiB bytes; cap is 32 KiB. Must reject before
    // any store write so the token-cost-amplification vector is closed at
    // the route boundary.
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let oversize = "a".repeat(33 * 1024);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({ "system_prompt": oversize })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    // `ValidationError` serialises both top-level and nested-`error.message`.
    let msg = body["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("system_prompt") && msg.contains("byte"),
        "expected byte-cap message, got {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_rejects_oversize_system_prompt_chars() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    // 16_385 ASCII chars exceed the 16 KiB character cap by one while
    // remaining below the independent 32 KiB byte cap.
    let oversize = "a".repeat(16 * 1024 + 1);
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({ "system_prompt": oversize })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    let msg = body["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("system_prompt") && msg.contains("character"),
        "expected character-cap message, got {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_ignores_client_is_active() {
    // The client requests `is_active: true`, attempting to side-channel
    // activation around the dedicated `/activate` endpoint. The server
    // MUST return a record with `is_active = false` regardless.
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "system_prompt": "Hello, world.",
            "is_active": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    assert_eq!(
        body["is_active"],
        serde_json::json!(false),
        "create handler must ignore client is_active=true; body={body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_prompt_version_ignores_client_version_and_numbers_monotonically() {
    // The client tries to inject `version = 999`. The server MUST
    // overwrite with its monotonic count — first version for an agent
    // is 1, the second is 2, regardless of the request payload.
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "system_prompt": "first",
            "version": 999,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    assert_eq!(
        body["version"],
        serde_json::json!(1),
        "first version must be 1 regardless of client-supplied 999; body={body:?}"
    );

    // Second create on the same agent must produce version 2.
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "system_prompt": "second",
            "version": 42,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    assert_eq!(
        body["version"],
        serde_json::json!(2),
        "second version must be 2 (monotonic); body={body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_experiment_ignores_client_status_and_timestamps() {
    // Same defensive pattern for experiments: the state machine —
    // `status`, `started_at`, `ended_at` — is server-owned and can only
    // advance through /start, /pause, /complete. A client cannot post
    // an already-Running experiment with a backdated `started_at`.
    let h = boot().await;
    let agent_id = seeded_agent_id(&h);
    let control = create_prompt_version(&h, &agent_id, "Control prompt.").await;
    let treatment = create_prompt_version(&h, &agent_id, "Treatment prompt.").await;
    let path = format!("/api/agents/{agent_id}/prompts/experiments");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({
            "name": "exp-state-machine-bypass",
            "status": "running",
            "started_at": "2020-01-01T00:00:00Z",
            "ended_at": "2020-01-02T00:00:00Z",
            "variants": [
                {"name": "control", "prompt_version_id": control["id"]},
                {"name": "treatment", "prompt_version_id": treatment["id"]},
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body:?}");
    assert_eq!(body["agent_id"], agent_id, "body={body:?}");
    assert_eq!(body["status"], "draft", "body={body:?}");
    assert_eq!(body["started_at"], serde_json::Value::Null, "body={body:?}");
    assert_eq!(body["ended_at"], serde_json::Value::Null, "body={body:?}");

    let experiment_id = body["id"].as_str().expect("created experiment id");
    let (status, stored) = json_request(
        &h,
        Method::GET,
        &format!("/api/prompts/experiments/{experiment_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "stored={stored:?}");
    assert_eq!(stored["status"], "draft", "stored={stored:?}");
    assert_eq!(
        stored["started_at"],
        serde_json::Value::Null,
        "stored={stored:?}"
    );
    assert_eq!(
        stored["ended_at"],
        serde_json::Value::Null,
        "stored={stored:?}"
    );
}
