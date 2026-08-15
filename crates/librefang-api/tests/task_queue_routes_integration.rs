//! Integration coverage for the task-queue HTTP contract.
//!
//! The focused router uses the real SQLite-backed task substrate from
//! `TestAppState`. Authentication middleware is not mounted, so tests that
//! need a principal inject `AuthenticatedApiUser` as a request extension.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::AuthenticatedApiUser;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::task_queue::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

fn api_user(name: &str) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role: UserRole::User,
        user_id: UserId::from_name(name),
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
    user: Option<AuthenticatedApiUser>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let bytes = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&value).unwrap()
        }
        None => Vec::new(),
    };
    let mut request = builder.body(Body::from(bytes)).unwrap();
    if let Some(user) = user {
        request.extensions_mut().insert(user);
    }

    let response = h.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

async fn post_task(h: &Harness, title: &str) -> String {
    let (status, body) = json_request(
        h,
        Method::POST,
        "/api/tasks",
        Some(serde_json::json!({
            "title": title,
            "description": "integration test"
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "task post failed: {body:?}");
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn list_total_describes_filtered_result_before_limit() {
    let h = boot();
    post_task(&h, "one").await;
    post_task(&h, "two").await;
    post_task(&h, "three").await;

    let (status, body) = json_request(&h, Method::GET, "/api/tasks?limit=1", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_uses_authenticated_principal_for_provenance() {
    let h = boot();
    let caller = api_user("alice");
    let expected_creator = caller.user_id.to_string();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/tasks",
        Some(serde_json::json!({
            "title": "owned",
            "description": "integration test",
            "created_by": "forged-agent"
        })),
        Some(caller),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let id = body["id"].as_str().unwrap();
    let (status, task) =
        json_request(&h, Method::GET, &format!("/api/tasks/{id}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(task["created_by"], expected_creator);
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_distinguishes_missing_from_ineligible_task() {
    let h = boot();
    let id = post_task(&h, "pending").await;

    let (status, _) = json_request(
        &h,
        Method::POST,
        &format!("/api/tasks/{id}/retry"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = json_request(&h, Method::POST, "/api/tasks/missing/retry", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_only_requeues_failed_tasks() {
    let h = boot();
    let id = post_task(&h, "stateful").await;
    let patch = serde_json::json!({"status": "pending"});

    let (status, _) = json_request(
        &h,
        Method::PATCH,
        &format!("/api/tasks/{id}"),
        Some(patch.clone()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let pool = h.state.kernel.memory_substrate().pool();
    let conn = pool.get().unwrap();
    let changed = conn
        .execute(
            "UPDATE task_queue SET status = 'failed' WHERE id = ?1",
            rusqlite::params![&id],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(conn);

    let (status, body) = json_request(
        &h,
        Method::PATCH,
        &format!("/api/tasks/{id}"),
        Some(patch),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "pending");
}
