//! Integration tests for the `/api/goals/*` route family (issue #3571).
//!
//! These exercise `routes::goals::router()` against a fresh `MockKernel`
//! (real SQLite-backed memory substrate, temp dir) via `tower::oneshot`.
//! No global env mutation, no fs writes outside the test's tempdir, so the
//! tests are safe to run in parallel.
//!
//! The goals router is unauthenticated and stateless beyond the kernel's
//! shared-memory KV (`__librefang_goals` under a fixed sentinel agent id),
//! so we mount only the goals sub-router — same pattern as `users_test.rs`.

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
        .nest("/api", routes::goals::router())
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

async fn create_goal(h: &Harness, payload: serde_json::Value) -> serde_json::Value {
    let (status, body) = json_request(h, Method::POST, "/api/goals", Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED, "create goal failed: {body:?}");
    body
}

// ---------------------------------------------------------------------------
// GET /api/goals
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_list_starts_empty() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/goals", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["offset"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_list_reflects_created_goals() {
    let h = boot().await;
    create_goal(&h, serde_json::json!({"title": "Ship v1"})).await;
    create_goal(&h, serde_json::json!({"title": "Ship v2"})).await;

    let (status, body) = json_request(&h, Method::GET, "/api/goals", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// POST /api/goals
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_happy_path_returns_201_with_id_and_defaults() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({
            "title": "Write docs",
            "description": "The README is sparse.",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got: {body:?}");
    assert_eq!(body["title"], "Write docs");
    assert_eq!(body["description"], "The README is sparse.");
    assert_eq!(body["status"], "pending");
    assert_eq!(body["progress"], 0);
    assert!(body["id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(body["created_at"].as_str().is_some());
    assert!(body["updated_at"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_missing_title() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::POST, "/api/goals", Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_empty_title() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_title_over_256_chars() {
    let h = boot().await;
    let title: String = "a".repeat(257);
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": title})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_description_over_4096_chars() {
    let h = boot().await;
    let desc: String = "x".repeat(4097);
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": "ok", "description": desc})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_invalid_status() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": "ok", "status": "bogus"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_rejects_progress_over_100() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": "ok", "progress": 101})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_create_with_unknown_parent_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/goals",
        Some(serde_json::json!({"title": "child", "parent_id": "no-such-parent"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// GET /api/goals/{id} + GET /api/goals/{id}/children
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_get_unknown_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(&h, Method::GET, "/api/goals/does-not-exist", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_get_returns_created_goal() {
    let h = boot().await;
    let created = create_goal(&h, serde_json::json!({"title": "find it"})).await;
    let id = created["id"].as_str().unwrap();

    let (status, body) = json_request(&h, Method::GET, &format!("/api/goals/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "find it");
    assert_eq!(body["id"], id);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_children_lists_only_direct_children() {
    let h = boot().await;
    let parent = create_goal(&h, serde_json::json!({"title": "parent"})).await;
    let pid = parent["id"].as_str().unwrap().to_string();

    create_goal(
        &h,
        serde_json::json!({"title": "child-a", "parent_id": pid}),
    )
    .await;
    let child_b = create_goal(
        &h,
        serde_json::json!({"title": "child-b", "parent_id": pid}),
    )
    .await;
    // Grandchild — must NOT appear in /children of root.
    create_goal(
        &h,
        serde_json::json!({"title": "grandchild", "parent_id": child_b["id"]}),
    )
    .await;

    let (status, body) =
        json_request(&h, Method::GET, &format!("/api/goals/{pid}/children"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2, "got: {body:?}");
    assert_eq!(body["children"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_children_unknown_parent_returns_empty_list() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::GET, "/api/goals/no-such-id/children", None).await;
    // Endpoint returns 200 with empty list rather than 404 — encode that.
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
}

// ---------------------------------------------------------------------------
// PUT /api/goals/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_changes_status_and_progress() {
    let h = boot().await;
    let created = create_goal(&h, serde_json::json!({"title": "updateable"})).await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/goals/{id}"),
        Some(serde_json::json!({"status": "in_progress", "progress": 42})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = json_request(&h, Method::GET, &format!("/api/goals/{id}"), None).await;
    assert_eq!(body["status"], "in_progress");
    assert_eq!(body["progress"], 42);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_unknown_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/goals/ghost",
        Some(serde_json::json!({"status": "completed"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_rejects_self_parent() {
    let h = boot().await;
    let g = create_goal(&h, serde_json::json!({"title": "self"})).await;
    let id = g["id"].as_str().unwrap().to_string();

    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/goals/{id}"),
        Some(serde_json::json!({"parent_id": id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_rejects_unknown_parent() {
    let h = boot().await;
    let g = create_goal(&h, serde_json::json!({"title": "x"})).await;
    let id = g["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/goals/{id}"),
        Some(serde_json::json!({"parent_id": "nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_rejects_circular_parent_chain() {
    let h = boot().await;
    // Build chain: a <- b <- c   (c's parent is b, b's parent is a)
    let a = create_goal(&h, serde_json::json!({"title": "a"})).await;
    let aid = a["id"].as_str().unwrap().to_string();
    let b = create_goal(&h, serde_json::json!({"title": "b", "parent_id": aid})).await;
    let bid = b["id"].as_str().unwrap().to_string();
    let c = create_goal(&h, serde_json::json!({"title": "c", "parent_id": bid})).await;
    let cid = c["id"].as_str().unwrap();

    // Try to make `a`'s parent be `c` — closes the loop.
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/goals/{aid}"),
        Some(serde_json::json!({"parent_id": cid})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_update_rejects_invalid_progress() {
    let h = boot().await;
    let g = create_goal(&h, serde_json::json!({"title": "x"})).await;
    let id = g["id"].as_str().unwrap().to_string();
    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/goals/{id}"),
        Some(serde_json::json!({"progress": 9999})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// DELETE /api/goals/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_delete_removes_goal_and_descendants() {
    let h = boot().await;
    let parent = create_goal(&h, serde_json::json!({"title": "parent"})).await;
    let pid = parent["id"].as_str().unwrap().to_string();
    let child = create_goal(&h, serde_json::json!({"title": "child", "parent_id": pid})).await;
    let cid = child["id"].as_str().unwrap().to_string();
    let grand = create_goal(&h, serde_json::json!({"title": "grand", "parent_id": cid})).await;
    let gid = grand["id"].as_str().unwrap().to_string();

    // Sibling (different subtree) — must survive the delete.
    let other = create_goal(&h, serde_json::json!({"title": "unrelated"})).await;
    let oid = other["id"].as_str().unwrap().to_string();

    // Issue #3832: DELETE returns 204 No Content per RFC 9110 §15.3.5 — the
    // response MUST have an empty body. Hit the router directly so we can
    // assert on the raw byte length.
    let raw_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/goals/{pid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_resp.status(), StatusCode::NO_CONTENT);
    let raw_bytes = axum::body::to_bytes(raw_resp.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(
        raw_bytes.is_empty(),
        "204 response must have empty body (got {} bytes: {:?})",
        raw_bytes.len(),
        String::from_utf8_lossy(&raw_bytes)
    );

    for missing in [&pid, &cid, &gid] {
        let (s, _) = json_request(&h, Method::GET, &format!("/api/goals/{missing}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "id {missing} should be gone");
    }
    let (s, _) = json_request(&h, Method::GET, &format!("/api/goals/{oid}"), None).await;
    assert_eq!(s, StatusCode::OK, "unrelated subtree must survive");
}

#[tokio::test(flavor = "multi_thread")]
async fn goals_delete_unknown_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(&h, Method::DELETE, "/api/goals/missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// GET /api/goals/templates
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn goals_templates_returns_built_in_catalog() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/goals/templates", None).await;
    assert_eq!(status, StatusCode::OK);
    let templates = body["templates"]
        .as_array()
        .expect("templates should be an array");
    assert!(
        !templates.is_empty(),
        "built-in template catalog must not be empty"
    );
    // Spot-check shape of the first template.
    let first = &templates[0];
    assert!(first["id"].as_str().is_some());
    assert!(first["name"].as_str().is_some());
    assert!(first["goals"].as_array().is_some());
}

// ---------------------------------------------------------------------------
// #5138 — `__librefang_goals` RMW race: concurrent writers must not lose
// each other's goals.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_goal_creates_lose_no_writes_5138() {
    // Before the fix, `create_goal` did `structured_get -> push ->
    // structured_set` with no transaction: N concurrent POSTs each loaded
    // the same array, each appended one goal, and the last writer's blob
    // clobbered every other writer's append. The substrate-level
    // `structured_modify` (BEGIN IMMEDIATE) serializes the RMW so every
    // POST that returns 201 is present in the final list.
    let h = boot().await;
    let app = h.app.clone();

    let n = 16usize;
    let mut handles = Vec::new();
    for i in 0..n {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let body = serde_json::to_vec(&serde_json::json!({
                "title": format!("goal-{i}")
            }))
            .unwrap();
            let req = Request::builder()
                .method(Method::POST)
                .uri("/api/goals")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            resp.status()
        }));
    }

    let mut created = 0usize;
    for hd in handles {
        let status = hd.await.unwrap();
        assert_eq!(status, StatusCode::CREATED, "each POST must succeed");
        created += 1;
    }
    assert_eq!(created, n);

    // Every accepted create must be readable back — no lost update.
    let (status, body) = json_request(&h, Method::GET, "/api/goals", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        n,
        "all {n} concurrently-created goals must persist; lost-update race not fixed"
    );
    let mut titles: Vec<String> = items
        .iter()
        .map(|g| g["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    let mut expected: Vec<String> = (0..n).map(|i| format!("goal-{i}")).collect();
    expected.sort();
    assert_eq!(titles, expected, "no individual goal may be clobbered");
}
