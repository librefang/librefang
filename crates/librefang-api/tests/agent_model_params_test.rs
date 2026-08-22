//! Integration tests for the agent-level inference parameters on
//! `PATCH /api/agents/{id}/config`.
//!
//! Three behaviours are pinned here, all of them regressions waiting to happen:
//!
//! 1. **Tri-state round-trip.** A value pins the knob for this agent, an
//!    explicit `null` hands it back to "inherit", and an absent key changes
//!    nothing. Before this the fields were plain numbers with no inherit
//!    state, which is what forced the per-model override to win over the agent
//!    manifest — an agent that could not say "I have no opinion" left the
//!    override no way to reach it.
//! 2. **All five preference knobs are settable per agent**, not just
//!    `max_tokens` and `temperature`.
//! 3. **A limit warns but never clamps**, and only when the limit came from a
//!    real source — a discovery placeholder stays silent (#7780).
//!
//! Run: cargo test -p librefang-api --test agent_model_params_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use librefang_types::model_catalog::{Modality, ModelCatalogEntry, ModelTier};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

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

/// Boots the production router and seeds two catalog entries that differ only
/// in whether their capacities were sourced:
///
/// * `ollama:known-model` — curated limits, `limits_known = true`.
/// * `ollama:discovered-model` — the `merge_discovered_models` shape: the same
///   `131_072` / `16_384` placeholders a gateway probe leaves behind, flagged
///   `limits_known = false` because nothing asserted them.
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

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();

    // Annotated because a concrete `LibreFangKernel` has both an inherent
    // `model_catalog_update` and the `KernelApi` one, so the closure parameter
    // has nothing to infer from.
    kernel.model_catalog_update(
        &mut |catalog: &mut librefang_runtime::model_catalog::ModelCatalog| {
            catalog.add_custom_model(ModelCatalogEntry {
                id: "known-model".to_string(),
                display_name: "Known model".to_string(),
                provider: "ollama".to_string(),
                tier: ModelTier::Custom,
                modality: Modality::Text,
                context_window: 200_000,
                max_output_tokens: 16_384,
                limits_known: true,
                ..Default::default()
            });
            catalog.add_custom_model(ModelCatalogEntry {
                id: "discovered-model".to_string(),
                display_name: "Discovered model".to_string(),
                provider: "ollama".to_string(),
                tier: ModelTier::Local,
                modality: Modality::Text,
                context_window: 131_072,
                max_output_tokens: 16_384,
                limits_known: false,
                ..Default::default()
            });
        },
    );

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

/// Spawns an agent already pointed at `model`, so the limit check has a
/// catalog entry to resolve.
fn spawn_on(state: &Arc<AppState>, name: &str, model: &str) -> AgentId {
    let mut manifest = AgentManifest {
        name: name.to_string(),
        ..AgentManifest::default()
    };
    manifest.model.provider = "ollama".to_string();
    manifest.model.model = model.to_string();
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

fn patch(id: AgentId, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PATCH)
        .uri(format!("/api/agents/{id}/config"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn get(id: AgentId) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(format!("/api/agents/{id}"))
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .expect("request")
}

/// Assert a sampling knob came back as the value that was sent.
///
/// Compared with a tolerance rather than for equality: these are `f32` in the
/// manifest and `f64` in JSON, so 0.15 widens to 0.15000000596046448 on the way
/// out. Exact equality here asserts something about float widths, not about the
/// behaviour under test.
fn assert_knob(actual: &serde_json::Value, expected: f64) {
    let got = actual
        .as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {actual}"));
    assert!(
        (got - expected).abs() < 1e-6,
        "expected ~{expected}, got {got}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn preferences_round_trip_through_all_three_states() {
    let h = boot().await;
    let id = spawn_on(&h.state, "prefs", "known-model");

    // A fresh agent has no opinion on any knob.
    let (status, body) = send(h.app.clone(), get(id)).await;
    assert_eq!(status, StatusCode::OK);
    // Guard against the assertions below passing vacuously: a typo in the path
    // would make every `is_null()` trivially true. `provider` is always
    // populated, so its presence proves we are reading the right object.
    assert_eq!(
        body["model"]["provider"], "ollama",
        "expected the model object at body.model: {body}"
    );
    assert!(
        body["model"]["temperature"].is_null(),
        "fresh agent must inherit: {body}"
    );
    assert!(
        body["model"]["max_tokens"].is_null(),
        "fresh agent must inherit: {body}"
    );

    // Pin every preference knob.
    let (status, _) = send(
        h.app.clone(),
        patch(
            id,
            serde_json::json!({
                "temperature": 0.15,
                "max_tokens": 8192,
                "top_p": 0.85,
                "frequency_penalty": 0.4,
                "presence_penalty": -0.3
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(h.app.clone(), get(id)).await;
    assert_knob(&body["model"]["temperature"], 0.15);
    assert_eq!(body["model"]["max_tokens"], serde_json::json!(8192));
    assert_knob(&body["model"]["top_p"], 0.85);
    assert_knob(&body["model"]["frequency_penalty"], 0.4);
    assert_knob(&body["model"]["presence_penalty"], -0.3);

    // An absent key leaves the others alone.
    let (status, _) = send(h.app.clone(), patch(id, serde_json::json!({"top_p": 0.5}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(h.app.clone(), get(id)).await;
    assert_knob(&body["model"]["top_p"], 0.5);
    assert_knob(&body["model"]["temperature"], 0.15);

    // An explicit null hands the field back to inherit.
    let (status, _) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"temperature": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(h.app.clone(), get(id)).await;
    assert!(
        body["model"]["temperature"].is_null(),
        "explicit null must clear the agent's own value: {body}"
    );
    assert_eq!(
        body["model"]["max_tokens"],
        serde_json::json!(8192),
        "clearing one knob must not clear the rest: {body}"
    );
}

/// The migration guarantee: an agent that already carries a number keeps it as
/// an explicit value. Nothing on an existing deployment starts inheriting
/// behind the operator's back.
#[tokio::test(flavor = "multi_thread")]
async fn an_existing_explicit_value_survives_unrelated_patches() {
    let h = boot().await;
    let id = spawn_on(&h.state, "migrated", "known-model");
    send(
        h.app.clone(),
        patch(id, serde_json::json!({"max_tokens": 4096})),
    )
    .await;

    // A PATCH touching only an unrelated field.
    let (status, _) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"description": "still explicit"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent entry");
    assert_eq!(
        entry.manifest.model.max_tokens,
        Some(4096),
        "an explicit value must stay explicit, not decay to inherit"
    );
}

/// Over a limit the registry actually asserted: the operator is told, and the
/// value they typed is what gets stored. A silent clamp would leave them
/// debugging a number they never chose.
#[tokio::test(flavor = "multi_thread")]
async fn exceeding_a_known_limit_warns_and_does_not_clamp() {
    let h = boot().await;
    let id = spawn_on(&h.state, "over-known", "known-model");

    let (status, body) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"max_tokens": 65_536})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an over-limit value is not an error"
    );

    let warnings = body["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "expected one warning, got: {body}");
    assert_eq!(warnings[0]["field"], "max_tokens");
    assert_eq!(warnings[0]["requested"], 65_536);
    assert_eq!(warnings[0]["limit"], 16_384);
    assert_eq!(warnings[0]["limit_source"], "registry");

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent entry");
    assert_eq!(
        entry.manifest.model.max_tokens,
        Some(65_536),
        "the requested value must be stored unmodified"
    );
}

/// The same number against a discovery placeholder stays silent. Warning
/// against an invented ceiling is noise, and noise is what makes operators
/// stop reading warnings (#7780).
#[tokio::test(flavor = "multi_thread")]
async fn exceeding_an_inferred_limit_stays_silent() {
    let h = boot().await;
    let id = spawn_on(&h.state, "over-inferred", "discovered-model");

    let (status, body) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"max_tokens": 65_536})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["warnings"].as_array().map(Vec::len),
        Some(0),
        "an inferred limit must not produce a warning: {body}"
    );
}

/// An operator-set `max_output_tokens` describes this endpoint and outranks
/// the catalog's figure for it.
#[tokio::test(flavor = "multi_thread")]
async fn operator_limit_outranks_the_catalog_limit() {
    let h = boot().await;
    let id = spawn_on(&h.state, "operator-limit", "known-model");

    let (status, body) = send(
        h.app.clone(),
        patch(
            id,
            serde_json::json!({"max_output_tokens": 4096, "max_tokens": 8192}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let warnings = body["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "expected one warning, got: {body}");
    assert_eq!(warnings[0]["limit"], 4096);
    assert_eq!(warnings[0]["limit_source"], "operator");
}

/// A context window inside the model's own is unremarkable; above it, warned.
#[tokio::test(flavor = "multi_thread")]
async fn context_window_is_checked_against_the_catalog_window() {
    let h = boot().await;
    let id = spawn_on(&h.state, "ctx", "known-model");

    let (_, body) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"context_window": 128_000})),
    )
    .await;
    assert_eq!(body["warnings"].as_array().map(Vec::len), Some(0));

    let (_, body) = send(
        h.app.clone(),
        patch(id, serde_json::json!({"context_window": 1_000_000})),
    )
    .await;
    let warnings = body["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "expected one warning, got: {body}");
    assert_eq!(warnings[0]["field"], "context_window");
    assert_eq!(warnings[0]["limit"], 200_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn out_of_range_preferences_are_rejected() {
    let h = boot().await;
    let id = spawn_on(&h.state, "ranges", "known-model");

    for body in [
        serde_json::json!({"temperature": 3.0}),
        serde_json::json!({"top_p": 1.5}),
        serde_json::json!({"frequency_penalty": -3.0}),
        serde_json::json!({"presence_penalty": 2.5}),
        serde_json::json!({"max_tokens": 0}),
        serde_json::json!({"context_window": 0}),
    ] {
        let (status, _) = send(h.app.clone(), patch(id, body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "should reject {body}");
    }

    // `null` is always valid — it stores nothing, so there is nothing to range-check.
    for body in [
        serde_json::json!({"temperature": null}),
        serde_json::json!({"top_p": null}),
        serde_json::json!({"max_tokens": null}),
        serde_json::json!({"context_window": null}),
    ] {
        let (status, _) = send(h.app.clone(), patch(id, body.clone())).await;
        assert_eq!(status, StatusCode::OK, "should accept {body}");
    }
}
