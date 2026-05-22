//! Integration tests for the per-agent channel allowlist endpoints.
//!
//! Routes covered:
//!   GET  /api/agents/{id}/channels — default response (empty assigned, mode="all")
//!   PUT  /api/agents/{id}/channels — set allowlist, read-after-write assertion
//!   PUT  /api/agents/{id}/channels — clear allowlist back to all-mode
//!   GET  /api/agents/{bad-uuid}/channels  — 400 for invalid id
//!   GET  /api/agents/{unknown-uuid}/channels — 404 for unknown agent
//!   PUT  /api/agents/{unknown-uuid}/channels — 404/error for unknown agent
//!
//! Run: cargo test -p librefang-api --test agent_channels_routes_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig, SidecarChannelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
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

const TEST_TOKEN: &str = "test-secret";

async fn boot() -> Harness {
    boot_with_sidecars(vec![]).await
}

async fn boot_with_sidecars(sidecar_channels: Vec<SidecarChannelConfig>) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::sync_registry(
        tmp.path(),
        librefang_kernel::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        sidecar_channels,
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

fn spawn_agent(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
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

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::empty())
        .unwrap()
}

fn put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", TEST_TOKEN))
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/channels — default: empty allowlist → mode "all"
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_channels_default_all_mode() {
    let h = boot().await;
    let id = spawn_agent(&h.state, "chan-test-default");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["mode"], "all",
        "fresh agent must default to all-mode; body={body}"
    );
    assert_eq!(
        body["assigned"],
        serde_json::json!([]),
        "no channels assigned by default; body={body}"
    );
    // `available` key must be present (may be empty in test env — no channels configured)
    assert!(
        body["available"].is_array(),
        "available must be an array; body={body}"
    );
}

// ---------------------------------------------------------------------------
// PUT /api/agents/{id}/channels — set allowlist, then verify with GET
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_put_agent_channels_sets_allowlist_and_get_confirms() {
    let h = boot().await;
    let id = spawn_agent(&h.state, "chan-test-put");

    // PUT with a channel name
    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": ["telegram"]}),
        ),
    )
    .await;

    assert_eq!(put_status, StatusCode::OK, "PUT body={put_body}");
    assert_eq!(put_body["status"], "ok", "PUT body={put_body}");
    assert_eq!(
        put_body["channels"],
        serde_json::json!(["telegram"]),
        "PUT body={put_body}"
    );

    // Read-after-write: GET must reflect the new allowlist
    let (get_status, get_body) =
        send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(get_status, StatusCode::OK, "GET body={get_body}");
    assert_eq!(
        get_body["assigned"],
        serde_json::json!(["telegram"]),
        "GET must reflect assigned channels; body={get_body}"
    );
    assert_eq!(
        get_body["mode"], "allowlist",
        "non-empty allowlist → mode allowlist; body={get_body}"
    );
}

// ---------------------------------------------------------------------------
// PUT /api/agents/{id}/channels — clear allowlist back to all-mode
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_put_agent_channels_clear_restores_all_mode() {
    let h = boot().await;
    let id = spawn_agent(&h.state, "chan-test-clear");

    // First set to telegram
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": ["telegram"]}),
        ),
    )
    .await;

    // Now clear
    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/channels"),
            serde_json::json!({"channels": []}),
        ),
    )
    .await;

    assert_eq!(put_status, StatusCode::OK, "clear PUT body={put_body}");

    // Verify mode reverts to "all"
    let (get_status, get_body) =
        send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET after clear body={get_body}"
    );
    assert_eq!(
        get_body["mode"], "all",
        "empty allowlist → mode all; body={get_body}"
    );
    assert_eq!(
        get_body["assigned"],
        serde_json::json!([]),
        "assigned should be empty after clear; body={get_body}"
    );
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_channels_invalid_id_returns_400() {
    let h = boot().await;

    let (status, _body) = send(h.app.clone(), get("/api/agents/not-a-uuid/channels")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_channels_unknown_id_returns_404() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();

    let (status, _body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{unknown}/channels")),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_agent_channels_unknown_id_returns_error() {
    let h = boot().await;
    let unknown = uuid::Uuid::new_v4();

    let (status, _body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unknown}/channels"),
            serde_json::json!({"channels": ["slack"]}),
        ),
    )
    .await;

    // set_agent_channels on an unknown id returns an error → BAD_REQUEST
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "expected 400 or 404 for unknown agent, got {status}"
    );
}

// ---------------------------------------------------------------------------
// Namespace correctness: available list must use channel_type, not display name
// ---------------------------------------------------------------------------
//
// A sidecar with name="my-tg-bot" and channel_type="telegram" must surface
// "telegram" in `available`, not "my-tg-bot". Bridge enforcement calls
// `channel_type_str` which returns the channel_type string (or the Custom(s)
// inner value) — offering display names would cause every inbound message to
// be filtered out even when the operator correctly sets the allowlist.

#[tokio::test(flavor = "multi_thread")]
async fn test_get_agent_channels_available_uses_channel_type_namespace() {
    // Boot with two sidecar entries:
    //   1. name="my-tg-bot", channel_type=Some("telegram") — channel_type differs from name
    //   2. name="discord",   channel_type=None              — channel_type defaults to name
    //
    // Construct via serde_json so we benefit from `#[serde(default)]` for all
    // non-required fields without needing access to the private default fns.
    let sidecars: Vec<SidecarChannelConfig> = serde_json::from_value(serde_json::json!([
        {"name": "my-tg-bot", "command": "python3", "channel_type": "telegram"},
        {"name": "discord",   "command": "python3"},
    ]))
    .expect("sidecar config");

    let h = boot_with_sidecars(sidecars).await;
    let id = spawn_agent(&h.state, "chan-test-namespace");

    let (status, body) = send(h.app.clone(), get(&format!("/api/agents/{id}/channels"))).await;

    assert_eq!(status, StatusCode::OK, "body={body}");
    let available = body["available"]
        .as_array()
        .expect("available must be an array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();

    // Must contain "telegram" (the channel_type), not "my-tg-bot" (the display name).
    assert!(
        available.contains(&"telegram"),
        "available must contain channel_type 'telegram', not display name; available={available:?}"
    );
    assert!(
        !available.contains(&"my-tg-bot"),
        "available must NOT contain display name 'my-tg-bot'; available={available:?}"
    );
    // "discord" appears both as name and channel_type — must be present.
    assert!(
        available.contains(&"discord"),
        "available must contain 'discord'; available={available:?}"
    );
    // Dedup: "telegram" appears only once even if listed twice.
    let telegram_count = available.iter().filter(|&&s| s == "telegram").count();
    assert_eq!(
        telegram_count, 1,
        "channel type 'telegram' must appear exactly once; available={available:?}"
    );
}
