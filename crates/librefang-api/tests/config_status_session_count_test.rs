//! Regression guard for the `list-sessions-decode-on-poll` audit fix.
//!
//! `/api/status` and `/api/dashboard/snapshot` both surface a
//! `session_count` field that the dashboard polls every 5 s. The
//! pre-fix code computed it via `list_sessions()?.len()`, which
//! pulled and rmp-decoded every session's full message blob just to
//! reach `.len()`. The fix swaps that for `count_sessions()` — a real
//! `SELECT COUNT(*)`. These tests pin the contract that:
//!
//!   1. The count surfaced over the wire matches the substrate's
//!      `count_sessions()` truth.
//!   2. The count actually responds to new sessions (i.e. the route
//!      isn't silently pinned to `0` via the `.unwrap_or(0)` error
//!      arm — exactly the kind of regression a "still returns
//!      something" smoke test would miss).
//!
//! Audit ticket: `docs/issues/list-sessions-decode-on-poll.md`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::AgentId;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".into(),
            model: "test-model".into(),
            api_key_env: "OLLAMA_API_KEY".into(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));

    let state = test.state.clone();
    let app = Router::new()
        .route("/api/status", axum::routing::get(routes::status))
        .route(
            "/api/dashboard/snapshot",
            axum::routing::get(routes::dashboard_snapshot),
        )
        .with_state(state.clone());

    Harness {
        app,
        state,
        _test: test,
    }
}

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

#[tokio::test(flavor = "multi_thread")]
async fn status_session_count_matches_substrate_after_seeding() {
    let h = boot().await;

    let (status, body) = get_json(&h, "/api/status").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let baseline = body["session_count"]
        .as_u64()
        .expect("session_count must be a number");

    let agent_id = AgentId(uuid::Uuid::new_v4());
    let substrate = h.state.kernel.memory_substrate();
    substrate.create_session(agent_id).expect("seed 1");
    substrate.create_session(agent_id).expect("seed 2");
    substrate.create_session(agent_id).expect("seed 3");

    let (status, body) = get_json(&h, "/api/status").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["session_count"].as_u64(),
        Some(baseline + 3),
        "session_count must reflect the 3 seeded sessions (was the route silently \
         pinned to {baseline} via the unwrap_or(0) error arm?): {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_snapshot_session_count_matches_substrate_after_seeding() {
    let h = boot().await;

    // The dashboard snapshot envelopes the status block under `status`,
    // so `session_count` lives at `body.status.session_count` here
    // (vs. `body.session_count` for the standalone `/api/status` route).
    let (status, body) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let baseline = body["status"]["session_count"]
        .as_u64()
        .expect("status.session_count must be a number");

    let agent_id = AgentId(uuid::Uuid::new_v4());
    let substrate = h.state.kernel.memory_substrate();
    substrate.create_session(agent_id).expect("seed 1");
    substrate.create_session(agent_id).expect("seed 2");

    let (status, body) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["status"]["session_count"].as_u64(),
        Some(baseline + 2),
        "snapshot status.session_count must reflect the 2 seeded sessions: {body:?}"
    );
}
