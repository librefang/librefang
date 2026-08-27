//! Integration tests for `GET /api/agents/{id}/ephemeral-runs` (refs #7752).
//!
//! An ephemeral worker (`agent_spawn` with `ephemeral: true`) runs one turn under its parent's identity and then vanishes: no registry entry, no persisted session, and a mission workspace deleted on the way out.
//! Its spend already reached the parent's ledger through `usage_events.billed_agent_id` (#7714), but the work behind the spend had no record at all, so an operator watching an agent misbehave through workers had nothing to inspect.
//!
//! These tests drive the real spawn path through the production kernel and then read the result back over the production router — the same `build_router` every other route test uses, so the auth middleware, the route registration in `routes/agents/mod.rs` and the handler are all in play.
//!
//! Run: cargo test -p librefang-api --test agent_ephemeral_runs_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    kernel: Arc<LibreFangKernel>,
    home: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Boot the production router on a driverless kernel.
///
/// `DefaultModelConfig::driverless()` resolves to the stub driver, whose refusal the agent loop recovers into an ordinary response — so a spawn reaches `run_agent_loop`, completes, and records, without a provider credential and without touching the network.
async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        default_model: DefaultModelConfig::driverless(),
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();

    let (app, state) =
        server::build_router(kernel.clone(), "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        kernel,
        home: tmp.path().to_path_buf(),
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

fn get_with(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {token}"));
    }
    b.body(Body::empty()).unwrap()
}

fn get(path: &str) -> Request<Body> {
    get_with(path, Some(TEST_TOKEN))
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

/// Entries left under the kernel's transient root — the mission workspaces.
fn transient_entries(home: &std::path::Path) -> Vec<String> {
    let root = home.join("transient");
    match std::fs::read_dir(&root) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// A completed ephemeral run is visible under the agent that spawned it.
///
/// The core of #7752: the run used to leave no reachable trace at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_completed_ephemeral_run_is_visible_under_its_parent() {
    let h = boot().await;
    let parent = spawn_named(&h.state, "eph-parent");

    let request = librefang_types::ephemeral::EphemeralSpawnRequest::new(
        parent,
        "researcher",
        "summarise the report",
    );
    let _ = h.kernel.spawn_ephemeral_worker(request).await;

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{parent}/ephemeral-runs")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let runs = body["runs"].as_array().expect("runs array");
    assert_eq!(
        runs.len(),
        1,
        "one spawn must surface as exactly one run under its parent: {body}"
    );
    let run = &runs[0];
    assert_eq!(run["label"], "researcher");
    assert_eq!(
        run["task"], "summarise the report",
        "the record must say what the parent delegated"
    );
    assert!(
        run["worker_name"]
            .as_str()
            .expect("worker_name")
            .starts_with("researcher-"),
        "the record must name the uid the worker ran under: {run}"
    );
    assert!(
        run["status"].is_string() && run["id"].is_string(),
        "a run must carry an identity and an outcome: {run}"
    );

    assert_eq!(
        body["rollup"]["runs"], 1,
        "the rollup must count the run it returned: {body}"
    );

    // The workspace is a separate guarantee (#7723) and it still holds: a run record is not a surviving scratch directory, and persisting the first must not have bought it by keeping the second.
    assert!(
        transient_entries(&h.home).is_empty(),
        "the mission workspace must still be removed, found: {:?}",
        transient_entries(&h.home)
    );
}

/// An agent that spawned no workers is unaffected: an empty list and a zero rollup, not an error and not another agent's runs.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_ephemeral_runs_gets_an_empty_list() {
    let h = boot().await;
    let busy = spawn_named(&h.state, "eph-busy");
    let idle = spawn_named(&h.state, "eph-idle");

    let request =
        librefang_types::ephemeral::EphemeralSpawnRequest::new(busy, "researcher", "do the thing");
    let _ = h.kernel.spawn_ephemeral_worker(request).await;

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{idle}/ephemeral-runs")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["runs"].as_array().expect("runs array").len(),
        0,
        "an agent that spawned nothing must not see another agent's workers: {body}"
    );
    assert_eq!(body["rollup"]["runs"], 0);
    assert_eq!(body["rollup"]["cost_usd"], 0.0);

    // …and the busy agent still has its own, so the empty answer above is scoping rather than a route that returns nothing to everyone.
    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{busy}/ephemeral-runs")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["runs"].as_array().expect("runs array").len(), 1);
}

/// `limit` caps the returned rows while the rollup keeps covering all retained runs.
#[tokio::test(flavor = "multi_thread")]
async fn the_limit_parameter_caps_rows_without_capping_the_rollup() {
    let h = boot().await;
    let parent = spawn_named(&h.state, "eph-limit");

    for i in 0..3 {
        let request = librefang_types::ephemeral::EphemeralSpawnRequest::new(
            parent,
            format!("mission{i}"),
            "do the thing",
        );
        let _ = h.kernel.spawn_ephemeral_worker(request).await;
    }

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{parent}/ephemeral-runs?limit=2")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["runs"].as_array().expect("runs array").len(),
        2,
        "limit must cap the rows returned: {body}"
    );
    assert_eq!(
        body["rollup"]["runs"], 3,
        "the rollup describes the retained runs, not the page: {body}"
    );
}

/// Unknown and malformed agent ids are refused the same way the sibling observability routes refuse them.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_and_invalid_agent_ids_are_refused() {
    let h = boot().await;

    let (status, _) = send(h.app.clone(), get("/api/agents/not-a-uuid/ephemeral-runs")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let missing = AgentId::new();
    let (status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{missing}/ephemeral-runs")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The route sits behind the same auth layer as the rest of `/api/agents`.
///
/// A run record carries the delegated task and the worker's answer verbatim, so an unauthenticated read would leak conversation content, not just counters.
#[tokio::test(flavor = "multi_thread")]
async fn the_route_requires_authentication() {
    let h = boot().await;
    let parent = spawn_named(&h.state, "eph-auth");

    let (status, _) = send(
        h.app.clone(),
        get_with(&format!("/api/agents/{parent}/ephemeral-runs"), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an ephemeral run record must not be readable without a token"
    );
}
