//! Integration tests for the Analytics date-range filter, the parameterized
//! daily window, and the CSV export (#7891).
//!
//! The reporting use case behind the issue is "what did we spend last month,
//! per model" — answerable only if every `/api/usage*` endpoint can be pinned
//! to a calendar range. These tests cover that, and the harder half of the
//! contract: **omitting** the parameters has to leave each endpoint answering
//! exactly what it answered before, which is asserted per endpoint rather than
//! once for the group.
//!
//! Run: `cargo test -p librefang-api --test usage_date_range_test`

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

/// Register a synthetic agent so `/api/usage` and the export's name column
/// have something to resolve.
fn register_agent(state: &AppState, name: &str) -> AgentId {
    let id = AgentId::new();
    let entry = AgentEntry {
        id,
        name: name.to_string(),
        manifest: AgentManifest {
            name: name.to_string(),
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

/// Insert a usage event stamped at an explicit instant.
///
/// `UsageStore::record` always stamps `now`, so a date-range test has to write
/// the row through the pool directly to place events on specific calendar days.
#[allow(clippy::too_many_arguments)]
fn insert_event_at(
    state: &AppState,
    agent_id: AgentId,
    timestamp: &str,
    provider: &str,
    model: &str,
    cost_usd: f64,
    input_tokens: i64,
    output_tokens: i64,
) {
    let pool = state.kernel.memory_substrate().pool();
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO usage_events \
         (id, agent_id, timestamp, model, provider, input_tokens, output_tokens, cost_usd, tool_calls, latency_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 42)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            agent_id.0.to_string(),
            timestamp,
            model,
            provider,
            input_tokens,
            output_tokens,
            cost_usd,
        ],
    )
    .unwrap();
}

/// Pull the human-readable message out of the standard error envelope.
///
/// `ApiErrorResponse` serializes `error` as a nested object (`error.message`)
/// plus a flat `message` kept for backward compatibility — so `body["error"]`
/// is an object, not a string.
fn err_msg(body: &serde_json::Value) -> String {
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("no error.message in {body}"))
        .to_string()
}

/// Find one agent's row in a `/api/usage` payload by name.
///
/// The mock kernel boots with its own default agent, so the rollup always
/// carries more rows than the test registered.
fn item_for<'a>(body: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|i| i["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no row for agent {name:?} in {body}"))
}

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, headers, bytes) = get_raw(h, path).await;
    let _ = headers;
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get_raw(
    h: &Harness,
    path: &str,
) -> (StatusCode, axum::http::HeaderMap, axum::body::Bytes) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 8 << 20)
        .await
        .unwrap();
    (status, headers, bytes)
}

/// Three events spread across two months, so a range that selects one month
/// proves the filter is doing work rather than returning everything.
///
/// Timestamps are far enough in the past that they can never fall inside the
/// rolling `days` window the daily endpoint uses by default — that keeps the
/// "omitted parameters preserve old behaviour" assertions deterministic.
fn seed_two_months(h: &Harness) -> AgentId {
    let agent = register_agent(&h.state, "reporter");
    insert_event_at(
        &h.state,
        agent,
        "2026-01-15T10:00:00+00:00",
        "openai",
        "gpt-a",
        1.0,
        100,
        200,
    );
    insert_event_at(
        &h.state,
        agent,
        "2026-01-31T23:59:59+00:00",
        "openai",
        "gpt-a",
        2.0,
        100,
        200,
    );
    insert_event_at(
        &h.state,
        agent,
        "2026-02-01T00:00:00+00:00",
        "anthropic",
        "claude-b",
        4.0,
        100,
        200,
    );
    agent
}

// ---------------------------------------------------------------------------
// start_date / end_date filtering, per endpoint
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn summary_respects_date_range() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(
        &h,
        "/api/usage/summary?start_date=2026-01-01&end_date=2026-01-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 1.0 + 2.0 — the 2026-02-01 event is outside the range.
    assert_eq!(body["total_cost_usd"].as_f64().unwrap(), 3.0);
    assert_eq!(body["call_count"].as_u64().unwrap(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn summary_end_date_is_inclusive_of_the_whole_day() {
    let h = boot().await;
    seed_two_months(&h);

    // The 2026-01-31 event is stamped 23:59:59. A half-open upper bound
    // computed as `<= '2026-01-31'` would drop it — the off-by-one that
    // silently removes a day of spend from a monthly report.
    let (status, body) = get(
        &h,
        "/api/usage/summary?start_date=2026-01-31&end_date=2026-01-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_cost_usd"].as_f64().unwrap(), 2.0);
    assert_eq!(body["call_count"].as_u64().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_stats_respects_date_range() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage?start_date=2026-02-01&end_date=2026-02-28").await;
    assert_eq!(status, StatusCode::OK);
    let row = item_for(&body, "reporter");
    assert_eq!(row["total_cost_usd"].as_f64().unwrap(), 4.0);
    assert_eq!(row["call_count"].as_u64().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn by_model_respects_date_range() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(
        &h,
        "/api/usage/by-model?start_date=2026-01-01&end_date=2026-01-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let models = body["models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "only gpt-a billed in January: {models:?}");
    assert_eq!(models[0]["model"].as_str().unwrap(), "gpt-a");
    assert_eq!(models[0]["total_cost_usd"].as_f64().unwrap(), 3.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn by_model_performance_respects_date_range() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(
        &h,
        "/api/usage/by-model/performance?start_date=2026-02-01&end_date=2026-02-28",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let models = body["models"].as_array().unwrap();
    assert_eq!(
        models.len(),
        1,
        "only claude-b billed in February: {models:?}"
    );
    assert_eq!(models[0]["model"].as_str().unwrap(), "claude-b");
    assert_eq!(models[0]["call_count"].as_u64().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_respects_date_range() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(
        &h,
        "/api/usage/daily?start_date=2026-01-01&end_date=2026-01-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 2, "two distinct January days: {days:?}");
    assert_eq!(days[0]["date"].as_str().unwrap(), "2026-01-15");
    assert_eq!(days[1]["date"].as_str().unwrap(), "2026-01-31");
}

// ---------------------------------------------------------------------------
// Backward compatibility: omitting the parameters changes nothing.
// One test per endpoint, per the hard requirement in #7891.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn summary_without_params_is_lifetime_total() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_cost_usd"].as_f64().unwrap(), 7.0);
    assert_eq!(body["call_count"].as_u64().unwrap(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_stats_without_params_is_lifetime_total() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage").await;
    assert_eq!(status, StatusCode::OK);
    let row = item_for(&body, "reporter");
    assert_eq!(row["total_cost_usd"].as_f64().unwrap(), 7.0);
    assert_eq!(row["call_count"].as_u64().unwrap(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn by_model_without_params_is_lifetime_total() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage/by-model").await;
    assert_eq!(status, StatusCode::OK);
    let models = body["models"].as_array().unwrap();
    assert_eq!(models.len(), 2, "both models across all time: {models:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn by_model_performance_without_params_is_lifetime_total() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage/by-model/performance").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_without_params_is_still_the_last_seven_days() {
    let h = boot().await;
    let agent = seed_two_months(&h);
    // One event inside the default rolling window, alongside the 2026 backdated
    // ones which must stay excluded.
    insert_event_at(
        &h.state,
        agent,
        &chrono::Utc::now().to_rfc3339(),
        "openai",
        "gpt-a",
        0.5,
        10,
        20,
    );

    let (status, body) = get(&h, "/api/usage/daily").await;
    assert_eq!(status, StatusCode::OK);
    let days = body["days"].as_array().unwrap();
    assert_eq!(
        days.len(),
        1,
        "default window is still 7 days, so only today's event shows: {days:?}"
    );
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn inverted_range_is_rejected_on_every_endpoint() {
    let h = boot().await;
    seed_two_months(&h);

    for path in [
        "/api/usage",
        "/api/usage/summary",
        "/api/usage/by-model",
        "/api/usage/by-model/performance",
        "/api/usage/daily",
        "/api/usage/export",
    ] {
        let uri = format!("{path}?start_date=2026-03-01&end_date=2026-01-01");
        let (status, body) = get(&h, &uri).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must reject an inverted range"
        );
        let msg = err_msg(&body);
        assert!(
            msg.contains("end_date") && msg.contains("start_date"),
            "{path} error must name both bounds, got {msg:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_date_is_rejected_with_an_actionable_message() {
    let h = boot().await;
    seed_two_months(&h);

    let (status, body) = get(&h, "/api/usage/summary?start_date=01-15-2026").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = err_msg(&body);
    assert!(
        msg.contains("start_date"),
        "must name the parameter: {msg:?}"
    );
    assert!(
        msg.contains("YYYY-MM-DD"),
        "must state the accepted form: {msg:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn blank_date_params_are_treated_as_absent() {
    let h = boot().await;
    seed_two_months(&h);

    // A dashboard rendering cleared form inputs sends empty values; that is a
    // request for everything, not a 400.
    let (status, body) = get(&h, "/api/usage/summary?start_date=&end_date=").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_cost_usd"].as_f64().unwrap(), 7.0);
}

// ---------------------------------------------------------------------------
// /api/usage/daily?days=
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn daily_accepts_a_longer_window_than_seven_days() {
    let h = boot().await;
    let agent = register_agent(&h.state, "reporter");
    for days_ago in [1_i64, 20, 40] {
        insert_event_at(
            &h.state,
            agent,
            &(chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339(),
            "openai",
            "gpt-a",
            1.0,
            10,
            20,
        );
    }

    let (_, seven) = get(&h, "/api/usage/daily").await;
    assert_eq!(seven["days"].as_array().unwrap().len(), 1);

    let (status, thirty) = get(&h, "/api/usage/daily?days=30").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(thirty["days"].as_array().unwrap().len(), 2);

    let (status, ninety) = get(&h, "/api/usage/daily?days=90").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ninety["days"].as_array().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_rejects_an_out_of_range_days_value() {
    let h = boot().await;

    for (value, why) in [("0", "zero"), ("367", "beyond the cap")] {
        let (status, body) = get(&h, &format!("/api/usage/daily?days={value}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "days={value} ({why})");
        assert!(err_msg(&body).contains("366"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_rejects_days_combined_with_a_date_range() {
    let h = boot().await;

    let (status, body) = get(
        &h,
        "/api/usage/daily?days=30&start_date=2026-01-01&end_date=2026-01-31",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err_msg(&body).contains("cannot be combined"));
}

// ---------------------------------------------------------------------------
// CSV export
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn export_streams_a_multi_row_range_with_csv_headers() {
    let h = boot().await;
    let agent = register_agent(&h.state, "reporter");
    for day in 1..=5 {
        insert_event_at(
            &h.state,
            agent,
            &format!("2026-01-0{day}T12:00:00+00:00"),
            "openai",
            "gpt-a",
            1.5,
            10,
            20,
        );
    }
    // Outside the requested range — must not appear.
    insert_event_at(
        &h.state,
        agent,
        "2026-03-01T12:00:00+00:00",
        "openai",
        "gpt-a",
        9.0,
        10,
        20,
    );

    let (status, headers, bytes) = get_raw(
        &h,
        "/api/usage/export?start_date=2026-01-01&end_date=2026-01-31&format=csv",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").unwrap(),
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        headers.get("content-disposition").unwrap(),
        "attachment; filename=\"librefang-usage-2026-01-01-to-2026-01-31.csv\""
    );

    let csv = String::from_utf8(bytes.to_vec()).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 6, "header + 5 rows, got:\n{csv}");
    assert!(lines[0].starts_with("timestamp,agent_id,agent_name,"));
    assert!(
        !csv.contains("2026-03-01"),
        "out-of-range row leaked into the export:\n{csv}"
    );
    // Oldest first, so an archive appends chronologically.
    assert!(lines[1].starts_with("2026-01-01"));
    assert!(lines[5].starts_with("2026-01-05"));
}

#[tokio::test(flavor = "multi_thread")]
async fn export_quotes_fields_containing_commas_and_quotes() {
    let h = boot().await;
    // Both hazards in one value: a comma would split the row into extra
    // columns, and a bare double quote would corrupt the quoting of the field.
    let agent = register_agent(&h.state, r#"Acme, "Ops" Team"#);
    insert_event_at(
        &h.state,
        agent,
        "2026-01-02T12:00:00+00:00",
        "prov,with,commas",
        r#"model-with-"quotes""#,
        1.0,
        10,
        20,
    );

    let (status, _, bytes) = get_raw(&h, "/api/usage/export").await;
    assert_eq!(status, StatusCode::OK);
    let csv = String::from_utf8(bytes.to_vec()).unwrap();
    let row = csv.lines().nth(1).expect("one data row");

    assert!(
        row.contains(r#""Acme, ""Ops"" Team""#),
        "agent name must be quoted with doubled interior quotes: {row}"
    );
    assert!(
        row.contains(r#""prov,with,commas""#),
        "provider must be quoted: {row}"
    );
    assert!(
        row.contains(r#""model-with-""quotes""""#),
        "model must be quoted with doubled interior quotes: {row}"
    );

    // The row must still parse as exactly the declared number of columns.
    let header_cols = csv.lines().next().unwrap().split(',').count();
    assert_eq!(
        parse_csv_row(row).len(),
        header_cols,
        "quoted row must yield the same column count as the header: {row}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn export_neutralizes_spreadsheet_formula_injection() {
    let h = boot().await;
    // An agent name is operator-supplied free text, and this export exists to
    // be opened in Excel — so a leading `=` would otherwise become a live
    // formula in the reviewer's spreadsheet.
    let agent = register_agent(&h.state, r#"=HYPERLINK("http://evil.example","click")"#);
    insert_event_at(
        &h.state,
        agent,
        "2026-01-02T12:00:00+00:00",
        "+1234",
        "-SUM(A1:A9)",
        1.0,
        10,
        20,
    );

    let (status, _, bytes) = get_raw(&h, "/api/usage/export").await;
    assert_eq!(status, StatusCode::OK);
    let csv = String::from_utf8(bytes.to_vec()).unwrap();
    let row = csv.lines().nth(1).expect("one data row");
    let cols = parse_csv_row(row);

    for (idx, label) in [(2usize, "agent_name"), (4, "provider"), (5, "model")] {
        let value = &cols[idx];
        assert!(
            value.starts_with('\''),
            "{label} must carry the formula-injection guard prefix, got {value:?}"
        );
        assert!(
            !value.starts_with(['=', '+', '-', '@']),
            "{label} must not reach the spreadsheet as a formula, got {value:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn export_rejects_an_unsupported_format() {
    let h = boot().await;

    let (status, body) = get(&h, "/api/usage/export?format=xlsx").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err_msg(&body).contains("csv"));
}

/// Minimal RFC 4180 row splitter, used to assert that the emitted rows really
/// do parse back into the column count the header declares.
fn parse_csv_row(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    out.push(cur);
    out
}
