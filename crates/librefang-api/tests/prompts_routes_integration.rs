//! Integration tests for the prompts router (#3571 — prompts slice).
//!
//! Mounts `routes::prompts::routes()` directly under `/api` against a
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
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::prompts::routes())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
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

// ----- prompt versions -----

#[tokio::test(flavor = "multi_thread")]
async fn list_prompt_versions_empty_for_unknown_agent() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/versions");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body, serde_json::json!([]));
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
    assert_eq!(status, StatusCode::OK, "body={body:?}");
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
    let arr = listed.as_array().expect("list is array");
    assert!(
        arr.iter().any(|v| v["id"] == new_id),
        "expected new version in list: {listed:?}"
    );

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
    // Either 204 (deleted) or some store-specific success — the contract
    // is "not 5xx" for an unknown id.
    assert!(
        !resp.status().is_server_error(),
        "delete returned {}",
        resp.status()
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
async fn activate_prompt_version_with_agent_id_in_body_succeeds() {
    let h = boot().await;
    let path = format!("/api/prompts/versions/{VERSION_ID}/activate");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({"agent_id": AGENT_UUID})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["success"], true);
}

// ----- experiments -----

#[tokio::test(flavor = "multi_thread")]
async fn list_experiments_empty_for_unknown_agent() {
    let h = boot().await;
    let path = format!("/api/agents/{AGENT_UUID}/prompts/experiments");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body, serde_json::json!([]));
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
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body:?}");
    assert!(body.get("error").is_some(), "error envelope: {body:?}");
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
