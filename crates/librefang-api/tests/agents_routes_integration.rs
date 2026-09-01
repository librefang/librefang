//! Integration tests for the `/api/agents` route family.
//!
//! Refs #3571 — agents-domain slice. These tests exercise the production
//! router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the
//! real auth middleware, route registration, and handler logic are all in
//! play. No real LLM calls (provider is `ollama` with a fake model) — every
//! test is hermetic.
//!
//! Routes covered:
//!   GET   /api/agents              (list — empty filter + populated)
//!   GET   /api/agents/{id}         (happy path + invalid id 400 + unknown 404)
//!   PATCH /api/agents/{id}         (success, invalid payload, unknown 404,
//!                                   read-after-write via GET, auth gate 401)
//!   PUT   /api/agents/{id}/suspend (suspend → state Suspended, unknown 404,
//!                                   invalid id 400)
//!   PUT   /api/agents/{id}/resume  (resume → state Running, unknown 404)
//!   PUT   /api/agents/{id}/mode    (mode change persisted + read-after-write,
//!                                   unknown 404, invalid id 400)
//!   PATCH /api/agents/{id}/identity (merge semantics: a partial body preserves
//!                                   the fields it omits, empty string clears,
//!                                   and the result matches PATCH /config for
//!                                   the same body — refs #6608)
//!   PUT   /api/agents/{id}/mcp_servers (an inherited declaration this instance
//!                                   has not installed saves rather than 400s,
//!                                   a genuinely unknown name is still rejected
//!                                   without "Internal error" wording — #7772)
//!   PUT   /api/agents/{id}/skills  (an on-disk-but-unloaded skill saves under
//!                                   its `[skill].name`, not its directory name
//!                                   — #7772)
//!   POST  /api/agents/{id}/message  (`reasoning_mode` deserializes into the real
//!                                   enum on both the buffered and streaming
//!                                   routes, an invalid mode is rejected rather
//!                                   than silently ignored, and the legacy
//!                                   `thinking` boolean keeps working — #7946)
//!
//! Run: cargo test -p librefang-api --test agents_routes_integration

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness — boots the production router with a configurable api_key.
// ---------------------------------------------------------------------------

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot(api_key: &str) -> Harness {
    boot_with_mcp_servers(api_key, Vec::new()).await
}

async fn boot_with_mcp_servers(
    api_key: &str,
    mcp_servers: Vec<librefang_types::config::McpServerConfigEntry>,
) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Seed the pinned registry fixture so the kernel boots with content, offline.
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        mcp_servers,
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
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

/// Bearer token used by all authenticated test requests. Every harness
/// (except the explicit auth-gate test) boots with this api_key so the
/// production middleware accepts the requests as authenticated.
const TEST_TOKEN: &str = "test-secret";

fn get(path: &str) -> Request<Body> {
    get_with(path, Some(TEST_TOKEN))
}

fn get_with(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

fn patch_json(path: &str, body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// PUT with no body — used by the suspend/resume lifecycle routes, which
/// take only the `{id}` path param.
fn put_empty(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::PUT).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

/// PUT with a JSON body — used by the `/mode` lifecycle route.
fn put_json(path: &str, body: serde_json::Value, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/agents
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_default_assistant_only() {
    // The kernel auto-spawns a single default assistant on boot — so the
    // "empty user-spawn" baseline is exactly one entry. We further filter by
    // a unique q= to assert the empty case truly returns zero matches.
    let h = boot(TEST_TOKEN).await;

    let (status, body) = send(
        h.app.clone(),
        get("/api/agents?q=__definitely_no_such_agent__"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert!(
        items.is_empty(),
        "expected empty filter result, got {:?}",
        items
    );
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_spawned_agents() {
    let h = boot(TEST_TOKEN).await;
    let id_a = spawn_named(&h.state, "alpha-agent");
    let id_b = spawn_named(&h.state, "beta-agent");

    let (status, body) = send(h.app.clone(), get("/api/agents")).await;
    assert_eq!(status, StatusCode::OK);

    let items = body["items"].as_array().expect("items array");
    let ids: Vec<String> = items
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&id_a.to_string()), "missing alpha: {:?}", ids);
    assert!(ids.contains(&id_b.to_string()), "missing beta: {:?}", ids);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_rejects_invalid_sort_field() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents?sort=not_a_field")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_happy_path() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "lookup-target");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id.to_string());
    assert_eq!(body["name"], "lookup-target");
    assert!(body["model"].is_object());
    assert!(body["capabilities"].is_object());
}

/// #6565: a consumer deciding whether an MCP server is reachable needs both halves of the kernel's gate (`!mcp_disabled && !mcp_servers.is_empty()`).
/// `mcp_servers` / `mcp_servers_mode` were already emitted; `mcp_disabled` was not, so `mcp_servers = ["*"]` on a disabled agent looked like a live grant.
/// `injected_footprint_tokens` is the static cost every request to this
/// agent carries before a single user message — system prompt plus tool
/// definitions. A longer system prompt must estimate to more tokens than a
/// short one, and the value must not be a fixed placeholder.
#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_reports_injected_footprint_tokens_from_system_prompt() {
    let h = boot(TEST_TOKEN).await;
    let short_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "footprint-short".to_string(),
            source_template: None,
            ..AgentManifest::default()
        })
        .expect("spawn_agent");
    let long_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "footprint-long".to_string(),
            source_template: None,
            model: librefang_types::agent::ModelConfig {
                system_prompt: "You are a helpful AI agent. ".repeat(200),
                ..Default::default()
            },
            ..AgentManifest::default()
        })
        .expect("spawn_agent");

    let (status, short_body) = send(h.app.clone(), get(&format!("/api/agents/{}", short_id))).await;
    assert_eq!(status, StatusCode::OK);
    let short_tokens = short_body["injected_footprint_tokens"]
        .as_u64()
        .expect("injected_footprint_tokens must be a number");
    assert!(short_tokens > 0);

    let (status, long_body) = send(h.app.clone(), get(&format!("/api/agents/{}", long_id))).await;
    assert_eq!(status, StatusCode::OK);
    let long_tokens = long_body["injected_footprint_tokens"]
        .as_u64()
        .expect("injected_footprint_tokens must be a number");
    assert!(
        long_tokens > short_tokens,
        "a longer system prompt must estimate to more tokens: long={long_tokens} short={short_tokens}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_exposes_the_full_mcp_grant_state_6565() {
    let h = boot(TEST_TOKEN).await;
    let manifest = AgentManifest {
        name: "mcp-granted".to_string(),
        source_template: None,
        mcp_servers: vec!["server-x".to_string()],
        ..AgentManifest::default()
    };
    let id = h
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mcp_servers"], serde_json::json!(["server-x"]));
    assert_eq!(body["mcp_servers_mode"], "allowlist");
    assert_eq!(
        body["mcp_disabled"], false,
        "mcp_disabled must be present, not absent: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_reports_mcp_disabled_even_with_a_wildcard_grant_6565() {
    let h = boot(TEST_TOKEN).await;
    let manifest = AgentManifest {
        name: "mcp-off".to_string(),
        source_template: None,
        // The combination that used to read as "all servers granted".
        mcp_servers: vec!["*".to_string()],
        mcp_disabled: true,
        ..AgentManifest::default()
    };
    let id = h
        .state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mcp_servers_mode"], "all");
    assert_eq!(
        body["mcp_disabled"], true,
        "the hard off switch must be visible alongside the wildcard: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(h.app.clone(), get("/api/agents/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_agent_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", unknown))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "agent_not_found");
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_name_and_description() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "patch-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({
                "name": "renamed-agent",
                "description": "updated via PATCH"
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read-after-write — GET should reflect the new name + description.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed-agent");
    assert_eq!(body["description"], "updated via PATCH");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_mcp_servers_payload_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "bad-payload");

    // mcp_servers must be an array of strings; nested objects are rejected.
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"mcp_servers": [{"oops": true}]}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_unknown_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", unknown),
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            "/api/agents/not-a-uuid",
            serde_json::json!({"name": "anything"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Auth gate — PATCH is a mutation, NOT in PUBLIC_ROUTES_DASHBOARD_READS, so
// once an api_key is configured a non-loopback request without a Bearer
// token must be rejected with 401. (oneshot has no ConnectInfo, so the
// loopback fast-path does NOT apply — the request is treated as remote.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_without_token_returns_401_when_api_key_set() {
    let h = boot("test-secret").await;
    let id = spawn_named(&h.state, "auth-gated");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "should-not-apply"}),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Sanity: with the correct Bearer token the same request succeeds.
    let (status_ok, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "did-apply"}),
            Some("test-secret"),
        ),
    )
    .await;
    assert_eq!(status_ok, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id} — schedule field (#4984 / #4986)
//
// Refs the linked issue: the dashboard's Schedule tab toggle PATCHed the
// `schedule` field, but the partial-update handler silently dropped it.
// These tests pin the contract so a future refactor of `patch_agent`
// cannot regress the same way.
// ---------------------------------------------------------------------------

/// Reactive happy path — set schedule to `"reactive"` and confirm the GET
/// response reflects it. `format_schedule_mode` renders Reactive as
/// `"manual"`, which is the dashboard-facing string and the read-after-
/// write assertion this test pins.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_schedule_to_reactive() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-reactive-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"schedule": "reactive"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    // ScheduleMode::Reactive renders as "manual" in the dashboard payload
    // (see `format_schedule_mode`); this assertion pins that contract.
    assert_eq!(body["schedule"], "manual", "body={body:?}");
}

/// Continuous schedule with explicit `check_interval_secs` — the snake-case
/// JSON shape (`{"continuous":{"check_interval_secs":N}}`) is what the
/// dashboard's payload normalizer is supposed to emit, and is the same
/// shape `ScheduleMode` derives via `#[serde(rename_all = "snake_case")]`.
/// Pin both the wire format AND the read-after-write side effect: GET
/// must now report the formatted continuous string.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_schedule_to_continuous() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-continuous-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({
                "schedule": {"continuous": {"check_interval_secs": 120}}
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["schedule"], "continuous · 120s", "body={body:?}");
}

/// Periodic schedule — covers the third non-Reactive variant so the test
/// matrix isn't strictly Reactive ↔ Continuous (which would let a regression
/// that only affects Periodic / Proactive land silently).
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_updates_schedule_to_periodic() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-periodic-target");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"schedule": {"periodic": {"cron": "*/15 * * * *"}}}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    // ScheduleMode::Periodic { cron } renders as the cron expression itself
    // (see `format_schedule_mode`); the dashboard shows it verbatim.
    assert_eq!(body["schedule"], "*/15 * * * *", "body={body:?}");
}

/// Malformed schedule — string that isn't a known variant must be rejected
/// with 400, not silently coerced. Pinning this prevents the
/// dashboard-currently-sends-`"manual"` case from quietly succeeding the
/// next time someone adds a permissive `serde(other)` fallback to
/// `ScheduleMode`.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_rejects_invalid_schedule_string() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-bad-string");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            // `"manual"` is the dashboard display string — there's no
            // `Manual` variant on `ScheduleMode`. The dashboard must
            // alias it to `"reactive"` on the wire; if it ever stops,
            // this test catches the regression at the API layer.
            serde_json::json!({"schedule": "manual"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(body["error"].is_string());
}

/// Malformed schedule payload — wrong inner shape (`continuous` without the
/// nested object) must be rejected with 400. Distinct from the unknown-
/// variant case above so a regression in either path surfaces on its own.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_rejects_malformed_schedule_payload() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-bad-shape");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            // `continuous` is a struct variant — passing a bare string
            // is a serde shape error, not an unknown variant.
            serde_json::json!({"schedule": "continuous"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert!(body["error"].is_string());
}

/// Schedule field absent → existing schedule must be preserved. PATCH is
/// a partial update, so a name-only PATCH should not touch the schedule.
/// This pins the "unrelated field updates leave schedule alone" guarantee.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_without_schedule_field_preserves_schedule() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-untouched-target");

    // First, set schedule to a non-default value so "untouched" is observable.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"schedule": {"continuous": {"check_interval_secs": 60}}}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Then, PATCH a different field. Schedule must NOT revert to default.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"name": "schedule-untouched-renamed"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "schedule-untouched-renamed");
    assert_eq!(body["schedule"], "continuous · 60s", "body={body:?}");
}

/// Refs #4984: PATCH from Reactive → Continuous must start the background
/// loop immediately, and PATCH from Continuous → Reactive must stop it.
/// Previously the registry was updated but `start_background_for_agent` /
/// `stop_agent` were never called, so the runtime kept running whatever
/// schedule was active at daemon start until restart.
///
/// We assert against the kernel's `background.active_count()` (via the
/// kernel handle on `AppState`) rather than waiting for a tick to fire,
/// because the test harness uses fake LLM models and `tokio::test` runs
/// don't sleep long enough for the jitter-delayed first tick anyway.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_schedule_starts_and_stops_background_loop() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "schedule-runtime-target");

    // Newly spawned agent defaults to Reactive — no background loop.
    let baseline_count = h.state.kernel.background_active_count();

    // Reactive → Continuous: a new loop must register.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"schedule": {"continuous": {"check_interval_secs": 3600}}}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_start = h.state.kernel.background_active_count();
    assert_eq!(
        after_start,
        baseline_count + 1,
        "Reactive→Continuous PATCH must start the background loop (was {baseline_count}, now {after_start})"
    );

    // Continuous → Reactive: the loop must be stopped.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"schedule": "reactive"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after_stop = h.state.kernel.background_active_count();
    assert_eq!(
        after_stop, baseline_count,
        "Continuous→Reactive PATCH must stop the background loop (was {after_start}, now {after_stop})"
    );
}

// ---------------------------------------------------------------------------
// DELETE /api/agents/{id} — idempotency (#3509)
// ---------------------------------------------------------------------------

fn delete(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(Method::DELETE).uri(path);
    if let Some(token) = bearer {
        b = b.header("authorization", format!("Bearer {}", token));
    }
    b.body(Body::empty()).unwrap()
}

/// Refs #4614 — DELETE /api/agents/{id} requires `?confirm=true` (or a
/// `{"confirm": true}` body) to gate the destructive canonical-UUID
/// purge. This helper appends the query param so tests don't have to
/// open-code the URL in every call site.
fn delete_confirmed(path: &str, bearer: Option<&str>) -> Request<Body> {
    let glue = if path.contains('?') { '&' } else { '?' };
    let with_confirm = format!("{path}{glue}confirm=true");
    delete(&with_confirm, bearer)
}

/// Refs #3509: DELETE is idempotent (RFC 9110 §9.2.2). Killing the same
/// agent twice MUST succeed both times — the second call returns
/// `200 OK` with `status: already-deleted` instead of `404 Not Found`,
/// so clients (dashboard double-clicks, CLI retries, network-recovery
/// loops) never see a phantom error for an outcome that already matches
/// their intent.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_twice_both_succeed_idempotent() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "kill-target");

    // First call — agent exists, normal kill path. Refs #4614: confirm
    // required to gate canonical-UUID purge.
    let (status1, body1) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status1,
        StatusCode::OK,
        "first DELETE should be 200; body={body1:?}"
    );
    assert_eq!(body1["status"], "killed", "first DELETE body={body1:?}");

    // Second call — agent already gone. MUST still be 200, not 404.
    let (status2, body2) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "second DELETE on a now-absent agent must be idempotent-200 (#3509); got {status2} body={body2:?}"
    );
    assert_eq!(
        body2["status"], "already-deleted",
        "second DELETE body={body2:?}"
    );
}

/// Refs #3509: 400 stays reserved for malformed-id rejection. Only the
/// `not-found` case relaxed to 200 idempotent. Without this the relaxation
/// could mask genuine client bugs (typo'd id, wrong path).
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_invalid_id_still_returns_400() {
    let h = boot(TEST_TOKEN).await;
    // Bare DELETE — malformed UUID short-circuits with 400 before the
    // confirmation check fires, so the response stays the same shape
    // post-#4614.
    let (status, body) = send(
        h.app.clone(),
        delete("/api/agents/not-a-uuid", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert_eq!(body["code"], "invalid_agent_id");
}

/// Refs #3509: deleting an unknown-but-well-formed UUID is idempotent —
/// no agent existed under that id, so the caller's intent ("agent {id}
/// should be gone") is already satisfied. 200 with `already-deleted` lets
/// idempotent clients (Terraform-style reconcilers) skip the dance.
#[tokio::test(flavor = "multi_thread")]
async fn test_delete_agent_unknown_uuid_is_idempotent_200() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    // Refs #4614: confirm required even on the idempotent-already-gone
    // path so the contract is consistent across all DELETEs.
    let (status, body) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", unknown), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["status"], "already-deleted", "body={body:?}");
}

// ---------------------------------------------------------------------------
// POST /api/agents — re-create an agent with a previously-used name (#4991)
// ---------------------------------------------------------------------------

/// Refs #4991: deleting an agent and immediately re-creating one with the
/// same name used to fail with `500 Internal error: Invalid workspace path`.
///
/// Root cause: `spawn_agent_inner` rewrites `manifest.workspace` to the
/// resolved absolute directory (`<workspaces>/agents/<name>`). The manifest
/// is round-tripped to `agent.toml` on disk; subsequent re-creation feeds
/// that absolute path back into `resolve_workspace_dir`, which blanket-
/// rejected anything `is_absolute()` — even when it pointed inside the
/// workspaces root the helper would have produced itself.
///
/// The fix accepts absolute paths under `workspaces_root` while still
/// rejecting paths outside it (and any `..` / Windows-prefix traversal).
#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_agent_with_absolute_workspace_inside_root_succeeds() {
    let h = boot(TEST_TOKEN).await;

    // Compute the absolute workspace path the kernel itself would assign
    // to an agent named "recreate-me". This mirrors what
    // `persist_manifest_to_disk` writes back into `agent.toml` after a
    // successful spawn — and what gets fed into a subsequent re-spawn.
    let cfg = h.state.kernel.config_ref();
    let abs_workspace = cfg.effective_agent_workspaces_dir().join("recreate-me");

    let manifest_toml = format!(
        r#"
name = "recreate-me"
description = "agent with absolute workspace path"
workspace = "{}"

[model]
provider = "ollama"
model = "test-model"
"#,
        // TOML basic-string escape: backslashes (Windows paths) must be
        // doubled so the parser sees a literal path component.
        abs_workspace.display().to_string().replace('\\', "\\\\"),
    );

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "spawn with absolute workspace inside root must succeed; body={body:?}"
    );
    assert_eq!(body["name"], "recreate-me", "body={body:?}");
}

/// Refs #4991: the full delete-then-recreate flow the issue reporter hit.
/// Spawn → confirmed DELETE → spawn again under the same name with the
/// manifest the kernel persisted on the first spawn (i.e. carrying the
/// resolved absolute workspace path). Without the fix the second spawn
/// returns `500 spawn_failed / api-error-agent-error`.
#[tokio::test(flavor = "multi_thread")]
async fn test_recreate_agent_same_name_after_delete_succeeds() {
    let h = boot(TEST_TOKEN).await;

    // First spawn — let the kernel synthesize the absolute workspace dir,
    // then read it back from the registry exactly as
    // `persist_manifest_to_disk` would serialize it.
    let id1 = spawn_named(&h.state, "recreated");
    let resolved_workspace = h
        .state
        .kernel
        .agent_registry()
        .get(id1)
        .and_then(|e| e.manifest.workspace.clone())
        .expect("spawn must have set manifest.workspace to an absolute path");
    assert!(
        resolved_workspace.is_absolute(),
        "first spawn should resolve workspace to an absolute path; got {}",
        resolved_workspace.display()
    );

    // Confirmed DELETE — purges canonical UUID, drops registry entry,
    // but leaves the workspace directory on disk (kill_agent does NOT
    // remove the workspace; the reporter's repro relies on this).
    let (del_status, del_body) = send(
        h.app.clone(),
        delete_confirmed(&format!("/api/agents/{}", id1), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(
        del_status,
        StatusCode::OK,
        "DELETE must succeed; body={del_body:?}"
    );
    assert_eq!(del_body["status"], "killed", "body={del_body:?}");

    // Second spawn — same name, carrying the absolute workspace that the
    // first spawn would have written back to agent.toml. This is exactly
    // what the dashboard form (or template instantiation) replays.
    let manifest_toml = format!(
        r#"
name = "recreated"
workspace = "{}"

[model]
provider = "ollama"
model = "test-model"
"#,
        resolved_workspace
            .display()
            .to_string()
            .replace('\\', "\\\\"),
    );
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "recreate after delete must succeed; got {status} body={body:?}"
    );
    assert_eq!(body["name"], "recreated", "body={body:?}");
    // AgentId is a deterministic UUIDv5 derived from the agent name
    // (`AgentId::new_for_name`), so the recreated agent intentionally
    // shares its UUID with the deleted one. We only assert the response
    // carries an agent_id at all — confirming the second spawn went
    // through the full code path rather than returning a stale 200 from
    // the idempotency cache.
    let id2 = body["agent_id"]
        .as_str()
        .expect("recreate response must carry the new agent id");
    assert_eq!(
        id2,
        id1.to_string(),
        "AgentId is name-deterministic; recreated UUID must match the original"
    );
}

/// Refs #4991: the security boundary the original blanket `is_absolute()`
/// reject was protecting must stay closed. An absolute path pointing
/// **outside** `workspaces_root` is still rejected.
#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_agent_with_absolute_workspace_outside_root_rejected() {
    let h = boot(TEST_TOKEN).await;

    // `/tmp/...` (or `C:\...` on Windows) is always outside the per-test
    // tempdir workspaces root. Use a platform-appropriate absolute path
    // so the test is meaningful on both Unix and Windows runners.
    #[cfg(unix)]
    let outside = "/tmp/librefang-4991-outside";
    #[cfg(windows)]
    let outside = "C:\\librefang-4991-outside";

    let manifest_toml = format!(
        r#"
name = "escape-me"
workspace = "{}"

[model]
provider = "ollama"
model = "test-model"
"#,
        outside.replace('\\', "\\\\"),
    );

    let (status, _body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "absolute workspace outside the workspaces root must still be rejected"
    );
}

/// Refs #4991: even when the absolute path's prefix matches `workspaces_root`,
/// a `..` component anywhere in the path must still be rejected. Without this
/// guard, `<workspaces_root>/x/../../escape` would syntactically `starts_with`
/// the root yet resolve outside it once the OS canonicalizes the path.
/// `has_unsafe_relative_components` runs *before* the `is_absolute()` branch
/// in `resolve_workspace_dir`, so any `ParentDir` component fails closed
/// regardless of which branch would otherwise accept the input.
#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_agent_with_absolute_workspace_dotdot_traversal_rejected() {
    let h = boot(TEST_TOKEN).await;

    // Build an absolute path that starts with the real workspaces root but
    // contains a `..` segment further down. Without the unsafe-component
    // check this would slip past the `starts_with(&root)` branch.
    let cfg = h.state.kernel.config_ref();
    let root = cfg.effective_agent_workspaces_dir();
    let traversal = root.join("x").join("..").join("escape");

    let manifest_toml = format!(
        r#"
name = "dotdot-escape"
workspace = "{}"

[model]
provider = "ollama"
model = "test-model"
"#,
        traversal.display().to_string().replace('\\', "\\\\"),
    );

    let (status, _body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "absolute workspace with `..` traversal must be rejected even when the prefix matches workspaces_root"
    );
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/session — thinking blocks reach the dashboard
// ---------------------------------------------------------------------------

/// Persisted `ContentBlock::Thinking` blocks must be surfaced on the
/// agent-scoped session endpoint so the dashboard can render the
/// collapsible reasoning drawer on history reload — same UX as live
/// streaming, where `thinking_delta` events accumulate into the message.
///
/// Before this fix the endpoint flattened blocks into a string and silently
/// swallowed Thinking via the catch-all match arm, so reload showed an
/// assistant turn with no reasoning even though the session JSON had it.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_surfaces_thinking_blocks() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "thinking-target");

    // Seed a session with an assistant turn that has interleaved thinking
    // and text blocks. Two thinking blocks exercise the multi-block join.
    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    session.push_message(Message {
        role: Role::User,
        content: MessageContent::Text("hi".to_string()),
        pinned: false,
        timestamp: None,
    });
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![
            ContentBlock::Thinking {
                thinking: "first reasoning step".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Text {
                text: "visible answer".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Thinking {
                thinking: "follow-up reasoning".to_string(),
                provider_metadata: None,
            },
        ]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array").clone();
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("assistant message");
    // Visible text still flattens — same shape the dashboard already
    // rendered before this change.
    assert_eq!(assistant["content"], "visible answer");
    // Thinking now surfaces as a flat string with multi-block join. The
    // dashboard's history mapper reads this directly into
    // `ChatMessage.thinking`, mirroring the live-streaming flat-string
    // accumulation from `thinking_delta` events.
    assert_eq!(
        assistant["thinking"], "first reasoning step\n\nfollow-up reasoning",
        "thinking field missing or wrong join — body={body:?}",
    );
}

/// Sessions without thinking blocks must NOT include a `thinking` field
/// on assistant messages. Omitting (vs. emitting `""`) keeps the response
/// shape unchanged for non-thinking models and avoids triggering the
/// dashboard's empty-drawer render gate.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_omits_thinking_when_none_present() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "no-thinking-target");

    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::Text {
            text: "plain answer".to_string(),
            provider_metadata: None,
        }]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array");
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("assistant message");
    assert_eq!(assistant["content"], "plain answer");
    assert!(
        assistant.get("thinking").is_none(),
        "thinking field should be absent — body={body:?}",
    );
}

/// A turn whose `MessageContent::Blocks` contains ONLY `Thinking`
/// (e.g. an aborted/cancelled response, or a server filter that
/// stripped the visible text) MUST still surface to the dashboard so
/// the collapsible thinking drawer renders. Pre-fix the route's
/// `if content.is_empty() && tools.is_empty()` early-skip dropped the
/// turn before the `thinking` field was attached, contradicting the
/// dashboard's `hasThinking` render branch which is explicitly
/// designed for thinking-only turns.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_endpoint_surfaces_thinking_only_turns() {
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "thinking-only-target");

    let mut session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(id)
        .expect("create_session");
    // Seed a user prompt followed by an assistant turn with NO text /
    // tool_use — only Thinking. Mirrors a cancelled-mid-stream
    // response that produced reasoning before the visible answer
    // started.
    session.push_message(Message {
        role: Role::User,
        content: MessageContent::Text("hi".to_string()),
        pinned: false,
        timestamp: None,
    });
    session.push_message(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::Thinking {
            thinking: "reasoning that never reached an answer".to_string(),
            provider_metadata: None,
        }]),
        pinned: false,
        timestamp: None,
    });
    let session_id = session.id.0;
    h.state
        .kernel
        .memory_substrate()
        .save_session(&session)
        .expect("save_session");

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session?session_id={}",
            id, session_id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let messages = body["messages"].as_array().expect("messages array").clone();
    let assistant = messages
        .iter()
        .find(|m| m["role"] == "Assistant")
        .expect("thinking-only assistant turn must NOT be dropped — body={body:?}");
    assert_eq!(
        assistant["content"], "",
        "thinking-only turn has no visible text — body={body:?}",
    );
    assert_eq!(
        assistant["thinking"], "reasoning that never reached an answer",
        "thinking field must surface so the dashboard's hasThinking branch can render — body={body:?}",
    );
}

// ---------------------------------------------------------------------------
// Incognito mode — refs #4073
// ---------------------------------------------------------------------------

/// The `incognito` field in the POST /api/agents/{id}/message body must
/// deserialize cleanly. A request with `incognito: true` must not return a
/// 422 Unprocessable Entity; if the provider auth is missing (the test
/// harness uses a fake ollama model) the server returns 412 as usual.
/// This verifies the API surface is wired end-to-end without a real LLM.
#[tokio::test(flavor = "multi_thread")]
async fn test_incognito_field_accepted_by_message_endpoint() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "incognito-test-agent");

    // incognito: true — must NOT be 422 (unknown field / bad deserialize)
    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message"),
            serde_json::json!({"message": "hello", "incognito": true}),
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "incognito field must deserialize cleanly — body={body:?}",
    );
    // Provider is unconfigured → 412 or 500, NOT 422.
    assert!(
        status == StatusCode::PRECONDITION_FAILED || status.is_server_error(),
        "expected provider-auth 412 or server error, got {status} — body={body:?}",
    );
}

/// Omitting `incognito` entirely must still work (backward compat: defaults to false).
#[tokio::test(flavor = "multi_thread")]
async fn test_incognito_defaults_to_false_when_omitted() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "incognito-omit-agent");

    let (status, _body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message"),
            serde_json::json!({"message": "hello"}),
        ),
    )
    .await;
    // Must not be 422 — the field absence defaults to false via #[serde(default)].
    assert_ne!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ---------------------------------------------------------------------------
// Per-task reasoning mode — refs #7946
// ---------------------------------------------------------------------------

/// `reasoning_mode` on the POST /api/agents/{id}/message body must deserialize
/// into the real enum, not be silently ignored.
///
/// The positive half of this test proves little on its own: `MessageRequest`
/// does not deny unknown fields, so a *typo'd* key is accepted with the same
/// status. The conclusive assertion is the negative one below it — an invalid
/// mode string can only be rejected if serde is actually parsing the field into
/// `ReasoningMode`, which is exactly the "field added to the struct but never
/// deserialized" failure this suite exists to catch.
#[tokio::test(flavor = "multi_thread")]
async fn test_reasoning_mode_field_accepted_by_message_endpoint() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "reasoning-mode-agent");

    for mode in ["none", "low", "high", "max"] {
        let (status, body) = send(
            h.app.clone(),
            post_json(
                &format!("/api/agents/{id}/message"),
                serde_json::json!({"message": "hello", "reasoning_mode": mode}),
            ),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "reasoning_mode={mode:?} must deserialize cleanly — body={body:?}",
        );
        // Provider is unconfigured → 412 or 500, NOT 422.
        assert!(
            status == StatusCode::PRECONDITION_FAILED || status.is_server_error(),
            "reasoning_mode={mode:?}: expected provider-auth 412 or server error, got {status} — body={body:?}",
        );
    }
}

/// An unrecognised mode must be rejected rather than quietly falling back to the
/// agent default — a silent fallback is how a typo'd `reasoning_mode` in a
/// caller's config ends up billing reasoning tokens nobody asked for.
#[tokio::test(flavor = "multi_thread")]
async fn test_invalid_reasoning_mode_is_rejected() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "reasoning-mode-invalid-agent");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message"),
            serde_json::json!({"message": "hello", "reasoning_mode": "banana"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an invalid reasoning_mode must be a deserialize error — body={body:?}",
    );
}

/// Omitting `reasoning_mode` must keep working: every existing client sends a
/// body without it, and `thinking: bool` remains its own supported override.
#[tokio::test(flavor = "multi_thread")]
async fn test_reasoning_mode_omitted_and_legacy_thinking_boolean_still_work() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "reasoning-mode-compat-agent");

    for body_json in [
        serde_json::json!({"message": "hello"}),
        serde_json::json!({"message": "hello", "thinking": true}),
        serde_json::json!({"message": "hello", "thinking": false}),
        // Both keys at once: documented precedence is reasoning_mode wins.
        serde_json::json!({"message": "hello", "thinking": false, "reasoning_mode": "max"}),
    ] {
        let (status, body) = send(
            h.app.clone(),
            post_json(&format!("/api/agents/{id}/message"), body_json.clone()),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "body {body_json} must deserialize cleanly — response={body:?}",
        );
    }
}

/// The streaming twin deserializes the same `MessageRequest`, so it must accept
/// the same key. Before #7946 this handler dropped the per-call thinking
/// override on the floor entirely — it passed a hardcoded `None` into the
/// kernel — so a per-task mode that only worked on the non-streaming route
/// would be invisible to any UI that streams.
#[tokio::test(flavor = "multi_thread")]
async fn test_reasoning_mode_accepted_by_streaming_message_endpoint() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "reasoning-mode-stream-agent");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message/stream"),
            serde_json::json!({"message": "hello", "reasoning_mode": "low"}),
        ),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "reasoning_mode must deserialize on the streaming route too — body={body:?}",
    );

    let (status, body) = send(
        h.app.clone(),
        post_json(
            &format!("/api/agents/{id}/message/stream"),
            serde_json::json!({"message": "hello", "reasoning_mode": "banana"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an invalid reasoning_mode must be rejected on the streaming route — body={body:?}",
    );
}

/// The per-call override the handler builds from the request body — the
/// injection site, not the implementation site. A `reasoning_mode` field that
/// exists on `MessageRequest` but is never folded into a `ThinkingOverride`
/// compiles and deserializes while doing nothing, which is the #7946 shape of
/// CLAUDE.md's "`Option::None` defaults compile silently but disable the
/// feature" warning.
#[test]
fn test_per_call_reasoning_mode_wins_over_the_legacy_boolean() {
    use librefang_types::config::{ReasoningMode, ThinkingOverride};

    // Exactly the expression both message handlers use.
    let resolve = |json: serde_json::Value| {
        let req: librefang_api::types::MessageRequest =
            serde_json::from_value(json).expect("deserialize MessageRequest");
        ThinkingOverride::resolve(req.thinking, req.reasoning_mode)
    };

    assert_eq!(
        resolve(serde_json::json!({"message": "x"})),
        ThinkingOverride::Inherit,
    );
    assert_eq!(
        resolve(serde_json::json!({"message": "x", "thinking": true})),
        ThinkingOverride::Enable,
    );
    assert_eq!(
        resolve(serde_json::json!({"message": "x", "thinking": false})),
        ThinkingOverride::Disable,
    );
    assert_eq!(
        resolve(serde_json::json!({"message": "x", "reasoning_mode": "max"})),
        ThinkingOverride::Mode(ReasoningMode::Max),
    );
    // Both present: the more specific key wins.
    assert_eq!(
        resolve(serde_json::json!({"message": "x", "thinking": true, "reasoning_mode": "none"})),
        ThinkingOverride::Mode(ReasoningMode::None),
    );
    assert_eq!(
        resolve(serde_json::json!({"message": "x", "thinking": false, "reasoning_mode": "high"})),
        ThinkingOverride::Mode(ReasoningMode::High),
    );
}

// The actual persistence-guard assertion lives in
// `librefang-runtime/src/agent_loop.rs`'s `#[cfg(test)] mod tests` as
// `test_incognito_skips_session_save_on_end_turn` (with a positive control
// `test_normal_turn_persists_session_as_incognito_control` next to it).
//
// Driving a real end-turn through this integration-test surface requires
// a mock LLM driver wired through `MockKernelBuilder` — but the kernel
// resolves drivers from the agent manifest (`provider`/`model` lookup),
// and `MockKernelBuilder` does not yet expose a driver-injection hook.
// Without that hook the LLM call fails before reaching any
// `save_session_async` site, so the test's premise (compare pre-call vs
// post-call message counts) is true by default whether or not the
// `incognito` guard is wired in. The two runtime-level tests exercise
// the `LoopOptions::incognito` guard at `finalize_successful_end_turn`
// end-to-end against a `NormalDriver` canned response, which is the
// minimum needed to actually verify the persistence-skip semantics.

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/session — compacted_summary field (#5202)
// ---------------------------------------------------------------------------

/// The `/session` endpoint must include a `compacted_summary` field. When no
/// compaction has happened the field must be `null` (not absent — the client
/// side uses a `null` check to hide the banner, not an undefined check).
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_returns_null_compacted_summary_when_none() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "no-compact-agent");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/session"))).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert!(
        body.get("compacted_summary").is_some(),
        "compacted_summary key must be present in response, got: {body:?}"
    );
    assert!(
        body["compacted_summary"].is_null(),
        "compacted_summary must be null before any compaction: {body:?}"
    );
}

/// After a compaction the `/session` endpoint for the canonical session must
/// return the summary text in `compacted_summary`. Uses `store_llm_summary`
/// directly to isolate the endpoint behaviour from the compactor logic.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_returns_compacted_summary_after_force_compact() {
    use librefang_types::message::{Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "compact-summary-agent");

    let kept: Vec<Message> = vec![
        Message {
            role: Role::User,
            content: MessageContent::Text("u".into()),
            pinned: false,
            timestamp: None,
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("a".into()),
            pinned: false,
            timestamp: None,
        },
    ];

    // The summary must be tagged with the session it belongs to (#6225);
    // the active session is the one compaction would have run against.
    let active_sid = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent registry entry")
        .session_id;

    // Store a summary directly, as compact_agent_session_with_id would.
    h.state
        .kernel
        .memory_substrate()
        .store_llm_summary(id, "A test compaction summary.", kept, Some(active_sid))
        .expect("store_llm_summary");

    // The canonical session endpoint must surface the summary.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/session"))).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert!(
        !body["compacted_summary"].is_null(),
        "compacted_summary must be non-null after compaction: {body:?}"
    );
    let summary = body["compacted_summary"]
        .as_str()
        .expect("compacted_summary must be a string");
    assert_eq!(summary, "A test compaction summary.", "got: {summary}");
}

/// For a pinned ?session_id= that is NOT the canonical session, the
/// `compacted_summary` field must be null even if the canonical session has a
/// summary. The channel/per-session context doesn't share the canonical store.
#[tokio::test(flavor = "multi_thread")]
async fn test_agent_session_returns_null_summary_for_non_canonical_session() {
    use librefang_memory::session::Session;
    use librefang_types::agent::SessionId;
    use librefang_types::message::{Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "non-canonical-summary-agent");

    let messages: Vec<Message> = vec![
        Message {
            role: Role::User,
            content: MessageContent::Text("u".into()),
            pinned: false,
            timestamp: None,
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("a".into()),
            pinned: false,
            timestamp: None,
        },
    ];
    // Own the summary with the agent's active session so the pinned
    // side-session query below is rejected on ownership, not absence (#6225).
    let active_sid = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent registry entry")
        .session_id;
    h.state
        .kernel
        .memory_substrate()
        .store_llm_summary(id, "A test summary.", messages.clone(), Some(active_sid))
        .expect("store_llm_summary");

    // Create a side session (non-canonical) and save it.
    let side_sid = SessionId::for_channel(id, "test:side-session");
    let side_session = Session {
        id: side_sid,
        agent_id: id,
        messages,
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 0,
        last_repaired_generation: None,
        peer_id: None,
    };
    h.state
        .kernel
        .memory_substrate()
        .save_session(&side_session)
        .expect("save side session");

    // Pinned to side session — should return null summary.
    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{id}/session?session_id={}",
            side_sid.0
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert!(
        body["compacted_summary"].is_null(),
        "non-canonical pinned session must have null compacted_summary: {body:?}"
    );
}

/// Regression for #6225: the agent-scoped compaction summary must NOT leak
/// onto a freshly created session that was never compacted.
///
/// `compacted_summary` lives in the agent-scoped `canonical_sessions` row and
/// outlives any individual session. Before the fix the GET handler exposed it
/// whenever the requested session was the agent's *active* session — so
/// creating a brand-new session (which makes it active without compacting it)
/// rendered the previous conversation's summary on message #1. This test
/// drives the full leak path through HTTP: compact session A, confirm A shows
/// the banner, create a NEW active session B, then assert B shows nothing.
#[tokio::test(flavor = "multi_thread")]
async fn test_compacted_summary_does_not_leak_to_new_session() {
    use librefang_types::message::{Message, MessageContent, Role};

    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "summary-leak-agent");

    // Session A: the agent's initial active session, which we "compact".
    let session_a = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent registry entry")
        .session_id;

    let kept: Vec<Message> = vec![Message {
        role: Role::Assistant,
        content: MessageContent::Text("kept".into()),
        pinned: false,
        timestamp: None,
    }];
    h.state
        .kernel
        .memory_substrate()
        .store_llm_summary(id, "Summary of session A.", kept, Some(session_a))
        .expect("store_llm_summary");

    // Legitimate case: GET session A surfaces the summary it owns.
    let (status_a, body_a) = send(h.app.clone(), get(&format!("/api/agents/{id}/session"))).await;
    assert_eq!(status_a, StatusCode::OK, "body={body_a:?}");
    assert_eq!(
        body_a["compacted_summary"].as_str(),
        Some("Summary of session A."),
        "session A owns the summary and must show it: {body_a:?}"
    );

    // Create session B via the public route; this makes B the active session
    // WITHOUT clearing the agent-scoped summary row.
    let (status_new, body_new) = send(
        h.app.clone(),
        post_json(&format!("/api/agents/{id}/sessions"), serde_json::json!({})),
    )
    .await;
    assert_eq!(status_new, StatusCode::OK, "body={body_new:?}");
    let session_b = body_new["session_id"]
        .as_str()
        .expect("new session_id")
        .to_string();
    assert_ne!(
        session_b,
        session_a.0.to_string(),
        "new session must differ from the compacted one"
    );

    // The leak: GET the new active session B. It was never compacted, so the
    // summary owned by session A must NOT appear.
    let (status_b, body_b) = send(h.app.clone(), get(&format!("/api/agents/{id}/session"))).await;
    assert_eq!(status_b, StatusCode::OK, "body={body_b:?}");
    assert_eq!(
        body_b["session_id"].as_str(),
        Some(session_b.as_str()),
        "GET /session must now resolve to the new active session: {body_b:?}"
    );
    assert!(
        body_b["compacted_summary"].is_null(),
        "freshly created session must NOT inherit the prior session's summary: {body_b:?}"
    );

    // The summary is scoped, not lost: the agent-scoped row still records
    // session A as the owner, so a read for A's id resolves it while B does
    // not. (Querying A via the active route is no longer possible once B is
    // active; the substrate-level ownership check is asserted directly.)
    let owned = h
        .state
        .kernel
        .memory_substrate()
        .compacted_summary_for_session(id, session_a)
        .expect("compacted_summary_for_session");
    assert_eq!(
        owned.as_deref(),
        Some("Summary of session A."),
        "session A must still own its summary after B became active"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle cluster: PUT /api/agents/{id}/suspend, /resume, /mode
//
// First slice of the agents-mutation-routes backfill
// (docs/issues/agents-mutation-routes-untested.md). These mutate registry
// state, so each write is followed by a GET read-back asserting the
// observable side effect (`state` / `mode` in the agent detail payload).
// The success-path "status" string returned by each handler is also pinned
// so a future handler refactor that silently flips the response shape is
// caught.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_suspend_agent_sets_state_to_suspended() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "suspend-target");

    let (status, body) = send(
        h.app.clone(),
        put_empty(&format!("/api/agents/{}/suspend", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["status"], "suspended");
    assert_eq!(body["agent_id"], id.to_string());

    // Read-after-write — GET should report the agent as Suspended.
    // `get_agent` renders state via `format!("{:?}", ..)`, so it is the
    // Debug (PascalCase) form, not the snake_case serde rename.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["state"], "Suspended",
        "agent must be Suspended: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resume_agent_sets_state_to_running() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "resume-target");

    // Suspend first so resume has an observable transition to assert.
    let (status, _) = send(
        h.app.clone(),
        put_empty(&format!("/api/agents/{}/suspend", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(body["state"], "Suspended", "precondition: {body:?}");

    let (status, body) = send(
        h.app.clone(),
        put_empty(&format!("/api/agents/{}/resume", id), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["status"], "running");
    assert_eq!(body["agent_id"], id.to_string());

    // Read-after-write — GET should now report the agent as Running.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["state"], "Running",
        "agent must be Running after resume: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_agent_mode_persists_new_mode() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "mode-target");

    // Default spawned mode is Full ("full"); flip to Observe and Assist and
    // assert each persists via read-after-write.
    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(
        body["mode"], "full",
        "precondition: default mode is full: {body:?}"
    );

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{}/mode", id),
            serde_json::json!({"mode": "observe"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["status"], "updated");
    assert_eq!(body["mode"], "observe");

    // Read-after-write — GET reflects the new mode.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["mode"], "observe",
        "mode must persist as observe: {body:?}"
    );

    // A second change to a different mode must also persist (not stuck).
    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{}/mode", id),
            serde_json::json!({"mode": "assist"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(
        body["mode"], "assist",
        "mode must persist as assist: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_agent_mode_rejects_unknown_mode_value() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "mode-bad-value");

    // `SetModeRequest` deserializes a known `AgentMode` variant; an
    // unrecognized string is a body deserialization failure handled by the
    // typed `Json` extractor. The semantic guarantee the issue cares about
    // is 4xx (client error), never a 5xx — assert that, not the exact
    // 400-vs-422 split, which the plain axum `Json` rejection owns.
    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{}/mode", id),
            serde_json::json!({"mode": "wat"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert!(
        status.is_client_error(),
        "unknown mode value must be a 4xx client error, not a 5xx; got {status}"
    );
}

// --- Negative paths: unknown agent must be a clean 4xx, never 500 ---------
// (refs the "agent-not-found-returns-500" issue: these handlers must map a
// missing agent to 404 with the `agent_not_found` code, not bubble a 5xx.)

#[tokio::test(flavor = "multi_thread")]
async fn test_suspend_unknown_agent_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(
        h.app.clone(),
        put_empty(
            &format!("/api/agents/{}/suspend", unknown),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");
    assert_eq!(body["code"], "agent_not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_resume_unknown_agent_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(
        h.app.clone(),
        put_empty(&format!("/api/agents/{}/resume", unknown), Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");
    assert_eq!(body["code"], "agent_not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_mode_unknown_agent_returns_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{}/mode", unknown),
            serde_json::json!({"mode": "full"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body:?}");
    assert_eq!(body["code"], "agent_not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_suspend_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        put_empty("/api/agents/not-a-uuid/suspend", Some(TEST_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert_eq!(body["code"], "invalid_agent_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_mode_invalid_id_returns_400() {
    let h = boot(TEST_TOKEN).await;
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/api/agents/not-a-uuid/mode",
            serde_json::json!({"mode": "full"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");
    assert_eq!(body["code"], "invalid_agent_id");
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id} — auto_evolve field
//
// Pins the contract: toggling auto_evolve off via PATCH must be reflected
// by the subsequent GET, confirming the field is persisted and serialised in
// the agent response.
// ---------------------------------------------------------------------------

/// PATCH `{"auto_evolve": false}` then GET — the response must contain
/// `auto_evolve: false`.  Also verifies round-trip back to `true`.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_agent_auto_evolve_persists() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "auto-evolve-target");

    // Default: auto_evolve should be true on a freshly spawned agent.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["auto_evolve"], true,
        "expected auto_evolve=true by default, got {:?}",
        body["auto_evolve"]
    );

    // PATCH auto_evolve to false.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"auto_evolve": false}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read-after-write — GET must reflect the new value.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["auto_evolve"], false,
        "expected auto_evolve=false after PATCH, got {:?}",
        body["auto_evolve"]
    );

    // Round-trip: restore auto_evolve to true.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{}", id),
            serde_json::json!({"auto_evolve": true}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}", id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["auto_evolve"], true,
        "expected auto_evolve=true after re-enable PATCH, got {:?}",
        body["auto_evolve"]
    );
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/session/context — context-window usage indicator.
//
// The dashboard polls this to render a "how full is the window" bar. It
// resolves the model context window (the Y denominator) that the full
// /session endpoint does not expose, via the same precedence chain the agent
// loop uses (manifest override > catalog > persisted session > 8192 fallback).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_returns_used_and_max_tokens() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "context-indicator");

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{}/session/context", id)),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    // Shape: all five fields present.
    assert!(body["used_tokens"].is_u64(), "used_tokens: {body:?}");
    assert!(
        body["max_context_tokens"].is_u64(),
        "max_context_tokens: {body:?}"
    );
    assert!(body["pct"].is_number(), "pct: {body:?}");
    assert!(body["model"].is_string(), "model: {body:?}");
    assert!(body["pressure"].is_string(), "pressure: {body:?}");

    // The unknown-model fallback (UNKNOWN_MODEL_CONTEXT_WINDOW = 8192) means the
    // denominator is always resolved to a positive value, so the indicator can
    // always render a bar.
    let max = body["max_context_tokens"].as_u64().unwrap();
    assert!(max > 0, "max_context_tokens must be > 0, got {max}");

    let pct = body["pct"].as_f64().unwrap();
    assert!(
        (0.0..=100.0).contains(&pct),
        "pct must be within [0, 100], got {pct}"
    );

    // The model id echoes the agent's own manifest model. spawn_named uses
    // AgentManifest::default(), whose model is "default" — not the global
    // config default_model ("test-model"), which agents do not inherit into
    // their manifest.
    assert_eq!(body["model"].as_str().unwrap(), "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_unknown_agent_404() {
    let h = boot(TEST_TOKEN).await;
    let unknown = AgentId::new();

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{}/session/context", unknown)),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body:?}");
    assert_eq!(body["code"], "agent_not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_is_authed() {
    // With an api_key configured, an unauthenticated (no Bearer) request from a
    // non-loopback caller must be rejected — confirms the endpoint was NOT
    // accidentally added to a PUBLIC_ROUTES_* allowlist slice.
    let h = boot("test-secret").await;
    let id = spawn_named(&h.state, "context-authed");

    let (status, _) = send(
        h.app.clone(),
        get_with(&format!("/api/agents/{}/session/context", id), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Sanity: the same request with the correct Bearer token succeeds.
    let (status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{}/session/context", id)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_honors_session_id_param() {
    // The dashboard sends `?session_id=` for a pinned tab; the bar must track the
    // viewed session, not always the canonical-active one. Pinning the agent's
    // own session must succeed with the same shape, and a malformed id is
    // rejected by the extractor (400) rather than silently ignored.
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "context-pinned");

    // Discover the agent's canonical session id via the public endpoint.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}/session", id))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    let sid = body["session_id"].as_str().expect("session_id").to_string();

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session/context?session_id={}",
            id, sid
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert!(body["max_context_tokens"].as_u64().unwrap() > 0);

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session/context?session_id=not-a-uuid",
            id
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");
    assert_eq!(body["code"], "invalid_session_id");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_rejects_cross_agent_session() {
    // Agent B's session id under agent A's path must 404 — never leak B's usage
    // counts through A's id (mirrors the get_agent_session ownership guard).
    let h = boot(TEST_TOKEN).await;
    let a = spawn_named(&h.state, "context-a");
    let b = spawn_named(&h.state, "context-b");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{}/session", b))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    let b_sid = body["session_id"].as_str().expect("session_id").to_string();

    let (status, body) = send(
        h.app.clone(),
        get(&format!(
            "/api/agents/{}/session/context?session_id={}",
            a, b_sid
        )),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body:?}");
    assert_eq!(body["code"], "session_agent_mismatch");
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/session/context — where the reported window came from.
//
// Refs #7774 item 5. `max_context_tokens` used to be a bare integer, so an
// operator could not tell a window they had configured from one the registry
// declared from one the runtime had invented because nothing knew.
// The reported incident is the last case: a gateway-served model reports no
// limits, the runtime assumes 8192, and a conversation well inside the model's
// real window is refused for an overflow that exists only in that assumption.
// ---------------------------------------------------------------------------

/// Spawn an agent pinned to one `(provider, model)`, optionally with an
/// `agent.toml`-style per-agent window.
fn spawn_on_model(
    state: &Arc<AppState>,
    name: &str,
    provider: &str,
    model: &str,
    context_window: Option<u64>,
) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        model: librefang_types::agent::ModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            context_window,
            ..Default::default()
        },
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

async fn context_source(app: axum::Router, id: AgentId) -> (u64, String, bool) {
    let (status, body) = send(app, get(&format!("/api/agents/{}/session/context", id))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    (
        body["max_context_tokens"]
            .as_u64()
            .unwrap_or_else(|| panic!("max_context_tokens: {body:?}")),
        body["max_context_tokens_source"]
            .as_str()
            .unwrap_or_else(|| panic!("max_context_tokens_source: {body:?}"))
            .to_string(),
        body["max_context_tokens_assumed"]
            .as_bool()
            .unwrap_or_else(|| panic!("max_context_tokens_assumed: {body:?}")),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn context_endpoint_names_the_layer_that_produced_the_window() {
    let h = boot(TEST_TOKEN).await;

    // Layer 3 — the catalog. A custom entry is the only catalog value a
    // hermetic test can state, and it reaches `resolve_context_window` through
    // exactly the path a registry-declared or probe-discovered entry does.
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/models/custom",
            serde_json::json!({
                "id": "sensor-model-generic-high",
                "provider": "ollama",
                "context_window": 32_768,
                "max_output_tokens": 4_096,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body:?}");

    let catalog_agent = spawn_on_model(
        &h.state,
        "ctx-src-catalog",
        "ollama",
        "sensor-model-generic-high",
        None,
    );
    let (max, source, assumed) = context_source(h.app.clone(), catalog_agent).await;
    assert_eq!((max, source.as_str(), assumed), (32_768, "catalog", false));

    // Layer 1 — `agent.toml [model] context_window`, the most specific layer.
    // Same model, so this also proves the per-agent value outranks the catalog
    // in what the report says as well as in what it computes.
    let agent_override = spawn_on_model(
        &h.state,
        "ctx-src-agent",
        "ollama",
        "sensor-model-generic-high",
        Some(96_000),
    );
    let (max, source, assumed) = context_source(h.app.clone(), agent_override).await;
    assert_eq!(
        (max, source.as_str(), assumed),
        (96_000, "agent_override", false)
    );

    // Layer 4 — nothing knows this model, so the number is the runtime's own
    // guess and has to say so. This is the case the issue was filed over.
    let unknown_agent = spawn_on_model(
        &h.state,
        "ctx-src-fallback",
        "litellm",
        "not-in-any-catalog",
        None,
    );
    let (max, source, assumed) = context_source(h.app.clone(), unknown_agent).await;
    assert_eq!(max, 8_192, "the conservative unknown-model fallback");
    assert_eq!(source, "fallback");
    assert!(
        assumed,
        "an assumed window must be flagged as assumed, or the operator reads a guess as a fact"
    );

    // Layer 2 — the per-model operator override, set through the route the
    // dashboard and the TUI both use. It must displace the catalog value AND
    // the label, for the agent that was already reporting `catalog`.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/api/models/overrides/ollama:sensor-model-generic-high",
            serde_json::json!({ "context_window": 16_384 }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let (max, source, assumed) = context_source(h.app.clone(), catalog_agent).await;
    assert_eq!(
        (max, source.as_str(), assumed),
        (16_384, "model_override", false),
        "the operator correction exists to beat the catalog, and the report must say it did"
    );

    // The per-agent value is still the most specific layer after the per-model
    // override lands — a model-level correction is inherited by every agent, so
    // an agent that states its own window keeps it.
    let (max, source, _) = context_source(h.app.clone(), agent_override).await;
    assert_eq!((max, source.as_str()), (96_000, "agent_override"));

    // Deleting the override returns the catalog value and the catalog label.
    let (status, _) = send(
        h.app.clone(),
        delete(
            "/api/models/overrides/ollama:sensor-model-generic-high",
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (max, source, _) = context_source(h.app.clone(), catalog_agent).await;
    assert_eq!((max, source.as_str()), (32_768, "catalog"));
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id}/config — api_key_env / base_url are applied, not
// silently dropped (the OpenAPI schema advertises both fields).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_config_applies_api_key_env_and_base_url() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "cred-agent");

    let (status, _body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/config"),
            serde_json::json!({
                "api_key_env": "MY_CUSTOM_KEY",
                "base_url": "https://api.example.com",
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent exists");
    assert_eq!(
        entry.manifest.model.api_key_env.as_deref(),
        Some("MY_CUSTOM_KEY"),
        "api_key_env must be applied, not dropped"
    );
    assert_eq!(
        entry.manifest.model.base_url.as_deref(),
        Some("https://api.example.com"),
        "base_url must be applied, not dropped"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_config_empty_string_clears_api_key_env_but_preserves_base_url() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "cred-agent-2");

    // Seed both.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/config"),
            serde_json::json!({ "api_key_env": "SEED_KEY", "base_url": "https://seed.example" }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Clear only api_key_env (empty string); base_url is absent -> unchanged.
    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/config"),
            serde_json::json!({ "api_key_env": "" }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent exists");
    assert_eq!(
        entry.manifest.model.api_key_env, None,
        "empty string must clear api_key_env"
    );
    assert_eq!(
        entry.manifest.model.base_url.as_deref(),
        Some("https://seed.example"),
        "an absent base_url must be left unchanged"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_patch_config_can_restore_global_default_inheritance() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "default-inheriting-agent");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/config"),
            serde_json::json!({
                "provider": "custom-provider",
                "model": "custom-model",
                "api_key_env": "CUSTOM_KEY",
                "base_url": "https://custom.example/v1",
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/config"),
            serde_json::json!({
                "provider": "default",
                "model": "default",
            }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent exists");
    assert_eq!(entry.manifest.model.provider, "default");
    assert_eq!(entry.manifest.model.model, "default");
    assert!(entry.manifest.model.api_key_env.is_none());
    assert!(entry.manifest.model.base_url.is_none());
}

// ---------------------------------------------------------------------------
// PATCH /api/agents/{id}/identity — merge semantics (refs #6608)
// ---------------------------------------------------------------------------

/// The full six-field identity used as the "before" state of the merge tests.
/// `color` must start with `#` and `avatar_url` with `http`/`https`/`data:` to pass both handlers' validators.
fn full_identity_body() -> serde_json::Value {
    serde_json::json!({
        "emoji": "🦊",
        "avatar_url": "https://example.invalid/avatar.png",
        "color": "#123456",
        "archetype": "researcher",
        "vibe": "technical",
        "greeting_style": "brief",
    })
}

/// Read all six identity fields straight off the registry entry.
///
/// `GET /api/agents/{id}` exposes only `emoji` / `avatar_url` / `color` (`routes::agents::agent_json`), so the three personality fields have no HTTP read surface at all.
/// Extending that response to make the assertions uniform would be a response-shape change well beyond this fix (it would drag in the dashboard's `AgentIdentity` type and the generated OpenAPI), so the test reads the same state the handler wrote instead.
fn stored_identity(state: &Arc<AppState>, id: AgentId) -> librefang_types::agent::AgentIdentity {
    state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent exists")
        .identity
}

/// A single-field PATCH must change that field and leave the other five untouched.
/// Before #6608 this handler built a fresh `AgentIdentity` from the request body alone, so the five omitted fields were nulled and the response was still 200.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_identity_merges_instead_of_replacing() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "identity-merge");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/identity"),
            full_identity_body(),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "seeding identity failed: {body:?}");

    // Partial update: one field only.
    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/identity"),
            serde_json::json!({"emoji": "🤖"}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "partial patch failed: {body:?}");

    let identity = stored_identity(&h.state, id);
    // The patched field really changed — without this the test would pass on a
    // handler that ignored the body entirely.
    assert_eq!(
        identity.emoji.as_deref(),
        Some("🤖"),
        "the submitted field must be applied"
    );
    // The five omitted fields survived.
    assert_eq!(
        identity.avatar_url.as_deref(),
        Some("https://example.invalid/avatar.png"),
        "avatar_url must survive a partial PATCH"
    );
    assert_eq!(
        identity.color.as_deref(),
        Some("#123456"),
        "color must survive a partial PATCH"
    );
    assert_eq!(
        identity.archetype.as_deref(),
        Some("researcher"),
        "archetype must survive a partial PATCH"
    );
    assert_eq!(
        identity.vibe.as_deref(),
        Some("technical"),
        "vibe must survive a partial PATCH"
    );
    assert_eq!(
        identity.greeting_style.as_deref(),
        Some("brief"),
        "greeting_style must survive a partial PATCH"
    );

    // The three fields the HTTP read surface does expose agree with the above.
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["identity"]["emoji"], serde_json::json!("🤖"));
    assert_eq!(
        body["identity"]["avatar_url"],
        serde_json::json!("https://example.invalid/avatar.png")
    );
    assert_eq!(body["identity"]["color"], serde_json::json!("#123456"));
}

/// `PATCH /identity` and `PATCH /config` write the same six identity fields, so the same partial body must produce the same stored identity through either route.
/// Two PATCH endpoints on one resource having opposite semantics is the core of #6608, and this equivalence is what keeps them from diverging again.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_identity_and_patch_config_agree_on_partial_updates() {
    let h = boot(TEST_TOKEN).await;
    let via_identity = spawn_named(&h.state, "identity-route-agent");
    let via_config = spawn_named(&h.state, "config-route-agent");

    for id in [via_identity, via_config] {
        let (status, body) = send(
            h.app.clone(),
            patch_json(
                &format!("/api/agents/{id}/identity"),
                full_identity_body(),
                Some(TEST_TOKEN),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "seeding identity failed: {body:?}");
    }

    // Same partial body, one through each route.
    let partial = serde_json::json!({"vibe": "playful", "emoji": "🐙"});

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{via_identity}/identity"),
            partial.clone(),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{via_config}/config"),
            partial,
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let from_identity = stored_identity(&h.state, via_identity);
    let from_config = stored_identity(&h.state, via_config);

    assert_eq!(from_identity.emoji, from_config.emoji, "emoji diverged");
    assert_eq!(
        from_identity.avatar_url, from_config.avatar_url,
        "avatar_url diverged"
    );
    assert_eq!(from_identity.color, from_config.color, "color diverged");
    assert_eq!(
        from_identity.archetype, from_config.archetype,
        "archetype diverged"
    );
    assert_eq!(from_identity.vibe, from_config.vibe, "vibe diverged");
    assert_eq!(
        from_identity.greeting_style, from_config.greeting_style,
        "greeting_style diverged"
    );

    // Pin the shared expectation too, so the test cannot pass by both routes
    // being broken in the same way.
    assert_eq!(from_identity.vibe.as_deref(), Some("playful"));
    assert_eq!(from_identity.emoji.as_deref(), Some("🐙"));
    assert_eq!(from_identity.archetype.as_deref(), Some("researcher"));
    assert_eq!(from_identity.greeting_style.as_deref(), Some("brief"));
}

/// Sending an empty string is as close as either route gets to clearing an identity field, since `null` / omitted is already spoken for by "not provided".
/// It must land as a stored `""` rather than being folded back to the previous value — the field ends up empty, not absent.
#[tokio::test(flavor = "multi_thread")]
async fn test_patch_identity_empty_string_clears_a_single_field() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "identity-clear");

    let (status, _) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/identity"),
            full_identity_body(),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}/identity"),
            serde_json::json!({"color": ""}),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "empty color must validate: {body:?}"
    );

    let identity = stored_identity(&h.state, id);
    assert_eq!(
        identity.color.as_deref(),
        Some(""),
        "an empty string must clear the field, not merge back the old value"
    );
    assert_eq!(
        identity.emoji.as_deref(),
        Some("🦊"),
        "clearing one field must not disturb the others"
    );
}

// ---------------------------------------------------------------------------
// Declared-but-unavailable skills and MCP servers (#7713)
//
// A template can name a skill nobody installed or an MCP server that is not
// reachable here. The declaration is kept verbatim, so without these fields
// the operator's only signal is a step that quietly does nothing.
// ---------------------------------------------------------------------------

/// A skill named in the manifest but absent from the registry surfaces on both
/// read routes, and clears once it is installed and the registry reloaded —
/// with no re-spawn, so the agent id is unchanged throughout.
#[tokio::test(flavor = "multi_thread")]
async fn test_pending_skill_surfaces_then_clears_after_registry_reload() {
    let h = boot(TEST_TOKEN).await;

    let manifest_toml = r#"
name = "pending-skills-agent"
description = "declares a skill that is not installed here"
skills = ["ghost-skill"]

[model]
provider = "ollama"
model = "test-model"
"#;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "spawn must succeed; body={body:?}"
    );
    let id = body["agent_id"].as_str().expect("agent_id").to_string();

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["skills"],
        serde_json::json!(["ghost-skill"]),
        "the declaration must be retained verbatim"
    );
    assert_eq!(
        body["pending_skills"],
        serde_json::json!(["ghost-skill"]),
        "an uninstalled skill must surface as pending on the detail payload"
    );
    assert_eq!(body["pending_mcp_servers"], serde_json::json!([]));

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["pending"],
        serde_json::json!(["ghost-skill"]),
        "the skills route must list the same pending name"
    );

    // Install the skill on disk and reload the registry through the API.
    let skill_dir = h.state.kernel.home_dir().join("skills").join("ghost-skill");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
[skill]
name = "ghost-skill"
version = "0.1.0"
description = "Test skill"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "ghost-skill_tool"
description = "A test tool"
input_schema = { type = "object" }
"#,
    )
    .expect("write skill.toml");

    let (status, _) = send(
        h.app.clone(),
        post_json("/api/skills/reload", serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "skills reload must succeed");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["pending_skills"],
        serde_json::json!([]),
        "installing the skill must clear the pending state without a re-spawn"
    );
    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pending"], serde_json::json!([]));
}

/// The MCP half, end to end: a configured server with no live connection reads
/// as pending on both routes, and clears once it actually connects.
///
/// The server is present in `effective_mcp_servers` for the whole test, so a
/// pending set derived from the configured snapshot would report `[]` at the
/// first assertion and this test would fail.
#[tokio::test(flavor = "multi_thread")]
async fn test_pending_mcp_server_surfaces_until_the_connection_is_live() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let backend = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&backend)
        .await;

    let server = librefang_types::config::McpServerConfigEntry {
        name: "ghost-mcp".to_string(),
        template_id: None,
        transport: Some(librefang_types::config::McpTransportEntry::HttpCompat {
            base_url: backend.uri(),
            headers: Vec::new(),
            tools: vec![librefang_types::config::HttpCompatToolConfig {
                name: "probe".to_string(),
                path: "/probe".to_string(),
                ..Default::default()
            }],
        }),
        timeout_secs: 5,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    };
    let h = boot_with_mcp_servers(TEST_TOKEN, vec![server]).await;

    let manifest_toml = r#"
name = "pending-mcp-agent"
description = "declares an MCP server that has not connected yet"
mcp_servers = ["ghost-mcp"]

[model]
provider = "ollama"
model = "test-model"
"#;
    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "manifest_toml": manifest_toml }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "spawn must succeed; body={body:?}"
    );
    let id = body["agent_id"].as_str().expect("agent_id").to_string();

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["mcp_servers"],
        serde_json::json!(["ghost-mcp"]),
        "the declaration must be retained verbatim"
    );
    assert_eq!(
        body["pending_mcp_servers"],
        serde_json::json!(["ghost-mcp"]),
        "a configured server with no live connection must surface as pending"
    );
    assert_eq!(body["pending_skills"], serde_json::json!([]));

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/mcp_servers"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["pending"],
        serde_json::json!(["ghost-mcp"]),
        "the mcp_servers route must list the same pending name"
    );
    assert_eq!(
        body["available"],
        serde_json::json!([]),
        "nothing is available while the server has not connected"
    );

    Arc::clone(&h.state.kernel).connect_mcp_servers().await;

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["pending_mcp_servers"],
        serde_json::json!([]),
        "a live connection must clear the pending state without a re-spawn"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/mcp_servers"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pending"], serde_json::json!([]));
    assert_eq!(
        body["available"],
        serde_json::json!(["ghost-mcp"]),
        "the connected server must now be in the available pool"
    );
}

// ---------------------------------------------------------------------------
// PUT /api/agents/{id}/mcp_servers and PUT /api/agents/{id}/skills — an
// inherited declaration this instance has not installed must not take the
// whole save down with it (#7772).
//
// The dashboard PUTs the entire array, so an agent that inherited `fetch`
// from a registry agent type could not be edited at all: the request failed
// with `[400] Internal error: Unknown MCP server: fetch`, naming a server the
// operator never asked for, and the servers they were actually adding were
// never stored.
// ---------------------------------------------------------------------------

/// A helper to build a configured-but-transportless MCP server entry, the
/// shape `config.toml`'s `[[mcp_servers]]` produces before anything dials.
fn configured_mcp_entry(name: &str) -> librefang_types::config::McpServerConfigEntry {
    librefang_types::config::McpServerConfigEntry {
        name: name.to_string(),
        template_id: None,
        transport: None,
        timeout_secs: 30,
        env: Vec::new(),
        headers: Vec::new(),
        oauth: None,
        taint_scanning: true,
        taint_policy: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_mcp_servers_keeps_inherited_declaration_while_adding_configured_ones() {
    // The reported failure, end to end. `fetch` ships in the pinned registry
    // fixture's MCP catalog and is never configured here, so it is installed
    // but not configured — exactly the state that used to 400.
    let h = boot_with_mcp_servers(
        TEST_TOKEN,
        vec![
            configured_mcp_entry("memory"),
            configured_mcp_entry("camoufox"),
            configured_mcp_entry("sequential-thinking"),
        ],
    )
    .await;
    let id = spawn_named(&h.state, "inherited-fetch-agent");

    let saved = serde_json::json!(["memory", "fetch", "camoufox", "sequential-thinking"]);
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/mcp_servers"),
            serde_json::json!({ "mcp_servers": saved }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an inherited catalog-only declaration must not block a save that adds configured servers; body={body:?}"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/mcp_servers"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"], saved,
        "every name in the PUT must be persisted, the pending one included"
    );
    // `pending` here is a liveness answer (`unconnected_mcp_declarations`), and
    // nothing dials in this harness, so every declared name is listed. What
    // matters is that the un-installed one is reported rather than dropped.
    let pending = body["pending"].as_array().expect("pending array");
    assert!(
        pending.iter().any(|v| v == "fetch"),
        "the un-installed declaration stays visible as pending rather than being silently dropped, got: {pending:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_mcp_servers_rejects_unknown_name_without_internal_error_wording() {
    // A name in neither config.toml nor the catalog is still rejected, but as
    // a user-input problem: the old `Internal` variant is what made the
    // dashboard say "Internal error" for a typo.
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "unknown-mcp-put-agent");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/mcp_servers"),
            serde_json::json!({ "mcp_servers": ["totally-made-up-server"] }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().expect("error message present");
    assert!(
        msg.contains("totally-made-up-server"),
        "the message must name the rejected server, got: {msg}"
    );
    assert!(
        !msg.contains("Internal error"),
        "a validation failure must not read as a server fault, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_skills_accepts_pending_manifest_name_not_directory_name() {
    // The skills half, with the directory name and `[skill].name` deliberately
    // different. The running registry is keyed by the manifest name, so
    // `actual-skill` is the only value that can ever match and `package-dir`
    // is the value that never will — validating against directory names got
    // this exactly backwards.
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "pending-skill-put-agent");

    // Written after boot, so the directory is on disk but not in the loaded registry.
    let skill_dir = h.state.kernel.home_dir().join("skills").join("package-dir");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("skill.toml"),
        r#"
[skill]
name = "actual-skill"
version = "0.1.0"
description = "directory name and manifest name deliberately differ"
author = "test"

[runtime]
type = "promptonly"

[source]
type = "local"
"#,
    )
    .expect("write skill.toml");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": ["actual-skill"] }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the manifest name of an on-disk-but-unloaded skill must be accepted; body={body:?}"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["assigned"], serde_json::json!(["actual-skill"]));

    // The directory name must not be accepted in its place — a stored
    // `package-dir` would match nothing once the skill loads.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": ["package-dir"] }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the directory name can never match after load and must be rejected; body={body:?}"
    );
    let msg = body["error"].as_str().expect("error message present");
    assert!(
        !msg.contains("Internal error"),
        "a validation failure must not read as a server fault, got: {msg}"
    );

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/skills"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["assigned"],
        serde_json::json!(["actual-skill"]),
        "a rejected save must leave the stored allowlist untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_skills_rejects_unknown_name_without_internal_error_wording() {
    let h = boot(TEST_TOKEN).await;
    let id = spawn_named(&h.state, "unknown-skill-put-agent");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/skills"),
            serde_json::json!({ "skills": ["totally-made-up-skill"] }),
            Some(TEST_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = body["error"].as_str().expect("error message present");
    assert!(
        msg.contains("totally-made-up-skill"),
        "the message must name the rejected skill, got: {msg}"
    );
    assert!(
        !msg.contains("Internal error"),
        "a validation failure must not read as a server fault, got: {msg}"
    );
}

/// #8018 — spawning an agent from a template records the template's name on
/// the manifest as `source_template`, so it round-trips through both the
/// detail endpoint and the list endpoint the dashboard reads from.
#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_from_template_records_source_template() {
    let h = boot(TEST_TOKEN).await;

    let tmpl_dir = h
        .state
        .kernel
        .config_ref()
        .home_dir
        .join("workspaces")
        .join("agents")
        .join("origin-tmpl");
    std::fs::create_dir_all(&tmpl_dir).expect("create template dir");
    std::fs::write(
        tmpl_dir.join("agent.toml"),
        r#"name = "origin-tmpl"
version = "0.1.0"
description = "template used to test source_template tracking"
module = "builtin:chat"

[model]
provider = "default"
model = "default"
"#,
    )
    .expect("write agent.toml");

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({ "template": "origin-tmpl", "name": "origin-tmpl-instance" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["agent_id"].as_str().expect("agent_id present");

    let (status, detail) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        detail["source_template"], "origin-tmpl",
        "GET /api/agents/{{id}} must report the template an agent was spawned from: {detail}"
    );

    let (status, list) = send(h.app.clone(), get("/api/agents")).await;
    assert_eq!(status, StatusCode::OK);
    let entries = list["items"].as_array().expect("items array");
    let entry = entries
        .iter()
        .find(|a| a["id"] == id)
        .expect("spawned agent present in list");
    assert_eq!(
        entry["source_template"], "origin-tmpl",
        "GET /api/agents list entry must also report source_template: {entry}"
    );
}

/// A directly-supplied `manifest_toml` (no `template`) must leave
/// `source_template` unset — provenance is only recorded when the manifest
/// actually came from a named template.
#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_without_template_leaves_source_template_unset() {
    let h = boot(TEST_TOKEN).await;

    let (status, body) = send(
        h.app.clone(),
        post_json(
            "/api/agents",
            serde_json::json!({
                "manifest_toml": "name = \"no-template-agent\"\nversion = \"0.1.0\"\ndescription = \"d\"\nmodule = \"builtin:chat\"\n\n[model]\nprovider = \"default\"\nmodel = \"default\"\n",
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["agent_id"].as_str().expect("agent_id present");

    let (status, detail) = send(h.app.clone(), get(&format!("/api/agents/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        detail["source_template"].is_null(),
        "an agent spawned from an inline manifest must not report a source_template: {detail}"
    );
}
