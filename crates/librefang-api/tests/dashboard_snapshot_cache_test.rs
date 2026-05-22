//! Regression guard for the `dashboard-snapshot-no-cache` audit fix.
//!
//! `/api/dashboard/snapshot` is the dashboard's primary poll endpoint,
//! hit every 5 seconds. Pre-fix, every request walked every agent and
//! enriched manifest + current session + recent message count from
//! scratch — at N=200 agents a single dashboard tab pushed ~40 QPS of
//! pure cache-miss work into SQLite + kernel snapshot accessors.
//!
//! The fix memoizes the assembled payload behind a short TTL inside
//! `routes/config.rs::dashboard_snapshot_inner`. These tests pin the
//! contract:
//!
//!   1. Two polls inside the TTL window return the *same* payload —
//!      including fields that we deliberately mutate behind the route's
//!      back between polls. If the cache is missing, the second poll
//!      would observe the mutation and the test would fail.
//!   2. Once the TTL elapses, the route rebuilds and observes the
//!      mutation. This is the "stays live for 5 s polls" guarantee —
//!      the cache must never out-live a single dashboard tick.
//!
//! Audit ticket: `docs/issues/dashboard-snapshot-no-cache.md`.

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
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Back-to-back polls within the TTL window must return the *same*
/// payload, even when a side-effect (a substrate session insert) lands
/// between them. We use `session_count` as the side-effect probe
/// because it's the cheapest piece of cached state that visibly
/// reflects substrate writes — if the second poll's `session_count`
/// already shows the new seeds, the cache is bypassed.
#[tokio::test(flavor = "multi_thread")]
async fn snapshot_is_cached_within_ttl_window() {
    let h = boot().await;

    // Prime the cache.
    let (status, first) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{first:?}");
    let baseline = first["status"]["session_count"]
        .as_u64()
        .expect("status.session_count must be a number");

    // Mutate substrate state behind the route's back. A fresh, un-cached
    // route would observe this; a cached one must not.
    let agent_id = AgentId(uuid::Uuid::new_v4());
    let substrate = h.state.kernel.memory_substrate();
    for _ in 0..5 {
        substrate.create_session(agent_id).expect("seed session");
    }

    // Second poll inside the TTL window: must serve the cached payload
    // and therefore must *not* observe the 5 fresh sessions yet.
    let (status, second) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{second:?}");
    assert_eq!(
        second["status"]["session_count"].as_u64(),
        Some(baseline),
        "in-TTL second poll must return cached session_count (baseline={baseline}); \
         observing the +5 substrate seeds means the cache is being bypassed: \
         second={second:?}"
    );

    // Stronger: the entire payload must be byte-identical between the
    // two cached calls. If anything inside the payload is non-deterministic
    // (e.g. a `now()` timestamp baked at serialize time), this catches it
    // — and would catch a bug where the cache stores stale data but the
    // route also re-stamps a "served_at" field.
    assert_eq!(
        first, second,
        "cached payload must be byte-identical between back-to-back polls"
    );
}

/// After the TTL elapses the cache must miss, the route must rebuild,
/// and the new payload must reflect substrate writes that happened
/// during the cached window. This is the "stays live for 5 s polls"
/// half of the contract — without TTL expiry the cache would become a
/// permanent staleness.
#[tokio::test(flavor = "multi_thread")]
async fn snapshot_rebuilds_after_ttl_expires() {
    let h = boot().await;

    let (status, first) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{first:?}");
    let baseline = first["status"]["session_count"]
        .as_u64()
        .expect("status.session_count must be a number");

    let agent_id = AgentId(uuid::Uuid::new_v4());
    let substrate = h.state.kernel.memory_substrate();
    for _ in 0..3 {
        substrate.create_session(agent_id).expect("seed session");
    }

    // Sleep past the cache TTL. The route's TTL is 900 ms; we wait
    // 1.5 s to leave generous headroom for slow CI without making the
    // test slow on a healthy machine.
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;

    let (status, second) = get_json(&h, "/api/dashboard/snapshot").await;
    assert_eq!(status, StatusCode::OK, "{second:?}");
    assert_eq!(
        second["status"]["session_count"].as_u64(),
        Some(baseline + 3),
        "post-TTL poll must rebuild and observe the 3 seeded sessions \
         (baseline={baseline}). If this fails, the cache TTL is too long \
         or the route is pinning to the cache without checking expiry: \
         second={second:?}"
    );
}
