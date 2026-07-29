//! Integration tests for the per-instance liveness fields on `GET /api/channels` (#6606).
//!
//! Two compounding defects motivated these:
//!
//! 1. The dashboard derived its status dot from `msgs_24h > 0`, and neither
//!    `connected` nor `last_error` was present on the payload at all — so a
//!    healthy-but-quiet channel rendered as idle and a channel that died after
//!    receiving traffic rendered as running.
//! 2. `msgs_24h` was a per-channel-*type* aggregate (`usage_events.channel`
//!    stores the type) published under a name that reads per-instance, behind
//!    an unreachable `msgs_24h.get(name)` fallback. On a host with six Telegram
//!    sidecars all six cards reported the Telegram total.
//!
//! The assertions below pin both halves: the supervisor's real per-instance
//! `ChannelStatus` reaches the payload, two sidecars of the same type report
//! *different* liveness, and the shared 24h figure is published under a name
//! that says what it covers.

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use futures::Stream;
use librefang_api::server;
use librefang_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
};
use librefang_kernel::LibreFangKernel;
use librefang_memory::usage::UsageRecord;
use librefang_types::agent::AgentId;
use librefang_types::config::{DefaultModelConfig, KernelConfig, SidecarChannelConfig};
use std::pin::Pin;
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

/// Adapter that only exists to publish a fixed `ChannelStatus`.
///
/// `GET /api/channels` reads liveness through `ChannelAdapter::status()`, so a
/// stub with a canned status is enough to drive every branch of the payload
/// without spawning a real sidecar child. `start` / `send` / `stop` are never
/// reached by the list handler.
struct StatusOnlyAdapter {
    name: String,
    channel_type: ChannelType,
    status: ChannelStatus,
}

#[async_trait]
impl ChannelAdapter for StatusOnlyAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(Box::pin(futures::stream::empty::<ChannelMessage>()))
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        _content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.clone()
    }
}

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// A `[[sidecar_channels]]` entry that the bridge will register but never
/// successfully spawn.
///
/// `build_router` starts the channel bridge, so a non-empty
/// `sidecar_channels` list means real spawn attempts. `command` points at a
/// path that cannot exist and `restart = false` makes the supervisor break out
/// of its loop after the single failed attempt instead of retrying with
/// backoff for the lifetime of the test. The stub adapters inserted after boot
/// overwrite these registrations, so the failed spawns never influence the
/// asserted payload.
///
/// Built through the TOML deserializer rather than a struct literal:
/// `SidecarChannelConfig` has no `Default` impl, every field but `name` and
/// `command` carries `#[serde(default)]`, and going through serde means a
/// newly-added field does not need a fixture edit to keep compiling.
fn unspawnable_sidecar(name: &str, channel_type: &str) -> SidecarChannelConfig {
    toml::from_str(&format!(
        "name = \"{name}\"\n\
         command = \"/nonexistent/librefang-test-sidecar\"\n\
         channel_type = \"{channel_type}\"\n\
         restart = false\n"
    ))
    .expect("sidecar fixture parses")
}

async fn boot_router(sidecar_channels: Vec<SidecarChannelConfig>) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
        sidecar_channels,
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

/// Replace whatever the bridge registered under `name` with a stub publishing
/// `status`. Registration is keyed by the sidecar instance name, which is the
/// same key `sidecar_channel_rows` looks up, so this is per-bot.
fn register_status(h: &RouterHarness, name: &str, channel_type: &str, status: ChannelStatus) {
    let adapter: Arc<dyn ChannelAdapter> = Arc::new(StatusOnlyAdapter {
        name: name.to_string(),
        channel_type: ChannelType::Custom(channel_type.to_string()),
        status,
    });
    h.state
        .kernel
        .channel_adapters_ref()
        .insert(name.to_string(), adapter);
}

/// Drop every adapter the bridge registered, so a configured channel with no
/// live adapter can be asserted on.
fn clear_adapters(h: &RouterHarness) {
    h.state.kernel.channel_adapters_ref().clear();
}

/// Record `count` telegram-attributed usage events. `usage_events.channel`
/// carries the channel TYPE, which is precisely why the 24h figure cannot be
/// per-instance.
fn seed_telegram_usage(h: &RouterHarness, count: usize) {
    let usage = h.state.kernel.memory_substrate().usage();
    for _ in 0..count {
        usage
            .record(&UsageRecord {
                agent_id: AgentId::new(),
                model: "test-model".to_string(),
                cost_usd: 0.01,
                channel: Some("telegram".to_string()),
                ..Default::default()
            })
            .expect("record usage event");
    }
}

async fn get_channels(h: &RouterHarness) -> serde_json::Value {
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
    serde_json::from_slice(&bytes).expect("channels payload is JSON")
}

fn row<'a>(payload: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    payload["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("no row named {name} in {payload}"))
}

/// The whole point of defect 2: two sidecars of the same channel type must
/// report their own liveness, and the shared 24h figure must be named for the
/// scope it actually has. This fails against the pre-fix handler, which
/// emitted neither `connected` nor `messages_received` and published the
/// shared aggregate as `msgs_24h`.
#[tokio::test(flavor = "multi_thread")]
async fn same_type_sidecars_report_independent_liveness() {
    let h = boot_router(vec![
        unspawnable_sidecar("tg-personal", "telegram"),
        unspawnable_sidecar("tg-alerts", "telegram"),
    ])
    .await;
    seed_telegram_usage(&h, 35);

    register_status(
        &h,
        "tg-personal",
        "telegram",
        ChannelStatus {
            connected: true,
            started_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()),
            last_message_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 12, 0, 0).unwrap()),
            messages_received: 30,
            messages_sent: 25,
            last_error: None,
            qr: None,
        },
    );
    register_status(
        &h,
        "tg-alerts",
        "telegram",
        ChannelStatus {
            connected: false,
            started_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()),
            last_message_at: None,
            messages_received: 2,
            messages_sent: 1,
            last_error: Some("sidecar exited with status 1".to_string()),
            qr: None,
        },
    );

    let payload = get_channels(&h).await;
    let live = row(&payload, "tg-personal");
    let dead = row(&payload, "tg-alerts");

    // Per-instance liveness diverges even though the two share a channel type.
    assert_eq!(live["supervised"], serde_json::json!(true));
    assert_eq!(live["connected"], serde_json::json!(true));
    assert_eq!(live["messages_received"], serde_json::json!(30));
    assert_eq!(live["messages_sent"], serde_json::json!(25));
    assert_eq!(live["last_error"], serde_json::Value::Null);
    // Asserted on the instant prefix rather than the whole RFC 3339 string:
    // the offset rendering (`+00:00` vs `Z`) is chrono's choice, not part of
    // this endpoint's contract.
    assert!(
        live["started_at"]
            .as_str()
            .is_some_and(|s| s.starts_with("2030-01-01T00:00:00")),
        "started_at must serialize the supervisor's timestamp, got {}",
        live["started_at"]
    );
    assert!(
        live["last_message_at"]
            .as_str()
            .is_some_and(|s| s.starts_with("2030-01-01T12:00:00")),
        "last_message_at must serialize the supervisor's timestamp, got {}",
        live["last_message_at"]
    );

    assert_eq!(dead["supervised"], serde_json::json!(true));
    assert_eq!(
        dead["connected"],
        serde_json::json!(false),
        "a dead sibling must not inherit the live one's connection state"
    );
    assert_eq!(dead["messages_received"], serde_json::json!(2));
    assert_eq!(dead["messages_sent"], serde_json::json!(1));
    assert_eq!(
        dead["last_error"],
        serde_json::json!("sidecar exited with status 1")
    );
    assert_eq!(dead["last_message_at"], serde_json::Value::Null);

    // Per-instance traffic differs; the 24h figure is shared BY DESIGN and
    // published under a name that says so. The pre-fix `msgs_24h` key —
    // indistinguishable from a per-bot number — must be gone entirely so no
    // consumer keeps reading the shared aggregate as per-bot traffic.
    assert_eq!(live["channel_type"], serde_json::json!("telegram"));
    assert_eq!(dead["channel_type"], serde_json::json!("telegram"));
    assert_eq!(live["msgs_24h_channel_type"], serde_json::json!(35));
    assert_eq!(dead["msgs_24h_channel_type"], serde_json::json!(35));
    assert!(
        live.get("msgs_24h").is_none() && dead.get("msgs_24h").is_none(),
        "the ambiguous `msgs_24h` key must not survive: {payload}"
    );
}

/// A channel that received traffic and then died must be reported as
/// disconnected. This is the reporter's incident: the old indicator keyed off
/// traffic alone, so this row rendered green.
#[tokio::test(flavor = "multi_thread")]
async fn channel_with_traffic_but_dead_process_reports_disconnected() {
    let h = boot_router(vec![unspawnable_sidecar("tg-personal", "telegram")]).await;
    seed_telegram_usage(&h, 12);
    register_status(
        &h,
        "tg-personal",
        "telegram",
        ChannelStatus {
            connected: false,
            started_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()),
            last_message_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 1, 0, 0).unwrap()),
            messages_received: 34,
            messages_sent: 21,
            last_error: None,
            qr: None,
        },
    );

    let payload = get_channels(&h).await;
    let r = row(&payload, "tg-personal");
    assert_eq!(r["connected"], serde_json::json!(false));
    assert!(
        r["messages_received"].as_u64().unwrap_or(0) > 0,
        "traffic is present, which is exactly why traffic is not a health signal"
    );
    assert!(
        r["msgs_24h_channel_type"].as_u64().unwrap_or(0) > 0,
        "the per-type aggregate is non-zero too and must not imply health"
    );
    assert!(
        r["started_at"].is_string(),
        "started_at distinguishes 'died' from 'never started'"
    );
}

/// A configured sidecar with no registered adapter is reported as
/// unsupervised rather than as a connected channel with zero traffic.
#[tokio::test(flavor = "multi_thread")]
async fn configured_sidecar_without_adapter_reports_unsupervised() {
    let h = boot_router(vec![unspawnable_sidecar("tg-personal", "telegram")]).await;
    clear_adapters(&h);

    let payload = get_channels(&h).await;
    let r = row(&payload, "tg-personal");
    assert_eq!(r["configured"], serde_json::json!(true));
    assert_eq!(r["supervised"], serde_json::json!(false));
    assert_eq!(r["connected"], serde_json::json!(false));
    assert_eq!(r["started_at"], serde_json::Value::Null);
    assert_eq!(r["last_message_at"], serde_json::Value::Null);
    assert_eq!(r["messages_received"], serde_json::json!(0));
    assert_eq!(r["messages_sent"], serde_json::json!(0));
    assert_eq!(r["last_error"], serde_json::Value::Null);
}

/// Liveness is keyed by the sidecar instance name, so a channel whose type
/// happens to match another instance's name must not borrow its status.
#[tokio::test(flavor = "multi_thread")]
async fn liveness_is_keyed_by_instance_name_not_channel_type() {
    let h = boot_router(vec![
        unspawnable_sidecar("tg-personal", "telegram"),
        unspawnable_sidecar("telegram", "telegram"),
    ])
    .await;
    clear_adapters(&h);
    // Only the instance literally named "telegram" gets an adapter.
    register_status(
        &h,
        "telegram",
        "telegram",
        ChannelStatus {
            connected: true,
            started_at: Some(Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()),
            last_message_at: None,
            messages_received: 4,
            messages_sent: 4,
            last_error: None,
            qr: None,
        },
    );

    let payload = get_channels(&h).await;
    assert_eq!(
        row(&payload, "telegram")["connected"],
        serde_json::json!(true)
    );
    assert_eq!(
        row(&payload, "tg-personal")["supervised"],
        serde_json::json!(false),
        "a sibling sharing only the channel type must not resolve to that adapter"
    );
}
