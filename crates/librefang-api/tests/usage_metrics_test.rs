//! Integration tests for the reporting fields the Analytics page binds to (#8062).
//!
//! The issue behind these tests is an operator who gave up on the built-in
//! Analytics page and maintains a hand-written HTML dashboard against the same
//! `/api/usage/*` endpoints, because the page did not surface the numbers a
//! cost or SLO report needs. Bringing the page up to parity means the dashboard
//! now reads a specific set of response fields, and a field silently dropped
//! from a handler would leave the page rendering `0` or `—` with nothing
//! failing — the exact failure mode `CLAUDE.md`'s mandatory-integration-test
//! rule exists to catch.
//!
//! So each test here pins one field to the endpoint that must keep serving it,
//! and the two new fields (`p95_latency_ms`, `retention_days`) additionally get
//! a correctness assertion rather than a presence check.
//!
//! Date-range behaviour itself is covered by `usage_date_range_test.rs` (#7891);
//! this file only asserts that the new percentile is computed *within* the
//! selected range rather than over the whole table.
//!
//! Run: `cargo test -p librefang-api --test usage_metrics_test`

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, SessionId,
};
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
        .nest("/api", routes::budget::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

fn register_agent(state: &AppState, name: &str, is_hand: bool) -> AgentId {
    let id = AgentId::new();
    let entry = AgentEntry {
        id,
        name: name.to_string(),
        is_hand,
        manifest: AgentManifest {
            name: name.to_string(),
            source_template: None,
            description: "test agent".to_string(),
            author: "test".to_string(),
            module: "builtin:chat".to_string(),
            ..Default::default()
        },
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    state.kernel.agent_registry().register(entry).unwrap();
    id
}

/// Insert one usage event at an explicit instant with an explicit latency.
///
/// `UsageStore::record` stamps `now` and takes its latency from the driver, so
/// a percentile test has to write rows through the pool to control both.
#[allow(clippy::too_many_arguments)]
fn insert_event(
    state: &AppState,
    agent_id: AgentId,
    timestamp: &str,
    model: &str,
    cost_usd: f64,
    input_tokens: i64,
    output_tokens: i64,
    tool_calls: i64,
    latency_ms: i64,
) {
    let pool = state.kernel.memory_substrate().pool();
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO usage_events \
         (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
         VALUES (?1, ?2, ?3, ?4, 'openai', ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            agent_id.0.to_string(),
            timestamp,
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            tool_calls,
            latency_ms,
        ],
    )
    .unwrap();
}

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 8 << 20)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn model_row<'a>(body: &'a serde_json::Value, model: &str) -> &'a serde_json::Value {
    body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .find(|m| m["model"].as_str() == Some(model))
        .unwrap_or_else(|| panic!("no row for model {model:?} in {body}"))
}

fn agent_row<'a>(body: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|i| i["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no row for agent {name:?} in {body}"))
}

/// Twenty events on one model with latencies 10, 20, … 200 ms.
///
/// Chosen so the three latency statistics are all different numbers: min 10,
/// avg 105, P95 190, max 200. A test seeded with a uniform latency would pass
/// against a P95 that was accidentally wired to `MAX(latency_ms)`.
fn seed_latency_ladder(h: &Harness) -> AgentId {
    let agent = register_agent(&h.state, "ladder", false);
    for i in 1..=20 {
        insert_event(
            &h.state,
            agent,
            "2026-03-15T10:00:00+00:00",
            "gpt-ladder",
            0.5,
            100,
            50,
            2,
            i * 10,
        );
    }
    agent
}

// ---------------------------------------------------------------------------
// P95 latency (#8062 item 8) — the one field the issue named that the API did
// not have.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn model_performance_reports_nearest_rank_p95_latency() {
    let h = boot().await;
    seed_latency_ladder(&h);

    let (status, body) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(status, StatusCode::OK);

    let row = model_row(&body, "gpt-ladder");
    // Nearest-rank P95 over n = 20 is the value at rank ceil(0.95 * 20) = 19,
    // which is 190 ms — distinct from max (200) and avg (105), so this asserts
    // the percentile rather than merely the presence of a number.
    assert_eq!(row["p95_latency_ms"], 190);
    assert_eq!(row["max_latency_ms"], 200);
    assert_eq!(row["min_latency_ms"], 10);
    assert_eq!(row["avg_latency_ms"], 105.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn p95_latency_of_a_single_event_is_that_events_latency() {
    let h = boot().await;
    let agent = register_agent(&h.state, "lonely", false);
    insert_event(
        &h.state,
        agent,
        "2026-03-15T10:00:00+00:00",
        "gpt-one",
        0.25,
        10,
        5,
        0,
        1234,
    );

    let (status, body) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(status, StatusCode::OK);
    // ceil(0.95 * 1) = 1, so the single sample is its own P95. A rank
    // computed as `n * 95 / 100` would truncate to 0 and select no row,
    // leaving the field at the COALESCE default of 0.
    assert_eq!(model_row(&body, "gpt-one")["p95_latency_ms"], 1234);
}

#[tokio::test(flavor = "multi_thread")]
async fn p95_latency_is_computed_within_the_selected_date_range() {
    let h = boot().await;
    let agent = register_agent(&h.state, "ranged", false);
    // March: latencies 100 and 200. April: a single 9000 ms outlier that must
    // not leak into March's percentile.
    for latency in [100, 200] {
        insert_event(
            &h.state,
            agent,
            "2026-03-10T10:00:00+00:00",
            "gpt-ranged",
            1.0,
            10,
            10,
            0,
            latency,
        );
    }
    insert_event(
        &h.state,
        agent,
        "2026-04-10T10:00:00+00:00",
        "gpt-ranged",
        1.0,
        10,
        10,
        0,
        9000,
    );

    let (status, march) = get(
        &h,
        "/api/usage/by-model/performance?start_date=2026-03-01&end_date=2026-03-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // n = 2 within March, rank ceil(1.9) = 2, so P95 is 200 — not 9000.
    assert_eq!(model_row(&march, "gpt-ranged")["p95_latency_ms"], 200);

    let (_, all) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(model_row(&all, "gpt-ranged")["p95_latency_ms"], 9000);
}

#[tokio::test(flavor = "multi_thread")]
async fn p95_latency_ignores_events_with_no_measured_latency() {
    let h = boot().await;
    let agent = register_agent(&h.state, "upgraded", false);
    // Ten measured samples, 100 … 1000 ms.
    for i in 1..=10 {
        insert_event(
            &h.state,
            agent,
            "2026-03-15T10:00:00+00:00",
            "gpt-upgraded",
            0.1,
            10,
            10,
            0,
            i * 100,
        );
    }
    // Ninety rows as they exist on any database upgraded across migration v14,
    // which added `latency_ms INTEGER NOT NULL DEFAULT 0` and so backfilled
    // every pre-existing event with `0`. Those rows never had a latency
    // measured; `0` is the column default, not a real observation.
    for _ in 0..90 {
        insert_event(
            &h.state,
            agent,
            "2026-03-15T10:00:00+00:00",
            "gpt-upgraded",
            0.1,
            10,
            10,
            0,
            0,
        );
    }

    let (status, body) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(status, StatusCode::OK);
    let row = model_row(&body, "gpt-upgraded");
    // n = 10 measured samples, rank ceil(9.5) = 10, so the P95 is 1000 —
    // exactly what `SessionStore::agent_stats_24h` reports for the same data,
    // because it applies the same `latency_ms > 0` guard.
    //
    // Ranking the ninety unmeasured rows instead makes n = 100 and rank 95,
    // and since the zeros occupy ranks 1..90 that rank lands on the fifth
    // measured sample: 500 ms, the median of the real latencies dressed up as
    // a 95th percentile. Halving a latency SLO's input is the quiet kind of
    // wrong this whole file exists to catch.
    assert_eq!(row["p95_latency_ms"], 1000);
    // The three long-standing aggregates deliberately keep counting every row:
    // their shape is already shipped and read by external dashboards, so the
    // fix is scoped to the new percentile rather than silently redefining them.
    assert_eq!(row["min_latency_ms"], 0);
    assert_eq!(row["max_latency_ms"], 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn p95_latency_is_zero_when_no_event_carries_a_latency() {
    let h = boot().await;
    let agent = register_agent(&h.state, "unmeasured", false);
    for _ in 0..5 {
        insert_event(
            &h.state,
            agent,
            "2026-03-15T10:00:00+00:00",
            "gpt-unmeasured",
            0.1,
            10,
            10,
            0,
            0,
        );
    }

    let (status, body) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(status, StatusCode::OK);
    let row = model_row(&body, "gpt-unmeasured");
    // The model still has spend, so it must still appear in the table — the
    // `LEFT JOIN` onto the percentile CTE is what keeps the row alive when
    // that CTE has nothing to contribute, and the percentile reads 0 rather
    // than dropping the model's cost off the report.
    assert_eq!(row["p95_latency_ms"], 0);
    assert_eq!(row["call_count"], 5);
}

// ---------------------------------------------------------------------------
// Retention horizon (#8062 item 10)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn daily_reports_the_configured_retention_horizon() {
    let h = boot().await;

    let (status, body) = get(&h, "/api/usage/daily").await;
    assert_eq!(status, StatusCode::OK);
    // The compiled default of `UsageConfig::retention_days`. The page captions
    // its "data since" indicator with this, so a missing field would render as
    // an unbounded window and quietly overstate how far back the numbers go.
    assert_eq!(body["retention_days"], 90);
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_reports_first_event_date_and_todays_cost() {
    let h = boot().await;
    let agent = register_agent(&h.state, "historian", false);
    insert_event(
        &h.state,
        agent,
        "2026-02-03T04:05:06+00:00",
        "gpt-old",
        1.0,
        10,
        10,
        0,
        50,
    );

    let (status, body) = get(&h, "/api/usage/daily").await;
    assert_eq!(status, StatusCode::OK);
    // `first_event_date` is deliberately unfiltered by the range — it answers
    // "how far back does the stored data go", which a range-scoped answer
    // cannot.
    //
    // Despite the `_date` suffix the field is `MIN(timestamp)`, so it carries a
    // full RFC 3339 instant rather than a `YYYY-MM-DD` day. That is pinned here
    // deliberately: the shape is already shipped and consumed by external
    // dashboards, so truncating it server-side would be a silent breaking
    // change for them. The dashboard renders the date portion instead.
    assert_eq!(body["first_event_date"], "2026-02-03T04:05:06+00:00");
    assert!(
        body["first_event_date"]
            .as_str()
            .is_some_and(|s| s.starts_with("2026-02-03")),
        "the date portion is what the page renders; got {body}"
    );
    assert!(
        body["today_cost_usd"].is_number(),
        "today_cost_usd must always be a number, got {body}"
    );
}

// ---------------------------------------------------------------------------
// Fields the page newly binds to. Presence + value, so a handler edit that
// drops one fails here instead of silently zeroing a tile.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn summary_reports_tool_calls_alongside_calls_and_cost() {
    let h = boot().await;
    seed_latency_ladder(&h);

    let (status, body) = get(&h, "/api/usage/summary").await;
    assert_eq!(status, StatusCode::OK);
    // 20 events × 2 tool calls. Cost-per-call is derived client-side from
    // `total_cost_usd / call_count`, so both of those are asserted too.
    assert_eq!(body["total_tool_calls"], 40);
    assert_eq!(body["call_count"], 20);
    assert_eq!(body["total_cost_usd"], 10.0);
    assert_eq!(body["total_input_tokens"], 2000);
    assert_eq!(body["total_output_tokens"], 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn by_model_reports_the_input_output_token_split() {
    let h = boot().await;
    seed_latency_ladder(&h);

    let (status, body) = get(&h, "/api/usage/by-model").await;
    assert_eq!(status, StatusCode::OK);
    let row = model_row(&body, "gpt-ladder");
    // The page showed a combined token count; a per-model prompt-vs-completion
    // split is what makes two models comparable on price.
    assert_eq!(row["total_input_tokens"], 2000);
    assert_eq!(row["total_output_tokens"], 1000);
    assert_eq!(row["call_count"], 20);
}

#[tokio::test(flavor = "multi_thread")]
async fn per_agent_rollup_reports_tool_calls_token_split_and_hand_flag() {
    let h = boot().await;
    let agent = register_agent(&h.state, "worker", false);
    let hand = register_agent(&h.state, "helper-hand", true);
    insert_event(
        &h.state,
        agent,
        "2026-03-15T10:00:00+00:00",
        "gpt-a",
        1.5,
        700,
        300,
        4,
        60,
    );
    insert_event(
        &h.state,
        hand,
        "2026-03-15T11:00:00+00:00",
        "gpt-a",
        0.5,
        100,
        100,
        1,
        60,
    );

    let (status, body) = get(&h, "/api/usage").await;
    assert_eq!(status, StatusCode::OK);

    let worker = agent_row(&body, "worker");
    assert_eq!(worker["tool_calls"], 4);
    assert_eq!(worker["input_tokens"], 700);
    assert_eq!(worker["output_tokens"], 300);
    assert_eq!(worker["total_tokens"], 1000);
    assert_eq!(worker["is_hand"], false);

    // `is_hand` is what lets the page label hand rows instead of dropping them
    // — a hand's spend is real money and was invisible before.
    let helper = agent_row(&body, "helper-hand");
    assert_eq!(helper["is_hand"], true);
    assert_eq!(helper["tool_calls"], 1);
}
