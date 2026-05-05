//! Integration tests for the `routes/plugins.rs` HTTP surface.
//!
//! Refs librefang/librefang#3571 — partial coverage for the plugins domain
//! (one slice of the wider "~80% of registered routes have no integration
//! test" backlog). Mounts the real `plugins::router()` against a freshly-
//! booted mock kernel via `tower::oneshot` so each test exercises the
//! handler exactly the way the live daemon would dispatch it.
//!
//! Scope of this file (intentionally tight):
//!   * Read endpoints whose behaviour is fully determined by kernel /
//!     config state (no installed plugins, no active context engine):
//!     `/api/context-engine/{config,chain,health,traces,sandbox-policy,
//!     metrics,metrics/summary,metrics/per-agent,metrics/prometheus,
//!     traces/history,traces/{trace_id}}`.
//!   * Pure validation / error paths on POST endpoints that reject the
//!     request before touching disk: `install`, `uninstall`, `scaffold`,
//!     `batch`, `install-with-deps`, `upgrade`, `test-hook`, `benchmark`,
//!     `prewarm` list-form, `registry/search` (validate_registry_param).
//!   * 404 paths on read endpoints that look up a plugin by name:
//!     `/api/plugins/{name}` family, when the name does not exist.
//!
//! Out of scope (skipped — see PR body for rationale):
//!   * Any handler that mutates `~/.librefang/plugins/` (enable/disable/
//!     reload/install/uninstall/scaffold/sign/state DELETE/upgrade/
//!     install-deps/prewarm-single/test-hook happy-path) — they would
//!     race across parallel test binaries on the user's home dir.
//!   * `list_plugin_registries` and `plugin_registry_search` happy-path —
//!     they perform live HTTPS calls to `raw.githubusercontent.com`.
//!   * `plugin_update_check` happy-path — same network reason.
//!   * `export_plugin` — needs a real plugin on disk.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    // No `default_model` override needed — the plugins routes never reach
    // the LLM driver. We do, however, want a deterministic `[context_engine]`
    // section: the default is `engine = "default"` with no plugin / stack,
    // which is exactly what most assertions below depend on.
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::plugins::router())
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

async fn raw_request(
    h: &Harness,
    method: Method,
    path: &str,
) -> (StatusCode, String, Option<String>) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes).to_string();
    (status, body, content_type)
}

/// A name guaranteed not to collide with any real plugin in the developer's
/// `~/.librefang/plugins/` directory. Used for 404 probes; never written to.
const ABSENT_PLUGIN: &str = "librefang_test_does_not_exist_3571_plugin";

// ---------------------------------------------------------------------------
// context-engine GET endpoints — driven by kernel config snapshot
// ---------------------------------------------------------------------------

/// `/api/context-engine/config` must surface the running daemon's
/// `[context_engine]` section verbatim. With the default mock kernel, no
/// plugin is configured, so `engine` must equal `"default"` and the
/// `plugin` / `plugin_stack` slots must be null.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_config_returns_default_engine() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/context-engine/config", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["engine"], "default", "{body:?}");
    assert!(body["plugin"].is_null(), "{body:?}");
    assert!(body["plugin_stack"].is_null(), "{body:?}");
    // The hooks section must always be present so the dashboard can render.
    assert!(body["hooks"].is_object(), "hooks missing: {body:?}");
    // Registries always include the official one (merged in by the handler).
    assert!(
        body["registries"].is_array(),
        "registries missing: {body:?}"
    );
}

/// `/api/context-engine/chain` reports the active topology. With no plugin
/// configured, `mode` must be `"default"` and the chain must be empty.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_chain_default_mode_empty_chain() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/context-engine/chain", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["mode"], "default", "{body:?}");
    assert_eq!(body["chain"], serde_json::json!([]));
    assert_eq!(body["chain_length"], 0);
    assert!(body["fallback"].is_string(), "{body:?}");
}

/// `/api/context-engine/health` returns 204 (No Content) when no plugin is
/// configured — the handler treats "no engine" as healthy-but-empty.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_health_no_plugin_returns_204() {
    let h = boot();
    let (status, body, _ct) = raw_request(&h, Method::GET, "/api/context-engine/health").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");
}

/// `/api/context-engine/sandbox-policy` falls back to global defaults when
/// no plugin is active.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_sandbox_policy_falls_back_to_defaults() {
    let h = boot();
    let (status, body) =
        json_request(&h, Method::GET, "/api/context-engine/sandbox-policy", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["mode"], "default", "{body:?}");
    assert_eq!(body["plugins"], serde_json::json!([]));
    assert!(
        body["global_defaults"].is_object(),
        "global_defaults missing: {body:?}"
    );
}

/// `/api/context-engine/metrics` returns 204 when no engine is loaded —
/// `context_engine_ref()` is `None` on the mock kernel.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_metrics_no_engine_returns_204() {
    let h = boot();
    let (status, _, _) = raw_request(&h, Method::GET, "/api/context-engine/metrics").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// `/api/context-engine/metrics/summary` likewise yields 204 when there are
/// no metrics to summarise.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_metrics_summary_no_engine_returns_204() {
    let h = boot();
    let (status, _, _) = raw_request(&h, Method::GET, "/api/context-engine/metrics/summary").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// `/api/context-engine/metrics/prometheus` returns 204 when no engine is
/// active. Important: this is the scrape endpoint Prometheus probes on a
/// fixed schedule, so a 5xx here would surface as a scrape failure.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_metrics_prometheus_no_engine_returns_204() {
    let h = boot();
    let (status, body, _) =
        raw_request(&h, Method::GET, "/api/context-engine/metrics/prometheus").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "body={body}");
}

/// `/api/context-engine/metrics/per-agent` always returns 200 with an empty
/// map — the per-agent path is plugin-feature-gated and the handler is a
/// stub today. We pin the response shape so the dashboard can rely on it.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_per_agent_metrics_returns_empty_envelope() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/context-engine/metrics/per-agent",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["per_agent"], serde_json::json!({}));
    assert_eq!(body["total_agents"], 0);
    assert!(body["note"].is_string());
}

/// `/api/context-engine/traces` returns 200 with an empty trace ring buffer
/// under the default mock kernel — the default context engine is always
/// built (engine = "default"), so the route reports zero traces rather
/// than 204. The 204 branch only fires when no engine is wired at all.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_traces_default_engine_returns_empty_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/context-engine/traces", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["traces"], serde_json::json!([]));
    assert_eq!(body["count"], 0);
}

/// `/api/context-engine/traces/history` returns 200 with empty traces and
/// surfaces the parsed filters back so callers can confirm their query.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_trace_history_returns_filters_envelope() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/context-engine/traces/history?plugin=foo&hook=ingest&limit=10&failures_only=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["traces"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(body["limit"], 10);
    assert_eq!(body["filters"]["plugin"], "foo");
    assert_eq!(body["filters"]["hook"], "ingest");
    assert_eq!(body["filters"]["failures_only"], true);
}

/// `/api/context-engine/traces/{trace_id}` validates the id shape (16 hex
/// chars) before consulting the trace store, so a malformed id must 400.
#[tokio::test(flavor = "multi_thread")]
async fn context_engine_get_trace_by_id_rejects_malformed_id() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/context-engine/traces/not-a-trace-id",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("16 lowercase hex"),
        "{body:?}"
    );
}

// ---------------------------------------------------------------------------
// /api/plugins read endpoints — 404 for unknown plugin names
// ---------------------------------------------------------------------------

/// `/api/plugins` always responds with the canonical
/// `PaginatedResponse{items,total,offset,limit}` envelope (#3842). Even if
/// the developer happens to have plugins on disk, the shape must match.
#[tokio::test(flavor = "multi_thread")]
async fn list_plugins_returns_envelope_shape() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/plugins", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(body["items"].is_array(), "{body:?}");
    assert!(body["total"].is_number(), "{body:?}");
    assert_eq!(body["offset"], 0, "{body:?}");
    assert!(body.get("limit").is_some(), "{body:?}");
}

/// `/api/plugins/{name}` returns 404 for an unknown plugin.
#[tokio::test(flavor = "multi_thread")]
async fn get_plugin_unknown_returns_404() {
    let h = boot();
    let path = format!("/api/plugins/{ABSENT_PLUGIN}");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]["message"].as_str().is_some(),
        "missing error string: {body:?}"
    );
}

/// `/api/plugins/{name}/advanced-config` returns 404 for an unknown plugin.
#[tokio::test(flavor = "multi_thread")]
async fn plugin_advanced_config_unknown_returns_404() {
    let h = boot();
    let path = format!("/api/plugins/{ABSENT_PLUGIN}/advanced-config");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/{name}/env` returns 404 for an unknown plugin.
#[tokio::test(flavor = "multi_thread")]
async fn plugin_env_unknown_returns_404() {
    let h = boot();
    let path = format!("/api/plugins/{ABSENT_PLUGIN}/env");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/{name}/health` returns 404 for an unknown plugin.
#[tokio::test(flavor = "multi_thread")]
async fn plugin_health_unknown_returns_404() {
    let h = boot();
    let path = format!("/api/plugins/{ABSENT_PLUGIN}/health");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/{name}/status` rejects unknown names with 400 (the handler
/// treats lookup failure as a bad-request because the same endpoint also
/// surfaces `validate_plugin_name` errors via the same path).
#[tokio::test(flavor = "multi_thread")]
async fn plugin_status_unknown_returns_400() {
    let h = boot();
    let path = format!("/api/plugins/{ABSENT_PLUGIN}/status");
    let (status, body) = json_request(&h, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"]["message"].as_str().is_some(), "{body:?}");
}

/// `/api/plugins/{name}/state` validates the name first; an invalid name
/// must 400 *before* any filesystem access. Pins the path-traversal guard.
#[tokio::test(flavor = "multi_thread")]
async fn get_plugin_state_rejects_invalid_name() {
    let h = boot();
    // Use a syntactically invalid name (contains `..`); the underlying
    // axum router treats this as a single path segment (no traversal),
    // and `validate_plugin_name` should reject it with 400.
    let (status, body) = json_request(&h, Method::GET, "/api/plugins/has..dots/state", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("invalid plugin name"),
        "{body:?}"
    );
}

// ---------------------------------------------------------------------------
// POST validation paths — handlers that reject before touching disk
// ---------------------------------------------------------------------------

/// `/api/plugins/install` requires a `source` field.
#[tokio::test(flavor = "multi_thread")]
async fn install_plugin_rejects_missing_source() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/install",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"]["message"].as_str().is_some(), "{body:?}");
}

/// `source = registry` requires `name`.
#[tokio::test(flavor = "multi_thread")]
async fn install_plugin_registry_source_requires_name() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/install",
        Some(serde_json::json!({"source": "registry"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("name"),
        "{body:?}"
    );
}

/// `/api/plugins/uninstall` requires `name`.
#[tokio::test(flavor = "multi_thread")]
async fn uninstall_plugin_requires_name() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/uninstall",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("name"),
        "{body:?}"
    );
}

/// Uninstalling an unknown plugin returns 404 (the manager errors with
/// "not installed" and the handler maps it).
#[tokio::test(flavor = "multi_thread")]
async fn uninstall_unknown_plugin_returns_404() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/uninstall",
        Some(serde_json::json!({"name": ABSENT_PLUGIN})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/scaffold` requires `name`.
#[tokio::test(flavor = "multi_thread")]
async fn scaffold_plugin_requires_name() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/scaffold",
        Some(serde_json::json!({"description": "no name"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// `/api/plugins/install-with-deps` requires `name`.
#[tokio::test(flavor = "multi_thread")]
async fn install_with_deps_requires_name() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/install-with-deps",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// `/api/plugins/install-with-deps` runs `validate_plugin_name` so a name
/// containing `/` is rejected (path-traversal guard).
#[tokio::test(flavor = "multi_thread")]
async fn install_with_deps_rejects_invalid_name() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/install-with-deps",
        Some(serde_json::json!({"name": "evil/../escape"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("invalid plugin name"),
        "{body:?}"
    );
}

/// `/api/plugins/batch` requires both `operation` and a non-empty `plugins`
/// array. Missing `operation`.
#[tokio::test(flavor = "multi_thread")]
async fn batch_plugin_operation_requires_operation() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/batch",
        Some(serde_json::json!({"plugins": ["a"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("operation"),
        "{body:?}"
    );
}

/// Missing `plugins` array.
#[tokio::test(flavor = "multi_thread")]
async fn batch_plugin_operation_requires_plugins_array() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/batch",
        Some(serde_json::json!({"operation": "enable"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("plugins"),
        "{body:?}"
    );
}

/// Empty `plugins` array.
#[tokio::test(flavor = "multi_thread")]
async fn batch_plugin_operation_rejects_empty_plugins_array() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/batch",
        Some(serde_json::json!({"operation": "enable", "plugins": []})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("empty"),
        "{body:?}"
    );
}

/// Batch over absent plugins still 200s — per-plugin failures are reported
/// inside the `results` array, with `all_ok: false`. This is the shape the
/// dashboard's bulk-action UI consumes.
#[tokio::test(flavor = "multi_thread")]
async fn batch_plugin_operation_reports_per_plugin_failures() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/batch",
        Some(serde_json::json!({
            "operation": "enable",
            "plugins": [ABSENT_PLUGIN],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["operation"], "enable");
    assert_eq!(body["all_ok"], false);
    let results = body["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["plugin"], ABSENT_PLUGIN);
    assert_eq!(results[0]["result"]["ok"], false);
}

/// Unknown `operation` flows through the same shape: per-plugin
/// `result.ok = false` with an `Unknown operation` error string.
#[tokio::test(flavor = "multi_thread")]
async fn batch_plugin_operation_unknown_op_reports_per_plugin_error() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/batch",
        Some(serde_json::json!({
            "operation": "rm-rf",
            "plugins": [ABSENT_PLUGIN],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["all_ok"], false);
    assert!(
        body["results"][0]["result"]["error"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown operation"),
        "{body:?}"
    );
}

/// `/api/plugins/{name}/upgrade` with `source = local` requires `path`.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_plugin_local_source_requires_path() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/upgrade"),
        Some(serde_json::json!({"source": "local"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// `/api/plugins/{name}/upgrade` with `source = git` requires `url`.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_plugin_git_source_requires_url() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/upgrade"),
        Some(serde_json::json!({"source": "git"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// `/api/plugins/{name}/upgrade` with `source = registry` and an explicitly
/// invalid `registry` value (not `owner/repo`) hits `validate_registry_param`
/// before any network call — pins the input-validation guard.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_plugin_rejects_invalid_registry_format() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/upgrade"),
        Some(serde_json::json!({
            "source": "registry",
            "registry": "not-owner-slash-repo",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("owner/repo"),
        "{body:?}"
    );
}

/// `/api/plugins/{name}/test-hook` requires `hook`.
#[tokio::test(flavor = "multi_thread")]
async fn test_plugin_hook_requires_hook_field() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/test-hook"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("hook"),
        "{body:?}"
    );
}

/// `/api/plugins/{name}/test-hook` returns 404 when the plugin doesn't
/// exist (after the `hook` field is provided).
#[tokio::test(flavor = "multi_thread")]
async fn test_plugin_hook_unknown_plugin_returns_404() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/test-hook"),
        Some(serde_json::json!({"hook": "ingest", "input": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/{name}/benchmark` requires `hook`.
#[tokio::test(flavor = "multi_thread")]
async fn benchmark_plugin_hook_requires_hook_field() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/benchmark"),
        Some(serde_json::json!({"runs": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
}

/// `/api/plugins/{name}/benchmark` returns 404 when the plugin doesn't
/// exist (after the `hook` field is provided).
#[tokio::test(flavor = "multi_thread")]
async fn benchmark_plugin_hook_unknown_plugin_returns_404() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/plugins/{ABSENT_PLUGIN}/benchmark"),
        Some(serde_json::json!({"hook": "ingest", "runs": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
}

/// `/api/plugins/prewarm` with an explicit absent plugin name reports a
/// per-plugin "not found" entry but still returns 200 with the envelope.
/// (Handler design: per-plugin failures don't fail the whole batch.)
#[tokio::test(flavor = "multi_thread")]
async fn prewarm_plugins_reports_missing_per_plugin() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/plugins/prewarm",
        Some(serde_json::json!({"plugins": [ABSENT_PLUGIN]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let entry = &body["results"][ABSENT_PLUGIN];
    assert_eq!(entry["ok"], false, "{body:?}");
    assert!(
        entry["message"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "{body:?}"
    );
}

/// `/api/plugins/registry/search` runs `validate_registry_param` first —
/// a malformed `registry` query param must 400 before any HTTP call.
#[tokio::test(flavor = "multi_thread")]
async fn plugin_registry_search_rejects_invalid_registry_param() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/plugins/registry/search?registry=not-a-valid-spec",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("owner/repo"),
        "{body:?}"
    );
}
