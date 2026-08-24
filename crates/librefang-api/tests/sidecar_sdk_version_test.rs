//! `GET /api/channels` must publish the SDK version each sidecar adapter reported on `--describe` (#7140).
//!
//! The reported incident was a Telegram sidecar running `librefang-sdk` 2026.3.2201 against a 2026.7.31 daemon, and the only way anyone could establish that was by shelling into the host: the version existed in the adapter's own package metadata, and there was no path — no wire frame, no API field — by which it could reach an operator.
//! `--describe` runs the same interpreter, with the same PYTHONPATH resolution, as the eventual spawn, so the version it reports is the version that will actually serve traffic.
//!
//! The route is asserted rather than only `sidecar_discovery_rows`, because the defect class here is a value that exists in the daemon but never reaches the payload — exactly what a handler-level unit test cannot see.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::routes::channels::{
    __test_seed_sidecar_schema_cache, __test_seed_sidecar_schema_error_cache,
};
use librefang_api::routes::sidecar_describe::{SidecarSchema, SidecarSchemaField};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";
const REPORTED_VERSION: &str = "2026.3.2201";

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        // Leave the process-wide describe caches empty so a seeded fixture cannot leak into another integration test's discovery rows.
        __test_seed_sidecar_schema_cache(&[]);
        __test_seed_sidecar_schema_error_cache(&[]);
        self.state.kernel.shutdown();
    }
}

/// Boot the router with no `[[sidecar_channels]]` configured, so every catalog adapter appears as an unconfigured discovery row.
async fn boot_router() -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
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
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    RouterHarness {
        app,
        _tmp: tmp,
        state,
    }
}

fn schema_reporting(version: Option<&str>) -> SidecarSchema {
    SidecarSchema {
        name: "telegram".to_string(),
        display_name: "Telegram".to_string(),
        description: "Telegram Bot API adapter (out-of-process sidecar)".to_string(),
        sdk_version: version.map(str::to_string),
        fields: vec![SidecarSchemaField {
            key: "TELEGRAM_BOT_TOKEN".to_string(),
            label: "Bot Token".to_string(),
            field_type: "secret".to_string(),
            required: true,
            placeholder: String::new(),
            advanced: false,
            options: None,
        }],
    }
}

async fn telegram_row(h: &RouterHarness) -> serde_json::Value {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/channels")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    let payload: serde_json::Value =
        serde_json::from_slice(&bytes).expect("channels payload is JSON");
    payload["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .find(|row| row["name"] == "telegram")
        .unwrap_or_else(|| panic!("no telegram row in {payload}"))
        .clone()
}

/// Both halves in one test: the describe caches are process-wide and the seeders clear-then-set, so splitting them into parallel tests would race on the shared maps.
#[tokio::test(flavor = "multi_thread")]
async fn channels_payload_carries_the_adapter_reported_sdk_version() {
    let h = boot_router().await;

    // --- adapter reported a version: it reaches the payload verbatim ---
    __test_seed_sidecar_schema_cache(&[("telegram", schema_reporting(Some(REPORTED_VERSION)))]);
    __test_seed_sidecar_schema_error_cache(&[]);
    let row = telegram_row(&h).await;
    assert_eq!(
        row["sdk_version"], REPORTED_VERSION,
        "the version the adapter reported on --describe must reach GET /api/channels"
    );

    // --- adapter reported nothing: the key is absent, not null ---
    // An SDK too old to carry `sdk_version` is exactly the deployment this field exists to diagnose, so the payload must distinguish "did not report" from "reported nothing" rather than inventing a value.
    __test_seed_sidecar_schema_cache(&[("telegram", schema_reporting(None))]);
    let row = telegram_row(&h).await;
    assert!(
        row.get("sdk_version").is_none(),
        "an adapter that reported no SDK version must not get a null field: {row}"
    );
}
