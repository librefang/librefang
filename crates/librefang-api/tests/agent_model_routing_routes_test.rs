//! Integration tests for the model-router HTTP surface.
//!
//! Exercises the production router (`server::build_router`) with
//! `tower::ServiceExt::oneshot`, mirroring `agent_channels_routes_test.rs`.
//! No real LLM calls — every test is hermetic.
//!
//! Routes covered:
//!   GET  /api/agents/{id}/model_routing  (default shape, flexible shape)
//!   PUT  /api/agents/{id}/model_routing  (round-trip, fixed/default_profile
//!                                         partial-save preservation, clear
//!                                         back to fixed, validation, bad id,
//!                                         unknown agent, non-owner)
//!   GET  /api/model-router/profiles      (builtin catalog, home override,
//!                                         deterministic ordering)
//!
//! Run: cargo test -p librefang-api --test agent_model_routing_routes_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::middleware;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use librefang_types::model_profile::ModelRouterConfig;
use std::path::Path;
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

/// Boot a kernel over a fresh temp home, optionally seeding a
/// `model_profiles.toml` override and a `[model_router]` config block.
async fn boot_with(model_router: ModelRouterConfig, seed_profiles: Option<&str>) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    if let Some(contents) = seed_profiles {
        std::fs::write(tmp.path().join("model_profiles.toml"), contents).expect("seed profiles");
    }

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        model_router,
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

async fn boot() -> Harness {
    boot_with(ModelRouterConfig::default(), None).await
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
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
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn names_of(profiles: &serde_json::Value) -> Vec<String> {
    profiles
        .as_array()
        .expect("profiles array")
        .iter()
        .map(|p| p["name"].as_str().expect("profile name").to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/agents/{id}/model_routing
// ---------------------------------------------------------------------------

/// A freshly spawned agent must report the backward-compatible default:
/// fixed mode, no profile allowlist, no cost cap.
#[tokio::test(flavor = "multi_thread")]
async fn get_model_routing_defaults_to_fixed() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-default");

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["mode"], "fixed");
    assert_eq!(body["allowed_profiles"], serde_json::json!([]));
    assert!(body["cost_budget"].is_null());
    assert!(body["default_profile"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_model_routing_rejects_a_malformed_agent_id() {
    let h = boot().await;
    let (status, _) = send(h.app.clone(), get("/api/agents/not-a-uuid/model_routing")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_model_routing_404s_for_an_unknown_agent() {
    let h = boot().await;
    let missing = AgentId::new();
    let (status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{missing}/model_routing")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// PUT /api/agents/{id}/model_routing
// ---------------------------------------------------------------------------

/// PUT then GET: flexible mode with an allowlist, a cost budget and a default
/// profile must all survive the round-trip.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_flexible_round_trips() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-roundtrip");

    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["coder", "architect"],
                "cost_budget": "medium",
                "default_profile": "coder",
            }),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "PUT body={put_body:?}");
    assert_eq!(put_body["status"], "ok");

    let (get_status, get_body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    assert_eq!(get_body["mode"], "flexible");
    assert_eq!(get_body["cost_budget"], "medium");
    assert_eq!(get_body["default_profile"], "coder");
    // Ordered because `allowed_profiles` is a BTreeSet on the manifest (#3298).
    assert_eq!(
        get_body["allowed_profiles"],
        serde_json::json!(["architect", "coder"])
    );
}

/// #7781 review: `fixed` — the documented per-agent router bypass — must be
/// readable through GET so a client can round-trip it (it was write-only
/// before), and `default_profile` must survive a save that omits it or
/// sends an explicit `null`; only an explicit empty string clears it.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_preserves_fixed_and_default_profile_across_partial_saves() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-fixed-and-default");

    // Establish fixed=true and default_profile="coder".
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "fixed": true,
                "default_profile": "coder",
            }),
        ),
    )
    .await;
    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(
        body["fixed"], true,
        "GET must round-trip the stored fixed flag"
    );
    assert_eq!(body["default_profile"], "coder");

    // A save that omits both keys must not clear either.
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "allowed_profiles": ["coder"] }),
        ),
    )
    .await;
    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(
        body["fixed"], true,
        "omitting fixed must preserve the stored value"
    );
    assert_eq!(
        body["default_profile"], "coder",
        "omitting default_profile must preserve it"
    );

    // An explicit null for default_profile must also preserve it, not clear it.
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "default_profile": null }),
        ),
    )
    .await;
    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(
        body["default_profile"], "coder",
        "explicit null must preserve, not clear"
    );

    // Only an explicit empty string clears it.
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "default_profile": "" }),
        ),
    )
    .await;
    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert!(
        body["default_profile"].is_null(),
        "an explicit empty string is the one way to clear it"
    );
}

/// The allowlist is a set: duplicates collapse and order is normalised, so
/// two clients sending the same names in different orders converge on the
/// same manifest and the same provider prompt cache (#3298).
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_normalises_the_allowlist() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-normalise");

    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["quick", "coder", "quick", "architect"],
            }),
        ),
    )
    .await;

    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(
        body["allowed_profiles"],
        serde_json::json!(["architect", "coder", "quick"])
    );
}

/// Switching back to fixed must clear the override entirely rather than leave
/// stale constraints hanging off the manifest for a later flip to resurrect.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_fixed_clears_the_override() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-clear");

    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["coder"],
                "cost_budget": "cheap",
            }),
        ),
    )
    .await;

    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "fixed" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(body["mode"], "fixed");
    assert_eq!(body["allowed_profiles"], serde_json::json!([]));
    assert!(body["cost_budget"].is_null());
    assert!(body["default_profile"].is_null());
}

/// A typo in `cost_budget` must be rejected, not silently read as "no cap".
/// Silently dropping a spending cap is exactly the failure an operator would
/// never notice until the bill arrived.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_rejects_an_unknown_cost_budget() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-bad-budget");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "cost_budget": "default" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");

    // The rejected write must not have taken effect.
    let (_, after) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(after["mode"], "fixed");
}

/// A non-string, non-null `cost_budget` (a number, or a boolean left behind
/// by an unset form toggle) must not silently read as "no cap" — that would
/// hand the agent the most expensive tier with no cap at all (#7781 review).
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_rejects_a_non_string_cost_budget() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-numeric-budget");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "cost_budget": 5 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body:?}");

    let (_, after) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(
        after["mode"], "fixed",
        "the rejected write must not take effect"
    );
}

/// An explicit `null` budget is the documented "no cap" value and is accepted.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_accepts_a_null_cost_budget() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-null-budget");

    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "flexible", "cost_budget": null }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(body["mode"], "flexible");
    assert!(body["cost_budget"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_rejects_an_unknown_mode() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-bad-mode");

    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({ "mode": "Flexible" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// #7781 review: an unknown agent id must 404, the same as the GET side,
/// instead of falling through to the kernel call and coming back as a 400
/// — the shape a malformed body gets, which hides "this agent does not
/// exist" behind "you sent something wrong".
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_404s_for_an_unknown_agent() {
    let h = boot().await;
    let missing = AgentId::new();
    let (status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{missing}/model_routing"),
            serde_json::json!({ "mode": "flexible" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// #7781 review: the write side of this route must apply the same
/// per-agent ownership scoping the GET side already does (`can_access_agent`)
/// — a caller who cannot see this agent must not be able to mutate its
/// routing settings either.
///
/// The full server's RBAC role gate (`middleware::user_role_allows_request`)
/// requires Admin+ for any non-GET verb, and `can_access_agent` itself
/// admits Admin+ unconditionally, so a real request through the whole
/// stack can never exercise a denial here. This mounts the agents router
/// directly and injects the caller identity the same way
/// `api_integration_test.rs` does for its handler-level ownership checks,
/// isolating `can_access_agent` from that unrelated role gate one layer up.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_404s_for_a_non_owner() {
    let h = boot().await;
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: "routing-owner-scope".to_string(),
            author: "Alice".to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored agent");

    let app = axum::Router::new()
        .nest("/api", librefang_api::routes::agents::router())
        .with_state(h.state.clone());

    let mut request = Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/agents/{agent_id}/model_routing"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "mode": "flexible" }).to_string(),
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(middleware::AuthenticatedApiUser {
            name: "Bob".to_string(),
            role: middleware::UserRole::User,
            user_id: librefang_types::agent::UserId::from_name("Bob"),
        });

    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a non-owner must not be able to see or mutate another user's agent"
    );

    // The refused write must not have taken effect.
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("agent still registered");
    assert_eq!(
        entry.manifest.model.mode,
        librefang_types::agent::ModelMode::Fixed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_rejects_a_malformed_agent_id() {
    let h = boot().await;
    let (status, _) = send(
        h.app.clone(),
        put_json(
            "/api/agents/not-a-uuid/model_routing",
            serde_json::json!({ "mode": "flexible" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The change must reach `agent.toml`, not just the in-memory registry —
/// otherwise it silently reverts on the next daemon restart.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_persists_to_the_agent_manifest() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-persist");

    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["coder"],
                "cost_budget": "medium",
            }),
        ),
    )
    .await;

    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent still registered");
    assert_eq!(
        entry.manifest.model.mode,
        librefang_types::agent::ModelMode::Flexible
    );
    let ov = entry
        .manifest
        .model
        .router_override
        .as_ref()
        .expect("router override stored");
    assert!(ov.allowed_profiles.contains("coder"));
    assert_eq!(
        ov.cost_budget,
        Some(librefang_types::model_profile::CostTier::Medium)
    );

    let manifest_path = find_agent_toml(h._tmp.path(), "routing-persist");
    let on_disk = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
    assert!(
        on_disk.contains("flexible"),
        "agent.toml must record the routing mode, got:\n{on_disk}"
    );
}

/// Locate a spawned agent's `agent.toml` without hard-coding the workspace
/// layout, which has moved before (`.identity/` migration).
fn find_agent_toml(home: &Path, agent_name: &str) -> std::path::PathBuf {
    fn walk(dir: &Path, agent_name: &str, out: &mut Option<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, agent_name, out);
            } else if path.file_name().is_some_and(|f| f == "agent.toml") {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if contents.contains(agent_name) {
                        *out = Some(path);
                    }
                }
            }
        }
    }
    let mut found = None;
    walk(home, agent_name, &mut found);
    found.unwrap_or_else(|| panic!("no agent.toml for '{agent_name}' under {}", home.display()))
}

// ---------------------------------------------------------------------------
// GET /api/model-router/profiles
// ---------------------------------------------------------------------------

/// With no override file on disk, the endpoint serves the builtin catalog.
#[tokio::test(flavor = "multi_thread")]
async fn list_profiles_serves_the_builtin_catalog() {
    let h = boot().await;

    let (status, body) = send(h.app.clone(), get("/api/model-router/profiles")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");

    // Off by default, but the catalog is still browsable so an operator can
    // review it before flipping the switch.
    assert_eq!(body["enabled"], false);

    let names = names_of(&body["profiles"]);
    for expected in ["architect", "coder", "quick", "researcher"] {
        assert!(
            names.iter().any(|n| n == expected),
            "builtin catalog is missing '{expected}': {names:?}"
        );
    }

    let profiles = body["profiles"].as_array().expect("profiles array");
    let coder = profiles
        .iter()
        .find(|p| p["name"] == "coder")
        .expect("coder profile");
    assert!(coder["provider"].is_string());
    assert!(coder["model"].is_string());
    assert!(coder["cost_tier"].is_string());
}

/// Profiles come back name-sorted, so the list is byte-identical across
/// processes regardless of the order they appear in the asset (#3298).
#[tokio::test(flavor = "multi_thread")]
async fn list_profiles_is_ordered_by_name() {
    let h = boot().await;
    let (_, body) = send(h.app.clone(), get("/api/model-router/profiles")).await;

    let names = names_of(&body["profiles"]);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "profile list must be deterministic (#3298)");
}

/// `~/.librefang/model_profiles.toml` overrides the builtin of the same name
/// and adds new ones — verified through the real HTTP surface, so the whole
/// config -> loader -> route chain is covered, not just the loader.
#[tokio::test(flavor = "multi_thread")]
async fn list_profiles_reflects_the_home_override_file() {
    let h = boot_with(
        ModelRouterConfig {
            enabled: true,
            ..Default::default()
        },
        Some(
            r#"
[[profiles]]
name = "coder"
tags = ["code"]
provider = "ollama"
model = "qwen3.5-coder"
cost_tier = "cheap"

[[profiles]]
name = "on-prem"
tags = ["internal"]
provider = "vllm"
model = "internal-70b"
"#,
        ),
    )
    .await;

    let (status, body) = send(h.app.clone(), get("/api/model-router/profiles")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["enabled"], true);

    let profiles = body["profiles"].as_array().expect("profiles array");

    // The builtin "coder" was replaced, not duplicated.
    let coders: Vec<_> = profiles.iter().filter(|p| p["name"] == "coder").collect();
    assert_eq!(coders.len(), 1, "override must replace, not duplicate");
    assert_eq!(coders[0]["provider"], "ollama");
    assert_eq!(coders[0]["model"], "qwen3.5-coder");
    assert_eq!(coders[0]["cost_tier"], "cheap");

    // A brand-new profile was added.
    assert!(profiles.iter().any(|p| p["name"] == "on-prem"));

    // Untouched builtins survive.
    assert!(profiles.iter().any(|p| p["name"] == "architect"));
}

/// A malformed override file must not take the endpoint (or the daemon) down:
/// the builtins are served instead.
#[tokio::test(flavor = "multi_thread")]
async fn list_profiles_survives_a_malformed_override_file() {
    let h = boot_with(
        ModelRouterConfig::default(),
        Some("this is not valid toml [[["),
    )
    .await;

    let (status, body) = send(h.app.clone(), get("/api/model-router/profiles")).await;
    assert_eq!(status, StatusCode::OK, "body={body:?}");
    let names = names_of(&body["profiles"]);
    assert!(names.iter().any(|n| n == "coder"));
}
