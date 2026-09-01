//! A configured `[[sidecar_channels]]` row on `GET /api/channels` must publish the same schema provenance a discovery row does — `sdk_version` when the adapter reported one, and `schema_error` when `--describe` left nothing to render (#8063).
//!
//! The reported incident was a Slack instance named `slack-hr` whose gear icon opened a configure drawer holding one collapsed TOML snippet and an enabled Save, with no form fields and no explanation.
//! `fields[]` is empty for a configured row exactly when no `--describe` schema is cached for its `channel_type`, and the row said nothing about that, so the dashboard could not tell "this adapter has no form" from "this adapter's form failed to load, here is how to fix it" and rendered a drawer that could only fail.
//!
//! Asserted against the route rather than `sidecar_channel_rows` directly, because the defect class is a fact the daemon already holds that never reaches the payload — which is precisely what a handler-level unit test cannot see.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::routes::channels::{
    __test_seed_sidecar_schema_cache, __test_seed_sidecar_schema_error_cache,
};
use librefang_api::routes::sidecar_describe::{SidecarSchema, SidecarSchemaField};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig, SidecarChannelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";
const REPORTED_VERSION: &str = "2026.8.3001";
/// The instance name from the issue. Deliberately NOT its adapter name: the
/// catalog key is `slack`, and everything schema-shaped on this row has to be
/// resolved through `channel_type`, not through the instance's own name.
const INSTANCE: &str = "slack-hr";

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        // Leave the process-wide describe caches empty so a seeded fixture cannot leak into another integration test's rows.
        __test_seed_sidecar_schema_cache(&[]);
        __test_seed_sidecar_schema_error_cache(&[]);
        self.state.kernel.shutdown();
    }
}

/// Boot the router with one configured Slack instance whose `[[sidecar_channels]].name` differs from its `channel_type`.
///
/// `command` points at a path that cannot exist so the supervisor's spawn fails once and stays failed instead of restart-looping a real Python sidecar for the length of the test; nothing this test asserts depends on the adapter running, since `fields` / `sdk_version` / `schema_error` are all resolved from the catalog's describe caches.
async fn boot_router() -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let sidecar: SidecarChannelConfig = serde_json::from_value(serde_json::json!({
        "name": INSTANCE,
        "channel_type": "slack",
        "command": "/nonexistent/librefang-test-sidecar",
        "args": [],
        "restart": false,
        "env": {"SLACK_ALLOWED_CHANNELS": "C0123"},
    }))
    .expect("sidecar fixture deserializes");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
        sidecar_channels: vec![sidecar],
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
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    RouterHarness {
        app,
        _tmp: tmp,
        state,
    }
}

fn slack_schema(sdk_version: Option<&str>) -> SidecarSchema {
    SidecarSchema {
        name: "slack".to_string(),
        display_name: "Slack".to_string(),
        description: "Slack Socket Mode bot adapter (out-of-process sidecar)".to_string(),
        sdk_version: sdk_version.map(str::to_string),
        fields: vec![
            SidecarSchemaField {
                key: "SLACK_APP_TOKEN".to_string(),
                label: "App Token (xapp-)".to_string(),
                field_type: "secret".to_string(),
                required: true,
                placeholder: "xapp-1-...".to_string(),
                advanced: false,
                options: None,
            },
            SidecarSchemaField {
                key: "SLACK_ALLOWED_CHANNELS".to_string(),
                label: "Allowed Channel IDs".to_string(),
                field_type: "text".to_string(),
                required: false,
                placeholder: "C0123, C0456".to_string(),
                advanced: false,
                options: None,
            },
        ],
    }
}

/// The configured row for `INSTANCE`, not the `slack` catalog row that sits
/// alongside it as the "add another instance" affordance.
async fn configured_row(h: &RouterHarness) -> serde_json::Value {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/channels")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .expect("request builds");
    let resp = h.app.clone().oneshot(req).await.expect("router responds");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body reads");
    let payload: serde_json::Value =
        serde_json::from_slice(&bytes).expect("channels payload is JSON");
    payload["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .find(|row| row["name"] == INSTANCE)
        .unwrap_or_else(|| panic!("no {INSTANCE} row in {payload}"))
        .clone()
}

/// Both halves live in one test on purpose: the describe caches are process-wide and the seeders clear-then-set, so splitting them into parallel tests would race on the shared maps.
#[tokio::test(flavor = "multi_thread")]
async fn configured_row_publishes_its_adapter_schema_provenance() {
    let h = boot_router().await;

    // --- a schema is cached for the instance's channel_type ---
    // The form fields the drawer renders, the instance's own stored value, and
    // the SDK version behind them all resolve through `channel_type` (`slack`),
    // never through the instance name (`slack-hr`).
    __test_seed_sidecar_schema_cache(&[("slack", slack_schema(Some(REPORTED_VERSION)))]);
    __test_seed_sidecar_schema_error_cache(&[]);
    let row = configured_row(&h).await;
    assert_eq!(row["channel_type"], "slack");
    assert_eq!(row["configured"], true);
    assert_eq!(
        row["sdk_version"], REPORTED_VERSION,
        "a configured row must publish the SDK version its adapter reported, same as a discovery row: {row}"
    );
    assert!(
        row.get("schema_error").is_none(),
        "a row with a usable schema must not carry a failure reason: {row}"
    );
    let fields = row["fields"].as_array().expect("fields is an array");
    assert_eq!(fields.len(), 2, "{row}");
    let allowed = fields
        .iter()
        .find(|f| f["key"] == "SLACK_ALLOWED_CHANNELS")
        .unwrap_or_else(|| panic!("no SLACK_ALLOWED_CHANNELS field in {row}"));
    assert_eq!(
        allowed["value"], "C0123",
        "the instance's stored non-secret value must reach the form: {row}"
    );
    let token = fields
        .iter()
        .find(|f| f["key"] == "SLACK_APP_TOKEN")
        .unwrap_or_else(|| panic!("no SLACK_APP_TOKEN field in {row}"));
    assert!(
        token.get("value").is_none(),
        "a secret must never be echoed back as plaintext: {row}"
    );

    // --- no schema is cached, and the daemon knows why ---
    // This is the #8063 payload. `fields` is empty either way; without the
    // reason riding along the dashboard cannot say anything about the empty
    // drawer it is about to render, which is how an unusable Save button ended
    // up looking enabled.
    let reason = "librefang-sdk is not installed in the Python interpreter resolved by 'python3'.";
    __test_seed_sidecar_schema_cache(&[]);
    __test_seed_sidecar_schema_error_cache(&[("slack", reason.to_string())]);
    let row = configured_row(&h).await;
    assert_eq!(
        row["fields"].as_array().map(Vec::len),
        Some(0),
        "no cached schema means no form fields: {row}"
    );
    assert_eq!(
        row["schema_error"], reason,
        "a configured row whose adapter has no usable schema must carry the actionable reason: {row}"
    );
    assert!(
        row.get("sdk_version").is_none(),
        "a failed describe reported no SDK version, so the key must be absent rather than null: {row}"
    );
}
