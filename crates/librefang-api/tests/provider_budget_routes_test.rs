//! Integration tests for the per-provider budget HTTP routes (#5650).
//!
//! Slice covered here:
//!
//!   * `GET /api/budget/providers`                         — snapshot list
//!   * `PUT /api/budget/providers/{provider_id}`           — upsert caps
//!
//! These are the dashboard-facing surface for the per-provider gate that
//! shipped in #4807. CLAUDE.md MANDATORY: every route domain has a
//! `#[tokio::test]` against `TestServer`-style `MockKernelBuilder` —
//! see refs #3721.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_memory::usage::{UsageRecord, UsageStore};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::config::ProviderBudget;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// Insert a usage row attributed to `provider` directly into the SQLite
/// usage store. Bypasses the metering engine — the per-provider read
/// endpoint aggregates straight from `usage_events`, so raw inserts are
/// sufficient and keep the test independent of provider catalogs.
fn record_usage(
    state: &AppState,
    provider: &str,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
) {
    let store = UsageStore::new(state.kernel.memory_substrate().pool());
    let mut rec = UsageRecord::anonymous(
        AgentId::new(),
        provider,
        "test-model",
        input_tokens,
        output_tokens,
        cost_usd,
        0,
        10,
    );
    rec.session_id = Some(SessionId::new());
    store.record(&rec).unwrap();
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        // Seed: one configured provider with a 1$/hr cap and a 100$/day
        // cap, plus unlimited monthly + tokens. The PUT test will mutate
        // a different provider id to make sure we round-trip a fresh
        // entry rather than overwriting the seed.
        cfg.budget.providers.insert(
            "openai".to_string(),
            ProviderBudget {
                max_cost_per_hour_usd: 1.0,
                max_cost_per_day_usd: 100.0,
                max_cost_per_month_usd: 0.0,
                max_tokens_per_hour: 0,
            },
        );
        cfg.budget.alert_threshold = 0.75;
    }));

    // Persist seed config so PUT round-trips through a real config.toml
    // (same rationale as `budget_routes_test::boot`: without a real
    // path, persist_budget would refuse / clobber sibling sections).
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);

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

async fn request(
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

// ---------------------------------------------------------------------------
// GET /api/budget/providers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn provider_list_returns_configured_provider_with_zero_spend() {
    let h = boot().await;
    let (status, body) = request(&h, Method::GET, "/api/budget/providers", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert!(
        (body["alert_threshold"].as_f64().unwrap() - 0.75).abs() < f64::EPSILON,
        "alert_threshold echoed back: {body:?}"
    );

    let providers = body["providers"].as_array().expect("providers array");
    // Only the configured provider should be present — no usage rows yet
    // means the observed-but-unconfigured branch contributes nothing.
    assert_eq!(providers.len(), 1, "{providers:?}");
    let row = &providers[0];
    assert_eq!(row["provider"], "openai");
    assert_eq!(row["unconfigured"], false);
    assert!((row["cap_hourly_usd"].as_f64().unwrap() - 1.0).abs() < f64::EPSILON);
    assert!((row["cap_daily_usd"].as_f64().unwrap() - 100.0).abs() < f64::EPSILON);
    assert_eq!(row["cap_monthly_usd"], 0.0); // unlimited
    assert_eq!(row["cap_tokens_per_hour"], 0); // unlimited
    assert_eq!(row["spend_hourly_usd"], 0.0);
    assert_eq!(row["spend_daily_usd"], 0.0);
    assert_eq!(row["spend_monthly_usd"], 0.0);
    assert_eq!(row["tokens_this_hour"], 0);
    assert_eq!(row["is_exhausted"], false);
    assert!(row["exhaustion_reason"].is_null());
    assert!(row["exhaustion_remaining_ms"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_list_reflects_observed_usage() {
    let h = boot().await;
    // Spend $0.50 on the configured provider — well under the $1/hr cap.
    record_usage(&h.state, "openai", 0.5, 1000, 2000);
    // And $0.10 on an unconfigured provider — should surface as a row
    // with `unconfigured = true` so the dashboard can prompt "Set a cap".
    record_usage(&h.state, "anthropic", 0.1, 500, 1500);

    let (status, body) = request(&h, Method::GET, "/api/budget/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    let providers = body["providers"].as_array().expect("providers array");
    assert_eq!(providers.len(), 2, "{providers:?}");

    // Rows must be sorted ascending by provider id — `anthropic` first,
    // `openai` second. This pins the #3298 determinism guarantee on the
    // dashboard wire shape.
    assert_eq!(providers[0]["provider"], "anthropic");
    assert_eq!(providers[0]["unconfigured"], true);
    assert!((providers[0]["spend_hourly_usd"].as_f64().unwrap() - 0.1).abs() < 1e-9);
    assert_eq!(providers[0]["tokens_this_hour"], 2000);
    // No cap configured → all four cap fields render as 0 (unlimited).
    assert_eq!(providers[0]["cap_hourly_usd"], 0.0);
    assert_eq!(providers[0]["cap_tokens_per_hour"], 0);

    assert_eq!(providers[1]["provider"], "openai");
    assert_eq!(providers[1]["unconfigured"], false);
    assert!((providers[1]["spend_hourly_usd"].as_f64().unwrap() - 0.5).abs() < 1e-9);
    assert_eq!(providers[1]["tokens_this_hour"], 3000);
}

// ---------------------------------------------------------------------------
// PUT /api/budget/providers/{provider_id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn provider_put_creates_new_entry_and_round_trips_via_get() {
    let h = boot().await;
    // Mutate a NEW provider id so we exercise the insert branch.
    let (status, body) = request(
        &h,
        Method::PUT,
        "/api/budget/providers/groq",
        Some(serde_json::json!({
            "max_cost_per_hour_usd": 2.5,
            "max_cost_per_day_usd": 25.0,
            "max_cost_per_month_usd": 250.0,
            "max_tokens_per_hour": 500_000,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["provider"], "groq");
    assert!((body["max_cost_per_hour_usd"].as_f64().unwrap() - 2.5).abs() < f64::EPSILON);
    assert_eq!(body["max_tokens_per_hour"], 500_000);

    // In-memory snapshot reflects the new entry — proves the
    // `HotAction::UpdateBudget` reload fired and the metering engine
    // sees the cap without a daemon restart.
    let live = h.state.kernel.budget_config();
    let entry = live.providers.get("groq").expect("groq entry persisted");
    assert!((entry.max_cost_per_hour_usd - 2.5).abs() < f64::EPSILON);
    assert!((entry.max_cost_per_day_usd - 25.0).abs() < f64::EPSILON);
    assert!((entry.max_cost_per_month_usd - 250.0).abs() < f64::EPSILON);
    assert_eq!(entry.max_tokens_per_hour, 500_000);
    // Seed provider entry was preserved (we only swapped/inserted one row).
    let openai = live.providers.get("openai").expect("seed survives PUT");
    assert!((openai.max_cost_per_hour_usd - 1.0).abs() < f64::EPSILON);

    // GET reflects the new row alongside the seed entry, still sorted.
    let (_, body) = request(&h, Method::GET, "/api/budget/providers", None).await;
    let providers = body["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0]["provider"], "groq");
    assert_eq!(providers[1]["provider"], "openai");
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_put_partial_body_keeps_prior_values() {
    let h = boot().await;
    // Seed `openai` already has hourly=1.0, daily=100.0. Send a PUT that
    // only carries the monthly cap — the unset fields must retain the
    // prior values, mirroring the partial-PUT contract on the global
    // budget handler.
    let (status, body) = request(
        &h,
        Method::PUT,
        "/api/budget/providers/openai",
        Some(serde_json::json!({ "max_cost_per_month_usd": 500.0 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let live = h.state.kernel.budget_config();
    let entry = live.providers.get("openai").unwrap();
    assert!((entry.max_cost_per_hour_usd - 1.0).abs() < f64::EPSILON);
    assert!((entry.max_cost_per_day_usd - 100.0).abs() < f64::EPSILON);
    assert!((entry.max_cost_per_month_usd - 500.0).abs() < f64::EPSILON);
    assert_eq!(entry.max_tokens_per_hour, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_put_rejects_negative_or_nan_caps() {
    let h = boot().await;
    for bad in [
        serde_json::json!({ "max_cost_per_hour_usd": -1.0 }),
        serde_json::json!({ "max_cost_per_day_usd": "not a number" }),
    ] {
        let (status, body) = request(
            &h,
            Method::PUT,
            "/api/budget/providers/openai",
            Some(bad.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?} -> {body:?}");
    }
    // Live config snapshot must be untouched after the failed PUTs.
    let live = h.state.kernel.budget_config();
    let entry = live.providers.get("openai").unwrap();
    assert!((entry.max_cost_per_hour_usd - 1.0).abs() < f64::EPSILON);
    assert!((entry.max_cost_per_day_usd - 100.0).abs() < f64::EPSILON);
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_put_rejects_blank_provider_id() {
    let h = boot().await;
    // axum routing strips whitespace before matching path captures, so
    // a `%20`-only id slips through to the handler — that's exactly the
    // case the handler-level guard rejects with 400 so we never burn a
    // useless `[budget.providers." "]` row into config.toml.
    let (status, body) = request(
        &h,
        Method::PUT,
        "/api/budget/providers/%20",
        Some(serde_json::json!({"max_cost_per_hour_usd": 1.0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}
