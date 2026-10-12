//! Integration tests for `routes::bindings::router()` (#7157).
//!
//! The DELETE contract is the point: it must answer a bodyless 204, not a 200 or a serialized
//! JSON `null`, because a generated client decoding a no-content response trips over either.
//! A unit test on the response helper cannot see that — only a request through the mounted
//! router proves the status, the empty body, and the absence of a `content-type` a client
//! would try to decode.
//!
//! Endpoints covered:
//!   - `POST   /api/bindings`          — seed a binding
//!   - `GET    /api/bindings`          — read back
//!   - `DELETE /api/bindings/{index}`  — 204 with no body, and the side effect
//!   - `DELETE /api/bindings/{index}`  — 404 for an out-of-range index
//!   - `DELETE /api/bindings/{index}`  — 400 for a non-`u32` index
//!   - `POST` / `DELETE` reach the running channel bridge's router, not just the kernel list

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state: Arc<AppState> = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::bindings::router())
        .with_state(state);
    Harness { app, _test: test }
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Response {
    let builder = Request::builder().method(method).uri(path);
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&value).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = h.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|value| value.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    Response {
        status,
        content_type,
        body: bytes.to_vec(),
    }
}

struct Response {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Response {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

fn binding(agent: &str) -> serde_json::Value {
    serde_json::json!({ "agent": agent, "match_rule": { "channel": "discord" } })
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_binding_returns_bodyless_204_and_removes_it() {
    let h = boot().await;

    let created = send(&h, Method::POST, "/api/bindings", Some(binding("alpha"))).await;
    assert_eq!(created.status, StatusCode::CREATED);

    let listed = send(&h, Method::GET, "/api/bindings", None).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        listed.json()["bindings"].as_array().map(Vec::len),
        Some(1),
        "seeded binding must be listed before the delete"
    );

    let deleted = send(&h, Method::DELETE, "/api/bindings/0", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(
        deleted.body.is_empty(),
        "204 must carry no body, got {:?}",
        String::from_utf8_lossy(&deleted.body)
    );
    assert_eq!(
        deleted.content_type, None,
        "a no-content response must not advertise a body a client would decode"
    );

    // Read back: the delete actually took effect, not just answered well.
    let after = send(&h, Method::GET, "/api/bindings", None).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.json()["bindings"].as_array().map(Vec::len), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_binding_out_of_range_returns_404_with_an_error_body() {
    let h = boot().await;

    let response = send(&h, Method::DELETE, "/api/bindings/7", None).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(
        response.json()["error"].is_string(),
        "404 must carry a translated error message, got {}",
        response.json()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_binding_rejects_an_index_outside_u32() {
    let h = boot().await;

    // The OpenAPI document declares `index` as `u32`; the handler now takes the same type, so a
    // value that does not fit is a request error rather than a silent wrap or a 404.
    let response = send(&h, Method::DELETE, "/api/bindings/4294967296", None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
}

/// The channel bridge copies the binding list into its router once, at start.
/// A binding added or removed through the API must reach that live router too, or it does not route a message until the next restart.
#[tokio::test(flavor = "multi_thread")]
async fn binding_mutations_reach_the_running_channel_router() {
    let h = boot().await;
    let state = h._test.state.clone();

    let router = Arc::new(librefang_channels::router::AgentRouter::new());
    let bridge = librefang_channels::bridge::BridgeManager::new(
        Arc::new(librefang_api::channel_bridge::KernelBridgeAdapter::new(
            state.kernel.clone(),
        )),
        router.clone(),
    );
    state.bridge_manager.store(Arc::new(Some(bridge)));

    // Spawned after the bridge started, so the router's name cache has never seen it.
    let agent_id = state
        .kernel
        .spawn_agent_typed(librefang_types::agent::AgentManifest {
            name: "late-agent".to_string(),
            ..Default::default()
        })
        .expect("spawn agent");

    let created = send(
        &h,
        Method::POST,
        "/api/bindings",
        Some(serde_json::json!({
            "agent": "late-agent",
            "match_rule": { "channel": "discord", "peer_id": "room-1" }
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(router.bindings().len(), 1, "the new binding must be live");
    assert_eq!(
        router.bound_recipients_for_agent(agent_id, "discord", None, None),
        vec!["room-1".to_string()],
        "the binding must resolve to the agent it names"
    );

    let deleted = send(&h, Method::DELETE, "/api/bindings/0", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(router.bindings().is_empty(), "the removal must be live");
}

/// With no channel bridge running there is no router to update; the mutation still succeeds against the kernel list.
#[tokio::test(flavor = "multi_thread")]
async fn binding_mutations_without_a_running_bridge_still_succeed() {
    let h = boot().await;
    assert!(h._test.state.bridge_manager.load().is_none());

    let created = send(&h, Method::POST, "/api/bindings", Some(binding("alpha"))).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let deleted = send(&h, Method::DELETE, "/api/bindings/0", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
}
