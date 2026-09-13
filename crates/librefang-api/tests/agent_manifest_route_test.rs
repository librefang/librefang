//! Integration tests for `GET /api/agents/{id}/manifest`.
//!
//! Refs #7742 — the dashboard's full manifest editor reads the agent's whole
//! `AgentManifest` as raw TOML here and writes it back through
//! `PATCH /api/agents/{id}` (`manifest_toml`). Tests exercise the production
//! router (`server::build_router`) with `tower::ServiceExt::oneshot`, so the
//! real auth middleware, route registration, and handler logic are in play.
//! No real LLM calls — every test is hermetic.
//!
//! Routes covered:
//!   GET   /api/agents/{id}/manifest  (200 TOML, bad id 400, unknown agent 404)
//!   PATCH /api/agents/{id}           (manifest_toml round trip, tag sync)
//!
//! Run: cargo test -p librefang-api --test agent_manifest_route_test

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
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

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
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
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

fn spawn_with(state: &Arc<AppState>, manifest: AgentManifest) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn patch_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn send_text(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The endpoint must hand back the *whole* manifest, not the curated
/// `AgentDetail` projection — that is the entire point of #7742. Assert on a
/// field the detail shape never carried (`max_history_messages`) as well as
/// the ones it did.
#[tokio::test(flavor = "multi_thread")]
async fn get_manifest_returns_full_manifest_as_toml() {
    let h = boot().await;
    let id = spawn_with(
        &h.state,
        AgentManifest {
            name: "manifest-full".to_string(),
            description: "a described agent".to_string(),
            tags: vec!["alpha".to_string()],
            max_history_messages: Some(123),
            ..AgentManifest::default()
        },
    );

    let (status, body) = send_text(h.app.clone(), get(&format!("/api/agents/{id}/manifest"))).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let parsed: AgentManifest =
        toml::from_str(&body).expect("response must parse as AgentManifest");
    assert_eq!(parsed.name, "manifest-full");
    assert_eq!(parsed.description, "a described agent");
    assert_eq!(parsed.tags, vec!["alpha".to_string()]);
    assert_eq!(
        parsed.max_history_messages,
        Some(123),
        "a field absent from the curated AgentDetail shape must still round-trip here"
    );
}

/// The read path and the `manifest_toml` write path must be a closed loop:
/// what `GET .../manifest` returns has to be accepted verbatim by
/// `PATCH /api/agents/{id}`, and a field edited in between must survive.
#[tokio::test(flavor = "multi_thread")]
async fn manifest_toml_round_trips_through_patch() {
    let h = boot().await;
    let id = spawn_with(
        &h.state,
        AgentManifest {
            name: "manifest-roundtrip".to_string(),
            ..AgentManifest::default()
        },
    );

    let (status, original) =
        send_text(h.app.clone(), get(&format!("/api/agents/{id}/manifest"))).await;
    assert_eq!(status, StatusCode::OK);

    let mut edited: AgentManifest = toml::from_str(&original).expect("parse");
    edited.description = "edited through the manifest editor".to_string();
    edited.max_history_messages = Some(42);
    let edited_toml = toml::to_string_pretty(&edited).expect("serialize");

    let (status, body) = send_text(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "manifest_toml": edited_toml }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, after) =
        send_text(h.app.clone(), get(&format!("/api/agents/{id}/manifest"))).await;
    assert_eq!(status, StatusCode::OK);
    let reloaded: AgentManifest = toml::from_str(&after).expect("parse");
    assert_eq!(reloaded.description, "edited through the manifest editor");
    assert_eq!(reloaded.max_history_messages, Some(42));
}

/// #7742: a manifest PATCH that changes `tags` must update the registry
/// entry's index-backing `tags` field, not only `entry.manifest`. Before the
/// fix, `replace_manifest` explicitly restored the spawn-time `entry.tags`
/// snapshot, so the manifest read back the new tag while every tag-indexed
/// lookup still saw the old one — the two silently disagreed.
#[tokio::test(flavor = "multi_thread")]
async fn patching_tags_updates_the_registry_entry_not_just_the_manifest() {
    let h = boot().await;
    let id = spawn_with(
        &h.state,
        AgentManifest {
            name: "manifest-tags".to_string(),
            tags: vec!["old-tag".to_string()],
            ..AgentManifest::default()
        },
    );

    let entry = h.state.kernel.agent_registry().get(id).expect("entry");
    assert_eq!(entry.tags, vec!["old-tag".to_string()]);

    let (status, original) =
        send_text(h.app.clone(), get(&format!("/api/agents/{id}/manifest"))).await;
    assert_eq!(status, StatusCode::OK);
    let mut edited: AgentManifest = toml::from_str(&original).expect("parse");
    edited.tags = vec!["new-tag".to_string()];

    let (status, body) = send_text(
        h.app.clone(),
        patch_json(
            &format!("/api/agents/{id}"),
            serde_json::json!({ "manifest_toml": toml::to_string_pretty(&edited).expect("ser") }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let entry = h.state.kernel.agent_registry().get(id).expect("entry");
    assert_eq!(
        entry.tags,
        vec!["new-tag".to_string()],
        "entry.tags must follow the manifest instead of keeping its spawn-time snapshot"
    );
    assert_eq!(
        entry.tags, entry.manifest.tags,
        "entry.tags and manifest.tags must not drift apart"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_manifest_bad_agent_id_returns_400() {
    let h = boot().await;
    let (status, _) = send_text(h.app.clone(), get("/api/agents/not-a-uuid/manifest")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_manifest_unknown_agent_returns_404() {
    let h = boot().await;
    let unknown = AgentId::new();
    let (status, _) = send_text(
        h.app.clone(),
        get(&format!("/api/agents/{unknown}/manifest")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
