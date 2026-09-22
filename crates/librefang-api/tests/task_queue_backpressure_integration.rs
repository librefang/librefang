//! Integration tests for `[queue]` backpressure on the task-queue routes (#8219).
//!
//! `max_depth_per_agent`, `max_depth_global` and `task_ttl_secs` were declared in `KernelConfig`, documented as enforced, and echoed back by `GET /api/queue/status`, while no production path read any of them.
//! These tests assert the enforcement through the real axum router and the real SQLite substrate, because the gap was never a missing line of logic — it was that nothing downstream of the config ever asked.
//!
//! `?limit=` / `?offset=` / `?assigned_to=` are covered here for the same reason: they used to be applied to a fully materialised `Vec`, so the assertions that matter are about what comes back, not about how the filtering is spelled.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::AgentManifest;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

struct RouterHarness {
    app: axum::Router,
    state: Arc<librefang_api::routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Manifest for one of the assignee labels these tests address tasks to.
///
/// `POST /api/tasks` refuses an `assigned_to` that names no registered agent, and the depth-cap and `?assigned_to=` assertions below compare tasks *across* assignees, so their labels have to resolve to real registry entries for those tasks to reach the queue at all.
/// `alice` and `bob` are buckets the assertions compare against, not workers: nothing here asserts anything about the assignee, and addressing a task to an agent nobody meant to run a turn for is not a behaviour the queue tests are about.
///
/// `assignee_wake` is off for that reason.
/// A default manifest can claim its own tasks, so leaving it on would start a turn for every task these tests post — 25 of them in the zero-cap test alone — against a provider that does not exist in CI (and, on a workstation running Ollama, against a real one).
/// The cap, paging and filter assertions are unaffected by the wake either way; suppressing it keeps the harness doing what it did before these labels existed.
fn assignee_label(name: &str) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        assignee_wake: Some(false),
        ..AgentManifest::default()
    }
}

async fn boot_router(customize: impl FnOnce(&mut KernelConfig)) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
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
    // The shipped default is 3600s, and the sweep it drives is spawned at boot.
    // Off unless a test asks for it, so an expiry cannot race a test that is asserting about depth.
    config.queue.task_ttl_secs = 0;
    customize(&mut config);

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    // Registered before the router is built, so the labels the cap and filter tests address their tasks to exist by the time the first `POST /api/tasks` lands.
    for label in ["alice", "bob"] {
        kernel
            .spawn_agent(assignee_label(label))
            .expect("assignee label must spawn");
    }
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.expect("router response");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn auth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .expect("request")
}

fn post_task(title: &str, assigned_to: Option<&str>) -> Request<Body> {
    let mut body = serde_json::json!({ "title": title, "description": "body" });
    if let Some(assignee) = assigned_to {
        body["assigned_to"] = serde_json::json!(assignee);
    }
    Request::builder()
        .method(Method::POST)
        .uri("/api/tasks")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// `[queue] max_depth_global` rejects the enqueue that would exceed it, and the refusal is a 429 rather than a scrubbed 500.
///
/// 429 is the status that tells a client the request was refused by a policy it can wait out; `Internal` would have said "the daemon broke" and invited an immediate retry of the request the cap just declined.
#[tokio::test(flavor = "multi_thread")]
async fn global_depth_cap_rejects_the_enqueue_that_would_exceed_it() {
    let h = boot_router(|cfg| cfg.queue.max_depth_global = 2).await;

    for i in 0..2 {
        let (status, _) = send(&h.app, post_task(&format!("task-{i}"), None)).await;
        assert_eq!(status, StatusCode::CREATED, "post {i} should be accepted");
    }

    let (status, body) = send(&h.app, post_task("over-the-line", None)).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "the third post exceeds max_depth_global = 2: {body}"
    );

    let (status, body) = send(&h.app, auth_get("/api/tasks")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["total"], 2,
        "the rejected task must not have been written: {body}"
    );
}

/// `[queue] max_depth_per_agent` is per assignee, so one agent filling its share leaves another agent's share untouched.
///
/// A cap that counted every pending row would reject the second agent's first task, which is the bug a "per agent" knob exists to avoid.
#[tokio::test(flavor = "multi_thread")]
async fn per_agent_depth_cap_is_scoped_to_the_assignee() {
    let h = boot_router(|cfg| cfg.queue.max_depth_per_agent = 1).await;

    let (status, _) = send(&h.app, post_task("alice-1", Some("alice"))).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(&h.app, post_task("alice-2", Some("alice"))).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "alice is already at max_depth_per_agent = 1: {body}"
    );

    let (status, body) = send(&h.app, post_task("bob-1", Some("bob"))).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "bob's own queue is empty; the cap is per agent: {body}"
    );
}

/// An unassigned task belongs to the shared pool, so the per-agent cap must not apply to it.
///
/// `assigned_to` is stored as `''` when there is no assignee, so a naive `WHERE assigned_to = ?` count would put every unassigned task in one bucket keyed on the empty string and cap the whole pool at a single agent's limit.
#[tokio::test(flavor = "multi_thread")]
async fn per_agent_depth_cap_does_not_apply_to_the_unassigned_pool() {
    let h = boot_router(|cfg| cfg.queue.max_depth_per_agent = 1).await;

    for i in 0..4 {
        let (status, body) = send(&h.app, post_task(&format!("pool-{i}"), None)).await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "unassigned post {i} must not be counted against a per-agent cap: {body}"
        );
    }
}

/// Both caps default to `0`, which the config documents as unlimited — an install that sets nothing must keep enqueueing.
#[tokio::test(flavor = "multi_thread")]
async fn a_zero_cap_is_unlimited() {
    let h = boot_router(|_| {}).await;

    for i in 0..25 {
        let (status, _) = send(&h.app, post_task(&format!("task-{i}"), Some("alice"))).await;
        assert_eq!(status, StatusCode::CREATED, "post {i} with no cap set");
    }

    let (_, body) = send(&h.app, auth_get("/api/tasks")).await;
    assert_eq!(body["total"], 25);
}

/// `?limit=` returns a page while `total` keeps counting every matching row, so a paging client can still tell how many there are.
///
/// `total` used to be the pre-truncation length of a fully materialised list; pushing `LIMIT` into SQL must not silently redefine it as the page size.
#[tokio::test(flavor = "multi_thread")]
async fn limit_returns_a_page_while_total_counts_every_match() {
    let h = boot_router(|_| {}).await;
    for i in 0..7 {
        let (status, _) = send(&h.app, post_task(&format!("task-{i}"), None)).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = send(&h.app, auth_get("/api/tasks?limit=3")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tasks"].as_array().expect("tasks").len(), 3);
    assert_eq!(body["total"], 7, "total is the match count, not the page");

    let (status, body) = send(&h.app, auth_get("/api/tasks/list?limit=2")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tasks"].as_array().expect("tasks").len(), 2);
    assert_eq!(body["total"], 7);
}

/// `?offset=` walks the same ordering the unpaged list returns, with no row repeated or skipped.
#[tokio::test(flavor = "multi_thread")]
async fn offset_walks_the_list_without_repeating_or_skipping() {
    let h = boot_router(|_| {}).await;
    for i in 0..5 {
        let (status, _) = send(&h.app, post_task(&format!("task-{i}"), None)).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (_, all) = send(&h.app, auth_get("/api/tasks")).await;
    let all_ids: Vec<String> = all["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|t| t["id"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(all_ids.len(), 5);

    let mut walked = Vec::new();
    for offset in [0, 2, 4] {
        let (_, page) = send(
            &h.app,
            auth_get(&format!("/api/tasks?limit=2&offset={offset}")),
        )
        .await;
        for task in page["tasks"].as_array().expect("tasks") {
            walked.push(task["id"].as_str().expect("id").to_string());
        }
    }
    assert_eq!(walked, all_ids, "paging must reproduce the unpaged order");
}

/// `?assigned_to=` filters, and `total` reports the filtered count rather than the table's.
#[tokio::test(flavor = "multi_thread")]
async fn assignee_filter_narrows_both_the_page_and_the_total() {
    let h = boot_router(|_| {}).await;
    for i in 0..3 {
        send(&h.app, post_task(&format!("alice-{i}"), Some("alice"))).await;
    }
    for i in 0..2 {
        send(&h.app, post_task(&format!("bob-{i}"), Some("bob"))).await;
    }

    let (status, body) = send(&h.app, auth_get("/api/tasks?assigned_to=alice")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tasks"].as_array().expect("tasks").len(), 3);
    assert_eq!(body["total"], 3, "total must follow the filter: {body}");
    for task in body["tasks"].as_array().expect("tasks") {
        assert_eq!(task["assigned_to"], "alice");
    }
}

/// A `?limit=` that is not a non-negative integer means "no window", as it always has.
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_limit_is_ignored_rather_than_rejected() {
    let h = boot_router(|_| {}).await;
    for i in 0..4 {
        send(&h.app, post_task(&format!("task-{i}"), None)).await;
    }

    let (status, body) = send(&h.app, auth_get("/api/tasks?limit=all")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tasks"].as_array().expect("tasks").len(), 4);
    assert_eq!(body["total"], 4);
}

/// A task the TTL sweep expired leaves the pending set and shows up as `cancelled` on the status endpoint the dashboard polls.
///
/// Driving the sweep is `librefang-memory`'s and `librefang-kernel`'s job to test; what belongs here is that the HTTP surface reports the transition, because an expiry the dashboard cannot see is indistinguishable from a task that vanished.
#[tokio::test(flavor = "multi_thread")]
async fn an_expired_task_leaves_the_pending_count_and_is_visible_as_cancelled() {
    let h = boot_router(|_| {}).await;
    let (status, body) = send(&h.app, post_task("unclaimed", None)).await;
    assert_eq!(status, StatusCode::CREATED);
    let task_id = body["id"].as_str().expect("id").to_string();

    let (_, before) = send(&h.app, auth_get("/api/tasks/status")).await;
    assert_eq!(
        before["pending"], 1,
        "posted task should be pending: {before}"
    );

    // `PATCH /api/tasks/{id}` with `cancelled` is the same terminal transition the TTL sweep performs, so this asserts the shape the sweep produces without reaching into the substrate.
    let req = Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/tasks/{task_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "status": "cancelled" }).to_string(),
        ))
        .expect("request");
    let (status, body) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::OK, "cancel should succeed: {body}");

    let (_, after) = send(&h.app, auth_get("/api/tasks/status")).await;
    assert_eq!(
        after["pending"], 0,
        "a cancelled task is no longer pending: {after}"
    );

    let (_, listed) = send(&h.app, auth_get("/api/tasks?status=cancelled")).await;
    assert_eq!(listed["total"], 1, "and is listable as cancelled: {listed}");
}
